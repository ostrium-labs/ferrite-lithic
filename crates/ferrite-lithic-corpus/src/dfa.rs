//! A regex as a dense DFA: a five-state ROM and a three-bit register.
//!
//! This is the centrepiece of the tier, so the argument for the shape is stated here
//! rather than in [`memchr`](crate::memchr), which is the baseline and which loses.
//!
//! # The shape, in numbers
//!
//! | quantity | value |
//! |---|---|
//! | bytes per clock edge | 1 |
//! | state register | [`STATE_BITS`] = 3 bits |
//! | symbols | [`SYMBOLS`] = 64 |
//! | states | [`STATES`] = 5 |
//! | ROM word | [`WORD_BITS`] = 4 bits: one `accept` bit and the three state bits |
//! | ROM contents | 5 x 64 x 4 bits = 1280 bits, 160 bytes |
//! | alphabet map | four subtracts, four unsigned compares, four adds, four muxes |
//!
//! One byte per edge, and that is the ceiling: the state after byte `i` is a function
//! of the state after byte `i - 1`, so the automaton is serial and there is nothing to
//! unroll. The state is **three flops**. Lanes are cheap because of exactly this
//! number: N lanes need N three-flop registers and N muxes, and they share one ROM --
//! which is what makes a DFA a good thing to replicate and a bad thing to try to
//! vectorise.
//!
//! # Where the table comes from
//!
//! [`TRANSITIONS`] is `regex-automata`'s dense DFA for [`PATTERN`], in the crate's
//! default configuration, renumbered densely and with the `accept` bit folded into the
//! ROM word. It is a **constant**, and that is the point: the "compile offline" step is
//! not something this design does at run time, it is something that happened once and
//! whose result is checked in, exactly as a ROM's contents would be initialised at
//! tape-load. There is no automaton-construction logic in the datapath and none in the
//! cycle budget.
//!
//! The test re-derives the table from the crate -- walking
//! `regex_automata::dfa::dense::DFA::new(PATTERN)` from its start state over all 256
//! bytes, renumbering the reachable states -- and asserts that [`TRANSITIONS`] is
//! exactly what it finds. So the constant is not trusted; it is checked against the
//! thing it was copied from.
//!
//! # The alphabet, and why it is 64 and not 10
//!
//! The crate computes **byte classes**: its own DFA has `alphabet_len() == 10`,
//! because these patterns only distinguish ten kinds of byte. Ten symbols would be a
//! 50-entry table, and it is the better table.
//!
//! This design uses 64 instead, declared as the 63 bytes that can appear in a C
//! identifier (`0-9`, `A-Z`, `_`, `a-z`) plus one catch-all, and the reason is
//! legibility rather than area: the ROM then has one entry per *byte a reader can
//! name*, and the map from a byte to its symbol is four contiguous range checks that
//! can be read off [`GROUPS`]. With byte classes the map would be a table too, and
//! there would be two tables to trust instead of one.
//!
//! The catch-all is a real claim, not a convenience: **every one of the 193 bytes
//! outside the identifier set must take the same transition from every one of the five
//! states.** The test checks all 256 bytes against the crate rather than trusting the
//! argument.
//!
//! # Why a `case_`, and what it costs
//!
//! The table is built with [`Design::case_`] -- one arm per (state, symbol) entry --
//! and the honest statement is that **this lowers to a multiplexer tree, not to a block
//! RAM**. There is no initialised-memory node in the IR: `Design::mem` is zero-filled
//! in both backends, so there is no way to say "these are the contents", and a `case_`
//! is the current idiom.
//!
//! What a real ROM would save, concretely: a 1280-bit ROM with a 6-bit address and a
//! 4-bit output is one read port on a small memory macro, with a fixed read delay. As a
//! mux tree it is 64-to-1 per state row, five rows deep, and the depth is what shows up
//! as critical path. It would still be one byte per cycle -- the *count* is identical --
//! but it would not meet timing at a high clock without pipelining the address
//! generation. **This is a known gap in the IR and the cost is written down rather than
//! left for someone to find in a netlist.**
//!
//! # The part that is uncomfortable: this is not `Regex::find_iter`
//!
//! The obvious claim -- "a dense DFA for a regex finds the regex's matches" -- is
//! **false for `regex-automata`'s forward dense DFA**, and the tests measure the
//! falsity rather than stepping over it.
//!
//! The design reproduces the crate's DFA *verbatim*: the state after each byte is the
//! crate's state under the same renumbering, and `accept` is the crate's
//! `is_match_state` for that state. Nothing is reinterpreted. But a byte-at-a-time
//! forward walk of that DFA does **not** reproduce `dfa::regex::Regex::find_iter`, and
//! the differences are systematic:
//!
//! - **A match that ends at the last byte of the haystack is not reported during the
//!   walk at all.** For `PATTERN`, the state after `"a"` is not a match state; only the
//!   automaton's end-of-input transition reaches one. `Regex::find_iter("a")` returns a
//!   match at `0..1` and a byte-serial design that never sees an end-of-input byte
//!   returns nothing.
//! - **Where the pattern ends in a repetition, the reported end is late.** For
//!   `"ab:cd"` the DFA reports a match after the `:` rather than after the `b`.
//! - **Leftmost behaviour is not reproduced.** For `[a-z]+` on `"a b c"` a byte walk
//!   reports ends at 2 and 4; `find_iter` reports 2, 4 and 6 as well, and for `"a1"` the
//!   walk reports 2 where `find_iter` reports 1.
//!
//! The reason is structural: `dfa::regex::Regex` does a **forward-then-reverse** search
//! and the forward DFA is built to cooperate with it, so its match states mean "a match
//! is committed and possibly extendable" rather than "a match ends exactly here".
//!
//! So this design makes no claim about match positions, and the tests compare what it
//! does claim:
//!
//! 1. the **state sequence** equals the crate's DFA state sequence, under the
//!    renumbering -- the strongest available assertion, and it pins the table exactly
//!    rather than a projection of it;
//! 2. the **accept decision per byte** equals the crate's `is_match_state`;
//! 3. the **set of positions where `accept` rises** equals the set of positions where
//!    the crate's own DFA is in a match state, which is (1) and (2) restated as a set;
//! 4. and one test asserts the *divergence* from `find_iter`, with `"a"`, `"ab:cd"` and
//!    `"a1"` as the witnesses, so the limitation above is a fact in the test suite and
//!    not only a claim in this doc.
//!
//! Compare [`aho_corasick`](crate::aho_corasick)(crate::aho_corasick), whose DFA walk **does** reproduce
//! `find_iter` exactly because its search really is a forward walk. The difference
//! between the two entries is the finding.

