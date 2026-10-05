//! FSE decode and the DEFLATE bit layer.
//!
//! # Two designs with two very different kinds of evidence
//!
//! [`fse`] has **no golden crate at all** -- `miniz_oxide` is DEFLATE, not FSE, and
//! `zstd` is not a dependency here, so there is nothing in this workspace to check an
//! FSE table against. What stands in for it is written out honestly below.
//!
//! [`deflate`] has the opposite problem: a real, world-class implementation sitting
//! right there in `miniz_oxide`, one layer above the bit reader. That makes the round
//! trip the strongest available test, and the file leans on it hard.
//!
//! # What the FSE evidence is worth -- stated plainly
//!
//! In order of strength:
//!
//! 1. **A hand-derived decoding table**, checked entry by entry. The spread step and the
//!    state-number assignment are derived by hand from the algorithm and written down as
//!    literals, so `Table::build` is checked against arithmetic done independently in
//!    this file rather than against itself. This is the only check that can tell an
//!    upward state counter from a downward one, because both produce a table whose
//!    transitions land inside the table.
//! 2. **Invariants derived from the algorithm rather than transcribed from it**: for
//!    every symbol and every target state at least one predecessor state exists (this is
//!    what the encoder below relies on, and a table construction mistake breaks it);
//!    each symbol's states number exactly its count; and each symbol's `nb_bits`
//!    multiset is the one the algorithm's own state-number arithmetic forces.
//! 3. **A round trip through an independently written encoder**, which inverts the
//!    decoder directly rather than reimplementing FSE's encoder.
//!
//! What none of that is: a cross-check against a real zstd stream. A construction that
//! misreads the specification in the same way the reference decoder does would round trip
//! perfectly and be wrong. [`fse`]'s module docs say the same thing, and it is the
//! honest ceiling for this workspace.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::VecDeque;

use ferrite_lithic::Design;
use ferrite_lithic_bits::Bits;
use ferrite_lithic_corpus::{deflate, fse};
use ferrite_lithic_cosim::{Harness, Plan, Stimulus, verilator};
use ferrite_lithic_tb::{Handle, Testbench};
use proptest::prelude::*;

// =======================================================================================
// FSE: a reference decoder, and an encoder that inverts it.
// =======================================================================================

/// Pushes `width` bits of `value`, least significant bit first.
fn push_bits(bits: &mut Vec<bool>, value: u32, width: u32) {
    for position in 0..width {
        bits.push(((value >> position) & 1) == 1);
    }
}

/// The decoding direction, in plain Rust: read the initial state, then one symbol and one
/// transition per iteration.
///
/// Independent of the design in the sense that matters: it indexes `entries` directly
/// where the design reads them out of a packed ROM field, and its bit cursor is a `usize`
/// where the design's is a shift register and a count.
fn reference_decode(table: &fse::Table, bits: &[bool], count: usize) -> Vec<u8> {
    let mut at = 0usize;
    let mut take = |width: u32| -> u32 {
        let mut value = 0u32;
        for position in 0..width {
            value |= u32::from(bits[at + position as usize]) << position;
        }
        at += width as usize;
        value
    };
    let mut state = take(table.table_log) as usize;
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        let entry = table.entries[state];
        let low = take(u32::from(entry.nb_bits));
        out.push(entry.symbol);
        state = usize::from(entry.new_state) + low as usize;
    }
    out
}

/// The encoding direction, by inverting the decoder.
///
/// FSE's own encoder walks a *different* table -- a per-symbol `deltaNbBits` /
/// `deltaFindState` pair plus a state table -- and reproducing that would be a second
/// transcription of the specification rather than a check on the first. So instead the
/// encoder here solves the decoder's own equation: to arrive at state `t` emitting symbol
/// `s`, find a state `u` with `symbolTable[u].symbol == s` and `t` in
/// `[newState(u), newState(u) + 2^nbBits(u))`. Every `(symbol, state)` pair has at least
/// one such `u`, which `every_symbol_and_target_state_has_a_predecessor` checks
/// exhaustively, and the encoder picks the lowest-numbered one. It is not unique, so this
/// is not zstd's encoder and does not produce zstd's bytes -- it produces a stream *this*
/// decoder accepts.
///
/// The walk is backwards, because FSE's bit stream is a LIFO: the initial state is the
/// first thing in the stream and the last symbol's transition is the last.
fn reference_encode(table: &fse::Table, symbols: &[u8]) -> Vec<bool> {
    let mut transitions: Vec<(u32, u32)> = Vec::with_capacity(symbols.len());
    let mut target = 0usize;
    for symbol in symbols.iter().rev() {
        let entry = (0..table.entries.len())
            .map(|index| (index, table.entries[index]))
            .find(|(_, entry)| {
                entry.symbol == *symbol
                    && usize::from(entry.new_state) <= target
                    && target < usize::from(entry.new_state) + (1usize << entry.nb_bits)
            })
            .map(|(index, _)| index)
            .expect("every symbol and target state has a predecessor");
        let found = table.entries[entry];
        transitions.push((
            (target - usize::from(found.new_state)) as u32,
            u32::from(found.nb_bits),
        ));
        target = entry;
    }

    let mut bits = Vec::new();
    push_bits(&mut bits, target as u32, table.table_log);
    for (value, width) in transitions.iter().rev() {
        push_bits(&mut bits, *value, *width);
    }
    bits
}

