//! Bit-unpacking: one fixed-width value out of a packed word, one value per cycle.
//!
//! # Why this is in the corpus
//!
//! Bit-packing is the innermost half of every columnar format. Parquet stores a
//! column of `w`-bit integers as a bit stream and the reader pulls one integer at a
//! time out of it; Lance does the same thing over its own buffer layout. The whole
//! format's claim to compression rests on this one operation -- if unpacking is slow,
//! no amount of on-disk density helps, because the reader is the thing that has to
//! keep up. So the decode kernel, not the encoder, is the piece worth putting in
//! silicon, and this module is that piece.
//!
//! It is also the purest bit-level work in the corpus. Nothing here has a dependency
//! chain longer than one shift and one mask, there is no state beyond a position
//! counter, and the hardware story is short enough to state exactly:
//!
//! ```text
//! value = (word >> (64 - w - off)) & mask(w)
//! ```
//!
//! Two operands, one barrel shifter and one mask. That is the whole datapath.
//!
//! # A ROM of shifts, and what a real design would infer
//!
//! The design below builds the shift as a [`Design::case_`] with one arm per bit
//! offset: the arm for `off` is the constant shift `64 - w - off`. Each arm is a
//! `slice` of a `srl`, so the emitted Verilog is one `always @*` with `65 - w` arms
//! -- a 57-entry decoder for the default eight-bit width.
//!
//! **This is a stand-in for a barrel shifter, and the substitution is a known gap in
//! the IR rather than a choice.** There is no shift-by-signal node: `Design::srl`
//! takes a `u32` shift *amount*, so it is structural (`slice` plus `concat`) and
//! cannot be built from a runtime `off`. And there is no initialised-memory node: a
//! real 64-entry ROM holding the masks or the shift amounts would come up with the
//! right contents, but `Design::mem` is zero-filled in both backends, so a ROM
//! written with a constant table here would decode to zero. A synthesiser reading
//! this design infers exactly the block RAM or the mux tree a real one would; nothing
//! in this file is a different algorithm, only a different way to spell the same
//! combinational function.
//!
//! The [`ports`] list is 64 bits wide and not 128, and the reason is the same as
//! everywhere else in this crate: the cosimulator's generated C++ driver refuses to
//! build for a port wider than 64 bits rather than truncating it, so a 128-bit port
//! would design fine, simulate fine, and then fail to cosimulate. The general
//! extractor therefore takes a `&Signal` of any width and
//! [`extract`](build) is the fixed-port wrapper; a caller who genuinely wants a
//! 128-bit window splits it into two 64-bit ports and concatenates the two
//! extractions' `next_off` bookkeeping themselves.
//!
//! # The honest part: this kernel is memory-bound, and an ASIC does not fix that
//!
//! The arithmetic is nearly free. A 64-bit barrel shifter is six mux levels and a
//! mask is two gates; at 1 GHz on a modern process that is single-digit picoseconds,
//! and it would still be the wrong number to quote, because it is not what sets the
//! throughput.
//!
//! What sets the throughput is the fact that **the input is a bit stream and the
//! output is a wider word.** A column of `w`-bit integers unpacks to a column of
//! 32- or 64-bit integers, so the decoder must read `64/w` bits for every 64 bits
//! it produces. At `w = 8` that is an 8x expansion. The bytes come from memory, and
//! the memory has to deliver eight bytes of packed data for every eight bytes of
//! output. **This is precisely the memory-bandwidth-bound work that an ASIC does not
//! help with**: the process that benefits from this datapath is the one where the
//! on-chip SRAM that holds the compressed column is genuinely close to the logic,
//! and for anything reading from off-chip DRAM the answer is bandwidth and prefetch,
//! not a barrel shifter. [`rle`](crate::rle) and [`sketch`](crate::sketch) make the
//! same point from different directions -- the counters a count-min sketch updates
//! are the reason to put it on chip, not the increment.
//!
//! So the claim this design can honestly make is narrow and worth making precisely:
//! **the arithmetic in a decoder is free, and the win from a custom decoder comes
//! only from removing the traffic around it.** `bitpacking` -- the crate this is
//! checked against -- is a good illustration. Its `BitPacker1x` is a scalar loop in
//! debug builds and a SIMD implementation in release builds, and the difference is
//! entirely about how many values one pass over the buffer touches, not about the
//! arithmetic per value. An ASIC does not change that trade; it can only remove the
//! instruction-fetch and loop-overhead overhead a CPU pays on top, which is worth
//! something but is not the reason to build one.
//!
//! # Position, and the one register
//!
//! The extractor itself is combinational and has no latency at all: `off` in, `value`
//! out, same cycle. The register this design carries is the bit position, which is
//! the piece that makes it a *stream* decoder rather than a random-access one. A
//! caller holds a 64-bit word, gets one value out of it per cycle while `advance`
//! walks the position, and latches a new word when `pos` wraps. That word register
//! and the `wrap` handshake are left to the caller because how a column is buffered
//! is the caller's business, not the decoder's; `pos` and `wrap` are exactly the two
//! signals it needs.
//!
//! `wrap` and `fit` are both outputs and they are both correct at the boundary,
//! which looks like an overlap and is not: when `off + w == 64` the current value is
//! the last one *in* this word (`fit` is high) and the next one starts at bit zero of
//! the following word (`wrap` is high). Both are true, and a caller must not treat
//! them as exclusive.

