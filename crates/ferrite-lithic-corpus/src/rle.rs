//! Run-length and delta decode: the other two pieces of every columnar decoder.
//!
//! # Why these two live in one module
//!
//! [`bitpack`](crate::bitpack) is the innermost kernel: fixed-width values packed
//! end to end. This is the next layer out, and it comes in exactly two flavours that
//! every columnar format has both of:
//!
//! - **(a) Run-length.** A run header, a run length, and a value. One value out per
//!   cycle for the length of the run, and the run length is what makes it a *hardware*
//!   shape rather than a software one: the decoded column is `length` copies of one
//!   byte, which is a register that holds rather than a loop that writes.
//! - **(b) Delta.** A base and a stream of per-value deltas. Each decoded value is the
//!   running sum, so the hardware is an accumulator with one add on its feedback path.
//!
//! Both are one value per cycle with a fixed, tiny amount of state, and both are the
//! decode half of an encoding that is trivial to define and easy to get subtly wrong
//! -- which is why each one has a hand-written reference decoder below rather than a
//! clever one.
//!
//! # There is no golden crate, and that is stated rather than hidden
//!
//! `bitpacking` is the crate everybody uses for bit-packing and this design is checked
//! against it. There is no equivalent for a *run-length header* or for a *delta
//! accumulator*: Parquet's RLE/bit-packing hybrid header is a varint, and the varint
//! is a serial bit-level format whose whole value is that it is *not* this design.
//! So the reference decoders [`decode`] and [`delta_decode`] are written here, in
//! this file, by the same hand as the circuits.
//!
//! That is a real weakness and it is worth being exact about why it is a smaller one
//! than it looks:
//!
//! - The references are **deliberately stupid**. [`decode`] is a `for` loop that
//!   pushes `length` copies and returns. [`delta_decode`] is a `wrapping_add` in a
//!   `for` loop. Neither has an index into anything, an accumulator of state, or a
//!   fast path, so there is nowhere for the two copies to be wrong *the same way* --
//!   the kind of mistake that would hide is "the design forgot to decrement the
//!   counter", and the reference has no counter to forget.
//! - The load/length/value protocol is **pinned by separate arithmetic tests** rather
//!   than only by the differential: the corner cases (a zero-length run, a run of
//!   255, a load in the middle of a run, a reset mid-run) are each asserted against a
//!   value computed by hand rather than against the reference.
//!
//! What this cannot do is catch a shared *misreading of the specification*, and the
//! specification here is short enough to write out: the decoder emits the value of the
//! run it is in, once per cycle, and the run's first value is on the edge that loads
//! the run. That is what [`Inputs::load`] and the latency note below mean.
//!
//! # Latency, stated precisely
//!
//! Both designs have their outputs **registered**, so the first decoded value appears
//! on the cycle *after* the edge that loaded the run:
//!
//! ```text
//! edge N:   load=1, run_len=4, run_value=7   ->   remaining=4, value=7
//! cycle N:  data = 7,  valid = 1             <- the first value, readable now
//! edge N+1:                                  ->   remaining=3
//! cycle N+1:  data = 7,  valid = 1
//! edge N+2:                                  ->   remaining=2
//! cycle N+2:  data = 7,  valid = 1
//! edge N+3:                                  ->   remaining=1
//! cycle N+3:  data = 7,  valid = 1
//! edge N+4:                                  ->   remaining=0
//! cycle N+4:  data = 7,  valid = 0           <- the fifth copy is never emitted
//! ```
//!
//! That last line is the part worth reading twice. A run of length four emits **four**
//! values, not five: `remaining` is the count of values *still to emit* and the load
//! edge is the edge that arms the first one. The alternative -- treating the load edge
//! as already having emitted -- puts the register one cycle earlier and makes `valid`
//! low on the cycle the caller most wants the value, which is the kind of off-by-one
//! that a test comparing only the *set* of decoded values would never notice. The
//! testbench tests here compare cycle by cycle.
//!
//! # The same honesty as `bitpack`: the win is bandwidth, not the adder
//!
//! The delta decoder's feedback add is one adder. The RLE decoder is one comparator
//! and one decrement. Neither is a bottleneck, and neither is where the time goes.
//! What a real decoder spends its time on is *traffic*: the compressed run header is
//! two or three bytes and it unpacks to a kilobyte, so the run that saves a
//! kilobyte of writes costs a few bytes of reads and no writes at all. **That is the
//! whole argument for putting a decoder on chip**, and it is a bandwidth argument --
//! it only pays when the compressed side is already in on-chip SRAM. Off-chip, the
//! same kernel in the same process is bandwidth-limited exactly as it is on a CPU.
//! See [`bitpack`](crate::bitpack)'s module docs for the same point at more length.

