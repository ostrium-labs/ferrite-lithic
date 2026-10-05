//! DEFLATE's bit layer: packed bits out of a byte stream, least significant bit
//! first.
//!
//! # Which half of DEFLATE this module is, and which half it is not
//!
//! This is option **(a)**: the bit layer. It converts a byte stream into the bit
//! sequence every other part of the format is written in, and it does nothing else.
//! It is deliberately *not* option (b), a fixed-Huffman block decode, and the reason
//! is worth stating rather than hiding: a fixed-Huffman block still needs the Huffman
//! decode *and* LZ77's back-reference copy out of a 32 KiB window, and the window
//! needs a memory with a write port and a variable-offset read port. Building that
//! here without a verified dynamic-tree path would have produced a decoder that
//! round-trips the cases I tried and nothing else, which is worse than not shipping
//! it. [`crate::huffman`] is the Huffman half of this format, built and checked
//! separately; the window is not built at all.
//!
//! # The bit order, and why it is the whole content of this module
//!
//! RFC 1951 section 3.1.1:
//!
//! ```text
//! Data elements are packed into bytes in order of increasing bit number within the
//! byte, i.e. starting with the least significant bit of the byte.
//! ```
//!
//! So byte `k`'s bit 0 goes first, then bit 1, up to bit 7, then byte `k + 1`'s bit 0.
//! Every multi-bit field in DEFLATE is then read **least significant bit first**, so a
//! field of `n` bits is `sum over i of bit(i) << i` over the `n` bits that follow.
//! There is one exception in the whole format: Huffman codes are packed **most
//! significant bit first** into that same LSB-first stream -- which is why a Huffman
//! decoder has to reverse each code before looking it up, and why
//! [`crate::huffman`] puts the next code bit in the *high* end of its peek.
//!
//! Getting this backwards is the failure mode that matters here: a design that peels
//! bits off the top of a byte produces a perfectly plausible bit stream that no
//! DEFLATE decoder has ever accepted.
//!
//! # The hardware shape, and the throughput it costs
//!
//! ```text
//!   byte in --> [ 8-bit shift register + 4-bit count ] --> bit out, one per cycle
//! ```
//!
//! Nine cycles per byte: eight to shift the bits out and one to take the next byte,
//! because the design will not load a byte over bits it has not emitted yet. That is
//! the honest cost of the simplest possible shape, and it is a real number to quote:
//!
//! | shape | cycles per byte |
//! |---|---|
//! | this design: one staging register, load only when empty | **9** |
//! | two staging registers, prefetch the next byte behind the current one | 8 |
//! | 64-bit refill buffer loaded a byte per cycle from a memory port | 1 |
//! | a CPU, reading `stream[byte] >> bit & 1` | about one, but eight times a byte and with no fixed latency |
//!
//! The third row is what a real inflate does and it needs a memory port and a
//! variable-position insert. The DSL's `sll`/`srl` take a *constant* amount, so a
//! 64-bit buffer with a variable fill level would need a barrel shifter for the insert
//! that this module does not build; the decode table in [`crate::huffman`] would need
//! the same one. Building the ninth-cycle version and saying so is better than building
//! a fast version whose fast part is unverified.
//!
//! # The tables in the sibling modules, restated for this one
//!
//! There is no table here at all -- that is the point of choosing the bit layer. The
//! mux trees that *do* exist are [`crate::huffman`]'s 512-entry decode table and
//! [`crate::fse`]'s 32-entry state table, both built as [`Design::case_`] blocks
//! because the IR has no initialised-memory node. See those modules for the area
//! comparison against block RAM; it is worth repeating here because a reader who only
//! looks at this module would not otherwise know.
//!
//! # What this module does not do
//!
//! * **No block structure.** BFINAL, BTYPE, the stored-block LEN/NLEN pair, the Huffman
//!   trees, the extra bits and the LZ77 lengths are all *above* this layer. The design
//!   has no notion of a block and will happily emit every bit it is given.
//! * **No error detection.** A truncated stream produces fewer bits, not a flag.
//! * **No variable-width field reader.** Field extraction is a consumer's job; see the
//!   test file, where the reference field reader consumes this design's output.

use ferrite_lithic::{Design, Signal};
use ferrite_lithic_derive::PortList;

use crate::BuildError;

/// A width that holds a count from zero to eight.
const LEFT_BITS: u32 = 4;

/// The input ports.
#[derive(Clone, Debug, PortList)]
pub struct Inputs {
    /// The clock. Every edge either takes a byte or shifts one bit out.
    #[clock]
    pub clk: Signal,
    /// Synchronous reset, which drops any partly consumed byte.
    pub rst: Signal,
    /// Start a new byte stream: drops any partly consumed byte so the next byte
    /// presented is the first one.
    pub init: Signal,
    /// A byte is available on [`Self::in_byte`].
    pub in_valid: Signal,
    /// The next byte of the compressed stream.
    #[bits(8)]
    pub in_byte: Signal,
    /// Discard bits until the next byte boundary, without emitting them.
    ///
    /// RFC 1951 section 3.2.4's "skip any remaining bits in the partially processed
    /// byte" before a stored block's LEN. Holding this high empties the staging
    /// register; [`Outputs::in_ready`] then goes high and the next byte is taken at a
    /// boundary.
    pub align: Signal,
}