/// Scales `weights` into counts that sum to `1 << table_log`, every count positive.
///
/// Deliberately **not** FSE's `FSE_normalizeCount`, which has its own rounding rule and
/// the low-probability `-1` case. All a property test needs is a distribution the table
/// builder will accept, and adding a second normalisation to get one would add a second
/// thing to get wrong.
fn normalise(weights: &[u32], table_log: u32) -> Vec<i32> {
    let size = i64::from(1u32 << table_log);
    let total: i64 = weights.iter().map(|weight| i64::from(*weight)).sum();
    let mut counts: Vec<i64> = weights
        .iter()
        .map(|weight| ((i64::from(*weight) * size) / total).max(1))
        .collect();
    let biggest = |counts: &[i64]| -> usize {
        counts
            .iter()
            .enumerate()
            .max_by_key(|(_, count)| **count)
            .map(|(index, _)| index)
            .expect("at least one weight")
    };
    let mut sum: i64 = counts.iter().sum();
    loop {
        let index = biggest(&counts);
        if sum <= size || counts[index] <= 1 {
            break;
        }
        counts[index] -= 1;
        sum -= 1;
    }
    while sum < size {
        let index = biggest(&counts);
        counts[index] += 1;
        sum += 1;
    }
    counts.into_iter().map(|count| count as i32).collect()
}

/// One run of the FSE design.
struct FseRun {
    symbols: Vec<u8>,
    nb_bits: Vec<u8>,
    /// The state register's last value.
    final_state: u64,
    /// Whether `done` ever pulsed.
    saw_done: bool,
    /// Whether `busy` was ever high.
    saw_busy: bool,
    /// The longest run of consecutive cycles with `out_valid` high.
    longest_run: usize,
}

async fn drive_fse(tb: &Handle, bits: &[bool], count: usize) {
    tb.set("count", count as u64).await;
    tb.set("init", 1).await;
    tb.step().await;
    tb.set("init", 0).await;
    let mut queue: VecDeque<bool> = bits.iter().copied().collect();
    while let Some(bit) = queue.front().copied() {
        tb.set("in_bit", u64::from(bit)).await;
        // `peek` reads the settled value, which is what the design latches on the edge
        // that follows. Presenting a bit on a cycle where `in_ready` is low would lose it,
        // and the loss would surface several symbols later.
        let ready = tb.value("in_ready") == 1;
        tb.set("in_valid", u64::from(ready)).await;
        tb.step().await;
        if ready {
            queue.pop_front();
        }
    }
    // Drop `in_valid` before draining: leaving it asserted feeds whatever is on `in_bit`
    // again, and the design would quite correctly decode it.
    tb.set("in_valid", 0).await;
    // Drain until the design says it is finished rather than for a fixed number of
    // cycles. Up to `BUFFER_BITS` bits can still be buffered, each of which may retire
    // another symbol, so the honest bound is the design's own `done` -- and a fixed guess
    // is a test that passes on the messages it happened to be written for and truncates
    // the rest.
    for _ in 0..64 {
        if tb.value("done") == 1 {
            break;
        }
        tb.step().await;
    }
}

fn collect_fse(tb: &Testbench) -> FseRun {
    let valid = tb.series("out_valid");
    let symbols = tb.series("symbol");
    let nb_bits = tb.series("nb_bits");
    let done = tb.series("done");
    let busy = tb.series("busy");
    let state = tb.series("state");

    let mut run = FseRun {
        symbols: Vec::new(),
        nb_bits: Vec::new(),
        final_state: state.last().map_or(0, |bits| bits.to_u64().unwrap_or(0)),
        saw_done: false,
        saw_busy: false,
        longest_run: 0,
    };
    let mut streak = 0usize;
    for index in 0..valid.len() {
        if done[index].to_u64().expect("one bit") == 1 {
            run.saw_done = true;
        }
        if busy[index].to_u64().expect("one bit") == 1 {
            run.saw_busy = true;
        }
        if valid[index].to_u64().expect("one bit") == 1 {
            run.symbols
                .push(symbols[index].to_u64().expect("eight bits") as u8);
            run.nb_bits
                .push(nb_bits[index].to_u64().expect("four bits") as u8);
            streak += 1;
            run.longest_run = run.longest_run.max(streak);
        } else {
            streak = 0;
        }
    }
    run
}

fn hardware_fse(table: &fse::Table, symbols: &[u8]) -> FseRun {
    let bits = reference_encode(table, symbols);
    let design = Design::new();
    fse::build(&design, table).unwrap();
    let tb = Testbench::new(&design).unwrap();
    tb.run(|tb| async move {
        tb.pulse(ferrite_lithic_corpus::RESET).await;
        drive_fse(&tb, &bits, symbols.len()).await;
    })
    .expect("the fse design drives only ports it declares");
    collect_fse(&tb)
}

// ---------------------------------------------------------------------------------------
// The hand-derived table.
// ---------------------------------------------------------------------------------------