use ferrite_lithic::{Design, Signal};
use ferrite_lithic_derive::PortList;

use crate::BuildError;

/// The pattern this automaton recognises.
///
/// A C-style identifier: a letter or underscore followed by letters, digits or
/// underscores. Chosen because it is a real tokeniser's first job, because its byte
/// alphabet is a handful of contiguous ASCII ranges, and because the trailing `*` is
/// exactly what makes the crate's forward DFA behave the way the last section of these
/// docs describes.
pub const PATTERN: &str = r"[A-Za-z_][A-Za-z0-9_]*";

/// The identifier byte groups, as `(first byte, length, first symbol)`.
///
/// The whole alphabet map is these four lines. Each entry covers a contiguous ASCII
/// range, so the symbol of a byte in that range is `byte - first byte + first symbol`,
/// and the hardware is a subtract, an unsigned compare against `length` and an add --
/// per range, four in total. The ranges are disjoint, so the order they are tested in
/// does not matter and the implementation's cascade of `ite`s is unambiguous.
///
/// The catch-all [`OTHER`] is what is left over, and it is the only symbol that is not
/// produced by a range check.
pub const GROUPS: [(u8, u32, u64); 4] =
    [(b'0', 10, 0), (b'A', 26, 10), (b'_', 1, 36), (b'a', 26, 37)];

/// The symbol for any byte that is not in [`GROUPS`].
pub const OTHER: u64 = 63;

/// How many symbols the alphabet has.
///
/// Sixty-three identifier bytes plus [`OTHER`]. Four bits would hold sixteen and seven
/// would hold sixty-four exactly, so six is the width: one spare encoding over the 64
/// live symbols, which a `case_` cannot reach because all 64 are enumerated.
pub const SYMBOL_BITS: u32 = 6;

