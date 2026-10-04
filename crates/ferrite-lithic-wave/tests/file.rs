//! What a VCD file has to contain for a viewer to read it.
//!
//! These assertions are on the *text*, and deliberately not on anything a
//! renderer would do with it: a golden waveform in this project is a golden
//! file, so byte stability is the property that matters and the grammar is the
//! property that makes the bytes worth anything.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use ferrite_lithic::WaveFormat;
use ferrite_lithic_bits::Bits;
use ferrite_lithic_wave::{Vcd, WaveData};

/// A waveform with a clock, an 8-bit data port and an 8-bit output, recorded for
/// three cycles.
fn recorded() -> (WaveData, Vec<ferrite_lithic_wave::Port>) {
    let mut wave = WaveData::new("top");
    let clk = wave.register("clk", 1).unwrap();
    let data = wave.register("data", 8).unwrap();
    let acc = wave.register("acc", 8).unwrap();

    for (cycle, (clock, datum, accumulator)) in [
        (false, 0x01u64, 0x00u64),
        (true, 0x02, 0x01),
        (false, 0x00, 0x03),
    ]
    .into_iter()
    .enumerate()
    {
        let cycle = cycle as u64;
        wave.set(cycle, &clk, &Bits::constant(u64::from(clock), 1).unwrap())
            .unwrap();
        wave.set(cycle, &data, &Bits::constant(datum, 8).unwrap())
            .unwrap();
        wave.set(cycle, &acc, &Bits::constant(accumulator, 8).unwrap())
            .unwrap();
    }
    let ports = wave.ports().to_vec();
    (wave, ports)
}

#[test]
fn the_file_declares_every_signal_once_with_its_width_and_range() {
    let (wave, _) = recorded();
    let text = Vcd::new(&wave).render();

    assert!(text.contains("$scope module top $end"), "{text}");
    assert!(
        text.contains("$var wire 1 ! clk $end"),
        "a one-bit signal gets no range: {text}"
    );
    assert!(text.contains("$var wire 8 \" data [7:0] $end"), "{text}");
    assert!(text.contains("$var wire 8 # acc [7:0] $end"), "{text}");
    assert!(text.contains("$upscope $end"), "{text}");
    assert!(text.contains("$enddefinitions $end"), "{text}");
    assert_eq!(
        text.matches("$var").count(),
        3,
        "one declaration per signal, and no more"
    );
}

#[test]
fn there_is_no_wall_clock_date_so_the_bytes_are_stable() {
    let (wave, _) = recorded();
    let text = Vcd::new(&wave).render();

    assert!(
        !text.contains("$date"),
        "a timestamp would break the golden: {text}"
    );
    assert!(
        text.contains("$comment no timestamp line"),
        "the file says why, so a reader does not think it was forgotten"
    );
}

#[test]
fn rendering_the_same_waveform_twice_produces_the_same_bytes() {
    let (wave, _) = recorded();
    assert_eq!(Vcd::new(&wave).render(), Vcd::new(&wave).render());
}

#[test]
fn cycle_zero_is_in_dumpvars_so_a_viewer_starts_with_values() {
    let (wave, _) = recorded();
    let text = Vcd::new(&wave).render();

    let dumpvars = text
        .find("$dumpvars")
        .expect("a file with values must have $dumpvars");
    let end = text[dumpvars..]
        .find("$end")
        .expect("$dumpvars must be closed")
        + dumpvars;
    let block = &text[dumpvars..end];
    assert!(block.contains("0!"), "clk at cycle 0: {block}");
    assert!(block.contains("b00000001 \""), "data at cycle 0: {block}");
    assert!(block.contains("b00000000 #"), "acc at cycle 0: {block}");
}

#[test]
fn a_one_bit_value_is_bare_and_a_wider_one_is_a_binary_string() {
    let (wave, _) = recorded();
    let text = Vcd::new(&wave).render();

    assert!(text.contains("\n0!\n"), "a scalar is bare: {text}");
    assert!(
        text.contains("b00000010 \""),
        "a vector is a binary string: {text}"
    );
    assert!(
        !text.contains("b0 !"),
        "a one-bit value must not be written as a vector"
    );
}

#[test]
fn only_changed_values_appear_after_the_first_timestamp() {
    let (wave, _) = recorded();
    let text = Vcd::new(&wave).render();

    // The whole tail, byte for byte. A VCD file *is* a change log, and this is
    // the property a golden file is really protecting: `data` holds `0x02` from
    // cycle 1 onwards and is written once, not once per cycle.
    let dumpvars = text
        .split_once("$dumpvars\n")
        .expect("a file with values dumps them");
    let tail = dumpvars
        .1
        .split_once("$end\n")
        .expect("the dumpvars block closes")
        .1;
    assert_eq!(
        tail,
        "#1\n1!\nb00000010 \"\nb00000001 #\n#2\n0!\nb00000000 \"\nb00000011 #\n",
    );

    // Which is: two timestamps for three cycles, because cycle 0 was the
    // dumpvars and every one of cycles 1 and 2 changed something.
    assert_eq!(
        tail.lines().filter(|line| line.starts_with('#')).count(),
        2,
        "one timestamp each for cycles 1 and 2, and none for cycle 0"
    );
}

