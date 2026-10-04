//! CRC-32/ISO-HDLC against `crc32fast`, the crate everyone actually uses.
//!
//! The golden model here is [`crc32fast`] itself rather than a transcription of the
//! parameters, because this algorithm has a crate whose entire purpose is to be the
//! reference implementation. Where `crc3` had to transcribe (there is no `crc3fast`
//! in the registry), this one does not, and that difference is worth noticing: the
//! test that checks this design is checking it against the thing that ships in
//! zlib, PNG and Ethernet.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use ferrite_lithic::Design;
use ferrite_lithic_bits::Bits;
use ferrite_lithic_corpus::crc32;
use ferrite_lithic_tb::Testbench;
use proptest::prelude::*;

fn golden() -> crc32fast::Hasher {
    crc32fast::Hasher::new()
}

/// Feeds `data` through the design, one byte per cycle, and returns `(state, crc)`.
///
/// The `init` pulse matters: the design has to start from `0xffffffff` rather than
/// from zero, and that is exactly the part a CRC with a zero init gets for free.
fn hardware(data: &[u8]) -> (u64, u64) {
    let design = Design::new();
    crc32::build(&design).unwrap();
    let tb = Testbench::new(&design).unwrap();
    tb.run(|tb| async move {
        // `set` rather than `drive`, because the design consumes a byte on every
        // edge: asserting init needs an edge of its own, and deasserting it must not
        // spend a cycle. The byte on the init edge is discarded, so it is 0 here and
        // stays 0 -- a leading zero byte is *not* neutral for this CRC.
        tb.set("init", 1).await;
        tb.set("byte", 0).await;
        tb.step().await;
        tb.set("init", 0).await;
        for byte in data {
            tb.drive("byte", *byte as u64).await;
        }
        (tb.value("state"), tb.value("crc"))
    })
    .expect("the crc32 design drives only ports it declares")
}

#[test]
fn the_design_agrees_with_crc32fast_on_the_catalogues_check_string() {
    // The check value is the catalogue's, and `crc32fast` independently produces it,
    // so this pins the init, the tap and the xorout all at once.
    let mut hasher = golden();
    hasher.update(b"123456789");
    assert_eq!(
        u64::from(hasher.finalize()),
        crc32::CHECK,
        "the published check value"
    );
    let (_, crc) = hardware(b"123456789");
    assert_eq!(crc, crc32::CHECK);
}

#[test]
fn the_design_starts_from_the_init_value_not_from_zero() {
    // A CRC with a zero init would agree on the empty message and disagree on
    // everything else. Checking the empty message alone therefore does not
    // distinguish them, which is why this asserts the state as well.
    let (state, crc) = hardware(&[]);
    assert_eq!(
        state,
        crc32::INIT,
        "an empty message still leaves the register at init, not at zero"
    );
    assert_eq!(
        crc, 0,
        "and crc is init xorout, which cancels to zero -- the same answer a \
         zero-init CRC would give for this message, which is why the state is checked"
    );
}

#[test]
fn the_raw_state_and_the_xorout_view_are_both_correct() {
    // `state` is what a receiver feeds back to get zero; `crc` is what software
    // prints. They are different answers, and the design must give both.
    let (state, crc) = hardware(b"hello");
    let mut hasher = golden();
    hasher.update(b"hello");
    let want = u64::from(hasher.finalize());
    assert_eq!(crc, want);
    assert_eq!(
        state,
        want ^ crc32::XOROUT,
        "state is crc with xorout removed"
    );
}

#[test]
fn the_design_agrees_with_crc32fast_across_the_byte_range() {
    // Every single-byte message. Cheap, and a wrong tap or a wrong bit order shows up
    // on most of them rather than on one.
    for value in 0..=255u64 {
        let data = [value as u8];
        let mut hasher = golden();
        hasher.update(&data);
        assert_eq!(
            hardware(&data).1,
            u64::from(hasher.finalize()),
            "one byte {value:#04x}"
        );
    }
}