/// The output ports.
#[derive(Clone, Debug, PortList)]
pub struct Outputs {
    /// The staging register is empty, so a byte will be taken this cycle.
    ///
    /// High on the cycle the design loads a byte and low for the eight cycles it spends
    /// shifting that byte out, which is why the byte rate is one per nine cycles.
    pub in_ready: Signal,
    /// A bit is on [`Self::out_bit`].
    ///
    /// Low while [`Inputs::align`] is discarding bits: those bits leave the register but
    /// are not part of the stream any more.
    pub out_valid: Signal,
    /// The next bit of the stream: bit zero of the current byte, then bit one, and so on
    /// up to bit seven, then the next byte's bit zero.
    pub out_bit: Signal,
    /// This is the last bit of a byte, so a consumer knows when it is byte aligned.
    pub out_last: Signal,
    /// How many bits of the staging register are still to come out.
    #[bits(4)]
    pub bits_left: Signal,
}

/// The signals [`build`] returns, for a caller that wants them by handle.
#[derive(Clone, Debug)]
pub struct Ports {
    /// The input ports.
    pub inputs: Inputs,
    /// The output ports.
    pub outputs: Outputs,
}

/// The design's registers, as one value each.
#[derive(Clone, Debug)]
struct State {
    staging: Signal,
    left: Signal,
}

/// Builds a DEFLATE bit reader into `design`.
///
/// The clock is named `clk` and the reset `rst`, which is what the crate-level
/// [`crate::CLOCK`] and [`crate::RESET`] say.
///
/// # Errors
///
/// Whatever [`Design`] returns and whatever the port-list helpers return. There is
/// nothing to configure: DEFLATE's bit order is a constant of the format.
///
/// ```
/// use ferrite_lithic::Design;
/// use ferrite_lithic_corpus::deflate::build;
///
/// let design = Design::new();
/// build(&design).unwrap();
/// assert!(design.node_count() > 0);
/// ```
pub fn build(design: &Design) -> Result<Ports, BuildError> {
    let inputs = ferrite_lithic::inputs::<Inputs>(design)?;

    let now = State {
        staging: design.wire(8)?,
        left: design.wire(LEFT_BITS)?,
    };

    let next = next_state(design, &inputs, &now)?;

    let hold = design.constant(false);
    let staging = design.reg(&next.staging, &inputs.clk, &inputs.rst, &hold)?;
    let left = design.reg(&next.left, &inputs.clk, &inputs.rst, &hold)?;

    design.drive(&now.staging, &staging)?;
    design.drive(&now.left, &left)?;

    // `in_ready` is a pure function of the count register, so it is a wire rather than a
    // registered copy: a registered copy would answer one edge late, which is the edge
    // the host needs the answer on.
    let empty = design.eq(&now.left, &design.lit(0, LEFT_BITS)?)?;
    let in_ready = empty.clone();

    let outputs = ferrite_lithic::outputs::<Outputs>(
        design,
        &Outputs {
            in_ready,
            out_valid: design.ite(&inputs.align, &design.constant(false), &design.not(&empty))?,
            out_bit: design.slice(&staging, 0, 1)?,
            out_last: design.eq(&now.left, &design.lit(1, LEFT_BITS)?)?,
            bits_left: left.clone(),
        },
    )?;

    Ok(Ports { inputs, outputs })
}

/// One cycle of the bit reader.
///
/// Loading a byte and shifting a bit out are mutually exclusive: a byte is taken only
/// when the count is zero, and the count is only nonzero while bits are outstanding.
/// That is the whole FSM -- there is no third case -- and it is why the byte rate is one
/// per nine cycles rather than one per eight.
///
/// # Errors
///
/// Whatever [`Design`] returns.
fn next_state(design: &Design, inputs: &Inputs, now: &State) -> Result<State, BuildError> {
    let empty = design.eq(&now.left, &design.lit(0, LEFT_BITS)?)?;
    let loading = design.and(&inputs.in_valid, &empty)?;

    // Shift one bit out of the low end. Bit zero of a byte is the first bit DEFLATE
    // emits, which is the whole reason this shift is to the right.
    let shifted = design.srl(&now.staging, 1)?;
    let staging = design.ite(&empty, &inputs.in_byte, &shifted)?;
    let decremented = design.sub(&now.left, &design.lit(1, LEFT_BITS)?)?;
    let shifted_count = design.ite(&empty, &now.left, &decremented)?;

    let left = design.ite(&loading, &design.lit(8, LEFT_BITS)?, &shifted_count)?;
    let staging = design.ite(&inputs.init, &design.zeros(8)?, &staging)?;
    let left = design.ite(&inputs.init, &design.zeros(LEFT_BITS)?, &left)?;

    Ok(State { staging, left })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::LEFT_BITS;

    #[test]
    fn the_count_register_holds_every_value_the_count_takes() {
        // `left` runs 0, 8, 7, ... 1 and back to 0. Four bits is the narrowest width that
        // holds 8, and one bit narrower would wrap the load to zero, so the design would
        // emit a byte's worth of zeros instead of the byte.
        assert!(8 < (1u32 << LEFT_BITS));
        for value in 0..=8u32 {
            assert_eq!(value, value & ((1u32 << LEFT_BITS) - 1));
        }
    }
}
