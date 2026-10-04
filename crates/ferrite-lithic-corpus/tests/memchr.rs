//! Single-byte search, checked three ways -- and losing the first one.
//!
//! The golden model is the `memchr` crate's `memchr::memchr`, which is the right golden
//! model here precisely because it is *good*: it is one of the most heavily optimised
//! byte scanners in existence, so a design that disagreed with it would be wrong and a
//! design that agreed with it would be right. The comparison here is deliberately
//! unfavourable to the hardware, and the last test measures the gap rather than
//! asserting a victory.
//!
//! Three things are compared, and it is worth being explicit about which is which:
//!
//! 1. **The answer.** `found` and `offset` against `memchr::memchr(NEEDLE, haystack)`.
//!    This is the differential that matters and it is checked over arbitrary input.
//! 2. **The shape of the answer.** `count` is checked to be exactly "bytes consumed",
//!    because an `offset` that is right for the wrong reason -- a counter that ran one
//!    cycle late, say -- would otherwise pass.
//! 3. **The two formulations of `hit`.** [`memchr::build`] uses a comparator and
//!    [`memchr::build_from_rom`] uses a 256-entry table. They are checked against each
//!    other over all 256 byte values, which is the only place a 256-arm `case_` gets
//!    exercised.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::Instant;

use ferrite_lithic::Design;
use ferrite_lithic_bits::Bits;
use ferrite_lithic_corpus::memchr;
use ferrite_lithic_cosim::{Harness, Plan, Stimulus, verilator};
use ferrite_lithic_tb::Testbench;
use proptest::prelude::*;

/// The design under test, built with the comparator form of `hit`.
fn design() -> Design {
    let design = Design::new();
    memchr::build(&design).unwrap();
    design
}

/// The design under test, built with the 256-entry table form of `hit`.
fn design_rom() -> Design {
    let design = Design::new();
    memchr::build_from_rom(&design).unwrap();
    design
}

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

/// Feeds `data` through the design, one byte per cycle, and returns `(found, offset)`.
///
/// `start` is asserted with `set` and then given an edge of its own, because the design
/// consumes a byte on *every* edge: spending the `start` edge with `drive` would discard
/// a real byte, and there is no way to deassert `start` without also spending an edge. The
/// byte on the `start` edge is discarded by the design, and it is 0 here -- which is
/// harmless for `memchr`, unlike a CRC, because a leading newline would simply be a
/// match at offset 0 and the differential would catch it.
fn hardware(design: &Design, data: &[u8]) -> (u64, u64) {
    let tb = Testbench::new(design).unwrap();
    tb.run(|tb| async move {
        tb.set("rst", 1).await;
        tb.set("start", 0).await;
        tb.set("data", 0).await;
        tb.step().await;
        tb.set("rst", 0).await;
        // The `start` edge: `set` so that asserting it costs no cycle, then one edge.
        tb.set("start", 1).await;
        tb.step().await;
        tb.set("start", 0).await;
        for byte in data {
            tb.drive("data", u64::from(*byte)).await;
        }
        (tb.value("found"), tb.value("offset"))
    })
    .expect("the memchr design drives only ports it declares")
}

/// Whether `data` contains more than one distinct byte.
///
/// A single-byte haystack either matches or does not, so a sweep made only of
/// one-byte buffers would pass against a design that compared against the wrong
/// constant for half its cases. Lengths below six are exempt because there is no room.
fn distinct(data: &[u8]) -> bool {
    let mut seen = [false; 256];
    for byte in data {
        seen[usize::from(*byte)] = true;
    }
    seen.iter().filter(|present| **present).count() > 1
}

#[test]
fn the_needle_is_the_one_the_module_documents() {
    // Stated rather than implied: the design searches for a newline, and a design that
    // searched for anything else would still be a working byte scanner and would pass
    // every differential below against a *different* golden model.
    assert_eq!(memchr::NEEDLE, b'\n');
    assert_eq!(memchr::COUNTER_BITS, 8);
}

#[test]
fn the_design_agrees_with_memchr_on_the_empty_buffer() {
    // No bytes, no match, and `offset` is zero because nothing latched it. A design
    // whose `offset` reset to something else would pass on every non-empty buffer too.
    assert_eq!(hardware(&design(), &[]), (0, 0));
    assert_eq!(
        ::memchr::memchr(memchr::NEEDLE, b""),
        None,
        "an empty haystack"
    );
}