#[test]
fn the_design_agrees_with_crc32fast_across_buffer_lengths() {
    // Lengths 0..=64, because the byte-parallel form is eight steps of the same
    // recurrence and a bug that only appears on certain alignments would otherwise
    // hide. (There is no alignment to get wrong here — a CRC is bit-serial by
    // construction — but a *width* boundary at 32 and 64 bits is exactly where an
    // accidentally-wide signal would show up.)
    for length in 0..=64usize {
        let data: Vec<u8> = (0..length)
            .map(|index| (index as u8).wrapping_mul(97).wrapping_add(3))
            .collect();
        let mut hasher = golden();
        hasher.update(&data);
        assert_eq!(
            hardware(&data).1,
            u64::from(hasher.finalize()),
            "length {length}"
        );
    }
}

#[test]
fn an_init_between_two_messages_restarts_the_crc() {
    // On a bus this is every packet. A design that can only be initialised by a reset
    // cannot start a second one, and a test that only ever sends one message would
    // not know.
    let design = Design::new();
    crc32::build(&design).unwrap();
    let tb = Testbench::new(&design).unwrap();
    let observed = tb
        .run(|tb| async move {
            tb.set("init", 1).await;
            tb.set("byte", 0).await;
            tb.step().await;
            tb.set("init", 0).await;
            for byte in b"first message" {
                tb.drive("byte", *byte as u64).await;
            }
            let first = tb.value("crc");
            tb.set("init", 1).await;
            tb.step().await;
            let restarted = tb.value("state");
            tb.set("init", 0).await;
            for byte in b"first message" {
                tb.drive("byte", *byte as u64).await;
            }
            let second = tb.value("crc");
            (first, restarted, second)
        })
        .unwrap();
    let mut hasher = golden();
    hasher.update(b"first message");
    assert_eq!(observed.0, u64::from(hasher.finalize()));
    assert_eq!(observed.1, crc32::INIT, "init reloads on its own edge");
    assert_eq!(
        observed.2, observed.0,
        "and the same message after an init gives the same CRC"
    );
}

#[test]
fn a_reset_and_an_init_agree() {
    // Both load the init value, so they must be indistinguishable from outside. If
    // only one of them did, a design would pass every test that used just one.
    let design = Design::new();
    crc32::build(&design).unwrap();
    let tb = Testbench::new(&design).unwrap();
    let observed = tb
        .run(|tb| async move {
            tb.drive("byte", 200).await;
            // One edge of reset, so that the edge carries a known byte rather than
            // whatever `pulse` leaves behind.
            tb.set("rst", 1).await;
            tb.step().await;
            tb.set("rst", 0).await;
            let after_reset = tb.value("state");
            for byte in b"abc" {
                tb.drive("byte", *byte as u64).await;
            }
            let moved_on = tb.value("state");
            tb.set("init", 1).await;
            tb.step().await;
            (after_reset, moved_on, tb.value("state"))
        })
        .unwrap();
    assert_eq!(observed.0, crc32::INIT, "reset loads init");
    assert_ne!(
        observed.1,
        crc32::INIT,
        "and bytes after it move the register"
    );
    assert_eq!(
        observed.2,
        crc32::INIT,
        "and init reloads it even mid-message"
    );
}

proptest! {
    /// The differential, on arbitrary data.
    ///
    /// Bounded because this is the slow half: the design is simulated a cycle per
    /// byte. The sweeps above cover the length and alignment boundaries; this covers
    /// the values.
    #[test]
    fn the_design_agrees_with_crc32fast_on_arbitrary_buffers(
        data in prop::collection::vec(any::<u8>(), 0..48)
    ) {
        let mut hasher = golden();
        hasher.update(&data);
        prop_assert_eq!(hardware(&data).1, u64::from(hasher.finalize()));
    }
}

