# ADR-006: Bioinformatics Indexing Library Evaluation

Status: Rejected for initial architecture (revisit conditionally)

## Problem

Section 6 asks whether mature bioinformatics-derived indexing frameworks
(FM-index/BWT/suffix-array/succinct-structure ecosystems built for genomic
search) have algorithms applicable to general document search here — without
assuming a bioinformatics library is appropriate merely because it's fast.

## Evidence (verified 2026-08-23 against crates.io/GitHub, not training-data recall)

| Crate | Version | License | Last update | Windows | Adoption signal | Incremental? |
|---|---|---|---|---|---|---|
| [`bio`](https://crates.io/crates/bio) (rust-bio) ([repo](https://github.com/rust-bio/rust-bio)) | 4.0.1 | MIT | 2026-06-29 | No blockers noted | 1M+ downloads — well-maintained, real adoption | **No** — its `data_structures` module builds suffix array → BWT → FM-index as one static pipeline; no update path |

`bio` is the canonical general bioinformatics/indexing framework in the Rust
ecosystem and the only one surfaced with meaningful adoption. Its relevant
indexing structures are the same static suffix-array/BWT/FM-index family
already evaluated in ADR-004/ADR-005, inheriting the identical dealbreaker:
built once from a fixed sequence, queried read-only, no incremental update.

## Decision

**Reject** for the reason common to ADR-004 and ADR-005: `bio`'s indexing
structures require full-corpus rebuild on any change, conflicting with
Section 11's incremental-indexing requirement. The crate's genomics-specific
functionality (alignment, k-mer counting, sequence I/O) is also not
applicable to general document text search and would be dead weight in the
dependency tree.

## Consequences

- No bioinformatics-framework crate is vendored in this phase.
- All three specialized-index avenues (FM-index, suffix array,
  bioinformatics framework — ADR-004/005/006) converge on the same finding:
  no mature, Windows-buildable, **incrementally-updatable** compressed-text
  index exists in the current Rust ecosystem. Tantivy-only is the correct
  initial architecture (ADR-001/ADR-002), not an accepted shortcut.

## Revisit conditions

Same as ADR-004/ADR-005 — only after Tantivy benchmarking demonstrates a
concrete, measured gap that justifies the complexity and update-strategy
cost these structures all carry.
