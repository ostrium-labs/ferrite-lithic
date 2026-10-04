//! Arena indices. A [`NodeId`] *is* a node's identity.

use core::fmt;

/// A handle to a node in a [`Circuit`](crate::Circuit).
///
/// The arena index is the identity, and that is the whole point of the design.
/// Hardcaml gives nodes identity from a **process-global mutable counter**
/// (`kernel/signal__type.ml:966`), which is neither thread-safe nor
/// deterministic; the recorded plan for this crate replaces it with an arena
/// index, and the design audit confirmed the arena is effectively required
/// rather than merely convenient — a plain immutable Rust tree cannot express a
/// feedback loop at all.
///
/// Three consequences follow, and all three are load-bearing:
///
/// - **Node ids are construction order.** Two runs of the same program allocate
///   the same ids, so byte-identical Verilog is structural rather than something
///   defended by a normalisation pass. Hardcaml needs `normalize_uids` for this
///   and ships it **off by default with its own test commented out as "brittle"**
///   (`test/lib/test_uid_normalization.ml:56-59`).
/// - **There is no global state**, so two `Circuit`s can be built on two threads
///   at once. With a global counter they cannot.
/// - **Ids are dense and ordered**, so arena order is a valid evaluation order to
///   start from, and a topological sort only has to reorder what cycles force.
///
/// A `NodeId` is only meaningful for the [`Circuit`](crate::Circuit) that minted
/// it. Handing one to a different circuit is a programming error, not a runtime
/// condition, and is not checked — the ids are `u32` indices and there is nothing
/// to validate cheaply. In practice the DSL layer never stores a `NodeId` longer
/// than the expression that produced it.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId(u32);

impl NodeId {
    /// The arena index, for indexing `Vec` storage directly.
    #[must_use]
    pub const fn index(self) -> usize {
        self.0 as usize
    }

    /// Build an id from an arena index.
    ///
    /// # Panics
    ///
    /// If `index` does not fit in a `u32`, which no arena can reach in practice:
    /// the allocation would fail first.
    #[must_use]
    pub fn from_index(index: usize) -> Self {
        assert!(
            u32::try_from(index).is_ok(),
            "arena index {index} exceeds the u32 id space"
        );
        Self(u32::try_from(index).expect("checked immediately above"))
    }

    /// The id as a `u32`, for serialising the graph.
    #[must_use]
    pub const fn as_u32(self) -> u32 {
        self.0
    }
}

impl fmt::Debug for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "n{}", self.0)
    }
}

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "n{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::NodeId;

    #[test]
    fn index_round_trips() {
        for index in [0usize, 1, 63, 64, 4096, u32::MAX as usize] {
            let id = NodeId::from_index(index);
            assert_eq!(id.index(), index);
            assert_eq!(id.as_u32() as usize, index);
        }
    }

    #[test]
    fn ids_order_by_construction_order() {
        let ids: Vec<NodeId> = (0..8usize).map(NodeId::from_index).collect();
        let mut shuffled = ids.clone();
        shuffled.reverse();
        shuffled.sort();
        assert_eq!(shuffled, ids);
    }

    #[test]
    fn debug_is_short_and_stable() {
        // Emitted Verilog and error messages both use this, so the format is part
        // of the observable surface, not an implementation detail.
        assert_eq!(format!("{:?}", NodeId::from_index(7)), "n7");
        assert_eq!(format!("{}", NodeId::from_index(7)), "n7");
    }
}
