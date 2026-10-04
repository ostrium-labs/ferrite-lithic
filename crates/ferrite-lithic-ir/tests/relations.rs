//! The dependency sets themselves, under each relation.
//!
//! `cycles.rs` tests what the relations *permit*; this tests what they *contain*,
//! which is where a wrong operand list hides. The regression that motivates the
//! file: `push_operands` once treated `Mem` as a leaf under every relation, so no
//! dependency through a memory was ever visible — which is why the
//! `WithoutCaseMatches` relation produced no cycles where it should have.

#![allow(clippy::unwrap_used)]

use ferrite_lithic_bits::Bits;
use ferrite_lithic_ir::{Circuit, Deps, Node};

fn bits(value: u64, width: u32) -> Bits {
    Bits::constant(value, width).unwrap()
}

/// A circuit with one node of every kind, wired up so nothing dangles.
fn every_kind() -> (Circuit, Vec<ferrite_lithic_ir::NodeId>) {
    let mut c = Circuit::new();
    let mut made = Vec::new();

    made.push(c.constant(bits(0, 8)));

    let w = c.wire(8).unwrap();
    made.push(w);

    let a = c.constant(bits(1, 8));
    let b = c.constant(bits(2, 8));
    let one_bit = c.constant(bits(1, 1));

    for id in [
        c.not(a),
        c.bit_and(a, b).unwrap(),
        c.bit_or(a, b).unwrap(),
        c.bit_xor(a, b).unwrap(),
        c.add(a, b).unwrap(),
        c.sub(a, b).unwrap(),
        c.mul(a, b).unwrap(),
        c.udiv(a, b).unwrap(),
        c.sdiv(a, b).unwrap(),
        c.urem(a, b).unwrap(),
        c.srem(a, b).unwrap(),
        c.eq(a, b).unwrap(),
        c.ult(a, b).unwrap(),
        c.cat(a, b).unwrap(),
        c.replicate(a, 2).unwrap(),
        c.ite(one_bit, a, b).unwrap(),
        c.reg(a, one_bit, one_bit, one_bit).unwrap(),
        c.instance("sub", Vec::new(), vec![a, b], 8).unwrap(),
    ] {
        made.push(id);
    }

    let addr = c.constant(bits(0, 2));
    let mem = c.mem(8, 4).unwrap();
    made.push(mem);
    c.write_port(mem, a, addr, one_bit).unwrap();
    made.push(c.read_port(mem, addr, one_bit).unwrap());

    c.drive(w, a).unwrap();
    (c, made)
}

fn deps(
    c: &Circuit,
    id: ferrite_lithic_ir::NodeId,
    relation: Deps,
) -> Vec<ferrite_lithic_ir::NodeId> {
    c.deps_in(id, relation).unwrap()
}

#[test]
fn a_wire_depends_on_its_driver_under_every_relation() {
    let (c, made) = every_kind();
    let wire = made[1];
    // This edge is the only mutable edge in the graph and the only reason a
    // feedback loop is visible at all, so it must not depend on the relation.
    for relation in [
        Deps::LoopChecking,
        Deps::SimulationScheduling,
        Deps::WithoutCaseMatches,
    ] {
        assert!(
            !deps(&c, wire, relation).is_empty(),
            "{relation:?} lost the wire's driver"
        );
    }
}

#[test]
fn a_plain_combinational_node_reports_every_operand() {
    let (c, made) = every_kind();
    let add = made
        .iter()
        .copied()
        .find(|&id| matches!(c.get(id).unwrap(), Node::Add { .. }))
        .unwrap();
    for relation in [
        Deps::LoopChecking,
        Deps::SimulationScheduling,
        Deps::WithoutCaseMatches,
    ] {
        assert_eq!(deps(&c, add, relation).len(), 2, "{relation:?}");
    }
}

#[test]
fn a_register_is_terminal_except_under_without_case_matches() {
    let (c, made) = every_kind();
    let reg = made
        .iter()
        .copied()
        .find(|&id| matches!(c.get(id).unwrap(), Node::Reg { .. }))
        .unwrap();
    assert!(deps(&c, reg, Deps::LoopChecking).is_empty());
    assert!(deps(&c, reg, Deps::SimulationScheduling).is_empty());
    // Its four inputs, when followed.
    assert_eq!(deps(&c, reg, Deps::WithoutCaseMatches).len(), 4);
}

#[test]
fn a_memory_is_terminal_for_the_two_timing_relations_and_followed_otherwise() {
    let (c, made) = every_kind();
    let mem = made
        .iter()
        .copied()
        .find(|&id| matches!(c.get(id).unwrap(), Node::Mem { .. }))
        .unwrap();
    assert!(deps(&c, mem, Deps::LoopChecking).is_empty());
    assert!(deps(&c, mem, Deps::SimulationScheduling).is_empty());
    // Three signals per write port, and this circuit has exactly one port.
    assert_eq!(deps(&c, mem, Deps::WithoutCaseMatches).len(), 3);
}

#[test]
fn a_memorys_write_ports_are_visible_under_the_follow_everything_relation() {
    // The regression test. Before it, `Mem` reported no dependencies at all under
    // any relation, which hid every feedback path through a memory.
    let (c, made) = every_kind();
    let mem = made
        .iter()
        .copied()
        .find(|&id| matches!(c.get(id).unwrap(), Node::Mem { .. }))
        .unwrap();
    let ports = c.write_ports_of(mem).unwrap();
    assert_eq!(ports.len(), 1);

    let all = deps(&c, mem, Deps::WithoutCaseMatches);
    for port in ports {
        for signal in port.signals() {
            assert!(
                all.contains(&signal),
                "write port signal {signal} is not a dependency of {mem}"
            );
        }
    }
}

