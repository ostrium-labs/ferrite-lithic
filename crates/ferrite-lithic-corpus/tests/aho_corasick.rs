//! Multi-pattern search, checked against `aho_corasick::AhoCorasick::find_iter`.
//!
//! The golden model is the crate's own front end, over the same patterns and the same
//! [`MatchKind`]. Nothing here is transcribed: [`aho_corasick::TRANSITIONS`] was
//! compiled from the crate's `dfa::DFA` and the tests below walk that same `dfa::DFA`
//! again to check the constant, and then compare the design's *emitted* matches --
//! `(end position, pattern index)` pairs -- against `find_iter`'s.
//!
//! What is compared, precisely:
//!
//! - **`(count, pattern)` at every cycle where `accept` is high** against
//!   `(match.end(), match.pattern())` from `find_iter`, as an ordered sequence. `count`
//!   at the accepting cycle *is* the exclusive end offset of the match.
//! - **Match starts are not compared**, because the design does not report them and
//!   cannot: recovering a start needs the pattern's length and a running match count.
//! - **`state` is compared** against the crate's DFA under the stop rule, which is what
//!   makes `accept` mean anything.
//!
//! The interesting cases are the prefix pair. `in` is a prefix of `info`, both are in
//! the pattern set, and `find_iter("info")` reports `in` at `0..2` and nothing else --
//! so a design that reported `info` there, or that reported both, would be wrong, and
//! neither mistake is caught by any test that only uses disjoint keywords.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use ferrite_lithic::Design;
use ferrite_lithic_bits::Bits;
use ferrite_lithic_corpus::aho_corasick;
// The design module is called `aho_corasick` and so is the golden crate, so the crate is
// brought in under names that cannot be confused with the module's own items.
use ::aho_corasick::automaton::Automaton as _;
use ::aho_corasick::{AhoCorasick, Anchored, MatchKind};
use ferrite_lithic_cosim::{Harness, Plan, Stimulus, verilator};
use ferrite_lithic_tb::Testbench;
use proptest::prelude::*;

use std::collections::hash_map::Entry::Vacant;

/// The same pattern set the design compiled, and the same match semantics.
///
/// Duplicated here rather than shared, because the point of the test is that the design
/// module and the golden model are two independent statements of the same thing.
fn patterns() -> Vec<&'static [u8]> {
    aho_corasick::PATTERNS.to_vec()
}

/// The crate's front end, in the same configuration the table was compiled from.
fn golden() -> AhoCorasick {
    AhoCorasick::builder()
        .match_kind(MatchKind::Standard)
        .build(patterns())
        .expect("the pattern set has no duplicates")
}

/// The `(end, pattern index)` pairs `find_iter` reports, in order.
fn golden_matches(haystack: &[u8]) -> Vec<(u64, u64)> {
    let ac = golden();
    ac.find_iter(haystack)
        .map(|m| {
            (
                u64::try_from(m.end()).expect("a test haystack is short"),
                m.pattern().as_usize() as u64,
            )
        })
        .collect()
}

fn design() -> Design {
    let design = Design::new();
    aho_corasick::build(&design).unwrap();
    design
}

/// The cosimulation plan for a built design.
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

/// The crate's dense DFA over [`patterns`], in the configuration the table was compiled
/// from.
fn crate_dfa() -> ::aho_corasick::dfa::DFA {
    ::aho_corasick::dfa::DFA::builder()
        .match_kind(MatchKind::Standard)
        .build(patterns())
        .expect("the pattern set has no duplicates")
}

/// Every byte that appears in any of the patterns, for the keyword-biased strategy.
fn keyword_bytes() -> Vec<u8> {
    let mut bytes: Vec<u8> = patterns().iter().flat_map(|p| p.iter().copied()).collect();
    bytes.sort_unstable();
    bytes.dedup();
    assert!(!bytes.is_empty());
    bytes
}

/// What one run of the design produced.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Run {
    /// The state after each consumed byte.
    states: Vec<u64>,
    /// The `(end, pattern index)` the design emitted, in order.
    matches: Vec<(u64, u64)>,
}

