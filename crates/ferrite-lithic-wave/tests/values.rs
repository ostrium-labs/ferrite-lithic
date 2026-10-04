//! What a waveform is for: values that survive the round trip, and a file that
//! is byte-stable.
//!
//! Two things are checked separately on purpose. [`values`] asserts on
//! cycle-indexed `Bits`; [`file`] asserts on the VCD text. A test that only
//! checked the file would pass with a writer that recorded nothing, and a test
//! that only checked the values would not notice a malformed file.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use ferrite_lithic::{Design, Signal};
use ferrite_lithic_bits::Bits;
use ferrite_lithic_derive::PortList;
use ferrite_lithic_wave::{Error, Port, WaveData, identifier_for, render_value};

/// A shape with a clock, a one-bit port, and two byte-wide ones.
#[derive(PortList)]
struct Shifter {
    #[clock]
    clk: Signal,
    #[bits(8)]
    d: Signal,
    #[bits(8)]
    q: Signal,
}

#[test]
fn a_value_written_is_a_value_read() {
    let mut wave = WaveData::new("top");
    let acc = wave.register("acc", 4).unwrap();

    for (cycle, value) in [0b0001u64, 0b0010, 0b0100].into_iter().enumerate() {
        wave.set(cycle as u64, &acc, &Bits::constant(value, 4).unwrap())
            .unwrap();
    }

    assert_eq!(wave.cycles(), 3);
    assert_eq!(wave.value(0, &acc).unwrap().to_u64().unwrap(), 1);
    assert_eq!(wave.value(2, &acc).unwrap().to_u64().unwrap(), 4);
    assert_eq!(
        wave.value(3, &acc),
        None,
        "an unrecorded cycle reads as absent"
    );
}

#[test]
fn a_one_bit_signal_reads_as_a_bit() {
    let mut wave = WaveData::new("top");
    let clk = wave.register("clk", 1).unwrap();

    wave.set(0, &clk, &Bits::zeros(1).unwrap()).unwrap();
    wave.set(1, &clk, &Bits::ones(1).unwrap()).unwrap();

    assert!(!wave.value(0, &clk).unwrap().bit(0).unwrap());
    assert!(wave.value(1, &clk).unwrap().bit(0).unwrap());
}

#[test]
fn writing_out_of_order_fills_the_gap_rather_than_moving_values() {
    let mut wave = WaveData::new("top");
    let acc = wave.register("acc", 4).unwrap();

    wave.set(4, &acc, &Bits::constant(9, 4).unwrap()).unwrap();
    wave.set(2, &acc, &Bits::constant(3, 4).unwrap()).unwrap();

    assert_eq!(wave.cycles(), 5);
    assert_eq!(wave.value(2, &acc).unwrap().to_u64().unwrap(), 3);
    assert_eq!(wave.value(4, &acc).unwrap().to_u64().unwrap(), 9);
    // Out of order does not move values: cycle 4 still holds what was written to
    // cycle 4, and the recording started at cycle 2 even though 4 was written
    // first.
    assert_eq!(
        wave.value(0, &acc),
        None,
        "cycle 0 is before the first recorded cycle, so it is absent rather than zero"
    );
    assert_eq!(
        wave.value(1, &acc),
        None,
        "and so is cycle 1, which nobody wrote"
    );
    // The series starts where the recording started, not at index zero.
    let series = wave.series(&acc).unwrap();
    assert_eq!(series.len(), 3, "cycles 2, 3 and 4");
    assert_eq!(series[0].to_u64().unwrap(), 3, "cycle 2");
    assert_eq!(series[2].to_u64().unwrap(), 9, "cycle 4");
}

