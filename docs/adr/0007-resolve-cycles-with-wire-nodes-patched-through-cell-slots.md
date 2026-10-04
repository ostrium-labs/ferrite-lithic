# 0007. Resolve cycles with wire nodes patched through Cell slots

- Status: Accepted
- Date: 2026-10-04
- Applies to: `ferrite-lithic`

## Context

Combinational cycles must be legal in a hardware DSL, because a ring oscillator or
a latch built from combinational feedback is a legitimate design.

Hardcaml solves this with one mechanism: `Wire` is the only node carrying a mutable
field, so it is the only possible back edge. Deferred resolution is
`Signal_graph.rewrite`, which creates unattached replacement wires, registers the
mapping, recursively rewrites drivers, then re-attaches. We need the same
three-phase shape, but the arena changes the representation.

## Decision

The arena holds `driver: Cell<Option<NodeId>>` for wire nodes. `Cell` not
`RefCell`: we never hand out references into the arena, so no runtime borrow
checking is needed and the graph stays cheap to traverse.

Wire-to-driver aliasing resolves once at build time, so a driven wire costs nothing
in simulation.

Three distinct dependency relations are an enum parameter on traversal rather than
a trait object, keeping cases monomorphised. They are semantically visible: a
combinational loop through a register is legal, through a memory read port is not,
into a memory write port is.

## Consequences

Interior mutability makes the arena single-threaded by construction, which is
acceptable for graph construction. `Rc<RefCell<..>>` per node was rejected as it
makes traversal expensive for no benefit.
