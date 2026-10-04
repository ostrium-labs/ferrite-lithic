//! Base64, checked three ways.
//!
//! The golden model is the `base64` crate's `engine::general_purpose::STANDARD`, and
//! this is the design where that matters *most*: the entire algorithm is a 64-entry
//! table, so a hand-written golden model would be the same table typed out again. Two
//! things about how the comparison is set up are worth saying:
//!
//! - The **stimulus is derived from the bytes, not from the golden output.** It would
//!   have been convenient to decode the crate's characters back into sextets with a
//!   reverse lookup built from the design's own alphabet, and that would have been a
//!   test that passes for a *wrong* alphabet: a URL-safe table decodes `+/` and `-_`
//!   to the same indices, and the encoder would hand the crate's characters straight
//!   back. So [`sextets`] packs the bytes with plain shifts, and the only thing the
//!   design is fed is a function of the input buffer.
//! - The comparison covers the **whole** output, padding included, so `=` is checked
//!   rather than excused. Stripping the `=` before comparing would leave the most
//!   interesting part of the design -- the padding countdown -- untested, and the
//!   length sweep below is built around exactly that.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use base64::Engine;
use ferrite_lithic::Design;
use ferrite_lithic_bits::Bits;
use ferrite_lithic_corpus::base64 as design;
use ferrite_lithic_cosim::{Harness, Plan, Stimulus, verilator};
use ferrite_lithic_tb::Testbench;
use proptest::prelude::*;

/// The golden encoder: RFC 4648 base64 with `=` padding.
fn golden(data: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(data)
}

/// The sextets of `data`, most significant bit first, zero-padded at the end.
///
/// RFC 4648 s4: the input is a bit stream taken most-significant-bit-first, cut into
/// six-bit groups, and a final group shorter than six bits is padded on the right with
/// zeroes. Written out of shifts rather than out of any table, so it cannot share a
/// bug with the design's ROM.
fn sextets(data: &[u8]) -> Vec<u8> {
    let mut bits: Vec<bool> = Vec::with_capacity(data.len() * 8);
    for byte in data {
        for shift in (0..8).rev() {
            bits.push((byte >> shift) & 1 == 1);
        }
    }
    // A short final chunk is padded on the *right* with zeroes, so bit `i` of the chunk
    // lands at sextet position `5 - i`. Packing it the other way -- shifting the chunk up
    // and OR-ing the bits in order -- would pad on the left and turn the last group of
    // every buffer into a different sextet, which is a quiet off-by-one that still
    // produces plausible output.
    bits.chunks(6)
        .map(|chunk| {
            chunk.iter().enumerate().fold(0u8, |acc, (index, bit)| {
                acc | (u8::from(*bit) << (5 - index))
            })
        })
        .collect()
}

/// Runs the design over `data` and returns the characters it emitted.
///
/// The testbench applies **one input port per rising edge**, so a cycle in which three
/// inputs change is three edges: the last one is the cycle the design sees, with
/// `in_valid` high and the other two already in place. Hence four edges per sextet --
/// deassert, `in_last`, `sextet`, assert -- and `the_accept_edge_is_the_one_the_design
/// _sees` is where that arrangement is checked rather than assumed.
fn hardware(data: &[u8]) -> String {
    let groups = sextets(data);
    let last = groups.len().saturating_sub(1);

    let circuit = Design::new();
    design::build(&circuit).unwrap();
    let tb = Testbench::new(&circuit).unwrap();
    tb.run(|tb| async move {
        tb.pulse(ferrite_lithic_corpus::RESET).await;
        for (index, group) in groups.iter().enumerate() {
            tb.drive("in_valid", 0).await;
            tb.drive("in_last", u64::from(index == last)).await;
            tb.drive("sextet", u64::from(*group)).await;
            tb.drive("in_valid", 1).await;
        }
        // Six edges of drain. The character for the last sextet comes out one edge
        // after it is accepted, and up to three `=` characters follow that, so the last
        // one can be three edges later still. Anything past the drain has `valid` low
        // and contributes nothing.
        tb.drive("in_valid", 0).await;
        for _ in 0..5 {
            tb.step().await;
        }
    })
    .expect("the base64 design drives only ports it declares");
    characters(&tb)
}

