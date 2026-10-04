//! Lowercase hex, checked three ways.
//!
//! The golden model is the `hex` crate's `hex::encode`, not a second encoder written
//! by the same hand that wrote the design. That matters more here than it does for
//! [`crc3`]: this design's *only* content is the sixteen-entry digit table, so a
//! hand-written golden model would be the same table typed out a second time, and a
//! typo in one copy would hide in the other. Everything below is therefore compared
//! against the crate.
//!
//! The design's table is separately checked against the crate's output in
//! `the_alphabet_is_the_one_the_hex_crate_uses`, so the design and the golden model
//! cannot agree on a wrong table by both being derived from the same wrong source.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use ferrite_lithic::Design;
use ferrite_lithic_bits::Bits;
use ferrite_lithic_corpus::hex;
// The design module is called `hex` and so is the golden crate, so the crate is
// brought in under a name that cannot be confused with the module.
use ::hex as hex_crate;
use ferrite_lithic_cosim::{Harness, Plan, Stimulus, verilator};
use ferrite_lithic_tb::Testbench;
use proptest::prelude::*;

/// The design's output character, collected from every cycle where `valid` is high.
///
/// The testbench applies **one input port per rising edge**, so a cycle in which two
/// inputs change is two edges: the second one is the cycle the design sees, and the
/// first leaves `in_valid` low while the other ports are put in place. That is why
/// every byte below takes three edges -- deassert, present the data, assert -- and why
/// the assertion about *which* edge matters is in `the_accept_edge_is_the_one_the
/// design_sees`.
/// The cosimulation plan for a built design.
///
/// [`Plan::of`] is what knows how to exclude the clock from the stimulus columns, so
/// the plan is always derived rather than written out; all this does is find the clock
/// to hand [`ferrite_lithic_rtl::Module::new`], by the crate-level name every design
/// in this crate uses.
fn module_of(design: &Design) -> ferrite_lithic_rtl::Module {
    let clock = design
        .input_ports()
        .iter()
        .find(|signal| signal.name().as_deref() == Some(ferrite_lithic_corpus::CLOCK))
        .expect("the clock is a declared input")
        .id();
    ferrite_lithic_rtl::Module::new(
        "hex",
        design.build().unwrap(),
        clock,
        design.input_ports().iter().map(|s| s.id()).collect(),
        design.output_ports().iter().map(|s| s.id()).collect(),
    )
}

/// The plan for a built design. [`Plan::of`] is what knows how to exclude the clock
/// from the stimulus columns, so the plan is always derived rather than written out.
fn plan_of(design: &Design) -> Plan {
    Plan::of(design, &module_of(design)).unwrap()
}

/// The Verilog the emitter produces for a built design.
fn verilog_of(design: &Design) -> String {
    module_of(design).emit().unwrap()
}

/// Reset, then a byte on *every* cycle with `in_valid` held high, then a short drain.
///
/// Unlike the step testbench, the cosimulator takes a whole cycle at once, so this can
/// feed the design at its maximum input rate -- which is the only way to see the output
/// bus saturate, and a much better exercise of the phase machine than a paced run.
fn saturated_stimulus(data: &[u8]) -> Stimulus {
    let mut stimulus = Stimulus::new();
    let row = |rst: u64, in_valid: u64, data: u64| {
        vec![
            Bits::constant(rst, 1).unwrap(),
            Bits::constant(in_valid, 1).unwrap(),
            Bits::constant(data, 8).unwrap(),
        ]
    };
    stimulus.push(row(1, 0, 0)).unwrap();
    stimulus.push(row(0, 0, 0)).unwrap();
    for byte in data {
        stimulus.push(row(0, 1, u64::from(*byte))).unwrap();
    }
    for _ in 0..4 {
        stimulus.push(row(0, 0, 0)).unwrap();
    }
    stimulus
}

