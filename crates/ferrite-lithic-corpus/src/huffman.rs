//! Canonical Huffman decode: a shift register and a table, one symbol per cycle.
//!
//! # The thesis this module exists to make
//!
//! **Entropy decoding is serial and entropy-bound, and that is the whole argument for
//! building it rather than calling a library.**
//!
//! A Huffman decoder cannot know where the next symbol starts until it has decoded the
//! previous one. The next code is a variable number of bits long, and its offset is the
//! length of the code that came before. There is no loop to vectorise, no lane to fill,
//! and no way to know where symbol `i + 4` starts without having resolved `i`, `i + 1`,
//! `i + 2` and `i + 3` first. On a CPU the answer is a table lookup per symbol inside a
//! tight loop with a data-dependent branch on the code length; on an ASIC the answer is
//! *two* pieces of hardware and no loop at all -- a shift register and a read-only table
//! -- and it retires one symbol per cycle with **fixed latency**, which no CPU branch can
//! promise.
//!
//! That is the strongest ASIC-over-SIMD argument in the corpus, and it is also why this
//! module is the hardest to get right: the variable-length bit handling is not a detail
//! of the algorithm, it *is* the algorithm.
//!
//! # The hardware shape
//!
//! ```text
//!   bit stream -> [ 9-bit shift register ] -peek 9 bits-> [ 512-entry table ] -> (symbol, length)
//!                                       ^                                          |
//!                                       +--------- drop `length` bits ------------ +
//! ```
//!
//! Two pieces of state: the bit buffer, and nothing else. The table entry gives both the
//! symbol *and* the number of bits to drop, which is why the design needs no second pass
//! over the bits -- one lookup and one variable shift per symbol.
//!
//! # Why the buffer is exactly [`TABLE_BITS`] deep
//!
//! [`BUFFER_BITS`] equals [`TABLE_BITS`], and that is not laziness, it is the whole
//! argument:
//!
//! * For a **complete** code (Kraft sum exactly one) and a table as wide as the longest
//!   code, *every* [`TABLE_BITS`]-bit prefix is the prefix of some code. So once
//!   [`BUFFER_BITS`] bits are buffered a symbol is always decodable.
//! * A decode consumes at least one bit. So the buffer can never need to hold more than
//!   [`BUFFER_BITS`] bits: at [`BUFFER_BITS`] it is guaranteed to drain.
//!
//! A wider buffer would be flops that can never be used. zlib and Brotli both use a much
//! deeper buffer, but only because they refill it a *byte* at a time from a memory port;
//! this design is fed one bit per cycle, so there is nothing to refill from and a deeper
//! buffer would be dead silicon. [`Outputs::in_ready`] falls when the buffer is full and
//! is a real output rather than left out, because a decoder whose input cannot be
//! back-pressured corrupts data when its source runs ahead.
//!
//! # How the canonical code is derived, and how the table is built from it
//!
//! A canonical Huffman code is the one determined entirely by the *code lengths*: sort
//! the symbols by `(length, symbol)` and hand out codes in that order, each length
//! starting where the previous length ran out. [`canonical_codes`] is that algorithm,
//! spelled out:
//!
//! ```text
//! next_code[1] = 0
//! next_code[len] = (next_code[len-1] + count[len-1]) << 1
//! then, for symbols in increasing order: code[sym] = next_code[length[sym]]++
//! ```
//!
//! The second line is the whole content of "canonical": each length's codes start one
//! place past the end of the previous length's, shifted left by one to make room for the
//! extra bit. Walking the symbols in increasing order *is* the sort by `(length, symbol)`,
//! because each length's cursor only ever moves forward -- which is why this is one loop
//! and not a sort. Note that length-zero entries contribute nothing: they count symbols
//! the code does not mention, and folding them into the count would shift every real code.
//!
//! [`Table::canonical`] then turns the code assignment into the hardware's single-level
//! lookup table. A code of length `L` with value `C` owns every [`TABLE_BITS`]-bit index
//! whose *top* `L` bits are `C` -- that is `2^(TABLE_BITS - L)` indices, starting at
//! `C << (TABLE_BITS - L)` -- because the peek presents the next [`TABLE_BITS`] bits with
//! the next code bit in the most significant position. The table is filled by
//! enumerating those indices, which is why it is [`TABLE_ENTRIES`] entries whatever the
//! alphabet size.
//!
//! Entries no code claims are marked with **length zero**, which is the design's "no code
//! matches here" value. For a complete code nothing is ever marked; for the one
//! incomplete code this module supports ([`Table::single`]) nothing is marked either. The
//! marker exists because a peek is a *partial* code: a buffer holding fewer than
//! [`TABLE_BITS`] bits peeks a zero-padded prefix, and the padding must not be mistaken
//! for a zero-length code.
//!
//! # What the table costs, honestly: it is a mux tree, not a memory
//!
//! **There is no initialised-memory node in the IR.** `Design::mem` is zero-filled in both
//! backends, so there is no way to say "these are the contents" and a decode table has to
//! be built as [`Design::case_`] -- which [`Design::rom`] does -- and that emits an
//! `always @*` block with one arm per entry.
//!
//! In silicon that is a different thing from a table:
//!
//! | | as built here | a real decoder |
//! |---|---|---|
//! | 512 x 13-bit lookup | 512-to-1 mux per output bit: **9 levels** of 2:1 mux | one read port on a block RAM |
//! | read latency | one cycle, behind a deep combinational path | one cycle, fixed |
//! | area | roughly 6 700 2:1 muxes for the table alone | 512 x 13 bits of storage plus a decoder |
//!
//! 512-to-1 over 13 bits is buildable and it will run at a lower clock than the rest of
//! the core; it is *not* what anyone would ship for a 512-entry table. A synthesis tool
//! would infer the RAM from the `case` in seconds. The honest summary is that this design
//! is correct and its *shape* is right, but the table is a placeholder for storage the IR
//! cannot express yet.
//!
//! # What this module does not implement
//!
//! * **No code-length (dynamic) tree.** [`Table`] is built from code lengths *at
//!   elaboration time*, which is what a decoder is handed by the block header. Parsing a
//!   DEFLATE dynamic block header -- nineteen code lengths, a seven-bit code-length code,
//!   and the run-length 16/17/18 codes -- is a different module and is not here.
//! * **No code longer than [`TABLE_BITS`] bits.** A DEFLATE literal code needs fifteen.
//!   Real decoders solve that with a two-level table (a [`TABLE_BITS`]-entry root plus a
//!   sub-table for the codes that overflow it), which is why [`TABLE_BITS`] is a constant
//!   and not a parameter.
//! * **No multi-symbol-per-cycle decode.** One symbol per cycle, which is the point.
//! * **No error flag for a malformed stream.** An index the table marks with length zero
//!   stalls the decoder with [`Outputs::out_valid`] low and [`Outputs::held`] pinned at
//!   [`BUFFER_BITS`], which a host can detect as a stall. It does not raise a signal.

