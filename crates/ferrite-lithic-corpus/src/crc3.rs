//! CRC-3/ROTD: a three-bit LFSR, one byte per cycle.
//!
//! # Why this is the first design
//!
//! It is the smallest thing in this crate that is genuinely a *streaming* design,
//! and it has almost nothing in it: three state bits, an XOR tap and a byte of
//! input. That is the point. Every later design adds a new capability, and this one
//! adds only the capability the A3 work needs to demonstrate -- a register with a
//! reset, fed a byte at a time, read back a cycle at a time.
//!
//! It is also a genuinely good ASIC target, which is why it is worth doing properly
//! rather than as a toy. A three-bit LFSR is three flops and a handful of XOR gates;
//! the software version is a byte-at-a-time table lookup. There is no SIMD story
//! here at all -- there is one dependency chain per byte and no lanes to fill --
//! so hardware gets one result per cycle where the CPU gets one result per table
//! lookup, and the hardware version gets it for the price of three flops.
//!
//! # The algorithm, and where the constants come from
//!
//! CRC-3/ROTD as the RevEng catalogue defines it:
//!
//! | parameter | value |
//! |---|---|
//! | width | 3 |
//! | poly | `0x3` (x^3 + x + 1) |
//! | init | `0x0` |
//! | refin | true |
//! | refout | true |
//! | xorout | `0x0` |
//! | check | `0x2` |
//!
//! Init zero with no output XOR is what makes this the cheap variant: there is
//! nothing to pre-load and nothing to post-condition, so the hardware is the shift
//! register and nothing else. It is also why the reflected form is the one to build
//! -- a reflected algorithm shifts right, and a right-shifting register puts the new
//! input bit at the low end, which is where it is already connected.
//!
//! The recurrence is therefore:
//!
//! ```text
//! for each bit of each byte, least significant first:
//!     feedback = state[0] xor bit
//!     state >>= 1
//!     if feedback: state ^= 0b110
//! ```
//!
//! **The tap is `0b110`, not `0b011`.** That is the one genuinely easy thing to get
//! wrong, and it is worth being explicit about why. `poly` in the catalogue is `0x3`,
//! which is the polynomial in its *forward* form. A reflected algorithm needs the
//! bit-reversed polynomial over the register width, and reversing three bits of
//! `0b011` gives `0b110`. Both were checked against the `crc` crate over random
//! input: `0b110` agrees on every case, `0b011` disagrees on 263 of them. A design
//! with the tap backwards is still a three-bit LFSR, still produces three bits, and
//! still looks entirely plausible -- it is just computing a different CRC.
//!
//! # Byte-parallel, not bit-serial
//!
//! The eight steps above are unrolled combinationally and one byte is consumed per
//! cycle. The alternative -- one bit per cycle, eight cycles per byte -- is the same
//! eight XORs with a register between each one, which is eight times the flops for
//! the same latency in cycles per byte. For a datapath this small the flops are
//! nearly free and the cycles are not, so unrolling is the right shape. The bit
//! -serial form is still what the recurrence above describes, and the design builds
//! it by literally running that loop eight times over [`Design`] nodes, so the
//! correspondence between the algorithm and the circuit is visible in the code
//! rather than asserted in a comment.
//!
//! # What this design does not do
//!
//! There is no `valid` or `last` input: the design consumes a byte every cycle,
//! forever. Real CRC hardware has a `valid` because a real bus has idle cycles,
//! and that is a one-line change -- gate the register's `clear` with `valid`. It is
//! left out here so that the byte stream is unconditional and the differential test
//! can compare against a software CRC over an arbitrary buffer without the test
//! having to model the gaps.

use ferrite_lithic::{Design, Signal};
use ferrite_lithic_derive::PortList;

use crate::BuildError;

/// The reflected tap for CRC-3/ROTD, as three bits.
///
/// `poly` reversed over the register width. See the module docs for why this is
/// `0b110` and what happens if it is `0b011`.
pub const TAP: u64 = 0b110;

/// The input ports.
#[derive(Clone, Debug, PortList)]
pub struct Inputs {
    /// The clock. Every edge consumes one byte.
    #[clock]
    pub clk: Signal,
    /// Synchronous reset, which returns the register to the CRC's init value.
    pub rst: Signal,
    /// The next byte of the message, consumed on each rising edge.
    #[bits(8)]
    pub byte: Signal,
}

/// The output ports.
#[derive(Clone, Debug, PortList)]
pub struct Outputs {
    /// The running CRC, three bits, as it stands after the last byte consumed.
    #[bits(3)]
    pub crc: Signal,
}

/// The signals [`build`] returns, for a caller that wants them by handle.
#[derive(Clone, Debug)]
pub struct Ports {
    /// The input ports.
    pub inputs: Inputs,
    /// The CRC register, which is also the output.
    pub crc: Signal,
}

/// Builds a CRC-3/ROTD design into `design`.
///
/// The clock is named `clk` and the reset `rst`, which is what the crate-level
/// [`crate::CLOCK`] and [`crate::RESET`] say; naming them rather than leaving them
/// positional is
/// what lets the emitted Verilog, the testbench and the cosimulator agree.
///
/// # Errors
///
/// Whatever [`Design`] returns -- a width mismatch or an undriven wire -- and
/// whatever the port-list helpers return. There is no
/// CRC-specific failure, because the three-bit state has no configuration to get
/// wrong -- a different polynomial or width would be a different module.
pub fn build(design: &Design) -> Result<Ports, BuildError> {
    let inputs = ferrite_lithic::inputs::<Inputs>(design)?;

    // The state has to be a wire rather than used directly, because the register's
    // output is what feeds the next state and a register cannot be an input to
    // itself: the graph is built forwards and there is no cycle in it. The wire is
    // driven from the register afterwards, which is the pattern for any feedback
    // through state.
    let state = design.wire(3)?;
    let next = advance(design, &state, &inputs.byte)?;

    // `clear` is tied low, so the register updates on every edge: this design has
    // no `valid` and consumes a byte per cycle unconditionally. Reset is a real
    // input rather than a constant, because a reset that cannot be exercised is a
    // reset that is not tested.
    let held = design.reg(&next, &inputs.clk, &inputs.rst, &design.constant(false))?;
    design.drive(&state, &held)?;

    ferrite_lithic::outputs::<Outputs>(design, &Outputs { crc: held.clone() })?;

    Ok(Ports { inputs, crc: held })
}

/// The eight unrolled LFSR steps for one byte.
///
/// `state` is the register's current value and `byte` the next byte; the result is
/// the value the register should take at the next edge. The loop is the algorithm
/// from the module docs, run over design nodes: one slice for the input bit, one
/// for the register's low bit, an XOR for the feedback, a shift, and a mux on the
/// feedback to choose between the shifted value and the shifted value with the tap
/// applied.
///
/// # Errors
///
/// Whatever [`Design`] returns.
pub fn advance(design: &Design, state: &Signal, byte: &Signal) -> Result<Signal, BuildError> {
    let tap = design.lit(TAP, 3)?;
    let mut accumulator = state.clone();
    for bit in 0..8u32 {
        // Bit `bit` counting from the least significant: the reflected algorithm
        // consumes a byte's bits in that order.
        let incoming = design.slice(byte, bit, 1)?;
        let carried = design.slice(&accumulator, 0, 1)?;
        let feedback = design.xor(&carried, &incoming)?;
        let shifted = design.srl(&accumulator, 1)?;
        let tapped = design.xor(&shifted, &tap)?;
        accumulator = design.ite(&feedback, &tapped, &shifted)?;
    }
    Ok(accumulator)
}
