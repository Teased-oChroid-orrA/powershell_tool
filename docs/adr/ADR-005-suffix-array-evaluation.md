# ADR-005: Suffix Array Evaluation

Status: Rejected for initial architecture (revisit conditionally)

## Problem

Section 7 asks whether suffix-array-based structures (suffix arrays,
compressed suffix arrays, suffix trees/automata, BWT-derived structures)
offer an advantage over Tantivy/FM-index for exact/prefix/substring search,
using mature Rust implementations rather than a custom build.

## Evidence (verified 2026-08-23 against crates.io/GitHub, not training-data recall)

| Crate | Version | License | Last update | Windows | Notes |
|---|---|---|---|---|---|
| [`libsais`/`libsais-rs`](https://github.com/feldroop/libsais-rs) | tracks upstream libsais 2.10.4 | — | active | Pure-Rust+rayon build option available, no GCC/OpenMP required | **Construction-only** — produces a suffix array, not a queryable index; no incremental update path. Using it means hand-building a query layer on top, which Section 23 prohibits |
| [`divsufsort-rs`](https://crates.io/crates/divsufsort-rs) | 0.6.0 | MIT | 2026-03-29 | Unverified, no repo link on the crates.io listing | 231 lifetime downloads — negligible adoption signal, weaker than libsais-rs |
| [`sux`](https://crates.io/crates/sux) ([repo](https://github.com/vigna/sux-rs)) | 0.14.0 | Apache-2.0/LGPL-2.1+ | 2026-05-01 | Pure Rust, mmap-friendly | Succinct rank/select + Elias-Fano structures — general compressed-structure toolkit, not a substring-search answer by itself; built-once/serialize-then-query design, not designed to mutate |

## Decision

**Reject** a dedicated suffix-array search structure for this phase.
`libsais-rs` and `divsufsort-rs` are construction primitives, not queryable
indexes — adopting either would mean building custom index/query
infrastructure on top, which Section 23 explicitly prohibits. `sux` is the
one actively-maintained, Windows-safe, general-purpose candidate here, but
it answers a different question (compact succinct data structures) than
"fast substring search over a changing document corpus," and shares the
same built-once assumption.

## Consequences

- No suffix-array crate is vendored or benchmarked in this phase.
- `sux` is noted as a candidate worth a second look for a narrower future
  problem (e.g., a compact read-mostly metadata/facet structure) — not as a
  primary search mechanism. Not scheduled for evaluation now.

## Revisit conditions

Same as ADR-004: only after Tantivy benchmarking (Section 13) shows a real,
measured gap for a specific query pattern (e.g., arbitrary substring or
prefix search at very large corpus size) that justifies the added
complexity and the update-strategy cost this ADR flags.