/// The decoding table for normalised counts `[20, 12]` at `tableLog = 5`, derived by hand
/// and written down as literals.
///
/// The derivation, so a reader can check it rather than trust it:
///
/// * `tableSize = 32`, `tableStep = 32/2 + 32/8 + 3 = 23`, `tableMask = 31`.
/// * Spreading symbol 0 twenty times from position 0 by 23 lands it on states
///   `{0,1,2,5,6,7,10,11,14,15,16,19,20,21,23,24,25,28,29,30}`; symbol 1 takes the other
///   twelve, `{3,4,8,9,12,13,17,18,22,26,27,31}`. The walk ends back at 0, which is the
///   check that the counts were a valid distribution.
/// * The state counters start at the counts and count *up*: symbol 0's states take
///   next-state numbers 20, 21, ... 39 and symbol 1's take 12, 13, ... 23.
/// * `nbBits = 5 - floor(log2(nextState))` and `newState = (nextState << nbBits) - 32`.
///   So next-state 20 gives one bit and 8; next-state 32 gives zero bits and 0; next-state
///   16 gives one bit and 0; next-state 1 gives five bits and 0.
///
/// The entries are then `(table[0].symbol, table[0].nbBits, table[0].newState)` and so on.
/// State 0 holds symbol 0 with `nbBits = 1` and `newState = 8`, which is what a
/// transposed field order, an off-by-one in the high-bit count, or a *downward* state
/// counter would each break.
const HAND_TABLE_COUNTS: [i32; 2] = [20, 12];
const HAND_TABLE: [(u8, u8, u8); 32] = [
    // (symbol, nbBits, newState)
    (0, 1, 8),
    (0, 1, 10),
    (0, 1, 12),
    (1, 2, 16),
    (1, 2, 20),
    (0, 1, 14),
    (0, 1, 16),
    (0, 1, 18),
    (1, 2, 24),
    (1, 2, 28),
    (0, 1, 20),
    (0, 1, 22),
    (1, 1, 0),
    (1, 1, 2),
    (0, 1, 24),
    (0, 1, 26),
    (0, 1, 28),
    (1, 1, 4),
    (1, 1, 6),
    (0, 1, 30),
    (0, 0, 0),
    (0, 0, 1),
    (1, 1, 8),
    (0, 0, 2),
    (0, 0, 3),
    (0, 0, 4),
    (1, 1, 10),
    (1, 1, 12),
    (0, 0, 5),
    (0, 0, 6),
    (0, 0, 7),
    (1, 1, 14),
];

#[test]
fn the_table_builder_reproduces_the_hand_derived_table() {
    let table = fse::Table::build(&HAND_TABLE_COUNTS, 5).unwrap();
    assert_eq!(table.entries.len(), 32);
    for (index, entry) in table.entries.iter().enumerate() {
        let (symbol, nb_bits, new_state) = HAND_TABLE[index];
        assert_eq!(
            (entry.symbol, entry.nb_bits, entry.new_state),
            (symbol, nb_bits, new_state),
            "state {index}"
        );
    }
}

#[test]
fn every_symbol_and_target_state_has_a_predecessor() {
    // The property the encoder above depends on, and the one a wrong table construction
    // breaks first: for every symbol and every target state, at least one state reaches
    // it. It is *not* unique -- several states can span the same target -- so the encoder
    // picks the lowest and this asserts existence, which is what correctness needs.
    for counts in [
        HAND_TABLE_COUNTS.to_vec(),
        vec![16, 12, 3, -1],
        vec![8, 8, 8, 8],
        vec![1, 1, 30],
        vec![17, 14, 1],
    ] {
        let table = fse::Table::build(&counts, 5).unwrap();
        for symbol in 0..counts.len() {
            if counts[symbol] <= 0 {
                continue;
            }
            for target in 0..table.entries.len() {
                assert!(
                    table.entries.iter().any(|entry| {
                        entry.symbol as usize == symbol
                            && usize::from(entry.new_state) <= target
                            && target < usize::from(entry.new_state) + (1usize << entry.nb_bits)
                    }),
                    "no state reaches {target} emitting symbol {symbol} of {counts:?}"
                );
            }
        }
    }
}

#[test]
fn the_nb_bits_of_each_symbol_is_forced_by_the_state_numbers() {
    // Derived from the algorithm's own arithmetic rather than transcribed from it: the
    // states of a symbol with count `c` take next-state numbers `c..2c`, so their `nbBits`
    // multiset is exactly `{table_log - floor(log2(n)) : n in c..2c}`. A spread that
    // assigned the wrong symbols to states, or a state counter that ran the other way,
    // would break this and nothing else.
    for counts in [
        HAND_TABLE_COUNTS.to_vec(),
        vec![16, 12, 3, -1],
        vec![8, 8, 8, 8],
        vec![1, 1, 30],
    ] {
        let table = fse::Table::build(&counts, 5).unwrap();
        for (symbol, count) in counts.iter().enumerate() {
            let count = i64::from(*count);
            let mut want: Vec<u8> = if count < 0 {
                vec![0]
            } else {
                (count..2 * count)
                    .map(|number| (5 - fse::highbit32(number as u32)) as u8)
                    .collect()
            };
            want.sort_unstable();
            let mut got: Vec<u8> = table
                .entries
                .iter()
                .filter(|entry| usize::from(entry.symbol) == symbol)
                .map(|entry| entry.nb_bits)
                .collect();
            got.sort_unstable();
            assert_eq!(got, want, "symbol {symbol} of {counts:?}");
        }
    }
}

