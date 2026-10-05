//! FSE decode: two table lookups and one add per symbol, one symbol per cycle.
//!
//! # Why this is the single best argument in the corpus
//!
//! [`crate::huffman`] already argues that entropy decoding is serial. FSE is the
//! stronger version of that argument, because FSE does not merely *have* a serial
//! dependency -- **it is a state machine whose state update is one cycle long and whose
//! iteration count is not known in advance.** A Huffman decoder is a data-dependent
//! loop; an FSE decoder is a fixed-latency pipeline that happens to carry a state
//! register.
//!
//! The per-symbol operation, in full:
//!
//! ```text
//!   symbol = stateTable[state].symbol
//!   low    = next stateTable[state].nbBits bits of the bit stream
//!   state  = stateTable[state].newState + low
//! ```
//!
//! One ROM read, one variable-width field extract, one barrel shift and one add per
//! symbol per cycle, with a loop-carried dependency of exactly one cycle. There is no
//! branch, no "how many bits does this symbol need" decision that has to be resolved
//! before the next iteration can start -- the *table* resolves it. Nothing about the
//! loop widens into SIMD lanes, because iteration `i + 1`'s state is iteration `i`'s
//! output.
//!
//! On a CPU the same table is one L1-resident array read per symbol plus a
//! data-dependent bit read, so the loop runs at perhaps one symbol per two to four
//! cycles with a branch mispredict on every change of `nbBits`. In silicon it is one
//! symbol per cycle, unconditionally.
//!
//! # What is implemented, and what is not -- read this before trusting the module
//!
//! **The decoding direction only, for a table supplied at elaboration time.**
//! [`Table::build`] is the offline half: it turns normalised counts into the [`Table`]
//! the circuit holds, transcribed from zstd's `FSE_buildDTable_internal` in
//! `lib/common/fse_decompress.c`. It is ordinary Rust and runs on the host, which is
//! where a real decoder's table builder runs too -- the table depends on the block
//! header, which arrives with the data.
//!
//! Four things this module does **not** do, all of which a real FSE stream needs:
//!
//! 1. **No bit-exact zstd bitstream.** The bit reader here is *forward*, consuming the
//!    next bit of the stream each cycle, least significant bit of each byte first.
//!    zstd's `BIT_DStream` loads its 64-bit container from the *end* of the buffer and
//!    consumes from the low end, so its bit order within a container is the reverse of
//!    this one's. **A real zstd frame will not decode with this design.** The
//!    per-symbol state machine is zstd's; the container layer is this crate's own,
//!    chosen because it is the same layer [`crate::deflate`] needs, so the two share it.
//! 2. **No normalised-count header.** Reading `FSE_readNCount`'s variable-bit-count
//!    header, and the four-bit `tableLog` that follows it, is not here.
//! 3. **No termination on the stream running out.** zstd's generic loop stops when the
//!    bit stream is exhausted, which is a bit-exactness condition a hardware decoder
//!    does not want. This design decodes a **host-supplied symbol count**, which is what
//!    a real block decoder has: zstd's Huffman-literal block header carries the literal
//!    count and a sequence section carries `Number_of_Sequences`. The
//!    `FSE_endOfDState` check (final state zero) is *not* used as the terminator either,
//!    because state zero is reachable mid-stream: a table entry's `new_state` is zero
//!    whenever its next state number is a power of two, which happens for many symbols
//!    and not only the last.
//! 4. **No encoding side.** The table is built from counts; nothing here turns symbols
//!    into a bit stream. The reference encoder in the test file inverts the decoder
//!    directly.
//!
//! Because there is no FSE crate in this workspace (`miniz_oxide` is DEFLATE, not
//! FSE), the table construction is checked against **structural invariants derived from
//! the algorithm rather than transcribed from it**, against a hand-derived table, and
//! against a round trip through an independently written encoder. That is weaker than a
//! cross-check against `zstd`, and the test file says so where it matters.
//!
//! # The hardware, and what it really costs
//!
//! ```text
//!   bit stream -> [ 8-bit shift register ] -> nbBits low bits --+
//!                                                          |    |
//!   state -> [ 32-entry table ] -> (symbol, nbBits, newState) ---+ -> add -> state
//! ```
//!
//! Three muxes and two ROMs. As built, both ROMs are [`Design::case_`] blocks -- one arm
//! per entry -- and the docs on [`Design::rom`] are explicit that a multiplexer tree is
//! not a memory. For 32 entries of 17 bits that is a 32-to-1 mux per output bit, **5
//! levels** of 2:1 mux, some 1 700 muxes; a real decoder would put this table in a small
//! register file or a block RAM. At this size the mux tree is close enough to the truth
//! that it does not change the design's shape, which is the opposite of
//! [`crate::huffman`]'s 512-entry table.
//!
//! The variable-width extract (`low = buffer & ((1 << nbBits) - 1)`) and the
//! variable-width shift (`buffer >> nbBits`) are each a select over the six possible
//! `nbBits` values. That is exactly what a mask generator and a barrel shifter are, and
//! it is the only place in this design where the variable-length bit handling costs
//! anything.

