//! The three dependency relations, and why there are three.

use crate::node::Node;

/// Which edges a traversal should follow.
///
/// # One relation is not enough, and the difference is observable
///
/// Hardcaml parameterises traversal over a functor and instantiates it three ways
/// (`src/signal_graph.ml:258-325`). The table, with the rows this crate's node set
/// covers:
///
/// | Relation | `Reg` | `Mem` | `Instance` | `Case` match constants |
/// |---|---|---|---|---|
/// | [`Deps::LoopChecking`] | terminal | terminal | terminal | followed |
/// | [`Deps::SimulationScheduling`] | terminal | terminal | **followed** | followed |
/// | [`Deps::WithoutCaseMatches`] | **followed** | **followed** | followed | dropped |
///
/// The consequences are specific, and each one is a test in `tests/relations.rs`:
///
/// - **A combinational loop through a register is legal**
///   (`test/lib/test_combinational_loop.ml:152-182` in Hardcaml). `Reg` is
///   terminal for loop checking, so a cycle through one is not a cycle at all.
/// - **A loop into a memory write port is legal** (`:270-313`). `Mem` is terminal
///   too. Feedback *into* a memory is a normal accumulator, not a combinational
///   loop.
/// - **A loop through a memory read port is not legal** (`:243-268`). Read ports
///   are ordinary combinational nodes, and the offending cycle is
///   `read port -> address logic -> read port`, which never needs the memory's own
///   write ports to be followed. So this is caught even though `Mem` is terminal.
///   That is the non-obvious part: the two memory cases differ only because the
///   read port is a separate node in the first place.
/// - **An `Instance` differs between the first two relations.** It is terminal for
///   loop checking and followed for simulation scheduling, so an instance is
///   treated as a combinational boundary by the check and as something with
///   ordered internals by the scheduler.
///
/// We implement this as an enum parameter rather than a trait object so the three
/// cases stay monomorphised, which is the reason the design notes chose an enum.
///
/// # The `Case` match-constant row is inert in A1, and that is honest
///
/// Hardcaml's `Case` matches against signals, so "following the match constants"
/// means traversing edges that do not exist in our graph. Ours match against
/// [`Bits`](ferrite_lithic_bits::Bits) literals, so there is nothing to follow and
/// [`Deps::includes_case_matches`] cannot change any result. It is kept because
/// dropping the dimension now and re-deriving it later would mean revisiting every
/// call site; if match constants ever become signals — a case on a computed
/// opcode, say — the accessor is already the place that decides.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Deps {
    /// Combinational loop checking. Registers, memories and instances terminate
    /// the traversal.
    LoopChecking,
    /// Simulation scheduling. Registers and memories terminate the traversal;
    /// instances do not.
    SimulationScheduling,
    /// The "follow everything" relation used by rewriting, where a register's
    /// inputs and a memory's write ports are ordinary combinational predecessors.
    WithoutCaseMatches,
}

impl Deps {
    /// A short name for the relation, for diagnostics.
    ///
    /// An error has to say *which* notion of a cycle was violated, because the
    /// three genuinely disagree: a loop through a register is legal for loop
    /// checking and is a scheduling loop for simulation.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::LoopChecking => "loop-checking",
            Self::SimulationScheduling => "simulation-scheduling",
            Self::WithoutCaseMatches => "without-case-matches",
        }
    }
}

impl Deps {
    /// Whether the traversal should recurse into this node's inputs.
    ///
    /// A terminal node is a leaf: the traversal stops there and does not consider
    /// what feeds it. For loop checking that is the whole point — a register's
    /// input is not part of the combinational cone, so feedback through a
    /// register does not make a cycle.
    #[must_use]
    pub const fn is_followed(self, node: &Node) -> bool {
        !matches!(
            (self, node),
            (
                Deps::LoopChecking,
                Node::Reg { .. } | Node::Mem { .. } | Node::Instance { .. }
            ) | (
                Deps::SimulationScheduling,
                Node::Reg { .. } | Node::Mem { .. }
            )
        )
    }