#[test]
fn the_round_trip_recovers_the_input() {
    for counts in [
        HAND_TABLE_COUNTS.to_vec(),
        vec![16, 12, 3, -1],
        vec![8, 8, 8, 8],
        vec![1, 1, 30],
    ] {
        let table = fse::Table::build(&counts, 5).unwrap();
        // Only the symbols the table actually holds, and only those with a positive
        // count: a `-1` symbol occupies a single state that returns to state zero, so it
        // cannot be *reached* by an arbitrary transition and asking the encoder for one
        // is asking for a stream no decoder could produce. `index % counts.len()` would
        // also index past a two-symbol table on a four-symbol message, which is a bug in
        // the test rather than a finding about the design.
        let present: Vec<u8> = counts
            .iter()
            .enumerate()
            .filter(|(_, count)| **count > 0)
            .map(|(symbol, _)| symbol as u8)
            .collect();
        let symbols: Vec<u8> = (0..200u32)
            .map(|index| present[index as usize % present.len()])
            .collect();
        let bits = reference_encode(&table, &symbols);
        assert_eq!(
            reference_decode(&table, &bits, symbols.len()),
            symbols,
            "the plain-Rust round trip for {counts:?}"
        );
        let run = hardware_fse(&table, &symbols);
        assert_eq!(
            run.symbols, symbols,
            "the design's round trip for {counts:?}"
        );
        assert!(run.saw_done, "and done pulses on the last symbol");
        assert!(run.saw_busy);
    }
}

#[test]
fn the_design_decodes_a_hand_computed_stream() {
    // One message, from the hand-derived table. The decoder reads five bits of initial
    // state and then `nbBits` bits of transition. State 0 holds symbol 0 with `nbBits = 1`
    // and `newState = 8`, so an initial state of 0 emits symbol 0, and reading one bit of
    // 1 takes the state to 9 -- which holds symbol 1.
    let table = fse::Table::build(&HAND_TABLE_COUNTS, 5).unwrap();
    assert_eq!(table.entries[0].symbol, 0);
    assert_eq!(table.entries[0].nb_bits, 1);
    assert_eq!(table.entries[0].new_state, 8);
    assert_eq!(table.entries[9].symbol, 1);
    assert_eq!(table.entries[9].nb_bits, 2);
    // 28, not 4: state 9 is symbol 1's *nineteenth* state number's slot, and the hand
    // table above records 28 for it. Reading 4 here would have been reading the answer for
    // some other state, which is what a copied-by-eye expectation usually is.
    assert_eq!(table.entries[9].new_state, 28);

    let symbols = [0u8, 1, 1, 0];
    let bits = reference_encode(&table, &symbols);
    assert_eq!(reference_decode(&table, &bits, 4), symbols.to_vec());
    let run = hardware_fse(&table, &symbols);
    assert_eq!(run.symbols, symbols.to_vec());
    assert_eq!(run.nb_bits[0], 1, "state 0's transition costs one bit");
    assert_eq!(run.final_state, 0, "and the terminal state is zero");
}

#[test]
fn the_decoder_retires_one_symbol_per_cycle_once_the_buffer_is_primed() {
    // The module's headline claim, measured. A message of the most probable symbol has a
    // short `nbBits`, so once the buffer holds enough the design emits on consecutive
    // cycles -- the property a CPU loop cannot offer and a SIMD lane cannot fill.
    let table = fse::Table::build(&HAND_TABLE_COUNTS, 5).unwrap();
    let symbols = vec![0u8; 40];
    let run = hardware_fse(&table, &symbols);
    assert_eq!(run.symbols, symbols);
    assert!(
        run.longest_run >= 4,
        "only {} consecutive symbols, which would not be a one-per-cycle decoder",
        run.longest_run
    );
}

#[test]
fn a_count_of_zero_pulses_done_without_decoding_anything() {
    // The edge case a host can produce and the design has to survive: nothing to decode,
    // so `done` fires and nothing else happens. The initial-state load still occurs,
    // because the design reads the state before it knows the count is zero.
    let table = fse::Table::build(&HAND_TABLE_COUNTS, 5).unwrap();
    let design = Design::new();
    fse::build(&design, &table).unwrap();
    let tb = Testbench::new(&design).unwrap();
    tb.run(|tb| async move {
        tb.pulse(ferrite_lithic_corpus::RESET).await;
        tb.set("count", 0).await;
        tb.set("init", 1).await;
        tb.step().await;
        tb.set("init", 0).await;
        for _ in 0..12 {
            tb.step().await;
        }
    })
    .expect("the fse design drives only ports it declares");
    let done = tb.series("done");
    let valid = tb.series("out_valid");
    assert!(
        done.iter().any(|bits| bits.to_u64().expect("one bit") == 1),
        "a zero count still terminates rather than hanging"
    );
    assert!(
        valid
            .iter()
            .all(|bits| bits.to_u64().expect("one bit") == 0),
        "and decodes nothing"
    );
}

proptest! {
    /// The differential, on arbitrary distributions over up to five symbols.
    ///
    /// Five symbols is the point: five symbols in a 32-state table means at least one of
    /// them has `nbBits` of 1 or 0, which is the regime where the one-symbol-per-cycle
    /// claim lives.
    #[test]
    fn the_round_trip_recovers_arbitrary_messages_over_arbitrary_distributions(
        weights in prop::collection::vec(1u32..500, 2..6),
        message in prop::collection::vec(any::<u8>(), 1..40),
    ) {
        let counts = normalise(&weights, 5);
        let table = fse::Table::build(&counts, 5).unwrap();
        let symbols: Vec<u8> = message
            .iter()
            .map(|index| index % counts.len() as u8)
            .collect();
        let bits = reference_encode(&table, &symbols);
        prop_assert_eq!(reference_decode(&table, &bits, symbols.len()), symbols.clone());
        let run = hardware_fse(&table, &symbols);
        prop_assert_eq!(run.symbols, symbols);
        prop_assert!(run.saw_done);
    }
}

