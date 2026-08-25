# ADR-004: FM-Index Evaluation

Status: Rejected for initial architecture (revisit conditionally)

## Problem

Section 5 asks whether an FM-index provides a meaningful advantage over
Tantivy for substring/exact-text search, and to evaluate mature Rust
implementations rather than assuming the answer.

## Evidence (verified 2026-08-23 against crates.io/GitHub, not training-data recall)

| Crate | Version | License | Last update | Windows | Incremental update? |
|---|---|---|---|---|---|
| [`fm-index`](https://crates.io/crates/fm-index) ([repo](https://github.com/ajalab/fm-index)) | 0.3.1 | MIT/Apache-2.0 | 2026-06-29 | No known blockers; pure-Rust deps (num-traits, serde, vers-vecs) | **No — static, built once from an immutable text blob** |
| [`lt-fm-index`](https://crates.io/crates/lt-fm-index) | — | — | — | unverified | Same static-index family (k-mer lookup table FM-index) |

Both candidates share the defining structural property of every FM-index:
the Burrows-Wheeler-transformed representation is built once from a fixed
text corpus and queried read-only. Neither exposes an add/update/delete API.

## Decision

**Reject** an FM-index for this phase. The static-build/read-only property
directly conflicts with Section 11's incremental-indexing requirement (files
are added, modified, and deleted continuously in a live filesystem). Using
one would require either full-corpus rebuild on every file change
(unacceptable at any real corpus size — violates Section 13's throughput
goals) or hand-building an update/merge layer on top of a static structure —
which Section 23 explicitly prohibits ("do not implement a custom...
FM-index... unless benchmarking demonstrates a need," "prefer mature library
+ thin adapter").

## Consequences

- No FM-index crate is vendored or benchmarked in this phase.
- Tantivy (ADR-002) remains the sole search engine for the vertical slice.

## Revisit conditions

Re-open only if a future benchmark (Section 13) shows Tantivy's substring/
exact-match query latency is an actual, measured bottleneck for a real
workload — and even then, budget for building a periodic-rebuild strategy
(a read-only secondary index refreshed on a schedule, alongside Tantivy's
incrementally-updated one) rather than expecting off-the-shelf incremental
FM-index support to appear.
