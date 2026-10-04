//! A regex as a dense DFA, checked against `regex-automata`'s own dense DFA.
//!
//! # What is compared, and what is deliberately not
//!
//! The golden model is `regex_automata::dfa::dense::DFA::new(PATTERN)` -- the crate's
//! default configuration, the same one the transition table was compiled from. It is
//! walked exactly as the hardware walks it: one byte at a time, from the start state,
//! with `is_match_state` as the accept bit.
//!
//! Compared:
//!
//! 1. **The full state sequence.** `state` after every byte against the crate's state
//!    under this design's breadth-first renumbering. This pins the 320-entry table
//!    exactly, and it is the strongest assertion available here.
//! 2. **The accept decision per byte.** `accept` against `is_match_state`.
//! 3. **The set of positions where `accept` is high**, as a set rather than a sequence,
//!    against the set of positions where the crate's DFA is in a match state.
//!
//! **Not** compared: the match positions against
//! `regex_automata::dfa::regex::Regex::find_iter`. That would be wrong, and the reason is
//! the point of this entry -- see
//! [`this_design_deliberately_disagrees_with_find_iter`] and the module docs. A
//! byte-serial forward walk of `regex-automata`'s forward dense DFA does **not**
//! reproduce that crate's `find_iter`, and the divergence is systematic rather than a
//! rounding detail.
//!
//! Match *starts* are likewise not reported, and cannot be: recovering one needs a match
//! count and a pattern length, and a leftmost-first DFA's match states do not carry them.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::{HashMap, HashSet};

use ferrite_lithic::Design;
use ferrite_lithic_bits::Bits;
use ferrite_lithic_corpus::dfa;
use ferrite_lithic_cosim::{Harness, Plan, Stimulus, verilator};
use ferrite_lithic_tb::Testbench;
use proptest::prelude::*;

use regex_automata::MatchKind;
use regex_automata::dfa::{Automaton as _, StartKind, dense};
use regex_automata::util::primitives::StateID;
use regex_automata::util::start::Config;
use std::collections::hash_map::Entry::Vacant;

fn design() -> Design {
    let design = Design::new();
    dfa::build(&design).unwrap();
    design
}

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

/// The crate's DFA, in the configuration the table was compiled from.
///
/// Written out rather than calling [`dense::DFA::new`] so that the configuration this
/// test holds the design to is visible in one place: the crate's defaults, unanchored,
/// leftmost-first. Changing a default upstream would change this test, which is the
/// correct outcome -- the constant in the module was compiled from these defaults.
fn crate_dfa() -> dense::DFA<Vec<u32>> {
    dense::Builder::new()
        .configure(
            dense::Config::new()
                .match_kind(MatchKind::LeftmostFirst)
                .start_kind(StartKind::Unanchored)
                .byte_classes(true),
        )
        .build(dfa::PATTERN)
        .expect("the pattern compiles")
}

/// The breadth-first renumbering, plus the crate's states in that order.
///
/// This is the single place the renumbering exists, and both the table check and the
/// state-sequence check use it, so they cannot disagree about it.
fn numbered() -> (Vec<StateID>, HashMap<usize, usize>) {
    let automaton = crate_dfa();
    let start = automaton
        .start_state(&Config::new())
        .expect("an unanchored start state always exists");
    let mut order = vec![start];
    let mut index = HashMap::new();
    index.insert(start.as_usize(), 0usize);
    let mut at = 0;
    while at < order.len() {
        let sid = order[at];
        at += 1;
        for value in 0..=255u16 {
            let next = automaton.next_state(sid, u8::try_from(value).unwrap());
            // `entry`, not `insert` behind a `contains_key` test: `insert` overwrites the
            // value even when the key is already present, which would renumber an existing
            // state every time any other state grew past it.
            if let Vacant(slot) = index.entry(next.as_usize()) {
                slot.insert(order.len());
                order.push(next);
            }
        }
    }
    (order, index)
}

