//! The four cycle claims, each one asserted against the graph it is about.
//!
//! Three of these come from Hardcaml's own `test_combinational_loop.ml`, and the
//! pair about memories is the one that is easy to get backwards: feedback *into* a
//! memory's write port is legal, and feedback *through* a read port is not. Both
//! hold here for the same reason — read ports are ordinary combinational nodes and
//! the memory itself is a terminal.

#![allow(clippy::unwrap_used)]

use ferrite_lithic_bits::Bits;
use ferrite_lithic_ir::{Circuit, Deps, Error, NodeId, Relation};

fn bits(value: u64, width: u32) -> Bits {
    Bits::constant(value, width).unwrap()
}

/// `a loop through a register is legal` (Hardcaml
/// `test/lib/test_combinational_loop.ml:152-182`).
///
/// A `Reg` is terminal for loop checking, so the accumulator's feedback path is
/// not a combinational cycle.
#[test]
fn a_loop_through_a_register_is_legal() {
    let mut c = Circuit::new();
    let acc = c.wire(8).unwrap();
    let one = c.constant(bits(1, 8));
    let clk = c.constant(bits(0, 1));
    let inc = c.add(acc, one).unwrap();
    let next = c.reg(inc, clk, clk, clk).unwrap();
    c.drive(acc, next).unwrap();

    assert!(c.topological_order(Deps::LoopChecking).is_ok());
    // And it is a genuine cycle under the relation that follows registers, which
    // is what proves the graph really does contain a loop and that the relation
    // is doing the work rather than the graph being acyclic by accident.
    let err = c.topological_order(Deps::WithoutCaseMatches).unwrap_err();
    assert!(
        matches!(
            err,
            Error::CombinationalLoop {
                relation: Relation::WithoutCaseMatches,
                ..
            }
        ),
        "{err:?}"
    );
    assert!(c.check().is_ok());
}

/// `a loop into a memory write port is legal` (`:270-313`).
///
/// An accumulator that reads a word and writes the incremented value back is the
/// most ordinary feedback structure there is, so it must not be reported as a
/// combinational loop.
#[test]
fn a_loop_into_a_memory_write_port_is_legal() {
    let mut c = Circuit::new();
    let acc = c.wire(8).unwrap();
    let one = c.constant(bits(1, 8));
    let inc = c.add(acc, one).unwrap();
    let addr = c.constant(bits(0, 1));
    let enable = c.constant(bits(1, 1));

    let mem = c.mem(8, 2).unwrap();
    c.write_port(mem, inc, addr, enable).unwrap();
    let read = c.read_port(mem, addr, enable).unwrap();
    c.drive(acc, read).unwrap();

    assert!(c.topological_order(Deps::LoopChecking).is_ok());
    assert!(c.topological_order(Deps::SimulationScheduling).is_ok());
    c.check().unwrap();

    // The loop is real: it only disappears because `Mem` is a terminal.
    assert!(c.topological_order(Deps::WithoutCaseMatches).is_err());
}

/// `a loop through a memory read port is not legal` (`:243-268`).
///
/// The read data feeds the address it was read at. The cycle is entirely in
/// combinational nodes, so no relation can excuse it.
#[test]
fn a_loop_through_a_memory_read_port_is_not_legal() {
    let mut c = Circuit::new();
    let addr = c.wire(4).unwrap();
    let enable = c.constant(bits(1, 1));
    let mem = c.mem(4, 16).unwrap();
    let read = c.read_port(mem, addr, enable).unwrap();

    // `addr_next` depends on `read`, and `read` depends on `addr`.
    let addr_next = c.add(addr, read).unwrap();
    c.drive(addr, addr_next).unwrap();

    for relation in [
        Deps::LoopChecking,
        Deps::SimulationScheduling,
        Deps::WithoutCaseMatches,
    ] {
        let err = c.topological_order(relation).unwrap_err();
        let Error::CombinationalLoop { cycle, .. } = err else {
            panic!("expected a cycle under {relation:?}, got {err:?}");
        };
        // The message must actually name the loop, starting and ending at the
        // same node so it reads as a path.
        assert_eq!(
            cycle.first(),
            cycle.last(),
            "cycle is not closed: {cycle:?}"
        );
        assert!(
            cycle.contains(&addr),
            "cycle does not mention the address: {cycle:?}"
        );
    }
}

/// A zero-delay combinational loop is not legal under any relation.
///
/// Two wires driving each other is the degenerate case, and it is what a
/// synthesiser would reject.
#[test]
fn a_zero_delay_combinational_loop_is_not_legal() {
    let mut c = Circuit::new();
    let a = c.wire(8).unwrap();
    let b = c.wire(8).unwrap();
    c.drive(a, b).unwrap();
    c.drive(b, a).unwrap();

    let err = c.topological_order(Deps::LoopChecking).unwrap_err();
    let Error::CombinationalLoop { cycle, relation } = err else {
        panic!("expected a cycle, got {err:?}");
    };
    assert_eq!(relation, Relation::LoopChecking);
    assert_eq!(cycle, vec![a, b, a]);

    // The advice has to name the relation, because the same cycle is legal when a
    // register sits in it and that is not something a reader should have to know.
    assert!(cycle.len() >= 2);
}