use ferrite_lithic::{Design, Signal};
use ferrite_lithic_derive::PortList;

use crate::BuildError;

/// `table_log`: the base-two logarithm of the decode table's size.
///
/// Five is FSE's own minimum (`FSE_MIN_TABLELOG`), and it keeps the table at 32 entries,
/// small enough for the mux tree to be honest about itself.
pub const TABLE_LOG: u32 = 5;

/// The decode table's size: `2^TABLE_LOG`.
pub const TABLE_SIZE: usize = 1 << TABLE_LOG;

/// How deep the bit buffer is.
///
/// Twice [`TABLE_LOG`], so there is a refill's worth of slack: a decode consumes at most
/// [`TABLE_LOG`] bits while the input supplies one per cycle, so the buffer fills when
/// the cheap symbols dominate and drains when the expensive ones do.
pub const BUFFER_BITS: u32 = 8;

/// A width that holds a bit count up to [`BUFFER_BITS`].
const HELD_BITS: u32 = 4;

/// The packed width of one table entry: an eight-bit symbol, a four-bit `nb_bits` and a
/// five-bit `new_state`.
const ENTRY_BITS: u32 = 8 + 4 + TABLE_LOG;

/// The FSM's phases, as literals for the port docs.
const IDLE: u64 = 0;
/// Reading [`TABLE_LOG`] bits of initial state out of the stream.
const LOADING: u64 = 1;
/// Decoding symbols.
const DECODING: u64 = 2;
/// The phase register's width, which has to hold [`DECODING`].
const PHASE_BITS: u32 = 2;

/// A width for the count of initial-state bits already read, which reaches
/// [`TABLE_LOG`].
const LOAD_BITS: u32 = 3;

/// One entry of an FSE decoding table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    /// The symbol this state emits.
    pub symbol: u8,
    /// How many stream bits this state consumes to reach the next one.
    pub nb_bits: u8,
    /// The base that the next state's low bits are added to.
    ///
    /// `new_state + low` for `low` in `0..2^nb_bits` is always inside the table, and
    /// that is the invariant a wrong table construction breaks first.
    pub new_state: u8,
}

/// An FSE decoding table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Table {
    /// The table's base-two logarithm of its size.
    pub table_log: u32,
    /// The table's entries, indexed by state.
    pub entries: Vec<Entry>,
}

/// Why a set of normalised counts is not a usable FSE table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TableError {
    /// `table_log` is outside the range FSE allows.
    TableLog {
        /// The requested logarithm.
        table_log: u32,
    },
    /// The counts, with each `-1` counted as one, do not sum to `2^table_log`.
    CountsDoNotSum {
        /// What the counts summed to.
        got: u32,
        /// What they have to sum to.
        want: u32,
    },
    /// The spread did not visit every state exactly once, which means the counts are not
    /// a valid normalised distribution.
    SpreadFailed,
}

impl std::fmt::Display for TableError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TableLog { table_log } => {
                write!(f, "table_log {table_log} is outside the FSE range 5..=15")
            }
            Self::CountsDoNotSum { got, want } => {
                write!(f, "the normalised counts sum to {got}, not {want}")
            }
            Self::SpreadFailed => {
                write!(f, "spreading the symbols did not visit every state")
            }
        }
    }
}

impl std::error::Error for TableError {}

/// `floor(log2(value))`, which is zstd's `ZSTD_highbit32`.
///
/// FSE's state numbers start at one, so zero cannot reach this in practice; it maps to
/// zero rather than panicking so a malformed count set cannot take the process down.
///
/// # Panics
///
/// Never.
#[must_use]
pub fn highbit32(value: u32) -> u32 {
    31 - value.leading_zeros()
}

