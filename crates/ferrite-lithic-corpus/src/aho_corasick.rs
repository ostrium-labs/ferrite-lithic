//! Multi-pattern search: nineteen states, eleven symbols, one byte per cycle.
//!
//! | quantity | value |
//! |---|---|
//! | bytes per clock edge | 1 |
//! | state register | [`STATE_BITS`] = 5 bits |
//! | symbols | [`SYMBOLS`] = 11 |
//! | states | [`STATES`] = 19 |
//! | ROM word | [`WORD_BITS`] = 6 bits: one `accept` bit and the five state bits |
//! | ROM contents | 19 x 11 x 6 bits = 1254 bits, 157 bytes |
//! | alphabet map | one [`Design::case_`] with 11 arms |
//!
//! # The shape, and why this one is the interesting one
//!
//! [`memchr`](crate::memchr)(crate::memchr) is one byte per cycle against an eight-bit comparator and
//! the CPU beats it by a factor of 32. This entry is the case where that argument stops
//! applying, and the reason is worth being precise about:
//!
//! - **The state is five flops and the table is 157 bytes.** N lanes cost N five-flop
//!   registers and share one ROM. Replication is nearly free, and the shared ROM is why
//!   it stays nearly free.
//! - **There is no SIMD form.** "Compare 32 bytes against 19 states at once" has no
//!   meaning: the state after byte `i` is a function of the state after byte `i - 1`, so
//!   the automaton is serial by construction. A CPU cannot widen this loop without
//!   changing the algorithm. The crate that does this on a CPU -- `aho-corasick` itself,
//!   and `regex-automata`'s dense DFA -- also runs one byte at a time; what it wins on
//!   is prefilters (memchr for the first byte of a rare literal), not on lanes.
//! - **The failure links are already compiled away.** Every state in [`TRANSITIONS`] is
//!   reachable and every transition out of it is already resolved, so there is no
//!   backtracking stack, no NFA simulation and no restart: one ROM read per byte.
//!
//! So: one byte per cycle, five flops of state, and a table small enough that N
//! replicated lanes share it. That is the shape a data path wants and the shape SIMD
//! cannot provide.
//!
//! # Where the table comes from
//!
//! [`TRANSITIONS`] is `aho_corasick::dfa::DFA` built from [`PATTERNS`] with
//! `MatchKind::Standard`, walked breadth-first from its unanchored start state over
//! all 256 bytes, renumbered densely, with the `accept` bit folded into the ROM word and
//! the winning pattern id in [`MATCH_PIDS`].
//!
//! **"Compile offline" is literal here.** The table is a checked-in constant. There is
//! no automaton construction in the datapath, none in the cycle budget, and no
//! dependence on the `aho-corasick` crate at run time -- it is a *dev*-dependency of
//! this crate, so it could not be a run-time dependency even if the design wanted one.
//! The tests walk the crate's DFA and assert the constant is exactly what they find, so
//! the constant is checked rather than trusted.
//!
//! # The alphabet, and the claim the catch-all makes
//!
//! The eleven symbols are the ten distinct bytes that appear in [`PATTERNS`] --
//! `a c e f i n o p r w` -- plus [`OTHER`] for the other 246 bytes.
//!
//! That catch-all is a strong claim: **every one of those 246 bytes must take the same
//! transition from every one of the nineteen states.** It happens to hold here because
//! none of these patterns can distinguish one non-alphabet byte from another, and it
//! would not hold for a pattern set containing a byte-class split -- `aho-corasick`
//! computes byte classes too, and this design's declared alphabet deliberately ignores
//! them in favour of a byte set a reader can name. The test checks all 256 bytes against
//! the crate rather than trusting the argument.
//!
//! The map itself is one [`Design::case_`] with 11 arms. Unlike [`dfa`](crate::dfa)(crate::dfa)'s
//! alphabet, whose 63 bytes are four contiguous ASCII ranges, these ten bytes are
//! scattered, so there is no range structure to exploit and a table is the honest
//! implementation.
//!
//! # Why a `case_`, and what it costs
//!
//! **This lowers to a multiplexer tree, not to a block RAM.** There is no
//! initialised-memory node in the IR -- `Design::mem` is zero-filled in both backends,
//! so there is no way to state a memory's contents -- and a `case_` is the current
//! idiom. A real implementation is a 1254-bit ROM with a 5 + 4 = 9-bit address and a
//! 6-bit word: one read port, fixed delay, and no depth. As a mux tree it is an 11-to-1
//! mux per row per output bit, cascaded over 19 rows.
//!
//! The cycle count is identical either way -- one byte per cycle in both -- so what the
//! ROM would save is **area and critical path**, not throughput. That is the honest
//! version of the claim, and it is a known gap in the IR rather than a property of this
//! design.
//!
//! # The stop rule, and why this entry *does* match `find_iter`
//!
//! Unlike [`dfa`](crate::dfa)(crate::dfa), this design reproduces
//! `AhoCorasick::find_iter` exactly, and it does so because of one extra gate:
//!
//! ```text
//! word   <- ROM[state, symbol(byte)]
//! accept  = word.accept
//! state'  = accept ? START : word.next_state
//! ```
//!
//! On reaching a match state the automaton **returns to the start state** instead of
//! following the table. That is the automaton's own search rule -- a leftmost search
//! stops when it matches and the next search begins at the end of that match -- lifted
//! out of the loop and into a multiplexer. It costs one `ite` and it is the entire
//! difference between a byte-serial design and a byte-serial design that agrees with the
//! crate.
//!
//! Two consequences worth stating:
//!
//! - The `accept` output and the `pattern` output are **registered**, because the state
//!   that produced them is overwritten on the very same edge. Reading a combinational
//!   `pattern` off `state` after that edge would report the *next* match's pattern.
//! - Five of the nineteen rows are never latched, because all five are match states and
//!   the stop rule jumps over them. They are kept anyway, so that [`TRANSITIONS`] is the
//!   crate's table and not a table derived from it.
//!
//! Reported per match: the byte position of the match's **end** (from `count`) and the
//! index of the pattern that matched (from `pattern`). Match *starts* are not reported,
//! and cannot be: recovering the start of a match needs the pattern's length and the
//! number of matches so far, and the leftmost rule is what makes it well defined. The
//! tests compare the `(end, pattern id)` sequence against `find_iter`.
//!
//! # The patterns, and why `in` is in the list
//!
//! [`PATTERNS`] is a log scanner's keyword set, and it contains a deliberate
//! awkwardness: **`in` is a prefix of `info`**. So the automaton has a state in which two
//! patterns match, one of them ending there, and the leftmost rule says report `in`.
//! That is what makes this a multi-pattern automaton rather than a loop of single-byte
//! searches, and `in in in`, `inf` and `info` are all in the tests for exactly that
//! reason.

