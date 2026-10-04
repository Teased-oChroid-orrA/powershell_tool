# Fast re-search index audit (2026-10)

Evidence record for an audit of index freshness, scheduling and cost. Each
item lists what was measured and what was decided. Harness:
`search-core/tests/index_scheduling.rs` (`#[ignore]`d; run with
`cargo test -p search-core --release --test index_scheduling -- --ignored --nocapture`).
Numbers are from one Apple-silicon macOS machine; ratios matter, absolute
times do not carry over to Windows.

## Shipped

| Item | Finding | Change |
|---|---|---|
| Extraction scheduling | Index build took windows of `throttle_limit` files and awaited each window in order, so one slow file held back its 15 finished neighbours. Corpus of 960 small files + a 5.6 MB PDF / scanned PDF every 16 files: 9.6 s. Every 60 files: 3.2 s. A build with no slow files: 1.8 s. | `JoinSet` sliding window, results taken in completion order. 5.1 s / 1.8 s - the slow files stopped adding to the total. Index writes stay serialized; cancel aborts in-flight tasks. |
| Freshness precision | The index stored `modified` as whole seconds; the normal result cache already used nanoseconds. A same-size edit inside the same second looked unchanged. | Index now stores `cache::ticks_from_modified` (ns). Regression test `same_size_edit_within_the_same_second_is_detected_as_changed`. |
| Semantic index version | Schema equality cannot see a change in what a stored value means (as above) or in extraction/normalisation. | `INDEX_SEMANTIC_VERSION` stamp file in the index folder; a missing or different stamp rebuilds the index on open and makes `verify_index` fail with `CorruptIndex`. Bump it with any such change. |
| Silent metadata fallback | `all_document_metadata().unwrap_or_default()` made an unreadable index look empty. | Build: logs, records the cause in `failure_summary`, re-indexes everything. Narrowing: returns the error, so the caller falls back to a full scan and says so. |
| Path-key consistency | `known.get(&full_name)` used the raw path while scope/stale sets used `path_key` (case/slash folded on Windows). Safe direction (extra candidates) but wasted work. | `known` is re-keyed by `path_key` once (`known_by_key`). Windows-only behaviour; unit-tested with the fold forced on. |
| Failure detail retention | `failed_files` held one string per failure. | Capped at `MAX_FAILURE_DETAILS` (100); `failed_count` and `failure_summary` stay complete. |
| Orphan detection | `narrow_candidates` already subtracts the scan scope first and calls `exists()` only for indexed ids outside it. `remove_orphaned_documents` is global (CLI) and cannot use the walk: an id outside the current scope may be excluded, not deleted. | No change; test `removed_counts_only_documents_whose_file_is_gone_not_ones_merely_out_of_scope` pins the scope rule. |
| Diagnostics + index health | Nothing showed where search time went or what state the index was in. | `NarrowOutcome` carries segment count and walk / freshness / lookup times. `toolboxes/search/diagnostics.rs` builds a per-run `SearchDiagnostics` (corpus, candidates and reduction, files verified, files with matches, result-cache reuse, index lookup / exact verification / total ms, or the reason for a full scan) plus an `IndexHealth` snapshot (docs, segments, on-disk size, time since last commit, new/changed and removed counts, format version, failures in the last build). Shown in a "Search summary" box under the Run view after each search (first two lines on terminals under 26 rows tall) and logged as counts/timings only. Segment count above 32 is flagged "background merge likely still running"; Tantivy exposes no merge progress, so no percentage is shown. |

## Measured and rejected

| Item | Measurement | Decision |
|---|---|---|
| Release profile (`lto = "thin"`, `codegen-units = 1`) | app-tui binary 30.8 -> 25.9 MB (-16 %), build 186 -> 233 s (+25 %), cold index build 4.37 -> 4.23 s (-3 %) | Rejected: trivial runtime gain for a slower build. |
| Writer memory budget 50 / 100 / 256 MB | 4.9-5.5 s at every setting, index size within noise | Rejected: keep 100 MB (see `native-search/AGENTS.md`). |
| Adaptive query planner | Worst case (20k files, every file a candidate): narrowing adds 129 ms freshness + 58 ms lookup over the walk a full scan needs anyway. Linear in corpus size (~1 s at 100k). | Rejected: a planner would save less than that, against a full scan of the same files. |

## Measured, needs a decision

**Duplicate `body` postings.** The schema indexes the extracted text twice:
`body` (word tokens, serves `NativeSearchEngine::search` / `search_fuzzy` and
the legacy FFI) and `trigram` (the only field the app uses). No caller in
`app-tui` or `cli` queries `body`. Not indexing `body` on the same 1,020-file
corpus: index 34 MB -> 8-9 MB (-75 %), build 4.8 -> 4.0 s (-17 %). Removing
it deletes ranked word/phrase/prefix search over file text from the library
and FFI, so it is a product decision, and a schema change (an index rebuild,
handled by the version stamp above).