/// The cycle reported is a real cycle, not an arbitrary node list.
///
/// A wrong cycle in an error message sends a reader looking in the wrong place, so
/// each consecutive pair in the reported path must actually be a dependency edge.
#[test]
fn the_reported_cycle_is_genuinely_a_path_of_dependencies() {
    let mut c = Circuit::new();
    let a = c.wire(8).unwrap();
    let b = c.wire(8).unwrap();
    let d = c.wire(8).unwrap();
    c.drive(a, b).unwrap();
    c.drive(b, d).unwrap();
    c.drive(d, a).unwrap();

    let err = c.topological_order(Deps::LoopChecking).unwrap_err();
    let Error::CombinationalLoop { cycle, .. } = err else {
        panic!("expected a cycle")
    };
    for pair in cycle.windows(2) {
        let (from, to) = (pair[0], pair[1]);
        let deps = c.deps_in(from, Deps::LoopChecking).unwrap();
        assert!(
            deps.contains(&to),
            "{from} -> {to} is not an edge; cycle {cycle:?} is wrong"
        );
    }
}

/// A feedback loop built through an instance is judged by the relation.
///
/// `Instance` is the only kind the loop-checking and scheduling relations disagree
/// about, so it is the only way to observe that they differ at all.
#[test]
fn an_instance_is_a_combinational_boundary_for_loop_checking_only() {
    let build = || {
        let mut c = Circuit::new();
        let w = c.wire(8).unwrap();
        let k = c.constant(bits(3, 8));
        let sub = c.instance("blender", Vec::new(), vec![w, k], 8).unwrap();
        c.drive(w, sub).unwrap();
        c
    };
    let c = build();
    assert!(c.topological_order(Deps::LoopChecking).is_ok());
    assert!(c.topological_order(Deps::SimulationScheduling).is_err());
    assert!(c.topological_order(Deps::WithoutCaseMatches).is_err());
}

/// Undriven wires are legal until something reads them.
///
/// `check` tolerates a leftover wire precisely so a half-built graph can be
/// inspected, but must still catch a read of a wire nobody drives.
#[test]
fn check_catches_a_read_of_an_undriven_wire() {
    let mut c = Circuit::new();
    let dangling = c.wire(8).unwrap();
    let one = c.constant(bits(1, 8));
    let _consumer = c.add(dangling, one).unwrap();

    let err = c.check().unwrap_err();
    assert!(
        matches!(err, Error::UndrivenWire { wire, width: 8 } if wire == dangling),
        "{err:?}"
    );
}

#[test]
fn check_tolerates_an_unreferenced_undriven_wire() {
    let mut c = Circuit::new();
    let _leftover = c.wire(8).unwrap();
    c.check().unwrap();
}

/// A wire's driver is exactly one node, and undriving gives it back.
#[test]
fn driver_of_reports_and_clears_the_driver() {
    let mut c = Circuit::new();
    let w = c.wire(8).unwrap();
    let d = c.constant(bits(7, 8));
    assert_eq!(c.driver_of(w).unwrap(), None);

    c.drive(w, d).unwrap();
    assert_eq!(c.driver_of(w).unwrap(), Some(d));

    assert_eq!(c.undrive(w).unwrap(), Some(d));
    assert_eq!(c.driver_of(w).unwrap(), None);
    // The node is still usable for its original purpose.
    let nine = c.constant(bits(9, 8));
    c.drive(w, nine).unwrap();
    assert_eq!(c.driver_of(w).unwrap(), Some(nine));
}

#[test]
fn driver_of_rejects_a_non_wire() {
    let mut c = Circuit::new();
    let k = c.constant(bits(0, 1));
    let err = c.driver_of(k).unwrap_err();
    assert!(
        matches!(err, Error::NotAWire { target, kind: "Constant" } if target == k),
        "{err:?}"
    );
}

#[test]
fn driving_a_non_wire_is_refused() {
    let mut c = Circuit::new();
    let k = c.constant(bits(0, 8));
    let d = c.constant(bits(0, 8));
    let err = c.drive(k, d).unwrap_err();
    let text = err.to_string();
    assert!(text.contains("Constant"), "{text}");
    assert!(text.contains("back edge"), "{text}");
}

#[test]
fn a_wire_cannot_be_driven_twice() {
    let mut c = Circuit::new();
    let w = c.wire(8).unwrap();
    let first = c.constant(bits(1, 8));
    let second = c.constant(bits(2, 8));
    c.drive(w, first).unwrap();

    let err = c.drive(w, second).unwrap_err();
    let Error::AlreadyDriven {
        wire,
        existing,
        attempted,
        ..
    } = err
    else {
        panic!("expected AlreadyDriven, got {err:?}");
    };
    assert_eq!((wire, existing, attempted), (w, first, second));
}

#[test]
fn a_driver_must_match_the_wires_width() {
    let mut c = Circuit::new();
    let w = c.wire(8).unwrap();
    let narrow = c.constant(bits(0, 4));
    let err = c.drive(w, narrow).unwrap_err();
    assert!(
        matches!(err, Error::WidthMismatch { lhs: 8, rhs: 4, .. }),
        "{err:?}"
    );
    // And the wire is still undriven, because the failed check must not have
    // half-applied.
    assert_eq!(c.driver_of(w).unwrap(), None);
}

#[test]
fn ids_are_allocated_in_construction_order() {
    let mut c = Circuit::new();
    let a = c.wire(8).unwrap();
    let b = c.constant(bits(0, 8));
    let d = c.add(a, b).unwrap();
    let collected: Vec<NodeId> = c.node_ids().collect();
    assert_eq!(collected, vec![a, b, d]);
    assert!(collected.windows(2).all(|w| w[0].index() < w[1].index()));
}