/// The characters the design emitted, in order, over every cycle where `valid` was
/// high.
fn characters(tb: &Testbench) -> String {
    tb.series("valid")
        .iter()
        .zip(tb.series("ascii").iter())
        .filter(|(valid, _)| valid.to_u64() == Ok(1))
        .map(|(_, bits)| {
            let code = bits.to_u64().expect("an eight-bit character fits in a u64");
            char::from(u8::try_from(code).expect("eight bits is a u8"))
        })
        .collect()
}

/// The cosimulation plan for a built design.
///
/// [`Plan::of`] is what knows how to exclude the clock from the stimulus columns, so
/// the plan is always derived rather than written out; all this does is find the clock
/// to hand [`ferrite_lithic_rtl::Module::new`], by the crate-level name every design
/// in this crate uses.
fn module_of(circuit: &Design) -> ferrite_lithic_rtl::Module {
    let clock = circuit
        .input_ports()
        .iter()
        .find(|signal| signal.name().as_deref() == Some(ferrite_lithic_corpus::CLOCK))
        .expect("the clock is a declared input")
        .id();
    ferrite_lithic_rtl::Module::new(
        "base64",
        circuit.build().unwrap(),
        clock,
        circuit.input_ports().iter().map(|s| s.id()).collect(),
        circuit.output_ports().iter().map(|s| s.id()).collect(),
    )
}

/// The plan for a built design. [`Plan::of`] is what knows how to exclude the clock
/// from the stimulus columns, so the plan is always derived rather than written out.
fn plan_of(circuit: &Design) -> Plan {
    Plan::of(circuit, &module_of(circuit)).unwrap()
}

/// The Verilog the emitter produces for a built design.
fn verilog_of(circuit: &Design) -> String {
    module_of(circuit).emit().unwrap()
}

/// How many `=` characters the design emits for a message of `n` sextets.
///
/// A fresh design per call, deliberately. The sextet counter and the countdown both
/// start at zero, so nothing has to be reset between rounds, and what is being
/// measured is the design's arithmetic rather than the order this test happens to ask
/// about it in.
fn padding_for(n: u64) -> u64 {
    let circuit = Design::new();
    design::build(&circuit).unwrap();
    let tb = Testbench::new(&circuit).unwrap();
    tb.run(|tb| async move {
        tb.pulse(ferrite_lithic_corpus::RESET).await;
        for index in 0..n {
            tb.drive("in_valid", 0).await;
            tb.drive("in_last", u64::from(index + 1 == n)).await;
            tb.drive("sextet", 0).await;
            tb.drive("in_valid", 1).await;
        }
        // Releasing `in_valid` is an edge of its own -- the testbench applies one port
        // per edge -- and it is the edge that emits the first `=`. It has to happen
        // before the counting rather than after: a bare `step` would leave `in_valid`
        // asserted, and the design would accept the stale sextet again once the
        // countdown reached zero, starting the whole message again.
        tb.drive("in_valid", 0).await;
        let mut pads = 0;
        for _ in 0..5 {
            if tb.value("valid") == 1 && tb.value("ascii") == u64::from(design::PADDING) {
                pads += 1;
            }
            tb.step().await;
        }
        pads
    })
    .expect("the base64 design drives only ports it declares")
}