#[test]
fn a_cycle_before_the_first_record_is_absent_and_not_zero() {
    // The simulator numbers its first edge cycle 1, so a waveform driven by one
    // starts at 1. Reporting cycle 0 as a zero row would put a plausible-looking
    // value in front of every series, which is how an off-by-one in a test becomes
    // an off-by-one that still passes.
    let mut wave = WaveData::new("top");
    let acc = wave.register("acc", 4).unwrap();
    for cycle in 1..=3u64 {
        wave.set(cycle, &acc, &Bits::constant(cycle, 4).unwrap())
            .unwrap();
    }
    assert_eq!(wave.value(0, &acc), None);
    assert_eq!(wave.value(1, &acc).unwrap().to_u64().unwrap(), 1);
    assert_eq!(wave.series(&acc).unwrap().len(), 3);
}

#[test]
fn a_value_of_the_wrong_width_is_refused_rather_than_truncated() {
    let mut wave = WaveData::new("top");
    let acc = wave.register("acc", 4).unwrap();

    let error = wave
        .set(0, &acc, &Bits::constant(0xff, 8).unwrap())
        .unwrap_err();
    match error {
        Error::Width {
            port,
            expected,
            got,
        } => {
            assert_eq!(port, "acc");
            assert_eq!(expected, 4);
            assert_eq!(got, 8);
        }
        other => panic!("expected a width error, got {other}"),
    }
    assert_eq!(wave.cycles(), 0, "a refused write records nothing");
}

#[test]
fn two_signals_cannot_share_one_name() {
    let mut wave = WaveData::new("top");
    wave.register("acc", 4).unwrap();

    let error = wave.register("acc", 8).unwrap_err();
    assert!(matches!(error, Error::DuplicateName { .. }), "{error}");
    assert_eq!(wave.ports().len(), 1);
}

#[test]
fn a_zero_width_signal_is_refused() {
    let mut wave = WaveData::new("top");
    let error = wave.register("nothing", 0).unwrap_err();
    assert!(matches!(error, Error::ZeroWidth { .. }), "{error}");
}

#[test]
fn a_port_from_another_waveform_is_refused() {
    let mut one = WaveData::new("one");
    let mut two = WaveData::new("two");
    two.register("acc", 4).unwrap();
    let foreign = one.register("acc", 4).unwrap();

    // Same name, different waveform: the error is about the waveform, not the
    // name, which is why it cannot be caught by name lookup alone.
    let error = two
        .set(0, &foreign, &Bits::constant(1, 4).unwrap())
        .unwrap_err();
    assert!(matches!(error, Error::UnknownPort { .. }), "{error}");
}

#[test]
fn an_unrecorded_cycle_is_absent_rather_than_zero() {
    let mut wave = WaveData::new("top");
    let acc = wave.register("acc", 4).unwrap();

    wave.set(0, &acc, &Bits::zeros(4).unwrap()).unwrap();
    assert_eq!(wave.cycles(), 1);
    assert!(wave.render(0, &acc).is_ok());
    assert!(matches!(
        wave.render(1, &acc).unwrap_err(),
        Error::UnknownCycle {
            cycle: 1,
            recorded: 1
        }
    ));
}

#[test]
fn a_lookup_that_misses_a_signal_is_refused_rather_than_zeroed() {
    let mut wave = WaveData::new("top");
    wave.register("acc", 4).unwrap();
    wave.register("d", 8).unwrap();

    let error = wave.record(0, |name| (name == "acc").then(|| Bits::zeros(4).unwrap()));
    assert!(
        matches!(error, Err(Error::UnknownSignal { .. })),
        "{error:?}"
    );
    assert_eq!(wave.cycles(), 0, "a refused record records nothing");
}

#[test]
fn a_record_fills_every_signal_for_a_cycle() {
    let mut wave = WaveData::new("top");
    wave.register("acc", 4).unwrap();
    wave.register("d", 8).unwrap();

    wave.record(0, |name| match name {
        "acc" => Some(Bits::constant(5, 4).unwrap()),
        "d" => Some(Bits::constant(0xab, 8).unwrap()),
        _ => None,
    })
    .unwrap();

    let acc = wave.ports()[0].clone();
    assert_eq!(wave.value(0, &acc).unwrap().to_u64().unwrap(), 5);
    assert_eq!(wave.series_named("d").unwrap()[0].to_u64().unwrap(), 0xab);
}

