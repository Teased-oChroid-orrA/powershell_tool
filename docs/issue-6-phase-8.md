# Issue #6 Phase 8: crash recovery (§51) + index health/maintenance (§50)

## §51 Crash recovery: atomic writes

Audited every direct file-write site outside Tantivy/SQLite (both of
which are already crash-safe by construction - see below) for the
"do not mark something as persisted until the persistence operation has
actually completed" principle. Found two real gaps, both using
`std::fs::write` (truncate-then-write) directly against the real path:

- `search-core/src/cache.rs`'s `save()` - the incremental JSON result
  cache. A crash mid-write would leave a truncated file that `try_load`'s
  `serde_json::from_str` fails to parse, silently discarding the *whole*
  cache (not just that run's updates) on the next launch.
- `app/src/persistence.rs`'s `save()` - the GUI's settings file. Same
  failure mode: a crash mid-write leaves an unparseable settings file,
  and the next launch silently falls back to defaults instead of the
  user's actual saved settings.

Fixed both with write-to-temp-then-rename (`cache::atomic_write`, and the
equivalent inline in `persistence::save`): write the new content to a
sibling `.tmp` path, then `std::fs::rename` it over the real path.
`std::fs::rename` is an atomic replace on both this project's target
platform (Windows' `MoveFileExW`/`MOVEFILE_REPLACE_EXISTING`, which
`std::fs::rename` uses under the hood) and Unix. A crash mid-write now
only ever leaves an orphaned `.tmp` file behind; the real file is always
either the complete old version or the complete new one.

**Already crash-safe, no change needed:**
- Tantivy's index (`native-search`) - already documented in
  `docs/issue-6-status.md`: an uncommitted document is simply absent
  after a crash, not corrupt. `commit()` is the durability boundary;
  nothing in this codebase treats a document as indexed before that call
  returns `Ok`.
- `search-core/src/failure_log.rs`'s SQLite table - SQLite's own
  rollback-journal/WAL mechanism makes every write crash-safe by
  construction; there's no manual atomic-write pattern to add on top of
  it.

## §50 Index health / maintenance

Four new `search-cli` maintenance actions - each performs one action and
exits, instead of running a search (CLI-only, same scope decision as the
failure log and cache-file flag: this is admin/maintenance tooling, not
something the GUI's `SettingsPanel` needs a control for):

- `--verify-index` - opens the fast-search index for the given folder and
  reports its document count, using `NativeSearchEngine::open_or_create`
  directly (**not** `open_or_create_with_rebuild`) so a corrupt or
  schema-mismatched index surfaces as a clear error instead of being
  silently auto-rebuilt out from under the caller - "verify" has to
  actually be able to report failure.
- `--remove-orphaned` - deletes every indexed document whose file no
  longer exists on disk (moved/renamed/deleted since it was indexed).
  New `NativeSearchEngine::all_document_ids()` (an `AllQuery` full-index
  scan - fine for maintenance tooling, not used anywhere on the normal
  search/index path) backs a new `native_index::remove_orphaned_documents`
  that checks each id (== path, per ADR-008) against `Path::exists()` and
  calls the existing `delete_document` for anything missing.
- `--clear-cache` (paired with a new `--cache-file <path>` flag, since the
  CLI didn't expose the JSON result cache at all before this) - deletes
  the cache file if present, a clean no-op if it's already gone.
- `--list-failures` (paired with the existing `--failure-log <path>`) -
  prints every recorded extraction failure as JSON (`FailureRecord` now
  derives `Serialize`), newest first, via the failure log's existing
  `list_failures()`.

"Compact/optimize index" and "rebuild metadata" from §50's list were
deliberately not given a separate action: `--index`'s existing "Rebuild
from scratch" behavior (delete-and-rebuild) already produces a fully
merged, minimal-segment index - a distinct merge-only "compact" action
would duplicate that outcome via a different, more fragile code path
(manual Tantivy segment-merge API calls) for no real benefit at this
app's scale. "Inspect unsupported formats" is already covered by the
existing per-run summary counters (`skipped_binary`, etc.) and the
extension catalog/picker - nothing new to add there.

## Verification

`cargo test --workspace`: **173/173 passing** (app 8, native-search 42 +
13 ffi_smoke, search-cli 4, search-core 109 + 10 fixtures). New tests:
`atomic_write_replaces_content_and_leaves_no_tmp_file_behind` (cache.rs),
`all_document_ids_returns_every_indexed_id` (native-search/engine.rs),
`remove_orphaned_documents_deletes_only_paths_missing_from_disk`,
`remove_orphaned_documents_is_a_no_op_when_nothing_is_orphaned`,
`verify_index_reports_the_document_count`,
`verify_index_reports_corrupt_index_instead_of_silently_rebuilding`
(native_index.rs). All four new CLI flags manually smoke-tested end-to-end
against a real folder (index, verify, delete a file, remove-orphaned,
verify again, clear-cache against a missing file, list-failures against
an empty log) - each produced the expected output.
