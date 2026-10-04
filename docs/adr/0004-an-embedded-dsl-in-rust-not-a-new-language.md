# 0004. An embedded DSL in Rust, not a new language

- Status: Accepted
- Date: 2026-10-04
- Applies to: `ferrite-lithic`

## Context

A separate HDL needs its own compiler, type system, editor support and community.
Spade and others already cover that route. Hardcaml's model lets the host
language's type system, package manager and CI do work a new language would have
to rebuild.

## Decision

`ferrite-lithic` is a Rust library called from Rust. `Signal` is a handle type
with operator-overload traits; the graph is built by ordinary function calls.
Simulation is a native library, not a generated executable, so designs debug with
normal host tooling.

## Consequences

Users cannot get a syntax error without also getting a Rust type error. We compete
for Rust users specifically.