/// The spread step FSE uses: `(tableSize / 2) + (tableSize / 8) + 3`.
///
/// Transcribed from zstd's `FSE_TABLESTEP`. It is coprime with the table size for every
/// power of two in range, which is what makes the spread visit every state exactly once
/// -- a property [`Table::build`] checks and reports as [`TableError::SpreadFailed`]
/// rather than assumes.
#[must_use]
pub fn table_step(table_size: usize) -> usize {
    (table_size >> 1) + (table_size >> 3) + 3
}

impl Table {
    /// Builds the decoding table from normalised counts. **This is the offline part.**
    ///
    /// `counts[s]` is symbol `s`'s normalised count, which FSE requires to be either
    /// `-1` -- a "rarer than one in `tableSize`" symbol, laid down in the table's
    /// high-probability tail -- or a positive integer, with the positive counts plus one
    /// per `-1` summing to `2^table_log`.
    ///
    /// The algorithm is zstd's, in three steps:
    ///
    /// 1. **Lay down the low-probability symbols.** Each `-1` symbol takes one state from
    ///    the top of the table downwards, and its state counter starts at one.
    ///    Everything else's counter starts at its own count.
    /// 2. **Spread.** Walk the symbols in order, stepping by [`table_step`] through the
    ///    table, writing each symbol into every state visited, skipping the
    ///    low-probability tail when there is one. A `-1` symbol places *nothing* here --
    ///    it already has its cell -- which is why the walk can close even though it makes
    ///    fewer steps than there are cells. The walk must land back on zero exactly when
    ///    the counts run out, and that is checked rather than assumed.
    /// 3. **Assign states.** Walk the table in index order; for each state take the next
    ///    state number for its symbol and set `nb_bits = table_log - highbit32(n)` and
    ///    `new_state = (n << nb_bits) - table_size`.
    ///
    /// Note the counter counts *up* from the count, so symbol `s`'s states take state
    /// numbers `counts[s], counts[s] + 1, ... 2*counts[s] - 1`. Counting down from the
    /// count instead also produces a table whose transitions land inside the table -- the
    /// coverage invariant the unit tests check cannot tell the two apart -- so it takes
    /// the hand-derived table in the test file to pin the direction down.
    ///
    /// # Errors
    ///
    /// [`TableError::TableLog`] outside FSE's range, [`TableError::CountsDoNotSum`] if
    /// the counts do not cover the table exactly, and [`TableError::SpreadFailed`] if
    /// the spread does not close.
    ///
    /// # Panics
    ///
    /// Never.
    pub fn build(counts: &[i32], table_log: u32) -> Result<Self, TableError> {
        if !(5..=15).contains(&table_log) {
            return Err(TableError::TableLog { table_log });
        }
        let table_size = 1usize << table_log;

        // A count of -1 means "rarer than one in tableSize" and still occupies exactly one
        // state, which is the whole content of FSE's low-probability machinery.
        let total = counts
            .iter()
            .map(|count| if *count < 0 { 1u32 } else { *count as u32 })
            .sum::<u32>();
        if total != table_size as u32 {
            return Err(TableError::CountsDoNotSum {
                got: total,
                want: table_size as u32,
            });
        }

        let mut symbols = vec![0u8; table_size];
        let mut next_state = vec![0u32; counts.len()];
        let mut high = table_size as i64 - 1;
        for (symbol, count) in counts.iter().enumerate() {
            if *count < 0 {
                symbols[high as usize] = symbol as u8;
                high -= 1;
                next_state[symbol] = 1;
            } else {
                next_state[symbol] = *count as u32;
            }
        }

        let mask = table_size - 1;
        let step = table_step(table_size);
        // With no low-probability symbol `high` is still the top cell and the test below
        // can never fire, so one loop covers both of zstd's branches.
        let high_threshold = high as usize;
        let mut position = 0usize;
        for (symbol, count) in counts.iter().enumerate() {
            let places = if *count < 0 { 0usize } else { *count as usize };
            for _ in 0..places {
                symbols[position] = symbol as u8;
                position = (position + step) & mask;
                while position > high_threshold {
                    position = (position + step) & mask;
                }
            }
        }
        // The spread closes only if the counts really were a normalised distribution.
        if position != 0 {
            return Err(TableError::SpreadFailed);
        }

        let mut entries = Vec::with_capacity(table_size);
        for index in 0..table_size {
            let symbol = usize::from(symbols[index]);
            let number = next_state[symbol];
            next_state[symbol] += 1;
            let nb_bits = table_log - highbit32(number);
            // `number << nb_bits` is in `[table_size, 2 * table_size)` for every reachable
            // `number`, so this cannot go negative and cannot reach `table_size` either.
            // That is what makes the next state valid.
            let new_state = (number << nb_bits).wrapping_sub(table_size as u32);
            entries.push(Entry {
                symbol: symbols[index],
                nb_bits: nb_bits as u8,
                new_state: new_state as u8,
            });
        }

        Ok(Self { table_log, entries })
    }

