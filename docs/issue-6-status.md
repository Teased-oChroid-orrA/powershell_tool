# Issue #6 investigation: gap analysis vs. current architecture

Issue #6 is an 80-section epic asking to transform the app from a
filesystem-scanning tool into a persistent, incremental, index-first local
search engine. Its own stated philosophy is explicit: inspect first,
identify what already solves part of the ask, don't rewrite working
components without evidence, prefer simpler solutions, validate before
implementing. This doc is that inspection pass, and the sections below are
its findings - see `docs/issue-6-phase-1.md` for what was actually
implemented from the "Recommended next step" this doc ends with.

## What already exists and satisfies real chunks of the epic

- **A working Tantivy-backed persistent full-text index** (`native-search/`
  crate, `native_index.rs` policy layer) - built under issue #2, already
  in the app as "Fast re-search." Covers real chunks of epic §11 (persistent
  inverted index, incremental via `get_document_metadata` skip-if-unchanged,
  §14 fingerprinting), §12 (a `(modified_unix, size)` metadata comparison
  per document, though not a general-purpose SQLite store), and - since
  Tantivy's own `QueryParser` is doing the work - very likely already
  supports boolean (`torque OR extension:.pdf`, demoed in the UI's own
  placeholder text), phrase (quoted), prefix, and field-scoped queries per
  §23 without any code changes, though this hasn't been explicitly tested
  per-mode yet (real next step, not a rewrite).
- **Bounded, tunable parallelism with backpressure** - `orchestrator.rs`'s
  `Semaphore`-throttled `JoinSet`, `throttle_limit` now scaled to CPU count.
  Satisfies §18/§20's "bounded queues and worker pools" ask; §19's
  "different resource classes for different extraction costs" is not
  separated (one throttle limit for everything), a real gap but a small one.
- **Real cancellation** - `CancellationToken` threaded through the whole
  orchestrator run and the native-search engine's own `CancellationFlag`
  (checked per-segment during a Tantivy search). Satisfies §25.
- **Incremental extraction caching** - `cache.rs`'s JSON cache, fingerprinted
  by the settings that affect matching, skips re-reading unchanged files.
  Satisfies the spirit of §13/§16, though it's a flat JSON map loaded whole
  into memory, not the SQLite-backed, queryable metadata store §12 asks for.
- **Bounded, batched progress reporting, not per-match events** -
  `SearchProgressReport` sent over an `mpsc` channel, throttled by a
  background ticker (150ms-ish cadence), not one event per file. Satisfies
  §33/§34/§66's spirit.
- **Live in-flight status per file**, not just an aggregate bar - satisfies
  part of §34.
- **Result-set memory bounded in the UI** - pagination (50/page) keeps
  Dioxus's reactive `results` `Signal` from holding a rendered node per
  result; not true windowed/virtualized scrolling (§32), but does satisfy
  §31's "don't put millions of result objects in reactive state" in
  practice, since only the current page's `FileResultView`s exist as
  Dioxus-visible state at once (the underlying `Vec<FileResultView>` itself
  is still fully in memory, though - see gaps below).
- **Search cancellation is decoupled from Dioxus** - `AppState::run_search`
  spawns the actual search work via `tokio::spawn`, not directly in a
  Dioxus event handler, satisfying §30's separation ask structurally, even
  though `search-core` and `app` aren't literally three separate crates per
  §2's suggested layout.
- **A benchmark harness already exists** - `native-search/benches/
  indexing_and_search.rs` (Tantivy indexing/search only, 5,000 synthetic
  docs, manual timing per `docs/benchmarking.md`'s own stated philosophy of
  "a small harness where practical, not a permanent perf-tracking suite" -
  directly matches epic §73's "avoid premature optimization" and §54's ask,
  just narrower in scope than §54's full discovery/extraction/indexing/
  search/memory/UI benchmark matrix).
- **`search-core` is already independently testable with zero GUI
  dependency** (82 tests, `cargo test -p search-core`, no `app` involved) -
  satisfies §30's "core must be independently testable," though not §60's
  "usable without Dioxus at all via a CLI" (no CLI entry point exists).
- **Filesystem watching exists** (`fs_watch.rs`, `notify` crate, its own
  OS thread) - but only as a UI hint ("files changed, run again"), not
  wired into incremental reindexing of the persistent index at all - see
  gaps below.

## The one architectural gap everything else sits on top of

**The persistent index is a cache of the last run's *hits*, not the
corpus.** `index_hits_for_fast_search` (`app/src/state.rs`) only indexes
files that were already found as literal-text-scan hits in a completed
`orchestrator::run` - "Fast re-search" can only ever re-search what a prior
full scan already surfaced. Epic #6's entire premise is the reverse: the
persistent index is built proactively over the whole corpus (or at least
every extension-matching file under the configured roots), kept current via
the filesystem watcher, and *is* the primary search path for a normal query
- a full text-scan becomes the fallback for regex/whole-word modes the
index doesn't serve, not the only way to search at all.

