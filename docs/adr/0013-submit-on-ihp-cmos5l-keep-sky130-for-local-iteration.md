# 0013. Submit on IHP CMOS5L, keep sky130 for local iteration

- Status: Accepted
- Date: 2026-10-04
- Applies to: `ferrite-lithic`

## Context

The rules specify the process verbatim: IHP 130nm CMOS5L through Tiny Tapeout, via
the `ttihp-verilog-template` `cmos5l` branch, tile size 6x4 in `info.yaml`. sky130
cannot be submitted.

sky130 is the most mature open PDK with the best-documented tooling and widest
example corpus, so it is valuable for iterating on the DSL regardless of what is
submitted.

## Decision

`ferrite-lithic-rtl` and the flow target CMOS5L for submission. sky130 is supported
for local iteration only and is never the submission path.

Both are 130nm-class but have different standard-cell libraries and DRC/LVS decks,
so cross-porting is real work rather than a config change.

Flow names as of 2026-10: OpenLane 2 was renamed LibreLane under FOSSi after
Efabless shut down; canonical repo `librelane/librelane`, current 3.0.x, which
supports both `ihp-sg13cmos5l` and sky130. PDK versioning is managed by `ciel`
(successor to volare). Yosys is 0.63 in OpenROAD-flow-scripts master.

## Consequences

Two flows to maintain. Accepted because iterating DSL ergonomics against the most
mature open PDK is worth more than single-target convenience, and the submission
target is fixed by the rules regardless.
