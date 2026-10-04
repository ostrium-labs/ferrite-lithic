# 0016. Claim no acceleration for graph, text or analytical workloads

- Status: Accepted
- Date: 2026-10-04
- Applies to: `ferrite-lithic`

## Context

A broad story would be easy and unsupported:

- Graph: no traversal exists in Loams; it delegates to `qdrant-edge` and, for the
  FPGA path, Grafeo.
- Analytical: `loams-query/src/exec/aggs.rs` contains no numeric kernel. Sum, avg,
  cardinality, percentiles, histogram and date_histogram are all Tantivy collectors.
- Text: the Porter stemmer allocates a `Vec<char>` per word; tokenisation walks
  Unicode word-boundary tables; the rest is posting-list traversal.
- Vector quantisation: only configuration. PQ, SQ, BQ and RaBitQ live in
  exact-pinned `lance = "=12.0.0"` and `qdrant-edge = "=0.8.0"` under a documented
  lockstep policy.

## Decision

The entry makes no acceleration claim for graph, text search or analytical
workloads, in the submission or any public description.

Sparse scoring and the sparse weight codec are recorded as SIMD-only targets: both
are branch-dominated over small trip counts, which is what SIMD cannot fix and what
an ASIC would still pay for in a merge network.

## Consequences

Narrower claim, stronger evidence. Overclaiming loses to a smaller honest entry,
and the judges have said they want write-ups others can reference.

One software bug found while auditing should be fixed regardless of hardware:
`loams-qdrant/src/scoring.rs:434-442` computes `key(i)` and
`key(remaining[best])` inside the `argmax` comparison, recomputing a
dimension-length dot product per element inside an already `O(limit * n * dim)`
loop.