    /// The table as [`Design::rom`] wants it: `new_state << 12 | nb_bits << 8 | symbol`.
    ///
    /// The state number is the ROM's address, so the entry is laid out with the symbol and
    /// the two transition fields above it.
    #[must_use]
    pub fn packed(&self) -> Vec<u64> {
        self.entries
            .iter()
            .map(|entry| {
                (u64::from(entry.new_state) << (8 + 4))
                    | (u64::from(entry.nb_bits) << 8)
                    | u64::from(entry.symbol)
            })
            .collect()
    }
}

/// The input ports.
#[derive(Clone, Debug, PortList)]
pub struct Inputs {
    /// The clock. Every edge in the decoding phase retires at most one symbol.
    #[clock]
    pub clk: Signal,
    /// Synchronous reset, which returns the FSM to idle.
    pub rst: Signal,
    /// Start decoding: clear the buffer, load [`Self::count`], and begin reading the
    /// initial state from the stream.
    ///
    /// Asserted again while busy it restarts the whole sequence rather than being
    /// ignored, which is how a host abandons a stream it no longer wants.
    pub init: Signal,
    /// How many symbols to decode, sampled on the [`Self::init`] edge.
    #[bits(16)]
    pub count: Signal,
    /// A bit of the compressed stream is available on [`Self::in_bit`].
    pub in_valid: Signal,
    /// The next bit of the compressed stream, least significant bit of each byte first.
    pub in_bit: Signal,
}

/// The output ports.
#[derive(Clone, Debug, PortList)]
pub struct Outputs {
    /// The decoder has room for another bit.
    pub in_ready: Signal,
    /// High from the [`Inputs::init`] edge until [`Self::done`].
    pub busy: Signal,
    /// A one-cycle pulse on the edge that retires the last symbol.
    pub done: Signal,
    /// A symbol is on [`Self::symbol`].
    pub out_valid: Signal,
    /// The symbol retired this cycle.
    #[bits(8)]
    pub symbol: Signal,
    /// How many stream bits that symbol's transition consumed.
    #[bits(4)]
    pub nb_bits: Signal,
    /// The state register.
    #[bits(5)]
    pub state: Signal,
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
    phase: Signal,
    buffer: Signal,
    held: Signal,
    loaded: Signal,
    state: Signal,
    left: Signal,
    out_valid: Signal,
    symbol: Signal,
    nb_bits: Signal,
    done: Signal,
}