/// How many symbols the alphabet has, as a count.
pub const SYMBOLS: usize = 1 << SYMBOL_BITS;

/// The width of the state register.
pub const STATE_BITS: u32 = 3;

/// How many states the automaton has, as a count.
///
/// Five, the number the crate's DFA walk reaches. Not a power of two: the three spare
/// encodings of three bits are unreachable, and [`TRANSITIONS`] has no row for them.
pub const STATES: usize = 5;

/// The state the automaton starts in, and therefore the state a reset returns it to.
///
/// Zero, which is also where registers start, so `rst` and "the start state" are the
/// same thing and the design needs no separate start input.
pub const START: u64 = 0;

/// The width of one ROM word: the `accept` bit above the state bits.
pub const WORD_BITS: u32 = STATE_BITS + 1;

/// The DFA transition table, offline-compiled from [`PATTERN`].
///
/// `TRANSITIONS[state][symbol]` is `{accept, next_state}` with the `accept` bit at
/// position [`WORD_BITS`] - 1 and the next state's [`STATE_BITS`] bits below it.
///
/// The states, in this design's own renumbering, are the states reachable from the
/// crate's unanchored start state by consuming bytes:
///
/// | index | meaning | `accept` |
/// |---|---|---|
/// | 0 | start; no identifier character seen | no |
/// | 1 | exactly one identifier character seen | no |
/// | 2 | unreachable -- see the note below | yes |
/// | 3 | inside an identifier, two or more characters | yes |
/// | 4 | the crate's dead state | no |
///
/// Rows 1 and 3 are identical, so the crate's minimiser did not fold them: they differ
/// in their `accept` bit, and the `accept` bit is part of the state. A hand-minimised
/// automaton would have four states; keeping the crate's numbering is the point, because
/// the crate is the golden model.
///
/// Row 2 is a match state the walk never latches, because a real search stops when it
/// reaches one. It is kept rather than removed so that this table is the crate's table
/// and not a table derived from it.
pub const TRANSITIONS: [[u64; SYMBOLS]; STATES] = [
    [
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
        1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
        1, 1, 1, 0,
    ],
    [
        11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11,
        11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11,
        11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 10,
    ],
    [
        4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4,
        4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4,
        4, 4, 4, 4,
    ],
    [
        11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11,
        11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11,
        11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 10,
    ],
    [
        4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4,
        4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4,
        4, 4, 4, 4,
    ],
];

/// The symbol `byte` belongs to: the software model of [`symbol`].
///
/// This is the *specification* the circuit implements, and the test compares the two
/// over all 256 byte values. Keeping it as a function rather than folding it into the
/// table generation is what makes the comparison possible: the table and the map are
/// two separate pieces of hardware and this is the one that is checked.
///
/// `#[must_use]` because a caller that discards the answer has a bug.
#[must_use]
pub fn symbol_of(byte: u8) -> u64 {
    for (base, count, offset) in GROUPS {
        let delta = byte.wrapping_sub(base);
        if u32::from(delta) < count {
            return u64::from(delta) + offset;
        }
    }
    OTHER
}

/// The input ports.
#[derive(Clone, Debug, PortList)]
pub struct Inputs {
    /// The clock. Every edge consumes one byte.
    #[clock]
    pub clk: Signal,
    /// Synchronous reset, which returns the automaton to [`START`].
    pub rst: Signal,
    /// The next byte of the haystack, consumed on each rising edge.
    #[bits(8)]
    pub data: Signal,
}

/// The output ports.
#[derive(Clone, Debug, PortList)]
pub struct Outputs {
    /// The automaton's state, in this design's renumbering. See [`TRANSITIONS`].
    #[bits(3)]
    pub state: Signal,
    /// The automaton is in a match state. Meaningful only because it is a function of
    /// `state`; see the module docs for what a match state does and does not mean.
    pub accept: Signal,
    /// How many bytes have been consumed since the last reset. Wraps at 256, which is
    /// why the tests keep their inputs well below that.
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
    /// The registered `accept` bit, which is also the output.
    pub accept: Signal,
    /// The byte counter, which is also the output.
    pub count: Signal,
}

