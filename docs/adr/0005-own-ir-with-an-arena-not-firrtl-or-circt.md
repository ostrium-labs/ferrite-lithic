# 0005. Own IR with an arena, not FIRRTL or CIRCT

- Status: Accepted
- Date: 2026-10-04
- Applies to: `ferrite-lithic`

## Context

Emitting FIRRTL or CIRCT would reuse their optimisers and Verilog backends, but
adds a heavy dependency, ties our release train to theirs, and puts an
MLIR-based toolchain between the DSL and the netlist.

The plan's decision 3 says to decide at the end of A2 with real designs in hand.
We honour that by building the own-IR version first and deciding with evidence.

## Decision

`ferrite-lithic-ir` is an arena: `Vec<Node>` with `NodeId` newtypes, node ids
being construction order by definition.

The research confirmed this is effectively required, not merely convenient.
Hardcaml has no arena and no node ids at construction time; a node is an OCaml
variant over already-constructed children with identity from a process-global
mutable counter. Cycles are legal only through a single mutable `Wire` node, the
only possible back edge. A plain immutable Rust tree cannot express that.

## Consequences

We own the IR, its validation passes and its lowering, and maintain optimisation
passes ourselves. Revisit at end of A2.