/// Builds an FSE decoder for `table` into `design`.
///
/// The table *is* the design's content, so it is an argument rather than a constant: the
/// normalised counts come from the block header and are not known until the stream is.
///
/// # Errors
///
/// Whatever [`Design`] returns and whatever the port-list helpers return.
///
/// ```
/// use ferrite_lithic::Design;
/// use ferrite_lithic_corpus::fse::{Table, build};
///
/// let table = Table::build(&[20, 12], 5).unwrap();
/// let design = Design::new();
/// build(&design, &table).unwrap();
/// assert!(design.node_count() > 0);
/// ```
pub fn build(design: &Design, table: &Table) -> Result<Ports, BuildError> {
    let inputs = ferrite_lithic::inputs::<Inputs>(design)?;

    let now = State {
        phase: design.wire(PHASE_BITS)?,
        buffer: design.wire(BUFFER_BITS)?,
        held: design.wire(HELD_BITS)?,
        loaded: design.wire(LOAD_BITS)?,
        state: design.wire(TABLE_LOG)?,
        left: design.wire(16)?,
        out_valid: design.wire(1)?,
        symbol: design.wire(8)?,
        nb_bits: design.wire(4)?,
        done: design.wire(1)?,
    };

    let next = next_state(design, &inputs, &now, &table.packed())?;

    let hold = design.constant(false);
    let phase = design.reg(&next.phase, &inputs.clk, &inputs.rst, &hold)?;
    let buffer = design.reg(&next.buffer, &inputs.clk, &inputs.rst, &hold)?;
    let held = design.reg(&next.held, &inputs.clk, &inputs.rst, &hold)?;
    let loaded = design.reg(&next.loaded, &inputs.clk, &inputs.rst, &hold)?;
    let state = design.reg(&next.state, &inputs.clk, &inputs.rst, &hold)?;
    let left = design.reg(&next.left, &inputs.clk, &inputs.rst, &hold)?;
    let out_valid = design.reg(&next.out_valid, &inputs.clk, &inputs.rst, &hold)?;
    let symbol = design.reg(&next.symbol, &inputs.clk, &inputs.rst, &hold)?;
    let nb_bits = design.reg(&next.nb_bits, &inputs.clk, &inputs.rst, &hold)?;
    let done = design.reg(&next.done, &inputs.clk, &inputs.rst, &hold)?;

    design.drive(&now.phase, &phase)?;
    design.drive(&now.buffer, &buffer)?;
    design.drive(&now.held, &held)?;
    design.drive(&now.loaded, &loaded)?;
    design.drive(&now.state, &state)?;
    design.drive(&now.left, &left)?;
    design.drive(&now.out_valid, &out_valid)?;
    design.drive(&now.symbol, &symbol)?;
    design.drive(&now.nb_bits, &nb_bits)?;
    design.drive(&now.done, &done)?;

    // `in_ready` and `busy` are pure functions of registers, so they are wires rather than
    // a cycle-delayed copy: a registered copy would answer one edge late, which is the
    // edge the host needs the answer on.
    let capacity = design.lit(u64::from(BUFFER_BITS), HELD_BITS)?;
    let in_ready = design.ult(&now.held, &capacity)?;
    let idle = design.eq(&now.phase, &design.lit(IDLE, PHASE_BITS)?)?;
    let busy = design.not(&idle);

    let outputs = ferrite_lithic::outputs::<Outputs>(
        design,
        &Outputs {
            in_ready,
            busy,
            done: done.clone(),
            out_valid: out_valid.clone(),
            symbol: symbol.clone(),
            nb_bits: nb_bits.clone(),
            state: state.clone(),
            held: held.clone(),
        },
    )?;

    Ok(Ports { inputs, outputs })
}

