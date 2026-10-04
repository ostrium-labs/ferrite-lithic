# 0009. Add integer division and remainder

- Status: Accepted
- Date: 2026-10-04
- Applies to: `ferrite-lithic`

## Context

Hardcaml has no integer division or remainder at any layer: not in the operator
interface, not in the signal op enum, not in the simulator op list, not in the RTL
binop enum. The only `/` and `%` are IEEE float, and those are simulation-only
opaque ops that cannot be synthesised.

A real datapath needs division for constant-reciprocal tricks, so this is worth
closing rather than inheriting.

## Decision

`udiv`, `sdiv`, `urem` and `srem` are genuine primitives in `ferrite-lithic-bits`,
the IR, the simulator and the Verilog emitter, documented as inferring large
dividers rather than being cheap.

## Consequences

We diverge from Hardcaml here, so behaviour is specified by tests against a
big-integer reference rather than a Hardcaml golden vector. That is the one place
our oracle is not Hardcaml.
