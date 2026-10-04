//! The three-phase rewrite, and the errors that keep each phase honest.
//!
//! This is how a separately-built sub-circuit is stitched into a parent graph
//! (`Signal_graph.rewrite`, `src/signal_graph.ml:105-204`). The phase split is the
//! design: validate and register the mapping, rewrite operands, then re-attach
//! drivers. Collapsing the phases would make a malformed mapping leave the graph
//! half-rewired, which is far harder to debug than a refused rewrite.

#![allow(clippy::unwrap_used)]

use std::collections::BTreeMap;

use ferrite_lithic_bits::Bits;
use ferrite_lithic_ir::{Circuit, Deps, Error, NodeId};

fn bits(value: u64, width: u32) -> Bits {
    Bits::constant(value, width).unwrap()
}

fn map(pairs: &[(NodeId, NodeId)]) -> BTreeMap<NodeId, NodeId> {
    pairs.iter().copied().collect()
}

/// A sub-circuit whose single output is an undriven wire the parent will drive.
///
/// Returns `(circuit, output_wire, internal_driver)`.
fn subcircuit(c: &mut Circuit) -> (NodeId, NodeId) {
    let out = c.wire(8).unwrap();
    let five = c.constant(bits(5, 8));
    let three = c.constant(bits(3, 8));
    let internal = c.add(five, three).unwrap();
    c.drive(out, internal).unwrap();
    (out, internal)
}

#[test]
fn a_replacement_takes_over_the_originals_users_and_driver() {
    let mut c = Circuit::new();
    let (out, internal) = subcircuit(&mut c);
    let one = c.constant(bits(1, 8));
    let consumer = c.add(out, one).unwrap();
    let replacement = c.wire(8).unwrap();

    c.apply_replacements(&map(&[(out, replacement)])).unwrap();

    // Phase 2: every user now sees the replacement.
    let deps = c.deps_in(consumer, Deps::WithoutCaseMatches).unwrap();
    assert!(
        deps.contains(&replacement),
        "consumer still reads the original: {deps:?}"
    );
    assert!(
        !deps.contains(&out),
        "consumer still reads the original: {deps:?}"
    );

    // Phase 3: the replacement took the original's driver.
    assert_eq!(c.driver_of(replacement).unwrap(), Some(internal));
    // The original is left behind, unreferenced but still driven: upstream does
    // not undrive either, and keeping the driver means the substitution is
    // reversible. It is a dead node until a dead-code pass collects it.
    assert_eq!(c.driver_of(out).unwrap(), Some(internal));
    c.check().unwrap();
}

#[test]
fn a_chain_of_replacements_resolves_in_one_pass() {
    // `first_out` is driven by `second_out`, which is itself replaced. Phase 3 has
    // to remap the driver through the same mapping, or the chain would be left
    // pointing at a wire that nothing drives.
    let mut c = Circuit::new();
    let five = c.constant(bits(5, 8));
    let three = c.constant(bits(3, 8));
    let internal = c.add(five, three).unwrap();
    let second_out = c.wire(8).unwrap();
    c.drive(second_out, internal).unwrap();
    let first_out = c.wire(8).unwrap();
    c.drive(first_out, second_out).unwrap();

    let consumer = c.cat(first_out, second_out).unwrap();

    let first_new = c.wire(8).unwrap();
    let second_new = c.wire(8).unwrap();
    c.apply_replacements(&map(&[(first_out, first_new), (second_out, second_new)]))
        .unwrap();

    // The chain resolved transitively in one pass.
    assert_eq!(c.driver_of(first_new).unwrap(), Some(second_new));
    assert_eq!(c.driver_of(second_new).unwrap(), Some(internal));

    let deps = c.deps_in(consumer, Deps::WithoutCaseMatches).unwrap();
    assert!(
        deps.contains(&first_new) && deps.contains(&second_new),
        "{deps:?}"
    );
    c.check().unwrap();
}

#[test]
fn several_replacements_at_once() {
    let mut c = Circuit::new();
    let mut outs = Vec::new();
    for _ in 0..4 {
        let (out, _) = subcircuit(&mut c);
        outs.push(out);
    }
    let sum = outs
        .iter()
        .copied()
        .reduce(|acc, out| c.add(acc, out).unwrap())
        .unwrap();

    let replacements: Vec<(NodeId, NodeId)> = outs
        .iter()
        .map(|&out| {
            let fresh = c.wire(8).unwrap();
            (out, fresh)
        })
        .collect();
    c.apply_replacements(&map(&replacements)).unwrap();

    // `sum` is a fold, so only the last output is a *direct* operand of it; the
    // others are reachable through the intermediate adds. Reachability is the
    // honest question to ask about a whole cone.
    let cone = c.reachable(&[sum], Deps::WithoutCaseMatches).unwrap();
    for (original, fresh) in &replacements {
        assert!(!cone.contains(original), "{original} still in the cone");
        assert!(cone.contains(fresh), "{fresh} missing from the cone");
    }
    c.check().unwrap();
}

#[test]
fn an_empty_mapping_is_a_no_op() {
    let mut c = Circuit::new();
    let (out, _) = subcircuit(&mut c);
    let before = c.len();
    c.apply_replacements(&BTreeMap::new()).unwrap();
    assert_eq!(c.len(), before);
    assert!(c.driver_of(out).unwrap().is_some());
}