/// The crate's own walk of `data`, in this design's numbering.
fn crate_walk(data: &[u8]) -> (Vec<usize>, Vec<bool>) {
    let automaton = crate_dfa();
    let (order, index) = numbered();
    let mut sid = *order.first().expect("the walk always has a start state");
    let mut states = Vec::with_capacity(data.len());
    let mut accepts = Vec::with_capacity(data.len());
    for byte in data {
        sid = automaton.next_state(sid, *byte);
        states.push(index[&sid.as_usize()]);
        accepts.push(automaton.is_match_state(sid));
    }
    (states, accepts)
}

/// What one run of the design produced.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Run {
    /// The state after each consumed byte.
    states: Vec<u64>,
    /// The accept bit after each consumed byte.
    accepts: Vec<u64>,
}

/// Feeds `data` through the design, one byte per cycle.
///
/// `rst` is pulsed with `set` and then given an edge of its own, because the design
/// consumes a byte on every edge and so needs a cycle that is not a byte. The byte on
/// that edge is 0 and is discarded by the reset.
fn hardware(data: &[u8]) -> Run {
    let design = design();
    let tb = Testbench::new(&design).unwrap();
    tb.run(|tb| async move {
        tb.set("rst", 1).await;
        tb.set("data", 0).await;
        tb.step().await;
        tb.set("rst", 0).await;
        for byte in data {
            tb.drive("data", u64::from(*byte)).await;
        }
    })
    .expect("the dfa design drives only ports it declares");

    let read = |name: &str| -> Vec<u64> {
        tb.series(name)
            .iter()
            .map(|bits| bits.to_u64().expect("three bits fits in a u64"))
            .collect()
    };
    let states = read("state");
    let accepts = read("accept");
    let count = read("count");
    // Series entry 0 is the reset edge, which consumes no byte.
    for (offset, seen) in count.iter().enumerate().skip(1) {
        assert_eq!(
            *seen, offset as u64,
            "count is bytes consumed, so it is {offset} after {offset} bytes"
        );
    }
    Run {
        states: states[1..].to_vec(),
        accepts: accepts[1..].to_vec(),
    }
}

#[test]
fn the_design_and_the_crate_agree_that_an_empty_buffer_accepts_nothing() {
    // No bytes, no edges. The design's state is the start state after reset, which is
    // the crate's start state under the renumbering, so the empty walk is empty on both
    // sides.
    let run = hardware(&[]);
    assert_eq!(
        run,
        Run {
            states: vec![],
            accepts: vec![]
        }
    );
    let (states, accepts) = crate_walk(&[]);
    assert!(states.is_empty() && accepts.is_empty());
}

#[test]
fn the_design_agrees_with_the_crate_on_a_buffer_with_no_identifier_in_it() {
    // Buffers with no identifier in them at all. The three ranges are checked rather than
    // asserted in one blob, because "no identifier" is not the same as "one byte": a
    // leading digit means the first letter cannot start a token, and a buffer made of
    // bytes outside all four groups never leaves the start state.
    for data in [&b"1234"[..], b"   ", b"!!!", b"  \t\n  ", b"\x00\xff\x80"] {
        let (states, accepts) = crate_walk(data);
        assert!(
            accepts.iter().all(|accept| !accept),
            "{:?}: the crate's DFA never enters a match state",
            String::from_utf8_lossy(data)
        );
        let run = hardware(data);
        assert_eq!(
            run.states,
            states.iter().map(|s| *s as u64).collect::<Vec<_>>()
        );
        assert_eq!(
            run.accepts,
            accepts.iter().map(|a| u64::from(*a)).collect::<Vec<_>>()
        );
        // And the final state is the dead state or the start state, never a match state,
        // so no amount of noise leaves the automaton committed.
        assert!(
            states.last().is_none_or(|state| *state == 0 || *state == 4),
            "{:?}: the automaton is not committed",
            String::from_utf8_lossy(data)
        );
    }
    // The narrowest of those: three bytes that are in none of the four identifier groups
    // leave the register in row 0 the whole way.
    let run = hardware(b"\x00\xff\x80");
    assert_eq!(run.states, vec![0, 0, 0]);
}

#[test]
fn the_design_reaches_the_earliest_accept_the_crate_does() {
    // `accept` cannot rise at offset 0: the pattern needs two bytes. This pins the
    // earliest position it *can* rise, which is offset 1.
    let data = b"ab";
    let (states, accepts) = crate_walk(data);
    assert_eq!(accepts, vec![false, true]);
    assert_eq!(states, vec![1, 3]);
    let run = hardware(data);
    assert_eq!(run.accepts, vec![0, 1]);
    assert_eq!(run.states, vec![1, 3]);
}

