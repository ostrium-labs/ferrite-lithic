//! The equivalence check itself, when Verilator is installed.
//!
//! # Why this file can pass without having run anything
//!
//! `verilator` is not a build dependency and is not installed everywhere, so a
//! hard requirement would make the suite depend on a vendor toolchain. The same
//! trade as the Icarus gate in `ferrite-lithic-rtl`, and the same rule about
//! silence: the skip is printed to stdout, so a green run that checked nothing is
//! visible rather than hidden.
//!
//! Everything checkable without Verilator is in `harness.rs` — the stimulus
//! encoding, the generated driver, the comparison and its reporting — so this
//! file is the thin part: run both backends over one stimulus and require they
//! agree.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use ferrite_lithic::{Design, Signal};
use ferrite_lithic_bits::Bits;
use ferrite_lithic_cosim::{Harness, Plan, Port, Stimulus, generator, verilator};
use ferrite_lithic_derive::PortList;

/// An 8-bit accumulator with a gated reset and a gated hold.
///
/// Both gates are inputs, so the stimulus exercises the paths where a register
/// does *not* update. A cosimulation that only ever clocks registers forward
/// agrees with a broken register-update block.
#[derive(PortList)]
struct AccInputs {
    #[clock]
    clk: Signal,
    rst: Signal,
    clr: Signal,
    #[bits(8)]
    d: Signal,
}

#[derive(PortList)]
struct AccOutputs {
    #[bits(8)]
    q: Signal,
}

fn accumulator() -> (Design, Plan) {
    let design = Design::new();
    let ports = ferrite_lithic::inputs::<AccInputs>(&design).unwrap();
    let state = design.wire(8).unwrap();
    let next = design.add(&state, &ports.d).unwrap();
    let held = design
        .reg(&next, &ports.clk, &ports.rst, &ports.clr)
        .unwrap();
    design.drive(&state, &held).unwrap();
    ferrite_lithic::outputs::<AccOutputs>(&design, &AccOutputs { q: held }).unwrap();

    // `Plan::new` rather than a struct literal, and the clock is deliberately
    // absent from `inputs`: it is a port, but both backends drive it themselves.
    // This test could not tell the difference before Verilator was installed --
    // the driver applied `clk` from the stimulus and then raised it again, so a
    // stimulus word of 1 meant no edge at all and no register ever updated. Both
    // backends then "agreed" on a design that never ran.
    let plan = Plan::new(
        "accumulator",
        Some("clk".to_string()),
        ["rst", "clr", "d"]
            .into_iter()
            .zip([1u32, 1, 8])
            .map(|(name, width)| Port::new(name, width))
            .collect(),
        vec![Port::new("q", 8)],
    )
    .unwrap();
    (design, plan)
}

/// Sixteen cycles that toggle every control input at least once.
fn accumulator_stimulus() -> Stimulus {
    let mut stimulus = Stimulus::new();
    for cycle in 0..16u64 {
        stimulus
            .push(vec![
                // `rst`: high on cycles 5 and 11. `clr`: high on 3 and 13.
                // Both gates are exercised so a cosimulation that only ever
                // clocks registers forward cannot pass.
                Bits::constant(u64::from(cycle == 5 || cycle == 11), 1).unwrap(),
                Bits::constant(u64::from(cycle == 3 || cycle == 13), 1).unwrap(),
                Bits::constant(cycle * 7, 8).unwrap(),
            ])
            .unwrap();
    }
    stimulus
}

#[test]
fn the_simulator_and_the_emitted_verilog_agree_on_an_accumulator() {
    let Some(verilator) = verilator() else {
        println!(
            "SKIPPED: verilator not found, so the accumulator equivalence did not run. \
             The parts that need no toolchain are in tests/harness.rs."
        );
        return;
    };
    println!("cosimulating with {}", verilator.display());

    let (design, plan) = accumulator();
    let stimulus = accumulator_stimulus();
    let verilog = ferrite_lithic_cosim::emit(&design, &plan).unwrap();
    let harness = Harness::build(&plan, &verilog, &plan.work_dir()).unwrap();

    let report = ferrite_lithic_cosim::run_with(&design, &plan, &stimulus, &harness).unwrap();
    assert!(
        report.is_equivalent(),
        "the two backends disagree: {report}\nfirst: {:?}",
        report.first()
    );
    assert_eq!(report.cycles, 16);
}

#[test]
fn random_designs_agree_between_the_two_backends() {
    let Some(verilator) = verilator() else {
        println!("SKIPPED: verilator not found, so the random-design differential did not run.");
        return;
    };
    println!("cosimulating random designs with {}", verilator.display());

    // Few seeds on purpose: each one is a full Verilator build, which is seconds
    // rather than milliseconds. The generator's own quality is checked without
    // Verilator in `harness.rs`, so these seeds are for the differential, not for
    // proving the generator works.
    for seed in 0..3u64 {
        let options = generator::Options::default();
        let generated = generator::build(0x3ead_beef + seed, &options).unwrap();
        let plan = Plan::of(&generated.design, &module_for(&generated.design, "random")).unwrap();
        let verilog = ferrite_lithic_cosim::emit(&generated.design, &plan).unwrap();
        let harness = Harness::build(&plan, &verilog, &plan.work_dir()).unwrap();
        let report =
            ferrite_lithic_cosim::run_with(&generated.design, &plan, &generated.stimulus, &harness)
                .unwrap_or_else(|error| panic!("seed {seed:#x}: {error}"));
        assert!(report.is_equivalent(), "seed {seed:#x}: {report}");
    }
}