#[test]
fn the_alphabet_is_the_standard_one_and_not_the_url_safe_one() {
    // The design's table is a constant, so it can be checked against the crate
    // directly rather than only through the circuit. Every one of the 64 entries is
    // checked by encoding a byte that puts exactly that sextet at the front.
    for (index, expected) in design::ALPHABET.iter().enumerate() {
        let sextet = u8::try_from(index).expect("a sextet fits in a byte");
        let encoded = golden(&[sextet << 2]);
        assert_eq!(
            encoded.as_bytes()[0],
            *expected,
            "sextet {index:#04x}: the crate says {encoded:?}"
        );
    }

    // And the shape, stated rather than implied: the standard table, which differs
    // from the URL-safe one in exactly entries 62 and 63.
    assert_eq!(design::ALPHABET.len(), 64);
    assert_eq!(design::ALPHABET[62], b'+');
    assert_eq!(design::ALPHABET[63], b'/');
    assert_eq!(
        base64::engine::general_purpose::URL_SAFE.encode([0xfb]),
        "-w==",
        "the URL-safe table differs in exactly those two entries, which is the whole \
         argument that the alphabet is a table and not structure"
    );
    assert_eq!(design::PADDING, b'=');
    assert!(
        !design::ALPHABET.contains(&design::PADDING),
        "`=` is framing, not a 65th symbol: all 64 sextets are spoken for"
    );
}

#[test]
fn the_design_agrees_with_the_base64_crate_across_the_length_range() {
    // Every length from 0 to 32 inclusive: four whole output quartets plus a bit, and
    // enough repetitions of each padding class to catch a countdown that is off by one
    // in a way that only shows up for some lengths. Buffers get a distinguishing value
    // per position so a length change cannot accidentally produce the same bytes and
    // pass.
    for length in 0..=32usize {
        let data: Vec<u8> = (0..length)
            .map(|index| (index as u8).wrapping_mul(83).wrapping_add(29))
            .collect();
        assert_eq!(hardware(&data), golden(&data), "length {length}");
    }
}

#[test]
fn every_length_class_of_padding_is_exercised_and_correct() {
    // RFC 4648's padding rule, one class at a time, against the crate. This is the
    // assertion that the module docs' claim -- `(4 - (n mod 4)) mod 4` where `n` is the
    // sextet count -- is the same function as the spec's, stated per class so a failure
    // says *which* class is wrong.
    for length in 0..=12usize {
        let data: Vec<u8> = (0..length).map(|index| index as u8).collect();
        let encoded = golden(&data);
        assert_eq!(
            encoded
                .bytes()
                .filter(|byte| *byte == design::PADDING)
                .count(),
            match length % 3 {
                0 => 0,
                1 => 2,
                _ => 1,
            },
            "length {length}: {encoded:?}"
        );
        assert_eq!(
            encoded.len(),
            4 * length.div_ceil(3),
            "and the output is a whole number of quartets"
        );
        assert_eq!(hardware(&data), encoded, "length {length}");
    }
}

#[test]
fn the_padding_count_is_the_negation_of_the_sextet_count() {
    // The design's padding rule as arithmetic rather than only through the encoding.
    // The counts are the ones that occur for real base64 messages -- 0, 2 and 3 mod 4
    // -- plus 1 mod 4, which cannot occur for a real message and which the design
    // therefore still has to *define*: 3, which is the coherent answer for a lone
    // sextet followed by three `=`.
    for (count, pads) in [(1u64, 3u64), (2, 2), (3, 1), (4, 0)] {
        assert_eq!(
            padding_for(count),
            pads,
            "{count} sextet(s): (4 - {count}) mod 4 = {pads}"
        );
    }
    // And the class counts, which is the other half of the claim: real messages have a
    // sextet count of 0, 2 or 3 mod 4, so only the middle three rows are reachable.
    for (count, expected) in [(5u64, 3u64), (6, 2), (7, 1), (8, 0)] {
        assert_eq!(padding_for(count), expected, "{count} sextet(s)");
    }
}

#[test]
fn the_design_emits_nothing_for_an_empty_buffer() {
    // `STANDARD.encode(b"")` is the empty string, not a quartet of padding, and the
    // design has no way to know it is idle unless nothing arrives -- so this checks that
    // no character is produced at all.
    assert_eq!(hardware(&[]), "");
    assert_eq!(golden(&[]), "");
    assert_eq!(sextets(&[]).len(), 0);
}