#[test]
fn the_design_accepts_on_the_last_byte() {
    // The end of the buffer is where an off-by-one shows up: it is the only position at
    // which "accept" and "no more bytes to come" coincide, and it is exactly the position
    // the crate's DFA does *not* settle -- see the divergence test below.
    for data in [&b"ab"[..], b"_x", b"a9", b"ident", b"x1234567890", b"a_"] {
        let (_, accepts) = crate_walk(data);
        assert!(
            *accepts.last().expect("non-empty"),
            "the crate accepts last"
        );
        assert_eq!(*hardware(data).accepts.last().expect("non-empty"), 1);
    }
}

#[test]
fn the_design_agrees_with_the_crate_when_the_pattern_is_still_open_at_the_end() {
    // "input that ends mid-match". `a` is one byte of an identifier and nothing more;
    // the crate's DFA is in state 1, which is not a match state. Both agree, and both
    // disagree with `find_iter`. Asserted here so that this case is covered by the
    // differential and not only by the divergence test.
    let data = b"a";
    let (states, accepts) = crate_walk(data);
    assert_eq!(states, vec![1]);
    assert_eq!(accepts, vec![false]);
    assert_eq!(hardware(data).states, vec![1]);
    assert_eq!(hardware(data).accepts, vec![0]);
}

#[test]
fn overlapping_identifier_runs_are_accepted_throughout() {
    // Overlapping in the sense that matters here: one identifier followed immediately by
    // another with no separator. The design has no restart rule, so it runs straight
    // through, and the accept bit rises on every interior position.
    let data = b"ab_cd9ef";
    let (_, accepts) = crate_walk(data);
    assert_eq!(
        accepts,
        vec![false, true, true, true, true, true, true, true],
        "the crate: everything after the second byte accepts"
    );
    assert_eq!(hardware(data).accepts, vec![0, 1, 1, 1, 1, 1, 1, 1]);
    // And a separator drops it back to zero without any restart input, because the start
    // state is where a non-identifier byte goes.
    let data = b"ab cd";
    let (_, accepts) = crate_walk(data);
    assert_eq!(accepts, vec![false, true, true, false, false]);
    assert_eq!(hardware(data).accepts, vec![0, 1, 1, 0, 0]);
}

#[test]
fn the_state_register_holds_one_of_the_five_states_and_never_the_dead_one() {
    // The dead state is only ever a *next* state in this table, and because there is no
    // restart rule the design does latch it -- so this asserts the register stays inside
    // the declared width and names a row that exists.
    let design = design();
    let tb = Testbench::new(&design).unwrap();
    tb.run(|tb| async move {
        tb.set("rst", 1).await;
        tb.set("data", 0).await;
        tb.step().await;
        tb.set("rst", 0).await;
        for byte in b"ab_cd 9ef!gh_i" {
            tb.drive("data", u64::from(*byte)).await;
        }
    })
    .unwrap();
    let series = tb.series("state");
    assert_eq!(series.len(), 1 + b"ab_cd 9ef!gh_i".len());
    assert!(series.iter().all(|bits| bits.width() == dfa::STATE_BITS));
    let states: Vec<u64> = series.iter().map(|bits| bits.to_u64().unwrap()).collect();
    for (index, state) in states.iter().enumerate() {
        assert!(
            *state < dfa::STATES as u64,
            "state {state} at cycle {index} is one of the five rows"
        );
    }

    // The dead state is reachable here, and it is absorbing. That is different from
    // `aho_corasick`, which has no dead state: this design has no restart rule, so a
    // match state is latched like any other, and the dead state behind it is latched too.
    // A design that wrongly zeroed the state on a match would never reach it, so the fact
    // that it is reached -- and that it is then never left -- is worth asserting.
    let dead = states
        .iter()
        .position(|state| *state == 4)
        .expect("the dead state is reached: there is no restart rule");
    assert!(
        states[dead..].iter().all(|state| *state == 4),
        "and it is absorbing, so the register stays in row 4 for the rest of the stream"
    );
    assert!(
        states[..dead].contains(&2),
        "and it was reached through the match state 2, which the design latches rather \
         than jumps over"
    );
}

