//! The parts of the cosimulator that need no vendor toolchain.
//!
//! Every check here runs whether or not `verilator` is installed, because these
//! are the parts with the arithmetic in them. `equivalence.rs` is the one that
//! needs Verilator, and it announces its skip rather than passing quietly.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use ferrite_lithic::{Design, Signal};
use ferrite_lithic_bits::Bits;
use ferrite_lithic_cosim::{Error, Plan, Port, Stimulus, compare, driver_source, generator};
use ferrite_lithic_derive::PortList;

/// An 8-bit accumulator: the smallest design with a clock, a register and an
/// output, which is the shape every interesting disagreement about a cycle
/// boundary shows up in.
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
    let in_ports = ferrite_lithic::inputs::<AccInputs>(&design).unwrap();
    let state = design.wire(8).unwrap();
    let next = design.add(&state, &in_ports.d).unwrap();
    let held = design
        .reg(&next, &in_ports.clk, &in_ports.rst, &in_ports.clr)
        .unwrap();
    design.drive(&state, &held).unwrap();
    let out_ports =
        ferrite_lithic::outputs::<AccOutputs>(&design, &AccOutputs { q: held }).unwrap();

    let inputs = vec![
        Port {
            name: "clk".to_string(),
            width: 1,
        },
        Port {
            name: "rst".to_string(),
            width: 1,
        },
        Port {
            name: "clr".to_string(),
            width: 1,
        },
        Port {
            name: "d".to_string(),
            width: 8,
        },
    ];
    let outputs = vec![Port {
        name: "q".to_string(),
        width: 8,
    }];
    let plan = Plan {
        top: "accumulator".to_string(),
        clock: Some("clk".to_string()),
        inputs,
        outputs,
    };
    assert_eq!(in_ports.to_signals().len(), 4);
    assert_eq!(out_ports.to_signals().len(), 1);
    (design, plan)
}

#[test]
fn the_simulator_side_runs_the_stimulus_and_produces_one_row_per_cycle() {
    let (design, plan) = accumulator();
    let mut stimulus = Stimulus::new();
    // clk low, rst low, clr low, d=5. The clock column is present because the
    // module declares it; both backends drive the clock themselves.
    stimulus
        .push(vec![
            Bits::zeros(1).unwrap(),
            Bits::zeros(1).unwrap(),
            Bits::zeros(1).unwrap(),
            Bits::constant(5, 8).unwrap(),
        ])
        .unwrap();
    for _ in 0..3 {
        stimulus
            .push(vec![
                Bits::ones(1).unwrap(),
                Bits::zeros(1).unwrap(),
                Bits::zeros(1).unwrap(),
                Bits::constant(0, 8).unwrap(),
            ])
            .unwrap();
    }

    let observed = ferrite_lithic_cosim::simulate(&design, &plan, &stimulus).unwrap();
    assert_eq!(observed.len(), 4);
    // `Sim::step` drives the clock itself -- high, then low -- so one step *is*
    // one rising edge, whatever the stimulus's clock column says. That is why
    // the first row is already 5: the edge it applied was the first one.
    assert_eq!(observed[0][0].to_u64().unwrap(), 5, "after the first edge");
    assert_eq!(
        observed[1][0].to_u64().unwrap(),
        5,
        "d was zero, so it holds"
    );
    assert_eq!(observed[2][0].to_u64().unwrap(), 5);
}

#[test]
fn a_stimulus_row_of_the_wrong_length_is_refused_at_push_time() {
    let mut stimulus = Stimulus::new();
    stimulus.push(vec![Bits::zeros(1).unwrap()]).unwrap();
    let error = stimulus
        .push(vec![Bits::zeros(1).unwrap(), Bits::zeros(1).unwrap()])
        .unwrap_err();
    assert!(
        matches!(
            error,
            Error::Arity {
                cycle: 1,
                expected: 1,
                got: 2
            }
        ),
        "{error}"
    );
    assert!(error.to_string().contains("cycle 1"), "{error}");
}

#[test]
fn a_value_of_the_wrong_width_is_refused_when_it_becomes_text() {
    let mut stimulus = Stimulus::new();
    stimulus
        .push(vec![Bits::constant(0xff, 8).unwrap()])
        .unwrap();

    let error = stimulus.encode(&[4]).unwrap_err();
    match error {
        Error::Width {
            cycle,
            port,
            expected,
            got,
        } => {
            assert_eq!((cycle, port, expected, got), (0, 0, 4, 8));
        }
        other => panic!("expected a width error, got {other}"),
    }
}