use ferrite_lithic::{Design, Signal};
use ferrite_lithic_derive::PortList;

use crate::BuildError;

/// The patterns this automaton searches for, in pattern-index order.
///
/// Log-scanner keywords, with a prefix pair on purpose -- see the module docs.
pub const PATTERNS: [&[u8]; 5] = [b"panic", b"error", b"warn", b"info", b"in"];

/// The distinct bytes that appear in [`PATTERNS`], sorted.
///
/// Alphabet order, so symbol indices are stable and a reader can check the map against
/// the patterns by eye.
pub const ALPHABET: [u8; 10] = *b"acefinoprw";

/// The symbol for any byte that is not in [`ALPHABET`].
pub const OTHER: u64 = 10;

/// The width of a symbol.
pub const SYMBOL_BITS: u32 = 4;

/// How many symbols the alphabet has, as a count.
///
/// Eleven: [`ALPHABET`]'s ten bytes plus [`OTHER`]. Not a power of two, which is fine
/// -- a `case_` enumerates its arms rather than addressing a power-of-two range, and
/// there is no ROM here to size.
pub const SYMBOLS: usize = ALPHABET.len() + 1;

/// The width of the state register.
pub const STATE_BITS: u32 = 5;

/// How many states the automaton has, as a count.
///
/// Nineteen, the number the crate's DFA walk reaches. Also not a power of two; the two
/// spare encodings of five bits are unreachable and the tests say so.
pub const STATES: usize = 19;

/// The state the automaton starts in, and therefore the state a reset returns it to.
///
/// Zero, which is also where registers start, so `rst` and "the start state" are the
/// same thing and the design needs no separate start input.
pub const START: u64 = 0;

/// The width of one ROM word: the `accept` bit above the state bits.
pub const WORD_BITS: u32 = STATE_BITS + 1;