#[test]
fn the_design_finds_a_match_at_offset_zero() {
    let data = *b"\nab";
    assert_eq!(hardware(&design(), &data), (1, 0));
    assert_eq!(::memchr::memchr(memchr::NEEDLE, &data), Some(0));
}

#[test]
fn the_design_finds_a_match_at_the_last_byte() {
    // The end of the buffer is where an off-by-one in the counter shows up, because the
    // last byte is the only one whose offset equals `len - 1`.
    for length in 1..=8usize {
        let mut data = vec![b'x'; length];
        data[length - 1] = memchr::NEEDLE;
        let want = ::memchr::memchr(memchr::NEEDLE, &data);
        assert_eq!(want, Some(length - 1), "the golden model, length {length}");
        assert_eq!(
            hardware(&design(), &data),
            (1, (length - 1) as u64),
            "length {length}: a match on the last byte"
        );
    }
}

#[test]
fn the_design_reports_no_match_when_there_is_none() {
    // Every byte value except the needle, one at a time and then all of them together.
    // A design that compared against the wrong constant, or against a truncated one,
    // fails here.
    let mut everything_else = Vec::new();
    for value in 0..=255u16 {
        let byte = u8::try_from(value).expect("the loop bounds a u8");
        if byte == memchr::NEEDLE {
            continue;
        }
        everything_else.push(byte);
        let data = [byte];
        assert_eq!(::memchr::memchr(memchr::NEEDLE, &data), None, "{byte:#04x}");
        assert_eq!(hardware(&design(), &data), (0, 0), "{byte:#04x}");
    }
    assert_eq!(everything_else.len(), 255);
    assert_eq!(hardware(&design(), &everything_else), (0, 0));
    assert_eq!(::memchr::memchr(memchr::NEEDLE, &everything_else), None);
}

#[test]
fn the_design_reports_the_first_match_and_not_the_last() {
    // Several newlines, so "the offset is just the counter" cannot pass. A design that
    // reported the most recent match would give 5 here instead of 2.
    let data = b"ab\ncd\nef\ngh\n";
    assert_eq!(::memchr::memchr(memchr::NEEDLE, data), Some(2));
    assert_eq!(hardware(&design(), data), (1, 2));
    assert_ne!(
        hardware(&design(), data).1,
        data.len() as u64 - 1,
        "and not the last one, which is what dropping the `not found` guard would give"
    );
}

#[test]
fn the_design_agrees_with_memchr_across_buffer_lengths() {
    // Lengths 0..=48, with a newline planted at a different position per length so that
    // a length-dependent bug cannot produce the right answer by accident. The needles
    // walk backwards through the buffer so both the "match at the end" and "match at the
    // start" boundaries are covered.
    for length in 0..=48usize {
        if length == 0 {
            assert_eq!(hardware(&design(), &[]), (0, 0));
            continue;
        }
        let at = length - 1 - (length / 3);
        let mut data = vec![b'.'; length];
        data[at] = memchr::NEEDLE;
        if length >= 6 {
            assert!(
                distinct(&data),
                "length {length}: a buffer of one repeated byte is not a useful case"
            );
        }
        let want = ::memchr::memchr(memchr::NEEDLE, &data);
        assert_eq!(want, Some(at), "the golden model, length {length}");
        assert_eq!(
            hardware(&design(), &data),
            (1, at as u64),
            "length {length}, needle at {at}"
        );
    }
}

#[test]
fn the_counter_is_the_number_of_bytes_consumed() {
    // `offset` being right does not prove the counter is right: an offset latched one
    // cycle early would report 1 instead of 2 and the differential would catch it, but
    // a counter that stops counting after a match would not be caught at all by a test
    // that only ever looks at the match. So the counter is read directly, over a buffer
    // with no needle in it and over one with a needle early.
    let design = design();
    let tb = Testbench::new(&design).unwrap();
    let clean = *b"xxxxxxxxxxxx";
    let dirty = b"ab\ncd\nefghijklmnop";
    tb.run(|tb| async move {
        tb.set("rst", 1).await;
        tb.set("start", 0).await;
        tb.set("data", 0).await;
        tb.step().await;
        tb.set("rst", 0).await;
        for (index, byte) in clean.iter().enumerate() {
            tb.drive("data", u64::from(*byte)).await;
            assert_eq!(
                tb.value("count"),
                index as u64 + 1,
                "no needle, byte {index}"
            );
            assert_eq!(tb.value("found"), 0);
        }
        tb.set("start", 1).await;
        tb.step().await;
        tb.set("start", 0).await;
        for (index, byte) in dirty.iter().enumerate() {
            tb.drive("data", u64::from(*byte)).await;
            assert_eq!(
                tb.value("count"),
                index as u64 + 1,
                "needle at 2, byte {index}: the counter keeps running after a match"
            );
        }
    })
    .unwrap();
    assert!(tb.series("count").iter().all(|bits| bits.width() == 8));
}