use ferrite_lithic::{Design, Signal};
use ferrite_lithic_derive::PortList;

use crate::BuildError;

/// How many bits of the bit stream one table lookup covers.
///
/// Also the width of the decode table's address and the deepest code this module can
/// represent, because a code longer than the peek cannot be resolved by one lookup.
pub const TABLE_BITS: u32 = 9;

/// How many entries the decode table has: `2^TABLE_BITS`.
///
/// Nine bits is the width a real decoder would use for a DEFLATE *fixed* literal/length
/// code, whose longest code is nine bits, so a table this wide holds that code exactly --
/// see [`FIXED_LITERAL_LENGTH_LENGTHS`].
pub const TABLE_ENTRIES: usize = 1 << TABLE_BITS;

/// How deep the bit buffer is.
///
/// Equal to [`TABLE_BITS`] and deliberately so; the module docs carry the argument for why
/// a wider buffer could never be used by this design.
pub const BUFFER_BITS: u32 = TABLE_BITS;

/// The width of a symbol.
///
/// Nine, not eight, because DEFLATE's alphabets run past 255: the end-of-block code is
/// 256 and the last literal/length code is 287. A decoder whose symbol port is eight bits
/// wide cannot represent the alphabet the format actually uses, which would make this
/// module a decoder for a code that does not exist.
pub const SYMBOL_BITS: u32 = 9;

/// How many bits the packed table entry's code-length field is wide.
///
/// [`TABLE_BITS`] needs four, and the spare value zero is the "no code" marker.
const LENGTH_BITS: u32 = 4;

/// A width that holds a bit count up to [`BUFFER_BITS`], which is what the `held` register
/// counts.
const HELD_BITS: u32 = 4;

/// The packed width of one table entry: a nine-bit symbol below a four-bit length.
const ENTRY_BITS: u32 = SYMBOL_BITS + LENGTH_BITS;

