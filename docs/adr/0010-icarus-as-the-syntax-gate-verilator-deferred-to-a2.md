# 0010. Icarus as the syntax gate, Verilator deferred to A2

- Status: Accepted
- Date: 2026-10-04
- Applies to: `ferrite-lithic`

## Context

Every emitted design needs a cheap well-formedness check. Waiting for Verilator in
A1 means a heavier CI image or no gate at all during the phase producing most RTL.

Hardcaml's own RTL tests solve this: emit the Verilog, shell out to
`iverilog -t null -g2012` to prove it parses, snapshot the text. No extra simulator
install, and it catches malformed output immediately.

## Decision

CI runs `iverilog -t null -g2012` on emitted Verilog in A1, as Hardcaml does.

Verilator arrives in A2 with `ferrite-lithic-cosim`, and ADR-0018 records what
that harness had to decide about the cycle boundary and the reset state. That
harness does not exist
upstream: Verilator support lives in the separate `janestreet/hardcaml_verilator`
repository, so `ferrite-lithic-cosim` is net-new work. Natural first targets are
register and memory designs, which already have a definition and expected output.

## Consequences

The A1 gate is syntax, not semantics. Equivalence against a real simulator starts
in A2; until then `ferrite-lithic-sim` is validated against nothing but itself.