/// Feeds `data` through the design, one byte per cycle.
///
/// `rst` is pulsed with `set` and then given an edge of its own, because the design
/// consumes a byte on every edge and so the reset needs a cycle that is not a byte. The
/// byte on that edge is 0 and is discarded by the reset.
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
    .expect("the aho_corasick design drives only ports it declares");

    let count: Vec<u64> = tb
        .series("count")
        .iter()
        .map(|bits| bits.to_u64().expect("eight bits fits in a u64"))
        .collect();
    let accept: Vec<u64> = tb
        .series("accept")
        .iter()
        .map(|bits| bits.to_u64().expect("one bit fits in a u64"))
        .collect();
    let pattern: Vec<u64> = tb
        .series("pattern")
        .iter()
        .map(|bits| bits.to_u64().expect("three bits fits in a u64"))
        .collect();
    let states: Vec<u64> = tb
        .series("state")
        .iter()
        .map(|bits| bits.to_u64().expect("five bits fits in a u64"))
        .collect();

    // Series entry 0 is the reset edge, which consumes no byte. Every later entry is
    // one consumed byte.
    let mut run = Run {
        states: states[1..].to_vec(),
        matches: Vec::new(),
    };
    for (index, hit) in accept.iter().enumerate().skip(1) {
        assert_eq!(
            count[index], index as u64,
            "count is the number of bytes consumed, so at byte {index} it is {index}"
        );
        if *hit == 1 {
            run.matches.push((count[index], pattern[index]));
        }
    }
    run
}

#[test]
fn the_design_and_the_crate_agree_that_there_are_no_matches_in_the_empty_buffer() {
    // No bytes means no edges means no series, and `find_iter("")` yields nothing. A
    // design that reported a match at offset 0 for an empty buffer would be reporting
    // something nobody asked for.
    let run = hardware(&[]);
    assert_eq!(
        run,
        Run {
            states: vec![],
            matches: vec![]
        }
    );
    assert_eq!(golden_matches(b""), vec![]);
}

#[test]
fn the_design_finds_a_match_at_the_start_of_the_buffer() {
    // `find_iter` reports the first match starting at 0, and the design's first emitted
    // match has `count` equal to that match's end.
    let run = hardware(b"info");
    assert_eq!(run.matches, vec![(2, 4)]);
    assert_eq!(golden_matches(b"info"), vec![(2, 4)]);
    assert_eq!(
        run.matches[0].0, 2,
        "the match ends at 2, so the design consumed exactly two bytes to find it"
    );
}

#[test]
fn the_design_finds_a_match_on_the_last_byte() {
    // Every keyword, planted so that its last byte is the last byte of the buffer. The
    // end of the buffer is where a counter that ran a cycle early would show up.
    for (index, pattern) in patterns().iter().enumerate() {
        let data = pattern.to_vec();
        let want = golden_matches(&data);
        assert!(!want.is_empty(), "pattern {index} must match itself");
        let run = hardware(&data);
        assert_eq!(run.matches, want, "pattern {index} on the last byte");
        assert_eq!(run.states.len(), data.len());
        let shadowed = patterns()
            .iter()
            .any(|other| other.len() < pattern.len() && pattern.starts_with(other));
        assert_eq!(
            want.last().expect("non-empty").0 == data.len() as u64,
            !shadowed,
            "pattern {index}: a shadowed pattern's match cannot end at the end of its own \
             buffer, because a shorter pattern already matched inside it"
        );
    }
    // Which leaves exactly the two shadowed ones, and the shadowing is worth stating as a
    // fact rather than as a carve-out: there is no haystack in which `info`'s match ends
    // at the end, because `in` is a prefix of it and the leftmost rule always prefers it.
    assert_eq!(golden_matches(b"info"), vec![(2, 4)], "`in` shadows `info`");
    assert_eq!(hardware(b"info").matches, vec![(2, 4)]);
    assert_eq!(
        golden_matches(b"info!"),
        vec![(2, 4)],
        "with trailing noise too"
    );
    assert_eq!(hardware(b"info!").matches, vec![(2, 4)]);
}

