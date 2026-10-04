# 0003. Runtime widths, not const generics

- Status: Accepted
- Date: 2026-10-04
- Applies to: `ferrite-lithic`

## Context

Const-generic widths give compile-time checking, which is valuable. The cost is
that graph construction inside loops and generators becomes painful, and a
hardware DSL is mostly loops and generators: memory decoders, crossbars,
systolic tiles.

Hardcaml made the same trade, keeping widths as runtime integers with a stored
width field and masking after every operation.

## Decision

Widths are runtime `u32`. `Bits` carries its width and masks unused upper bits
after every arithmetic operation, because every downstream operation silently
depends on that invariant.

Error messages must carry the compensation: name both widths and say an explicit
widening operation exists. This is the most common source of hardware bugs, so the
message is part of the design.

## Consequences

No compile-time width checking. We buy it back with error message quality and
property tests covering small widths explicitly.