#[test]
fn hex_words_are_fixed_width_so_a_reader_knows_the_nibble_count() {
    assert_eq!(
        ferrite_lithic_cosim::stimulus::hex(&Bits::constant(1, 8).unwrap()),
        "01"
    );
    assert_eq!(
        ferrite_lithic_cosim::stimulus::hex(&Bits::constant(0xa5, 8).unwrap()),
        "a5"
    );
    assert_eq!(
        ferrite_lithic_cosim::stimulus::hex(&Bits::constant(1, 12).unwrap()),
        "001",
        "three nibbles for twelve bits"
    );
    assert_eq!(
        ferrite_lithic_cosim::stimulus::hex(&Bits::zeros(1).unwrap()),
        "0",
        "one bit still takes a nibble, so the width is unambiguous"
    );
}

#[test]
fn a_hex_word_survives_the_round_trip_through_text() {
    for width in [1u32, 3, 8, 17, 33, 64] {
        let value = Bits::constant(0xdead_beef_cafe_babe, width)
            .unwrap()
            .to_hex()
            .trim_start_matches("0x")
            .to_string();
        let word = value
            .get(..(width as usize).div_ceil(4))
            .unwrap_or("0")
            .to_string();
        let back = ferrite_lithic_cosim::stimulus::parse_hex(&word, width).unwrap();
        assert_eq!(back.width(), width, "width {width}");
    }
}

#[test]
fn a_word_that_is_not_hex_is_refused_by_name() {
    let error = ferrite_lithic_cosim::stimulus::parse_hex("nope", 8).unwrap_err();
    assert!(matches!(error, Error::HexWord { .. }), "{error}");
    assert!(error.to_string().contains("nope"), "{error}");
}

#[test]
fn a_word_wider_than_its_port_is_refused_rather_than_truncated() {
    let error = ferrite_lithic_cosim::stimulus::parse_hex("ffff", 4).unwrap_err();
    assert!(matches!(error, Error::Width { .. }), "{error}");
}

#[test]
fn comparing_two_runs_that_agree_reports_how_much_they_covered() {
    let names = vec!["q".to_string()];
    let a = vec![
        vec![Bits::constant(1, 8).unwrap()],
        vec![Bits::constant(2, 8).unwrap()],
    ];
    let report = compare(&names, &a, &a.clone()).unwrap();

    assert!(report.is_equivalent());
    assert_eq!(report.cycles, 2);
    assert!(report.first().is_none());
    assert_eq!(report.to_string(), "equivalent over 2 cycle(s)");
}

#[test]
fn a_divergence_is_reported_with_both_values_and_both_backends() {
    let names = vec!["q".to_string()];
    let simulator = vec![
        vec![Bits::constant(1, 8).unwrap()],
        vec![Bits::constant(2, 8).unwrap()],
        vec![Bits::constant(3, 8).unwrap()],
    ];
    let verilator = vec![
        vec![Bits::constant(1, 8).unwrap()],
        vec![Bits::constant(9, 8).unwrap()],
        vec![Bits::constant(3, 8).unwrap()],
    ];
    let report = compare(&names, &simulator, &verilator).unwrap();

    assert!(!report.is_equivalent());
    let first = report.first().unwrap();
    assert_eq!(first.cycle, 1);
    assert_eq!(first.port, "q");
    assert_eq!(first.simulator.to_u64().unwrap(), 2);
    assert_eq!(first.verilator.to_u64().unwrap(), 9);

    let text = first.to_string();
    assert!(text.contains("ferrite-lithic-sim"), "{text}");
    assert!(text.contains("verilator"), "{text}");
}

#[test]
fn a_run_that_stopped_early_is_not_a_pass_on_the_common_prefix() {
    let names = vec!["q".to_string()];
    let simulator = vec![
        vec![Bits::constant(1, 8).unwrap()],
        vec![Bits::constant(2, 8).unwrap()],
    ];
    let verilator = vec![vec![Bits::constant(1, 8).unwrap()]];

    let error = compare(&names, &simulator, &verilator).unwrap_err();
    assert!(
        matches!(
            error,
            Error::Arity {
                expected: 2,
                got: 1,
                ..
            }
        ),
        "{error}"
    );
    assert!(error.to_string().contains("2"), "{error}");
}

#[test]
fn a_cycle_with_the_wrong_number_of_outputs_is_refused() {
    let names = vec!["a".to_string(), "b".to_string()];
    let simulator = vec![vec![Bits::zeros(1).unwrap(), Bits::zeros(1).unwrap()]];
    let verilator = vec![vec![Bits::zeros(1).unwrap()]];
    assert!(matches!(
        compare(&names, &simulator, &verilator),
        Err(Error::Arity { .. })
    ));
}