// =======================================================================================
// DEFLATE: the bit layer, against miniz_oxide.
// =======================================================================================

/// Packs a bit sequence into bytes, least significant bit first.
///
/// RFC 1951 section 3.1.1: "data elements are packed into bytes in order of increasing
/// bit number within the byte". Bit `k` of the sequence is bit `k % 8` of byte `k / 8`.
fn pack(bits: &[bool]) -> Vec<u8> {
    let mut out = vec![0u8; bits.len().div_ceil(8)];
    for (index, bit) in bits.iter().enumerate() {
        if *bit {
            out[index / 8] |= 1u8 << (index % 8);
        }
    }
    out
}

/// A field reader over a bit sequence, reading multi-bit fields least significant bit
/// first -- the other half of RFC 1951 section 3.1.1.
struct BitReader<'a> {
    bits: &'a [bool],
    at: usize,
}

impl<'a> BitReader<'a> {
    fn new(bits: &'a [bool]) -> Self {
        Self { bits, at: 0 }
    }

    fn field(&mut self, width: usize) -> u64 {
        let mut value = 0u64;
        for position in 0..width {
            value |= u64::from(self.bits[self.at + position]) << position;
        }
        self.at += width;
        value
    }

    /// RFC 1951 section 3.2.4: "skip any remaining bits in the partially processed byte".
    fn align(&mut self) {
        self.at = self.at.div_ceil(8) * 8;
    }

    fn remaining(&self) -> usize {
        self.bits.len() - self.at
    }
}

/// A DEFLATE block header, read from a bit sequence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct BlockHeader {
    b_final: bool,
    b_type: u64,
}

fn read_block_header(bits: &[bool]) -> BlockHeader {
    let mut reader = BitReader::new(bits);
    let b_final = reader.field(1) == 1;
    let b_type = reader.field(2);
    BlockHeader { b_final, b_type }
}

/// A stored block's length and payload, read from a bit sequence starting at a block
/// header. `None` if the block is not a stored block or the stream is inconsistent.
fn read_stored_block(bits: &[bool]) -> Option<(usize, Vec<u8>)> {
    let mut reader = BitReader::new(bits);
    reader.field(1);
    if reader.field(2) != 0 {
        return None;
    }
    reader.align();
    let len = reader.field(16) as usize;
    let nlen = reader.field(16) as usize;
    // RFC 1951's complement is taken over the *sixteen-bit* field. `!nlen as u64` is a
    // sixty-four-bit complement, which no sixteen-bit `len` can ever equal -- so the
    // comparison would reject every stored block including the correct ones, and the
    // test that a broken complement is refused would have been passing for that reason
    // rather than because the complement was checked.
    if (len as u32) != (!nlen as u32 & 0xffff) {
        return None;
    }
    if reader.remaining() < 8 * len {
        return None;
    }
    let payload = bits[reader.at..reader.at + 8 * len]
        .chunks(8)
        .map(|byte| {
            byte.iter().enumerate().fold(0u8, |value, (offset, bit)| {
                value | (u8::from(*bit) << offset)
            })
        })
        .collect();
    Some((len, payload))
}

async fn drive_deflate(tb: &Handle, bytes: &[u8]) {
    tb.set("init", 1).await;
    tb.step().await;
    tb.set("init", 0).await;
    let mut index = 0usize;
    while index < bytes.len() {
        tb.set("in_byte", u64::from(bytes[index])).await;
        // The design takes a byte only when its staging register is empty, which is one
        // cycle in nine. Reading `in_ready` before the edge is what makes the handshake
        // correct rather than hopeful.
        let ready = tb.value("in_ready") == 1;
        tb.set("in_valid", u64::from(ready)).await;
        tb.step().await;
        if ready {
            index += 1;
        }
    }
    tb.set("in_valid", 0).await;
    // The loop exits on the cycle the *last* byte is loaded, so that byte's eight bits are
    // still in the staging register. Draining here rather than inside the loop is what
    // makes the count `8 * len` instead of `8 * (len - 1) + 1`, and the shortfall is the
    // kind of off-by-one that looks like a design bug and is not.
    for _ in 0..8 {
        tb.step().await;
    }
}

fn collect_bits(tb: &Testbench) -> (Vec<bool>, usize) {
    let valid = tb.series("out_valid");
    let bit = tb.series("out_bit");
    let last = tb.series("out_last");
    let mut bits = Vec::new();
    let mut boundaries = 0usize;
    for index in 0..valid.len() {
        if valid[index].to_u64().expect("one bit") == 1 {
            bits.push(bit[index].to_u64().expect("one bit") == 1);
            if last[index].to_u64().expect("one bit") == 1 {
                boundaries += 1;
            }
        }
    }
    (bits, boundaries)
}

/// Runs the compressed bytes through the bit layer and returns the bit sequence.
fn hardware_deflate(bytes: &[u8]) -> (Vec<bool>, usize) {
    let design = Design::new();
    deflate::build(&design).unwrap();
    let tb = Testbench::new(&design).unwrap();
    tb.run(|tb| async move {
        tb.pulse(ferrite_lithic_corpus::RESET).await;
        drive_deflate(&tb, bytes).await;
    })
    .expect("the deflate design drives only ports it declares");
    collect_bits(&tb)
}