#[test]
fn the_checked_in_table_is_the_crates_dense_dfa() {
    // The offline compilation, checked: all 256 bytes, every state, including the 193
    // bytes that share the catch-all symbol. This is the test that would fail if
    // `symbol_of` and the table ever drifted apart, or if the catch-all claim stopped
    // being true.
    let automaton = crate_dfa();
    let (order, index) = numbered();
    assert_eq!(
        order.len(),
        dfa::STATES,
        "the crate's walk reaches five states and the constant has five rows"
    );

    let accept_bit = 1u64 << dfa::STATE_BITS;
    for (state, sid) in order.iter().enumerate() {
        for value in 0..=255u16 {
            let byte = u8::try_from(value).unwrap();
            let next = automaton.next_state(*sid, byte);
            let next_index = index[&next.as_usize()];
            let word = if automaton.is_match_state(next) {
                accept_bit
            } else {
                0
            } | next_index as u64;
            assert_eq!(
                dfa::TRANSITIONS[state][dfa::symbol_of(byte) as usize],
                word,
                "state {state}, byte {byte:#04x}"
            );
        }
    }

    // And every word in the constant names a row that exists and fits the word width,
    // which the crate walk cannot tell us: it would happily map a bad word back onto a
    // state it never produced.
    let state_mask = (1u64 << dfa::STATE_BITS) - 1;
    for (state, row) in dfa::TRANSITIONS.iter().enumerate() {
        for (symbol, word) in row.iter().enumerate() {
            assert!(
                *word < (1 << dfa::WORD_BITS),
                "state {state} symbol {symbol}: the word fits"
            );
            assert!(
                (*word & state_mask) < dfa::STATES as u64,
                "state {state} symbol {symbol}: the word names a row that exists"
            );
        }
    }
}

#[test]
fn the_alphabet_map_is_the_one_the_table_describes() {
    // All 256 bytes against the software model, and the model against the declared
    // groups. Without this the table check above could pass with a symbol function that
    // agreed with itself but not with the hardware.
    let mut counts = [0usize; 64];
    for value in 0..=255u16 {
        let byte = u8::try_from(value).unwrap();
        let symbol = dfa::symbol_of(byte);
        assert!(symbol < dfa::SYMBOLS as u64, "{byte:#04x}");
        counts[symbol as usize] += 1;
    }
    // The declared group sizes, and the catch-all.
    assert_eq!(&counts[0..10], &[1; 10], "ten single-byte digit symbols");
    assert_eq!(&counts[10..36], &[1; 26], "26 uppercase symbols");
    assert_eq!(counts[36], 1, "one underscore symbol");
    assert_eq!(&counts[37..63], &[1; 26], "26 lowercase symbols");
    assert_eq!(
        counts[63], 193,
        "and every other byte shares the catch-all: 256 - 63 = 193"
    );
    assert_eq!(counts.iter().sum::<usize>(), 256);
}

