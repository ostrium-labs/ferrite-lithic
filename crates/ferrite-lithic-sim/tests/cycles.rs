//! What a *cycle* means: the three phases, the two snapshots, and state.
//!
//! `tests/ops.rs` checks that every operation computes the right value. This file
//! checks the things that only exist between cycles — which is where the design's
//! structural decisions actually live. A simulator that gets every gate right and
//! commits registers in the wrong order is still wrong, and in a way no
//! single-operation test would notice.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use ferrite_lithic::Design;
use ferrite_lithic_bits::Bits;
use ferrite_lithic_sim::Sim;

/// A constant low, which is what an unasserted control line usually is.
fn low() -> Bits {
    Bits::constant(0, 1).expect("one bit")
}

/// A constant high.
fn high() -> Bits {
    Bits::constant(1, 1).expect("one bit")
}

#[test]
fn an_accumulator_counts_once_per_cycle() {
    let d = Design::new();
    let clk = d.input("clk", 1).unwrap();
    let acc = d.wire(8).unwrap();
    let next = d.add(&acc, &d.lit(1, 8).unwrap()).unwrap();
    let next = d
        .reg(&next, &clk, &d.constant(false), &d.constant(false))
        .unwrap();
    d.drive(&acc, &next).unwrap();
    d.output("acc", 8, &acc).unwrap();

    let mut sim = Sim::new(&d).unwrap();
    assert_eq!(sim.peek("acc").unwrap(), Bits::zeros(8).unwrap());
    for expected in 1..=5u64 {
        let (_before, after) = sim.step().unwrap();
        assert_eq!(
            after.get("acc").unwrap().to_u64().unwrap(),
            expected,
            "cycle {expected}"
        );
    }
}

#[test]
fn the_before_and_after_snapshots_straddle_the_edge() {
    let d = Design::new();
    let clk = d.input("clk", 1).unwrap();
    let acc = d.wire(8).unwrap();
    let next = d.add(&acc, &d.lit(1, 8).unwrap()).unwrap();
    let next = d
        .reg(&next, &clk, &d.constant(false), &d.constant(false))
        .unwrap();
    d.drive(&acc, &next).unwrap();
    d.output("acc", 8, &acc).unwrap();

    let mut sim = Sim::new(&d).unwrap();
    let (before, after) = sim.step().unwrap();

    // The distinction is the whole reason there are two snapshots: `before` is the
    // pre-edge value and `after` is the value the edge produced.
    assert_eq!(before.get("acc").unwrap().to_u64().unwrap(), 0);
    assert_eq!(after.get("acc").unwrap().to_u64().unwrap(), 1);
    assert_eq!(before.cycle(), 1);
    assert_eq!(after.cycle(), 1);
    assert_eq!(sim.cycle(), 1);
}

#[test]
fn the_phases_can_be_driven_one_at_a_time() {
    let d = Design::new();
    let clk = d.input("clk", 1).unwrap();
    let acc = d.wire(8).unwrap();
    let next = d.add(&acc, &d.lit(1, 8).unwrap()).unwrap();
    let next = d
        .reg(&next, &clk, &d.constant(false), &d.constant(false))
        .unwrap();
    d.drive(&acc, &next).unwrap();
    d.output("acc", 8, &acc).unwrap();

    let mut sim = Sim::new(&d).unwrap();
    let before = sim.before_clock_edge().unwrap();
    assert_eq!(before.get("acc").unwrap().to_u64().unwrap(), 0);
    sim.at_clock_edge().unwrap();
    let after = sim.after_clock_edge().unwrap();
    assert_eq!(after.get("acc").unwrap().to_u64().unwrap(), 1);
    assert_eq!(after.cycle(), 1);
}

#[test]
fn reset_clears_a_register_and_clear_holds_it() {
    // The pair is easy to swap, and a swapped pair produces a register that never
    // updates -- indistinguishable from a simulator that is not advancing.
    let d = Design::new();
    let clk = d.input("clk", 1).unwrap();
    let reset = d.input("reset", 1).unwrap();
    let hold = d.input("hold", 1).unwrap();
    let data = d.input("data", 8).unwrap();
    let reg = d.reg(&data, &clk, &reset, &hold).unwrap();
    d.output("q", 8, &reg).unwrap();

    let mut sim = Sim::new(&d).unwrap();
    sim.set_input("data", Bits::constant(0xaa, 8).unwrap())
        .unwrap();

    // A clock edge with clear asserted holds the register at zero.
    sim.set_input("hold", high()).unwrap();
    sim.step().unwrap();
    assert_eq!(sim.peek("q").unwrap().to_u64().unwrap(), 0);

    // Released, it takes the data input.
    sim.set_input("hold", low()).unwrap();
    sim.step().unwrap();
    assert_eq!(sim.peek("q").unwrap().to_u64().unwrap(), 0xaa);

    // Reset wins over both data and clear.
    sim.set_input("reset", high()).unwrap();
    sim.set_input("hold", high()).unwrap();
    sim.step().unwrap();
    assert_eq!(sim.peek("q").unwrap().to_u64().unwrap(), 0);
}

