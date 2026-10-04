# 0001. Apache-2.0 for the Ferrite projects

- Status: Accepted
- Date: 2026-10-04
- Applies to: `ferrite-lithic`

## Context

Jane Street's ASIC competition requires open-source submissions, and Strata's
premise is that third-party hardware vendors link against our plugin ABI without
reading our source.

MIT is simpler and matches Hardcaml, but carries no patent grant. The first
question a vendor's legal review asks about a closed-source plugin boundary is
patent exposure, and leaving it open is a needless adoption risk on the one
surface where adoption is the goal.

## Decision

Both `ferrite-lithic` and `ferrite-strata` are Apache-2.0, copyright appendix
filled as "Copyright 2026 The Ferrite Authors".

`NOTICE` in each repo records design references with no code copied: Hardcaml for
the signal-graph model, PJRT/OpenXLA, CubeCL and Burn for the runtime interface.

## Consequences

Vendors get an explicit patent grant, which is the point. Dual-licensing Lithic
more closely to Hardcaml remains available as a per-crate decision after A1.