#[test]
fn the_generated_driver_drives_the_clock_low_applies_inputs_then_reads_after_the_edge() {
    let (_, plan) = accumulator();
    let source = driver_source(&plan);

    assert!(source.contains("#include \"Vaccumulator.h\""), "{source}");
    assert!(source.contains("VerilatedContext context;"), "{source}");
    assert!(source.contains("dut->clk = 0;"), "{source}");

    // The order is the whole contract with the simulator: low, inputs, high, read.
    let low = source
        .find("dut->clk = 0;\n    dut->eval();")
        .expect("low phase");
    let drive = source.find("dut->d = std::strtoull").expect("input drive");
    let high = source
        .rfind("dut->clk = 1;\n    dut->eval();")
        .expect("high phase");
    let read = source.find("dut->q &").expect("output read");
    assert!(low < drive, "{source}");
    assert!(drive < high, "{source}");
    assert!(high < read, "{source}");
}

#[test]
fn the_generated_driver_declares_the_reset_state_it_is_checking_against() {
    let (_, plan) = accumulator();
    let source = driver_source(&plan);
    assert!(
        source.contains("Registers start at zero"),
        "the driver says why it may assume a zero reset: {source}"
    );
}

#[test]
fn a_combinational_module_gets_a_settle_instead_of_an_edge() {
    let plan = Plan {
        top: "comb".to_string(),
        clock: None,
        inputs: vec![Port {
            name: "d".to_string(),
            width: 8,
        }],
        outputs: vec![Port {
            name: "q".to_string(),
            width: 8,
        }],
    };
    let source = driver_source(&plan);
    assert!(!source.contains("dut->clk"), "{source}");
    assert!(source.contains("dut->eval();"), "{source}");
}

#[test]
fn the_driver_masks_every_port_to_its_declared_width() {
    let (_, plan) = accumulator();
    let source = driver_source(&plan);
    assert!(
        source.contains("dut->d = std::strtoull(word, nullptr, 16) & mask<8>"),
        "{source}"
    );
    assert!(source.contains("dut->q & mask<8>"), "{source}");
    assert!(
        source.contains("mask<1>"),
        "the clock is masked too: {source}"
    );
}

#[test]
fn the_driver_refuses_a_port_wider_than_it_can_drive_rather_than_truncating() {
    // A `static_assert` in the generated C++ rather than a runtime check: the
    // failure should be a compile error in the harness, where the stack trace
    // points at the design, not a divergence three cycles later.
    let source = driver_source(&Plan {
        top: "wide".to_string(),
        clock: Some("clk".to_string()),
        inputs: vec![Port {
            name: "d".to_string(),
            width: 8,
        }],
        outputs: vec![Port {
            name: "q".to_string(),
            width: 8,
        }],
    });
    assert!(source.contains("static_assert(W <= 64"), "{source}");
    assert!(
        source.contains("ports up to 64 bits wide"),
        "and says which port widths it will drive: {source}"
    );
}

#[test]
fn a_plan_reads_its_ports_from_the_design_it_will_emit() {
    let (design, _) = accumulator();
    let module = ferrite_lithic_rtl::Module::new(
        "accumulator",
        design.build().unwrap(),
        design.input_ports()[0].id(),
        design.input_ports().iter().map(|s| s.id()).collect(),
        design.output_ports().iter().map(|s| s.id()).collect(),
    );
    let plan = Plan::of(&design, &module);

    assert_eq!(plan.top, "accumulator");
    assert_eq!(plan.clock.as_deref(), Some("clk"));
    assert_eq!(
        plan.inputs
            .iter()
            .map(|p| p.name.as_str())
            .collect::<Vec<_>>(),
        ["clk", "rst", "clr", "d"]
    );
    assert_eq!(plan.output_names(), ["q"]);
    assert_eq!(plan.input_widths(), [1, 1, 1, 8]);
}

#[test]
fn a_plan_whose_clock_is_not_a_declared_input_is_refused_by_name() {
    let (design, mut plan) = accumulator();
    plan.clock = Some("nope".to_string());
    let error = plan.clock_id(&design).unwrap_err();
    assert!(matches!(error, Error::ClockNotDeclared { .. }), "{error}");
    assert!(error.to_string().contains("rst"), "{error}");
}

