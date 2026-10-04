//! CRC-3/ROTD, checked three ways.
//!
//! The golden model is the `crc` crate, not a second CRC written by the same hand
//! that wrote the design. A hand-written golden model can be wrong in exactly the
//! same way as the design it is checking, and the wrong version of this design is
//! *very* plausible: a three-bit LFSR with the tap reversed still produces three
//! bits and still looks like a CRC. So the parameters here come from the RevEng
//! catalogue, transcribed into the `crc` crate's own `Algorithm` type, and the
//! transcription is itself checked against the catalogue's published check value
//! before any of it is trusted.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use crc::{Algorithm, Crc};
use ferrite_lithic::Design;
use ferrite_lithic_bits::Bits;
use ferrite_lithic_corpus::crc3;
use ferrite_lithic_cosim::{Harness, Plan, Stimulus, verilator};
use ferrite_lithic_tb::Testbench;
use proptest::prelude::*;

/// CRC-3/ROTD, transcribed from the RevEng catalogue.
///
/// `check` is the catalogue's own value for the register after `"123456789"`, and
/// it is asserted below against the `crc` crate's computation. If the transcription
/// were wrong, that assertion would fail here rather than quietly agreeing with a
/// wrong design.
static ROTD: Algorithm<u8> = Algorithm {
    width: 3,
    poly: 0x3,
    init: 0x0,
    refin: true,
    refout: true,
    xorout: 0x0,
    check: 0x2,
    residue: 0x0,
};

fn golden() -> Crc<u8> {
    Crc::<u8>::new(&ROTD)
}

/// Runs the design over `data`, one byte per cycle, and returns the CRC.
fn hardware(data: &[u8]) -> u64 {
    let design = Design::new();
    crc3::build(&design).unwrap();
    let tb = Testbench::new(&design).unwrap();
    tb.run(|tb| async move {
        tb.pulse(ferrite_lithic_corpus::RESET).await;
        for byte in data {
            tb.drive("byte", *byte as u64).await;
        }
        tb.value("crc")
    })
    .expect("the crc3 design drives only ports it declares")
}

#[test]
fn the_transcribed_parameters_reproduce_the_catalogues_check_value() {
    // Without this, every differential test below could be comparing two things
    // that are both wrong in the same way.
    assert_eq!(
        golden().checksum(b"123456789"),
        ROTD.check,
        "the transcribed CRC-3/ROTD parameters disagree with their own check value"
    );
}

#[test]
fn the_design_agrees_with_the_crc_crate_on_the_catalogue_check_string() {
    assert_eq!(hardware(b"123456789"), u64::from(ROTD.check));
}

#[test]
fn the_design_agrees_with_the_crc_crate_across_the_byte_range() {
    // Every single-byte message, which covers all 256 values of the input and is
    // the cheapest way to find a bit-order or tap mistake: a reversed tap differs
    // on most of them rather than on one.
    for value in 0..=255u64 {
        let data = [value as u8];
        assert_eq!(
            hardware(&data),
            u64::from(golden().checksum(&data)),
            "one byte {value:#04x}"
        );
    }
}

#[test]
fn the_design_agrees_with_the_crc_crate_on_the_empty_message() {
    assert_eq!(hardware(&[]), u64::from(golden().checksum(b"")));
    assert_eq!(
        golden().checksum(b""),
        0,
        "init zero, xorout zero, empty input"
    );
}

#[test]
fn the_design_agrees_with_the_crc_crate_on_buffers_that_straddle_word_boundaries() {
    // Lengths 0..=40: the LFSR has no word boundary, but a design that had been
    // built a word at a time would. Buffers are also given a distinguishing prefix
    // so a length change cannot accidentally produce the same bytes.
    for length in 0..=40usize {
        let data: Vec<u8> = (0..length)
            .map(|index| (index as u8).wrapping_mul(37).wrapping_add(11))
            .collect();
        assert_eq!(
            hardware(&data),
            u64::from(golden().checksum(&data)),
            "length {length}"
        );
    }
}