/// One canonical code: a symbol and the bits it is written as.
///
/// `code` is the numeric value with its **most significant bit first**, which is the order
/// the bits appear in the stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CodeWord {
    /// The symbol this code stands for.
    pub symbol: u16,
    /// How many bits the code is written as.
    pub length: u8,
    /// The code, most significant bit first.
    pub code: u16,
}

/// Why a set of code lengths is not a usable canonical code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CodeError {
    /// No symbol had a nonzero length, so there is no code at all.
    NoSymbols,
    /// A code was longer than [`MAX_CODE_LENGTH`].
    TooDeep {
        /// The offending code length.
        length: u8,
    },
    /// The lengths sum to more than a full code: the Kraft sum exceeds one.
    OverSubscribed,
    /// The lengths sum to less than a full code, so some [`TABLE_BITS`]-bit prefix claims
    /// no code. Use [`Table::single`] for the one incomplete code this module supports.
    Incomplete,
}

impl std::fmt::Display for CodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoSymbols => write!(f, "no symbol has a nonzero code length"),
            Self::TooDeep { length } => {
                write!(
                    f,
                    "a code of {length} bits is longer than {MAX_CODE_LENGTH}"
                )
            }
            Self::OverSubscribed => write!(f, "the code lengths sum to more than one"),
            Self::Incomplete => write!(f, "the code lengths sum to less than one"),
        }
    }
}

impl std::error::Error for CodeError {}

/// The longest code [`canonical_codes`] will consider.
///
/// DEFLATE's own limit (RFC 1951 section 3.2.7) is fifteen and this is the same number,
/// so the function can check a real code even though [`Table`] cannot hold one that deep.
pub const MAX_CODE_LENGTH: u8 = 15;

/// The canonical code assigned to every symbol, given code lengths.
///
/// `lengths` is indexed by symbol and a zero entry means the symbol does not appear in
/// this code. The returned vector has the same shape, holding each symbol's `(code,
/// length)` and `(0, 0)` for symbols that do not appear.
///
/// This is the whole of "canonical": sort by `(length, symbol)`, hand out codes in that
/// order. No tree is built, no symbol ordering is chosen, and the code words are a pure
/// function of the lengths -- which is exactly what lets a decoder be built from lengths
/// alone.
///
/// # Errors
///
/// [`CodeError::NoSymbols`] if no length is nonzero, [`CodeError::TooDeep`] if a length
/// exceeds [`MAX_CODE_LENGTH`], and [`CodeError::OverSubscribed`] if the lengths need
/// more than one whole code's worth of space. An *incomplete* set of lengths is accepted
/// here, because whether a partial code is representable depends on the table's width and
/// [`Table::canonical`] is what knows that.
///
/// ```
/// use ferrite_lithic_corpus::huffman::{CodeWord, canonical_codes};
///
/// // A hand-checkable complete code: one code of length 1, one of 2, two of 3.
/// // Kraft: 1/2 + 1/4 + 2/8 = 1. Canonical order is by (length, symbol), so the
/// // length-1 code is 0, the length-2 code is (0 + 1) << 1 = 2 = "10", and the two
/// // length-3 codes are (2 + 1) << 1 = 6 = "110" and 7 = "111".
/// let codes = canonical_codes(&[1u8, 2, 3, 3]).unwrap();
/// assert_eq!(codes[0], CodeWord { symbol: 0, length: 1, code: 0 });
/// assert_eq!(codes[1], CodeWord { symbol: 1, length: 2, code: 2 });
/// assert_eq!(codes[2], CodeWord { symbol: 2, length: 3, code: 6 });
/// assert_eq!(codes[3], CodeWord { symbol: 3, length: 3, code: 7 });
/// ```
///
/// # Panics
///
/// Never. Every failure is a [`CodeError`].
pub fn canonical_codes(lengths: &[u8]) -> Result<Vec<CodeWord>, CodeError> {
    let mut counts = [0u32; 16];
    for length in lengths {
        if *length > MAX_CODE_LENGTH {
            return Err(CodeError::TooDeep { length: *length });
        }
        if *length > 0 {
            counts[*length as usize] += 1;
        }
    }
    let max_length = (1..=usize::from(MAX_CODE_LENGTH))
        .rev()
        .find(|length| counts[*length] > 0)
        .ok_or(CodeError::NoSymbols)?;

    // Kraft: how much of the code space the lengths claim, in units of the shortest code.
    // Exactly one whole code is complete; more is over-subscribed.
    let capacity = 1u32 << max_length;
    let claimed = (1..=max_length)
        .map(|length| counts[length] << (max_length - length))
        .sum::<u32>();
    if claimed > capacity {
        return Err(CodeError::OverSubscribed);
    }

    // The canonical assignment. `next_code[len]` is the first code of that length and
    // starts one place past the end of the previous length, shifted left by one to make
    // room for the extra bit. `counts[0]` is deliberately excluded: a length-zero symbol
    // occupies no space in the tree.
    let mut next_code = [0u32; 17];
    let mut after = 0u32;
    for length in 1..=max_length {
        after = (after + if length == 1 { 0 } else { counts[length - 1] }) << 1;
        next_code[length] = after;
    }

    let mut codes: Vec<CodeWord> = lengths
        .iter()
        .enumerate()
        .map(|(symbol, length)| CodeWord {
            symbol: symbol as u16,
            length: *length,
            code: 0,
        })
        .collect();
    // Walking the symbols in increasing order is the `(length, symbol)` sort: each
    // length's cursor only moves forward, so within a length the codes are handed out in
    // symbol order, and across lengths in length order.
    for symbol in 0..codes.len() {
        let length = lengths[symbol] as usize;
        if length == 0 {
            continue;
        }
        codes[symbol].code = next_code[length] as u16;
        next_code[length] += 1;
    }

    Ok(codes)
}