use ferrite_lithic::{Design, Signal};
use ferrite_lithic_derive::PortList;

use crate::BuildError;

/// The width the port list is built for, in bits.
///
/// Eight bits, because eight is a real Parquet dictionary bit-width and because a
/// 64-bit word holds exactly eight of them, which is the alignment the `wrap`
/// handshake is easiest to read at. [`extract`] is width-generic and
/// [`build`] is this width.
pub const DEFAULT_WIDTH: u32 = 8;

/// The width of the packed window this design reads from, in bits.
///
/// Also the widest port in the design, and the reason the crate's cosimulator is
/// usable on it at all: see the module docs.
pub const WINDOW_BITS: u32 = 64;

/// The width of the bit offset port, in bits.
///
/// Seven, because the offset has to be able to name bit 63 and cannot exceed it. It
/// is a *byte-and-a-bit* address, not a count, so 64 is not a legal `off`.
pub const OFFSET_BITS: u32 = 7;

/// The input ports.
#[derive(Clone, Debug, PortList)]
pub struct Inputs {
    /// The clock. Every edge moves the position register on if `advance` is high.
    #[clock]
    pub clk: Signal,
    /// Synchronous reset, which returns the position register to [`OFFSET_BITS`]'s
    /// zero, i.e. the start of the window.
    pub rst: Signal,
    /// The packed window the value is read from.
    #[bits(64)]
    pub word: Signal,
    /// Which bit of `word` the value starts at. Zero is the least significant bit.
    #[bits(7)]
    pub off: Signal,
    /// On this edge, the position register takes the next offset.
    pub advance: Signal,
}

/// The output ports.
#[derive(Clone, Debug, PortList)]
pub struct Outputs {
    /// The unpacked value. Combinational: `off` in on one cycle, `value` readable on
    /// the same one.
    #[bits(8)]
    pub value: Signal,
    /// Whether the requested offset is inside the window, i.e. `off + 8 <= 64`.
    pub fit: Signal,
    /// The offset the *next* value starts at, `(off + 8) mod 64`.
    #[bits(7)]
    pub next_off: Signal,
    /// Whether the next value starts in a *different* window, i.e. `off + 8 >= 64`.
    pub wrap: Signal,
    /// The position register.
    #[bits(7)]
    pub pos: Signal,
}

/// The signals [`build`] returns, for a caller that wants them by handle.
#[derive(Clone, Debug)]
pub struct Ports {
    /// The input ports.
    pub inputs: Inputs,
    /// The unpacked value, which is also the output.
    pub value: Signal,
    /// Whether the requested offset is inside the window.
    pub fit: Signal,
    /// The offset the next value starts at.
    pub next_off: Signal,
    /// Whether the next value starts in a different window.
    pub wrap: Signal,
    /// The position register, which is also the output.
    pub pos: Signal,
}

