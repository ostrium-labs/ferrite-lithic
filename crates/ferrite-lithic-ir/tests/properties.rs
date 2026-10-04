//! Properties that must hold for *every* graph, checked against randomly
//! generated ones.
//!
//! The unit and integration tests assert specific claims about specific graphs.
//! These assert invariants that no amount of hand-picking finds: that every node's
//! width agrees with an independent recomputation, that a topological order is
//! genuinely valid, and that building the same program twice gives the same graph.
//!
//! The generator deliberately produces wires and feedback paths, not just trees,
//! because that is where the arena's invariants actually have work to do.

#![allow(clippy::unwrap_used)]

use ferrite_lithic_bits::Bits;
use ferrite_lithic_ir::{CaseArm, Circuit, Deps, Node, NodeId, address_width_for_depth};
use proptest::prelude::*;

/// A deterministic LCG, so a failing seed is reproducible from the report alone.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 11
    }
}

/// A tree of nodes, generated as an index into the nodes built so far.
fn tree() -> impl Strategy<Value = Vec<(u8, usize, usize)>> {
    prop::collection::vec((0u8..13, 0usize..64, 0usize..64), 1..48)
}

/// A standalone program description: a shape plus which node to close loops on.
#[derive(Clone, Debug)]
struct Program {
    shape: Vec<(u8, usize, usize)>,
    loops: Vec<(usize, usize)>,
    seed: u64,
}

/// Build a graph deterministically from a `Program`.
fn build(program: &Program) -> Circuit {
    let mut c = Circuit::new();
    let mut rng = Lcg(program.seed | 1);
    let mut widths: Vec<u32> = Vec::new();
    let mut idents: Vec<NodeId> = Vec::new();

    // A pool of seeds so the generated program is never empty and every width
    // referenced actually exists. Kept as parallel vectors indexed together,
    // because a node's width is what decides whether an operation is legal and the
    // generator has to know it before building.
    for w in [1u32, 4, 8, 16] {
        let value = rng_below(&mut rng, w);
        let id = c.constant(Bits::constant(value, w).unwrap());
        widths.push(w);
        idents.push(id);
    }
    for _ in 0..4 {
        let id = c.wire(8).unwrap();
        widths.push(8);
        idents.push(id);
    }

    fn first_of_width(idents: &[NodeId], widths: &[u32], want: u32) -> NodeId {
        idents
            .iter()
            .copied()
            .find(|id| widths[id.index()] == want)
            .expect("the seed pool covers every width the generator uses")
    }
    let one_bit = first_of_width(&idents, &widths, 1);

    for (kind, left, right) in &program.shape {
        let l = *left % idents.len();
        let r = *right % idents.len();
        let lw = widths[l];
        let rw = widths[r];
        let made = match kind % 13 {
            0 => Some((c.not(idents[l]), lw)),
            1..=4 => match lw == rw {
                true => Some((c.bit_and(idents[l], idents[r]).unwrap(), lw)),
                false => None,
            },
            5 => Some((c.mul(idents[l], idents[r]).unwrap(), lw + rw)),
            6 => Some((c.cat(idents[l], idents[r]).unwrap(), lw + rw)),
            7 => Some((c.replicate(idents[l], 2).unwrap(), lw * 2)),
            8 => match lw == rw {
                true => Some((c.ult(idents[l], idents[r]).unwrap(), 1)),
                false => None,
            },
            9 => match lw == rw {
                true => Some((c.ite(one_bit, idents[l], idents[r]).unwrap(), lw)),
                false => None,
            },
            10 => {
                let arms = vec![CaseArm::new(Bits::constant(0, rw).unwrap(), idents[l])];
                c.case_(idents[r], arms, idents[l]).ok().map(|id| (id, lw))
            }
            11 => Some((c.reg(idents[l], one_bit, one_bit, one_bit).unwrap(), lw)),
            _ => Some((
                c.instance("sub", Vec::new(), vec![idents[l], idents[r]], lw)
                    .unwrap(),
                lw,
            )),
        };
        if let Some((id, w)) = made {
            widths.push(w);
            idents.push(id);
        }
    }

    // Memories, which are the case with the write-port dependency bug.
    let en = first_of_width(&idents, &widths, 1);
    let eight = first_of_width(&idents, &widths, 8);
    for depth in [2u32, 4, 16] {
        let mem = c.mem(8, depth).unwrap();
        let addr = c.constant(Bits::constant(0, address_width_for_depth(depth)).unwrap());
        c.write_port(mem, eight, addr, en).unwrap();
        let port = c.read_port(mem, addr, en).unwrap();
        widths.push(8);
        idents.push(port);
    }

    // Close any loops the program asked for, ignoring the ones that cannot close.
    for &(from, to) in &program.loops {
        let from_id = idents[from % idents.len()];
        let to_id = idents[to % idents.len()];
        if widths[from % idents.len()] == widths[to % idents.len()] {
            let _ = c.drive(from_id, to_id);
        }
    }

    // Finally, drive every wire the graph actually reads. A generated program
    // refers to wires it never closed, which is exactly the "read of an undriven
    // wire" mistake `check` exists to catch — so the generator repairs it, and
    // `generated_graphs_pass_check` becomes the claim that a *fully wired* graph is
    // always accepted.
    //
    // "Reads" has to include a wire used as another wire's driver, because that
    // is an edge `check` follows and `operands` does not report. Missing it is how
    // a chain `a <- b` with `b` undriven slips through.
    let mut needed: Vec<NodeId> = Vec::new();
    let note = |id: NodeId, needed: &mut Vec<NodeId>| {
        if matches!(c.get(id).unwrap(), Node::Wire { .. }) && !needed.contains(&id) {
            needed.push(id);
        }
    };
    for id in c.node_ids() {
        for operand in c.get(id).unwrap().operands() {
            note(operand, &mut needed);
        }
        let driver = match c.get(id).unwrap() {
            Node::Wire { driver, .. } => driver.get(),
            _ => None,
        };
        if let Some(driver) = driver {
            note(driver, &mut needed);
        }
    }
    for wire in needed {
        let want = c.width_of(wire);
        // Donors must be driven nodes. Picking a wire here would just move the
        // problem: that wire would be referenced and undriven, which is the very
        // thing this pass exists to prevent.
        let donor = idents
            .iter()
            .copied()
            .find(|id| {
                *id != wire
                    && widths[id.index()] == want
                    && !matches!(c.get(*id).unwrap(), Node::Wire { .. })
            })
            .expect("the seed constants cover every width a wire can have");
        let _ = c.drive(wire, donor);
    }
    c
}

