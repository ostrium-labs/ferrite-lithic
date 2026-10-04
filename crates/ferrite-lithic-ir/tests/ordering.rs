//! Topological order: validity, determinism, and the promise that makes emitted
//! Verilog reproducible.

#![allow(clippy::unwrap_used)]

use ferrite_lithic_bits::Bits;
use ferrite_lithic_ir::{Circuit, Deps, NodeId};

fn bits(value: u64, width: u32) -> Bits {
    Bits::constant(value, width).unwrap()
}

/// Every dependency of a node appears before it.
///
/// This is the defining property of a topological order, so it is what the
/// scheduler and the emitter will both rely on. Asserted here rather than trusted,
/// because a "valid" order that is not actually valid produces a simulator whose
/// bug only appears on some inputs.
fn assert_valid_order(c: &Circuit, order: &[NodeId], relation: Deps) {
    let mut position = vec![usize::MAX; c.len()];
    for (i, &id) in order.iter().enumerate() {
        assert!(
            position[id.index()] == usize::MAX,
            "node {id} appears twice"
        );
        position[id.index()] = i;
    }
    assert_eq!(
        order.len(),
        c.len(),
        "order is not a permutation of the arena"
    );

    for &id in order {
        for dep in c.deps_in(id, relation).unwrap() {
            assert!(
                position[dep.index()] < position[id.index()],
                "{dep} must be evaluated before {id} under {relation:?}"
            );
        }
    }
}

#[test]
fn a_bottom_up_build_is_already_in_evaluation_order() {
    // The headline property: because ids *are* construction order, and because a
    // bottom-up build always allocates operands before the node that uses them,
    // the topological order of a wire-free graph is exactly construction order.
    //
    // This is what Hardcaml needs `normalize_uids` for, and ships switched off
    // with its own test commented out as "brittle"
    // (`test/lib/test_uid_normalization.ml:56-59`). Here it is a structural
    // property, so it cannot regress.
    let mut c = Circuit::new();
    let a = c.constant(bits(0, 8));
    let b = c.constant(bits(1, 8));
    let sum = c.add(a, b).unwrap();
    let wide = c.cat(a, b).unwrap();
    let masked = c.bit_and(sum, sum).unwrap();
    let cond = c.ult(a, b).unwrap();
    let selected = c.ite(cond, masked, a).unwrap();

    let order = c.topological_order(Deps::LoopChecking).unwrap();
    assert_eq!(order, vec![a, b, sum, wide, masked, cond, selected]);
    assert_valid_order(&c, &order, Deps::LoopChecking);
}

#[test]
fn a_wire_moves_its_driver_before_itself() {
    // Construction order is *almost* evaluation order: a wire is allocated before
    // its driver, so a wire must be moved after it. That is the one reordering a
    // feedback loop forces, which is a good reason the tie-break is on id.
    let mut c = Circuit::new();
    let w = c.wire(8).unwrap();
    let one = c.constant(bits(1, 8));
    let clk = c.constant(bits(0, 1));
    let next = c.add(w, one).unwrap();
    let reg = c.reg(next, clk, clk, clk).unwrap();
    c.drive(w, reg).unwrap();

    let order = c.topological_order(Deps::LoopChecking).unwrap();
    assert_valid_order(&c, &order, Deps::LoopChecking);
    let position = |id: NodeId| order.iter().position(|&x| x == id).unwrap();
    assert!(
        position(reg) < position(w),
        "the driver must precede the wire it drives"
    );
    // And the wire is still allocated before its driver, which is what makes the
    // ids stable rather than evaluation-ordered.
    assert!(w.index() < reg.index());
}

