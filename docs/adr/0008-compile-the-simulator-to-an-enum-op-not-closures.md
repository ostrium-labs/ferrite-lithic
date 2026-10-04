# 0008. Compile the simulator to an enum Op, not closures

- Status: Accepted
- Date: 2026-10-04
- Applies to: `ferrite-lithic`

## Context

Hardcaml compiles each scheduled node to a zero-argument OCaml closure capturing
word addresses. Closures are natural in OCaml and a poor fit in Rust, where they
mean `dyn` or boxing.

More importantly the op list is what we want to inspect. A
simulator-versus-emitted-Verilog differential test must compare what the simulator
computed against what the netlist computes, and an opaque closure list cannot be
dumped or diffed.

## Decision

`ferrite-lithic-sim` compiles to `Vec<Op>` with word-address fields, interpreted by
a `match`. Same execution model: one settle pass per `comb()` in precomputed
topological order, two-phase register update with a shadow section and a blit,
three-phase stepping with before/after output snapshots, zero-initialised registers.

Combinational loops are a hard error, never an iterative relaxation fallback.

## Consequences

No `dyn` in the hot loop. The op list is inspectable, dumpable and diffable, which
is what makes the differential test possible.