/// Inputs covering the categories that matter for a bit layer: nothing at all, one byte,
/// highly compressible text, and incompressible bytes.
fn corpus() -> Vec<Vec<u8>> {
    let mut out: Vec<Vec<u8>> = vec![Vec::new(), vec![0x5a]];
    out.push(
        b"the quick brown fox jumps over the lazy dog, the quick brown fox jumps over \
          the lazy dog, the quick brown fox"
            .to_vec(),
    );
    out.push(b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_vec());
    // A deterministic pseudo-random sequence: incompressible by construction, which is
    // what makes a DEFLATE encoder fall back to stored blocks.
    let mut noise = Vec::new();
    let mut word = 0x2545_f491_4f6c_dd1du64;
    for _ in 0..600 {
        word ^= word << 13;
        word ^= word >> 7;
        word ^= word << 17;
        noise.push((word >> 24) as u8);
    }
    out.push(noise);
    out.push((0..=255u8).collect());
    out
}

#[test]
fn the_bit_layer_round_trips_real_deflate_streams() {
    // The strongest test available for a bit layer: take bytes `miniz_oxide` produced,
    // run them through the design one bit at a time, and check that the bit sequence it
    // emits is *exactly* the stream's bits, least significant bit of each byte first.
    // Repacking the bits must give the compressed bytes back byte for byte.
    //
    // And separately that those bytes really are a DEFLATE stream of the input, so the
    // bit layer is checked against real data from the format rather than against a
    // stream this file built.
    for input in corpus() {
        for level in [0u8, 6u8, 9u8] {
            let compressed = miniz_oxide::deflate::compress_to_vec(&input, level);
            let inflated = miniz_oxide::inflate::decompress_to_vec(&compressed)
                .expect("miniz round-trips its own output");
            assert_eq!(inflated, input, "level {level} of {} bytes", input.len());

            let (bits, boundaries) = hardware_deflate(&compressed);
            assert_eq!(
                bits.len(),
                8 * compressed.len(),
                "every bit of the stream comes out, for {} bytes at level {level}",
                input.len()
            );
            assert_eq!(
                boundaries,
                compressed.len(),
                "and `out_last` marks each byte boundary"
            );
            assert_eq!(
                pack(&bits),
                compressed,
                "the bit layer is bit-exact on {} bytes at level {level}",
                input.len()
            );
        }
    }
}

#[test]
fn the_block_header_the_design_reads_is_the_one_the_bytes_say() {
    // The bit layer is not just producing *a* bit sequence, it is producing the one the
    // format defines -- so check the first field every DEFLATE stream starts with against
    // a reference reader over the same bytes. A design that peeled bits off the top of a
    // byte would still emit eight bits per byte and would fail exactly here.
    for input in corpus() {
        for level in [0u8, 1u8, 6u8, 9u8] {
            let compressed = miniz_oxide::deflate::compress_to_vec(&input, level);
            let reference: Vec<bool> = (0..8 * compressed.len())
                .map(|index| compressed[index / 8] >> (index % 8) & 1 == 1)
                .collect();
            let (bits, _) = hardware_deflate(&compressed);
            assert_eq!(bits, reference, "bit-for-bit, level {level}");

            let want = read_block_header(&reference);
            let got = read_block_header(&bits);
            assert_eq!(got, want, "first block header for {} bytes", input.len());
            assert!(
                matches!(want.b_type, 0..=2),
                "three is reserved, so seeing it would mean the bit order is wrong: got {}",
                want.b_type
            );
        }
    }
}

#[test]
fn a_stored_block_round_trips_to_the_original_bytes() {
    // **The original bytes, back, out of the design.** A stored block's payload *is* the
    // uncompressed data, so a bit layer that peels bits off in the right order is enough
    // to recover it -- and that is the strongest round trip this module can have, because
    // it depends on no part of the design this module claims not to implement.
    //
    // Level 0 makes `miniz_oxide` emit stored blocks for every input, so this covers all
    // the categories rather than only the incompressible ones.
    for input in corpus() {
        let compressed = miniz_oxide::deflate::compress_to_vec(&input, 0);
        let (bits, _) = hardware_deflate(&compressed);
        let (len, payload) = read_stored_block(&bits)
            .unwrap_or_else(|| panic!("level 0 must emit a stored block, got {bits:?}"));
        assert_eq!(len, input.len(), "LEN says how long the payload is");
        assert_eq!(
            payload, input,
            "the payload the design's bits carry is the original data"
        );
    }
}

#[test]
fn a_stored_block_with_a_wrong_complement_is_rejected() {
    // RFC 1951 section 3.2.4 requires NLEN to be LEN's ones complement. The design does
    // not check it -- it has no notion of a block -- so the reference reader here does,
    // and a stream with a broken complement must be refused rather than decoded.
    let mut bits = Vec::new();
    push_bits(&mut bits, 1, 1); // BFINAL
    push_bits(&mut bits, 0, 2); // BTYPE = stored
    bits.extend(std::iter::repeat_n(false, 5)); // pad to the byte boundary
    push_bits(&mut bits, 3, 16); // LEN = 3
    push_bits(&mut bits, 3, 16); // NLEN = 3, which is not ~3
    push_bits(&mut bits, 0xaa, 8);
    push_bits(&mut bits, 0x55, 8);
    push_bits(&mut bits, 0x12, 8);
    assert_eq!(read_stored_block(&bits), None);
}

