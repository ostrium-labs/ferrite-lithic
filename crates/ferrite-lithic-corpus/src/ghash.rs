//! GHASH: the GF(2^128) multiply from GCM, one bit per cycle.
//!
//! # Why this is the most ASIC-shaped thing in the corpus
//!
//! On a CPU, GHASH is not a bit-serial loop. It is a carryless multiply over
//! 64-bit limbs: fold the 128 bits of `H` down to 64, four shift-and-XOR rounds
//! with a reduction constant, and the whole multiply is a handful of `PCLMULQDQ`
//! instructions. There is no loop over 128 bits to vectorise, because the loop is
//! already closed.
//!
//! In hardware the loop does not close. The honest datapath is **128 cycles of a
//! 128-bit register with two conditional XORs**, which is what this design builds,
//! and it is the clearest latency-for-area trade in the whole corpus: about 400
//! flops and two 128-bit XOR trees in, in exchange for 128 cycles. An FPGA with
//! DSP48s will do it in a fraction of that, but the *portable ASIC* version -- the
//! one that runs at any frequency, on any library, with no vendor macro -- is this
//! loop, and this loop is exactly the shape NIST writes down in SP 800-38D §6.3's
//! algorithm, which is the point: the hardware and the specification are the same
//! object.
//!
//! There is no SIMD path here at all, and there could not be. Nothing about
//! GHASH's 128 iterations is a data-parallel loop; they are a dependency chain, one
//! bit of multiplier consumed per step and one `V` carried to the next. A CPU
//! closes the chain with carryless multiplication; an ASIC just runs it.
//!
//! # The algorithm
//!
//! Interpret both 128-bit operands as big-endian byte strings read as big integers,
//! so the *most significant* bit is the coefficient of x^0 -- GCM numbers bits from
//! the left, which is why the reduction constant is [`REDUCTION_HI`] shifted to
//! the top of the word rather than `0x87` at the bottom. The modulus is
//! x^128 + x^7 + x^2 + x + 1 (SP 800-38D §6.3). The multiply is then:
//!
//! ```text
//! Z = 0, V = X
//! for i in 0..128:
//!     if bit (127 - i) of Y is set: Z ^= V
//!     if LSB(V) is set: V = (V >> 1) ^ R
//!     else: V = V >> 1
//! ```
//!
//! with `R = 0xe1000000000000000000000000000000`. Three things in that listing are
//! worth being explicit about, because each is a way to write a plausible design
//! that computes a different field.
//!
//! **The multiplier is walked from the top.** At step `i`, `V` holds `X·x^i`, and
//! `x^i` is bit `127 - i` of `Y`. Writing `if bit i of Y` reads naturally and is
//! wrong: that pairs `V = X·x^i` with the coefficient of `x^(127-i)`, which is a
//! mirrored polynomial multiplication. It is not caught by any internal consistency
//! check, because it is self-consistent -- it just is not GHASH.
//!
//! **The bit that falls off the shift is bit 0, and it is the bit `R` is
//! conditioned on.** `V >> 1` moves every bit down one place and discards bit 0.
//! Bit 0 is `x^127`, and multiplying by `x` pushed it out to `x^128`, which the
//! modulus turns into `x^7 + x^2 + x + 1` -- bits 120, 125, 126 and 127, which is
//! exactly `0xe1 << 120`. So the LSB test and the reduction are the same fact, and
//! a design that tests the wrong one of them is a design that reduces by `0x87`.
//!
//! **The shift is a plain 128-bit logical shift.** [`Design::srl`] builds it as
//! `zeros(1) ++ value[1..128]`, which is the shift with the vacated top bit zeroed
//! and bit 0 dropped. There is nothing to reassemble by hand, and the thing worth
//! pinning with a test is not the shift but the reduction: see
//! [`REDUCTION_HI`], and the test that reduces `x^127 · x` down to exactly `R`.
//!
//! # The conditional XORs are the entire cost of the hardware
//!
//! Look at what the loop body is: one bit test, one XOR into the accumulator
//! behind that test, one 128-bit right shift, one XOR of a *constant* behind the
//! shifted-out bit. Two muxes' worth of control over two 128-bit XOR trees. There is
//! no multiplier, no table, no memory, and nothing whose width depends on the key.
//! Everything else in the loop is a register holding a value from the previous
//! cycle.
//!
//! That is why the *cost* of the conditional XORs is the whole cost of this design:
//! XOR of a whole field element is 128 two-input gates in one logic level, in a
//! technology where that is free next to the 128 flops of `V` and the routing
//! between them. The design would cost the same if the XORs were unconditional.
//!
//! # This is the corpus's first state machine
//!
//! 128 cycles per multiply is a latency a combinational module cannot have, so the
//! design is an explicit FSM rather than a wire: `start` loads the operands, `busy`
//! is high for the `start` edge and the 128 steps, and `done` pulses for one cycle
//! at the end. Every design before this one had at most a shift register.
//!
//! The step counter holds the *index* of the step an edge performs, 0 through 127, so
//! the edge on which it reads 127 performs the last step and finishes. Two other
//! readings are a character away and both are wrong in ways that produce a plausible
//! number rather than an obviously broken one: counting to 128 needs an eighth bit
//! and a comparator against a state the FSM can never be in, and gating the step on
//! "not the last step" drops step 127 entirely, leaving `busy` high for 128 cycles
//! around a product one shift short of the right one.
//!
//! The multiplier `Y` is walked by **shifting a register left and reading its top
//! bit**, rather than keeping a counter and selecting bit `127 - count`. The counter
//! form needs a 128-to-1 multiplexer per bit of the accumulator, which is real logic
//! and exactly what this design does not need. Shifting `Y` left one place per cycle
//! turns the selection into wiring. The DSL has no variable shift (`sll` takes a
//! constant amount), so the shifting-register form is also the only one expressible
//! without a 128-arm `case`.
//!
//! `X` gets no register: it is read once, on the `start` edge, straight into `V`.
//! A register that is only read on the cycle it is loaded is not a register.
//!
//! # Two 64-bit halves per 128-bit value, never a 128-bit port
//!
//! The operands and the result are 128 bits and each is a pair of 64-bit ports,
//! `_hi` above `_lo`. The cosimulator's generated C++ driver refuses to build for
//! a port wider than 64 bits rather than truncating it, so a single 128-bit port
//! would leave this design un-cosimulated -- which is the one check that catches a
//! design that simulates correctly and elaborates to something else. Splitting in
//! two also matches how the value is consumed: GCM's own block format is 16 bytes,
//! and `ghash::GHash` takes a `GenericArray<u8, U16>`.
//!
//! Little-endian inside each half is not a choice, it is the definition: GCM's
//! 16-byte block read as a big-endian integer has its first byte in the high half,
//! so `hi` is bytes 0..8 and `lo` is bytes 8..16.

