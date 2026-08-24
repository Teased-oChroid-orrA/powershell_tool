# ADR-010: Multi-Index vs. Tantivy-Only Architecture

Status: Accepted

## Problem

Section 8 sketches a possible architecture with a query planner routing
across multiple specialized indexes (Tantivy + FM-index + suffix index) and
a result aggregator, but explicitly says: "Do not implement this
abstraction prematurely. First establish whether multiple indexes are
actually useful," preferring the simpler `.NET → Rust → Tantivy` pipeline
"if it meets requirements."

## Evidence

- ADR-004 (FM-Index Evaluation), ADR-005 (Suffix Array Evaluation), and
  ADR-006 (Bioinformatics Indexing Library Evaluation) each independently
  reached the same conclusion: every mature, Windows-buildable candidate in
  each category (`fm-index`, `libsais-rs`/`divsufsort-rs`, `bio`) is a
  **static, build-once, read-only structure with no incremental-update
  path** — a hard, structural disqualifier against Section 11's
  incremental-indexing requirement, not a performance question a benchmark
  could resolve either way.
- Because the disqualification is structural (no update API exists at all)
  rather than a measured performance gap, no candidate ever reached the
  point where a query-planner/aggregator layer would have anything to
  route to. There is currently exactly one adopted index type: Tantivy.

## Decision

Tantivy-only. No query planner, no multi-index result aggregator, no
abstraction layer for "which index should handle this query." The
architecture is the simple pipeline Section 8 itself calls out as
preferred when it's sufficient:

```
.NET → NativeSearchService → native_search.dll (C ABI) → Tantivy
```

## Consequences

- Zero premature abstraction — directly satisfies Section 8's own
  instruction and Section 23's "do not overengineer" list.
- If a future specialized index candidate emerges that *does* support
  incremental updates (the dealbreaker in every candidate evaluated so
  far), introducing it would be a new ADR revisiting ADR-004/005/006's
  verdicts with fresh evidence, and *then* this ADR would need revisiting
  too — but not before that evidence exists.
- All query semantics (Section 9: terms, phrases, boolean, fuzzy, field
  filters, ranges) route through Tantivy's own query parser and are
  already exercised by the structured-filter test in
  `engine::tests::structured_extension_filter_scopes_results` — no custom
  query language or routing logic was needed to get that working, matching
  Section 9's own preference.

## Rejected alternatives

The multi-index/query-planner architecture itself (Section 8's sketch) —
rejected for now, for the reason stated above: nothing has been adopted
that a planner would route to. This isn't a rejection of the *idea* so much
as a statement that the precondition for building it ("multiple indexes are
actually useful") has not been met, per ADR-004/005/006's evidence.