#[test]
fn a_register_starts_at_zero_and_can_be_given_a_starting_value() {
    let d = Design::new();
    let clk = d.input("clk", 1).unwrap();
    let data = d.input("data", 8).unwrap();
    let hold = d.input("hold", 1).unwrap();
    let reg = d.reg(&data, &clk, &d.constant(false), &hold).unwrap();
    d.output("q", 8, &reg).unwrap();

    let mut sim = Sim::new(&d).unwrap();
    assert_eq!(sim.peek("q").unwrap(), Bits::zeros(8).unwrap());

    sim.initialize_to(&reg, Bits::constant(0x5a, 8).unwrap())
        .unwrap();
    assert_eq!(sim.peek("q").unwrap().to_u64().unwrap(), 0x5a);

    // Holding the register across an edge is what makes this a test of the shadow
    // copy. Had only the live copy been written, the blit would bring in the
    // untouched shadow -- still zero -- and the register would silently lose its
    // starting value on the very first edge.
    sim.set_input("hold", high()).unwrap();
    sim.step().unwrap();
    assert_eq!(sim.peek("q").unwrap().to_u64().unwrap(), 0x5a);

    // Released, it takes its data input, which is still zero.
    sim.set_input("hold", low()).unwrap();
    sim.step().unwrap();
    assert_eq!(sim.peek("q").unwrap().to_u64().unwrap(), 0);
}

#[test]
fn an_initial_value_only_applies_to_a_register() {
    let d = Design::new();
    let clk = d.input("clk", 1).unwrap();
    let data = d.input("data", 8).unwrap();
    let reg = d
        .reg(&data, &clk, &d.constant(false), &d.constant(false))
        .unwrap();
    let mem = d.mem(8, 4).unwrap();
    let sum = d.add(&data, &data).unwrap();
    d.output("q", 8, &reg).unwrap();

    let mut sim = Sim::new(&d).unwrap();
    let err = sim
        .initialize_to(&mem, Bits::zeros(8).unwrap())
        .unwrap_err();
    assert!(
        matches!(
            err,
            ferrite_lithic_sim::Error::InitializeTarget { kind: "Mem" }
        ),
        "{err}"
    );

    let err = sim
        .initialize_to(&sum, Bits::zeros(16).unwrap())
        .unwrap_err();
    assert!(
        matches!(err, ferrite_lithic_sim::Error::InitializeTarget { .. }),
        "{err}"
    );
}

#[test]
fn a_chain_of_registers_shifts_by_one_cycle_not_one_per_register() {
    // The reason for the shadow section. Writing each register's next value in
    // place would make register 2 see register 1's *new* value, so the chain would
    // move two bits per cycle instead of one.
    let d = Design::new();
    let clk = d.input("clk", 1).unwrap();
    let first = d.input("in", 8).unwrap();
    let second = d
        .reg(&first, &clk, &d.constant(false), &d.constant(false))
        .unwrap();
    let third = d
        .reg(&second, &clk, &d.constant(false), &d.constant(false))
        .unwrap();
    d.output("q2", 8, &second).unwrap();
    d.output("q3", 8, &third).unwrap();

    let mut sim = Sim::new(&d).unwrap();
    sim.set_input("in", Bits::constant(1, 8).unwrap()).unwrap();
    for _ in 0..3 {
        sim.step().unwrap();
    }
    assert_eq!(sim.peek("q2").unwrap().to_u64().unwrap(), 1);
    assert_eq!(sim.peek("q3").unwrap().to_u64().unwrap(), 1);

    sim.set_input("in", Bits::constant(0, 8).unwrap()).unwrap();
    sim.step().unwrap();
    assert_eq!(sim.peek("q2").unwrap().to_u64().unwrap(), 0);
    assert_eq!(sim.peek("q3").unwrap().to_u64().unwrap(), 1);
}