#[test]
fn a_reset_mid_stream_drops_the_queued_padding() {
    // A reset whose "the reset did something" check cannot fail is not a reset test, so
    // the reset lands *inside* a padding drain: `YQ==` needs two `=`, one has come out
    // and one is still queued, and the queued one must be dropped rather than emitted
    // after the reset.
    const MESSAGE: &[u8] = b"a";

    let circuit = Design::new();
    design::build(&circuit).unwrap();
    let tb = Testbench::new(&circuit).unwrap();
    let observed = tb
        .run(|tb| async move {
            tb.pulse(ferrite_lithic_corpus::RESET).await;
            let groups = sextets(MESSAGE);
            let mut accepted = (0, 0);
            for (index, group) in groups.iter().enumerate() {
                tb.drive("in_valid", 0).await;
                tb.drive("in_last", u64::from(index + 1 == groups.len()))
                    .await;
                tb.drive("sextet", u64::from(*group)).await;
                tb.drive("in_valid", 1).await;
                if index + 1 == groups.len() {
                    accepted = (tb.value("valid"), tb.value("ascii"));
                }
            }
            // Releasing `in_valid` is an edge of its own -- the testbench applies one
            // port per edge -- and it is the edge that emits the *first* `=`.
            tb.drive("in_valid", 0).await;
            let draining = (tb.value("valid"), tb.value("ascii"));
            // One `=` is still queued here, and reset has to empty the countdown.
            tb.drive(ferrite_lithic_corpus::RESET, 1).await;
            let reset = (tb.value("valid"), tb.value("ascii"));
            tb.drive(ferrite_lithic_corpus::RESET, 0).await;
            for _ in 0..3 {
                tb.step().await;
            }
            let after = (tb.value("valid"), tb.value("ascii"));
            (accepted, draining, reset, after)
        })
        .unwrap();

    let encoded = golden(MESSAGE);
    assert_eq!(
        encoded, "YQ==",
        "a one-byte message is two characters and two `=`"
    );
    assert_eq!(
        observed.0,
        (1, u64::from(encoded.as_bytes()[1])),
        "the character for the accepted sextet comes out one edge after the accept"
    );
    assert_eq!(
        observed.1,
        (1, u64::from(design::PADDING)),
        "and the first padding character follows it, so one of the two `=` is out"
    );
    assert_eq!(
        observed.2,
        (0, 0),
        "reset asserted: the countdown is emptied so `valid` falls, and the character \
         register is cleared so it reads zero. The queued `=` is never emitted."
    );
    assert_eq!(
        observed.3,
        (0, 0),
        "and it stays silent afterwards rather than finishing the drain late"
    );
}

#[test]
fn the_accept_edge_is_the_one_the_design_sees() {
    // The testbench applies one port per edge, so "in_valid, in_last and sextet changed
    // on the same cycle" is four edges, and only the last of them may latch. A buffer of
    // distinct bytes whose expected encoding is written out makes a latch on the wrong
    // edge produce visibly wrong characters rather than the right answer.
    let data = b"\x00\x01\x02\x03\x04\x05\x06";
    assert_eq!(
        hardware(data),
        golden(data),
        "a latch on any edge but the last would encode a stale sextet"
    );
    assert_eq!(
        golden(data),
        "AAECAwQFBg==",
        "and the buffer is chosen so its encoding is checkable by eye: two sextets and \
         two padding characters, which is the length-mod-3-is-1 case"
    );
}

proptest! {
    /// The differential, on arbitrary data.
    ///
    /// Length is bounded because the design is simulated four edges per sextet and this
    /// is the slow half of the test; the length sweep above covers every padding class
    /// and this covers the values.
    #[test]
    fn the_design_agrees_with_the_base64_crate_on_arbitrary_buffers(
        data in prop::collection::vec(any::<u8>(), 0..48)
    ) {
        prop_assert_eq!(hardware(&data), golden(&data));
    }
}

