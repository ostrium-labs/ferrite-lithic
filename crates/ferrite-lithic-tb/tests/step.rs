//! What a step testbench promises, and what it refuses.
//!
//! The promises are mostly about *where the cycle boundary falls*, because that is
//! the thing a testbench exists to make unambiguous: a `.await` is one rising edge,
//! a read afterwards sees its effect, and the count of `.await`s is the count of
//! edges. Getting any of those wrong produces a test that passes or fails for a
//! reason nobody can see, so each one is checked directly rather than inferred from
//! a design's behaviour.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use ferrite_lithic::{Design, Signal};
use ferrite_lithic_derive::PortList;
use ferrite_lithic_tb::{Error, Testbench};

/// A gated accumulator: `q` is `q + d` every cycle, reset and cleared.
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

fn accumulator() -> Design {
    let design = Design::new();
    let ports = ferrite_lithic::inputs::<AccInputs>(&design).unwrap();
    let state = design.wire(8).unwrap();
    let next = design.add(&state, &ports.d).unwrap();
    let held = design
        .reg(&next, &ports.clk, &ports.rst, &ports.clr)
        .unwrap();
    design.drive(&state, &held).unwrap();
    ferrite_lithic::outputs::<AccOutputs>(&design, &AccOutputs { q: held }).unwrap();
    design
}

/// A register-free design, to check that a cycle without a clock is a settle.
#[derive(PortList)]
struct CombInputs {
    #[bits(8)]
    a: Signal,
    #[bits(8)]
    b: Signal,
}

#[derive(PortList)]
struct WideOutputs {
    // `add` is the width of its left operand, so a hundred-bit add is a hundred bits.
    #[bits(100)]
    sum: Signal,
}

#[derive(PortList)]
struct CombOutputs {
    #[bits(9)]
    sum: Signal,
}

fn adder() -> Design {
    let design = Design::new();
    let ports = ferrite_lithic::inputs::<CombInputs>(&design).unwrap();
    let wide_a = design.zero_extend(&ports.a, 9).unwrap();
    let wide_b = design.zero_extend(&ports.b, 9).unwrap();
    let sum = design.add(&wide_a, &wide_b).unwrap();
    ferrite_lithic::outputs::<CombOutputs>(&design, &CombOutputs { sum }).unwrap();
    design
}

#[test]
fn every_await_is_exactly_one_rising_edge() {
    let design = accumulator();
    let tb = Testbench::new(&design).unwrap();
    let (edges, awaits) = tb
        .run(|tb| async move {
            let before = tb.cycle();
            tb.drive("rst", 0).await;
            tb.drive("clr", 0).await;
            tb.drive("d", 1).await;
            tb.drive("d", 2).await;
            tb.drive("d", 3).await;
            (tb.cycle() - before, 5)
        })
        .unwrap();
    assert_eq!(edges, awaits, "the cycle count is the number of awaits");
    assert_eq!(tb.cycle(), 5);
}

#[test]
fn a_read_after_an_await_sees_that_edge() {
    // The boundary is *after* the edge, like the cosimulator (ADR-0018). A read that
    // saw the pre-edge value would make every assertion in every test one step late,
    // and the first line of a test would have to assert about a cycle it did not ask
    // for.
    let design = accumulator();
    let tb = Testbench::new(&design).unwrap();
    tb.run(|tb| async move {
        tb.drive("rst", 0).await;
        tb.drive("clr", 0).await;
        tb.drive("d", 5).await;
        assert_eq!(tb.value("q"), 5, "the edge just taken is already visible");
        tb.drive("d", 7).await;
        assert_eq!(tb.value("q"), 12, "0 + 5, then 5 + 7");
    })
    .unwrap();
}

#[test]
fn a_pulse_takes_two_edges() {
    // Asserted for one edge, released for one. A reset that existed only *between*
    // two samples would be a reset a recorded waveform cannot show happening, which
    // is the same reason the simulator's snapshot is taken after the edge.
    let design = accumulator();
    let tb = Testbench::new(&design).unwrap();
    tb.run(|tb| async move {
        tb.drive("rst", 0).await;
        tb.drive("clr", 0).await;
        tb.pulse("rst").await;
    })
    .unwrap();
    // Two drives and a two-edge pulse.
    assert_eq!(tb.cycle(), 4);
}