/// The width of the pattern index.
pub const PATTERN_BITS: u32 = 3;

/// The DFA transition table, offline-compiled from [`PATTERNS`].
///
/// `TRANSITIONS[state][symbol]` is `{accept, next_state}` with the `accept` bit at
/// position [`WORD_BITS`] - 1 and the next state's [`STATE_BITS`] bits below it.
///
/// States are numbered by a breadth-first walk from the crate's unanchored start state
/// over all 256 bytes, so index 0 is the start state, indices 1 to 13 are partial
/// prefixes, index 14 is `"info"`, 15 is the `"er"`-of-`"err"`-of-`"error"` state, 16 is
/// `"warn"`, 17 is `"error"`, 18 is `"panic"`, and states 6, 14, 16, 17 and 18 are match
/// states -- none of which the stop rule latches.
pub const TRANSITIONS: [[u64; SYMBOLS]; STATES] = [
    [0, 0, 1, 0, 2, 0, 0, 3, 0, 4, 0],
    [0, 0, 1, 0, 2, 0, 0, 3, 5, 4, 0],
    [0, 0, 1, 0, 2, 38, 0, 3, 0, 4, 0],
    [7, 0, 1, 0, 2, 0, 0, 3, 0, 4, 0],
    [8, 0, 1, 0, 2, 0, 0, 3, 0, 4, 0],
    [0, 0, 1, 0, 2, 0, 0, 3, 9, 4, 0],
    [0, 0, 1, 10, 2, 0, 0, 3, 0, 4, 0],
    [0, 0, 1, 0, 2, 11, 0, 3, 0, 4, 0],
    [0, 0, 1, 0, 2, 0, 0, 3, 12, 4, 0],
    [0, 0, 1, 0, 2, 0, 13, 3, 0, 4, 0],
    [0, 0, 1, 0, 2, 0, 46, 3, 0, 4, 0],
    [0, 0, 1, 0, 15, 0, 0, 3, 0, 4, 0],
    [0, 0, 1, 0, 2, 48, 0, 3, 0, 4, 0],
    [0, 0, 1, 0, 2, 0, 0, 3, 49, 4, 0],
    [0, 0, 1, 0, 2, 0, 0, 3, 0, 4, 0],
    [0, 50, 1, 0, 2, 38, 0, 3, 0, 4, 0],
    [0, 0, 1, 0, 2, 0, 0, 3, 0, 4, 0],
    [0, 0, 1, 0, 2, 0, 0, 3, 0, 4, 0],
    [0, 0, 1, 0, 2, 0, 0, 3, 0, 4, 0],
];

/// The pattern index each match state reports, and zero for every other state.
///
/// The **first** pattern in the state's match list, which under `MatchKind::Standard`
/// is the one the crate's own search would report: for `MatchKind::Standard` the
/// earliest-ending match wins, and among those the lowest pattern index. `in` is index 4
/// and `info` is index 3, and the automaton reports `in` for `"in..."` and `info` only
/// when the bytes go past it.
///
/// Zero is a legal pattern index, so the entries for non-match states are not
/// distinguishable from "reports `panic`" by themselves. They are only ever read while
/// `accept` is high, which is why that is what `accept` is for.
///
/// `MatchKind::Standard`: aho_corasick::MatchKind
pub const MATCH_PIDS: [u64; STATES] = [0, 0, 0, 0, 0, 0, 4, 0, 0, 0, 0, 0, 0, 0, 3, 0, 2, 1, 0];

/// The symbol `byte` belongs to: the software model of [`symbol`].
///
/// The specification the circuit implements, kept as a function so the test can compare
/// the two over all 256 byte values. `#[must_use]` because a caller that discards the
/// answer has a bug.
#[must_use]
pub fn symbol_of(byte: u8) -> u64 {
    ALPHABET
        .iter()
        .position(|candidate| *candidate == byte)
        .map_or(OTHER, |index| index as u64)
}

/// The input ports.
#[derive(Clone, Debug, PortList)]
pub struct Inputs {
    /// The clock. Every edge consumes one byte.
    #[clock]
    pub clk: Signal,
    /// Synchronous reset, which returns the automaton to [`START`] and zeroes `count`.
    pub rst: Signal,
    /// The next byte of the haystack, consumed on each rising edge.
    #[bits(8)]
    pub data: Signal,
}