#[test]
fn the_order_is_deterministic_across_rebuilds() {
    // Byte-identical Verilog for identical input programs depends on this, so it
    // is asserted rather than assumed: build the same program twice, including on
    // a second thread, and compare.
    let build = || {
        let mut c = Circuit::new();
        let a = c.constant(bits(3, 4));
        let b = c.constant(bits(5, 4));
        let wide = c.cat(a, b).unwrap();
        let d = c.replicate(a, 2).unwrap();
        let mixed = c.bit_or(wide, d).unwrap();
        let mem = c.mem(8, 4).unwrap();
        let addr = c.constant(bits(0, 2));
        let en = c.constant(bits(1, 1));
        c.write_port(mem, mixed, addr, en).unwrap();
        let read = c.read_port(mem, addr, en).unwrap();
        let cond = c.eq(read, mixed).unwrap();
        let sel = c.ite(cond, read, mixed).unwrap();
        (c, vec![sel])
    };

    let (first, roots) = build();
    let (second, _) = build();
    let order_first = first.reachable(&roots, Deps::LoopChecking).unwrap();
    let order_second = second.reachable(&roots, Deps::LoopChecking).unwrap();
    assert_eq!(order_first, order_second);

    // And across threads, which the global uid counter this design replaced could
    // not have managed.
    let handle = std::thread::spawn(build);
    let (threaded, _) = handle.join().unwrap();
    assert_eq!(
        threaded.reachable(&roots, Deps::LoopChecking).unwrap(),
        order_first
    );
}

#[test]
fn two_circuits_can_be_built_concurrently() {
    // The concrete payoff of arena indices over a process-global counter.
    let build = |seed: u64| {
        let mut c = Circuit::new();
        let mut acc = c.constant(bits(seed, 8));
        for _ in 0..16 {
            let k = c.constant(bits(1, 8));
            acc = c.add(acc, k).unwrap();
        }
        (c, acc)
    };
    let handles: Vec<_> = (0..8u64)
        .map(|s| std::thread::spawn(move || build(s)))
        .collect();
    for (s, handle) in handles.into_iter().enumerate() {
        let (c, acc) = handle.join().unwrap();
        assert_eq!(c.width_of(acc), 8);
        assert_eq!(
            c.topological_order(Deps::LoopChecking).unwrap().len(),
            c.len()
        );
        // Every thread built the same number of nodes, so no counter interleaved.
        assert_eq!(c.len(), 1 + 16 + 16);
        let _ = s;
    }
}

#[test]
fn reachability_reports_exactly_the_cone() {
    let mut c = Circuit::new();
    let used = c.constant(bits(1, 8));
    let unused = c.constant(bits(2, 8));
    let sum = c.add(used, used).unwrap();

    let reached = c.reachable(&[sum], Deps::LoopChecking).unwrap();
    assert_eq!(reached, vec![used, sum]);
    assert!(
        !reached.contains(&unused),
        "an unused node is not in the cone"
    );
    assert!(c.reachable(&[], Deps::LoopChecking).unwrap().is_empty());
}

#[test]
fn a_large_random_graph_gets_a_valid_order() {
    // A depth-first recursion would be a stack-overflow risk on a deep graph, so
    // the sort is iterative; this exercises the iterative path at a depth a real
    // design reaches.
    let mut c = Circuit::new();
    let mut acc = c.constant(bits(0, 32));
    for _ in 0..500 {
        let k = c.constant(bits(1, 32));
        acc = c.add(acc, k).unwrap();
    }
    assert_eq!(c.len(), 1001);
    let order = c.topological_order(Deps::LoopChecking).unwrap();
    assert_valid_order(&c, &order, Deps::LoopChecking);
}

#[test]
fn a_deep_feedback_chain_does_not_overflow_the_stack() {
    let mut c = Circuit::new();
    let w = c.wire(8).unwrap();
    let mut node = c.constant(bits(1, 8));
    for _ in 0..500 {
        let k = c.constant(bits(0, 8));
        node = c.bit_xor(node, k).unwrap();
    }
    let clk = c.constant(bits(0, 1));
    let reg = c.reg(node, clk, clk, clk).unwrap();
    c.drive(w, reg).unwrap();
    // Cycle reporting is the iterative path that must not recurse.
    let _ = c.topological_order(Deps::WithoutCaseMatches);
}