#[test]
fn a_start_mid_stream_clears_found_and_restarts_the_offset() {
    // A scanner that can only find one match per reset cannot scan two buffers, which is
    // every real use of one. The needle is planted twice and the second `start` must
    // discard the first match's offset.
    assert_eq!(::memchr::memchr(memchr::NEEDLE, b"x\n"), Some(1));
    assert_eq!(::memchr::memchr(memchr::NEEDLE, b"y\n"), Some(1));

    let design = design();
    let tb = Testbench::new(&design).unwrap();
    let observed = tb
        .run(|tb| async move {
            tb.set("rst", 1).await;
            tb.set("start", 0).await;
            tb.set("data", 0).await;
            tb.step().await;
            tb.set("rst", 0).await;
            for byte in b"x\n" {
                tb.drive("data", u64::from(*byte)).await;
            }
            let first = (tb.value("found"), tb.value("offset"), tb.value("count"));
            tb.set("start", 1).await;
            tb.step().await;
            let restarted = (tb.value("found"), tb.value("offset"), tb.value("count"));
            tb.set("start", 0).await;
            for byte in b"y\n" {
                tb.drive("data", u64::from(*byte)).await;
            }
            (first, restarted, (tb.value("found"), tb.value("offset")))
        })
        .unwrap();
    assert_eq!(observed.0, (1, 1, 2), "the first buffer");
    assert_eq!(
        observed.1,
        (0, 0, 0),
        "and the `start` edge clears all three: the byte on that edge is not part of \
         either search"
    );
    assert_eq!(
        observed.2,
        (1, 1),
        "the second buffer finds its own offset 1"
    );
}

#[test]
fn a_reset_mid_stream_does_the_same_thing_as_a_start() {
    // Both load zero. If only one did, every test above -- all of which reset at the
    // beginning -- would still pass.
    let observed = {
        let design = design();
        let tb = Testbench::new(&design).unwrap();
        tb.run(|tb| async move {
            tb.set("rst", 1).await;
            tb.set("start", 0).await;
            tb.set("data", 0).await;
            tb.step().await;
            tb.set("rst", 0).await;
            for byte in b"ab\n" {
                tb.drive("data", u64::from(*byte)).await;
            }
            let found = (tb.value("found"), tb.value("offset"));
            tb.set("rst", 1).await;
            tb.step().await;
            (
                found,
                (tb.value("found"), tb.value("offset"), tb.value("count")),
            )
        })
        .unwrap()
    };
    assert_eq!(observed.0, (1, 2));
    assert_eq!(
        observed.1,
        (0, 0, 0),
        "reset and start are indistinguishable"
    );
}

#[test]
fn the_comparator_and_the_256_entry_table_are_the_same_function() {
    // The only difference between the two builds is how `hit` is computed, so building
    // both and comparing their whole output series over every single byte value is a
    // complete check of that claim -- and it is the only test in the corpus that puts a
    // 256-arm `case_` through the simulator.
    let comparator = design();
    let rom = design_rom();
    for value in 0..=255u16 {
        let byte = u8::try_from(value).expect("the loop bounds a u8");
        let data = [byte, b'z'];
        assert_eq!(
            hardware(&comparator, &data),
            hardware(&rom, &data),
            "byte {byte:#04x}: the comparator and the 256-entry ROM disagree"
        );
    }
    // And on a longer buffer, so that the latches and the counter are compared too and
    // not only the one-cycle answer.
    let buffer: Vec<u8> = (0..40u8).map(|index| index.wrapping_mul(37)).collect();
    assert_eq!(hardware(&comparator, &buffer), hardware(&rom, &buffer));
}

