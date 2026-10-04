# 0015. The SIMD instruction earns its place through bit-exactness

- Status: Accepted
- Date: 2026-10-04
- Applies to: `ferrite-lithic`

## Context

The original ambition was to accelerate Loams' vector search with a MAC array. An
audit of all ~293k lines found the entire arithmetic core is roughly 200 lines
across six functions, and Loams owns no ANN algorithm at all: it configures IVF-PQ
and delegates to `lance-index`, configures HNSW, SQ, PQ and BQ and delegates to
`qdrant-edge`. Aggregations have no numeric kernels and delegate to Tantivy. Text
processing is allocation- and Unicode-table-bound.

The one real kernel is `loams-query/src/vector.rs:20-44`, a row-major dense GEMV
with broadcast query, which is the textbook systolic-array input. It is also
deliberately shaped against that.

## Decision

The `SIMD` instruction ships narrow and width-parameterised, and its first
deliverable is **bit-exactness certification against
`loams-query/src/vector.rs:20-44`**, not a throughput claim.

Decision D79 in Loams makes summation order part of the public API contract: "f64
accumulation in index order, rounded to f32... SIMD kernels differ in summation
order between libraries". Certification must therefore cover subnormals, intermediate
infinities and `-0.0`, since `loams-hnsw/src/flat.rs` adds `+ 0.0` to normalise
negative zero and the sign of zero is observable in tie-breaking.

A bit-exact software kernel lands first and is benchmarked before silicon is
justified. The audit's own verdict was "do SIMD first, and measure"; there is no
committed search benchmark in Loams at all.

## Consequences

No throughput claim is made for Loams workloads. At the `dim 16` the ANN corpus
uses, the primary-key parse and the `BinaryHeap` sift in `TopK::push` cost tens of
nanoseconds against roughly 16 multiply-adds, so hardware would make that query
slower. The flip happens at high dimension and the production dimension
distribution is unknown.

The honest target for future Loams silicon is D92 sampled continuous recall: batch,
throughput-oriented, f64, and not yet implemented.