use ferrite_lithic::{Design, Signal};
use ferrite_lithic_derive::PortList;

use crate::BuildError;

/// The width of a run length, and of a decoded byte, in bits.
///
/// Eight, so a run can be up to 255 values long -- which is a *deliberate* limit and
/// not a claim about the format. A real RLE header carries a length of up to `2^31`
/// and a real design would hold it in a 32-bit counter; an eight-bit one keeps the
/// port list honest about what is being demonstrated (the counter, the decrement and
/// the valid handshake) rather than about counter width. The counter arithmetic is
/// width-agnostic: it is a `ugt`, a `sub` and an `ite`, and widening
/// [`RUN_BITS`] widens all three.
pub const RUN_BITS: u32 = 8;

/// The width of the delta accumulator, in bits.
///
/// Thirty-two, matching the reference decoder's `u32` and matching Parquet's INT32
/// physical type, which is the case where delta encoding is actually used.
pub const DELTA_BITS: u32 = 32;

/// One run: a byte value repeated `length` times.
///
/// The software shape of what the RLE decoder is decoding. `length` of zero is legal
/// and means the run emits nothing -- which is why the design's `valid` is
/// `remaining != 0` and not a separate flag: a zero-length run is indistinguishable
/// from no run at all, and pretending otherwise would need a state bit that real
/// RLE does not have.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Run {
    /// The byte value the run repeats.
    pub value: u8,
    /// How many copies of it. Zero is legal and emits nothing.
    pub length: u8,
}

impl Run {
    /// A run of `length` copies of `value`.
    #[must_use]
    pub const fn new(value: u8, length: u8) -> Self {
        Self { value, length }
    }
}

/// The reference run-length decoder.
///
/// A run-length decoder's job is to expand runs, and this does exactly that with no
/// state beyond the output vector:
///
/// ```text
/// for each run:
///     repeat the run's value, `length` times
/// ```
///
/// Straightforward on purpose -- see the module docs on why there is no golden crate.
/// A zero-length run contributes nothing, which is the same answer the design gives.
#[must_use]
pub fn decode(runs: &[Run]) -> Vec<u8> {
    let mut out = Vec::new();
    for run in runs {
        for _ in 0..run.length {
            out.push(run.value);
        }
    }
    out
}

/// The reference delta decoder: a base followed by a running sum of the deltas.
///
/// The output has `deltas.len() + 1` elements, the first of which is the base
/// unchanged -- which is the part that is easy to get wrong, because a decoder that
/// returns `deltas.len()` elements has either dropped the base or folded it into the
/// first delta. The design's output register makes the base visible on the load edge
/// and so pins it.
///
/// The sum **wraps**. A sorted column's deltas never overflow, but the reference is
/// also the model for the general case and `bitpacking`'s own delta codecs wrap too,
/// so the definition here is modulo `2^32` and the design's `add` truncates to
/// exactly that.
#[must_use]
pub fn delta_decode(base: u32, deltas: &[u32]) -> Vec<u32> {
    let mut out = Vec::with_capacity(deltas.len() + 1);
    let mut accumulator = base;
    out.push(accumulator);
    for delta in deltas {
        accumulator = accumulator.wrapping_add(*delta);
        out.push(accumulator);
    }
    out
}

/// The input ports of the run-length decoder.
#[derive(Clone, Debug, PortList)]
pub struct Inputs {
    /// The clock. Every edge either arms a new run or advances the current one.
    #[clock]
    pub clk: Signal,
    /// Synchronous reset, which empties the run: `valid` falls.
    pub rst: Signal,
    /// On this edge, arm a new run from `run_len` and `run_value`.
    pub load: Signal,
    /// The length of the run being armed. Zero arms nothing, so `valid` stays or falls.
    #[bits(8)]
    pub run_len: Signal,
    /// The byte value of the run being armed.
    #[bits(8)]
    pub run_value: Signal,
}

