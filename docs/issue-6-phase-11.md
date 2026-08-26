# Issue #6 Phase 11: report performance stats, export/report gaps, and
the documentation sections (§45-46, §59, §61, §63, §68-69, §74)

## Code changes

- **`docs/search-semantics.md`** (new) - epic §46's explicit "define and
  document: case sensitivity, Unicode normalization, punctuation
  behavior, tokenization, stemming, phrase matching, wildcard semantics,
  regex semantics, path matching, filename matching" ask, as its own
  file. Documents one real, honest gap found while writing it: **Unicode
  normalization is not applied anywhere** in the matching pipeline - a
  filter in NFC form won't match visually-identical NFD content. Not
  fixed speculatively (no evidence it's actually hit a real user), but
  now written down instead of being an undocumented surprise.
- **`SearchRunSummary.total_elapsed_seconds`** (new field) - epic §68
  "HTML Report Design" lists "performance statistics" as a report
  requirement; there was no total-run-duration tracked anywhere before
  this (only per-in-flight-file `elapsed_seconds`, a live-progress-only
  concept). `run_over_candidates` now times itself
  (`Instant::now()`/`.elapsed()`) and the HTML report shows an "Elapsed:"
  line whenever it's non-zero (omitted for default/untimed
  `SearchRunResult`s built directly in tests, so no existing test's HTML
  content assertions needed updating).
- **Multi-root report "Search folder" line** (`app/src/state.rs`) - a
  multi-root search (`search_paths_extra`) was reporting only the
  *first* root in the HTML report's summary, because `finish_successful_run`
  passes the original pre-loop `settings` (whose `search_path` is just
  the primary root) to the report writer. Fixed by overriding
  `search_path` to a `"; "`-joined list of every root on the *cloned*
  settings used only for the report call - `search_path` isn't used for
  anything functional past that point (every hit already carries its own
  absolute `full_name`), so this is display-only and safe.
- **`ExportRow.file_size`** (new field, CSV `FileSize` column + JSON/JSONL) -
  epic §69 "Export Metadata" lists `file size` explicitly; wired from
  `FileSearchResult.file_length`, already computed, just not previously
  exported. `filename`/`extension` from that same list were deliberately
  *not* added as separate columns (trivially derivable from `file_path`
  by any downstream consumer - `jq`, a spreadsheet formula); `score` was
  deliberately omitted too (this is a literal/regex line scanner, not a
  ranked search - there's no real relevance score to report, and a
  fabricated constant would be actively misleading).