#[test]
fn the_state_port_is_thirty_two_bits_wide_and_never_exceeds_it() {
    // A register that grew past its width would still produce plausible CRCs for
    // short messages and be wrong for long ones, and the recorded series is the
    // cheapest place to notice.
    let design = Design::new();
    crc32::build(&design).unwrap();
    let tb = Testbench::new(&design).unwrap();
    tb.run(|tb| async move {
        tb.set("init", 1).await;
        tb.set("byte", 0).await;
        tb.step().await;
        tb.set("init", 0).await;
        for index in 0..64u64 {
            tb.drive("byte", index.wrapping_mul(31)).await;
        }
    })
    .unwrap();
    let series = tb.series("state");
    assert_eq!(series.len(), 65, "the one init edge and 64 bytes");
    assert!(series.iter().all(|value| value.width() == 32));
    assert!(
        series.iter().all(|value| value.bit(32).is_err()),
        "no bit above the width"
    );
}

#[test]
fn the_simulator_and_the_emitted_verilog_agree_on_this_design() {
    let Some(verilator) = ferrite_lithic_cosim::verilator() else {
        println!(
            "SKIPPED: verilator not found, so the crc32 equivalence did not run. The \
             differential against `crc32fast` above does not need it."
        );
        return;
    };
    println!("cosimulating crc32 with {}", verilator.display());

    let design = Design::new();
    let ports = crc32::build(&design).unwrap();
    let module = ferrite_lithic_rtl::Module::new(
        "crc32",
        design.build().unwrap(),
        ports.inputs.clk.id(),
        design.input_ports().iter().map(|s| s.id()).collect(),
        design.output_ports().iter().map(|s| s.id()).collect(),
    );
    let plan = ferrite_lithic_cosim::Plan::of(&design, &module).unwrap();
    assert_eq!(
        plan.inputs
            .iter()
            .map(|p| p.name.as_str())
            .collect::<Vec<_>>(),
        ["rst", "init", "byte"],
        "the clock is a port but not a stimulus column"
    );

    // A stimulus that initialises, resets, and then feeds a message long enough to
    // exercise every one of the eight steps many times over.
    let mut stimulus = ferrite_lithic_cosim::Stimulus::new();
    let row = |rst: u64, init: u64, byte: u64| {
        vec![
            Bits::constant(rst, 1).unwrap(),
            Bits::constant(init, 1).unwrap(),
            Bits::constant(byte, 8).unwrap(),
        ]
    };
    stimulus.push(row(0, 1, 0)).unwrap();
    stimulus.push(row(1, 0, 0)).unwrap();
    stimulus.push(row(0, 0, 0)).unwrap();
    stimulus.push(row(0, 1, 0)).unwrap();
    stimulus.push(row(0, 0, 0)).unwrap();
    for index in 0..64u64 {
        stimulus
            .push(row(0, 0, index.wrapping_mul(37).wrapping_add(11)))
            .unwrap();
    }
    assert_eq!(stimulus.cycles(), 69);

    let verilog = ferrite_lithic_cosim::emit(&design, &plan).unwrap();
    let harness = ferrite_lithic_cosim::Harness::build(&plan, &verilog, &plan.work_dir()).unwrap();
    let report = ferrite_lithic_cosim::run_with(&design, &plan, &stimulus, &harness).unwrap();
    assert!(
        report.is_equivalent(),
        "the two backends disagree: {report}\nfirst: {:?}",
        report.first()
    );

    // And the same bytes through the testbench agree with `crc32fast`, so all three
    // -- simulator, Verilator and the crate that ships in zlib -- meet.
    let message: Vec<u8> = (0..64u8)
        .map(|index| index.wrapping_mul(37).wrapping_add(11))
        .collect();
    let mut hasher = golden();
    hasher.update(&message);
    assert_eq!(hardware(&message).1, u64::from(hasher.finalize()));
}