#[test]
fn a_combinational_feedback_loop_through_a_register_is_legal() {
    // `Reg` is terminal for loop checking, so this is not a loop at all. It is the
    // shape every accumulator and every saturating counter has.
    let d = Design::new();
    let clk = d.input("clk", 1).unwrap();
    let acc = d.wire(8).unwrap();
    // Saturating, not wrapping: holding at the maximum is what makes a
    // stuck-at-maximum distinguishable from a counter that is still counting.
    let full = d.lit(0xff, 8).unwrap();
    let incremented = d.add(&acc, &d.lit(1, 8).unwrap()).unwrap();
    let next = d
        .ite(&d.eq(&acc, &full).unwrap(), &full, &incremented)
        .unwrap();
    let next = d
        .reg(&next, &clk, &d.constant(false), &d.constant(false))
        .unwrap();
    d.drive(&acc, &next).unwrap();
    d.output("acc", 8, &acc).unwrap();

    let mut sim = Sim::new(&d).unwrap();
    for _ in 0..255 {
        sim.step().unwrap();
    }
    assert_eq!(sim.peek("acc").unwrap().to_u64().unwrap(), 255);
    for _ in 0..45 {
        sim.step().unwrap();
    }
    // 45 further edges change nothing: the loop has settled, not oscillated.
    assert_eq!(sim.peek("acc").unwrap().to_u64().unwrap(), 255);
}

#[test]
fn a_register_free_design_settles_without_a_clock() {
    let d = Design::new();
    let a = d.input("a", 8).unwrap();
    let b = d.input("b", 8).unwrap();
    let out = d.xor(&a, &b).unwrap();
    d.output("out", 8, &out).unwrap();

    let mut sim = Sim::new(&d).unwrap();
    sim.set_input("a", Bits::constant(0b1100, 8).unwrap())
        .unwrap();
    sim.set_input("b", Bits::constant(0b1010, 8).unwrap())
        .unwrap();
    sim.comb().unwrap();
    assert_eq!(sim.peek("out").unwrap().to_u64().unwrap(), 0b0110);

    // No register means no clock, and saying so is better than inventing one.
    let err = sim.step().unwrap_err();
    assert!(matches!(err, ferrite_lithic_sim::Error::NoClock), "{err}");
}

#[test]
fn two_clocks_are_refused_rather_than_simulated_as_one() {
    let d = Design::new();
    let clk_a = d.input("clk_a", 1).unwrap();
    let clk_b = d.input("clk_b", 1).unwrap();
    let data = d.input("data", 8).unwrap();
    let _a = d
        .reg(&data, &clk_a, &d.constant(false), &d.constant(false))
        .unwrap();
    let _b = d
        .reg(&data, &clk_b, &d.constant(false), &d.constant(false))
        .unwrap();
    d.output("q", 8, &_a).unwrap();

    let err = Sim::new(&d).unwrap_err();
    assert!(
        matches!(err, ferrite_lithic_sim::Error::MultipleClocks { .. }),
        "{err}"
    );
    assert!(err.to_string().contains("clk_a"), "{err}");
    assert!(err.to_string().contains("clk_b"), "{err}");
}

#[test]
fn an_unmodelled_instance_is_refused() {
    let d = Design::new();
    let a = d.input("a", 8).unwrap();
    let out = d
        .instance(
            "fifo",
            &[("DEPTH", Bits::constant(4, 32).unwrap())],
            &[a],
            8,
        )
        .unwrap();
    d.output("q", 8, &out).unwrap();

    let err = Sim::new(&d).unwrap_err();
    assert!(
        matches!(&err, ferrite_lithic_sim::Error::NoInstanceModel { name } if name == "fifo"),
        "{err}"
    );
}

#[test]
fn a_signal_from_another_design_is_refused() {
    let a_design = Design::new();
    let foreign = a_design.lit(1, 8).unwrap();

    let d = Design::new();
    let clk = d.input("clk", 1).unwrap();
    let data = d.input("data", 8).unwrap();
    let reg = d
        .reg(&data, &clk, &d.constant(false), &d.constant(false))
        .unwrap();
    d.output("q", 8, &reg).unwrap();

    let sim = Sim::new(&d).unwrap();
    // A node id from another arena would alias an unrelated node here and produce a
    // simulation of a circuit the user never wrote.
    let err = sim.word_of(&foreign).unwrap_err();
    assert!(
        matches!(err, ferrite_lithic_sim::Error::ForeignSignal { .. }),
        "{err}"
    );
}

#[test]
fn an_unknown_port_name_lists_the_ports_that_exist() {
    let d = Design::new();
    let clk = d.input("clk", 1).unwrap();
    let data = d.input("data", 8).unwrap();
    let reg = d
        .reg(&data, &clk, &d.constant(false), &d.constant(false))
        .unwrap();
    d.output("q", 8, &reg).unwrap();

    let mut sim = Sim::new(&d).unwrap();
    let err = sim
        .set_input("clc", Bits::constant(0, 1).unwrap())
        .unwrap_err();
    let message = err.to_string();
    assert!(message.contains("no input port named `clc`"), "{message}");
    assert!(message.contains("clk, data"), "{message}");
}