#[test]
fn align_discards_the_rest_of_a_byte() {
    // RFC 1951 section 3.2.4's "skip any remaining bits in the partially processed byte",
    // which is what a stored block after a Huffman block needs. Without it the LEN that
    // follows would be read from the wrong bit offset, which is a corruption no amount of
    // downstream checking would catch.
    let design = Design::new();
    deflate::build(&design).unwrap();
    let tb = Testbench::new(&design).unwrap();
    tb.run(|tb| async move {
        tb.pulse(ferrite_lithic_corpus::RESET).await;
        tb.set("init", 1).await;
        tb.step().await;
        tb.set("init", 0).await;
        // Three bits of a block header...
        tb.set("in_byte", 0b101).await;
        tb.set("in_valid", 1).await;
        tb.step().await;
        tb.set("in_valid", 0).await;
        for _ in 0..8 {
            tb.step().await;
        }
        // ...then ask for the rest of the byte to be dropped.
        tb.set("align", 1).await;
        for _ in 0..9 {
            tb.step().await;
        }
    })
    .expect("the deflate design drives only ports it declares");
    let left = tb.series("bits_left");
    assert_eq!(
        left.last().map(|bits| bits.to_u64().unwrap_or(0)),
        Some(0),
        "align empties the staging register"
    );
    let ready = tb.series("in_ready");
    assert_eq!(
        ready.last().map(|bits| bits.to_u64().unwrap_or(0)),
        Some(1),
        "and the next byte is taken at a boundary"
    );
}

#[test]
fn the_bit_rate_is_one_byte_in_nine_cycles() {
    // The throughput claim, measured rather than asserted, because it is the cost the
    // module docs quote against the alternatives. The design loads a byte only when its
    // staging register is empty, so a byte costs nine edges: one to take it and eight to
    // shift it out.
    let design = Design::new();
    deflate::build(&design).unwrap();
    let tb = Testbench::new(&design).unwrap();
    let cycles = tb
        .run(|tb| async move {
            tb.pulse(ferrite_lithic_corpus::RESET).await;
            let bytes = [0x5au8; 10];
            let mut index = 0usize;
            while index < bytes.len() {
                tb.set("in_byte", u64::from(bytes[index])).await;
                let ready = tb.value("in_ready") == 1;
                tb.set("in_valid", u64::from(ready)).await;
                tb.step().await;
                if ready {
                    index += 1;
                }
            }
            tb.set("in_valid", 0).await;
            // The loop leaves the tenth byte loaded and unshifted, so it costs its eight
            // shift cycles here rather than inside the loop. Without the drain the design
            // measures eighty-five cycles rather than ninety, which is one byte cheaper
            // than the design can actually be.
            for _ in 0..8 {
                tb.step().await;
            }
            tb.step().await;
            tb.cycle()
        })
        .expect("the deflate design drives only ports it declares");
    // Reset costs two edges and the ten bytes cost ninety, so the design is exactly as
    // slow as its documentation claims.
    assert_eq!(cycles, 2 + 10 * 9 + 1);
}

proptest! {
    /// The bit layer, on arbitrary byte sequences.
    ///
    /// Arbitrary bytes rather than arbitrary compressed streams, because the property is
    /// about the *bit order* and it has to hold for every byte pattern -- including the
    /// ones that carry no DEFLATE structure at all. The round trip through
    /// `miniz_oxide` above covers the "real stream" side.
    #[test]
    fn the_bit_layer_round_trips_arbitrary_bytes(
        bytes in prop::collection::vec(any::<u8>(), 0..24),
    ) {
        let (bits, boundaries) = hardware_deflate(&bytes);
        prop_assert_eq!(pack(&bits), bytes.clone());
        prop_assert_eq!(bits.len(), 8 * bytes.len());
        prop_assert_eq!(boundaries, bytes.len());
    }

    /// And on arbitrary *bit* sequences, through the field reader, so the
    /// least-significant-bit-first field convention is checked too.
    #[test]
    fn a_field_is_read_least_significant_bit_first(
        // Sixteen bits of input at minimum, because the width below goes up to sixteen
        // and the fold indexes `bits[position]`: a shorter vector would fail on an
        // out-of-bounds index, which says nothing about bit order.
        bits in prop::collection::vec(any::<bool>(), 16..64),
        width in 1usize..=16,
    ) {
        let mut reader = BitReader::new(&bits);
        let want = (0..width)
            .fold(0u64, |value, position| value | (u64::from(bits[position]) << position));
        prop_assert_eq!(reader.field(width), want);
    }
}

// =======================================================================================
// Verilator.
// =======================================================================================

/// The cosimulation plan for a built design.
///
/// [`Plan::of`] is what knows how to exclude the clock from the stimulus columns, so the
/// plan is always derived rather than written out.
fn plan_of(design: &Design, name: &str) -> Plan {
    let clock = design
        .input_ports()
        .iter()
        .find(|signal| signal.name().as_deref() == Some(ferrite_lithic_corpus::CLOCK))
        .expect("the clock is a declared input")
        .id();
    let module = ferrite_lithic_rtl::Module::new(
        name,
        design.build().unwrap(),
        clock,
        design.input_ports().iter().map(|s| s.id()).collect(),
        design.output_ports().iter().map(|s| s.id()).collect(),
    );
    Plan::of(design, &module).unwrap()
}