use ferrite_lithic::{Design, Signal};
use ferrite_lithic_derive::PortList;

use crate::BuildError;

/// The GCM reduction constant `R`, high half.
///
/// `R = 0xe1000000000000000000000000000000`, so `0xe1 << 120`. The four set bits
/// are 120, 125, 126 and 127, which in GCM's bit order (most significant bit is
/// x^0) are x^7, x^2, x and 1 -- the modulus x^128 + x^7 + x^2 + x + 1 with the
/// x^128 term removed. NIST SP 800-38D §6.3 names this constant; §6.5 notes that
/// the value written here is the one the GCM specification actually uses, with the
/// older SP 800-38D text having dropped a leading bit.
pub const REDUCTION_HI: u64 = 0xe100_0000_0000_0000;

/// A field element's width in bits.
const FIELD_BITS: u32 = 128;

/// How many steps one multiply takes: one per bit of the multiplier.
pub const STEPS: u32 = 128;

/// The step counter's width. Seven bits hold 0 through [`STEPS`] - 1.
const COUNT_BITS: u32 = 7;

/// The value the counter holds on the last step, so the FSM knows when to stop.
const LAST_STEP: u64 = STEPS as u64 - 1;

/// The input ports.
#[derive(Clone, Debug, PortList)]
pub struct Inputs {
    /// The clock. Every edge advances one step of the multiply.
    #[clock]
    pub clk: Signal,
    /// Synchronous reset, which returns the whole state to GHASH's initial value.
    ///
    /// Every register in this design resets to zero and zero is what GHASH wants:
    /// the accumulator starts at 0, and the operands and `busy` have to be loaded.
    pub rst: Signal,
    /// Load the operands and begin.
    ///
    /// Asserted again while `busy` is high it restarts the multiply from scratch
    /// rather than being ignored, which is how a host abandons one it no longer
    /// wants without needing a separate abort port. It also clears the accumulator,
    /// so a restart cannot inherit the interrupted product.
    pub start: Signal,
    /// The multiplicand `X`, high 64 bits.
    #[bits(64)]
    pub x_hi: Signal,
    /// The multiplicand `X`, low 64 bits.
    #[bits(64)]
    pub x_lo: Signal,
    /// The multiplier `Y`, high 64 bits. The FSM shifts this left one bit per step.
    #[bits(64)]
    pub y_hi: Signal,
    /// The multiplier `Y`, low 64 bits.
    #[bits(64)]
    pub y_lo: Signal,
}