#[test]
fn the_design_reports_no_match_when_there_is_none() {
    // A buffer built from bytes that are in *none* of the patterns, so it cannot contain a
    // keyword however the automaton is wired. `in` is two of the ten alphabet bytes, so a
    // buffer of the other eight cannot accidentally match -- which is worth having as a
    // case, because an earlier version of this test used `"hello world, this is a log
    // line"` and `line` contains `in`.
    const QUIET: &[u8] = b"ghjklqstuvxyz0123456789!@#$%^&*()_+";
    for byte in QUIET {
        assert!(
            !aho_corasick::ALPHABET.contains(byte),
            "{byte:#04x} is not in the alphabet"
        );
    }
    assert_eq!(golden_matches(QUIET), vec![]);
    let run = hardware(QUIET);
    assert_eq!(run.matches, vec![]);
    assert!(
        run.states.iter().all(|state| *state == 0),
        "and it never left the start state, because every byte maps to the catch-all"
    );
}

#[test]
fn a_prefix_pair_reports_the_shorter_pattern_and_only_one_match() {
    // The case the whole pattern set exists for. `in` is a prefix of `info`; the
    // leftmost rule says report `in` at `0..2` and start the next search there, where
    // `fo` matches nothing. A design that reported `info`, or that reported both, would
    // pass every disjoint-keyword test in this file.
    for input in [b"info".as_slice(), b"inf", b"in"] {
        assert_eq!(
            golden_matches(input),
            vec![(2, 4)],
            "{}: the crate says `in`, not `info`",
            String::from_utf8_lossy(input)
        );
        assert_eq!(
            hardware(input).matches,
            vec![(2, 4)],
            "{}: the design agrees",
            String::from_utf8_lossy(input)
        );
    }
    // And `info` really is in the set, so the above is a discrimination and not a
    // tautology.
    let set = patterns();
    assert!(set.iter().any(|p| *p == b"info".as_slice()));
    assert!(set.iter().any(|p| *p == b"in".as_slice()));
}

#[test]
fn overlapping_occurrences_are_reported_one_after_another() {
    // Three matches from one buffer, and the restart rule is what makes the second and
    // third appear: without it the automaton would stay in the `n`-of-`in` state and
    // miss them. This is the test that fails if the `ite` in `advance` is removed.
    let data = b"in in in";
    assert_eq!(golden_matches(data), vec![(2, 4), (5, 4), (8, 4)]);
    assert_eq!(hardware(data).matches, vec![(2, 4), (5, 4), (8, 4)]);
}

#[test]
fn a_match_at_a_position_nonzero_is_found() {
    // The needle is not at the start, which is where a design that reset to the start
    // state on every non-alphabet byte and forgot to *stay* there would go wrong.
    let data = b"an info line";
    assert_eq!(golden_matches(data), vec![(5, 4), (11, 4)]);
    assert_eq!(hardware(data).matches, vec![(5, 4), (11, 4)]);
}

#[test]
fn input_that_ends_mid_match_still_reports_what_completed() {
    // `war` is three of the four bytes of `warn`: no match. `warn` truncated to `war` and
    // then continued is a match. Both are in the tests because a design with a stale
    // match bit would report `warn` for `war`.
    assert_eq!(golden_matches(b"war"), vec![]);
    assert_eq!(hardware(b"war").matches, vec![]);
    assert_eq!(golden_matches(b"warn"), vec![(4, 2)]);
    assert_eq!(hardware(b"warn").matches, vec![(4, 2)]);
    // A pattern that starts and is abandoned, with a later complete match after it.
    let data = b"erro then error";
    assert_eq!(
        golden_matches(data),
        vec![(15, 1)],
        "the crate finds the complete `error` and not the abandoned `erro`"
    );
    assert_eq!(hardware(data).matches, vec![(15, 1)]);
}