#[test]
fn an_instance_is_terminal_only_for_loop_checking() {
    let (c, made) = every_kind();
    let inst = made
        .iter()
        .copied()
        .find(|&id| matches!(c.get(id).unwrap(), Node::Instance { .. }))
        .unwrap();
    assert!(deps(&c, inst, Deps::LoopChecking).is_empty());
    assert_eq!(deps(&c, inst, Deps::SimulationScheduling).len(), 2);
    assert_eq!(deps(&c, inst, Deps::WithoutCaseMatches).len(), 2);
}

#[test]
fn a_read_port_always_reports_all_three_of_its_signals() {
    // The read port being followed while the memory is terminal is the whole
    // reason a loop through a read port is illegal and a loop into a write port is
    // not.
    let (c, made) = every_kind();
    let port = made
        .iter()
        .copied()
        .find(|&id| matches!(c.get(id).unwrap(), Node::ReadPort { .. }))
        .unwrap();
    for relation in [
        Deps::LoopChecking,
        Deps::SimulationScheduling,
        Deps::WithoutCaseMatches,
    ] {
        let all = deps(&c, port, relation);
        assert_eq!(all.len(), 3, "{relation:?} reported {all:?}");
    }
}

#[test]
fn a_constant_and_a_leaf_wire_have_no_operands() {
    let (c, made) = every_kind();
    let konst = made[0];
    for relation in [
        Deps::LoopChecking,
        Deps::SimulationScheduling,
        Deps::WithoutCaseMatches,
    ] {
        assert!(deps(&c, konst, relation).is_empty());
    }
    // A wire's only dependency is its driver, never an operand.
    let Node::Wire { .. } = c.get(made[1]).unwrap() else {
        panic!("expected a wire")
    };
    assert!(c.get(made[1]).unwrap().operands().is_empty());
}

#[test]
fn operands_agree_with_the_widest_relation() {
    // `operands` is the relation-free view. For every node the widest relation is
    // `WithoutCaseMatches`, so the two must agree there — which makes
    // `Node::operands` a usable cross-check on `deps_in`.
    let (c, made) = every_kind();
    for id in made {
        let operands = c.get(id).unwrap().operands();
        let widest = deps(&c, id, Deps::WithoutCaseMatches);
        if matches!(c.get(id).unwrap(), Node::Wire { .. }) {
            // A wire's driver is an edge but not an operand, which is the one
            // documented difference.
            assert!(operands.is_empty());
            assert_eq!(widest.len(), 1);
        } else {
            assert_eq!(operands, widest, "operands and deps disagree for {id}");
        }
    }
}

#[test]
fn the_case_match_row_is_inert_with_literal_matches() {
    // Documented honestly rather than assumed: with literal match values there is
    // nothing to follow, so dropping the row changes no result in A1. If match
    // constants ever become signals, this is the test that has to change.
    let mut c = Circuit::new();
    let scrutinee = c.constant(bits(0, 4));
    let arm = c.constant(bits(7, 8));
    let other = c.constant(bits(9, 8));
    let arms = vec![
        ferrite_lithic_ir::CaseArm::new(bits(1, 4), arm),
        ferrite_lithic_ir::CaseArm::new(bits(2, 4), other),
    ];
    let case = c.case_(scrutinee, arms, arm).unwrap();

    for relation in [
        Deps::LoopChecking,
        Deps::SimulationScheduling,
        Deps::WithoutCaseMatches,
    ] {
        // scrutinee, both arm values, and the default.
        assert_eq!(deps(&c, case, relation).len(), 4, "{relation:?}");
    }
    assert!(Deps::LoopChecking.includes_case_matches());
    assert!(Deps::SimulationScheduling.includes_case_matches());
    assert!(!Deps::WithoutCaseMatches.includes_case_matches());
}

#[test]
fn a_multiport_memory_reports_every_port_under_the_widest_relation() {
    let mut c = Circuit::new();
    let en = c.constant(bits(1, 1));
    let addr = c.constant(bits(0, 2));
    let data = c.constant(bits(0, 8));
    let mem = c.mem(8, 4).unwrap();
    for _ in 0..3 {
        c.write_port(mem, data, addr, en).unwrap();
    }
    assert_eq!(deps(&c, mem, Deps::WithoutCaseMatches).len(), 9);
    assert!(deps(&c, mem, Deps::LoopChecking).is_empty());
}

#[test]
fn a_dependency_list_is_in_the_nodes_fixed_operand_order() {
    // Not arena order: `Mem` reports `[data, address, enable]` per port, and the
    // data signal was usually allocated last. What matters is that the order is
    // fixed per node kind and reproducible, because the scheduler reads the list
    // positionally.
    let (c, made) = every_kind();
    let mem = made
        .iter()
        .copied()
        .find(|&id| matches!(c.get(id).unwrap(), Node::Mem { .. }))
        .unwrap();
    let port = c.write_ports_of(mem).unwrap()[0].clone();
    let expected = vec![port.data, port.address, port.enable];
    assert_eq!(deps(&c, mem, Deps::WithoutCaseMatches), expected);

    for id in made {
        for relation in [
            Deps::LoopChecking,
            Deps::SimulationScheduling,
            Deps::WithoutCaseMatches,
        ] {
            let list = deps(&c, id, relation);
            assert_eq!(
                list,
                deps(&c, id, relation),
                "deps of {id} under {relation:?} are unstable"
            );
        }
    }
}