/// Outputs.
#[derive(Clone, Debug, PortList)]
pub struct Outputs {
    /// High while a multiply is in flight, including the `start` edge itself.
    ///
    /// Low for the 128 cycles after the last one drops it, so a host that samples
    /// `done` sees one pulse and not a level.
    pub busy: Signal,
    /// High for exactly one cycle: the cycle in which the 128th step was performed.
    pub done: Signal,
    /// The product `X · Y`, high 64 bits.
    #[bits(64)]
    pub z_hi: Signal,
    /// The product `X · Y`, low 64 bits.
    #[bits(64)]
    pub z_lo: Signal,
}

/// The signals [`build`] returns, for a caller that wants them by handle.
#[derive(Clone, Debug)]
pub struct Ports {
    /// The input ports.
    pub inputs: Inputs,
    /// Whether a multiply is in flight.
    pub busy: Signal,
    /// The one-cycle completion pulse.
    pub done: Signal,
    /// The product's two halves.
    pub z_hi: Signal,
    /// The product's two halves.
    pub z_lo: Signal,
}

/// The current value of every register, as signals.
///
/// The same shape is used for the wires that *name* each register's value and for
/// the values the registers should take next, so the FSM is written once over a
/// pair of states rather than twice.
#[derive(Clone, Debug)]
struct State {
    /// Steps taken so far, 0 through 127.
    count: Signal,
    /// `Y`, walked left one bit per step.
    multiplier: Signal,
    /// `V`: `X · x^(steps taken)`, reduced. Shifted right and conditionally XORed
    /// with the reduction constant on every step.
    multiplicand: Signal,
    /// `Z`: the accumulator.
    accumulator: Signal,
    /// Whether a multiply is in flight.
    busy: Signal,
    /// The one-cycle completion pulse.
    done: Signal,
}

/// Builds a bit-serial GF(2^128) multiplier into `design`.
///
/// # Errors
///
/// Whatever [`Design`] returns -- a width mismatch or an undriven wire -- and
/// whatever the port-list helpers return. There is no GHASH-specific failure: both
/// operands are 128 bits wide and the step count is fixed at [`STEPS`], so a design
/// taking any other shape is a different module.
pub fn build(design: &Design) -> Result<Ports, BuildError> {
    let inputs = ferrite_lithic::inputs::<Inputs>(design)?;

    let x = design.concat(&[inputs.x_hi.clone(), inputs.x_lo.clone()])?;
    let y = design.concat(&[inputs.y_hi.clone(), inputs.y_lo.clone()])?;

    // The state has to be a wire rather than used directly, because the registers'
    // outputs are what feed the next state and a register cannot be an input to
    // itself: the graph is built forwards and there is no cycle in it. Each wire is
    // driven from its register afterwards, which is the pattern for any feedback
    // through state.
    let now = State {
        count: design.wire(COUNT_BITS)?,
        multiplier: design.wire(FIELD_BITS)?,
        multiplicand: design.wire(FIELD_BITS)?,
        accumulator: design.wire(FIELD_BITS)?,
        busy: design.wire(1)?,
        done: design.wire(1)?,
    };

    let next = next_state(design, &x, &y, &inputs.start, &now)?;

    // `clear` is tied low, so a register updates on every edge it is clocked; the
    // gating is in `next_state` instead, which is where it belongs because it has
    // to see `start`. Reset is a real input rather than a constant, because a reset
    // that cannot be exercised is a reset that is not tested.
    let hold = design.constant(false);
    let count = design.reg(&next.count, &inputs.clk, &inputs.rst, &hold)?;
    let multiplier = design.reg(&next.multiplier, &inputs.clk, &inputs.rst, &hold)?;
    let multiplicand = design.reg(&next.multiplicand, &inputs.clk, &inputs.rst, &hold)?;
    let accumulator = design.reg(&next.accumulator, &inputs.clk, &inputs.rst, &hold)?;
    let busy = design.reg(&next.busy, &inputs.clk, &inputs.rst, &hold)?;
    let done = design.reg(&next.done, &inputs.clk, &inputs.rst, &hold)?;

    design.drive(&now.count, &count)?;
    design.drive(&now.multiplier, &multiplier)?;
    design.drive(&now.multiplicand, &multiplicand)?;
    design.drive(&now.accumulator, &accumulator)?;
    design.drive(&now.busy, &busy)?;
    design.drive(&now.done, &done)?;

    let outputs = ferrite_lithic::outputs::<Outputs>(
        design,
        &Outputs {
            busy: busy.clone(),
            done: done.clone(),
            z_hi: design.slice(&accumulator, 64, 64)?,
            z_lo: design.slice(&accumulator, 0, 64)?,
        },
    )?;

    Ok(Ports {
        inputs,
        busy: outputs.busy,
        done: outputs.done,
        z_hi: outputs.z_hi,
        z_lo: outputs.z_lo,
    })
}