/// A single-level decode table: [`TABLE_ENTRIES`] entries of `(symbol, length)`.
///
/// A **length of zero means no code claims this index**, which is the "no code here"
/// marker the design's decoder needs for a peek that lands in a gap.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Table {
    entries: Vec<(u16, u8)>,
}

impl Table {
    /// The decode table for a canonical code given its code lengths.
    ///
    /// # Errors
    ///
    /// Whatever [`canonical_codes`] returns, plus [`CodeError::TooDeep`] when a code is
    /// longer than [`TABLE_BITS`] -- which is a different failure from the same name in
    /// [`canonical_codes`], where the limit is DEFLATE's fifteen bits. An incomplete code
    /// is [`CodeError::Incomplete`], because some [`TABLE_BITS`]-bit prefix then claims
    /// nothing.
    ///
    /// ```
    /// use ferrite_lithic_corpus::huffman::Table;
    ///
    /// // Symbols 0, 1, 2 with lengths 1, 2, 2. Canonical codes: "0", "10", "11". A code
    /// // of length L owns every index whose top L bits are its value, so with a nine-bit
    /// // peek symbol 0 owns 0..256, symbol 1 owns 256..384 and symbol 2 owns 384..512.
    /// // Every index is claimed, which is what makes the code complete.
    /// let table = Table::canonical(&[1u8, 2, 2]).unwrap();
    /// assert_eq!(table.entry(0), (0, 1));
    /// assert_eq!(table.entry(255), (0, 1));
    /// assert_eq!(table.entry(256), (1, 2));
    /// assert_eq!(table.entry(383), (1, 2));
    /// assert_eq!(table.entry(384), (2, 2));
    /// assert_eq!(table.entry(511), (2, 2));
    /// ```
    ///
    /// # Panics
    ///
    /// Never.
    pub fn canonical(lengths: &[u8]) -> Result<Self, CodeError> {
        let codes = canonical_codes(lengths)?;
        if let Some(longest) = codes.iter().map(|word| word.length).max() {
            if u32::from(longest) > TABLE_BITS {
                return Err(CodeError::TooDeep { length: longest });
            }
        }
        let mut entries = vec![(0u16, 0u8); TABLE_ENTRIES];
        for word in &codes {
            if word.length == 0 {
                continue;
            }
            let length = u32::from(word.length);
            let span = 1u32 << (TABLE_BITS - length);
            let first = u32::from(word.code) << (TABLE_BITS - length);
            for index in first..first + span {
                entries[index as usize] = (word.symbol, word.length);
            }
        }
        // An entry that survived with length zero is a prefix no code claims, which is
        // exactly what "incomplete" means for a table of this width.
        if entries.iter().any(|(_, length)| *length == 0) {
            return Err(CodeError::Incomplete);
        }
        Ok(Self { entries })
    }