#[test]
fn the_256_entry_table_is_really_256_arms() {
    // The module docs claim the table is the stress point for the emitter. Counted in
    // the emitted Verilog rather than in the graph, because "a `case_` with 256 arms" is
    // a claim about what comes out the other end.
    let design = design_rom();
    let clock = design
        .input_ports()
        .iter()
        .find(|signal| signal.name().as_deref() == Some(ferrite_lithic_corpus::CLOCK))
        .unwrap()
        .id();
    let module = ferrite_lithic_rtl::Module::new(
        "memchr_rom_probe",
        design.build().unwrap(),
        clock,
        design.input_ports().iter().map(|s| s.id()).collect(),
        design.output_ports().iter().map(|s| s.id()).collect(),
    );
    let verilog = module.emit().unwrap();
    let table: Vec<&str> = verilog
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with("8'h") && line.contains("= 1'h"))
        .collect();
    assert_eq!(table.len(), 256, "one arm per byte value, in\n{verilog}");
    assert_eq!(
        table.iter().filter(|arm| arm.ends_with("= 1'h1;")).count(),
        1,
        "and exactly one of them is a match, which is the whole point of the table's \
         one bit of content"
    );
    assert!(
        table.contains(&"8'h0a: signal_Case = 1'h1;"),
        "the needle's arm, in\n{verilog}"
    );
}

proptest! {
    /// The differential, on arbitrary input.
    ///
    /// Bounded because this is the slow half: the design is simulated a cycle per byte
    /// and a fresh simulator is built for every case. The sweeps above cover the length
    /// and position boundaries; this covers the values.
    #[test]
    fn the_design_agrees_with_the_memchr_crate_on_arbitrary_buffers(
        data in prop::collection::vec(any::<u8>(), 0..40)
    ) {
        let want = ::memchr::memchr(memchr::NEEDLE, &data);
        let (found, offset) = hardware(&design(), &data);
        if want.is_none() {
            prop_assert_eq!((found, offset), (0, 0));
        } else {
            let at = u64::try_from(want.unwrap()).expect("a haystack under 40 bytes");
            prop_assert_eq!((found, offset), (1, at));
        }
    }
}

#[test]
fn the_cpu_wins_on_bytes_per_cycle_and_this_test_says_so() {
    // The honest comparison, measured rather than asserted.
    //
    // **On bytes per cycle the CPU wins by more than an order of magnitude.** The
    // `memchr` crate compiles to an AVX2 compare of 32 bytes followed by a `tzcnt`, so
    // it retires roughly one instruction per 32 bytes; this design retires one per byte
    // by construction and cannot do better without abandoning the serial-automaton shape.
    //
    // What is measured here is *this host's* wall clock for the same work, which is not
    // a claim about an ASIC at all -- an ASIC's number is one byte per clock edge by
    // definition, and it is compared against a CPU that has to keep up with a stream
    // while doing something else. So the assertion is deliberately weak: only that the
    // hardware finds the same answer, and that both finished. The ratio is printed.
    // A megabyte of filler with a single needle in the middle. The filler is a single
    // repeated byte rather than a pattern, because `index % 251` would put a newline at
    // offset 10 -- the needle -- and the measurement would then be of a short scan.
    let mut with_needle = vec![b'x'; 1 << 20];
    let needle_at = with_needle.len() / 2;
    with_needle[needle_at] = memchr::NEEDLE;
    assert!(
        with_needle
            .iter()
            .filter(|byte| **byte == memchr::NEEDLE)
            .count()
            == 1
    );

    // Best of two so the numbers are not dominated by whichever run the scheduler
    // decided to interrupt.
    let mut hardware_ns = f64::MAX;
    let mut crate_ns = f64::MAX;
    for _ in 0..2 {
        let started = Instant::now();
        let (found, offset) = hardware(&design(), &with_needle);
        hardware_ns = hardware_ns.min(started.elapsed().as_secs_f64() * 1e9);
        // `offset` is eight bits and wraps -- see `COUNTER_BITS` -- so on a buffer this
        // long it is the offset modulo 256 and nothing else. The differential tests are
        // the ones that check the offset exactly, and they bound their inputs to 48
        // bytes; this one is measuring time, not correctness.
        assert_eq!((found, offset), (1, (needle_at % 256) as u64));

        let started = Instant::now();
        let want = ::memchr::memchr(memchr::NEEDLE, &with_needle);
        crate_ns = crate_ns.min(started.elapsed().as_secs_f64() * 1e9);
        assert_eq!(want, Some(needle_at));
    }

    let bytes = with_needle.len() as f64;
    println!(
        "memchr: design {:.3} ns/byte of simulated time, `memchr` crate {:.4} ns/byte, \
         ratio {:.1}x in the crate's favour. The design is one byte per clock edge by \
         construction; the crate is 32 bytes per AVX2 instruction.",
        hardware_ns / bytes,
        crate_ns / bytes,
        hardware_ns / crate_ns,
    );
    assert!(
        crate_ns.is_finite() && crate_ns > 0.0,
        "the measurement produced a usable number"
    );
}