#[test]
fn changes_are_reported_where_the_value_actually_moved() {
    let mut wave = WaveData::new("top");
    let acc = wave.register("acc", 4).unwrap();

    for (cycle, value) in [1u64, 1, 2, 2, 3].into_iter().enumerate() {
        wave.set(cycle as u64, &acc, &Bits::constant(value, 4).unwrap())
            .unwrap();
    }

    let changes: Vec<(u64, u64)> = wave
        .series_changes(&acc)
        .into_iter()
        .map(|(cycle, value)| (cycle, value.to_u64().unwrap()))
        .collect();
    assert_eq!(changes, [(0, 1), (2, 2), (4, 3)]);
}

#[test]
fn asserting_a_series_reports_the_first_cycle_that_differs() {
    let mut wave = WaveData::new("top");
    let acc = wave.register("acc", 4).unwrap();
    for (cycle, value) in [1u64, 2, 3].into_iter().enumerate() {
        wave.set(cycle as u64, &acc, &Bits::constant(value, 4).unwrap())
            .unwrap();
    }

    let expected: Vec<Bits> = [1u64, 9, 3]
        .into_iter()
        .map(|value| Bits::constant(value, 4).unwrap())
        .collect();
    let mismatch = wave.assert_series(&acc, &expected).unwrap_err();
    assert_eq!(mismatch.cycle, 1);
    assert_eq!(mismatch.port, "acc");
    assert_eq!(mismatch.expected.to_u64().unwrap(), 9);
    assert_eq!(
        mismatch.got.as_ref().unwrap().to_u64().unwrap(),
        2,
        "the message is built before the move below, so this checks the value first"
    );
    assert!(mismatch.to_string().contains("cycle 1"), "{mismatch}");
}

#[test]
fn a_waveform_that_ran_too_long_fails_rather_than_passing_the_prefix() {
    let mut wave = WaveData::new("top");
    let acc = wave.register("acc", 4).unwrap();
    wave.set(0, &acc, &Bits::constant(1, 4).unwrap()).unwrap();
    for (cycle, value) in [1u64, 2].into_iter().enumerate() {
        wave.set(cycle as u64, &acc, &Bits::constant(value, 4).unwrap())
            .unwrap();
    }

    let expected = vec![Bits::constant(1, 4).unwrap()];
    assert!(wave.assert_series(&acc, &expected).is_err());
}

#[test]
fn asserting_a_series_reports_a_cycle_that_was_never_recorded() {
    let mut wave = WaveData::new("top");
    let acc = wave.register("acc", 4).unwrap();
    wave.set(0, &acc, &Bits::constant(1, 4).unwrap()).unwrap();

    let expected: Vec<Bits> = [1u64, 2]
        .into_iter()
        .map(|value| Bits::constant(value, 4).unwrap())
        .collect();
    let mismatch = wave.assert_series(&acc, &expected).unwrap_err();
    assert_eq!(mismatch.cycle, 1);
    assert_eq!(mismatch.got, None);
    assert!(
        mismatch.to_string().contains("never recorded"),
        "{mismatch}"
    );
}

#[test]
fn formats_render_the_value_not_the_file() {
    let value = Bits::constant(0xab, 8).unwrap();
    assert_eq!(
        render_value(&value, ferrite_lithic::WaveFormat::Binary),
        "10101011"
    );
    assert_eq!(
        render_value(&value, ferrite_lithic::WaveFormat::Hex),
        "0xab"
    );
    assert_eq!(
        render_value(&value, ferrite_lithic::WaveFormat::Decimal),
        "171"
    );

    let mut wave = WaveData::new("top");
    let hex = wave
        .register_with("acc", 8, ferrite_lithic::WaveFormat::Hex)
        .unwrap();
    wave.set(0, &hex, &value).unwrap();
    assert_eq!(wave.render(0, &hex).unwrap(), "0xab");
}