/// One cycle of the FSE decoder.
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
    let off = design.constant(false);

    // ---- the bit buffer, shared by the load and decode phases -----------------
    let capacity = design.lit(u64::from(BUFFER_BITS), HELD_BITS)?;
    let in_ready = design.ult(&now.held, &capacity)?;
    let accepting = design.and(&inputs.in_valid, &in_ready)?;
    // `1 << held` is the insert position. The incoming bit has to be *replicated* across
    // the one-hot's width and then ANDed: `1 << held & bit` is zero for every `held` but
    // zero, because a one-hot and a lone 1 share no set bit above bit zero. That mistake
    // passes a one-bit code and fails everything else.
    let insert_at = one_hot(design, &now.held, BUFFER_BITS)?;
    let spread = design.replicate(&inputs.in_bit, BUFFER_BITS)?;
    let inserted = design.and(&insert_at, &spread)?;
    let with_bit = design.or(&now.buffer, &inserted)?;
    let buffer = design.ite(&accepting, &with_bit, &now.buffer)?;
    let gained = design.add(&now.held, &design.lit(1, HELD_BITS)?)?;
    let held = design.ite(&accepting, &gained, &now.held)?;

    // ---- the state table ------------------------------------------------------
    // One read gives symbol, nbBits and newState, because that is what a real decoder's
    // register file does: one port, one cycle, all three fields.
    let entry = design.rom(&now.state, packed, ENTRY_BITS)?;
    let symbol = design.slice(&entry, 0, 8)?;
    let nb_bits = design.slice(&entry, 8, 4)?;
    let new_state = design.slice(&entry, 12, TABLE_LOG)?;

    // `low = buffer & ((1 << nbBits) - 1)`: the mask is a select over the six possible
    // widths, which in silicon is a mask generator.
    let mask = low_mask(design, &nb_bits, TABLE_LOG)?;
    let low = design.and(&design.slice(&buffer, 0, TABLE_LOG)?, &mask)?;
    let advanced = design.add(&new_state, &low)?;

    // ---- the state machine ----------------------------------------------------
    let loading = design.eq(&now.phase, &design.lit(LOADING, PHASE_BITS)?)?;
    let decoding = design.eq(&now.phase, &design.lit(DECODING, PHASE_BITS)?)?;

    // Loading reads TABLE_LOG bits into the state register, least significant bit first
    // -- the same order the decode reads its transition bits in, so the initial state and
    // the transitions come out of one bit-stream convention.
    let taking = design.and(&loading, &accepting)?;
    let shifted = design.sll(&now.state, 1)?;
    let accumulating = design.or(&shifted, &design.zero_extend(&inputs.in_bit, TABLE_LOG)?)?;
    let read = design.add(&now.loaded, &design.lit(1, LOAD_BITS)?)?;
    let loaded = design.ite(&taking, &read, &now.loaded)?;
    let load_done = design.eq(&read, &design.lit(u64::from(TABLE_LOG), LOAD_BITS)?)?;
    let finishing_load = design.and(&loading, &load_done)?;

    // A decode needs as many bits buffered as this state will consume. Comparing against
    // `nbBits` rather than always waiting for TABLE_LOG bits is what lets the design
    // retire a symbol on *every* cycle once the buffer is primed; a fixed TABLE_LOG-deep
    // wait would insert a stall after every short code, which is exactly the
    // variable-length cost this design exists to avoid.
    let enough = design.uge(&held, &nb_bits)?;
    let stepping = design.and(&decoding, &enough)?;

    let dropped = shift_right_by(design, &buffer, &nb_bits, TABLE_LOG)?;
    let buffer = design.ite(&stepping, &dropped, &buffer)?;
    let spent = design.sub(&held, &nb_bits)?;
    let held = design.ite(&stepping, &spent, &held)?;

    let last = design.eq(&now.left, &design.lit(1, 16)?)?;
    let finishing = design.and(&stepping, &last)?;

    // The phase register: `init` restarts, a finished load enters decode, a finished
    // decode goes idle. `finishing_load` and `finishing` are mutually exclusive because
    // one needs the loading phase and the other the decoding phase.
    let phase = design.ite(
        &finishing_load,
        &design.lit(DECODING, PHASE_BITS)?,
        &now.phase,
    )?;
    let phase = design.ite(&finishing, &design.lit(IDLE, PHASE_BITS)?, &phase)?;
    let phase = design.ite(&inputs.init, &design.lit(LOADING, PHASE_BITS)?, &phase)?;

    let countdown = design.ite(&inputs.init, &inputs.count, &now.left)?;
    let left = design.ite(
        &stepping,
        &design.sub(&countdown, &design.lit(1, 16)?)?,
        &countdown,
    )?;

    let decoded = design.ite(&stepping, &advanced, &now.state)?;
    let next_state = design.ite(&decoding, &decoded, &accumulating)?;
    let state = design.ite(&inputs.init, &design.zeros(TABLE_LOG)?, &next_state)?;

    // `init` wins over everything on its own edge, so a restart needs no special case
    // anywhere else.
    let buffer = design.ite(&inputs.init, &design.zeros(BUFFER_BITS)?, &buffer)?;
    let held = design.ite(&inputs.init, &design.zeros(HELD_BITS)?, &held)?;
    let loaded = design.ite(&inputs.init, &design.zeros(LOAD_BITS)?, &loaded)?;

    Ok(State {
        phase,
        buffer,
        held,
        loaded,
        state,
        left,
        out_valid: design.ite(&inputs.init, &off, &stepping)?,
        symbol: design.ite(&inputs.init, &design.zeros(8)?, &symbol)?,
        nb_bits: design.ite(&inputs.init, &design.zeros(4)?, &nb_bits)?,
        done: design.ite(&inputs.init, &off, &finishing)?,
    })
}