    /// The one incomplete code this module supports: a single symbol, one bit long.
    ///
    /// RFC 1951 section 3.2.7's single-code rule, and what zlib's `inflate` does when a
    /// block can contain exactly one symbol: the symbol is written as one bit, so the
    /// decoder consumes a bit and emits the symbol. Every table entry says the same thing,
    /// which is what makes a code with a Kraft sum of one half representable at all.
    ///
    /// ```
    /// use ferrite_lithic_corpus::huffman::Table;
    ///
    /// let table = Table::single(7);
    /// for index in 0..512u32 {
    ///     assert_eq!(table.entry(index as u16), (7, 1), "index {index}");
    /// }
    /// ```
    #[must_use]
    pub fn single(symbol: u16) -> Self {
        Self {
            entries: vec![(symbol, 1u8); TABLE_ENTRIES],
        }
    }

    /// What the design reads for one peek: `(symbol, length)`, or `(0, 0)` for an index no
    /// code claims.
    #[must_use]
    pub fn entry(&self, index: u16) -> (u16, u8) {
        self.entries[usize::from(index) & (TABLE_ENTRIES - 1)]
    }

    /// The table as [`Design::rom`] wants it: `(length << 9) | symbol` per entry.
    ///
    /// Symbol low and length high because the symbol is the field the design reads without
    /// arithmetic.
    #[must_use]
    pub fn packed(&self) -> Vec<u64> {
        self.entries
            .iter()
            .map(|(symbol, length)| (u64::from(*length) << SYMBOL_BITS) | u64::from(*symbol))
            .collect()
    }
}

/// RFC 1951 section 3.2.6's **fixed literal/length** code lengths.
///
/// Literals `0..=143` are eight bits, `144..=255` are nine, the end-of-block code and the
/// first 23 length codes (`256..=279`) are seven, and the remaining eight length codes
/// (`280..=287`) are eight. This is a real, externally specified code whose assignment is
/// published in the RFC, so running [`canonical_codes`] on it is checked against a source
/// outside this repository -- see the test file.
///
/// It is also the deepest code a [`Table`] can hold exactly, because nine bits is
/// [`TABLE_BITS`].
pub const FIXED_LITERAL_LENGTH_LENGTHS: [u8; 288] = {
    let mut lengths = [0u8; 288];
    let mut symbol = 0;
    while symbol < 288 {
        lengths[symbol] = match symbol {
            0..=143 => 8,
            144..=255 => 9,
            256..=279 => 7,
            _ => 8,
        };
        symbol += 1;
    }
    lengths
};

/// RFC 1951 section 3.2.5's **fixed distance** code lengths: every one of the 32 distance
/// codes is five bits.
///
/// The fixed distance code is the identity -- distance code `d` is written as the five-bit
/// value `d` -- which makes it the sharpest available check on the canonical construction,
/// because it is published in the RFC *and* it is what a five-bit-wide table has to
/// produce.
pub const FIXED_DISTANCE_LENGTHS: [u8; 32] = [5u8; 32];

/// The input ports.
#[derive(Clone, Debug, PortList)]
pub struct Inputs {
    /// The clock. Every edge advances the decoder by at most one symbol.
    #[clock]
    pub clk: Signal,
    /// Synchronous reset, which empties the bit buffer.
    pub rst: Signal,
    /// Start a new message: empties the bit buffer and drops any pending symbol.
    ///
    /// A decoder streams many messages and only a reset could restart it otherwise.
    pub init: Signal,
    /// A bit of the compressed stream is available on [`Self::in_bit`].
    pub in_valid: Signal,
    /// The next bit of the compressed stream, least significant bit of each byte first --
    /// DEFLATE's order, and the one every other module here uses.
    pub in_bit: Signal,
}

/// The output ports.
#[derive(Clone, Debug, PortList)]
pub struct Outputs {
    /// The decoder has room for another bit.
    ///
    /// Low exactly when [`Self::held`] is at [`BUFFER_BITS`], which for a complete code is
    /// the one cycle before the buffer must drain. See the module docs for why the buffer
    /// never needs to be any deeper.
    pub in_ready: Signal,
    /// High while undecoded bits are buffered.
    pub busy: Signal,
    /// A decoded symbol is on [`Self::symbol`], and it cost [`Self::code_len`] bits.
    pub out_valid: Signal,
    /// The decoded symbol.
    #[bits(9)]
    pub symbol: Signal,
    /// How many bits the symbol on [`Self::symbol`] was written as.
    #[bits(4)]
    pub code_len: Signal,
    /// How many bits are buffered and undecoded.
    #[bits(4)]
    pub held: Signal,
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
    buffer: Signal,
    held: Signal,
    out_valid: Signal,
    symbol: Signal,
    code_len: Signal,
}