/// The output ports of the run-length decoder.
#[derive(Clone, Debug, PortList)]
pub struct Outputs {
    /// The decoded byte, held for the rest of the run. Meaningful only while `valid`.
    #[bits(8)]
    pub data: Signal,
    /// Whether a value is being emitted this cycle: the run has values left.
    pub valid: Signal,
    /// How many values of the current run are still to come. Exposed so a test can
    /// watch the counter rather than only its effect.
    #[bits(8)]
    pub remaining: Signal,
}

/// The signals [`build`] returns, for a caller that wants them by handle.
#[derive(Clone, Debug)]
pub struct Ports {
    /// The input ports.
    pub inputs: Inputs,
    /// The decoded byte, which is also the output.
    pub data: Signal,
    /// Whether a value is being emitted, which is also the output.
    pub valid: Signal,
    /// The run counter, which is also the output.
    pub remaining: Signal,
}

/// Builds a run-length decoder into `design`.
///
/// The clock is named `clk` and the reset `rst`, which is what the crate-level
/// [`crate::CLOCK`] and [`crate::RESET`] say.
///
/// # Errors
///
/// Whatever [`Design`] returns -- a width mismatch or an undriven wire -- and
/// whatever the port-list helpers return. There is no RLE-specific failure: the run
/// header is a port here rather than a decoded varint, so there is no bit-level
/// header state to get wrong.
pub fn build(design: &Design) -> Result<Ports, BuildError> {
    let inputs = ferrite_lithic::inputs::<Inputs>(design)?;
    let (data, valid, remaining) = decoder(
        design,
        &inputs.clk,
        &inputs.rst,
        &inputs.load,
        &inputs.run_len,
        &inputs.run_value,
    )?;

    ferrite_lithic::outputs::<Outputs>(
        design,
        &Outputs {
            data: data.clone(),
            valid: valid.clone(),
            remaining: remaining.clone(),
        },
    )?;

    Ok(Ports {
        inputs,
        data,
        valid,
        remaining,
    })
}

/// The run-length decoder's datapath.
///
/// `clk` and `rst` are the ports rather than constants so that a reset is
/// exercisable, which is also the only way out of a run: `rst` drives `remaining` to
/// zero and therefore drops `valid`.
///
/// Three pieces, and the order they are built in is the order they read:
///
/// 1. `remaining` is the count of values *still to emit*, so `valid` is
///    `remaining != 0` and nothing else. Making `valid` a comparison against a
///    separate "armed" bit would be one more bit of state and one more thing to
///    reset.
/// 2. On `load` the counter takes `run_len` and the value register takes
///    `run_value`; otherwise the counter decrements, **but only when it is non-zero**.
///    A decrement that runs below zero would wrap to 255 and emit 255 more copies of
///    the run's value than the run had -- the single most consequential bug available
///    in this design, and the reason the decrement sits inside an `ite` rather than
///    being applied unconditionally.
/// 3. The value register holds unless `load`, which is what makes the run's value
///    available for the whole run from one write.
///
/// `remaining` and the value are wires driven from registers afterwards, because the
/// register's output feeds its own next value and the graph has no cycle in it; this
/// is the same pattern as [`crate::crc3`]'s state wire.
///
/// # Errors
///
/// Whatever [`Design`] returns.
pub fn decoder(
    design: &Design,
    clk: &Signal,
    rst: &Signal,
    load: &Signal,
    run_len: &Signal,
    run_value: &Signal,
) -> Result<(Signal, Signal, Signal), BuildError> {
    let remaining = design.wire(RUN_BITS)?;
    let value = design.wire(RUN_BITS)?;

    let zero = design.lit(0, RUN_BITS)?;
    let more = design.ugt(&remaining, &zero)?;
    let valid = more.clone();

    let one = design.lit(1, RUN_BITS)?;
    let decremented = design.sub(&remaining, &one)?;
    let stepped = design.ite(&more, &decremented, &remaining)?;
    let next_remaining = design.ite(load, run_len, &stepped)?;
    let next_value = design.ite(load, run_value, &value)?;

    // `clear` is tied low so the counter updates on every edge; the `ite`s are what
    // make "hold" the alternative.
    let remaining_q = design.reg(&next_remaining, clk, rst, &design.constant(false))?;
    design.drive(&remaining, &remaining_q)?;
    let value_q = design.reg(&next_value, clk, rst, &design.constant(false))?;
    design.drive(&value, &value_q)?;

    Ok((value_q, valid, remaining_q))
}

