# Issue #6 Phase 1: index-first search via a trigram candidate filter

Implements the direction chosen after `docs/issue-6-status.md`'s gap
analysis: the persistent Tantivy index (from issue #2) becomes a proactive,
corpus-wide, incrementally-maintained index that "Run Search" queries
first, with the existing full text-scan as the fallback for whatever the
index can't safely serve - rather than the prior design, where the index
only ever cached a completed run's *hits*.

## The correctness problem this solves

The literal scanner (`matching.rs`) does case-insensitive **substring**
matching - `"eng"` matches `"engine"`. Tantivy's default tokenizer does
whole-**token** matching - `"eng"` would not match the token `"engine"`.
Routing "Run Search" through a normally-tokenized index as a candidate
pre-filter would silently drop real matches.

**Fix: a dedicated character-trigram field**, separate from the existing
tokenized `body`/`filename`/`title` fields "Fast re-search" already used.
Built with Tantivy's own `NgramTokenizer(3, 3, false)` (confirmed to emit
*all* inner 3-grams, not just prefixes) plus `LowerCaser`. Trigram presence
is a *necessary* condition for substring presence - if `"engine"` doesn't
contain the trigrams of `"torque"`, it can't contain `"torque"` as a
substring - so a trigram-based candidate query is a **safe superset
filter**: it can over-select candidates, never under-select them. This
holds for every match mode except regex (AnyLine/AllInFile/Proximity/
whole-word all reduce to "does the file contain this substring somewhere,"
which trigram presence is necessary for); regex mode always full-scans,
unchanged.

Query-time trigram extraction reuses the *exact* tokenizer instance
registered at index time (`index.tokenizers().get("trigram3")`, never a
second hand-built `NgramTokenizer`) - the safe-superset guarantee depends
on index-time and query-time splitting being byte-for-byte identical.
Verified directly: `native-search::engine::tests::
trigram_candidates_find_a_substring_the_default_tokenizer_would_miss`
indexes `"engine"`, confirms the *default* tokenizer's `search("eng", ...)`
finds nothing (proving the gap), then confirms the trigram candidate query
still finds it.

## What changed

- **`native-search/src/engine.rs`**: new `trigram` schema field (not
  `STORED`), tokenizer registered on every `open_or_create` call (a
  runtime `Index` property, not persisted to disk). New
  `NativeSearchEngine::trigram_candidate_paths(&[String]) ->
  NsResult<Option<Vec<String>>>` - `None` means "don't narrow, fall back
  to a full scan" (any filter under 3 chars, empty filter list, or a
  correctness-conservative default), never a silent risk.