/// Builds a canonical Huffman decoder for `table` into `design`.
///
/// The table *is* the design's content, so it is an argument rather than a constant: the
/// code is whatever the block header said, and baking one in would make this a decoder for
/// a code nobody uses.
///
/// The clock is named `clk` and the reset `rst`, which is what the crate-level
/// [`crate::CLOCK`] and [`crate::RESET`] say.
///
/// # Errors
///
/// Whatever [`Design`] returns -- a width mismatch or an undriven wire -- and whatever the
/// port-list helpers return. A table is not re-validated here: it arrives already built by
/// [`Table`], and the one thing the builder cannot express is a table deeper than
/// [`TABLE_BITS`], which is a constant rather than a runtime property.
///
/// ```
/// use ferrite_lithic::Design;
/// use ferrite_lithic_corpus::huffman::{Table, build};
///
/// let design = Design::new();
/// build(&design, &Table::canonical(&[2, 2, 2, 2]).unwrap()).unwrap();
/// assert!(design.node_count() > 0);
/// ```
pub fn build(design: &Design, table: &Table) -> Result<Ports, BuildError> {
    let inputs = ferrite_lithic::inputs::<Inputs>(design)?;

    let now = State {
        buffer: design.wire(BUFFER_BITS)?,
        held: design.wire(HELD_BITS)?,
        out_valid: design.wire(1)?,
        symbol: design.wire(SYMBOL_BITS)?,
        code_len: design.wire(LENGTH_BITS)?,
    };

    let next = next_state(design, &inputs, &now, &table.packed())?;

    // The registers have to be wired through a signal because the graph is built forwards:
    // a register cannot be an input to itself.
    let hold = design.constant(false);
    let buffer = design.reg(&next.buffer, &inputs.clk, &inputs.rst, &hold)?;
    let held = design.reg(&next.held, &inputs.clk, &inputs.rst, &hold)?;
    let out_valid = design.reg(&next.out_valid, &inputs.clk, &inputs.rst, &hold)?;
    let symbol = design.reg(&next.symbol, &inputs.clk, &inputs.rst, &hold)?;
    let code_len = design.reg(&next.code_len, &inputs.clk, &inputs.rst, &hold)?;

    design.drive(&now.buffer, &buffer)?;
    design.drive(&now.held, &held)?;
    design.drive(&now.out_valid, &out_valid)?;
    design.drive(&now.symbol, &symbol)?;
    design.drive(&now.code_len, &code_len)?;

    // `in_ready` and `busy` are pure functions of the `held` register, so they are wires
    // rather than a cycle-delayed copy of it: a registered copy would answer one edge
    // late, which is the edge the host needs the answer on.
    let capacity = design.lit(u64::from(BUFFER_BITS), HELD_BITS)?;
    let in_ready = design.ult(&now.held, &capacity)?;
    let idle = design.eq(&now.held, &design.lit(0, HELD_BITS)?)?;
    let busy = design.not(&idle);

    let outputs = ferrite_lithic::outputs::<Outputs>(
        design,
        &Outputs {
            in_ready,
            busy,
            out_valid: out_valid.clone(),
            symbol: symbol.clone(),
            code_len: code_len.clone(),
            held: held.clone(),
        },
    )?;

    Ok(Ports { inputs, outputs })
}

