# 0002. Prefix crate names with ferrite-

- Status: Accepted
- Date: 2026-10-04
- Applies to: `ferrite-lithic`

## Context

The working names `lithic` and `strata` are permanently unavailable on crates.io.
Names are never released once claimed, so this cannot be fixed later. Checked
against the crates.io API on 2026-10-04:

| Name | Status | Occupant |
|---|---|---|
| `lithic` | taken 1.0.1 | mod manager for Vintage Story, 16 downloads |
| `strata` | taken 0.1.1 | "a unique search technology", 3,647 downloads |
| `strata-core` | taken 0.1.0 | algebraic trait hierarchy, 742 downloads |
| `ferrite` | taken 0.1.28 | image viewer, 18,355 downloads |

Every `lithic-*` component name plus `strata-runtime` and `strata-plugin-api`
were free.

Both bare names also collide with established projects elsewhere: `lithic` is a
card-issuing fintech at lithic.com, `strata` is OpenGamma Strata plus a popular AI
repository. GitHub org names were free, so the collision is specifically the crate
namespace and search.

## Decision

Repositories `ostrium-labs/ferrite-lithic` and `ostrium-labs/ferrite-strata`.
Crates `ferrite-lithic`, `-bits`, `-ir`, `-sim`, `-rtl`, `-derive`, `-wave`,
`-cosim`, `-tb`, and `ferrite-strata`, `-runtime`, `-plugin-api`, `-cpu`,
`-cubecl`, `-pytorch`. Repository and crate names match so the suite reads as one.

## Consequences

No top-level `ferrite` crate is available, so the suite's identity lives in the
prefix. Acceptable; nothing in either design needs one. Naming is settled.