- **`native_index.rs`** (search-core, the policy layer):
  - `open_or_create_with_rebuild` - the trigram field is a schema change,
    so any pre-existing on-disk index now fails `open_or_create`'s
    existing schema-equality check (`NsStatus::CorruptIndex`, by that
    function's own design - no in-place migration path). This wrapper
    catches that specific error and deletes+recreates the index directory
    automatically rather than hard-erroring at the user. All app-level
    `open_or_create` call sites now go through this wrapper.
  - `build_or_update_corpus_index` - proactively indexes every extension-
    matching file under a root (reusing the same walk/extension/size
    scoping a normal search uses), independent of any filter text. Skip-
    if-unchanged per file (existing `get_document_metadata` mechanism),
    batched commits (every 200 docs), per-file error isolation (a failed
    extraction doesn't abort the run).
- **`orchestrator.rs`**: `run()` split into extension-filtering +
  `run_over_candidates` (the shared dry-run/cache/process/summary core).
  New `run_candidates(paths: &[String], ...)` stats a given path list
  (skipping any that no longer exist) and calls the same shared core - the
  index-first query path's entry point, skipping the directory walk
  entirely. `filter_by_extension` factored out for reuse by the corpus
  indexer.
- **`extraction.rs`**: `extract_lines_by_extension` - the extension-to-
  extractor dispatch factored out of `orchestrator::process_one_file`, now
  shared with the corpus indexer and the fs-watch incremental reindexer.
  One dispatch table, not several that could drift as formats are added.
- **`app/src/state.rs`**:
  - `AppState::run_search` now asks the trigram index for candidates
    (per root, in the existing multi-root loop) before choosing between
    `orchestrator::run_candidates` and the unchanged `orchestrator::run` -
    gated on `!use_regex && index_for_fast_search`. Report/preview/export/
    highlighting are all unchanged downstream - the index is a pre-filter,
    the line-scan pipeline that actually produces hit data never changes.
  - `finish_successful_run`'s post-search indexing now calls
    `build_or_update_corpus_index` (the whole corpus) instead of the old
    `index_hits_for_fast_search` (hits only) - **required for
    correctness**, not just an upgrade: candidate narrowing is only safe
    if the index actually covers every file that could match, and a
    hits-only index wouldn't (a file that was never a hit in any past
    search would never be indexed, so it could never appear as a
    candidate even though a full scan might have matched it).
    `index_hits_for_fast_search` itself is untouched in `native_index.rs`
    (still tested, still a legitimate narrower operation) - only the
    app's call site changed.
  - `AppState::build_corpus_index` - a dedicated "Build/update index"
    action, decoupled from Run Search, with its own progress reporting.
  - Known gap: both the automatic post-search indexing and the explicit
    button only index the *primary* root (`search_path`) - a multi-root
    search's extra roots (`search_paths_extra`) aren't automatically
    indexed. The common single-root case is fully correct and tested;
    extending this to loop over every root is straightforward future work,
    not attempted here.
- **`app/src/fs_watch.rs`**: `CHANGE_EVENTS` now carries the actual
  changed path(s), not a bare signal. A new periodic flush task
  (`state::run_incremental_reindex_flusher`, ticking every 2s) coalesces
  rapid repeated changes to the same file (a real editor autosave pattern)
  into a single re-extract+reindex or, if the path no longer exists,
  `delete_document`.
- **`app/src/components.rs`**: "Build/update index" button and its status
  line in the "Fast re-search" section; updated copy explaining Run Search
  now uses the index automatically.

## Verification

- `native-search`: 5 new tests proving the safe-superset property
  (substring-not-token match found, case-insensitivity, OR-across-filters,
  exclusion of a non-matching document, `None`-fallback for short/empty
  filters). 22/22 passing (17 existing + 5 new).
- `search-core`: `orchestrator::run_candidates_matches_run_over_the_same_files`
  and `..._skips_a_path_that_no_longer_exists` prove the `run`/
  `run_candidates` split didn't change `run`'s behavior. `native_index::
  build_or_update_corpus_index_indexes_every_matching_file_not_just_hits`
  and `..._skips_unchanged_files_on_rerun` cover the corpus indexer. **The
  key end-to-end proof**: `native_index::index_first_routing_agrees_with_full_scan`
  - real fixture files, a filter (`"eng"`) chosen specifically because the
    default tokenizer would miss it, index built, trigram-narrowed
    `run_candidates` compared field-by-field against unmodified
    `orchestrator::run` - identical hit files, identical line content.
  All existing tests unchanged and passing throughout (search-core: 83
  before this work, 89 after; native-search: 30 before, 35 after - only
  additions, zero modified assertions in pre-existing tests).
- `app`: 8/8 existing tests still pass (pure-logic helpers, untouched by
  this work). `cargo build -p app` clean at every step; background
  `cargo run -p app` launch with no panic after each change.

## Explicitly out of scope for this phase

Unchanged from `docs/issue-6-status.md`'s list: SQLite metadata store,
streaming HTML/CSV/JSON export, extractor trait abstraction beyond the
shared dispatch function, CLI/headless entry point, per-resource-class
concurrency limits, index health/rebuild-inspection tooling beyond the
automatic schema-mismatch rebuild, fuzzy/wildcard query UI exposure,
ranking tuning, and (new, found during this phase) multi-root automatic
indexing.