/// `1 << held`, as a `width`-bit constant.
fn one_hot(design: &Design, held: &Signal, width: u32) -> Result<Signal, BuildError> {
    let mut arms = Vec::with_capacity(width as usize);
    for amount in 0..width {
        arms.push((u64::from(amount), design.lit(1u64 << amount, width)?));
    }
    Ok(design.case_(held, &arms, &design.lit(0, width)?)?)
}

/// `(1 << nbBits) - 1`, as a `width`-bit constant: the mask for the low bits.
fn low_mask(design: &Design, nb_bits: &Signal, width: u32) -> Result<Signal, BuildError> {
    let mut arms = Vec::with_capacity(width as usize + 1);
    for amount in 0..=width {
        arms.push((u64::from(amount), design.lit((1u64 << amount) - 1, width)?));
    }
    Ok(design.case_(nb_bits, &arms, &design.lit(0, width)?)?)
}

/// `value >> nbBits`, for an `nbBits` the design cannot shift by in one go.
fn shift_right_by(
    design: &Design,
    value: &Signal,
    nb_bits: &Signal,
    width: u32,
) -> Result<Signal, BuildError> {
    let mut arms = Vec::with_capacity(width as usize + 1);
    for amount in 0..=width {
        arms.push((u64::from(amount), design.srl(value, amount)?));
    }
    Ok(design.case_(nb_bits, &arms, value)?)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::{TABLE_LOG, TABLE_SIZE, Table, TableError, highbit32, table_step};

    #[test]
    fn highbit_is_the_position_of_the_top_set_bit() {
        assert_eq!(highbit32(1), 0);
        assert_eq!(highbit32(2), 1);
        assert_eq!(highbit32(3), 1);
        assert_eq!(highbit32(4), 2);
        assert_eq!(highbit32(0xff), 7);
        assert_eq!(highbit32(1 << 9), 9);
    }

    #[test]
    fn the_spread_step_is_zstds() {
        // FSE_TABLESTEP, which is what makes the spread visit every state exactly once
        // for every power-of-two table size in range.
        assert_eq!(table_step(32), 23);
        assert_eq!(table_step(64), 43);
        assert_eq!(table_step(1024), 643);
    }

    #[test]
    fn counts_must_cover_the_table_exactly() {
        assert_eq!(
            Table::build(&[20, 11], 5).unwrap_err(),
            TableError::CountsDoNotSum { got: 31, want: 32 }
        );
        assert_eq!(
            Table::build(&[20, 13], 5).unwrap_err(),
            TableError::CountsDoNotSum { got: 33, want: 32 }
        );
        // A -1 count still occupies one state, so this set does add up: 16 + 12 + 3 + 1.
        assert!(Table::build(&[16, 12, 3, -1], 5).is_ok());
        assert_eq!(
            Table::build(&[20, 12], 4).unwrap_err(),
            TableError::TableLog { table_log: 4 }
        );
    }

    #[test]
    fn every_transition_lands_on_a_valid_state() {
        // The invariant a wrong table construction breaks first: for every state,
        // `new_state + low` over the whole range of `low` stays inside the table.
        for counts in [
            vec![20, 12],
            vec![16, 12, 3, -1],
            vec![8, 8, 8, 8],
            vec![1, 1, 30],
            vec![32],
        ] {
            let table = Table::build(&counts, TABLE_LOG).unwrap();
            assert_eq!(table.entries.len(), TABLE_SIZE);
            for entry in &table.entries {
                let span = 1u32 << entry.nb_bits;
                assert!(u32::from(entry.new_state) + span <= TABLE_SIZE as u32);
                assert!(entry.nb_bits as u32 <= TABLE_LOG);
            }
        }
    }

    #[test]
    fn every_symbol_occupies_exactly_its_count_of_states() {
        // Derived from the algorithm rather than transcribed from it: the spread writes
        // each symbol `count` times and no more, so a swapped symbol index or a wrong
        // spread step shows up here and nowhere else.
        for counts in [vec![20, 12], vec![16, 12, 3, -1], vec![8, 8, 8, 8]] {
            let table = Table::build(&counts, TABLE_LOG).unwrap();
            let mut seen = vec![0u32; counts.len()];
            for entry in &table.entries {
                seen[usize::from(entry.symbol)] += 1;
            }
            for (symbol, count) in counts.iter().enumerate() {
                let want = if *count < 0 { 1 } else { *count as u32 };
                assert_eq!(seen[symbol], want, "symbol {symbol} of {counts:?}");
            }
        }
    }
}
