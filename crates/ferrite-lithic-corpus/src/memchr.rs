//! Single-byte search: one byte per cycle, a comparator, and an honest loss.
//!
//! # This entry is the baseline, and it loses
//!
//! The three modules in this tier -- [`memchr`](crate::memchr), [`aho_corasick`](crate::aho_corasick) and [`dfa`](crate::dfa) -- share
//! one shape: a state register, a transition table, one byte in per clock edge. The
//! argument for that shape in an ASIC is concrete and worth stating before the
//! individual designs muddy it:
//!
//! - **Cycles per byte: exactly 1.** The state after byte `i` depends on the state
//!   after byte `i - 1`; there is no loop unrolling to be had and no lane structure to
//!   fill. A byte per clock edge is the ceiling for any *serial* automaton.
//! - **State width: a handful of flops.** [`memchr`](crate::memchr) carries 17 (a [`COUNTER_BITS`]
//!   -bit position counter, an 8-bit offset and a `found` bit). [`aho_corasick`](crate::aho_corasick) and
//!   [`dfa`](crate::dfa) carry a state register of 5 and 3 bits respectively *plus* the same
//!   reporting registers. Lanes are cheap because the per-lane state is a few flops
//!   and the transition table is **shared** -- one ROM read by N lanes, so N lanes cost
//!   N registers and N comparators, not N ROMs.
//! - **The CPU gets SIMD, and it wins by a lot.** The `memchr` crate is one of the
//!   best-optimised byte scanners ever written: on any host with AVX2 it compares 32
//!   bytes per instruction and finds the first match with a `tzcnt`, so it retires
//!   roughly *one instruction per 32 bytes*. This design retires one instruction per
//!   byte. On bytes per cycle the CPU is ahead by a factor of about 32, and there is
//!   no way to close that from inside this design -- the automaton is serial.
//!
//! So the honest conclusion, which the tests measure and print rather than assert:
//! **for one needle, the CPU wins, by more than an order of magnitude.** An ASIC does
//! not win a single-byte search against a SIMD instruction; it loses at the first
//! comparison and it keeps losing.
//!
//! The argument that *does* survive is narrower and is the one the other two modules
//! are for:
//!
//! 1. **A single needle is a special case, not the general one.** [`aho_corasick`](crate::aho_corasick)
//!    carries 11 symbols and 19 states; [`dfa`](crate::dfa) carries 64 symbols and 5 states. Both
//!    are one byte per cycle and neither has a SIMD form: there is no 32-lane
//!    formulation of "advance 32 copies of a 19-state automaton and OR their accept
//!    bits", because the state has to be carried serially. The crate that does this
//!    on a CPU -- `regex-automata`'s dense DFA -- also runs one byte at a time; it
//!    wins on constant factors (SIMD prefilters, memchr-style scanning for the first
//!    byte of a rare literal) rather than on lanes.
//! 2. **The byte rate can be imposed rather than chosen.** A design sitting between two
//!    blocks at 1 byte/clock is not competing with a CPU's ability to go faster; it is
//!    satisfying a bandwidth requirement that a CPU would have to keep up with while
//!    doing something else.
//! 3. **A CPU gets nothing for free here.** Every automaton state transition is a
//!    dependent load. The DFA's serial dependency chain is exactly as long as its
//!    latency, so it cannot be widened without changing the algorithm.
//!
//! # The design
//!
//! One byte per edge, unconditionally, with no `valid`: this is a *streaming* searcher,
//! and the shape is the same as [`crc3`](crate::crc3)'s -- the byte stream is
//! unconditional so the differential can push an arbitrary buffer through without the
//! test having to model idle cycles. What is added is a `start` input, because a
//! scanner that cannot be restarted without a full reset cannot scan two buffers, and
//! a scanner that can only scan one buffer is not a scanner.
//!
//! `start` costs a cycle of its own and the byte presented on the `start` edge is
//! discarded. That is the same contract as [`crc32`](crate::crc32)'s `init`, and the
//! tests use `tb.set` rather than `tb.drive` for it so that asserting and releasing it
//! does not each spend an edge.
//!
//! # Two formulations of `hit`, and which one this uses
//!
//! The obvious way to ask "is this byte the needle" is [`Design::eq`] against a
//! constant: eight XNORs and an AND, about two logic levels, no memory at all.
//!
//! The other way is a 256-entry ROM of (match, offset) -- one bit of match per byte
//! value -- which is what a block-RAM implementation would build, and which
//! [`match_rom`] builds here as a [`Design::case_`] because the IR has no
//! initialised-memory node yet. The two are the same function and
//! [`build_from_rom`] exists so a test can build the whole design both ways and check
//! that they agree on all 256 byte values. That test is worth having for a second
//! reason: a 256-arm `case_` is the largest table this crate emits, and it is the
//! stress point for the emitter, the simulator and Verilator.
//!
//! **A real block RAM would save area, not cycles.** 256 x 1 bits is one bit per byte
//! value, which is smaller than the 64-input comparator a synthesiser builds from
//! `eq` on some processes, and larger on others -- it depends entirely on whether the
//! library has a 256x1 ROM macro. It buys no throughput either way: one byte per
//! cycle, one read per cycle, both formulations.