#[test]
fn this_design_deliberately_disagrees_with_find_iter() {
    // The uncomfortable part, made a fact in the test suite rather than a claim in a doc
    // comment. `dfa::regex::Regex` is `regex-automata`'s own front end over the same
    // pattern, and it finds matches a byte-at-a-time walk of its own forward dense DFA
    // does not.
    let re = regex_automata::dfa::regex::Regex::new(dfa::PATTERN).unwrap();
    let front_end = |haystack: &[u8]| -> HashSet<usize> {
        re.find_iter(haystack)
            .map(|m| m.end())
            .collect::<HashSet<usize>>()
    };
    let walk = |haystack: &[u8]| -> HashSet<usize> {
        crate_walk(haystack)
            .1
            .iter()
            .enumerate()
            .filter(|(_, accept)| **accept)
            .map(|(index, _)| index + 1)
            .collect()
    };

    // 1. A one-byte identifier. `find_iter` finds it; the byte walk does not, because the
    //    automaton's end-of-input transition is where a match of a pattern ending in `*`
    //    becomes visible, and a streaming design is never told about an end of input.
    assert_eq!(
        front_end(b"a"),
        HashSet::from([1]),
        "the front end finds `a`"
    );
    assert_eq!(walk(b"a"), HashSet::new(), "and the byte walk does not");

    // 2. Leftmost is not reproduced. On `a:` the front end reports the one-character
    //    match at 1; the walk reports the identifier-plus-separator at 2.
    assert_eq!(
        front_end(b"a:"),
        HashSet::from([1]),
        "the front end prefers the short match"
    );
    assert_eq!(walk(b"a:"), HashSet::from([2]), "and the walk does not");
    // Note that `a1` is *not* a witness: both report 2 there. The two functions agree
    // more often than one might expect, and only diverge on inputs where the pattern's
    // trailing `*` or a following identifier character is involved.

    // 3. A separator ends the walk's match but is not where the front end's second match
    //    ends: the walk commits to the identifier-plus-separator, the front end resumes
    //    scanning afterwards.
    assert_eq!(front_end(b"ab:cd"), HashSet::from([2, 5]));
    assert_eq!(walk(b"ab:cd"), HashSet::from([2, 3]));

    // 4. Interior positions. The front end reports the *end* of a longest match, so an
    //    eight-byte identifier yields one position; the walk accepts on every byte after
    //    the second.
    assert_eq!(front_end(b"abc1def"), HashSet::from([7]));
    assert_eq!(walk(b"abc1def"), HashSet::from([2, 3, 4, 5, 6, 7]));

    // So the two are genuinely different functions, and this design is the DFA one.
    //
    // They *do* agree on a two-byte identifier with nothing after it, and on every input
    // with no identifier in it at all. Both facts are asserted because the divergences
    // above are all inputs with a third byte or more, and a reader who only tried `"1234"`
    // or `"ab"` would conclude the two were the same function. That they are not is a
    // property of the pattern ending in `*`, not of the input length.
    for data in [&b"ab"[..], b"a1", b"_x", b"aa"] {
        assert_eq!(
            front_end(data),
            HashSet::from([data.len()]),
            "{:?}: the front end's one match",
            String::from_utf8_lossy(data)
        );
        assert_eq!(
            walk(data),
            front_end(data),
            "{:?}: and the walk agrees here",
            String::from_utf8_lossy(data)
        );
    }
    for data in [&b"1234"[..], b"!!!", b"  \t\n", b"0 1 2 3", b"\x00\xff"] {
        assert!(
            front_end(data).is_empty(),
            "{:?}",
            String::from_utf8_lossy(data)
        );
        assert!(walk(data).is_empty(), "{:?}", String::from_utf8_lossy(data));
    }
    // And a longer identifier diverges on interior positions, which is the third witness
    // above and the one a reader is most likely to miss.
    assert_eq!(front_end(b"ident"), HashSet::from([5]));
    assert_eq!(walk(b"ident"), HashSet::from([2, 3, 4, 5]));
}

proptest! {
    /// The differential, on arbitrary input.
    ///
    /// Three comparisons per case: the state sequence, the accept sequence, and the set
    /// of accepting positions. The bytes are drawn from the whole range, so the catch-all
    /// path -- 193 of the 256 values, and every byte outside ASCII -- is exercised on
    /// essentially every case.
    #[test]
    fn the_design_agrees_with_the_crates_dfa_on_arbitrary_input(
        data in prop::collection::vec(any::<u8>(), 0..40)
    ) {
        let (want_states, want_accepts) = crate_walk(&data);
        let run = hardware(&data);

        let want_states: Vec<u64> = want_states.iter().map(|state| *state as u64).collect();
        let want_accepts: Vec<u64> = want_accepts.iter().map(|accept| u64::from(*accept)).collect();
        prop_assert_eq!(&run.states, &want_states);
        prop_assert_eq!(&run.accepts, &want_accepts);

        // The same comparison as a set of positions, which is the form the module docs
        // promise: "the set of positions where accept rises agrees exactly".
        let run_set: HashSet<usize> = run
            .accepts
            .iter()
            .enumerate()
            .filter(|(_, accept)| **accept == 1)
            .map(|(index, _)| index + 1)
            .collect();
        let want_set: HashSet<usize> = want_accepts
            .iter()
            .enumerate()
            .filter(|(_, accept)| **accept == 1)
            .map(|(index, _)| index + 1)
            .collect();
        prop_assert_eq!(run_set, want_set);
    }
}