    /// Whether `Case` match constants are part of the dependency set.
    ///
    /// Always observable as `false` for [`Deps::WithoutCaseMatches`] and `true`
    /// otherwise, but see the type-level note: with literal match constants it
    /// changes no result in A1.
    #[must_use]
    pub const fn includes_case_matches(self) -> bool {
        !matches!(self, Deps::WithoutCaseMatches)
    }
}

#[cfg(test)]
mod tests {
    use super::Deps;
    use crate::{Node, NodeId, WritePort};

    fn reg() -> Node {
        Node::Reg {
            data: NodeId::from_index(0),
            clock: NodeId::from_index(1),
            reset: NodeId::from_index(2),
            clear: NodeId::from_index(3),
        }
    }

    fn mem() -> Node {
        Node::Mem {
            data_width: 8,
            depth: 4,
            write_ports: Vec::new(),
        }
    }

    fn inst() -> Node {
        Node::Instance {
            name: "sub".to_string(),
            params: Vec::new(),
            inputs: vec![NodeId::from_index(0)],
            output_width: 8,
        }
    }

    fn add() -> Node {
        Node::Add {
            left: NodeId::from_index(0),
            right: NodeId::from_index(1),
        }
    }

    #[test]
    fn the_three_relations_differ_where_the_research_says_they_do() {
        // The table in the type docs, asserted. Reg is terminal for the two
        // relations that reason about hardware timing, and followed only by the
        // "follow everything" relation used for rewriting.
        assert!(!Deps::LoopChecking.is_followed(&reg()));
        assert!(!Deps::SimulationScheduling.is_followed(&reg()));
        assert!(Deps::WithoutCaseMatches.is_followed(&reg()));
        assert!(!Deps::LoopChecking.is_followed(&mem()));
        assert!(!Deps::SimulationScheduling.is_followed(&mem()));
        assert!(Deps::WithoutCaseMatches.is_followed(&mem()));

        assert!(!Deps::LoopChecking.is_followed(&inst()));
        assert!(Deps::SimulationScheduling.is_followed(&inst()));
        assert!(Deps::WithoutCaseMatches.is_followed(&inst()));

        // A plain combinational node is followed under every relation, or loop
        // checking would be vacuous.
        for relation in [
            Deps::LoopChecking,
            Deps::SimulationScheduling,
            Deps::WithoutCaseMatches,
        ] {
            assert!(
                relation.is_followed(&add()),
                "{relation:?} should follow Add"
            );
        }
    }

    #[test]
    fn read_ports_are_combinational_under_every_relation() {
        // This is what makes "a loop through a memory read port is not legal" and
        // "a loop into a memory write port is" both true at once: the read port is
        // followed, the memory is terminal.
        let port = Node::ReadPort {
            mem: NodeId::from_index(0),
            address: NodeId::from_index(1),
            enable: NodeId::from_index(2),
            data_width: 8,
        };
        for relation in [
            Deps::LoopChecking,
            Deps::SimulationScheduling,
            Deps::WithoutCaseMatches,
        ] {
            assert!(
                relation.is_followed(&port),
                "{relation:?} should follow ReadPort"
            );
        }
    }

    #[test]
    fn only_without_case_matches_drops_match_constants() {
        assert!(Deps::LoopChecking.includes_case_matches());
        assert!(Deps::SimulationScheduling.includes_case_matches());
        assert!(!Deps::WithoutCaseMatches.includes_case_matches());
    }

    #[test]
    fn a_memory_with_write_ports_is_still_judged_by_its_kind_not_its_ports() {
        let m = Node::Mem {
            data_width: 8,
            depth: 2,
            write_ports: vec![WritePort {
                data: NodeId::from_index(0),
                address: NodeId::from_index(1),
                enable: NodeId::from_index(2),
            }],
        };
        assert!(!Deps::LoopChecking.is_followed(&m));
        assert!(!Deps::SimulationScheduling.is_followed(&m));
        assert!(Deps::WithoutCaseMatches.is_followed(&m));
    }
}