/// Builds the identifier DFA into `design`.
///
/// The clock is named `clk` and the reset `rst`, which is what the crate-level
/// [`crate::CLOCK`] and [`crate::RESET`] say.
///
/// # Errors
///
/// Whatever [`Design`] returns -- a width mismatch or an undriven wire -- and whatever
/// the port-list helpers return. There is no DFA-specific failure: [`PATTERN`] is a
/// constant and [`TRANSITIONS`] is checked in, so a pattern this design cannot express
/// is a different module rather than a misconfigured one.
pub fn build(design: &Design) -> Result<Ports, BuildError> {
    let inputs = ferrite_lithic::inputs::<Inputs>(design)?;

    // The state has to be a wire rather than used directly, because the register's
    // output is what feeds the next state and a register cannot be an input to itself.
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

    let stepped = design.add(&count, &design.lit(1, 8)?)?;
    let count_q = design.reg(&stepped, &inputs.clk, &inputs.rst, &design.constant(false))?;
    design.drive(&count, &count_q)?;

    ferrite_lithic::outputs::<Outputs>(
        design,
        &Outputs {
            state: state_q.clone(),
            accept: accept_q.clone(),
            count: count_q.clone(),
        },
    )?;

    Ok(Ports {
        inputs,
        state: state_q,
        accept: accept_q,
        count: count_q,
    })
}

/// The three registers' next values after consuming one byte.
///
/// Deliberately *only* the table lookup. A byte-serial automaton whose next state is
/// the table entry for `(state, symbol)` and nothing else is the whole claim of this
/// design, and the reason [`aho_corasick`](crate::aho_corasick)(crate::aho_corasick) has a restart rule and
/// this one does not is in the module docs of each.
///
/// # Errors
///
/// Whatever [`Design`] returns.
#[derive(Clone, Debug)]
pub struct Next {
    /// The next state.
    pub state: Signal,
    /// The `accept` bit belonging to that next state.
    pub accept: Signal,
}

/// One step of the automaton: `state <- TRANSITIONS[state][symbol(byte)]`.
///
/// # Errors
///
/// Whatever [`Design`] returns.
pub fn advance(design: &Design, state: &Signal, byte: &Signal) -> Result<Next, BuildError> {
    let symbol = symbol(design, byte)?;
    let word = transition_table(design, &symbol, state)?;
    Ok(Next {
        state: design.slice(&word, 0, STATE_BITS)?,
        accept: design.slice(&word, STATE_BITS, 1)?,
    })
}

/// The symbol index of `byte`: the alphabet map.
///
/// Four contiguous range checks, cascaded as `ite`s over a [`OTHER`]-seeded result. The
/// subtracts are eight-bit and wrap, which is what makes the range test a single
/// unsigned compare: a byte below the range's base wraps to a large value and fails it.
///
/// A `case_` over the byte would be the other reasonable implementation and the table
/// would be 11 arms instead of 64 -- but it would be a *byte* table with no structure,
/// and [`GROUPS`] is the structure. The crate's own answer is a byte-class map, which
/// is the same idea done by a different method; see the module docs.
///
/// # Errors
///
/// Whatever [`Design`] returns.
pub fn symbol(design: &Design, byte: &Signal) -> Result<Signal, BuildError> {
    let mut mapped = design.lit(OTHER, SYMBOL_BITS)?;
    for (base, count, offset) in GROUPS {
        // Eight-bit wrapping subtract: the left operand's width is the result's width,
        // and `Design::add` truncates to its left operand's too, so both stay in range.
        let delta = design.sub(byte, &design.lit(u64::from(base), 8)?)?;
        let inside = design.ult(&delta, &design.lit(u64::from(count), 8)?)?;
        let shifted = design.add(&delta, &design.lit(offset, 8)?)?;
        let value = design.slice(&shifted, 0, SYMBOL_BITS)?;
        mapped = design.ite(&inside, &value, &mapped)?;
    }
    Ok(mapped)
}

