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
//! value = (word >> off) & mask(w)
//! ```
//!
//! with `off` counted from the least significant bit. Two operands, one barrel shifter
//! and one mask. That is the whole datapath.
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
//! The `ports` list is 64 bits wide and not 128, and the reason is the same as
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
//! not a barrel shifter. [`rle`](crate::rle) and `sketch` make the
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
/// is a byte-and-a-bit *position*, not an index, so 64 is not a legal `off`.
pub const OFFSET_BITS: u32 = 7;

/// The width of a *bit index* into the window, as opposed to a bit position.
///
/// Six, i.e. `log2(WINDOW_BITS)`, and the distinction is not pedantry:
/// [`advance_offset`]'s answer is `(off + width) mod 64`, and `64` needs seven bits,
/// so slicing the sum to [`OFFSET_BITS`] would reduce it mod 128 rather than mod 64 and
/// hand back 64 where the answer is 0. The index width is what actually reduces mod the
/// window; the position width is what the port has to be.
pub const INDEX_BITS: u32 = 6;

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
    let inputs = ferrite_lithic::inputs::<Inputs>(design)?;

    let value = extract(design, &inputs.word, &inputs.off, DEFAULT_WIDTH)?;
    let (next_off, wrap) = advance_offset(design, &inputs.off, DEFAULT_WIDTH)?;
    let fit = fits(design, &inputs.off, DEFAULT_WIDTH)?;

    // The position register has to be a wire rather than used directly: the register's
    // output is what feeds its own next value and a register cannot be an input to
    // itself, because the graph is built forwards and has no cycle in it. The wire is
    // driven from the register afterwards, which is the pattern for any feedback
    // through state and the same one `crc3` uses for its state.
    let pos = design.wire(OFFSET_BITS)?;
    let held = design.ite(&inputs.advance, &next_off, &pos)?;
    let pos_q = design.reg(&held, &inputs.clk, &inputs.rst, &design.constant(false))?;
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

/// The signals a standalone [`build_extractor`] design exposes.
#[derive(Clone, Debug)]
pub struct Extractor {
    /// The packed window, an input port named `word`.
    pub word: Signal,
    /// The bit offset, an input port named `off`.
    pub off: Signal,
    /// The unpacked value, an output port named `value`.
    pub value: Signal,
    /// Whether the offset is inside the window, an output port named `fit`.
    pub fit: Signal,
    /// The next offset, an output port named `next_off`.
    pub next_off: Signal,
    /// Whether the next value is in the next window, an output port named `wrap`.
    pub wrap: Signal,
}

/// Builds a standalone, clockless extractor for `width`-bit values.
///
/// The port list of [`Inputs`]/[`Outputs`] fixes `value` at [`DEFAULT_WIDTH`] bits, and
/// `Design::output` does **not** check its declared width against the driver it is
/// given -- it creates a wire of the declared width and drives it, so a width-4 value
/// on a width-8 port would quietly come back zero-extended. Rather than paper over that
/// with a second port list per width, the width-generic form is a function of the width
/// and gets its ports from [`Design::input`] and [`Design::output`] directly.
///
/// No clock and no register, so the module is pure combinational logic: the testbench's
/// rule is that a design with no clock settles rather than taking a rising edge.
///
/// # Errors
///
/// Whatever [`Design`] returns. A `width` outside `1..=64` is not an error: [`extract`]
/// clamps it, because the clamp is total and refusing would only move the failure to
/// the caller.
pub fn build_extractor(design: &Design, width: u32) -> Result<Extractor, BuildError> {
    let width = width.clamp(1, WINDOW_BITS);
    let word = design.input("word", WINDOW_BITS)?;
    let off = design.input("off", OFFSET_BITS)?;
    let value = extract(design, &word, &off, width)?;
    let (next_off, wrap) = advance_offset(design, &off, width)?;
    let fit = fits(design, &off, width)?;
    design.output("value", width, &value)?;
    design.output("fit", 1, &fit)?;
    design.output("next_off", OFFSET_BITS, &next_off)?;
    design.output("wrap", 1, &wrap)?;
    Ok(Extractor {
        word,
        off,
        value,
        fit,
        next_off,
        wrap,
    })
}