fn hardware(data: &[u8]) -> String {
    let design = Design::new();
    hex::build(&design).unwrap();
    let tb = Testbench::new(&design).unwrap();
    tb.run(|tb| async move {
        tb.pulse(ferrite_lithic_corpus::RESET).await;
        for byte in data {
            tb.drive("in_valid", 0).await;
            tb.drive("data", u64::from(*byte)).await;
            tb.drive("in_valid", 1).await;
        }
        // The last byte's two characters are still in the pipe: the low nibble comes
        // out one edge after the high one. Two further edges drain it, and any cycle
        // past the drain has `valid` low so it contributes nothing.
        tb.drive("in_valid", 0).await;
        tb.step().await;
        tb.step().await;
    })
    .expect("the hex design drives only ports it declares");
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

#[test]
fn the_alphabet_is_the_one_the_hex_crate_uses() {
    // The design's table is a constant, so it can be checked against the crate
    // directly rather than only through the circuit: for each nibble value, the first
    // character of `hex::encode` of a byte with that nibble in the high position.
    for (value, expected) in hex::ALPHABET.iter().enumerate() {
        let probe = u8::try_from(value << 4).expect("a nibble shifted left fits in a byte");
        assert_eq!(
            hex_crate::encode([probe]).chars().next(),
            Some(char::from(*expected)),
            "nibble {value:#x}"
        );
    }

    // And the shape of it, stated rather than implied: sixteen digits, the first ten
    // ASCII '0'-'9' with nothing skipped, the last six ASCII 'a'-'f'. This is the
    // lowercase form; `hex::encode_upper` would be 'A'-'F', and the difference is
    // ASCII bit 5.
    assert_eq!(hex::ALPHABET.len(), 16);
    assert!(hex::ALPHABET[..10].iter().all(u8::is_ascii_digit));
    assert!(hex::ALPHABET[10..].iter().all(u8::is_ascii_lowercase));
    assert!(
        hex::ALPHABET[10..]
            .iter()
            .all(|c| (c - b'a') <= (b'f' - b'a')),
        "and the letters are the six of them, in order, with no gap"
    );
    assert_eq!(
        hex_crate::encode_upper([0xab]),
        "AB",
        "the uppercase form differs only in case, which is why the design is one OR"
    );
    assert_eq!(hex_crate::encode([0xab]), "ab");
}

#[test]
fn the_design_agrees_with_the_hex_crate_on_every_byte_value() {
    // Exhaustive over the whole 256-value input space, which is the cheapest possible
    // way to cover the sixteen-entry ROM twice over (every nibble as high and as low)
    // and also covers the boundary between the two: a design that always emitted the
    // high nibble first, or swapped them, fails on most bytes rather than on one.
    for value in 0..=255u16 {
        let byte = u8::try_from(value).expect("the loop bounds a u8");
        assert_eq!(
            hardware(&[byte]),
            hex_crate::encode([byte]),
            "byte {value:#04x}"
        );
    }
}

#[test]
fn the_design_agrees_with_the_hex_crate_on_the_empty_buffer() {
    // No bytes, no characters: the design must not emit a character it was not given,
    // which is what `valid` is for.
    assert_eq!(hardware(&[]), "");
    assert_eq!(hex_crate::encode([]), "");
}

#[test]
fn the_design_agrees_with_the_hex_crate_on_buffers_that_straddle_word_boundaries() {
    // Lengths 0..=40. The design has no word boundary of its own -- it works a byte at
    // a time -- but a design that had been built a word at a time would, and 8, 16 and
    // 32 are exactly where that shows up. Buffers also get a distinguishing value per
    // position so a length change cannot accidentally produce the same bytes.
    for length in 0..=40usize {
        let data: Vec<u8> = (0..length)
            .map(|index| (index as u8).wrapping_mul(61).wrapping_add(7))
            .collect();
        assert_eq!(hardware(&data), hex_crate::encode(&data), "length {length}");
    }
}

#[test]
fn every_byte_produces_exactly_two_characters() {
    // The 8:4 split stated as an assertion rather than as prose: two characters in,
    // two characters out, for a buffer of any length. A design that emitted one
    // character per byte would be the bug this catches, and it would pass a
    // `starts_with` comparison on a single byte.
    for length in 0..=16usize {
        let data: Vec<u8> = (0..length).map(|index| index as u8).collect();
        assert_eq!(
            hardware(&data).len(),
            length * 2,
            "length {length}: two characters per byte"
        );
    }
}

#[test]
fn the_output_bus_saturates_when_the_input_is_fed_at_its_maximum_rate() {
    // The rate claim in the module docs, as a measurement rather than as prose: fed a
    // byte on *every* cycle, the design accepts one every two cycles and emits a
    // character on every cycle, so the output bus runs at 4 bits per cycle and the
    // pipeline never stalls.
    //
    // This cannot be measured through the step testbench, which applies one input port
    // per rising edge and so cannot present `in_valid` and a new `data` on the same
    // cycle. `ferrite_lithic_cosim::simulate` takes whole cycles -- it is the other
    // backend's view of the same design -- so the rate claim is checked there, and the
    // characters it produces are still compared against the crate.
    const DATA: &[u8] = b"the quick brown fox jumps over the lazy dog";

    let design = Design::new();
    hex::build(&design).unwrap();
    let plan = plan_of(&design);
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

    let stimulus = saturated_stimulus(DATA);
    assert_eq!(stimulus.cycles(), 2 + DATA.len() + 4);
    let cycles = ferrite_lithic_cosim::simulate(&design, &plan, &stimulus).unwrap();
    let valid: Vec<u64> = cycles
        .iter()
        .map(|outputs| outputs[valid_at].to_u64().unwrap())
        .collect();
    let character: Vec<u64> = cycles
        .iter()
        .map(|outputs| outputs[ascii_at].to_u64().unwrap())
        .collect();

    // A byte offered on every cycle is accepted on every *other* one, because the
    // design takes a byte on the idle cycle and again on the low-nibble cycle. So the
    // characters it emits are those of every second byte -- a stronger statement than
    // "it encodes what it was given", because it pins down which cycles the design
    // claims a byte on.
    let accepted: Vec<u8> = DATA.iter().copied().step_by(2).collect();
    assert_eq!(accepted.len(), DATA.len().div_ceil(2));
    let emitted: String = valid
        .iter()
        .zip(character.iter())
        .filter(|(valid, _)| **valid == 1)
        .map(|(_, code)| char::from(u8::try_from(*code).unwrap()))
        .collect();
    assert_eq!(emitted, hex_crate::encode(&accepted), "fed every cycle");

    // And the rate claim: two characters per accepted byte with no cycle of `valid` low
    // in between. A design that needed three cycles per byte would emit the right
    // characters and fail here, which is the one thing this test sees that the
    // differential cannot.
    let first = valid
        .iter()
        .position(|v| *v == 1)
        .expect("some cycles are valid");
    let last = valid
        .iter()
        .rposition(|v| *v == 1)
        .expect("some cycles are valid");
    assert_eq!(
        last - first + 1,
        accepted.len() * 2,
        "two characters per byte"
    );
    assert_eq!(
        valid[first..=last].iter().filter(|v| **v == 0).count(),
        0,
        "the output bus never stalls between the first and last character"
    );
}

#[test]
fn a_reset_mid_stream_stops_the_characters_and_restarts_the_byte() {
    // A reset whose "the reset did something" check cannot fail is not a reset test,
    // so the mid-stream reset is asserted *between* two characters of one byte: the
    // design must drop the second nibble rather than emit it late.
    const MESSAGE: &[u8] = b"abc";

    let design = Design::new();
    hex::build(&design).unwrap();
    let tb = Testbench::new(&design).unwrap();
    let observed = tb
        .run(|tb| async move {
            tb.pulse(ferrite_lithic_corpus::RESET).await;
            tb.drive("in_valid", 0).await;
            tb.drive("data", u64::from(MESSAGE[0])).await;
            tb.drive("in_valid", 1).await;
            let high = (tb.value("valid"), tb.value("ascii"));
            // Reset asserted on its own edge. The phase goes back to idle, so this
            // cycle emits nothing and the low nibble of the byte already accepted is
            // dropped.
            tb.drive(ferrite_lithic_corpus::RESET, 1).await;
            let reset = (tb.value("valid"), tb.value("ascii"));
            // And the design works again afterwards, on a fresh byte.
            tb.drive(ferrite_lithic_corpus::RESET, 0).await;
            tb.drive("in_valid", 0).await;
            tb.drive("data", u64::from(MESSAGE[1])).await;
            tb.drive("in_valid", 1).await;
            let restarted = (tb.value("valid"), tb.value("ascii"));
            // One edge takes the phase to the low nibble, so this is where the second
            // character of the *new* byte is. A second edge would take it to idle and
            // the character would already be junk.
            tb.step().await;
            let restarted_low = (tb.value("valid"), tb.value("ascii"));
            (high, reset, restarted, restarted_low)
        })
        .unwrap();

    let first = hex_crate::encode([MESSAGE[0]]);
    let second = hex_crate::encode([MESSAGE[1]]);
    assert_eq!(
        observed.0,
        (1, u64::from(first.as_bytes()[0])),
        "the high nibble comes out the cycle after the accept edge"
    );
    assert_eq!(
        observed.1,
        (0, u64::from(hex::ALPHABET[0])),
        "reset asserted: the phase returns to idle so `valid` falls, and the byte \
         register returns to zero so the character is the table's entry for zero. \
         `valid` is the only thing that says it is meaningless"
    );
    assert_eq!(
        observed.2,
        (1, u64::from(second.as_bytes()[0])),
        "and the design restarts on the next byte"
    );
    assert_eq!(
        observed.3,
        (1, u64::from(second.as_bytes()[1])),
        "with its low nibble following, so the reset cost exactly one character"
    );
}

#[test]
fn the_accept_edge_is_the_one_the_design_sees() {
    // The testbench applies one port per edge, so "in_valid and data changed on the
    // same cycle" is three edges. This asserts that only the third one latches: if the
    // design latched on the *first* edge instead it would be latching a byte whose
    // data had not been presented yet, and the emitted string would be built from the
    // previous byte's value. Driving a known-distinct byte per position and checking
    // every character catches exactly that.
    let data = b"\x00\x11\x22\x33\x44\x55\x66\x77";
    assert_eq!(hardware(data), hex_crate::encode(data));
    assert_eq!(
        hex_crate::encode(data),
        "0011223344556677",
        "and the buffer really is eight distinct bytes, so a shifted or stale latch \
         cannot produce the right answer by accident"
    );
}

proptest! {
    /// The differential, on arbitrary data.
    ///
    /// Length is bounded because the design is simulated several edges per byte and
    /// this is the slow half of the test; the buffer sweep above covers the length
    /// boundaries and this covers the values.
    #[test]
    fn the_design_agrees_with_the_hex_crate_on_arbitrary_buffers(
        data in prop::collection::vec(any::<u8>(), 0..48)
    ) {
        prop_assert_eq!(hardware(&data), hex_crate::encode(&data));
    }
}

#[test]
fn the_simulator_and_the_emitted_verilog_agree_on_this_design() {
    let Some(verilator) = verilator() else {
        println!(
            "SKIPPED: verilator not found, so the hex equivalence did not run. The \
             differential against the `hex` crate above does not need it."
        );
        return;
    };
    println!("cosimulating hex with {}", verilator.display());

    let design = Design::new();
    hex::build(&design).unwrap();
    let plan = plan_of(&design);
    assert_eq!(
        plan.inputs
            .iter()
            .map(|p| p.name.as_str())
            .collect::<Vec<_>>(),
        ["rst", "in_valid", "data"],
        "the clock is a port but not a stimulus column"
    );
    assert_eq!(
        plan.outputs
            .iter()
            .map(|p| p.name.as_str())
            .collect::<Vec<_>>(),
        ["valid", "ascii"],
        "both outputs are compared, including the one that says the other is junk"
    );
    assert_eq!(
        plan.inputs
            .iter()
            .map(|p| p.verilog.as_str())
            .collect::<Vec<_>>(),
        ["rst", "in_valid", "data"],
        "none of this design's port names is a Verilog keyword, so the design names \
         and the emitted names agree; `data` and `valid` are ordinary identifiers"
    );

    // A byte on every cycle with `in_valid` held high, which the step testbench cannot
    // arrange: the design accepts one every two cycles on its own, and the compared
    // window therefore covers the saturated output bus rather than the paced one.
    let stimulus = saturated_stimulus(b"the quick brown fox jumps over the lazy dog");
    assert_eq!(stimulus.cycles(), 2 + 43 + 4);

    let verilog = ferrite_lithic_cosim::emit(&design, &plan).unwrap();
    let harness = Harness::build(&plan, &verilog, &plan.work_dir()).unwrap();
    let report = ferrite_lithic_cosim::run_with(&design, &plan, &stimulus, &harness).unwrap();
    assert!(
        report.is_equivalent(),
        "the two backends disagree: {report}\nfirst: {:?}",
        report.first()
    );

    // And the same run through the testbench agrees with the crate, so all three of
    // them -- simulator, Verilator and the original crate -- meet.
    assert_eq!(
        hardware(b"the quick brown fox jumps over the lazy dog"),
        hex_crate::encode(b"the quick brown fox jumps over the lazy dog")
    );
}

#[test]
fn the_emitted_verilog_is_a_sixteen_entry_table() {
    // The design's claim about its own shape, checked against the emitted text rather
    // than against the graph: a `case_` becomes an `always @*` block with one arm per
    // alphabet entry, so counting the arms counts the ROM.
    let design = Design::new();
    hex::build(&design).unwrap();
    let verilog = verilog_of(&design);
    let arms = verilog
        .lines()
        .filter(|line| {
            let line = line.trim();
            line.starts_with("4'h") && line.contains("= 8'h")
        })
        .count();
    assert_eq!(arms, 16, "one arm per nibble in\\n{verilog}");
    assert!(
        verilog.contains("always @* begin"),
        "and a case is an `always @*` block, because that is what a ROM is"
    );
}

#[test]
fn the_emitted_ports_are_the_ones_the_testbench_drives() {
    // The testbench and the cosimulator address ports by name, so a rename that only
    // lands in one of them turns every differential above into a test that fails for
    // a reason that has nothing to do with the hex digit table.
    let design = Design::new();
    hex::build(&design).unwrap();
    let inputs: Vec<String> = design
        .input_ports()
        .iter()
        .filter_map(|signal| signal.name())
        .collect();
    let outputs: Vec<String> = design
        .output_ports()
        .iter()
        .filter_map(|signal| signal.name())
        .collect();
    assert_eq!(inputs, ["clk", "rst", "in_valid", "data"]);
    assert_eq!(outputs, ["valid", "ascii"]);
}