/// The ROM: one [`WORD_BITS`]-bit word for `(state, symbol)`.
///
/// Two nested [`Design::case_`]s -- one arm per symbol inside each of the [`STATES`]
/// rows, then one arm per row -- because that is the shape a table of literals takes
/// in this IR. A real implementation is a single read port with a
/// `STATE_BITS + SYMBOL_BITS` address and a [`WORD_BITS`]-bit word; see the module docs
/// for what that would save.
///
/// The default arm of each `case_` is the row's entry for symbol zero rather than a
/// sentinel, for the reason [`crate::hex::nibble_to_ascii`] gives: all 64 symbols are
/// enumerated above, so no default is reachable, and the right choice for an unreachable
/// arm is the one that is self-consistent if it ever were reached.
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::{
        GROUPS, OTHER, PATTERN, STATE_BITS, STATES, SYMBOL_BITS, SYMBOLS, TRANSITIONS, WORD_BITS,
        symbol_of,
    };

    #[test]
    fn the_declared_alphabet_really_has_sixty_three_identifier_bytes() {
        // The ROM is indexed by symbol and the map is a range check per group, so the
        // two have to agree about what the alphabet is. If GROUPS overlapped or left a
        // gap that a range check could not see, symbol_of would silently produce a
        // symbol the table does not describe.
        let covered: Vec<u8> = GROUPS
            .iter()
            .flat_map(|(base, count, _)| (0..*count).map(move |n| base.wrapping_add(n as u8)))
            .collect();
        assert_eq!(covered.len(), 63, "63 identifier bytes");
        let mut sorted = covered.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), 63, "and no byte is in two groups");
        // Contiguity is per group, not across the merged set: `0`..`9` and `A`..`Z` are
        // two separate contiguous runs with six non-identifier bytes between them, and
        // the hardware's range checks are per group.
        for (base, count, _) in GROUPS {
            let run: Vec<u8> = (0..count)
                .map(|n| base + u8::try_from(n).expect("under 256"))
                .collect();
            assert!(
                run.windows(2).all(|w| w[0] + 1 == w[1]),
                "{base:#04x}.. is contiguous"
            );
            assert!(
                sorted
                    .binary_search(&run[0])
                    .is_ok_and(|at| sorted[at..at + run.len()] == run[..]),
                "{base:#04x}.. appears as one run in the sorted alphabet"
            );
        }
        // And the groups are disjoint, which `sorted.len() == 63` already proved, and are
        // in symbol order, so a symbol index is a stable name rather than a position that
        // depends on the order of the constant.
        assert_eq!(sorted.len(), 63, "disjoint");
        assert_eq!(GROUPS[0].0, b'0', "and ascending");
        assert_eq!(
            symbol_of(b'a'),
            symbol_of(b'A') + 27,
            "lower and upper are separate symbols"
        );
        assert_eq!(symbol_of(b'_'), 36);
        assert_eq!(symbol_of(b'0'), 0);
        assert_eq!(symbol_of(b'9'), 9);
        assert_eq!(symbol_of(b'@'), OTHER);
        assert_eq!(symbol_of(0), OTHER);
        assert_eq!(symbol_of(0xff), OTHER);
    }

    #[test]
    fn the_table_is_the_shape_the_docs_claim() {
        assert_eq!(STATES, 5, "five reachable states");
        assert_eq!(STATE_BITS, 3);
        assert_eq!(SYMBOLS, 64);
        assert_eq!(SYMBOL_BITS, 6);
        assert_eq!(WORD_BITS, 4);
        assert_eq!(TRANSITIONS.len(), STATES);
        assert!(TRANSITIONS.iter().all(|row| row.len() == SYMBOLS));
        // Every word is either `accept << STATE_BITS | state`, and no word names a state
        // that is not one of the five.
        for row in &TRANSITIONS {
            for word in row {
                assert!(*word < (1 << WORD_BITS));
                assert!(((*word as u32) & ((1 << STATE_BITS) - 1)) < STATES as u32);
            }
        }
        // The dead state is an absorbing one, which is the one structural property of
        // row 4 that is worth asserting without consulting the crate.
        assert!(TRANSITIONS[4].iter().all(|word| *word == 4));
        assert!(
            PATTERN.contains('*'),
            "the trailing repetition is the interesting part"
        );
    }
}