/// The output ports.
#[derive(Clone, Debug, PortList)]
pub struct Outputs {
    /// Where the automaton will be after the next byte, given the stop rule. Zero on
    /// the cycle a match is reported, because that is where the automaton goes.
    #[bits(5)]
    pub state: Signal,
    /// A match ended on the byte consumed by the previous edge. Together with `count`
    /// that is the match's end position.
    pub accept: Signal,
    /// The index of the pattern that matched. Meaningful only while `accept` is high,
    /// and it is registered rather than combinational for the reason in the module docs.
    #[bits(3)]
    pub pattern: Signal,
    /// How many bytes have been consumed since the last reset, so `count - 1` is the
    /// offset of the byte just consumed. Wraps at 256.
    #[bits(8)]
    pub count: Signal,
}

/// The signals [`build`] returns, for a caller that wants them by handle.
#[derive(Clone, Debug)]
pub struct Ports {
    /// The input ports.
    pub inputs: Inputs,
    /// The state register, which is also the output.
    pub state: Signal,
    /// The registered accept bit, which is also the output.
    pub accept: Signal,
    /// The registered pattern index, which is also the output.
    pub pattern: Signal,
    /// The byte counter, which is also the output.
    pub count: Signal,
}

/// Builds the multi-pattern searcher into `design`.
///
/// # Errors
///
/// Whatever [`Design`] returns -- a width mismatch or an undriven wire -- and whatever
/// the port-list helpers return. There is no searcher-specific failure: [`PATTERNS`] is
/// a constant and [`TRANSITIONS`] is checked in, so a different pattern set would be a
/// different module rather than a misconfigured one.
pub fn build(design: &Design) -> Result<Ports, BuildError> {
    let inputs = ferrite_lithic::inputs::<Inputs>(design)?;

    // Both state and count are read by the logic that computes their own next values,
    // and a register cannot be an input to itself.
    let state = design.wire(STATE_BITS)?;
    let count = design.wire(8)?;

    let next = advance(design, &state, &inputs.data)?;

    let state_q = design.reg(
        &next.state,
        &inputs.clk,
        &inputs.rst,
        &design.constant(false),
    )?;
    design.drive(&state, &state_q)?;

    let accept_q = design.reg(
        &next.accept,
        &inputs.clk,
        &inputs.rst,
        &design.constant(false),
    )?;

    let pattern_q = design.reg(
        &next.pattern,
        &inputs.clk,
        &inputs.rst,
        &design.constant(false),
    )?;

    let stepped = design.add(&count, &design.lit(1, 8)?)?;
    let count_q = design.reg(&stepped, &inputs.clk, &inputs.rst, &design.constant(false))?;
    design.drive(&count, &count_q)?;

    ferrite_lithic::outputs::<Outputs>(
        design,
        &Outputs {
            state: state_q.clone(),
            accept: accept_q.clone(),
            pattern: pattern_q.clone(),
            count: count_q.clone(),
        },
    )?;

    Ok(Ports {
        inputs,
        state: state_q,
        accept: accept_q,
        pattern: pattern_q,
        count: count_q,
    })
}

/// The four registers' next values after consuming one byte.
///
/// The only non-table content is the stop rule: `state' = accept ? START : next_state`.
/// See the module docs for why that one gate is what makes this design agree with the
/// crate's `find_iter` where [`dfa`](crate::dfa)(crate::dfa) does not.
///
/// # Errors
///
/// Whatever [`Design`] returns.
#[derive(Clone, Debug)]
pub struct Next {
    /// The next state, after the stop rule.
    pub state: Signal,
    /// The `accept` bit for the byte just consumed.
    pub accept: Signal,
    /// The pattern index the match state reports.
    pub pattern: Signal,
}

/// One step of the automaton.
///
/// # Errors
///
/// Whatever [`Design`] returns.
pub fn advance(design: &Design, state: &Signal, byte: &Signal) -> Result<Next, BuildError> {
    let symbol = symbol(design, byte)?;
    let word = transition_table(design, &symbol, state)?;
    let accept = design.slice(&word, STATE_BITS, 1)?;
    let reached = design.slice(&word, 0, STATE_BITS)?;

    // The stop rule. One multiplexer, and the difference between a byte-serial DFA that
    // agrees with the crate and one that does not.
    let next_state = design.ite(&accept, &design.lit(START, STATE_BITS)?, &reached)?;

    Ok(Next {
        state: next_state,
        accept,
        pattern: match_pattern(design, &reached)?,
    })
}