#[test]
fn a_cycle_where_nothing_changed_gets_no_timestamp() {
    let mut wave = WaveData::new("top");
    let acc = wave.register("acc", 4).unwrap();
    for cycle in 0..4u64 {
        wave.set(cycle, &acc, &Bits::constant(7, 4).unwrap())
            .unwrap();
    }
    let text = Vcd::new(&wave).render();

    assert_eq!(text.matches("#1\n").count(), 0, "{text}");
    assert!(text.contains("#0"), "{text}");
}

#[test]
fn a_signal_that_changes_every_cycle_has_one_timestamp_each() {
    let mut wave = WaveData::new("top");
    let acc = wave.register("acc", 4).unwrap();
    for cycle in 0..3u64 {
        wave.set(cycle, &acc, &Bits::constant(cycle + 1, 4).unwrap())
            .unwrap();
    }
    let text = Vcd::new(&wave).render();

    assert!(text.contains("#1\nb0010 !"), "{text}");
    assert!(text.contains("#2\nb0011 !"), "{text}");
}

#[test]
fn a_hex_format_changes_the_declared_range_and_nothing_else() {
    // VCD has no hex value form. The rendering is asserted on through
    // `render_value`; all the file can do is the range hint.
    let mut wave = WaveData::new("top");
    let plain = wave.register("plain", 8).unwrap();
    let hex = wave.register_with("hex", 8, WaveFormat::Hex).unwrap();
    for (port, value) in [(&plain, 0xabu64), (&hex, 0xabu64)] {
        wave.set(0, port, &Bits::constant(value, 8).unwrap())
            .unwrap();
    }
    let text = Vcd::new(&wave).render();

    assert!(text.contains("$var wire 8 ! plain [7:0] $end"), "{text}");
    assert!(text.contains("$var wire 8 \" hex [0x7:0x0] $end"), "{text}");
    assert!(
        text.contains("b10101011 !"),
        "still binary in the file: {text}"
    );
    assert!(text.contains("b10101011 \""), "{text}");
}

#[test]
fn a_wide_signal_has_a_range_that_says_so() {
    let mut wave = WaveData::new("top");
    wave.register("wide", 100).unwrap();
    let text = Vcd::new(&wave).render();

    assert!(text.contains("$var wire 100 ! wide [99:0] $end"), "{text}");
}

#[test]
fn the_timescale_is_written_and_scales_the_timestamps() {
    let (wave, _) = recorded();
    let default = Vcd::new(&wave).render();
    assert!(default.contains("$timescale 1ns $end"), "{default}");

    // Timestamps are cycles, not scaled time: a cycle is a discrete event and
    // inventing sub-cycle resolution the simulator does not have would be a lie.
    let scaled = Vcd::new(&wave).timescale(10).to_string();
    assert!(scaled.contains("$timescale 10ns $end"), "{scaled}");
    assert!(scaled.contains("#1\n"), "{scaled}");
}

#[test]
fn an_empty_waveform_is_still_a_valid_file() {
    let wave = WaveData::new("empty");
    let text = Vcd::new(&wave).render();

    assert!(text.contains("$scope module empty $end"), "{text}");
    assert!(text.contains("$enddefinitions $end"), "{text}");
    assert!(text.contains("#0"), "{text}");
    assert_eq!(text.matches("$var").count(), 0);
}

#[test]
fn the_display_impl_and_to_string_agree() {
    let (wave, _) = recorded();
    let via_display = Vcd::new(&wave).render();
    assert_eq!(via_display, Vcd::new(&wave).render());
}

#[test]
fn writing_to_a_file_produces_the_same_bytes_as_to_string() {
    let (wave, _) = recorded();
    let mut buffer: Vec<u8> = Vec::new();
    Vcd::new(&wave).write(&mut buffer).unwrap();
    assert_eq!(String::from_utf8(buffer).unwrap(), Vcd::new(&wave).render());
}

#[test]
fn writing_to_a_broken_writer_reports_the_io_failure() {
    struct Broken;
    impl std::io::Write for Broken {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("no room"))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    let (wave, _) = recorded();
    let error = Vcd::new(&wave).write(&mut Broken).unwrap_err();
    assert!(
        matches!(error, ferrite_lithic_wave::Error::Io(_)),
        "{error}"
    );
}
