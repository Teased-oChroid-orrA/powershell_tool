# ADR-001: Rust Native Search Boundary

Status: Proposed

## Problem

Issue #2 asks for a high-throughput, offline, incrementally-indexed,
relevance-ranked search subsystem. The existing app (`TextInFilesSearch.Core`)
does per-run line scanning over freshly-extracted text with no persistent
index or ranking. We need to decide where the Rust boundary sits relative to
the existing C# codebase before writing any Rust.

## Alternatives considered

1. **Rewrite the whole search/extraction pipeline in Rust**, exposing a broad
   FFI surface (file discovery, extraction, matching, everything).
2. **Rust owns extraction + indexing + search; .NET only calls in and renders
   results.**
3. **Rust owns indexing + search only; .NET keeps owning file discovery and
   text extraction, and hands already-extracted text to Rust to index.**

## Evidence

- `TextExtractionService.cs` (892 lines) already implements DOCX/PPTX/XLSX/
  ZIP(nested)/RTF/PDF extraction by hand, zero third-party packages, backed
  by 60 passing integration tests and several previously-fixed bug classes
  (ASCII85+FlateDecode PDF filter chain, nested-ZIP handling, Windows-1252
  fallback). See `docs/native-search-assessment.md`.
- `SearchOrchestrator.cs` already implements the parallel-processing,
  cancellation, progress-ticker, and incremental-cache behavior the epic
  asks for in Sections 11/13/17 — in C#, working, tested.
- The epic's own Section 23 ("Do Not Overengineer") explicitly prohibits
  duplicating mature/working functionality without justification, and its
  own Section 1 asks to "identify whether any existing functionality should
  be preserved, replaced, wrapped, or migrated" before writing Rust.
- `Core`'s zero-WinUI, zero-third-party-package property is a deliberate,
  documented architectural invariant (`CLAUDE.md`), not an accident to route
  around.

## Decision

Option 3. The Rust module (`native-search/`, new top-level crate) owns
**indexing and search only**:

```
.NET (existing extraction/matching pipeline, unchanged)
        │  already-extracted text + metadata
        ▼
NativeSearch.dll (Rust: Tantivy indexing/search behind a thin C ABI)
        │
        ▼
Persistent local index (%LOCALAPPDATA%, see ADR-007 — not yet written)
```

`TextInFilesSearch.Core` gains a new `NativeSearchService` alongside
`SearchOrchestrator`, calling into the native DLL via P/Invoke. Nothing in
the WinUI head project changes to support this — the seam is entirely inside
`Core`, preserving the "zero WinUI dependency in Core" invariant.

Text extraction ownership is **not** migrated to Rust in this pass. Re-derive
this decision only if a concrete need appears (e.g., a background indexer
that runs without the .NET UI extracting text first) — not preemptively.

## Consequences

- Fast path to a working vertical slice (Section 25, Phase 3): Rust only
  needs to accept `(doc_id, path, extracted_text, metadata)` and index it —
  no PDF/DOCX/ZIP parsing to port and re-verify in Rust first.
- The existing 60-check test harness and bug-fix history for extraction stay
  authoritative; Rust adds a new, separately-tested surface rather than
  replacing a tested one.
- Two search paths will coexist for a while: the existing per-run line scan
  (`MatchingEngine`) and the new indexed/ranked search. Reconciling or
  merging these into one UI concept is explicitly deferred — see the open
  item in `docs/native-search-assessment.md`.
- FFI surface stays narrow (index/search operations only), which also
  directly serves Section 16's requirement for a narrow, stable ABI.

## Rejected alternatives

- **Option 1 (full rewrite)** — rejected: throws away tested, working,
  bug-fixed extraction code for no demonstrated benefit; directly conflicts
  with Section 23.
- **Option 2 (Rust owns extraction too)** — rejected for now: the epic's own
  "trust but verify" principle applies equally to the claim that Rust
  extraction crates will be a net improvement over the existing hand-rolled
  extraction, which is unverified and out of scope for this phase. Revisit
  once a vertical slice proves indexing/search alone is worth the FFI
  investment.