#[test]
fn a_plan_with_no_clock_emits_a_combinational_module() {
    let design = Design::new();
    let a = design.input("a", 4).unwrap();
    let b = design.input("b", 4).unwrap();
    let sum = design.add(&a, &b).unwrap();
    design.output("y", 4, &sum).unwrap();

    let plan = Plan {
        top: "adder".to_string(),
        clock: None,
        inputs: vec![Port {
            name: "a".to_string(),
            width: 4,
        }],
        outputs: vec![Port {
            name: "y".to_string(),
            width: 4,
        }],
    };
    // `b` is declared by the design but not in the plan: the plan says which
    // ports the driver drives, so the emitted module still declares it.
    let verilog = ferrite_lithic_cosim::emit(&design, &plan).unwrap();
    assert!(verilog.contains("module adder"), "{verilog}");
    assert!(verilog.contains("input  wire [3:0] a"), "{verilog}");
    assert!(verilog.contains("input  wire [3:0] b"), "{verilog}");
    assert_eq!(plan.clock_id(&design).unwrap(), None);
}

#[test]
fn a_generated_design_is_a_pure_function_of_its_seed() {
    let options = generator::Options::default();
    let one = generator::build(0x3ead_beef, &options).unwrap();
    let two = generator::build(0x3ead_beef, &options).unwrap();
    let three = generator::build(0x3ead_beef + 1, &options).unwrap();

    assert_eq!(
        ferrite_lithic_cosim::emit(&one.design, &plan_for(&one)).unwrap(),
        ferrite_lithic_cosim::emit(&two.design, &plan_for(&two)).unwrap(),
        "the same seed must produce the same design, or a failure is not reproducible"
    );
    assert_ne!(
        one.stimulus, three.stimulus,
        "and a different seed must produce a different one, or the generator is stuck"
    );
}

#[test]
fn more_than_two_thirds_of_generated_designs_have_a_non_constant_output() {
    // Hardcaml's generator-quality guard, kept for the same reason: a generator
    // that produced constants would make the differential test pass by never
    // disagreeing.
    let options = generator::Options::default();
    let mut varying = 0;
    let total = 40;
    for seed in 0..total as u64 {
        let generated = generator::build(seed * 0x9e37, &options).unwrap();
        let plan = plan_for(&generated);
        let observed =
            ferrite_lithic_cosim::simulate(&generated.design, &plan, &generated.stimulus).unwrap();
        let first = observed[0].clone();
        if observed.iter().any(|cycle| cycle != &first) {
            varying += 1;
        }
    }
    assert!(
        varying * 3 > total * 2,
        "only {varying} of {total} generated designs produced a changing output"
    );
}

#[test]
fn a_generated_design_at_every_hardcaml_width_builds_and_simulates() {
    // `1`, `2` and `3` are the widths that break implementations that assume a
    // whole word, and `64` is the fast path's edge. All four are in `WIDTHS`.
    for width in generator::WIDTHS {
        let options = generator::Options {
            width,
            depth: 8,
            ..generator::Options::default()
        };
        let generated = generator::build(0x3ead_beef, &options).unwrap();
        let plan = plan_for(&generated);
        let observed =
            ferrite_lithic_cosim::simulate(&generated.design, &plan, &generated.stimulus)
                .unwrap_or_else(|error| panic!("width {width} failed to simulate: {error}"));
        assert_eq!(observed.len(), generated.stimulus.cycles());
        assert_eq!(observed[0].len(), plan.outputs.len());
    }
}

#[test]
fn a_generated_design_wider_than_a_word_does_not_silently_lose_its_high_bits() {
    // `value` builds from words, so a 100-bit random value has random bits above
    // 64. If it did not, this design's output would be constant in the high half
    // and the guard above would pass for the wrong reason.
    let options = generator::Options {
        width: 100,
        ..generator::Options::default()
    };
    let generated = generator::build(0x3ead_beef, &options).unwrap();
    let mut high = 0;
    for row in generated.stimulus.cycle(0).unwrap().iter().skip(3) {
        if row.width() == 100 && row.bit(99).unwrap_or(false) {
            high += 1;
        }
    }
    assert!(
        high > 0,
        "no input had bit 99 set, so the width is not being exercised"
    );
}

#[test]
fn a_width_outside_the_generator_limits_is_refused() {
    let error = generator::build(
        1,
        &generator::Options {
            width: 0,
            ..generator::Options::default()
        },
    )
    .unwrap_err();
    assert!(matches!(error, Error::Arity { .. }), "{error}");
}

fn plan_for(generated: &generator::Generated) -> Plan {
    let inputs = generated
        .design
        .input_ports()
        .iter()
        .map(|signal| Port {
            name: signal.name().expect("a generated input is named"),
            width: signal.width(),
        })
        .collect();
    let outputs = generated
        .design
        .output_ports()
        .iter()
        .map(|signal| Port {
            name: signal.name().expect("a generated output is named"),
            width: signal.width(),
        })
        .collect();
    Plan {
        top: "random".to_string(),
        clock: Some("clk".to_string()),
        inputs,
        outputs,
    }
}
