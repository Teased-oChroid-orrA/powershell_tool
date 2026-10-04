# search-core

## Purpose
Owns: business logic for searching text inside files — settings/models, matching, text extraction (TXT/DOCX/PPTX/XLSX/ZIP/RTF/PDF, optional OCR), robust async file I/O, incremental caching, HTML/CSV/JSON/JSONL reporting, parallel orchestration, and the policy layer over the `native-search` fast-index crate.
Does not own: any GUI (`app/`, `app-egui/`), CLI parsing (`cli/`), or `native-search`'s own indexing internals (`native-search/src/engine.rs`) — this crate only calls that engine in-process.
Plain Rust library, zero GUI dependency — `cargo test -p search-core` must build/pass with no GUI toolchain present (1:1 port of C# `TextInFilesSearch.Core`'s own Core/head split; see `docs/rust-rewrite-status.md`). Never add a dependency that pulls in Dioxus, egui, or any windowing toolkit.

## Code Map

### Find It Fast
| Looking for... | Go to |
|----------------|-------|
| Settings/result/hit types, match-mode/exclude-scope/group-by enums | `src/models.rs` (`SearchSettings`, `FileSearchResult`, `LineHit`, `SearchRunResult`) |
| Default extension list / UI filter picker data | `src/models.rs::extension_catalog` module |
| Filter compilation, whole-word regex, AllInFile/Proximity gating | `src/matching.rs` (`CompiledMatchState`, `apply_line_matching`, `whole_word_pattern`) |
| Why a search returned 0 hits despite matches existing | `matching::LineMatchOutcome.passes_mode` — never infer from `hits.is_empty()` |
| DOCX/PPTX/XLSX/ZIP text extraction | `src/extraction.rs` (`extract_docx_lines`, `extract_pptx_lines`, `extract_xlsx_lines`, `extract_zip_archive_lines`) |
| PDF text extraction | `src/extraction.rs::extract_pdf_lines` (~line 1433); stream scanner is `find_stream_blocks`, not a regex |
| PDF hex-string / CID-font operands | `extraction.rs`: `hex_string_re`, `parse_tounicode_cmap`, `hex_string_to_unicode` (~1128-1300) |
| Encoding fallback (non-UTF8 text files) | `src/extraction.rs::decode_text` (Windows-1252 fallback) |
| Regex-candidate literal-chunk extraction (index pre-filtering) | `src/regex_literals.rs::required_literal_chunks` |
| Async robust file read (retry/timeout) + directory walk | `src/file_reader.rs` (`read_file_bytes_robust`, `enumerate_files_safely`) |
| `exclude_folders` matching logic | `file_reader.rs::is_excluded_directory` — whole path-segment match, not substring |
| Incremental JSON result cache | `src/cache.rs` (`try_load`, `save`, `compute_fingerprint`) |
| Persistent extraction-failure log (SQLite) | `src/failure_log.rs::FailureLog` |
| HTML/CSV/JSON/JSONL report generation | `src/report.rs` (`build_html_report`, `build_export_rows`, `write_csv`/`write_json`/`write_jsonl`) |
| Parallel search driver, progress/in-flight status | `src/orchestrator.rs` (`run`, `run_candidates`, `InFlightMap`, `ticker_handle`) |
| Fast re-search index policy (folder placement, auto-exclude) | `src/native_index.rs` (`index_directory`, `ensure_index_folder_excluded`, `build_or_update_corpus_index`) |
| OCR fallback for scanned/image-only PDFs | `src/ocr.rs` (feature `ocr`; called only from `extraction.rs` when a PDF has zero text-showing operators) |
| Real-fixture + mixed-corpus integration tests | `tests/fixtures.rs`, `tests/realistic_mixed_corpus.rs` (`#[ignore]`d, slow) |

### Key Relationships
`file_reader` (enumerate/read) → `extraction` (bytes→lines) → `matching` (lines→hits) → `orchestrator` (drives all three in parallel, writes `cache`/`failure_log`) → `report` / `native_index` (consume results). `regex_literals` feeds only the candidate-filter path in `orchestrator`/`native_index`, never `matching` directly. `ocr` is called only from inside `extraction::extract_pdf_lines` — extraction owns all PDF-format knowledge.

## Design Rationale
- **Problem solved**: correct whole-word/regex matching across TXT/Office/PDF with live per-file progress on slow files, fast enough for 100k+ file trees.
- **Core insight**: `fancy-regex` (not `regex`) for *every* match mode, including plain literal-mode compilation, so whole-word lookaround (`(?<![\p{L}\p{N}_])...(?![\p{L}\p{N}_])`) is available uniformly — `regex` has no lookaround by design. One engine for all modes avoids two engines' semantics quietly diverging on edge cases (e.g. `"C#"` standing alone between spaces).
- **Constraint**: DOCX/PPTX/XLSX/ZIP extraction mirrors the C# original's dependency-free approach (`zip` + regex tag-stripping), not a real OOXML parser — a "better" library would extract differently on edge cases and silently drift from the byte-for-byte-tested original. Same reasoning for PDF's hand-rolled stream/ASCII85/FlateDecode walker.

## Public API
| Export | Used By | Change Impact |
|--------|---------|----------------|
| `orchestrator::run` | `app/`, `app-egui/`, `cli/` | Breaking if signature changes — the one entry point every frontend calls |
| `models::SearchSettings` | All frontends, `cache`, `report` | New fields need a `Default`; feed `cache::compute_fingerprint` if they affect matching |
| `models::{FileSearchResult, LineHit, SearchRunResult}` | All frontends, `report`, `native_index` | Breaking if fields removed/renamed |
| `matching::apply_line_matching` | `orchestrator` | `passes_mode` relied on for correct gating — see Contracts |
| `native_index::{index_directory, ensure_index_folder_excluded}` | `orchestrator`, `app*` | Governs on-disk index placement (ADR-011) |

## External Dependencies
| Crate | Used For | Note |
|-------|----------|------|
| `fancy-regex` | All match modes | `regex` deliberately excluded — no lookaround |
| `zip` (`default-features=false`) | DOCX/PPTX/XLSX/ZIP reads | Only `xz` dropped (native `links="lzma"` conflict elsewhere in workspace) — see Pitfalls |
| `flate2` | PDF `/FlateDecode` (raw DEFLATE) | Bounded via `Read::take` |
| `rusqlite` (`bundled`) | `failure_log.rs` | `bundled` required — no system SQLite on target machines |
| `ocrs`+`rten` (feature `ocr`) | Scanned-PDF OCR fallback | Pure-Rust ONNX; `rten` pinned `=0.24.0` to match `ocrs` 0.12's expected type version |

## Data Flow
```
enumerate_files_safely → read_file_bytes_robust → extract_lines_by_extension
   → apply_line_matching (parallel, Semaphore+JoinSet)
   → FileSearchResult (progress via mpsc/ticker_handle)
   → cache::save (fingerprinted) + failure_log (on extraction error)
   → report::build_html_report / build_export_rows  [or]  native_index::build_or_update_corpus_index
```

## Entry Points
| Task | Start Here |
|------|------------|
| Run the test suite | `cargo test -p search-core` |
| Add a new file-format extractor | `src/extraction.rs` (see Patterns below) |
| Change matching/filter/gating behavior | `src/matching.rs::apply_line_matching` |
| Change parallel execution or progress reporting | `src/orchestrator.rs::run` |
| Change fast re-search index policy | `src/native_index.rs` |
| Diagnose a PDF extraction gap | `src/extraction.rs::extract_pdf_lines` (see Patterns below) |

## Contracts
- `InFlightMap` (`orchestrator.rs`) is `std::sync::Mutex`, not `tokio::sync::Mutex` — the PDF-progress/retry-status callbacks in `extraction.rs`/`file_reader.rs` are plain sync `FnMut`, not async. Live per-file progress is a hard requirement (app exists because the legacy PowerShell tool's PDF processing went silent for seconds with no way to tell "still working" from "stuck"): any change to `orchestrator.rs`/`extract_pdf_lines` must preserve the ~150ms PDF progress callback, the background `ticker_handle`, and per-file in-flight status — don't collapse into a simpler start/done model, and don't "upgrade" the mutex without also making the callbacks async.
- `matching::LineMatchOutcome.passes_mode` must stay an explicit struct field, never re-derived from `hits.is_empty()` — a prior version conflated "no hits at all" with "hits existed but failed AllInFile/Proximity gating" (see `orchestrator.rs:610`).
- `is_excluded_directory` (`file_reader.rs`) matches `exclude_folders` by whole path segment (case-insensitive), never substring — `bin` must not exclude `cabinet/`.
- `read_zip_entry_to_string` and `inflate_raw_deflate` (`extraction.rs`) must stay size-bounded (`Read::take`), independent of the orchestrator's `max_file_size_mb` gate — that gate only sees on-disk *compressed* size. Fixed after an audit found the single choke point every DOCX/PPTX/XLSX XML read goes through had no size check at all (commit `049820b`, epic #6 §62).
- `regex_literals::required_literal_chunks` must return `None` (never narrow) on any syntax it isn't fully sure about — a wrong chunk silently drops real matches, worse than not narrowing. Bounded quantifiers (`{n}`,`{n,}`,`{n,m}`) are the one deliberate exception (commit `a07bfd8`).

## Patterns

### Fixing a PDF text-extraction gap
1. Confirm the content stream is located/decompressed (`find_stream_blocks`) before assuming the matching regex is wrong.
2. Check both operand shapes: `text_re` (parenthesized-literal `Tj`/`TJ`) vs `hex_string_re` (hex-string — the *default* for headless-Chromium print-to-PDF, invoicing tools, LaTeX, not a rare edge case).
3. Hex-string operands must resolve through the file's own `/ToUnicode` CMap (`parse_tounicode_cmap`) — never treat a raw CID as a Unicode codepoint (produces wrong text, not just missing text; real bug, commit `3c4f94c`).
4. Concatenate all hex-derived chars within one content stream into a single line before pushing — some generators emit one `Tj` per glyph, fragmenting every word into single characters and silently breaking whole-word/substring search even with byte-correct text (same commit).
5. Add a fixture-backed test in `tests/fixtures.rs` reproducing the exact operand shape.

### Adding a new file-format extractor
1. Add extension(s) to `models.rs::extension_catalog::CATEGORIES` — single source of truth for both the engine's default list and the UI picker.
2. Implement in `extraction.rs`, following the existing "skip what isn't confidently handled, never guess" convention.
3. Add a real fixture (reused from the C# test harness where possible) and a case in `tests/fixtures.rs`.
4. Update `orchestrator::is_heavy_extension` if the format warrants the separate heavy-throttle semaphore (`heavy_throttle_limit` vs `throttle_limit` are independent semaphores, so light/heavy file mixes don't starve each other).

- **Index freshness is `(modified in ns, size)` per document, nothing stronger** (`native_index.rs`, `cache::ticks_from_modified`; no content hash, no filesystem watcher). A same-size edit that restores the original mtime to the nanosecond is invisible to it. Changing what the index stores or how text is extracted/normalised means bumping `INDEX_SEMANTIC_VERSION` (stamp file in the index folder; mismatch rebuilds on open).
- `build_or_update_corpus_index` keeps a sliding window of extractions in a `JoinSet` and indexes in completion order - do not go back to awaiting a fixed window in order (one slow file stalled its whole window; measured in `docs/index-audit-2026-10.md`).
- `CorpusIndexOutcome.failed_files` is capped (`MAX_FAILURE_DETAILS`); use `failed_count` / `failure_summary` for totals.
- Index metadata that cannot be read must never be treated as an empty index (`read_known`).
- `remove_orphaned_documents` is global and must keep per-id `exists()`: an id outside the current scan scope may be merely excluded by settings. `narrow_candidates` already subtracts the scope first.

## Boundaries

### Always
- Run `cargo test -p search-core` before considering any change done — must build without the `ocr` feature or any GUI crate present.
- Add a new test for new behavior; a passing build is not evidence of correctness here (see Pitfalls).

### Never
- Never add a GUI/windowing dependency to this crate, even transitively.
- Never use the `regex` crate for match compilation — only `fancy-regex`.

## Pitfalls
- PDF hex-string/CID-font PDFs silently produced zero text (default encoding for most modern generators, not an edge case) *and*, separately, could fragment every word into single characters even after that fix — both real bugs, both from commit `3c4f94c`. See Patterns above before touching PDF extraction.
- fancy-regex's combined candidate pre-check fails OPEN (`unwrap_or(true)`) while the authoritative per-filter check fails CLOSED (`unwrap_or(false)`) on the 1,000,000-step ReDoS backtrack limit — a pathological filter silently reports "no match" on the line it choked on rather than erroring the whole search (`matching.rs` module doc, verified commit `a07bfd8`).
- `zip`'s `xz` feature is deliberately disabled workspace-wide (native `links="lzma"` conflict elsewhere in the workspace) — an XZ-compressed ZIP entry now silently extracts *less* text (that entry skipped) instead of erroring; never more. Check compression method first if content goes missing from a DOCX/XLSX.
- `.native-search-index/` lives inside the searched folder (ADR-011) — `ensure_index_folder_excluded` must run before every walk or the index self-indexes. Commit `6439980`: indexing was succeeding but its status message went to a collapsed `<details>` panel nobody had expanded, making working indexing look broken — check status routing, not just the indexing logic, when "indexing isn't working" is reported.
- Literal (non-regex, non-whole-word) mode deliberately bypasses `fancy_regex` entirely via plain lowercase `str::contains` (`filters_lower`) — looks inconsistent with "one engine for all modes" above, but was a real perf fix for a "large-folder search slower than the old PowerShell tool" regression. Don't route literal mode back through regex compilation.