#[test]
fn a_reset_mid_stream_returns_the_crc_to_its_init_value() {
    // Several short messages have a CRC-3 of zero by chance -- there are only eight
    // possible results -- so the message is chosen for having a nonzero one and then
    // asserted to have it. A reset test whose "the reset did something" check cannot
    // fail is not a reset test.
    const MESSAGE: &[u8] = b"abc";
    let nonzero = golden().checksum(MESSAGE);
    assert_ne!(
        nonzero, 0,
        "the message needs a nonzero CRC for this to mean anything"
    );

    let design = Design::new();
    crc3::build(&design).unwrap();
    let tb = Testbench::new(&design).unwrap();
    let observed = tb
        .run(|tb| async move {
            tb.pulse(ferrite_lithic_corpus::RESET).await;
            for byte in MESSAGE {
                tb.drive("byte", *byte as u64).await;
            }
            let streamed = tb.value("crc");
            // One edge with reset asserted. The register takes the CRC's init value
            // on this edge, so the read is after it -- the same boundary every other
            // read in this file uses.
            tb.drive(ferrite_lithic_corpus::RESET, 1).await;
            let reset = tb.value("crc");
            // Releasing reset is another edge, and the design consumes a byte on
            // every edge, so the byte input is still the last byte of the message and
            // this is the CRC of the message plus one more byte. Asserting that
            // rather than leaving it out is the difference between testing the
            // release path and pretending it was not exercised.
            tb.drive(ferrite_lithic_corpus::RESET, 0).await;
            let released = tb.value("crc");
            (streamed, reset, released)
        })
        .unwrap();

    assert_eq!(observed.0, u64::from(nonzero));
    assert_eq!(
        observed.1, 0,
        "reset asserted returns the register to its init value"
    );

    // From zero, not from where the message left it: the reset edge discarded the
    // byte that was on the input and zeroed the register, so the release edge is the
    // *first* byte of a new message.
    let last = *MESSAGE.last().expect("the message is not empty");
    assert_eq!(
        observed.2,
        u64::from(golden().checksum(&[last])),
        "and releasing reset consumes the byte still on the input, from zero"
    );
}

#[test]
fn every_byte_consumed_changes_the_crc_or_is_a_fixed_point() {
    // A counter that never changes is a broken design, and so is one that changes
    // on every single byte regardless of value. Both show up as a constant or an
    // always-different series.
    let design = Design::new();
    crc3::build(&design).unwrap();
    let tb = Testbench::new(&design).unwrap();
    tb.run(|tb| async move {
        tb.pulse(ferrite_lithic_corpus::RESET).await;
        for byte in 0..64u64 {
            tb.drive("byte", byte).await;
        }
    })
    .unwrap();
    let series = tb.series("crc");
    assert_eq!(
        series.len(),
        66,
        "the pulse is two cycles and 64 bytes follow"
    );
    let crc: Vec<u64> = series.iter().map(|bits| bits.to_u64().unwrap()).collect();
    assert!(
        crc.iter().all(|value| *value < 8),
        "three bits stay three bits"
    );
    assert!(
        crc.windows(2).any(|pair| pair[0] != pair[1]),
        "the register never changed across 64 bytes"
    );
    assert!(
        crc.windows(2).any(|pair| pair[0] == pair[1]),
        "the register changed on every single byte, which a CRC should not do"
    );
}

proptest! {
    /// The differential, on arbitrary data.
    ///
    /// Length is bounded because the design is simulated a cycle per byte and this
    /// is the slow half of the test; the buffer shapes above cover the boundaries
    /// that a length sweep would, and this covers the values.
    #[test]
    fn the_design_agrees_with_the_crc_crate_on_arbitrary_buffers(data in prop::collection::vec(any::<u8>(), 0..64)) {
        prop_assert_eq!(
            hardware(&data),
            u64::from(golden().checksum(&data)),
        );
    }
}