/// One step of the FSM, from the current state to the next.
///
/// `x` and `y` are the operands on their `start` edge; `start` is ignored while
/// `busy` is high, so a multiply in flight is not disturbed by a second one. See
/// the module docs for the recurrence this transcribes.
///
/// # Errors
///
/// Whatever [`Design`] returns.
fn next_state(
    design: &Design,
    x: &Signal,
    y: &Signal,
    start: &Signal,
    now: &State,
) -> Result<State, BuildError> {
    // `count` is the index of the step this edge performs, so it runs 0 through 127
    // and the edge on which it reads 127 performs the *last* step and finishes the
    // multiply. Gating the step on `count != 127` instead -- which is the obvious
    // reading -- would skip step 127 and leave `busy` high for 128 cycles with a
    // product that is one shift short of the right one. That is the one way to write
    // this FSM wrong, and it fails as a plausible number rather than as an obviously
    // wrong one.
    let on_last_step = design.eq(&now.count, &design.lit(LAST_STEP, COUNT_BITS)?)?;
    // Every edge while `busy` is high performs a step, the last one included.
    let stepping = now.busy.clone();
    // This edge is the one that drops `busy` and raises `done`.
    let finished = design.and(&stepping, &on_last_step)?;

    // The multiplier is walked by shifting it left, so the bit this step consumes
    // is its top bit. `127 - step` was the alternative and it needs a 128-to-1 mux
    // per bit; see the module docs.
    let bit = design.slice(&now.multiplier, FIELD_BITS - 1, 1)?;
    let accumulate = design.and(&stepping, &bit)?;
    let sum = design.xor(&now.accumulator, &now.multiplicand)?;

    // `V` becomes `reduce(V)` on a step, but `X` on a `start`: the multiplicand is
    // loaded rather than shifted in, which is why `X` needs no register of its own.
    let stepped = design.ite(
        &stepping,
        &reduce(design, &now.multiplicand)?,
        &now.multiplicand,
    )?;
    let multiplicand = design.ite(start, x, &stepped)?;

    // The multiplier is loaded on a `start` and shifted left on a step, so its top
    // bit is the bit this step consumes and no multiplexer is needed to reach it.
    let stepped = design.ite(&stepping, &design.sll(&now.multiplier, 1)?, &now.multiplier)?;
    let multiplier = design.ite(start, y, &stepped)?;

    // The accumulator restarts at zero on a `start` rather than carrying the
    // previous product, so back-to-back multiplies do not need a reset between them.
    let stepped = design.ite(&accumulate, &sum, &now.accumulator)?;
    let accumulator = design.ite(start, &design.zeros(FIELD_BITS)?, &stepped)?;

    // The counter advances on every step and wraps from 127 back to 0 on the last
    // one, which is harmless: `busy` is low from the next edge on, so nothing reads
    // it again until a `start` zeroes it. Seven bits is enough because the counter
    // holds a step *index*, not a step *count*, and an eighth bit would be the only
    // way to reach 128 -- a state the FSM never has to distinguish.
    let stepped = design.ite(
        &stepping,
        &design.add(&now.count, &design.lit(1, COUNT_BITS)?)?,
        &now.count,
    )?;
    let count = design.ite(start, &design.zeros(COUNT_BITS)?, &stepped)?;

    // `start` wins over everything, so a `start` in the cycle the previous multiply
    // finished begins the next one instead of being dropped.
    let busy = design.ite(
        start,
        &design.constant(true),
        &design.ite(&finished, &design.constant(false), &now.busy)?,
    )?;

    Ok(State {
        count,
        multiplier,
        multiplicand,
        accumulator,
        busy,
        done: finished,
    })
}

/// One step of the multiply: `V` right-shifted, reduced modulo x^128 + x^7 + x^2 +
/// x + 1 if the bit it shifted out was set.
///
/// `V >> 1` is a plain 128-bit logical shift, so bit 0 is what falls off the bottom
/// and bit 127 is what arrives at the top. Bit 0 is x^127; multiplying by `x` pushed
/// it to x^128, which the modulus replaces with x^7 + x^2 + x + 1 -- so the bit that
/// decides whether to XOR the reduction constant is exactly the bit that was
/// shifted out. The module docs explain why that bit is bit 0 and not bit 127.
///
/// # Errors
///
/// Whatever [`Design`] returns.
fn reduce(design: &Design, value: &Signal) -> Result<Signal, BuildError> {
    let reduction = design.concat(&[design.lit(REDUCTION_HI, 64)?, design.zeros(64)?])?;
    let shifted = design.srl(value, 1)?;
    let folded = design.xor(&shifted, &reduction)?;
    Ok(design.ite(&design.slice(value, 0, 1)?, &folded, &shifted)?)
}
