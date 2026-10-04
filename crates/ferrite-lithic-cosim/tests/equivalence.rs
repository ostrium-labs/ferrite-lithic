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

    let plan = Plan {
        top: "accumulator".to_string(),
        clock: Some("clk".to_string()),
        inputs: ["clk", "rst", "clr", "d"]
            .into_iter()
            .zip([1u32, 1, 1, 8])
            .map(|(name, width)| Port {
                name: name.to_string(),
                width,
            })
            .collect(),
        outputs: vec![Port {
            name: "q".to_string(),
            width: 8,
        }],
    };
    (design, plan)
}

/// Sixteen cycles that toggle every control input at least once.
fn accumulator_stimulus() -> Stimulus {
    let mut stimulus = Stimulus::new();
    for cycle in 0..16u64 {
        stimulus
            .push(vec![
                Bits::constant(cycle % 2, 1).unwrap(),
                // Reset on cycles 5 and 11, clear on 3 and 13.
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
        let plan = Plan::of(&generated.design, &module_for(&generated.design, "random"));
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