#[test]
fn the_simulator_and_the_emitted_verilog_agree_on_the_comparator_design() {
    let Some(verilator) = verilator() else {
        println!(
            "SKIPPED: verilator not found, so the memchr equivalence did not run. The \
             differential against the `memchr` crate above does not need it."
        );
        return;
    };
    println!("cosimulating memchr with {}", verilator.display());

    let design = design();
    let plan = plan_of(&design, "memchr");
    assert_eq!(
        plan.inputs
            .iter()
            .map(|p| p.name.as_str())
            .collect::<Vec<_>>(),
        ["rst", "start", "data"],
        "the clock is a port but not a stimulus column"
    );
    assert_eq!(
        plan.outputs
            .iter()
            .map(|p| p.name.as_str())
            .collect::<Vec<_>>(),
        ["found", "offset", "count"],
        "all three outputs are compared, including the counter"
    );

    let mut stimulus = Stimulus::new();
    let row = |rst: u64, start: u64, data: u64| {
        vec![
            Bits::constant(rst, 1).unwrap(),
            Bits::constant(start, 1).unwrap(),
            Bits::constant(data, 8).unwrap(),
        ]
    };
    stimulus.push(row(1, 0, 0)).unwrap();
    stimulus.push(row(0, 0, 0)).unwrap();
    stimulus.push(row(0, 1, 0)).unwrap();
    stimulus.push(row(0, 0, 0)).unwrap();
    for index in 0..96u64 {
        stimulus
            .push(row(0, 0, index.wrapping_mul(37).wrapping_add(11)))
            .unwrap();
    }
    // A `start` in the middle, so the equivalence covers the restart path as well as the
    // scan: a design whose `start` gating disagreed between the two backends would pass
    // every test above and fail here.
    stimulus.push(row(0, 1, 0)).unwrap();
    stimulus.push(row(0, 0, 0)).unwrap();
    for index in 0..32u64 {
        stimulus.push(row(0, 0, 200 + index % 7)).unwrap();
    }
    assert_eq!(stimulus.cycles(), 4 + 96 + 2 + 32);

    let verilog = ferrite_lithic_cosim::emit(&design, &plan).unwrap();
    let harness = Harness::build(&plan, &verilog, &plan.work_dir()).unwrap();
    let report = ferrite_lithic_cosim::run_with(&design, &plan, &stimulus, &harness).unwrap();
    assert!(
        report.is_equivalent(),
        "the two backends disagree: {report}\nfirst: {:?}",
        report.first()
    );

    // And the same bytes through the testbench agree with the crate, so all three of
    // them -- simulator, Verilator and `memchr` -- meet.
    let message: Vec<u8> = (0..96u8)
        .map(|index| index.wrapping_mul(37).wrapping_add(11))
        .collect();
    let want = ::memchr::memchr(memchr::NEEDLE, &message).unwrap();
    assert_eq!(hardware(&design, &message), (1, want as u64));
}

#[test]
fn the_simulator_and_the_emitted_verilog_agree_on_the_256_entry_table_design() {
    // A second equivalence run, on the 256-arm `case_`. Distinct module name so the two
    // do not share a work directory.
    let Some(verilator) = verilator() else {
        println!("SKIPPED: verilator not found, so the memchr_rom equivalence did not run.");
        return;
    };
    println!("cosimulating memchr_rom with {}", verilator.display());

    let design = design_rom();
    let plan = plan_of(&design, "memchr_rom");
    let mut stimulus = Stimulus::new();
    let row = |rst: u64, start: u64, data: u64| {
        vec![
            Bits::constant(rst, 1).unwrap(),
            Bits::constant(start, 1).unwrap(),
            Bits::constant(data, 8).unwrap(),
        ]
    };
    stimulus.push(row(1, 0, 0)).unwrap();
    stimulus.push(row(0, 0, 0)).unwrap();
    stimulus.push(row(0, 1, 0)).unwrap();
    stimulus.push(row(0, 0, 0)).unwrap();
    for value in 0..=255u64 {
        stimulus.push(row(0, 0, value)).unwrap();
    }
    assert_eq!(stimulus.cycles(), 4 + 256);

    let verilog = ferrite_lithic_cosim::emit(&design, &plan).unwrap();
    let harness = Harness::build(&plan, &verilog, &plan.work_dir()).unwrap();
    let report = ferrite_lithic_cosim::run_with(&design, &plan, &stimulus, &harness).unwrap();
    assert!(
        report.is_equivalent(),
        "the two backends disagree on the 256-entry table: {report}\nfirst: {:?}",
        report.first()
    );
}
