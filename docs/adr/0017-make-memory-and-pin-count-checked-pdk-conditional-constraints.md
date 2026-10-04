# 0017. Make memory and pin count checked, PDK-conditional constraints

- Status: Accepted
- Date: 2026-10-04
- Applies to: `ferrite-lithic`

## Context

SRAM availability differs by 4-8x across the three live open nodes, and the Tiny
Tapeout pin budget is a hard 26 with no negotiation. A DSL that hard-codes either
becomes non-portable, or silently produces an unplaceable design.

## Decision

`ferrite-lithic-ir` carries a memory abstraction that emits a macro instantiation
only when the selected PDK provides one (IHP `sg13g2_sram`, GF180MCU
`gf180mcu_fd_ip_sram`, sky130 `sky130_sram_macros`), and otherwise falls back to a
generated latch or DFF array.

I/O count is a checked constraint with a static error, not a runtime surprise. For
the competition chip this is load-bearing: 26 pins means a wide SIMD datapath must
serialise or time-multiplex across `uio[7:0]`.

The Tiny Tapeout wrapper module name `tt_um_<user>_<project>` and its exact port
list are generated automatically by the emitter. It is a mechanical, unforgiving
requirement and the most common submission failure.

## Consequences

Memory becomes conditional in the IR, which is more machinery than a hard-coded
macro but is the difference between a portable DSL and a single-node one. A
generated latch array costs area versus a macro and is the right default when no
macro exists or when DRC waivers are unavailable.