/// The stimulus both backends are driven from: reset, then every sextet back to back
/// with no gaps, then enough drain for the longest possible padding run.
///
/// Unlike the testbench, the cosimulator takes a whole cycle at once, so `in_valid` can
/// be held high and the design accepts a sextet every cycle -- which is also a better
/// exercise of the accept path than the paced testbench run.
fn stimulus(message: &[u8]) -> Stimulus {
    let groups = sextets(message);
    let last = groups.len().saturating_sub(1);
    let mut stimulus = Stimulus::new();
    let row = |rst: u64, in_valid: u64, in_last: u64, sextet: u64| {
        vec![
            Bits::constant(rst, 1).unwrap(),
            Bits::constant(in_valid, 1).unwrap(),
            Bits::constant(in_last, 1).unwrap(),
            Bits::constant(sextet, 6).unwrap(),
        ]
    };
    stimulus.push(row(1, 0, 0, 0)).unwrap();
    stimulus.push(row(0, 0, 0, 0)).unwrap();
    for (index, group) in groups.iter().enumerate() {
        stimulus
            .push(row(0, 1, u64::from(index == last), u64::from(*group)))
            .unwrap();
    }
    for _ in 0..5 {
        stimulus.push(row(0, 0, 0, 0)).unwrap();
    }
    stimulus
}

#[test]
fn the_simulator_and_the_emitted_verilog_agree_on_this_design() {
    let Some(verilator) = verilator() else {
        println!(
            "SKIPPED: verilator not found, so the base64 equivalence did not run. The \
             differential against the `base64` crate above does not need it."
        );
        return;
    };
    println!("cosimulating base64 with {}", verilator.display());

    let circuit = Design::new();
    design::build(&circuit).unwrap();
    let plan = plan_of(&circuit);
    assert_eq!(
        plan.inputs
            .iter()
            .map(|p| p.name.as_str())
            .collect::<Vec<_>>(),
        ["rst", "in_valid", "in_last", "sextet"],
        "the clock is a port but not a stimulus column"
    );
    assert_eq!(
        plan.inputs
            .iter()
            .map(|p| p.verilog.as_str())
            .collect::<Vec<_>>(),
        ["rst", "in_valid", "in_last", "sextet"],
        "and none of these is a Verilog keyword, so the design names and the emitted \
         names agree"
    );
    assert_eq!(
        plan.outputs
            .iter()
            .map(|p| p.name.as_str())
            .collect::<Vec<_>>(),
        ["valid", "ascii"],
        "both outputs are compared, including the one that says the other is junk"
    );

    // The pangram is 43 bytes, which is 58 sextets -- more than 64 would be needed to
    // visit every alphabet entry, so the compared window covers the whole table, and
    // 43 mod 3 is 1 so the stimulus ends in a padding run.
    let message = b"the quick brown fox jumps over the lazy dog";
    let groups = sextets(message);
    assert_eq!(groups.len(), 58, "ceil(43 * 8 / 6)");
    let stimulus = stimulus(message);
    assert_eq!(stimulus.cycles(), 2 + groups.len() + 5);

    let verilog = ferrite_lithic_cosim::emit(&circuit, &plan).unwrap();
    let harness = Harness::build(&plan, &verilog, &plan.work_dir()).unwrap();
    let report = ferrite_lithic_cosim::run_with(&circuit, &plan, &stimulus, &harness).unwrap();
    assert!(
        report.is_equivalent(),
        "the two backends disagree: {report}\nfirst: {:?}",
        report.first()
    );

    // And the same run through the testbench agrees with the crate, so all three of
    // them -- simulator, Verilator and the original crate -- meet.
    assert_eq!(hardware(message), golden(message));
}