/// One cycle of the decoder: shift a bit in, decode a symbol, shift its bits out.
///
/// The shift-in and the shift-out happen on the same edge, against the *same* intermediate
/// buffer, so a code that becomes complete on the very bit that arrived is decoded on that
/// edge rather than one edge late.
///
/// # Errors
///
/// Whatever [`Design`] returns.
fn next_state(
    design: &Design,
    inputs: &Inputs,
    now: &State,
    packed: &[u64],
) -> Result<State, BuildError> {
    let capacity = design.lit(u64::from(BUFFER_BITS), HELD_BITS)?;
    let in_ready = design.ult(&now.held, &capacity)?;
    let accepting = design.and(&inputs.in_valid, &in_ready)?;

    // Shift one bit in at position `held`, which is the *end* of the buffer, because bits
    // enter the low end and are consumed from the low end. `1 << held` is that position;
    // in silicon it is a decode of `held` and an OR, a handful of gates rather than a
    // barrel shifter.
    //
    // The incoming bit has to be *replicated* across the one-hot's width and then ANDed,
    // not ANDed with the one-hot and then placed: `1 << held & bit` is zero for every
    // `held` but zero, because a one-hot and a lone 1 share no set bit above bit zero.
    // That mistake passes a one-bit code and fails everything else, which is a fair
    // description of how subtle it is.
    let insert_at = one_hot(design, &now.held, BUFFER_BITS)?;
    let spread = design.replicate(&inputs.in_bit, BUFFER_BITS)?;
    let inserted = design.and(&insert_at, &spread)?;
    let with_bit = design.or(&now.buffer, &inserted)?;
    let buffer = design.ite(&accepting, &with_bit, &now.buffer)?;
    let gained = design.add(&now.held, &design.lit(1, HELD_BITS)?)?;
    let held = design.ite(&accepting, &gained, &now.held)?;

    // Peek the next TABLE_BITS bits and look the code up.
    //
    // The reversal is not optional. Bits enter the buffer's *low* end in stream order and
    // a Huffman code is written most significant bit first, so the bit that arrives first
    // is the code's *most* significant bit, and the table is indexed by the code's numeric
    // value. So the peek puts buffer bit zero at index bit `TABLE_BITS - 1`.
    //
    // In silicon that reversal is a wire permutation and costs no gates at all. Here it is
    // nine one-bit selects and eight concatenations, because the DSL has no way to say "the
    // same bits, in the other order".
    //
    // Bits above `held` are zero because every operation here shifts zeros in at the
    // bottom, so a partly filled buffer peeks a zero-padded prefix. That is safe for a
    // complete code: a match found inside the padding either lies within `held` bits -- in
    // which case prefix-freeness makes it the true code -- or is longer than `held` and is
    // rejected by the count check below.
    let mut peek = Vec::with_capacity(TABLE_BITS as usize);
    for position in 0..TABLE_BITS {
        peek.push(design.slice(&buffer, position, 1)?);
    }
    let index = design.concat(&peek)?;
    let entry = design.rom(&index, packed, ENTRY_BITS)?;
    let symbol = design.slice(&entry, 0, SYMBOL_BITS)?;
    let code_len = design.slice(&entry, SYMBOL_BITS, LENGTH_BITS)?;

    // A length of zero is the "no code here" marker, so it must not be taken as a zero-bit
    // code -- otherwise a peek that lands in a gap would emit a symbol per cycle instead
    // of waiting for more bits.
    let unmatched = design.eq(&code_len, &design.lit(0, LENGTH_BITS)?)?;
    let matched = design.not(&unmatched);
    let enough = design.uge(&held, &code_len)?;
    let decoding = design.and(&matched, &enough)?;

    // Drop `code_len` bits. `srl` takes a constant amount, so a variable shift has to be a
    // select over the possible amounts: a ten-way mux in silicon, where a real design
    // would infer a barrel shifter from this same expression.
    let dropped = shift_right_by(design, &buffer, &code_len)?;
    let buffer = design.ite(&decoding, &dropped, &buffer)?;
    let spent = design.sub(&held, &code_len)?;
    let held = design.ite(&decoding, &spent, &held)?;

    // `init` restarts a message, so it wins over everything on its own edge.
    let clearing = inputs.init.clone();
    let buffer = design.ite(&clearing, &design.zeros(BUFFER_BITS)?, &buffer)?;
    let held = design.ite(&clearing, &design.zeros(HELD_BITS)?, &held)?;

    Ok(State {
        buffer,
        held,
        out_valid: design.and(&decoding, &design.not(&clearing))?,
        symbol: design.ite(&clearing, &design.zeros(SYMBOL_BITS)?, &symbol)?,
        code_len: design.ite(&clearing, &design.zeros(LENGTH_BITS)?, &code_len)?,
    })
}

/// `1 << held`, as a `width`-bit constant.
///
/// The arms cover zero through `width - 1`, which is every value [`held`] can take while a
/// bit is being accepted: at `held == width` the buffer is full, `in_ready` is low and
/// nothing is inserted. The default arm is therefore unreachable and zero.
fn one_hot(design: &Design, held: &Signal, width: u32) -> Result<Signal, BuildError> {
    let mut arms = Vec::with_capacity(width as usize);
    for amount in 0..width {
        arms.push((u64::from(amount), design.lit(1u64 << amount, width)?));
    }
    Ok(design.case_(held, &arms, &design.lit(0, width)?)?)
}