#[test]
fn hex_is_padded_to_the_ports_width_in_nibbles() {
    let value = Bits::constant(0x5, 12).unwrap();
    assert_eq!(
        render_value(&value, ferrite_lithic::WaveFormat::Hex),
        "0x005",
        "twelve bits is three nibbles, and a short number would misalign in a viewer"
    );
}

#[test]
fn a_value_wider_than_a_word_still_renders_in_decimal() {
    // Wider than 64 bits has no `u64` decimal, and the fallback is the binary
    // digits rather than a truncated number.
    let wide = Bits::ones(96).unwrap();
    assert_eq!(
        render_value(&wide, ferrite_lithic::WaveFormat::Decimal),
        "1".repeat(96)
    );
}

#[test]
fn a_shape_declares_its_own_signals_in_the_verilog_spelling() {
    let design = Design::new();
    let ports = ferrite_lithic::inputs::<Shifter>(&design).unwrap();

    let mut wave = WaveData::new("top");
    let declared = wave.register_shape::<Shifter>().unwrap();

    assert_eq!(
        declared.iter().map(Port::name).collect::<Vec<_>>(),
        ["clk", "d", "q"]
    );
    assert_eq!(
        declared.iter().map(Port::width).collect::<Vec<_>>(),
        [1, 8, 8]
    );
    assert_eq!(ports.to_signals().len(), declared.len());
}

#[test]
fn identifiers_are_positional_so_any_name_can_be_dumped() {
    assert_eq!(identifier_for(0), "!");
    assert_eq!(identifier_for(1), "\"");
    assert_eq!(identifier_for(93), "~");
    // Most significant first, so the codes sort in declaration order.
    assert_eq!(identifier_for(94), "\"!", "the first two-character code");
    assert_eq!(identifier_for(95), "\"\"");
    assert_eq!(
        identifier_for(94 * 94),
        "\"!!",
        "the first three-character code"
    );
    assert_eq!(
        identifier_for(94 * 94 * 94 - 1),
        "~~~",
        "the last three-character code"
    );

    // Two ports whose names legalise alike still get distinct identifiers, which
    // is the whole reason the codes are not derived from names.
    assert_ne!(identifier_for(0), identifier_for(1));
}

#[test]
fn a_shape_registers_its_wave_formats() {
    #[derive(PortList)]
    struct Mixed {
        plain: Signal,
        #[bits(4)]
        #[wave_format("decimal")]
        count: Signal,
    }

    let mut wave = WaveData::new("top");
    let declared = wave.register_shape::<Mixed>().unwrap();
    assert_eq!(declared[0].format(), ferrite_lithic::WaveFormat::Binary);
    assert_eq!(declared[1].format(), ferrite_lithic::WaveFormat::Decimal);
}

#[test]
fn a_lookup_of_a_name_the_waveform_does_not_have_lists_the_ones_it_does() {
    let mut wave = WaveData::new("top");
    wave.register("acc", 4).unwrap();

    let error = wave.series_named("nope").unwrap_err();
    assert!(error.to_string().contains("acc"), "{error}");
}

#[test]
fn a_registered_port_can_change_its_format_without_changing_its_width() {
    let mut wave = WaveData::new("top");
    let acc = wave.register("acc", 8).unwrap();
    let hex = acc.clone().with_format(ferrite_lithic::WaveFormat::Hex);
    wave.set(0, &hex, &Bits::constant(0x1f, 8).unwrap())
        .unwrap();
    assert_eq!(hex.index(), 0);

    // The same name, so the value is readable through either handle.
    assert_eq!(wave.render(0, &hex).unwrap(), "0x1f");
    assert_eq!(wave.render(0, &acc).unwrap(), "00011111");
}