/// The unpacked `width`-bit value that starts at bit `off` of `word`.
///
/// ```text
/// value = (word >> off) & (2^width - 1)
/// ```
///
/// with `off` counted from the **least** significant bit, which is the convention
/// `bitpacking` uses and the one the differential in `tests/bitpack.rs` checks.
///
/// **The shift is structural, so the offset becomes a table.** [`Design::srl`] takes a
/// `u32` shift *amount*: it is a `slice` plus a `concat` of a zero fill, built at
/// graph-construction time, and there is no node whose shift operand is a signal. A
/// barrel shifter is therefore not expressible, and the shape that *is* expressible
/// is one [`Design::case_`] arm per offset, each arm a constant-shift `slice`. That
/// is what this is, and the arm for `off` is the shift `off`.
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
    // The default is **zero**, not the mask. An offset with no `width`-bit value inside
    // the window has bits *above* the window rather than below the offset, so filling
    // them with ones would answer with a plausible-looking value for a request that has
    // no answer; zero plus `fit` low says "there is nothing here" unambiguously, which is
    // the contract [`fits`] exists to let a caller act on.
    let out = design.zeros(width)?;
    let mut arms = Vec::new();
    for offset in 0..=(WINDOW_BITS - width) {
        let moved = design.srl(word, offset)?;
        arms.push((u64::from(offset), design.slice(&moved, 0, width)?));
    }
    Ok(design.case_(off, &arms, &out)?)
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
/// The sum is **widened before it is added** rather than added at seven bits.
/// [`Design::add`] truncates to its *left* operand's width, so a seven-bit `off`
/// added to a seven-bit `width` would throw away the bit that carries `off + 64` back
/// down to zero -- and `wrap` is exactly that bit. The sum is taken at
/// `OFFSET_BITS + 1` bits, which is the only order in which the carry survives; the
/// alternative (slice first, compare second) cannot see the carry at all.
///
/// **The modulo is [`INDEX_BITS`], not [`OFFSET_BITS`].** `off + width` reaches 64 at
/// `off = 56`, and 64 is representable in seven bits -- so narrowing to the port's
/// width leaves it at 64, which is not a bit of this window at all. Narrowing to six
/// bits is what turns 64 into 0. The result is then widened back to
/// [`OFFSET_BITS`] for the port.
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
    let wide = OFFSET_BITS + 1;
    let sum = design.add(
        &design.zero_extend(off, wide)?,
        &design.lit(u64::from(width), wide)?,
    )?;
    let wrapped = design.slice(&sum, 0, INDEX_BITS)?;
    let next = design.zero_extend(&wrapped, OFFSET_BITS)?;
    let boundary = design.lit(u64::from(WINDOW_BITS), wide)?;
    let wrap = design.uge(&sum, &boundary)?;
    Ok((next, wrap))
}

/// The mask for a `width`-bit value.
#[must_use]
#[cfg(test)]
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

    use ferrite_lithic::Design;

    use super::{DEFAULT_WIDTH, INDEX_BITS, OFFSET_BITS, WINDOW_BITS, mask_for};

    #[test]
    fn every_representable_width_builds_an_extractor() {
        // The table has `65 - width` arms, so the degenerate widths are the ones most
        // likely to fall out of the generator's arithmetic: one bit is the Parquet
        // definition-level case with a 64-entry table, sixty-four is a whole word with a
        // single arm and therefore a case with nothing to select.
        for width in [1u32, 2, 5, 8, 63, 64] {
            let design = Design::new();
            let extractor = super::build_extractor(&design, width).expect("1..=64 builds");
            assert_eq!(extractor.value.width(), width, "width {width}");
            assert!(
                WINDOW_BITS - width < (1 << OFFSET_BITS),
                "width {width} has {} legal offsets, all nameable by the port",
                WINDOW_BITS - width + 1
            );
        }
    }

    #[test]
    fn the_offset_port_is_wider_than_the_window_and_the_index_is_not() {
        // The port is seven bits and the window is 64, so `off` can name offsets 64 to
        // 127 that do not exist. That is deliberate rather than sloppy: an offset is a
        // bit *position* and one past the end is the natural thing for a caller to hand
        // in, and [`fits`] is what turns it into an answer. The *index* width, which is
        // what `next_off` is narrowed to, has to be six bits or the modulo would be
        // against 128 rather than 64.
        assert_eq!(OFFSET_BITS, 7);
        assert_eq!(INDEX_BITS, 6);
        assert_eq!(1u64 << INDEX_BITS, u64::from(WINDOW_BITS));
        assert!(
            (1u64 << OFFSET_BITS) > u64::from(WINDOW_BITS),
            "seven bits reaches past the window, and `fits` gates it"
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
