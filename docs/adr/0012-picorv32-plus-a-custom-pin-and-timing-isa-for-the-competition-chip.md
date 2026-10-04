# 0012. PicoRV32 plus a custom pin and timing ISA for the competition chip

- Status: Accepted
- Date: 2026-10-04
- Applies to: `ferrite-lithic`

## Context

The competition asks for "a tiny CPU with an instruction set designed for reading
pins, writing pins, counting cycles, and hitting timing precisely". The CPU is the
deliverable. With 24 tiles at roughly 1K logic cells each, a small RISC-V core is a
small fraction of the budget. The question is whether to write our own.

## Decision

PicoRV32 (RV32I) as the core, plus custom instructions:

| Instruction | Purpose |
|---|---|
| `PIN_OUT` / `PIN_DIR` | drive and tri-state a pin |
| `PIN_SAMPLE rd, pin, delay` | sample after N cycles, enabling oversampling |
| `PIN_EDGE` with oversample config | edge detect, programmable rate |
| `CYCLE` | read the cycle counter |
| `WAIT_CYCLES` | delay |
| `SIMD` | 8x8 int8 MAC / dot product |

PicoRV32's licence must be confirmed before committing.

## Consequences

We do not own the core, so core-level novelty is unavailable. That is deliberate:
the core is not where differentiated work sits, and writing one would spend
schedule on the least interesting part of a 15-week design. If the licence proves
unsuitable the fallback is a second permissively licensed small RISC-V, not our own.