#[test]
fn the_simulator_and_the_emitted_verilog_agree_on_the_fse_design() {
    let Some(verilator) = verilator() else {
        println!(
            "SKIPPED: verilator not found, so the fse equivalence did not run. The round \
             trip above does not need it."
        );
        return;
    };
    println!("cosimulating fse with {}", verilator.display());

    let counts = vec![16, 12, 3, -1i32];
    let table = fse::Table::build(&counts, 5).unwrap();
    // Only the three symbols with a positive count. The `-1` symbol holds a single state
    // that consumes no bits and returns to state zero, so no transition reaches it except
    // from state zero itself: a message containing it cannot be encoded against this
    // table at all, and asking for one fails in the encoder rather than in the design.
    let symbols: Vec<u8> = (0..24u32).map(|index| (index % 3) as u8).collect();
    let bits = reference_encode(&table, &symbols);

    let design = Design::new();
    fse::build(&design, &table).unwrap();
    let plan = plan_of(&design, "fse");

    assert_eq!(
        plan.inputs
            .iter()
            .map(|p| p.name.as_str())
            .collect::<Vec<_>>(),
        ["rst", "init", "count", "in_valid", "in_bit"],
        "the clock is a port but not a stimulus column"
    );
    assert!(
        plan.outputs.iter().all(|port| port.width <= 64),
        "no port is wider than the generated driver's limit"
    );

    // One bit every two cycles, for the same reason the Huffman stimulus does it: the
    // design's `in_ready` drops when the buffer is full, and a one-bit-per-cycle
    // stimulus would lose a bit.
    let mut stimulus = Stimulus::new();
    let row = |rst: u64, init: u64, count: u64, valid: u64, bit: u64| {
        vec![
            Bits::constant(rst, 1).unwrap(),
            Bits::constant(init, 1).unwrap(),
            Bits::constant(count, 16).unwrap(),
            Bits::constant(valid, 1).unwrap(),
            Bits::constant(bit, 1).unwrap(),
        ]
    };
    stimulus.push(row(1, 0, 0, 0, 0)).unwrap();
    stimulus.push(row(0, 0, 0, 0, 0)).unwrap();
    stimulus
        .push(row(0, 1, symbols.len() as u64, 0, 0))
        .unwrap();
    for bit in bits {
        stimulus.push(row(0, 0, 0, 1, u64::from(bit))).unwrap();
        stimulus.push(row(0, 0, 0, 0, 0)).unwrap();
    }
    for _ in 0..12 {
        stimulus.push(row(0, 0, 0, 0, 0)).unwrap();
    }

    let verilog = ferrite_lithic_cosim::emit(&design, &plan).unwrap();
    let harness = Harness::build(&plan, &verilog, &plan.work_dir()).unwrap();
    let report = ferrite_lithic_cosim::run_with(&design, &plan, &stimulus, &harness).unwrap();
    assert!(
        report.is_equivalent(),
        "the two backends disagree: {report}\nfirst: {:?}",
        report.first()
    );

    // And all three meet.
    let run = hardware_fse(&table, &symbols);
    assert_eq!(run.symbols, symbols);
}

#[test]
fn the_simulator_and_the_emitted_verilog_agree_on_the_deflate_design() {
    let Some(verilator) = verilator() else {
        println!(
            "SKIPPED: verilator not found, so the deflate equivalence did not run. The \
             round trip against `miniz_oxide` above does not need it."
        );
        return;
    };
    println!("cosimulating deflate with {}", verilator.display());

    // A real DEFLATE stream, so the compared window is a real one.
    let input: Vec<u8> = b"the quick brown fox jumps over the lazy dog".to_vec();
    let compressed = miniz_oxide::deflate::compress_to_vec(&input, 6);

    let design = Design::new();
    deflate::build(&design).unwrap();
    let plan = plan_of(&design, "deflate");

    assert_eq!(
        plan.inputs
            .iter()
            .map(|p| p.name.as_str())
            .collect::<Vec<_>>(),
        ["rst", "init", "in_valid", "in_byte", "align"],
        "the clock is a port but not a stimulus column"
    );
    assert!(
        plan.outputs.iter().all(|port| port.width <= 64),
        "no port is wider than the generated driver's limit"
    );

    let mut stimulus = Stimulus::new();
    let row = |rst: u64, init: u64, valid: u64, byte: u64, align: u64| {
        vec![
            Bits::constant(rst, 1).unwrap(),
            Bits::constant(init, 1).unwrap(),
            Bits::constant(valid, 1).unwrap(),
            Bits::constant(byte, 8).unwrap(),
            Bits::constant(align, 1).unwrap(),
        ]
    };
    stimulus.push(row(1, 0, 0, 0, 0)).unwrap();
    stimulus.push(row(0, 0, 0, 0, 0)).unwrap();
    for byte in &compressed {
        stimulus.push(row(0, 0, 1, u64::from(*byte), 0)).unwrap();
        // Eight cycles of shifting the byte out, then one more for the design to take the
        // next one: nine cycles per byte is the rate the docs claim.
        for _ in 0..8 {
            stimulus.push(row(0, 0, 0, 0, 0)).unwrap();
        }
    }
    for _ in 0..9 {
        stimulus.push(row(0, 0, 0, 0, 0)).unwrap();
    }

    let verilog = ferrite_lithic_cosim::emit(&design, &plan).unwrap();
    let harness = Harness::build(&plan, &verilog, &plan.work_dir()).unwrap();
    let report = ferrite_lithic_cosim::run_with(&design, &plan, &stimulus, &harness).unwrap();
    assert!(
        report.is_equivalent(),
        "the two backends disagree: {report}\nfirst: {:?}",
        report.first()
    );

    let (bits, boundaries) = hardware_deflate(&compressed);
    assert_eq!(pack(&bits), compressed);
    assert_eq!(boundaries, compressed.len());
}