fn module_for(design: &Design, name: &str) -> ferrite_lithic_rtl::Module {
    let clock = design
        .input_ports()
        .iter()
        .find(|signal| signal.name().as_deref() == Some("clk"))
        .expect("a generated design has a clock")
        .id();
    ferrite_lithic_rtl::Module::new(
        name,
        design.build().unwrap(),
        clock,
        design.input_ports().iter().map(|s| s.id()).collect(),
        design.output_ports().iter().map(|s| s.id()).collect(),
    )
}

/// Two harnesses built from the same plan must not share a directory.
///
/// They used to. `Plan::work_dir` was keyed only by module name, so anything
/// cosimulating the same top module wrote the same `dut.v`, the same `main.cpp` and
/// the same `obj/`, and `run` rewrote a shared `stimulus.txt` per stimulus. The
/// common symptom is a permission error or a link failure; the one that matters is
/// silent, because `is_equivalent()` then answers about a stimulus the caller never
/// passed.
#[test]
fn two_harnesses_for_the_same_module_do_not_share_a_directory() {
    let Some(verilator) = verilator() else {
        println!("SKIPPED: verilator not found, so the shared-directory check did not run.");
        return;
    };
    println!("checking harness isolation with {}", verilator.display());

    let (design, plan) = accumulator();
    let verilog = ferrite_lithic_cosim::emit(&design, &plan).unwrap();
    // The *same* root for both, which is exactly the case that used to collide.
    let root = plan.work_dir();
    let first = Harness::build(&plan, &verilog, &root).unwrap();
    let second = Harness::build(&plan, &verilog, &root).unwrap();

    assert_ne!(
        first.root(),
        second.root(),
        "two builds must get separate object trees and separate stimulus files"
    );
    for harness in [&first, &second] {
        assert!(
            harness.root().starts_with(&root),
            "and both stay under the directory the caller asked for: {}",
            harness.root().display()
        );
    }
}

/// A port named after a C++ keyword, driven by the generated driver.
///
/// `char` is legal Verilog and illegal C++, and the generated driver assigns
/// `dut->char`. Verilator names the class member from the Verilog name verbatim, so
/// the driver cannot rename its way out of it — the emitter has to escape the name,
/// and `Plan::of` has to agree about the spelling. Both used to be unchecked by
/// anything, and the failure appeared as a C++ syntax error two crates away, in a
/// generated file, about a port name nobody thought of as a keyword in the language
/// being emitted.
#[test]
fn a_port_named_after_a_cxx_keyword_drives_and_simulates() {
    let Some(verilator) = verilator() else {
        println!("SKIPPED: verilator not found, so the cxx-keyword port did not run.");
        return;
    };
    println!(
        "cosimulating a port named `char` with {}",
        verilator.display()
    );

    let d = Design::new();
    let value = d.input("char", 8).unwrap();
    let doubled = d.add(&value, &value).unwrap();
    d.output("char_echo", doubled.width(), &doubled).unwrap();
    let module = ferrite_lithic_rtl::Module::combinational(
        "cxx_keyword",
        d.build().unwrap(),
        d.input_ports().iter().map(|s| s.id()).collect(),
        d.output_ports().iter().map(|s| s.id()).collect(),
    );

    let verilog = module.emit().unwrap();
    assert!(
        verilog.contains("input  wire [7:0] char_"),
        "the emitter escapes it: {verilog}"
    );

    let plan = Plan::of(&d, &module).unwrap();
    assert_eq!(
        plan.inputs
            .iter()
            .map(|p| p.verilog.as_str())
            .collect::<Vec<_>>(),
        ["char_"],
        "and the plan gives the driver the escaped spelling"
    );
    assert_eq!(
        plan.inputs
            .iter()
            .map(|p| p.name.as_str())
            .collect::<Vec<_>>(),
        ["char"],
        "while the simulator keeps the design's own name"
    );

    let mut stimulus = Stimulus::new();
    for byte in 0..=255u64 {
        stimulus
            .push(vec![Bits::constant(byte, 8).unwrap()])
            .unwrap();
    }
    let harness = Harness::build(&plan, &verilog, &plan.work_dir()).unwrap();
    let report = ferrite_lithic_cosim::run_with(&d, &plan, &stimulus, &harness).unwrap();
    assert!(
        report.is_equivalent(),
        "the two backends disagree: {report}\nfirst: {:?}",
        report.first()
    );
}
