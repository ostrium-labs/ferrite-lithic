# 0011. Assert on cycle-indexed values, not rendered waveforms

- Status: Accepted
- Date: 2026-10-04
- Applies to: `ferrite-lithic`

## Context

Hardcaml's golden expectations are ASCII waveform renders from `hardcaml_waveterm`,
an external proprietary Jane Street library. We cannot reproduce that renderer and
should not try. The underlying per-cycle data, however, is independent of rendering.

## Decision

Tests compare cycle-indexed `Bits`. Waveform rendering, when `ferrite-lithic-wave`
lands, is presentation over the same data and is not part of any golden assertion.

We lose the ability to reuse Hardcaml's waveform goldens and write fresh stimulus
instead. Note no stimulus corpus is checked into Hardcaml either; it lives in inline
`[%expect]` blocks.

## Consequences

Golden assertions become machine-checkable and renderer-independent, at the cost of
not lifting existing waveform goldens directly.