/// The symbol index of `byte`: the alphabet map.
///
/// One [`Design::case_`] with [`SYMBOLS`] arms: [`ALPHABET`]'s ten bytes in order, and
/// [`OTHER`] as the default. Unlike [`dfa`](crate::dfa)(crate::dfa)'s map there is no range
/// structure to exploit -- these ten bytes are scattered across ASCII -- so a table is
/// the honest implementation and the eleven arms are the table.
///
/// # Errors
///
/// Whatever [`Design`] returns.
pub fn symbol(design: &Design, byte: &Signal) -> Result<Signal, BuildError> {
    let mut arms = Vec::with_capacity(ALPHABET.len());
    for (index, candidate) in ALPHABET.iter().enumerate() {
        // The arm key is the byte, its value is the index; the index fits in
        // SYMBOL_BITS because ALPHABET has ten entries and SYMBOL_BITS is four.
        arms.push((
            u64::from(*candidate),
            design.lit(index as u64, SYMBOL_BITS)?,
        ));
    }
    // `OTHER` is the default rather than an enumerated arm: it is the value of every
    // byte the other ten do not match.
    let default = design.lit(OTHER, SYMBOL_BITS)?;
    Ok(design.case_(byte, &arms, &default)?)
}

/// The ROM: one [`WORD_BITS`]-bit word for `(state, symbol)`.
///
/// Two nested [`Design::case_`]s, one per symbol inside each of the [`STATES`] rows and
/// then one per row. A real implementation is a single read port with a
/// `STATE_BITS + SYMBOL_BITS` address; see the module docs for what that would save.
///
/// The default arm of each `case_` is symbol zero's entry rather than a sentinel, for
/// the reason [`crate::hex::nibble_to_ascii`] gives: every symbol is enumerated, so no
/// default is reachable, and the right choice for an unreachable arm is the one that is
/// self-consistent if it ever were reached.
///
/// # Errors
///
/// Whatever [`Design`] returns.
pub fn transition_table(
    design: &Design,
    symbol: &Signal,
    state: &Signal,
) -> Result<Signal, BuildError> {
    let mut rows = Vec::with_capacity(STATES);
    for row in TRANSITIONS.iter() {
        let mut arms = Vec::with_capacity(SYMBOLS);
        for (index, word) in row.iter().enumerate() {
            arms.push((index as u64, design.lit(*word, WORD_BITS)?));
        }
        let default = arms[0].1.clone();
        rows.push(design.case_(symbol, &arms, &default)?);
    }
    let arms: Vec<(u64, Signal)> = rows
        .into_iter()
        .enumerate()
        .map(|(index, row)| (index as u64, row))
        .collect();
    let default = arms[0].1.clone();
    Ok(design.case_(state, &arms, &default)?)
}

/// The pattern index `state` reports when it is a match state.
///
/// The per-state match table: one arm per match state, everything else zero. A real
/// engine keeps this as a second small ROM indexed by state rather than as logic, and
/// this is that ROM with the zero rows dropped -- there are only five of them and
/// [`MATCH_PIDS`] is the whole table.
///
/// # Errors
///
/// Whatever [`Design`] returns.
pub fn match_pattern(design: &Design, state: &Signal) -> Result<Signal, BuildError> {
    let mut arms = Vec::new();
    for (index, pid) in MATCH_PIDS.iter().enumerate() {
        if *pid == 0 && !is_match_state(index) {
            continue;
        }
        arms.push((index as u64, design.lit(*pid, PATTERN_BITS)?));
    }
    // Zero is `panic`, so the default is not distinguishable from a real answer; it is
    // only ever read while `accept` is high.
    let default = design.lit(0, PATTERN_BITS)?;
    Ok(design.case_(state, &arms, &default)?)
}

