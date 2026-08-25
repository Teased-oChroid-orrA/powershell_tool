# ADR-008: Incremental Indexing Strategy

Status: Accepted (implemented in `native-search/src/engine.rs`)

## Problem

Section 11 asks for incremental indexing tracking id/path/size/modified/
content-hash/extractor-version/schema-version, handling new/modified/
deleted/renamed/inaccessible/unsupported files, with corrupted documents
never aborting a whole indexing run.

## Evidence

- ADR-002 item 6 (confirmed against source): Tantivy documents are
  immutable. There is no in-place "update field X of document Y" API —
  the only path to changing a document's content is delete-by-term, then
  add a new document.
- This app already has a working, tested change-detection mechanism for a
  related but distinct problem: `CacheService` (in `TextInFilesSearch.Core`)
  fingerprints files by path + size + modified-time to decide whether
  *extraction* needs to re-run. That's a different question than "does the
  search index need updating," but the pattern (caller decides what
  changed, based on cheap filesystem metadata, without native_search doing
  its own filesystem stat-ing) is directly reusable.
- `engine::NativeSearchEngine::index_document` already deletes any existing
  document with the same `id` before adding the new one (see its doc
  comment) — this was implemented as part of the Phase 3 vertical slice,
  before this ADR was written up formally; this ADR records that decision
  rather than introducing a new one.

## Decision

- **Stable ID**: the caller (.NET) supplies the `id` for every document.
  The recommended scheme (not yet enforced by any validation on the Rust
  side) is the file's normalized absolute path — stable across re-runs,
  unique per file, and requires no extra bookkeeping. A content hash was
  considered and rejected as the ID itself (see Rejected alternatives).
- **Change detection lives in .NET, not native-search**. `native-search`
  does not stat files, compare timestamps, or hash content — it only
  knows "index this text under this id" and "delete this id." Whether a
  given file actually needs re-indexing is exactly the kind of decision
  `CacheService`'s existing fingerprinting already makes for extraction;
  the eventual caller of `NativeSearchService.IndexDocument` should reuse
  that same signal (or a close variant of it) rather than native-search
  re-inventing file-change detection.
- **Update = delete-by-id + re-add**, always, unconditionally, every time
  `index_document` is called for a given id — this is what Tantivy's
  immutable-document model requires, and it's cheap enough (a `delete_term`
  call is O(1) to enqueue; the actual removal happens on the next merge)
  that there's no need for the caller to first check "does this id already
  exist" before indexing.
- **Deleted files**: the caller calls `DeleteDocument(id)` when a
  previously-indexed file no longer exists or no longer matches whatever
  criteria put it in the index. Detecting "this file is gone" is a
  filesystem-walk concern that belongs in .NET (mirroring how
  `SearchOrchestrator`'s directory walk already works), not something
  native-search can discover on its own — it has no visibility into the
  filesystem at all (ADR-001).
- **Renamed files**: treated as delete(old id) + index(new id under the new
  path). No rename-detection heuristic (e.g. content-hash matching to infer
  a rename) is implemented — Section 11 itself only asks for this "where
  detectable," and detecting it would require the caller to notice a
  disappeared path and a newly-appeared file with matching content, which
  is out of scope for this pass.
- **One bad document never aborts a run**: proven, not just designed —
  `engine::tests::a_rejected_document_does_not_affect_documents_indexed_around_it`
  shows a failed `index_document` call (e.g. empty id) leaves the engine
  fully usable for subsequent calls. Each FFI call is independent by
  construction (no shared mutable state that a failed call could corrupt
  beyond the attempted document itself), so this property holds by design,
  not by a special-cased error-recovery path.

## Consequences

- The .NET-side caller (whoever eventually wires `NativeSearchService` into
  a real indexing run) owns: walking the filesystem, deciding what changed,
  generating a stable id per file, and calling `IndexDocument`/
  `DeleteDocument` accordingly. `native-search` stays a pure indexing/search
  primitive with no filesystem or change-tracking responsibilities, per
  ADR-001.
- Content-hash tracking (Section 11's "content hash" field) is not
  implemented anywhere yet — neither as the id nor as a stored field. If a
  future need arises to detect "file path unchanged but content changed
  without a modified-time bump," that's a gap to close in the .NET-side
  caller's change-detection logic, not in native-search.

## Rejected alternatives

- **Content hash as the document id** — rejected: a hash changes every
  time the file's content changes, which would mean "updating" a document
  is actually indexing it under a *new* id and never cleaning up the old
  one (the caller would need to separately track and delete the previous
  hash-id, doubling the bookkeeping for no benefit over just using the
  stable path).
- **native-search doing its own file-change detection** (stat-ing files,
  comparing timestamps) — rejected per ADR-001: this module has no
  filesystem access and Section 1 already asks not to duplicate existing
  functionality (`CacheService` solves an adjacent version of this problem
  already).