This is a real, deliberate redesign, not a bug - the current design is a
legitimate "index only what you've already proven you care about" choice,
and it's genuinely cheap and simple. Moving to an index-first architecture
is the actual substance of this epic, and it's the fork in the road every
other gap below is downstream of. **This needs a decision before any other
part of #6 is worth implementing** - see "Recommended next step" below.

## Other concrete gaps against the epic's checklist

- **No streaming report/export generation** (§35/§36/§37/§38).
  `report::build_html_report`/`write_csv`/`write_json` all build a complete
  `String`/`Vec` in memory, then do one `std::fs::write`. Fine at the
  result-set sizes this app has actually been used at; would need real
  rework (a `Write`-based streaming API through `report.rs`) before this
  app could handle epic §72's "millions of files" scale without memory
  blowing up. No evidence yet that current usage needs this - §73's
  "measure before optimizing" applies directly.
- **No SQLite metadata store** (§12). `cache.rs`'s flat JSON map is the
  closest equivalent - works, but doesn't support the incremental
  new/modified/deleted reconciliation queries §13 describes, or the
  extractor-versioning invalidation §15 asks for (no schema/version field
  exists anywhere in the cache format today).
- **No extractor trait abstraction** (§5). `extraction.rs` is per-format
  free functions (`extract_docx_lines`, `extract_pptx_lines`, ...) matched
  on extension in `orchestrator.rs`. Works fine, and CLAUDE.md's own
  documented reasoning for hand-rolled extraction (byte-for-byte parity
  with the tested C# original) is a real constraint a generic `Extractor`
  trait wouldn't change - but adding new formats (§6's XLSX/CSV/JSON/XML/
  HTML/Markdown/EPUB/ODT list; XLSX and ZIP are already done) means editing
  the orchestrator's match arm each time, not registering a new
  implementation. A trait wrapping the existing functions (not
  reimplementing them) would satisfy §5 cheaply if/when format count grows
  enough to justify it.
- **Filesystem watching doesn't drive incremental reindexing** (§39/§40).
  Exists only as a "your last search may be stale" banner. Directly
  downstream of the architectural gap above - there's no proactive index to
  keep current yet.
- **No resource-class separation for concurrency** (§19) - one
  `throttle_limit` covers TXT/PDF/DOCX/everything.
- **No index health/maintenance tooling** (§50) - no rebuild/verify/
  compact/inspect-failed-documents affordances; `.native-search-index`
  can only be fixed today by deleting the folder by hand.
- **No CLI/headless entry point** (§60) - `search-core` is headless-*capable*
  (zero GUI dependency, real tests prove it), but nothing actually exposes
  it outside the Dioxus app.
- **Regex/whole-word modes always full-scan** (§24) - there's no
  "narrow via indexed candidates first, then regex-scan only those" path;
  regex mode in this app has only ever meant the literal text-scan
  orchestrator, never the Tantivy index. Consistent with today's index
  being an opt-in secondary path, not primary.
- **Ranking is whatever Tantivy's default `TopDocs::with_limit(...)
  .order_by_score()` produces** (§48) - not tuned (filename/title
  weighting, phrase-match boosting, etc.), but Tantivy's own BM25 default
  is a reasonable, not-broken baseline; no evidence yet it needs tuning.
- **No crash-recovery-specific handling** (§51) beyond what Tantivy's own
  commit semantics already provide (a commit is atomic; anything indexed
  but not yet committed is simply absent after a crash, not corrupt) -
  never explicitly tested.

## What this app's real scale actually looks like (important context missing from the epic itself)

Epic #6's benchmark corpus suggestions (§55: "1,000,000 small TXT/LOG
files," "50,000 PDF files," "500MB+ documents") and Definition of Done
(§78: "memory usage remains bounded under large workloads") describe an
enterprise document-search-service scale. This is a desktop tool one person
runs against local/shared-drive folders for compliance-style keyword
sweeps - real usage has been at the scale of individual searched folders
with up to a few thousand hit files (`native-search`'s own bench uses 5,000
synthetic docs as a validation ceiling, per `docs/benchmarking.md`).
Section 73's own "do not introduce complicated optimizations without
profiling evidence" and §80's "measure first, optimize second, validate
third" directly argue against building out disk-backed streaming
everything before there's a real workload that needs it. This doesn't mean
skip the epic - it means the *P0/P1/P2 priority order in §77 should be
re-derived against this app's actual usage pattern*, not assumed from the
epic's own suggested order, which is written generically.

## Recommended next step

This is a multi-phase epic by the issue's own admission (§76's 11-phase
migration strategy). Before writing any code against it, the fork-in-the-
road decision above needs to be made explicitly: does "search" in this app
become index-first (Tantivy as the primary path, full-scan as the regex/
whole-word fallback), or does the current "full-scan first, opt-in index of
hits" design stay and only get the smaller, additive improvements (trait-
wrapped extractors, resource-class-separated concurrency, streaming
exports, a CLI entry point) layered on top without the index-first rework?
The former is what issue #6 is actually asking for; the latter delivers a
meaningful chunk of its value at a fraction of the risk and effort. Recommend
confirming this before starting implementation - happy to draft a phased
plan for whichever direction is chosen.