use ferrite_lithic::{Design, Signal};
use ferrite_lithic_derive::PortList;

use crate::BuildError;

/// The byte this design searches for.
///
/// A newline, because "where does this line end" is the single-byte search that
/// actually shows up in a data path, and because it is a byte that a random test
/// buffer hits often enough that a bug in the latch cannot hide. The needle is a
/// **constant**, not an input: an input needle would need an equality comparator
/// against a register rather than against a literal, which is the same gate count,
/// but it would also mean the design is not a fixed-function scanner.
pub const NEEDLE: u8 = b'\n';

/// The width of the position counter, in bits.
///
/// Eight, so `count` covers a 256-byte window and `offset` can hold any offset in it.
/// The counter **wraps**: it is a `+1` and nothing else, and a wrap makes `offset`
/// wrong for any buffer longer than 256 bytes. That is a real limitation and it is
/// stated here rather than hidden: a design that had to scan a megabyte would make
/// this a 32-bit register and nothing else would change. The tests bound their inputs
/// well below 256 bytes.
pub const COUNTER_BITS: u32 = 8;

/// The input ports.
#[derive(Clone, Debug, PortList)]
pub struct Inputs {
    /// The clock. Every edge consumes one byte.
    #[clock]
    pub clk: Signal,
    /// Synchronous reset, which clears the whole search: `found`, `offset` and
    /// `count` all return to zero.
    pub rst: Signal,
    /// Restart the search. Asserted for one edge, on which the byte on `data` is
    /// discarded; afterwards `count` counts from zero and `found` is low again.
    pub start: Signal,
    /// The next byte of the haystack, examined on each rising edge.
    #[bits(8)]
    pub data: Signal,
}

/// The output ports.
#[derive(Clone, Debug, PortList)]
pub struct Outputs {
    /// High once the needle has been seen. Sticky: it stays high until the next
    /// `start` or `rst`, because "found" is a fact about the buffer rather than about
    /// this cycle.
    pub found: Signal,
    /// The offset of the **first** occurrence of the needle, counted from the
    /// `start` that began this search. Meaningful only while `found` is high.
    #[bits(8)]
    pub offset: Signal,
    /// How many bytes have been consumed since the `start`. Wraps at 256; see
    /// [`COUNTER_BITS`].
    #[bits(8)]
    pub count: Signal,
}

/// The signals [`build`] returns, for a caller that wants them by handle.
#[derive(Clone, Debug)]
pub struct Ports {
    /// The input ports.
    pub inputs: Inputs,
    /// The sticky found flag, which is also the output.
    pub found: Signal,
    /// The latched first-match offset, which is also the output.
    pub offset: Signal,
    /// The byte counter, which is also the output.
    pub count: Signal,
}

/// Builds the single-byte searcher, with `hit` as an equality comparator.
///
/// See [`build_from_rom`] for the same design with `hit` as a 256-entry table, and
/// [`build_with`] for how they share everything else.
///
/// # Errors
///
/// Whatever [`Design`] returns -- a width mismatch or an undriven wire -- and whatever
/// the port-list helpers return. There is no memchr-specific failure: the needle is a
/// constant, so a different needle would be a different module rather than a
/// misconfigured one.
pub fn build(design: &Design) -> Result<Ports, BuildError> {
    build_with(design, |design, byte| {
        let needle = design.lit(u64::from(NEEDLE), 8)?;
        Ok(design.eq(byte, &needle)?)
    })
}

/// Builds the single-byte searcher, with `hit` as a 256-entry table.
///
/// Identical to [`build`] except for how `hit` is computed, which is the point: a test
/// builds both and checks they agree on every byte value, so "the ROM and the
/// comparator are the same function" is asserted rather than claimed.
///
/// # Errors
///
/// Whatever [`Design`] returns, and whatever [`match_rom`] returns.
pub fn build_from_rom(design: &Design) -> Result<Ports, BuildError> {
    build_with(design, match_rom)
}