/// Whether this design's state `index` is a match state.
///
/// Read off [`TRANSITIONS`] rather than written down, so it cannot disagree with the
/// table: a state is a match state exactly when some word in the table *arrives* at it
/// with the `accept` bit set. It is deliberately not "some word in that state's row has
/// the accept bit set" -- that is a different predicate, it asks whether the state has
/// an edge into a match, and it is true of most of the automaton.
///
/// `#[must_use]` because a caller that discards the answer has a bug.
#[must_use]
pub fn is_match_state(index: usize) -> bool {
    let mask = (1u64 << STATE_BITS) - 1;
    TRANSITIONS
        .iter()
        .flatten()
        .any(|word| *word & (1u64 << STATE_BITS) != 0 && (*word & mask) as usize == index)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::{
        ALPHABET, MATCH_PIDS, OTHER, PATTERNS, STATE_BITS, STATES, SYMBOL_BITS, SYMBOLS,
        TRANSITIONS, WORD_BITS, is_match_state, symbol_of,
    };

    #[test]
    fn the_alphabet_is_exactly_the_bytes_the_patterns_use() {
        let mut from_patterns: Vec<u8> = PATTERNS
            .iter()
            .flat_map(|pattern| pattern.iter().copied())
            .collect();
        from_patterns.sort_unstable();
        from_patterns.dedup();
        assert_eq!(from_patterns, ALPHABET.to_vec());
        assert_eq!(ALPHABET.len(), 10);
        assert_eq!(OTHER, 10);
        assert_eq!(SYMBOLS, 11);
        assert_eq!(SYMBOL_BITS, 4);
        assert_eq!(STATES, 19);
        assert_eq!(STATE_BITS, 5);
        assert_eq!(WORD_BITS, 6);
        for (index, byte) in ALPHABET.iter().enumerate() {
            assert_eq!(symbol_of(*byte), index as u64, "{byte:#04x}");
        }
        for byte in 0..=255u16 {
            let byte = u8::try_from(byte).expect("the loop bounds a u8");
            assert!(symbol_of(byte) < OTHER + 1, "{byte:#04x} has a symbol");
        }
        assert_eq!(symbol_of(b'z'), OTHER, "a byte not in the patterns");
        assert_eq!(symbol_of(0), OTHER);
    }

    #[test]
    fn the_pattern_set_really_does_contain_a_prefix_pair() {
        // The reason `in` is in PATTERNS. If it were removed this automaton would be a
        // loop of independent literal searches and the interesting test cases would go
        // away with it.
        assert!(PATTERNS.iter().any(|p| *p == b"in".as_slice()));
        assert!(PATTERNS.iter().any(|p| *p == b"info".as_slice()));
        assert_eq!(PATTERNS.iter().filter(|p| **p == b"in").count(), 1);
        assert_eq!(
            PATTERNS.len(),
            5,
            "five patterns, so three bits of pattern index"
        );
        let mut sorted: Vec<&[u8]> = PATTERNS.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), PATTERNS.len(), "no duplicate patterns");
    }

    #[test]
    fn the_match_states_are_the_ones_the_table_says() {
        let states: Vec<usize> = (0..STATES).filter(|i| is_match_state(*i)).collect();
        assert_eq!(states, vec![6, 14, 16, 17, 18]);
        // And the pattern each one reports is a real pattern index.
        for state in &states {
            assert!(
                MATCH_PIDS[*state] < super::PATTERNS.len() as u64,
                "state {state}"
            );
        }
        assert_eq!(MATCH_PIDS[6], 4, "\"in\"");
        assert_eq!(MATCH_PIDS[14], 3, "\"info\"");
        assert_eq!(MATCH_PIDS[16], 2, "\"warn\"");
        assert_eq!(MATCH_PIDS[17], 1, "\"error\"");
        assert_eq!(MATCH_PIDS[18], 0, "\"panic\"");
    }

    #[test]
    fn the_start_state_resets_on_every_non_alphabet_byte() {
        // Aho-Corasick has no dead state -- every byte either extends a prefix or
        // resets to the start -- which is the structural property that makes the
        // catch-all symbol safe. Asserting it here means the test file does not have to
        // re-derive it from the crate before it can trust the constant.
        assert_eq!(
            TRANSITIONS[0][OTHER as usize], 0,
            "a non-alphabet byte resets"
        );
        for (state, row) in TRANSITIONS.iter().enumerate() {
            let word = row[OTHER as usize];
            assert!(
                word & ((1 << STATE_BITS) - 1) < STATES as u64,
                "row {state} names a state"
            );
            assert!(word < (1 << WORD_BITS), "row {state} fits the ROM word");
        }
    }
}