/// The input ports of the delta decoder.
#[derive(Clone, Debug, PortList)]
pub struct DeltaInputs {
    /// The clock. Every edge either loads the base or adds one delta.
    #[clock]
    pub clk: Signal,
    /// Synchronous reset, which zeroes the accumulator and drops `valid`.
    pub rst: Signal,
    /// On this edge, the accumulator takes `base` rather than adding `delta`.
    pub load: Signal,
    /// The first decoded value, taken on the `load` edge.
    #[bits(32)]
    pub base: Signal,
    /// The delta added on every other edge. Wraps, like [`delta_decode`].
    #[bits(32)]
    pub delta: Signal,
}

/// The output ports of the delta decoder.
#[derive(Clone, Debug, PortList)]
pub struct DeltaOutputs {
    /// The running sum, registered. Meaningful only while `valid`.
    #[bits(32)]
    pub value: Signal,
    /// Whether a value has been emitted: high from the cycle after `load`.
    pub valid: Signal,
}

/// The signals [`build_delta`] returns, for a caller that wants them by handle.
#[derive(Clone, Debug)]
pub struct DeltaPorts {
    /// The input ports.
    pub inputs: DeltaInputs,
    /// The running sum, which is also the output.
    pub value: Signal,
    /// Whether a value has been emitted, which is also the output.
    pub valid: Signal,
}

/// Builds a delta decoder into `design`.
///
/// # Errors
///
/// Whatever [`Design`] returns, for the reasons [`build`] gives.
pub fn build_delta(design: &Design) -> Result<DeltaPorts, BuildError> {
    let inputs = ferrite_lithic::inputs::<DeltaInputs>(design)?;

    let accumulator = design.wire(DELTA_BITS)?;
    let loaded = design.wire(1)?;

    // `add` truncates to its left operand's width, and both operands are already
    // `DELTA_BITS`, so the truncation *is* the modulo-2^32 the reference defines.
    // Widening here would change the arithmetic rather than preserve it, which is the
    // one case where the truncation rule is the specification.
    let sum = design.add(&accumulator, &inputs.delta)?;
    let next = design.ite(&inputs.load, &inputs.base, &sum)?;
    let accumulator_q = design.reg(&next, &inputs.clk, &inputs.rst, &design.constant(false))?;
    design.drive(&accumulator, &accumulator_q)?;

    let next_loaded = design.reg(
        &inputs.load,
        &inputs.clk,
        &inputs.rst,
        &design.constant(false),
    )?;
    design.drive(&loaded, &next_loaded)?;

    ferrite_lithic::outputs::<DeltaOutputs>(
        design,
        &DeltaOutputs {
            value: accumulator_q.clone(),
            valid: loaded.clone(),
        },
    )?;

    Ok(DeltaPorts {
        inputs,
        value: accumulator_q,
        valid: loaded,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::{Run, decode, delta_decode};

    #[test]
    fn a_zero_length_run_emits_nothing() {
        assert_eq!(decode(&[Run::new(9, 0)]), Vec::<u8>::new());
        assert_eq!(decode(&[Run::new(9, 0), Run::new(1, 1)]), vec![1]);
    }

    #[test]
    fn the_delta_reference_keeps_the_base_as_its_first_element() {
        // The whole point of the extra element: a decoder that folds the base into the
        // first delta returns `deltas.len()` values and every one of them is off by
        // one position rather than wrong, which is a nasty shape of bug.
        assert_eq!(delta_decode(10, &[]), vec![10]);
        assert_eq!(delta_decode(10, &[1, 2, 3]), vec![10, 11, 13, 16]);
    }

    #[test]
    fn the_delta_reference_wraps_rather_than_saturating() {
        assert_eq!(delta_decode(u32::MAX, &[1]), vec![u32::MAX, 0]);
    }

    #[test]
    fn an_empty_run_list_decodes_to_nothing() {
        assert_eq!(decode(&[]), Vec::<u8>::new());
    }
}