#[test]
fn the_reset_is_visible_on_the_edge_that_asserts_it() {
    // Driven edge by edge rather than with `pulse`, because the release edge is a
    // normal edge: the accumulator consumes `d` on every edge, so the cycle after a
    // reset has already loaded again. Reading the reset means reading the edge it was
    // asserted on, which is the boundary everything else in this crate uses.
    let design = accumulator();
    let tb = Testbench::new(&design).unwrap();
    tb.run(|tb| async move {
        tb.drive("rst", 0).await;
        tb.drive("clr", 0).await;
        tb.drive("d", 9).await;
        assert_eq!(tb.value("q"), 9);
        tb.drive("rst", 1).await;
        assert_eq!(tb.value("q"), 0, "reset wins over the data on its own edge");
        tb.drive("rst", 0).await;
        assert_eq!(tb.value("q"), 9, "and the next edge loads again");
    })
    .unwrap();
}

#[test]
fn a_step_with_no_input_change_still_advances() {
    let design = accumulator();
    let tb = Testbench::new(&design).unwrap();
    tb.run(|tb| async move {
        tb.drive("rst", 0).await;
        tb.drive("clr", 0).await;
        tb.drive("d", 0).await;
        assert_eq!(tb.value("q"), 0);
        tb.step().await;
        tb.step().await;
    })
    .unwrap();
    // Two edges with no new data, and `d` is zero so the accumulator holds.
    let handle = tb.handle();
    assert_eq!(tb.cycle(), 5);
    assert_eq!(handle.value("q"), 0);
}

#[test]
fn a_combinational_design_settles_rather_than_refusing() {
    // A register-free design has no edge, so a "cycle" there is a settle -- the same
    // rule the cosimulator's generated driver follows. Without it every step on a
    // combinational design would fail with `NoClock`, and half the corpus would be
    // untestable.
    let design = adder();
    let tb = Testbench::new(&design).unwrap();
    tb.run(|tb| async move {
        tb.drive("a", 200).await;
        tb.drive("b", 100).await;
        assert_eq!(tb.value("sum"), 300);
        tb.settle().await;
        assert_eq!(tb.value("sum"), 300);
    })
    .unwrap();
}

#[test]
fn phases_share_one_cycle_count() {
    // A testbench is a sequence of phases -- reset, data, drain -- and each phase
    // being its own `run` is what makes a long test readable. So the count has to
    // keep running across calls rather than restarting.
    let design = accumulator();
    let tb = Testbench::new(&design).unwrap();
    tb.run(|tb| async move {
        tb.drive("rst", 0).await;
    })
    .unwrap();
    assert_eq!(tb.cycle(), 1);
    tb.run(|tb| async move {
        tb.drive("clr", 0).await;
    })
    .unwrap();
    assert_eq!(tb.cycle(), 2);
    tb.run(|tb| async move {
        tb.drive("d", 3).await;
    })
    .unwrap();
    assert_eq!(tb.cycle(), 3);
}

#[test]
fn an_unknown_port_is_reported_when_the_body_returns() {
    let design = accumulator();
    let tb = Testbench::new(&design).unwrap();
    let error = tb
        .run(|tb| async move {
            tb.drive("nope", 1).await;
        })
        .unwrap_err();
    assert!(
        error.to_string().contains("no port named `nope`"),
        "{error}"
    );
    // The message lists what the design does declare, because "no port named `q`"
    // is only half an answer.
    assert!(error.to_string().contains("rst"), "{error}");
}

#[test]
fn a_failure_is_reported_even_when_the_body_returns_a_value() {
    // The body "succeeded" as far as its own return value goes, and the testbench
    // would otherwise hand that value back and read as a pass. A step that failed is
    // a failed run.
    let design = accumulator();
    let tb = Testbench::new(&design).unwrap();
    let result: Result<u64, Error> = tb.run(|tb| async move {
        tb.drive("nope", 1).await;
        42
    });
    assert!(
        result.is_err(),
        "a recorded failure outranks a returned value"
    );
}

#[test]
fn a_narrow_port_takes_the_low_bits_of_a_value_rather_than_refusing_it() {
    // Documented behaviour, and the opposite of a bug: `drive` takes a `u64` and
    // resizes it to the *port's* width, so `0xffff_ffff` on a one-bit port is 1.
    // That is what writing to a wire does, and it is why the width comes from the
    // design rather than from the literal -- a value offered as a 1-bit `Bits` would
    // silently lose 63 bits instead.
    let design = accumulator();
    let tb = Testbench::new(&design).unwrap();
    tb.run(|tb| async move {
        tb.drive("rst", 0xffff_ffff).await;
        assert_eq!(
            tb.value("q"),
            0,
            "a one-bit port took the low bit, so reset is asserted"
        );

        // Reset stays asserted until it is driven low, so releasing it is part of
        // the test rather than something to assume.
        tb.drive("rst", 0).await;
        tb.drive("clr", 0).await;
        tb.drive("d", 0x1ff).await;
        // The accumulator is eight bits, so `d` held 0xff.
        assert_eq!(tb.value("q"), 0xff);
    })
    .unwrap();
}