/// `value >> amount`, for an `amount` the design cannot shift by in one go.
///
/// The arms cover zero through [`TABLE_BITS`] and the default is the unshifted value, so
/// an amount outside that range leaves the buffer alone -- the safe answer, because the
/// caller only reaches here having checked that the amount fits in what is buffered.
fn shift_right_by(design: &Design, value: &Signal, amount: &Signal) -> Result<Signal, BuildError> {
    let mut arms = Vec::with_capacity(TABLE_BITS as usize + 1);
    for shift in 0..=TABLE_BITS {
        arms.push((u64::from(shift), design.srl(value, shift)?));
    }
    Ok(design.case_(amount, &arms, value)?)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::{
        BUFFER_BITS, CodeError, FIXED_DISTANCE_LENGTHS, FIXED_LITERAL_LENGTH_LENGTHS,
        MAX_CODE_LENGTH, TABLE_BITS, TABLE_ENTRIES, Table, canonical_codes,
    };

    #[test]
    fn a_code_is_complete_when_the_kraft_sum_is_one() {
        // 1/2 + 1/4 + 1/8 + 1/8 is one, so four symbols of these lengths are a complete
        // code and the constructor must not call it incomplete.
        assert!(Table::canonical(&[1, 2, 3, 3]).is_ok());
        // One bit short of complete: a peek of all zeros could resolve to nothing.
        assert_eq!(
            Table::canonical(&[1, 2, 3]).unwrap_err(),
            CodeError::Incomplete
        );
        // One bit over.
        assert_eq!(
            Table::canonical(&[1, 1, 2]).unwrap_err(),
            CodeError::OverSubscribed
        );
        assert_eq!(canonical_codes(&[0, 0]).unwrap_err(), CodeError::NoSymbols);
    }

    #[test]
    fn unused_symbols_do_not_shift_the_codes() {
        // The Kraft sum counts only the symbols that appear, and so does the canonical
        // assignment. A code padded with trailing zero-length entries -- which is what a
        // 288-entry alphabet with a short code looks like -- must produce exactly the same
        // codes as the short alphabet.
        let short = canonical_codes(&[1, 2, 2]).unwrap();
        let padded = canonical_codes(&[1, 2, 2, 0, 0, 0, 0, 0]).unwrap();
        assert_eq!(short, padded[..3]);
        assert!(
            padded[3..]
                .iter()
                .all(|word| word.length == 0 && word.code == 0),
            "and the unused symbols carry no code"
        );
    }

    #[test]
    fn a_deeper_code_than_the_peek_cannot_be_tabulated() {
        // `canonical_codes` knows about DEFLATE's 15-bit limit...
        let mut lengths = vec![0u8; 16];
        lengths[0] = 1;
        lengths[1] = 15;
        lengths[2] = 15;
        assert!(canonical_codes(&lengths).is_ok());
        // ...but a `Table` is only TABLE_BITS deep.
        assert_eq!(
            Table::canonical(&lengths).unwrap_err(),
            CodeError::TooDeep { length: 15 }
        );
        assert_eq!(TABLE_BITS, 9);
        assert_eq!(MAX_CODE_LENGTH, 15);
    }

    #[test]
    fn the_single_symbol_code_is_one_bit_and_every_bit() {
        let table = Table::single(200);
        for index in 0..TABLE_ENTRIES {
            assert_eq!(table.entry(index as u16), (200, 1));
        }
        // The alphabet runs to 287 in DEFLATE, so a one-symbol code has to be able to name
        // a symbol an eight-bit port could not.
        assert_eq!(Table::single(287).entry(0), (287, 1));
    }

    #[test]
    fn the_fixed_codes_have_the_depth_the_peek_covers() {
        // The point of the constant: the RFC's fixed literal/length code is exactly as deep
        // as the table, which is why this module can hold it and a seven-bit table could
        // not.
        assert_eq!(FIXED_LITERAL_LENGTH_LENGTHS[143], 8);
        assert_eq!(FIXED_LITERAL_LENGTH_LENGTHS[144], 9);
        assert_eq!(FIXED_LITERAL_LENGTH_LENGTHS[256], 7);
        assert_eq!(FIXED_LITERAL_LENGTH_LENGTHS[287], 8);
        let deepest = FIXED_LITERAL_LENGTH_LENGTHS
            .iter()
            .copied()
            .max()
            .expect("the array is not empty");
        assert_eq!(u32::from(deepest), TABLE_BITS);
        assert_eq!(u32::from(deepest), BUFFER_BITS);
        assert!(Table::canonical(&FIXED_LITERAL_LENGTH_LENGTHS).is_ok());

        // The fixed distance code is five bits throughout and therefore exactly the kind
        // of code a shallow table is for.
        assert!(Table::canonical(&FIXED_DISTANCE_LENGTHS).is_ok());
    }
}
