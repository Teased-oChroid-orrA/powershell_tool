# native-search

## Purpose
Owns: Tantivy-backed indexing/search engine crate ("Fast re-search" / issue #2). Given already-extracted document text + metadata, indexes it, searches it (exact/boolean/phrase/prefix/fuzzy/trigram-candidate), reports Tantivy-specific failure modes.
Does not own: file discovery, text extraction, or change-detection policy (all `search-core`'s job - see `search-core/src/native_index.rs`). Never reads a file from disk itself (ADR-001). No logging dependency of its own.

## Code Map

### Find It Fast
| Looking for... | Go to |
|----------------|-------|
| Open/create an index | `src/engine.rs` `NativeSearchEngine::open_or_create` |
| Index/update a document | `src/engine.rs` `index_document` (delete-by-id + re-add; Tantivy docs are immutable) |
| Delete a document | `src/engine.rs` `delete_document` |
| Run a query | `src/engine.rs` `search` / `search_fuzzy` (both via `build_parser`) |
| Substring/regex candidate pre-filter | `src/engine.rs` `trigram_candidate_paths` / `..._for_chunk_sets` |
| Writer-death recovery | `src/engine.rs` `with_writer_retry` |
| Mutex-poison recovery | `src/engine.rs` `lock_writer` |
| Skip-reindex-if-unchanged lookup | `src/engine.rs` `get_document_metadata` |
| Orphan-cleanup support | `src/engine.rs` `all_document_ids` |
| Schema definition | `src/engine.rs` `build_schema` / `struct Fields` |
| Error/status codes | `src/error.rs` `NsStatus`, `NsError` |
| The C ABI (P/Invoke surface) | `src/ffi.rs` - every `ns_*` `extern "C"` fn |
| Panic containment at FFI boundary | `src/ffi.rs` `guard` / `guard_readonly` |
| Cancelling an in-flight search | `src/engine.rs` `CancellationFlag`; FFI `ns_cancel_token_*` |
| Raw-ABI round-trip tests | `tests/ffi_smoke.rs` |
| Perf sanity harness (not an SLA) | `benches/*.rs` |
| Full FFI contract writeup | `docs/ffi.md` (repo root) |
| Boundary/Tantivy/index-location rationale | `docs/adr/ADR-001`, `ADR-002`, `ADR-011` |

### Key Relationships
- `search-core/src/native_index.rs` (policy) → `native-search/src/engine.rs` (in-process call, **no FFI**) → Tantivy.
- `src/ffi.rs` exists **only** for the legacy C#/WinUI P/Invoke layer (`docs/ffi.md`). Zero callers anywhere in this repo's own Rust stack - that's expected, not evidence it's dead.
- `src/error.rs` is shared by `engine.rs` (Rust errors) and `ffi.rs` (converted to `i32` status) - keep both in sync.

## Design Rationale
- **Problem solved**: the line scanner (`search-core::matching`) re-scans every file on every search with no persistent index/ranking. This crate adds a fast, incrementally-updated, relevance-ranked index as an accelerant - the scanner stays authoritative for regex mode and as the correctness fallback elsewhere (ADR-002, `docs/issue-6-phase-1.md`).
- **Core insight**: a character-trigram field (`TRIGRAM_TOKENIZER_NAME = "trigram3"`) makes substring queries index-searchable even though Tantivy's default tokenizer only matches whole tokens. Trigram presence is a *safe superset* filter - over-selects, never drops a real match - for every mode except regex (always full-scans).
- **Constraints**: must serve a legacy C#/WinUI P/Invoke caller (`ffi.rs`'s narrow, opaque-handle, panic-proof C ABI) *and* an in-process Rust caller (`search-core`) simultaneously during the migration.

## Public API
| Export | Used By | Change Impact |
|--------|---------|----------------|
| `engine::NativeSearchEngine` | `search-core::native_index` (in-process) | Method signature changes ripple into that policy layer |
| `engine::{DocumentInput, SearchHit, CancellationFlag}` | `search-core`, `ffi.rs` | `SearchHit` also serializes to JSON for the C# caller (ADR-009) |
| `ffi::ns_*` (C ABI) | Legacy C# `NativeSearchInterop.cs` only | Breaking change requires updating `docs/ffi.md` + that C# file together |

## Decisions
| Decision | Why | Rejected |
|----------|-----|----------|
| Rust owns indexing+search only (ADR-001) | Preserve `search-core`'s tested extraction; keep FFI narrow | Rust owning extraction too |
| Tantivy-only, no query planner (ADR-002/010) | Verified viable; FM-index/suffix-array/bio candidates (ADR-004-006) all lack incremental updates | Multi-index architecture |
| JSON over FFI, not C structs/Protobuf (ADR-009) | `serde`/`System.Text.Json` already present; shape too simple for codegen | Fixed C structs (ABI-fragile); Protobuf (overengineered) |
| Caller supplies stable `id` = file path; no file-change detection here (ADR-008) | No filesystem access (ADR-001); `native_index.rs` owns that | native-search stat/hash files itself |
| Index at `<SearchPath>/.native-search-index/`, not `%LOCALAPPDATA%` (ADR-011, supersedes ADR-007) | Direct user direction: index travels with the folder | Global per-machine index (shipped, then reversed) |
| `panic = "unwind"` set at **workspace-root** `Cargo.toml` | Cargo ignores `[profile.*]` in non-root members; `ffi.rs`'s `catch_unwind` needs unwind, not abort | Leaving it in `native-search/Cargo.toml` (silently ignored) |

## Entry Points
| Task | Start Here |
|------|------------|
| Add a new indexed/searchable field | `build_schema`/`Fields`/`index_document`/`SearchHit`; existing indexes then fail open with `NsStatus::CorruptIndex` - `native_index::open_or_create_with_rebuild` auto-rebuilds |
| Add a new query mode | `build_parser`/`search`/`search_fuzzy`; add positive + negative tests (see Pitfalls - the query grammar has real gaps) |
| Add a new `ns_*` FFI function | `src/ffi.rs`: `guard(...)`-wrapped body, `# Safety` doc, test in `tests/ffi_smoke.rs`, update `docs/ffi.md` + `NativeSearchInterop.cs` |
| Investigate a Windows crash in indexing/search | Read `with_writer_retry` and `lock_writer` doc comments first - two distinct, already-diagnosed failure classes |

## Contracts
- **`TantivyError::ErrorInThread` = the `IndexWriter` is permanently dead**, distinct from Mutex poisoning. Confirmed in tantivy 0.26.1 source: a dead segment-writer thread sets `index_writer_status` not-alive; only a fresh `index.writer(budget)` recovers. `with_writer_retry` rebuilds once and retries (fixed in `3133c6a`).
- **A poisoned `writer` Mutex must never propagate a panic to later calls** - a real Windows crash ("stays broken after one crash") was a poisoning cascade via a bare `.expect(...)`. Fixed by `lock_writer()`'s poison-recovery; proven by `engine_survives_a_poisoned_writer_mutex` (fixed in `e0e5dbc`).
- **Schema is checked on every `open_or_create`, not just at creation** - `Index::open` has no compatibility check of its own; a mismatch fails fast as `NsStatus::CorruptIndex` (no in-place migration exists; ADR-002 item 9).
- **Query-time and index-time trigram splitting must use the same registered tokenizer instance** (`index.tokenizers().get(...)`), never a second hand-built one - the safe-superset guarantee depends on it.
- **No `unsafe`/FFI types outside `src/ffi.rs`; every `ns_*` export must run inside `guard`/`guard_readonly`** - no panic may unwind into .NET (issue #2 Section 16/18).

- **The `IndexWriter` is created lazily on the first write**, not in `open_or_create`: a query-only open (re-search narrowing, `verify_index`) must not take `.tantivy-writer.lock` or spawn indexing threads. `commit()` with no writer only reloads the reader.

## Patterns

### Adding a new `ns_*` FFI export
1. Write the safe logic in `engine.rs` first (testable without `unsafe`).
2. Thin `extern "C"` wrapper in `ffi.rs`: opaque handles only, `guard(...)`-wrapped, `# Safety` doc.
3. Add a round-trip test in `tests/ffi_smoke.rs` (raw ABI, not just the safe API).
4. Update `docs/ffi.md` AND `src/TextInFilesSearch.Core/Native/NativeSearchInterop.cs` - nothing enforces they match at compile time.

## Pitfalls
- **Bare single-word `term*` is silently NOT a prefix query.** Unquoted `engin*` becomes an exact-term query for "engin" (0 results, no error); quoted single-word `"engin"*` is a hard `QueryParserError`. Only a multi-word quoted phrase ending in `*` (`"the engin"*`) works, on its last term only (see `search_supports_prefix_wildcard_on_multi_word_phrases_only`).
- **`~` in query text means phrase slop, not fuzzy distance** - no per-term `~N` fuzzy syntax exists; use `QueryParser::set_field_fuzzy` (`search_fuzzy`). `search_fuzzy` itself is real/tested but has no UI entry point today - not dead code.
- **The `trigram` field is presence-only (`IndexRecordOption::Basic`), deliberately** - the literal/regex scanner downstream is always authoritative, so positional info there buys nothing (confirmed, issue #8; `docs/issue-8-status.md`). Issue #8 overall found no perf bottleneck and made no code change (`fe5e6a1`) - re-verify before assuming a waiting perf fix.
- **A failed `commit()` drops the writer and loses every uncommitted document** (Windows `os error 5` creating segment files, seen in the field). `commit` clears the writer so the next write starts fresh; callers must re-stage the batch (`search-core::native_index::commit_with_replay`).
- **`WRITER_MEMORY_BUDGET` was raised 50MB→100MB** (`3133c6a`) - a small budget forces frequent flushes, plausibly increasing exposure to the `ErrorInThread` bug above. Don't lower it without considering that link.
- **Never call `get_document_metadata` in a per-file loop.** It is a term lookup that visits every segment (~0.6us x segments; measured ~150-180us each at 100k docs / ~300 segments = ~18s per folder freshness check). Use `all_document_metadata()` (one pass, ~0.25s at 100k) - `search-core::native_index` does for index builds and `narrow_via_index`. Single-file callers (`index_hits_for_fast_search`, FFI) may keep the per-id call.
- **Segment count is transiently high after a build; merges are asynchronous and DO converge.** Measured at 100k docs: 256 segments right after commit, 151 at +5s, 4 at +15s (10k: 31 -> 3 in 5s). Benchmark queries/lookups taken before merging settles are 10-15x slower (query p50 ~1ms vs ~84us settled; per-id metadata lookup ~180us vs ~13us) - wait for the segment count to settle before benchmarking, and don't "fix" merge policy on a mid-merge measurement.
- **`[profile.*]` in `native-search/Cargo.toml` is silently ignored by Cargo** (non-root workspace member) - it must live in the workspace-root `Cargo.toml`.

## Boundaries

### Always
- Keep `unsafe`/FFI types confined to `src/ffi.rs`; `engine.rs`/`error.rs` stay pure, testable safe Rust.
- Wrap every new `ns_*` export in `guard`/`guard_readonly`.
- Update `docs/ffi.md` when the FFI surface changes - it is the authoritative contract, not this file.

### Never
- Remove `src/ffi.rs` before the legacy C#/WinUI app is retired.
- Have this crate read files, walk directories, or do file-change detection - that's `search-core`'s job (ADR-001/008).
- Add a second index backend without revisiting ADR-004/005/006's evidence.

### Ask First
- Changing `build_schema()` (breaks every existing on-disk index, no migration path).
- Changing `src/ffi.rs` function signatures (breaks the compiled C# P/Invoke layer).

## Downlinks
None - leaf node (single crate).