#[test]
fn a_port_too_wide_for_a_u64_is_refused_and_points_at_drive_bits() {
    // A hundred-bit port cannot be driven by a `u64`, and the error says which
    // function can, rather than leaving the caller to work it out from the type.
    let design = Design::new();
    let wide = design.input("wide", 100).unwrap();
    let narrow = design.input("narrow", 3).unwrap();
    let sum = design.add(&wide, &wide).unwrap();
    let _ = narrow;
    ferrite_lithic::outputs::<WideOutputs>(&design, &WideOutputs { sum }).unwrap();
    let tb = Testbench::new(&design).unwrap();
    let error = tb
        .run(|tb| async move {
            tb.drive("wide", 1).await;
        })
        .unwrap_err();
    let message = error.to_string();
    assert!(message.contains("100 bits wide"), "{message}");
    assert!(message.contains("drive_bits"), "{message}");
}

#[test]
fn a_future_that_never_completes_is_refused_rather_than_hung() {
    // The testbench polls once, because advancing a cycle is work rather than a
    // yield and nothing in this crate ever suspends. A foreign future that waits for
    // a wake is waiting for something that will never happen, and the honest outcome
    // is an error rather than a test run that never returns.
    let design = accumulator();
    let tb = Testbench::new(&design).unwrap();
    let error = tb
        .run(
            |_| -> core::pin::Pin<Box<dyn std::future::Future<Output = ()>>> {
                // A poll that yields and asks to be woken again, which is what a future
                // depending on an external event does. Nothing will ever wake it here.
                Box::pin(core::future::poll_fn(|cx| {
                    cx.waker().wake_by_ref();
                    core::task::Poll::<()>::Pending
                }))
            },
        )
        .unwrap_err();
    assert!(matches!(error, Error::Suspended), "{error}");
}

#[test]
fn recording_collects_every_output_port_from_the_first_recorded_cycle() {
    let design = accumulator();
    let tb = Testbench::new(&design).unwrap();
    tb.run(|tb| async move {
        tb.drive("rst", 0).await;
        tb.drive("clr", 0).await;
        for value in [1u64, 2, 3] {
            tb.drive("d", value).await;
        }
    })
    .unwrap();
    assert_eq!(tb.recorded_ports(), ["q"]);
    // The series has one entry per edge and no leading row: the simulator numbers its
    // first edge cycle 1, and a series that started with a zero row would put a
    // plausible-looking value in front of every one of them.
    assert_eq!(tb.series("q").len(), 5);
    let values: Vec<u64> = tb.series("q").iter().map(|b| b.to_u64().unwrap()).collect();
    assert_eq!(values, [0, 0, 1, 3, 6]);
}

#[test]
fn a_handle_is_a_view_onto_the_same_simulation() {
    let design = accumulator();
    let tb = Testbench::new(&design).unwrap();
    let outer = tb.handle();
    tb.run(|tb| async move {
        tb.drive("rst", 0).await;
        tb.drive("clr", 0).await;
        tb.drive("d", 11).await;
    })
    .unwrap();
    // The handle outlives the body and sees everything it did.
    assert_eq!(outer.cycle(), 3);
    assert_eq!(outer.value("q"), 11);
}

#[test]
fn the_testbench_compiles_a_design_it_cannot_step_yet_without_panicking() {
    // `Testbench::new` is where an undriven wire or an unresolved read port is caught,
    // and it is a `Result` so the caller says what to do about it.
    let design = Design::new();
    let ports = ferrite_lithic::inputs::<AccInputs>(&design).unwrap();
    let dangling = design.wire(8).unwrap();
    let _ = ports;
    ferrite_lithic::outputs::<AccOutputs>(&design, &AccOutputs { q: dangling }).unwrap();
    let error = Testbench::new(&design).unwrap_err();
    assert!(
        error.to_string().contains("undriven") || error.to_string().contains("driven"),
        "the message should be about the undriven wire: {error}"
    );
}