/// Builds the searcher, given the function that decides whether a byte is a hit.
///
/// The counter, the latches and the ports are shared between the two formulations so
/// that the only difference between them is `hit`. That is what makes the comparison
/// between them a comparison of the table and nothing else.
///
/// # Errors
///
/// Whatever [`Design`] returns, and whatever `hit` returns.
pub fn build_with(
    design: &Design,
    hit: impl Fn(&Design, &Signal) -> Result<Signal, BuildError>,
) -> Result<Ports, BuildError> {
    let inputs = ferrite_lithic::inputs::<Inputs>(design)?;

    // All three state bits are read by the logic that computes their own next values,
    // and a register cannot be an input to itself, so they are wires driven from
    // registers afterwards. Same pattern, and same reason, as `crc3`'s state wire.
    let count = design.wire(COUNTER_BITS)?;
    let offset = design.wire(8)?;
    let found = design.wire(1)?;

    let next = step(
        design,
        &count,
        &offset,
        &found,
        &inputs.data,
        &inputs.start,
        hit,
    )?;

    let count_q = design.reg(
        &next.count,
        &inputs.clk,
        &inputs.rst,
        &design.constant(false),
    )?;
    design.drive(&count, &count_q)?;

    let offset_q = design.reg(
        &next.offset,
        &inputs.clk,
        &inputs.rst,
        &design.constant(false),
    )?;
    design.drive(&offset, &offset_q)?;

    let found_q = design.reg(
        &next.found,
        &inputs.clk,
        &inputs.rst,
        &design.constant(false),
    )?;
    design.drive(&found, &found_q)?;

    ferrite_lithic::outputs::<Outputs>(
        design,
        &Outputs {
            found: found_q.clone(),
            offset: offset_q.clone(),
            count: count_q.clone(),
        },
    )?;

    Ok(Ports {
        inputs,
        found: found_q,
        offset: offset_q,
        count: count_q,
    })
}

/// The three registers' next values after consuming one byte.
///
/// Split out from [`build_with`] so the arithmetic is visible on its own: the whole
/// state update is an increment, two muxes and an OR, and the *only* part that
/// depends on the algorithm is `hit`.
///
/// # Errors
///
/// Whatever [`Design`] returns, and whatever `hit` returns.
#[derive(Clone, Debug)]
pub struct Next {
    /// The next byte counter.
    pub count: Signal,
    /// The next latched offset.
    pub offset: Signal,
    /// The next sticky found flag.
    pub found: Signal,
}

/// One step of the search.
///
/// `hit` decides whether `data` is the needle. Then:
///
/// | condition | `count` | `offset` | `found` |
/// |---|---|---|---|
/// | `start` | 0 | 0 | 0 |
/// | `hit and not found` | +1 | `count` | 1 |
/// | otherwise | +1 | hold | hold |
///
/// The `not found` in the middle row is what makes the offset the **first** match's
/// rather than the most recent one, and it is one AND rather than a wider comparison
/// because `found` and `offset` are already separate registers. A scanner that wanted
/// the last match would drop that AND.
///
/// `start` beats everything else, which is why it is an `ite` around each of the three
/// rather than being folded into the update: the byte on the `start` edge is not part
/// of the new search, and a design that counted it would report offset 0 for a buffer
/// whose first byte is the needle.
///
/// # Errors
///
/// Whatever [`Design`] returns, and whatever `hit` returns.
pub fn step(
    design: &Design,
    count: &Signal,
    offset: &Signal,
    found: &Signal,
    data: &Signal,
    start: &Signal,
    hit: impl Fn(&Design, &Signal) -> Result<Signal, BuildError>,
) -> Result<Next, BuildError> {
    let hit = hit(design, data)?;

    let zero = design.lit(0, COUNTER_BITS)?;
    let one = design.lit(1, COUNTER_BITS)?;
    let stepped = design.add(count, &one)?;

    let fresh = design.and(&hit, &design.not(found))?;
    let candidate = design.ite(&fresh, count, offset)?;
    let keep_offset = design.ite(found, offset, &candidate)?;
    let keep_found = design.ite(found, found, &hit)?;

    let zero8 = design.lit(0, 8)?;
    let next_count = design.ite(start, &zero, &stepped)?;
    let next_offset = design.ite(start, &zero8, &keep_offset)?;
    let next_found = design.ite(start, &design.constant(false), &keep_found)?;

    Ok(Next {
        count: next_count,
        offset: next_offset,
        found: next_found,
    })
}

/// The 256-entry match ROM, as a [`Design::case_`].
///
/// One arm per byte value, and only [`NEEDLE`]'s arm is a match; the other 255 are
/// the same constant. That is a deliberately silly ROM and it is the honest one: the
/// table for "does this byte equal one fixed byte" has one bit of content, which is
/// exactly why [`build`] uses [`Design::eq`] instead.
///
/// # Errors
///
/// Whatever [`Design`] returns.
pub fn match_rom(design: &Design, byte: &Signal) -> Result<Signal, BuildError> {
    let mut arms = Vec::with_capacity(256);
    for value in 0..256u64 {
        let hit = u8::try_from(value).expect("the loop bounds a u8") == NEEDLE;
        arms.push((value, design.constant(hit)));
    }
    // Unreachable: `byte` is eight bits and all 256 values are enumerated above. The
    // IR requires a default for a case to have a value at all, and "no match" is the
    // only self-consistent choice for an arm that cannot be reached.
    let default = design.constant(false);
    Ok(design.case_(byte, &arms, &default)?)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::{COUNTER_BITS, NEEDLE};

    #[test]
    fn the_counter_is_eight_bits_and_the_docs_say_so() {
        // The wrap limitation in the module docs is only honest if the width really is
        // eight, so it is stated as an assertion as well.
        assert_eq!(COUNTER_BITS, 8);
        assert_eq!(u64::from(NEEDLE), 0x0a);
    }
}