#[test]
fn a_value_of_the_wrong_width_is_refused_with_both_widths() {
    let d = Design::new();
    let clk = d.input("clk", 1).unwrap();
    let data = d.input("data", 8).unwrap();
    let reg = d
        .reg(&data, &clk, &d.constant(false), &d.constant(false))
        .unwrap();
    d.output("q", 8, &reg).unwrap();

    let mut sim = Sim::new(&d).unwrap();
    let err = sim
        .set_input("data", Bits::constant(1, 12).unwrap())
        .unwrap_err();
    let message = err.to_string();
    assert!(message.contains("12 bits"), "{message}");
    assert!(message.contains("8 bits"), "{message}");
}

#[test]
fn dividing_by_zero_is_reported_rather_than_answered() {
    let d = Design::new();
    let num = d.input("num", 8).unwrap();
    let den = d.input("den", 8).unwrap();
    let out = d.udiv(&num, &den).unwrap();
    d.output("out", 8, &out).unwrap();

    let mut sim = Sim::new(&d).unwrap();
    sim.set_input("num", Bits::constant(10, 8).unwrap())
        .unwrap();
    sim.set_input("den", Bits::constant(0, 8).unwrap()).unwrap();
    let err = sim.comb().unwrap_err();
    assert!(
        matches!(err, ferrite_lithic_sim::Error::DivisionByZero { .. }),
        "{err}"
    );

    // A nonzero divisor is fine, which is what makes the refusal about the design
    // rather than about the operation.
    sim.set_input("den", Bits::constant(3, 8).unwrap()).unwrap();
    sim.comb().unwrap();
    assert_eq!(sim.peek("out").unwrap().to_u64().unwrap(), 3);
}

#[test]
fn signed_division_overflow_is_reported() {
    let d = Design::new();
    let num = d.input("num", 8).unwrap();
    let den = d.input("den", 8).unwrap();
    let out = d.sdiv(&num, &den).unwrap();
    d.output("out", 8, &out).unwrap();

    let mut sim = Sim::new(&d).unwrap();
    // -128 / -1 has no 8-bit result.
    sim.set_input("num", Bits::constant(0b1000_0000, 8).unwrap())
        .unwrap();
    sim.set_input("den", Bits::constant(0b1111_1111, 8).unwrap())
        .unwrap();
    let err = sim.comb().unwrap_err();
    assert!(
        matches!(
            err,
            ferrite_lithic_sim::Error::SignedDivisionOverflow { width: 8 }
        ),
        "{err}"
    );

    // The same shape as a remainder *can* be represented, so it answers zero
    // rather than reporting an overflow. In a design of its own, because the
    // dividing node above is still in this graph and still overflows.
    let d = Design::new();
    let num = d.input("num", 8).unwrap();
    let den = d.input("den", 8).unwrap();
    let rem = d.srem(&num, &den).unwrap();
    d.output("rem", 8, &rem).unwrap();
    let mut sim = Sim::new(&d).unwrap();
    sim.set_input("num", Bits::constant(0b1000_0000, 8).unwrap())
        .unwrap();
    sim.set_input("den", Bits::constant(0b1111_1111, 8).unwrap())
        .unwrap();
    sim.comb().unwrap();
    assert_eq!(sim.peek("rem").unwrap().to_u64().unwrap(), 0);
}

#[test]
fn a_wire_is_free_at_runtime_and_reads_as_its_driver() {
    let d = Design::new();
    let a = d.input("a", 8).unwrap();
    let direct = d.add(&a, &a).unwrap();
    let wire = d.wire(8).unwrap();
    let through_wire = d.add(&wire, &wire).unwrap();
    d.drive(&wire, &direct).unwrap();
    d.output("direct", 8, &direct).unwrap();
    d.output("through_wire", 8, &through_wire).unwrap();

    let mut sim = Sim::new(&d).unwrap();
    sim.set_input("a", Bits::constant(3, 8).unwrap()).unwrap();
    sim.comb().unwrap();
    assert_eq!(sim.peek("direct").unwrap().to_u64().unwrap(), 6);
    assert_eq!(sim.peek("through_wire").unwrap().to_u64().unwrap(), 12);

    // The aliasing is visible in the addresses: the wire and its driver share one
    // slot, which is why the op list never mentions a wire.
    assert_eq!(sim.word_of(&wire).unwrap(), sim.word_of(&direct).unwrap());
    for op in sim.program().ops() {
        let rendered = format!("{op:?}");
        assert!(!rendered.contains("Wire"), "{rendered}");
    }
}
