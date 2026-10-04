# 0006. Port Hardcaml's design, write tests from behaviour

- Status: Accepted
- Date: 2026-10-04
- Applies to: `ferrite-lithic`

## Context

Hardcaml is MIT licensed (Jane Street Group, `LICENSE.md` and `license: "MIT"` in
`hardcaml.opam`, revision `ff4fe60`, `v0.18~preview`), so copying is permitted with
attribution. The plan's decision 4 says to port the design and write tests from
behaviour anyway.

Two reasons to prefer that even where copying is legal: Hardcaml relies on OCaml
features with no clean Rust equivalent, so a copy would not compile anyway, and the
behaviour is the specification we actually want to assert.

## Decision

`NOTICE` records Hardcaml as the design reference at revision `ff4fe60` and states
no source has been copied. The entry is written to be extended if code or test
vectors are ever derived rather than rewritten, listing specific files and revision
and retaining the MIT notice.

## Consequences

If code or vectors are derived later, `NOTICE` must be updated in the same commit.
That obligation comes from choosing MIT-compatible prior art.