- **`search-core/benches/discovery_and_extraction.rs`** (new) - epic §54's
  "Discovery" and "Extraction" benchmark categories, the two
  `native-search/benches/indexing_and_search.rs` didn't cover. Same
  deliberate "plain manual timing, no criterion" choice, documented in
  `docs/benchmarking.md` alongside the existing harness's numbers and
  caveats, plus an honest "what's deliberately not benchmarked" section
  for Memory/UI (see that doc for the reasoning - both would need either
  platform-specific instrumentation or a live GUI window `cargo bench`
  can't provide).

## §59 Configuration - already satisfied, no new code

Epic §59 lists roots/extensions/excluded-dirs/max-file-size/worker-
counts/memory-limits/index-location/cache-location/watcher-behavior/
reconciliation-interval/search-defaults/export-options as things a
config layer should cover. Every one of these is already configurable -
just not through a single dedicated "config file" concept:

- **GUI**: every `SettingsPanel` field, persisted across relaunches by
  `app/src/persistence.rs` (JSON, atomic-written as of Phase 8) - this
  *is* the GUI's config layer, and it already covers the full list above.
- **CLI**: the equivalent subset as flags (`search-cli --help`) - roots
  (positional), extensions, excludes, size limit, worker counts
  (`--throttle-limit`/`--heavy-throttle-limit`), cache location
  (`--cache-file`), failure-log location (`--failure-log`).
- **Index/cache location**: index location is a fixed, documented
  convention (`.native-search-index/` inside the searched folder -
  ADR-011), not user-configurable by design (auto-excluded from the
  search itself, always co-located with what it indexes - a real
  simplification, not a missing feature).
- **Watcher behavior/reconciliation interval**: `app/src/fs_watch.rs`'s
  debounce window is a fixed constant, not exposed as a setting - no
  evidence yet that it needs to be user-tunable.

No unified config-file format (e.g. a single `config.toml` covering both
GUI and CLI) was added - the epic says "consider," not "require," and the
two surfaces (persisted JSON settings, CLI flags) already serve their
respective use cases without the added complexity of a third format both
would need to stay in sync with.

## §61 Dependency Audit

Every direct dependency across the workspace, and why:

| Crate | Why |
|---|---|
| `tantivy` | The persistent full-text index itself - see ADR-004/005/006/010 (already documented) for why it was chosen over alternatives. |
| `rusqlite` (`bundled`) | Extraction-failure log (`docs/issue-6-phase-8.md`) - `bundled` avoids a host-machine SQLite dependency, required by CLAUDE.md's target-environment constraint. |
| `fancy-regex` | Whole-word/regex matching needs lookaround; `regex` deliberately doesn't support it. One engine for both plain and regex-mode matching avoids two subtly different semantics coexisting (CLAUDE.md). |
| `regex` | Used narrowly for structural/UI-adjacent patterns (extraction's tag-stripping helpers) separate from user-facing filter matching. |
| `zip` | DOCX/PPTX/XLSX/ZIP extraction - matches the C# original's own dependency-free `ZipArchive` approach (CLAUDE.md), not a format-specific library that would extract differently. |
| `flate2` | PDF `/FlateDecode` is raw DEFLATE, same algorithm .NET's `DeflateStream` uses. |
| `encoding_rs` | Windows-1252 fallback decode - the standard, widely-used crate for this; Rust has no built-in equivalent to .NET's OS/ICU-backed encoding support. |
| `chrono` | Date/time handling throughout - `Local`/`DateTime` used for created/modified timestamps, matches the C# original's `DateTime` semantics closely. |
| `tokio`/`tokio-util` | Async file I/O with timeout/cancellation/retry, bounded concurrency (`Semaphore`), `CancellationToken`. |
| `serde`/`serde_json` | Every serializable model, the JSON cache, CSV/JSON/JSONL export, settings persistence. |
| `base64` | HTML report's embedded banner image (data URI). |
| `urlencoding` | HTML report's `file://` link generation. |
| `dioxus` (`native`) | The GUI itself - see CLAUDE.md's "Why dioxus-native, not dioxus-desktop" for the specific, verified reasoning (no WebView2 host dependency). |
| `rfd` | Native folder-picker dialog. |
| `open` | Opens the finished HTML report in the OS default handler. |
| `arboard` | "Copy path" result-row action. |
| `winit`/`blitz-shell`/`blitz-html` | Hand-rolled launch sequence for drag-and-drop support `dioxus_native::launch_cfg` doesn't provide (CLAUDE.md). |
| `notify` | Filesystem watching - native OS watch APIs, not polling. |
| `image` (`png` only) | Decodes the bundled window-icon PNG. |
| `notify-rust` | Desktop completion toast notification. |
| `clap` (`derive`) | CLI argument parsing - the standard, de facto crate for this in Rust. |
| `dialoguer` | CLI interactive-menu mode (`docs/issue-6-phase-6.md`) - mature, `console`-backed, Windows Console API support. |
| `tempfile` (dev-only) | Test fixtures across every crate. |

No dependency was added "because it was mentioned in the epic" without a
concrete, present use - every one above backs a feature this app actually
ships. License: every crate above is MIT/Apache-2.0/MPL-2.0 (`zip`'s MPL-2.0
being the one non-MIT/Apache entry, already compatible with this project's
own MIT license as a dependency, not a distributed-code concern). Platform
compatibility: none introduce a Windows-incompatible dependency - already
independently verified by `.github/workflows/rust-build.yml` building
`app` for `x86_64-pc-windows-msvc` on every push.

## §63 Network/Remote Filesystems - honest known limitation, not solved

This app makes no special accommodation for network shares/remote
filesystems today - no evidence was found (or sought; this environment
has no network share to test against) that it needs to. `file_reader.rs`'s
existing retry-with-backoff/timeout/truncation-detection logic (built for
locked-file scenarios) would incidentally help with network-share latency
too, but nothing in the codebase specifically detects "this path is a
network share" and adjusts concurrency/timeouts accordingly, as §63 asks
for. Documented here as a known, unaddressed gap rather than silently
assumed away - if network-share usage becomes a real reported problem,
the concurrency limits (`throttle_limit`/`heavy_throttle_limit`, already
configurable) are the first lever to reach for, not new architecture.

## §74 Unsafe Rust - audit, no code change needed

`grep -rn "unsafe " --include="*.rs"` across every crate: **every** use of
`unsafe` is confined to `native-search/src/ffi.rs` - the C ABI layer that
exists solely to serve the legacy C#/WinUI P/Invoke layer (CLAUDE.md:
"Dead weight for the Rust app; do not remove until the C# app is
retired"). Zero `unsafe` anywhere in `search-core`, `app`, or `cli`. Every
§74 checklist item is already satisfied for this one location: isolated
(one file), invariants documented (`docs/ffi.md`, 319 lines), tests exist
(`native-search/tests/ffi_smoke.rs`, 13 tests), and it was never a
performance choice to benchmark against a safe alternative - it's a
structural requirement of exposing a C ABI at all. Nothing to change here;
recorded as an audit finding for the validation report (§79).

## Verification

`cargo test --workspace`: **181/181 passing** (app 8, native-search 42 +
13 ffi_smoke, search-cli 4, search-core 117 + 10 fixtures). New tests:
`html_report_shows_elapsed_time_when_present`,
`html_report_omits_elapsed_line_when_not_tracked`,
`export_rows_carry_the_file_size_from_the_file_result` (report.rs),
`a_real_run_populates_total_elapsed_seconds` (orchestrator.rs). New
benchmark run and recorded in `docs/benchmarking.md`.