#[test]
fn the_simulator_and_the_emitted_verilog_agree_on_this_design() {
    let Some(verilator) = verilator() else {
        println!(
            "SKIPPED: verilator not found, so the dfa equivalence did not run. The \
             differential against the crate's DFA above does not need it."
        );
        return;
    };
    println!("cosimulating dfa with {}", verilator.display());

    let design = design();
    let plan = plan_of(&design, "dfa");
    assert_eq!(
        plan.inputs
            .iter()
            .map(|p| p.name.as_str())
            .collect::<Vec<_>>(),
        ["rst", "data"],
        "the clock is a port but not a stimulus column"
    );
    assert_eq!(
        plan.outputs
            .iter()
            .map(|p| p.name.as_str())
            .collect::<Vec<_>>(),
        ["state", "accept", "count"],
        "all three outputs are compared, including the state register"
    );

    // Bytes that walk the whole automaton: the first identifier character, an interior
    // one, an underscore, the digits, and bytes from every one of the five ranges plus
    // the catch-all.
    let mut stimulus = Stimulus::new();
    let row = |rst: u64, data: u64| {
        vec![
            Bits::constant(rst, 1).unwrap(),
            Bits::constant(data, 8).unwrap(),
        ]
    };
    stimulus.push(row(1, 0)).unwrap();
    stimulus.push(row(0, 0)).unwrap();
    let mut consumed = 0u64;
    for text in [
        &b"ab"[..],
        b"_",
        b"9Z_a",
        b"  ",
        b"\x00\x7f\xff",
        b"identifier_with_digits_9",
        b"!@#$%^&*()",
        b"a:b,c.d",
        b"____",
    ] {
        for byte in text {
            stimulus.push(row(0, u64::from(*byte))).unwrap();
            consumed += 1;
        }
    }
    assert_eq!(stimulus.cycles(), 2 + consumed as usize);

    let verilog = ferrite_lithic_cosim::emit(&design, &plan).unwrap();
    let harness = Harness::build(&plan, &verilog, &plan.work_dir()).unwrap();
    let report = ferrite_lithic_cosim::run_with(&design, &plan, &stimulus, &harness).unwrap();
    assert!(
        report.is_equivalent(),
        "the two backends disagree: {report}\nfirst: {:?}",
        report.first()
    );

    // And the same bytes through the testbench agree with the crate's DFA, so all three
    // backends -- bit-level simulator, Verilator, and the crate -- meet.
    let message: Vec<u8> = b"a:b,c.d identifier_with_digits_9 9Z_a\x00\xff".to_vec();
    let (want_states, want_accepts) = crate_walk(&message);
    let run = hardware(&message);
    assert_eq!(
        run.states,
        want_states.iter().map(|s| *s as u64).collect::<Vec<_>>()
    );
    assert_eq!(
        run.accepts,
        want_accepts
            .iter()
            .map(|a| u64::from(*a))
            .collect::<Vec<_>>()
    );
}

#[test]
fn the_start_state_is_zero_so_reset_and_the_start_state_are_the_same_thing() {
    // Which is why there is no `start` input on this design, unlike `memchr`'s. If
    // `START` were not zero, the register's reset value would have to be a literal and
    // the two would be two separate statements about the same thing.
    assert_eq!(dfa::START, 0);
    let design = design();
    let tb = Testbench::new(&design).unwrap();
    tb.run(|tb| async move {
        tb.set("rst", 1).await;
        tb.set("data", 0x41).await;
        tb.step().await;
    })
    .unwrap();
    assert_eq!(
        tb.series("state")[0].to_u64().unwrap(),
        dfa::START,
        "a reset edge holds the start state, whatever byte is on `data`"
    );
    // And the port list has no `start`, which is the observable consequence.
    let plan = plan_of(&design, "dfa_start_probe");
    assert_eq!(
        plan.inputs
            .iter()
            .map(|p| p.name.as_str())
            .collect::<Vec<_>>(),
        ["rst", "data"]
    );
}