#[test]
fn rewriting_a_whole_dependent_cone_preserves_widths() {
    // The substitution has to be width-preserving or the graph's arithmetic stops
    // meaning what it said, which is why phase 1 checks it.
    let mut c = Circuit::new();
    let (out, _) = subcircuit(&mut c);
    let widened = c.cat(out, out).unwrap();
    assert_eq!(c.width_of(widened), 16);

    let replacement = c.wire(8).unwrap();
    c.apply_replacements(&map(&[(out, replacement)])).unwrap();
    assert_eq!(
        c.width_of(widened),
        16,
        "width must survive the substitution"
    );
    c.check().unwrap();
}

#[test]
fn a_replacement_must_be_an_unattached_wire() {
    let mut c = Circuit::new();
    let (out, _) = subcircuit(&mut c);

    // Not a wire at all.
    let not_a_wire = c.constant(bits(0, 8));
    let err = c
        .apply_replacements(&map(&[(out, not_a_wire)]))
        .unwrap_err();
    assert!(
        matches!(
            err,
            Error::ReplacementNotAWire {
                kind: "Constant",
                ..
            }
        ),
        "{err:?}"
    );

    // A wire, but already driven.
    let already_driven = c.wire(8).unwrap();
    let driver = c.constant(bits(1, 8));
    c.drive(already_driven, driver).unwrap();
    let err = c
        .apply_replacements(&map(&[(out, already_driven)]))
        .unwrap_err();
    assert!(
        matches!(err, Error::ReplacementDriven { driver: d, .. } if d == driver),
        "{err:?}"
    );

    // A wire of the wrong width.
    let too_narrow = c.wire(4).unwrap();
    let err = c
        .apply_replacements(&map(&[(out, too_narrow)]))
        .unwrap_err();
    assert!(
        matches!(err, Error::ReplacementWidthMismatch { lhs: 8, rhs: 4, .. }),
        "{err:?}"
    );
}

#[test]
fn the_original_must_be_a_wire() {
    // A sub-circuit's *internal* node is not something a parent can substitute for:
    // it is not a port, and swapping it would leave every wire that was supposed to
    // cross the boundary pointing at internals.
    let mut c = Circuit::new();
    let (_, internal) = subcircuit(&mut c);
    let replacement = c.wire(8).unwrap();
    let err = c
        .apply_replacements(&map(&[(internal, replacement)]))
        .unwrap_err();
    assert!(
        matches!(err, Error::NotAWire { kind: "Add", .. }),
        "{err:?}"
    );
}

#[test]
fn a_refused_rewrite_leaves_the_graph_untouched() {
    // Phase 1 validates the whole mapping before mutating anything, so a bad entry
    // halfway through cannot leave the graph half-rewired.
    let mut c = Circuit::new();
    let (first_out, _) = subcircuit(&mut c);
    let (second_out, _) = subcircuit(&mut c);
    let consumer = c.cat(first_out, second_out).unwrap();

    let good = c.wire(8).unwrap();
    let bad = c.wire(4).unwrap();
    let err = c
        .apply_replacements(&map(&[(first_out, good), (second_out, bad)]))
        .unwrap_err();
    assert!(
        matches!(err, Error::ReplacementWidthMismatch { .. }),
        "{err:?}"
    );

    // The first, valid entry must not have been applied.
    let deps = c.deps_in(consumer, Deps::WithoutCaseMatches).unwrap();
    assert!(
        deps.contains(&first_out),
        "the good entry leaked into the graph"
    );
    assert!(deps.contains(&second_out));
    assert!(!deps.contains(&good));
}

#[test]
fn a_node_with_no_driver_is_left_undriven_by_the_rewrite() {
    // An undriven original has no driver to hand over, so the replacement stays
    // undriven too — and `check` is what decides whether that matters.
    let mut c = Circuit::new();
    let dangling = c.wire(8).unwrap();
    let one = c.constant(bits(1, 8));
    let consumer = c.add(dangling, one).unwrap();
    let replacement = c.wire(8).unwrap();

    c.apply_replacements(&map(&[(dangling, replacement)]))
        .unwrap();
    assert_eq!(c.driver_of(replacement).unwrap(), None);
    assert!(
        c.deps_in(consumer, Deps::LoopChecking)
            .unwrap()
            .contains(&replacement)
    );
    // Still an error, now reported against the replacement rather than the original.
    let err = c.check().unwrap_err();
    assert!(
        matches!(err, Error::UndrivenWire { wire, width: 8 } if wire == replacement),
        "{err:?}"
    );
}

#[test]
fn a_register_can_be_the_original_of_a_substitution() {
    // A sub-circuit whose output port is driven by a register: the port is still a
    // wire, and the register simply becomes the replacement's driver.
    let mut c = Circuit::new();
    let clk = c.constant(bits(0, 1));
    let out = c.wire(8).unwrap();
    let seven = c.constant(bits(7, 8));
    let reg = c.reg(seven, clk, clk, clk).unwrap();
    c.drive(out, reg).unwrap();

    let replacement = c.wire(8).unwrap();
    c.apply_replacements(&map(&[(out, replacement)])).unwrap();
    assert_eq!(c.driver_of(replacement).unwrap(), Some(reg));
    c.check().unwrap();
}