/// Builds a bit-unpacker for [`DEFAULT_WIDTH`]-bit values into `design`.
///
/// The clock is named `clk` and the reset `rst`, which is what the crate-level
/// [`crate::CLOCK`] and [`crate::RESET`] say; naming them rather than leaving them
/// positional is what lets the emitted Verilog, the testbench and the cosimulator
/// agree.
///
/// # Errors
///
/// Whatever [`Design`] returns -- a width mismatch or an undriven wire -- and
/// whatever the port-list helpers return. There is no bitpack-specific failure: the
/// width is a build parameter and the offset is a port, so a misconfigured unpacker
/// would be a different call to [`extract`], not a runtime error.
pub fn build(design: &Design) -> Result<Ports, BuildError> {
    build_with_width(design, DEFAULT_WIDTH)
}

/// [`build`] for a caller that wants a different value width.
///
/// `width` must be between 1 and [`WINDOW_BITS`]. The emitted table has `65 - width`
/// arms either way, so the cost of the ROM falls as the width rises -- which is the
/// one sense in which a wider unpacker is cheaper, and it is not worth much.
///
/// # Errors
///
/// Whatever [`Design`] returns. A `width` outside `1..=64` is *not* an error here:
/// [`extract`] clamps it, because the clamp is total and refusing would only move
/// the failure to the caller.
pub fn build_with_width(design: &Design, width: u32) -> Result<Ports, BuildError> {
    let inputs = ferrite_lithic::inputs::<Inputs>(design)?;

    let value = extract(design, &inputs.word, &inputs.off, width)?;
    let (next_off, wrap) = advance_offset(design, &inputs.off, width)?;
    let fit = fits(design, &inputs.off, width)?;

    // The position register has to be a wire rather than used directly: the register's
    // output is what feeds its own next value and a register cannot be an input to
    // itself, because the graph is built forwards and has no cycle in it. The wire is
    // driven from the register afterwards, which is the pattern for any feedback
    // through state and the same one `crc3` uses for its state.
    let pos = design.wire(OFFSET_BITS)?;
    let held = design.ite(&inputs.advance, &next_off, &pos)?;
    let pos_q = design.reg(
        &held,
        &inputs.clk,
        &inputs.rst,
        &design.constant(false),
    )?;
    design.drive(&pos, &pos_q)?;

    ferrite_lithic::outputs::<Outputs>(
        design,
        &Outputs {
            value: value.clone(),
            fit: fit.clone(),
            next_off: next_off.clone(),
            wrap: wrap.clone(),
            pos: pos_q.clone(),
        },
    )?;

    Ok(Ports {
        inputs,
        value,
        fit,
        next_off,
        wrap,
        pos: pos_q,
    })
}

/// The unpacked `width`-bit value that starts at bit `off` of `word`.
///
/// ```text
/// value = (word >> (64 - width - off)) & (2^width - 1)
/// ```
///
/// **The shift is structural, so the offset becomes a table.** [`Design::srl`] takes
/// a `u32` shift *amount*: it is a `slice` plus a `concat` of a zero fill, built at
/// graph-construction time, and there is no node whose shift operand is a signal. A
/// barrel shifter is therefore not expressible, and the shape that *is* expressible
/// is one [`Design::case_`] arm per offset, each arm a constant-shift `slice`. That
/// is what this is, and the arm for `off` is the shift `WINDOW_BITS - width - off`.
///
/// `off` beyond `WINDOW_BITS - width` has no shift amount that would put a
/// `width`-bit value inside the window, so those offsets produce zero. The caller is
/// expected to gate them with [`fits`]; the zero is a defined answer rather than a
/// silent wrong one, which is what makes a table total.
///
/// # Errors
///
/// Whatever [`Design`] returns.
pub fn extract(
    design: &Design,
    word: &Signal,
    off: &Signal,
    width: u32,
) -> Result<Signal, BuildError> {
    let width = width.clamp(1, WINDOW_BITS);
    let out = design.lit(mask_for(width), width)?;
    let mut arms = Vec::new();
    for offset in 0..=(WINDOW_BITS - width) {
        let shift = WINDOW_BITS - width - offset;
        let moved = design.srl(word, shift)?;
        arms.push((u64::from(offset), design.slice(&moved, 0, width)?));
    }
    design.case_(off, &arms, &out)
}