/// A random constant that fits in `width` bits.
fn rng_below(rng: &mut Lcg, width: u32) -> u64 {
    let modulus = 1u128 << width.min(64);
    (rng.next() as u128 % modulus) as u64
}

fn program() -> impl Strategy<Value = Program> {
    (
        tree(),
        prop::collection::vec((0usize..96, 0usize..96), 0..6),
        0u64..10_000,
    )
        .prop_map(|(shape, loops, seed)| Program { shape, loops, seed })
}

/// An independent recomputation of a node's width, written against the reference
/// rather than against `Node::width`, so a bug in the arena's derivation cannot
/// make this agree with itself.
fn reference_width(c: &Circuit, id: NodeId) -> u32 {
    let node = c.get(id).unwrap();
    match node {
        Node::Constant { value } => value.width(),
        Node::Wire { width, .. } => *width,
        Node::Not { arg } => reference_width(c, *arg),
        Node::BitAnd { left, .. }
        | Node::BitOr { left, .. }
        | Node::BitXor { left, .. }
        | Node::Add { left, .. }
        | Node::Sub { left, .. }
        | Node::UDiv { left, .. }
        | Node::SDiv { left, .. }
        | Node::URem { left, .. }
        | Node::SRem { left, .. } => reference_width(c, *left),
        Node::Mul { left, right } => reference_width(c, *left)
            .checked_add(reference_width(c, *right))
            .unwrap(),
        Node::Eq { .. }
        | Node::Ult { .. }
        | Node::Ule { .. }
        | Node::Ugt { .. }
        | Node::Uge { .. }
        | Node::Slt { .. }
        | Node::Sle { .. }
        | Node::Sgt { .. }
        | Node::Sge { .. } => 1,
        Node::Cat { high, low } => reference_width(c, *high)
            .checked_add(reference_width(c, *low))
            .unwrap(),
        Node::Replicate { value, count } => reference_width(c, *value).checked_mul(*count).unwrap(),
        Node::Ite { then_value, .. } => reference_width(c, *then_value),
        Node::Case { width, .. } => *width,
        Node::Reg { data, .. } => reference_width(c, *data),
        Node::Mem { data_width, .. } => *data_width,
        Node::ReadPort { data_width, .. } => *data_width,
        Node::Instance { output_width, .. } => *output_width,
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 96, ..ProptestConfig::default() })]

    /// Every node's width agrees with an independent recomputation.
    #[test]
    fn widths_match_an_independent_recomputation(program in program()) {
        let c = build(&program);
        for id in c.node_ids() {
            prop_assert_eq!(c.width_of(id), reference_width(&c, id), "width of {}", id);
        }
    }

    /// `check` never rejects a graph this generator produced.
    ///
    /// The generator only ever closes loops it can close, and leaves any wire it
    /// did not drive unreferenced, so a rejection is a real finding rather than a
    /// generator artefact.
    #[test]
    fn generated_graphs_pass_check(program in program()) {
        let c = build(&program);
        prop_assert!(c.check().is_ok(), "check rejected a generated graph: {:?}", c.check());
    }

    /// A topological order, when one exists, is genuinely valid.
    #[test]
    fn any_topological_order_is_valid(program in program()) {
        let c = build(&program);
        for relation in [Deps::LoopChecking, Deps::SimulationScheduling, Deps::WithoutCaseMatches] {
            let Ok(order) = c.topological_order(relation) else { continue };
            prop_assert_eq!(order.len(), c.len(), "order is not a permutation under {:?}", relation);
            let mut position = vec![usize::MAX; c.len()];
            for (i, &id) in order.iter().enumerate() {
                position[id.index()] = i;
            }
            for &id in &order {
                for dep in c.deps_in(id, relation).unwrap() {
                    prop_assert!(
                        position[dep.index()] < position[id.index()],
                        "{} must precede {} under {:?}", dep, id, relation
                    );
                }
            }
        }
    }

    /// A wire-free graph is already in evaluation order, because ids *are*
    /// construction order.
    #[test]
    fn a_wire_free_graph_is_already_ordered(program in program()) {
        let c = build(&program);
        if c.node_ids().any(|id| matches!(c.get(id).unwrap(), Node::Wire { .. })) {
            return Ok(());
        }
        let order = c.topological_order(Deps::LoopChecking).unwrap();
        prop_assert_eq!(order, c.node_ids().collect::<Vec<_>>());
    }

    /// Building the same program twice gives the identical graph.
    ///
    /// This is the property Hardcaml needs `normalize_uids` for, and the reason
    /// this crate can promise byte-identical Verilog.
    #[test]
    fn a_program_builds_identically_every_time(program in program()) {
        let first = build(&program);
        let second = build(&program);
        prop_assert_eq!(first.len(), second.len());
        for (a, b) in first.node_ids().zip(second.node_ids()) {
            prop_assert_eq!(first.width_of(a), second.width_of(b), "width of {} vs {}", a, b);
            prop_assert_eq!(
                format!("{:?}", first.get(a).unwrap().kind()),
                format!("{:?}", second.get(b).unwrap().kind())
            );
            prop_assert_eq!(
                first.deps_in(a, Deps::WithoutCaseMatches).unwrap(),
                second.deps_in(b, Deps::WithoutCaseMatches).unwrap(),
                "dependencies of {} vs {}", a, b
            );
        }
        prop_assert_eq!(
            first.topological_order(Deps::LoopChecking),
            second.topological_order(Deps::LoopChecking)
        );
    }

    /// Dependencies are always listed in the same fixed order for a given node
    /// kind, which is what lets the scheduler rely on the list.
    #[test]
    fn dependencies_are_deterministic(program in program()) {
        let c = build(&program);
        for relation in [Deps::LoopChecking, Deps::SimulationScheduling, Deps::WithoutCaseMatches] {
            for id in c.node_ids() {
                prop_assert_eq!(
                    c.deps_in(id, relation).unwrap(),
                    c.deps_in(id, relation).unwrap(),
                    "deps of {} under {:?} are not stable", id, relation
                );
            }
        }
    }

    /// A memory's write ports really are operands of the memory node.
    ///
    /// This is the regression test for the bug where `push_operands` treated `Mem`
    /// as a leaf under every relation, which silently hid every feedback path
    /// through a memory from `WithoutCaseMatches`.
    #[test]
    fn memory_write_ports_are_real_dependencies(program in program()) {
        let c = build(&program);
        let mut saw_memory = false;
        for id in c.node_ids() {
            let Node::Mem { write_ports, .. } = c.get(id).unwrap() else { continue };
            saw_memory = true;
            let ports = c.write_ports_of(id).unwrap().to_vec();
            prop_assert_eq!(write_ports, &ports);
            let deps = c.deps_in(id, Deps::WithoutCaseMatches).unwrap();
            for port in ports {
                for signal in port.signals() {
                    prop_assert!(deps.contains(&signal), "write port signal {} missing from deps of {}", signal, id);
                }
            }
            // ...and the memory is still terminal where it must be.
            prop_assert!(c.deps_in(id, Deps::LoopChecking).unwrap().is_empty());
        }
        prop_assert!(saw_memory, "generator produced no memories");
    }
}