#[test]
fn the_output_bus_never_stalls_mid_message() {
    // Base64 is rate-balanced: a sextet in, a character out, every cycle. Fed at one
    // sextet per cycle the design emits one character per cycle, and the only cycles
    // where `valid` falls are the reset at the front and the drain at the end. This is
    // the one thing the paced testbench cannot see, because it cannot present
    // `in_valid` and a sextet on the same cycle.
    let message = b"the quick brown fox jumps over the lazy dog";

    let circuit = Design::new();
    design::build(&circuit).unwrap();
    let plan = plan_of(&circuit);
    let cycles = ferrite_lithic_cosim::simulate(&circuit, &plan, &stimulus(message)).unwrap();
    let valid_at = plan
        .output_names()
        .iter()
        .position(|n| n == "valid")
        .unwrap();
    let ascii_at = plan
        .output_names()
        .iter()
        .position(|n| n == "ascii")
        .unwrap();
    let valid: Vec<u64> = cycles
        .iter()
        .map(|outputs| outputs[valid_at].to_u64().unwrap())
        .collect();

    // The encoding of the whole message is known, so the number of characters is
    // known: one per sextet plus the padding run.
    let expected = golden(message);
    assert_eq!(
        valid.iter().filter(|v| **v == 1).count(),
        expected.len(),
        "one character per sextet plus {expected:?}"
    );

    let first = valid
        .iter()
        .position(|v| *v == 1)
        .expect("some cycles are valid");
    let last = valid
        .iter()
        .rposition(|v| *v == 1)
        .expect("some cycles are valid");
    assert_eq!(
        valid[first..=last].iter().filter(|v| **v == 0).count(),
        0,
        "no stall between the first and last character of a {}-character message",
        expected.len()
    );

    // And the characters, for completeness: the cosimulator's simulator is the same one
    // the testbench drives, so this also says the fast path and the paced path agree.
    let emitted: String = valid
        .iter()
        .zip(
            cycles
                .iter()
                .map(|outputs| outputs[ascii_at].to_u64().unwrap()),
        )
        .filter(|(valid, _)| **valid == 1)
        .map(|(_, code)| char::from(u8::try_from(code).unwrap()))
        .collect();
    assert_eq!(emitted, expected);
}

#[test]
fn the_emitted_verilog_is_a_sixty_four_entry_table() {
    // The design's claim about its own shape, checked against the emitted text rather
    // than against the graph: a `case_` becomes an `always @*` block with one arm per
    // alphabet entry, so counting the arms counts the ROM. The address is six bits
    // here, which is what separates this table from `hex`'s sixteen-entry one.
    let circuit = Design::new();
    design::build(&circuit).unwrap();
    let verilog = verilog_of(&circuit);
    let arms = verilog
        .lines()
        .filter(|line| {
            let line = line.trim();
            line.starts_with("6'h") && line.contains("= 8'h")
        })
        .count();
    assert_eq!(arms, 64, "one arm per sextet in\n{verilog}");
    assert!(
        verilog.contains("always @* begin"),
        "and a case is an `always @*` block, because that is what a ROM is"
    );
    assert!(
        verilog.contains("8'h3d"),
        "and the padding character is in there too, as a literal"
    );
}

#[test]
fn the_emitted_ports_are_the_ones_the_testbench_drives() {
    // The testbench and the cosimulator address ports by name, so a rename that only
    // lands in one of them turns every differential above into a test that fails for a
    // reason that has nothing to do with the alphabet.
    let circuit = Design::new();
    design::build(&circuit).unwrap();
    let inputs: Vec<String> = circuit
        .input_ports()
        .iter()
        .filter_map(|signal| signal.name())
        .collect();
    let outputs: Vec<String> = circuit
        .output_ports()
        .iter()
        .filter_map(|signal| signal.name())
        .collect();
    assert_eq!(inputs, ["clk", "rst", "in_valid", "in_last", "sextet"]);
    assert_eq!(outputs, ["valid", "ascii"]);
}