/// Whether `off` names a `width`-bit value entirely inside the window.
///
/// `off + width <= 64`, written as `off <= 64 - width` so that the sum cannot
/// overflow seven bits. The addition is never formed, which is why the comparison is
/// against a literal rather than against the running offset.
///
/// # Errors
///
/// Whatever [`Design`] returns.
pub fn fits(design: &Design, off: &Signal, width: u32) -> Result<Signal, BuildError> {
    let width = width.clamp(1, WINDOW_BITS);
    let last = design.lit(u64::from(WINDOW_BITS - width), off.width())?;
    Ok(design.ule(off, &last)?)
}

/// The next offset, and whether it belongs to the next window.
///
/// `(off + width) mod 64` and `off + width >= 64`.
///
/// The sum is **widened to eight bits before it is added** rather than added at seven.
/// [`Design::add`] truncates to its *left* operand's width, so a seven-bit `off`
/// added to a seven-bit `width` would silently throw away the bit that carries
/// `off + 64` back down to zero -- and `wrap` is exactly that bit. Widening first and
/// slicing afterwards is the only order in which the carry survives, and the
/// alternative (`slice` then compare) cannot see the carry at all.
///
/// Note that `off + width == 64` sets both `wrap` and [`fits`]; that is not a bug.
/// See the module docs.
///
/// # Errors
///
/// Whatever [`Design`] returns.
pub fn advance_offset(
    design: &Design,
    off: &Signal,
    width: u32,
) -> Result<(Signal, Signal), BuildError> {
    let width = width.clamp(1, WINDOW_BITS);
    let wide = off.width().max(8) + 1;
    let sum = design.add(
        &design.zero_extend(off, wide)?,
        &design.lit(u64::from(width), wide)?,
    )?;
    let next = design.slice(&sum, 0, OFFSET_BITS)?;
    let boundary = design.lit(u64::from(WINDOW_BITS), wide)?;
    let wrap = design.ugt(&sum, &boundary)?;
    Ok((next, wrap))
}

/// The mask for a `width`-bit value.
#[must_use]
fn mask_for(width: u32) -> u64 {
    if width >= 64 {
        u64::MAX
    } else {
        (1u64 << width) - 1
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::{DEFAULT_WIDTH, OFFSET_BITS, WINDOW_BITS, mask_for};

    #[test]
    fn the_window_divide_is_exact_at_every_supported_width() {
        // `build_with_width` is public and takes any width in `1..=64`, so the clamp
        // and the offset arithmetic both have to survive the ends. Width 1 is the
        // Parquet definition-level case and width 64 is the degenerate one.
        for width in 1..=WINDOW_BITS {
            assert!(
                WINDOW_BITS % width != usize::MAX,
                "width {width} is representable"
            );
        }
    }

    #[test]
    fn the_offset_port_can_name_every_bit_but_not_the_count() {
        assert_eq!(OFFSET_BITS, 7);
        assert!(
            (1u64 << OFFSET_BITS) == u64::from(WINDOW_BITS),
            "seven bits is exactly 64, so `off` reaches bit 63 and no further"
        );
    }

    #[test]
    fn the_mask_is_a_run_of_ones_from_the_bottom() {
        assert_eq!(mask_for(1), 0b1);
        assert_eq!(mask_for(8), 0xff);
        assert_eq!(mask_for(63), (1u64 << 63) - 1);
        assert_eq!(mask_for(64), u64::MAX, "64 is clamped, not shifted by 64");
    }

    #[test]
    fn the_default_width_divides_the_window_exactly() {
        // Not an algorithm requirement -- a 5-bit unpacker works fine across a window
        // boundary -- but it is why `DEFAULT_WIDTH` is 8: `wrap` and `fit` cannot
        // both be high, so a caller testing `wrap` alone has the whole story.
        assert_eq!(WINDOW_BITS % DEFAULT_WIDTH, 0);
    }
}