#[test]
fn the_design_agrees_with_the_crate_across_buffer_lengths() {
    // Lengths 0..=40 over a fixed interesting body, so the length is the only thing
    // varying. Forty is chosen because the longest pattern is five bytes and a buffer
    // has to be able to contain several of them plus the separators.
    let body = b"panic warn info error in ";
    for length in 0..=40usize {
        let data: Vec<u8> = body.iter().copied().cycle().take(length).collect();
        let want = golden_matches(&data);
        assert_eq!(hardware(&data).matches, want, "length {length}");
    }
}

#[test]
fn the_checked_in_table_is_the_crates_dfa() {
    // The offline compilation, checked. This walks the crate's `dfa::DFA` -- all 256
    // bytes, breadth-first from the start state, renumbered densely -- and asserts that
    // the constant in the module is exactly what the walk produces. Every byte, every
    // state, including the 246 bytes that share the catch-all symbol.
    let ac = crate_dfa();
    let start = ac.start_state(Anchored::No).unwrap();

    let mut order = vec![start];
    let mut index = std::collections::HashMap::new();
    index.insert(start, 0usize);
    let mut at = 0;
    while at < order.len() {
        let sid = order[at];
        at += 1;
        for value in 0..=255u16 {
            let next = ac.next_state(Anchored::No, sid, u8::try_from(value).unwrap());
            // `entry`, not `insert` behind a `contains_key` test: `insert` overwrites the
            // value even when the key is already present, which would renumber an existing
            // state every time any other state grew past it.
            if let Vacant(slot) = index.entry(next) {
                slot.insert(order.len());
                order.push(next);
            }
        }
    }
    assert_eq!(
        order.len(),
        aho_corasick::STATES,
        "the crate's walk reaches nineteen states and the constant has nineteen rows"
    );

    let accept_bit = 1u64 << aho_corasick::STATE_BITS;
    for (state, sid) in order.iter().enumerate() {
        for value in 0..=255u16 {
            let byte = u8::try_from(value).unwrap();
            let next = ac.next_state(Anchored::No, *sid, byte);
            let next_index = index[&next];
            let word = if ac.is_match(next) { accept_bit } else { 0 } | next_index as u64;
            assert_eq!(
                aho_corasick::TRANSITIONS[state][aho_corasick::symbol_of(byte) as usize],
                word,
                "state {state}, byte {byte:#04x}"
            );
        }
        let pid = if ac.is_match(*sid) {
            u64::from(ac.match_pattern(*sid, 0).as_u32())
        } else {
            0
        };
        assert_eq!(
            aho_corasick::MATCH_PIDS[state],
            pid,
            "state {state}: the pattern it reports"
        );
    }
}

#[test]
fn the_state_sequence_is_the_crates_dfa_under_the_stop_rule() {
    // The same walk as above, applied to a haystack, so that the *design's* state
    // register is pinned to the crate's automaton rather than only to the constant.
    for input in [
        &b"panic"[..],
        b"warn and error",
        b"in in in",
        b"an info line",
        b"hello world",
        b"p",
        b"pa",
        b"pan",
        b"pana",
        b"panic! panic? error.",
    ] {
        let ac = crate_dfa();
        let start = ac.start_state(Anchored::No).unwrap();
        let mut order = vec![start];
        let mut index = std::collections::HashMap::new();
        index.insert(start, 0usize);
        let mut at = 0;
        while at < order.len() {
            let sid = order[at];
            at += 1;
            for value in 0..=255u16 {
                let next = ac.next_state(Anchored::No, sid, u8::try_from(value).unwrap());
                // `entry`, not `insert` behind a `contains_key` test: `insert` overwrites
                // the value even when the key is already present, which would renumber an
                // existing state every time any other state grew past it. That is a real
                // bug and it was written, caught by the test, and is now the shape here.
                if let Vacant(slot) = index.entry(next) {
                    slot.insert(order.len());
                    order.push(next);
                }
            }
        }
        let mut want = Vec::new();
        let mut sid = start;
        for byte in input {
            sid = ac.next_state(Anchored::No, sid, *byte);
            if ac.is_match(sid) {
                // The stop rule: a leftmost search restarts at the end of its match.
                sid = start;
            }
            want.push(index[&sid] as u64);
        }
        assert_eq!(
            hardware(input).states,
            want,
            "{}",
            String::from_utf8_lossy(input)
        );
    }
}

