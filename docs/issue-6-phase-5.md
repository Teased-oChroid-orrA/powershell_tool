# Issue #6 Phase 5: persistent extraction-failure log (SQLite)

Last of the 7 explicitly-requested epic #6 continuation items (fuzzy/
wildcard query UI, ranking tuning, extractor trait abstraction, streaming
HTML export, CLI/headless entry point, per-resource-class concurrency
limits, SQLite metadata store).

## Scope decision: not a full metadata-store replacement

Epic #6 §12 asks for "a SQLite metadata store." Two existing mechanisms
already cover metadata:

- `cache.rs`'s incremental JSON result cache (fingerprinted by settings,
  keyed by path+size+mtime) - already serves "skip unchanged files."
- Tantivy's own stored document fields (`native_index.rs`/`engine.rs`) -
  already serve "what's indexed, when, from where."

Epic §12 itself warns against duplicating data across stores. Replacing
either of the above with SQLite, or adding a third store that duplicates
their fields, would violate that warning with no evidence either existing
mechanism is inadequate. The one genuine gap: **extraction failures are
never persisted**. A file that fails to extract (corrupt DOCX, malformed
PDF stream, etc.) is retried on every single run, forever, with no way to
inspect failure history across runs (epic §16/§17/§50). That gap is what
this phase fills - narrowly.

## What was built

- **`search-core/src/failure_log.rs`** (new) - `FailureLog`, a thin wrapper
  around a `rusqlite::Connection` (`bundled` feature - statically links
  SQLite's C amalgamation, so no runtime dependency on the target Windows
  machine, per CLAUDE.md's "Target environment" requirement). One table,
  `extraction_failures(path PRIMARY KEY, size, modified_unix, status,
  reason, failed_at_unix, extractor_version)`.
  - `record_failure` - upsert (overwrite, not append - one row per path).
  - `clear_failure` - delete on successful (re-)extraction.
  - `known_failure_reason(path, size, modified_unix)` - returns `Some`
    only on an exact fingerprint match; any change to the file (or a
    bumped `EXTRACTOR_VERSION`) makes it `None`, so a fixed/changed file
    is always retried rather than permanently skipped.
  - `list_failures` - full history, newest first (inspection, not wired
    to any UI yet).
- **`SearchSettings.failure_log_path: Option<String>`** (models.rs) -
  `None` by default (opt-in; no behavior change for existing callers).
- **`orchestrator.rs` wiring** - `run_over_candidates` opens the log once
  (if a path is set) and threads it through both the parallel and
  sequential per-file processing paths. `process_one_file`:
  - Early-skips extraction entirely when `known_failure_reason` matches
    the file's current fingerprint, reporting the same `ReadError` +
    reason as the original failure.
  - Calls `record_failure` when extraction produces no usable text.
  - Calls `clear_failure` on any successful extraction (so a file fixed
    externally, or a corrected extractor, stops being skipped).
- **CLI**: `search-cli --failure-log <path>` opts a headless run into this
  same behavior - the only place it's currently exposed (not wired into
  the GUI's `SettingsPanel` yet; `AppState::build_settings()` passes
  `None` with a comment marking this as a deliberate, not accidental,
  scope boundary).

## Verification

`cargo test --workspace`: **149/149 passing** (app 8, native-search 25 +
13 ffi_smoke, search-cli 4, search-core 89 + 10 fixtures).

New tests:
- `failure_log.rs` (6 unit tests): record-and-find, fingerprint-mismatch
  misses, clear removes, re-record overwrites not duplicates, list
  returns everything, reopening the same on-disk `.db` file preserves
  records (real cross-reopen persistence, not just in-memory).
- `orchestrator.rs` (2 integration tests): a known failure is skipped
  (not re-extracted) on a rerun with an unchanged file; a file that now
  extracts successfully clears its stale failure record. The second test
  deliberately pre-seeds a *mismatched* fingerprint so the early-skip
  check doesn't short-circuit before reaching real extraction - the first
  draft of this test pre-seeded the file's actual current fingerprint,
  which would have skipped extraction entirely and never exercised
  `clear_failure` at all; caught via reasoning before running it, not via
  a failing result.
- `cli_smoke.rs`: unaffected (the flag is additive/optional), all 4
  pre-existing smoke tests still pass with it present.

## Explicitly still out of scope

- No GUI exposure (checkbox/path field) for `failure_log_path` yet - CLI
  only. Adding it is a small, isolated `app/src/components.rs` +
  `state.rs` change if/when requested; not done speculatively here.
- No `list_failures` UI/CLI surface (e.g. `search-cli --list-failures`) -
  the data is there or reachable via `FailureLog::list_failures`, but no
  consumer was requested.
- `cache.rs` and Tantivy's stored fields are unchanged - this phase adds
  one new table for one new fact, not a metadata-store migration.