#[test]
fn the_simulator_and_the_emitted_verilog_agree_on_this_design() {
    let Some(verilator) = verilator() else {
        println!(
            "SKIPPED: verilator not found, so the crc3 equivalence did not run. The \
             differential against the `crc` crate above does not need it."
        );
        return;
    };
    println!("cosimulating crc3 with {}", verilator.display());

    let design = Design::new();
    let ports = crc3::build(&design).unwrap();

    let module = ferrite_lithic_rtl::Module::new(
        "crc3",
        design.build().unwrap(),
        ports.inputs.clk.id(),
        design.input_ports().iter().map(|s| s.id()).collect(),
        design.output_ports().iter().map(|s| s.id()).collect(),
    );
    let plan = Plan::of(&design, &module).unwrap();
    assert_eq!(
        plan.inputs
            .iter()
            .map(|p| p.name.as_str())
            .collect::<Vec<_>>(),
        ["rst", "byte"],
        "the clock is a port but not a stimulus column, and the simulator gets the \
         design's own name"
    );
    assert_eq!(
        plan.inputs
            .iter()
            .map(|p| p.verilog.as_str())
            .collect::<Vec<_>>(),
        ["rst", "byte_"],
        "while the driver gets the name the Verilog declares"
    );

    // A stimulus that resets, then feeds a message, so the register is exercised
    // rather than sitting at its initial value.
    let mut stimulus = Stimulus::new();
    stimulus
        .push(vec![Bits::constant(1, 1).unwrap(), Bits::zeros(8).unwrap()])
        .unwrap();
    stimulus
        .push(vec![Bits::constant(0, 1).unwrap(), Bits::zeros(8).unwrap()])
        .unwrap();
    for byte in b"the quick brown fox jumps over the lazy dog" {
        stimulus
            .push(vec![
                Bits::zeros(1).unwrap(),
                Bits::constant(*byte as u64, 8).unwrap(),
            ])
            .unwrap();
    }
    assert_eq!(stimulus.cycles(), 45);

    let verilog = ferrite_lithic_cosim::emit(&design, &plan).unwrap();
    let harness = Harness::build(&plan, &verilog, &plan.work_dir()).unwrap();
    let report = ferrite_lithic_cosim::run_with(&design, &plan, &stimulus, &harness).unwrap();
    assert!(
        report.is_equivalent(),
        "the two backends disagree: {report}\nfirst: {:?}",
        report.first()
    );

    // And the same run through the testbench agrees with the software CRC, so all
    // three of them -- simulator, Verilator and the original crate -- meet.
    let message = b"the quick brown fox jumps over the lazy dog";
    assert_eq!(u64::from(golden().checksum(message)), hardware(message));
}

#[test]
fn a_port_named_after_a_keyword_is_escaped_consistently_everywhere() {
    // The design's input is called `byte`, which is a SystemVerilog keyword. The
    // emitter escapes it to `byte_`; if the cosimulator's plan used the raw name it
    // would generate a driver referring to a member the Verilator class does not
    // have, and the error would point at the emitter rather than at the plan. That
    // happened once already: `byte` was missing from the emitter's reserved-word
    // list for the whole of A1 and A2.
    let design = Design::new();
    let ports = crc3::build(&design).unwrap();
    let module = ferrite_lithic_rtl::Module::new(
        "crc3",
        design.build().unwrap(),
        ports.inputs.clk.id(),
        design.input_ports().iter().map(|s| s.id()).collect(),
        design.output_ports().iter().map(|s| s.id()).collect(),
    );
    let verilog = module.emit().unwrap();
    assert!(
        verilog.contains("input  wire [7:0] byte_"),
        "the emitter escapes the keyword: {verilog}"
    );
    assert!(
        !verilog.contains("[7:0] byte,"),
        "and does not emit the bare keyword: {verilog}"
    );

    let plan = Plan::of(&design, &module).unwrap();
    assert_eq!(
        plan.inputs
            .iter()
            .map(|p| p.verilog.as_str())
            .collect::<Vec<_>>(),
        ["rst", "byte_"],
        "the driver is given the names the Verilog declares, and the clock is still \
         not a stimulus column"
    );
    assert_eq!(
        plan.inputs
            .iter()
            .map(|p| p.name.as_str())
            .collect::<Vec<_>>(),
        ["rst", "byte"],
        "and the simulator is given the design's own names"
    );
}

#[test]
fn the_emitted_ports_are_the_ones_the_testbench_drives() {
    // The testbench and the cosimulator address ports by name, so a rename that
    // only lands in one of them turns every differential above into a test that
    // fails for a reason that has nothing to do with the CRC.
    let design = Design::new();
    crc3::build(&design).unwrap();
    // The design's own names, which is what the testbench drives. The *emitted*
    // names differ for `byte`, and `a_port_named_after_a_keyword_is_escaped_...`
    // is where that is checked.
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
    assert_eq!(inputs, ["clk", "rst", "byte"]);
    assert_eq!(outputs, ["crc"]);
}