#[test]
fn the_state_port_is_five_bits_and_never_exceeds_it() {
    let design = design();
    let tb = Testbench::new(&design).unwrap();
    tb.run(|tb| async move {
        tb.set("rst", 1).await;
        tb.set("data", 0).await;
        tb.step().await;
        tb.set("rst", 0).await;
        for _ in 0..3 {
            for byte in b"panic error warn info in" {
                tb.drive("data", u64::from(*byte)).await;
            }
        }
    })
    .unwrap();
    let series = tb.series("state");
    assert_eq!(
        series.len(),
        1 + 3 * b"panic error warn info in".len(),
        "the reset edge plus three passes over the keywords"
    );
    assert!(series.iter().all(|bits| bits.width() == 5));
    assert!(
        series.iter().all(|bits| bits.bit(5).is_err()),
        "no bit above the width"
    );
    // The match states are never latched, which is the stop rule's whole point.
    assert!(
        series.iter().all(|bits| {
            let state = bits.to_u64().unwrap();
            !aho_corasick::is_match_state(state as usize)
        }),
        "the register never holds one of the five match states"
    );
}

proptest! {
    /// The differential, on arbitrary input.
    ///
    /// The haystack is drawn from the pattern bytes most of the time, because random
    /// bytes almost never contain a keyword and a differential that almost never finds a
    /// match is not a differential. One in four cases is drawn from the whole byte range
    /// so the no-match path is covered too.
    #[test]
    fn the_design_agrees_with_the_crate_on_arbitrary_haystacks(
        data in prop::collection::vec(
            prop_oneof![
                1 => prop::sample::select(keyword_bytes()),
                3 => any::<u8>(),
            ],
            0..40
        )
    ) {
        prop_assert_eq!(hardware(&data).matches, golden_matches(&data));
    }
}

#[test]
fn the_simulator_and_the_emitted_verilog_agree_on_this_design() {
    let Some(verilator) = verilator() else {
        println!(
            "SKIPPED: verilator not found, so the aho_corasick equivalence did not run. \
             The differential against `find_iter` above does not need it."
        );
        return;
    };
    println!("cosimulating aho_corasick with {}", verilator.display());

    let design = design();
    let plan = plan_of(&design, "aho_corasick");
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
        ["state", "accept", "pattern", "count"],
        "all four outputs are compared, including the pattern index"
    );

    // A haystack that walks the whole automaton: every keyword, the prefix pair, the
    // separators that reset to the start state, and bytes outside the alphabet.
    let mut stimulus = Stimulus::new();
    let row = |rst: u64, data: u64| {
        vec![
            Bits::constant(rst, 1).unwrap(),
            Bits::constant(data, 8).unwrap(),
        ]
    };
    stimulus.push(row(1, 0)).unwrap();
    stimulus.push(row(0, 0)).unwrap();
    let mut counter = 0u64;
    for text in [
        &b"panic"[..],
        b" error ",
        b"warn",
        b" info in ",
        b"\x00\xff",
        b"p a n i c e r r o r",
        b"in in in",
        b"war",
        b"nope",
        b"info info error warn panic in",
    ] {
        for byte in text {
            stimulus.push(row(0, u64::from(*byte))).unwrap();
            counter += 1;
        }
    }
    assert_eq!(stimulus.cycles(), 2 + counter as usize);

    let verilog = ferrite_lithic_cosim::emit(&design, &plan).unwrap();
    let harness = Harness::build(&plan, &verilog, &plan.work_dir()).unwrap();
    let report = ferrite_lithic_cosim::run_with(&design, &plan, &stimulus, &harness).unwrap();
    assert!(
        report.is_equivalent(),
        "the two backends disagree: {report}\nfirst: {:?}",
        report.first()
    );

    // And the same bytes through the testbench agree with the crate, so all three meet.
    let message: Vec<u8> = b"info info error warn panic in".to_vec();
    assert_eq!(hardware(&message).matches, golden_matches(&message));
}
