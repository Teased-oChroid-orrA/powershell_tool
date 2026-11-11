# search-core

## Purpose
Owns: business logic for searching text inside files — settings/models, matching, text extraction (TXT/DOCX/PPTX/XLSX/ZIP/RTF/PDF, optional OCR), robust async file I/O, incremental caching, HTML/CSV/JSON/JSONL reporting, parallel orchestration, and the policy layer over the `native-search` fast-index crate.
Does not own: any GUI (`app/`, `app-egui/`), CLI parsing (`cli/`), or `native-search`'s own indexing internals (`native-search/src/engine.rs`) — this crate only calls that engine in-process.
Plain Rust library, zero GUI dependency — `cargo test -p search-core` must build/pass with no GUI toolchain present (a 1:1 port of the removed C# `TextInFilesSearch.Core`; see `docs/rust-rewrite-status.md`; fixtures in `test-data/`, report banner in `assets/`). Never add a dependency that pulls in Dioxus, egui, or any windowing toolkit.

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
| Regex-candidate literal-chunk extraction (index pre-filtering) | `src/regex_literals.rs::required_literal_alternatives` (OR of AND chunk sets; `required_literal_chunks` is one alternative) |
| Accent/ligature/quote folding, typo tolerance (opt-in) | `src/textfold.rs` (`fold_lower`, `FuzzyFilter`); settings `fold_accents`, `typo_tolerance` |
| Relevance order of exact results (BM25 over hit lines, filename bonus) | `src/ordering.rs` (`relevance_order`, `score_results`); found order stays the default |
| Why a line/file matched (match spans, per-filter evidence) | `src/explain.rs` (`LineExplainer::spans`, `explain_file`); computed on demand, nothing stored in `FileSearchResult` |
| Timing harness for exact search / ordering benchmark | `examples/exact_bench.rs`, `examples/exact_order_eval.rs` |
| Async robust file read (retry/timeout) + directory walk | `src/file_reader.rs` (`read_file_bytes_robust`, `enumerate_files_safely`) |
| `exclude_folders` matching logic | `file_reader.rs::is_excluded_directory` — whole path-segment match, not substring |
| Incremental JSON result cache | `src/cache.rs` (`try_load`, `save`, `compute_fingerprint`) |
| Persistent extraction-failure log (SQLite) | `src/failure_log.rs::FailureLog` |
| HTML/CSV/JSON/JSONL report generation | `src/report.rs` (`build_html_report`, `build_export_rows`, `write_csv`/`write_json`/`write_jsonl`) |
| Parallel search driver, progress/in-flight status | `src/orchestrator.rs` (`run`, `run_candidates`, `InFlightMap`, `ticker_handle`) |
| Machine-level catalog of roots/files, cross-root fast search (derived, additive; single-root search never uses it) | `src/catalog.rs` (`Catalog::register_root`, `sync_root`, `search_all`, `duplicate_groups`) |
| Stored entities with evidence + "files sharing X" (pattern matches only; feature `neural-search`) | `src/catalog_entities.rs` (`Catalog::index_entities`, `files_with_entity`, `files_sharing_entities`) |
| Fast re-search index policy (folder placement, auto-exclude) | `src/native_index.rs` (`index_directory`, `ensure_index_folder_excluded`, `build_or_update_corpus_index`) |
| Optional neural suggestion ranking over caller-provided candidates | `src/neural_search.rs` (feature `neural-search`; model execution is in `search-ml`) |
| Natural-language Find orchestration (per-condition hybrid retrieval, model judge with verified quote, weakest-condition label, negation as exclusion) | `src/nl_find.rs` (feature `nl-find`; generic over `search_llm::ConditionModel`, retrieval is a closure; tests use a fake model) |
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

- **Semantic retrieval is hybrid** (`lexical.rs`, `semantic_index::search_filtered`): short queries (<= `MAX_QUERY_TERMS` distinct terms) fuse cosine rank with BM25 rank (RRF, lexical weight 2.0) so bare numbers, spec codes and part numbers (`M8` finds `M8x1.25`) are found; longer queries (related-document discovery) stay dense-only. `SemanticHit::similarity` stays the cosine value; only order changes, and `evidence::select_evidence` still sorts selected passages by cosine. The scan keeps compact per-chunk counts (memory is proportional to chunk count for fused queries, not `limit`). Guarded by `fused_ranking_beats_dense_only_on_the_hard_retrieval_set` (40-question set in `search-ml/examples/data/retrieval_eval.tsv`; dense 34/40 -> fused 37/40; synthetic, so confirm on real documents before retuning).
- **Semantic index stamp**: `semantic_meta.embedding_version` = `search_ml::EMBEDDING_MODEL_VERSION` (1 = MiniLM-L6-v2 fp32). `SemanticIndex::open` deletes all stored vectors on mismatch, so documents re-embed on the next build. Bump the constant in `search-ml` whenever the embedding weights, pooling or normalisation change (e.g. an int8 swap); guarded by `embedding_model_version_mismatch_discards_stored_vectors`.
- **Index freshness is `(modified in ns, size)` per document, nothing stronger** (`native_index.rs`, `cache::ticks_from_modified`; no content hash, no filesystem watcher). A same-size edit that restores the original mtime to the nanosecond is invisible to it. Changing what the index stores or how text is extracted/normalised means bumping `INDEX_SEMANTIC_VERSION` (stamp file in the index folder; mismatch rebuilds on open).
- `build_or_update_corpus_index` keeps a sliding window of extractions in a `JoinSet` and indexes in completion order - do not go back to awaiting a fixed window in order (one slow file stalled its whole window; measured in `docs/index-audit-2026-10.md`).
- `CorpusIndexOutcome.failed_files` is capped (`MAX_FAILURE_DETAILS`); use `failed_count` / `failure_summary` for totals.
- Index metadata that cannot be read must never be treated as an empty index (`read_known`).
- `remove_orphaned_documents` is global and must keep per-id `exists()`: an id outside the current scan scope may be merely excluded by settings. `narrow_candidates` already subtracts the scope first.

- **Exact-match speed path (2026-11-10)** (`matching.rs`): in literal and whole-word modes with all-ASCII filters, `ascii_prefilter` (one case-insensitive regex over all filters) rejects non-candidate lines before any lowercasing; it is skipped for lines containing U+0130 (the only non-ASCII char whose lowercase holds ASCII that `(?i)` does not match; U+212A is matched by both), proved by `lowercase_to_ascii_sources_are_only_kelvin_and_dotted_i`. Whole-word on an ASCII line decides word boundaries by hand (`whole_word_ascii`, differential test against the lookaround regex) instead of running the lookaround regex once per filter. Measured on 150k lines: literal 437 -> 71 ms, whole word 5236 -> 181 ms; with filters that match most lines (3 common words) the extra prefilter costs ~20 % (836 -> ~1000 ms). Do not "simplify" these back into the regex path.
- **Opt-in folding and typo tolerance**: `SearchSettings::fold_accents` (literal and whole-word only) compares `textfold::fold_lower` of line and filter; `typo_tolerance` (1 or 2) adds `FuzzyFilter` word matches after the exact check (words under 4 letters or with digits stay exact; excludes never fuzzy). Both feed `cache::compute_fingerprint` (skipped at defaults) and make `native_index::narrow_candidates` return no candidates (the index holds unfolded text; trigrams cannot answer them). Highlighting must use `explain::LineExplainer`, not a plain substring search.
- **OCR flag of the fast index (`native_index.rs`)**: the index directory holds an `ocr-pdfs` stamp (`1`/`0`, written when a build finishes) because the per-file (modified, size) check cannot see a settings change. Turning OCR on re-extracts every PDF once on the next build; until then `narrow_candidates` counts each PDF as stale (`needs_update`, so `NarrowOutcome::is_stale` shows the update hint) and scans it directly, so scanned PDFs are not missed. Test: `turning_ocr_on_re_extracts_pdfs_once_and_leaves_other_files_alone`.
- **Result cache hardening (`cache.rs`, `orchestrator.rs`)**: entries are dropped on load when their mtime is within 2 s of the cache's own `saved_at_ns` (git's "racily clean" rule; coarse mtimes + same size could otherwise hide an edit), and `ReadError`/`UnexpectedError` entries are never reused (a locked-file error was replayed until the file was next modified). `max_embed_lines`, `fold_accents`, `typo_tolerance` join the fingerprint at non-default values.
- **`regex_literals`**: an escape with operand characters (`\x41`, `\pL`, `\1`, `\u{..}`, `\k<n>`) now bails; before, the operand characters (`41`, `L`) were kept as required literal text and would have dropped real matches. Plain character classes now break the run like `.`; top-level `a|b` becomes OR'd chunk sets (no groups). Guarded by a test that checks every chunk set against real regex matches.

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
- `nl_find` labels files "all matched" / "one weak" (needs 3+ wanted conditions) / "partial", never a percentage, and judges only the 10 best-ranked candidate files with 2 passages per (file, condition): a file with no hit for a condition inside the top-100 retrieval is unconfirmed, not judged. The language model is ~9 s per judge call on an 8 GB M1 CPU, which is why the cap exists. Language-model prompts, grammars and the quote check live in `search-llm/`; its judge grammar must keep the model's natural spacing (`{ "quote": ...`): a compact `{"quote":` grammar made Qwen3-4B emit a garbled quote (issue #0128). The model never indexes or OCRs: OCR is `ocrs`+`rten` in `ocr.rs`, embeddings are MiniLM in `search-ml`.
- **PDF/OCR line quality** (`line_quality.rs`, `docs/search-ocr-line-quality.md`): `extract_pdf_lines_capture` joins each `TJ` array into one line (`join_tj_array`), scans only streams that are at least 60% printable ASCII (`looks_like_content_stream`), then `line_quality::clean_lines` repairs Windows-1252 and Computer Modern ligature codes and drops lines with no plausible content, keeping the parallel `origins` vector aligned; OCR lines go through it per page as `Source::Ocr`. It runs before the OCR-fallback check so an all-junk text layer falls through to OCR. Bias is toward keeping: a line survives on one word-like, numeric or part-number token. Guarded by the keep and drop sets in `line_quality::tests` and `test-data/line-quality-labels.tsv`. Changing what it drops means bumping `native_index::INDEX_SEMANTIC_VERSION`, `semantic_index::CITATION_VERSION` and `cache::EXTRACTION_VERSION`. MiniLM measured useless for this and the language model is far too slow per line (see the doc); do not add a per-line model call.

- **PDF line cleaning joins soft-hyphen line breaks** (`line_quality::join_soft_hyphens`, called at the end of `clean_lines`): a line ending in U+00AD absorbs the next line, keeping the first line's page origin. Changing it changes stored text, so bump `native_index::INDEX_SEMANTIC_VERSION`, `semantic_index::CITATION_VERSION` and `cache::EXTRACTION_VERSION` together (all three were raised on 2026-11-08).
- **Ordinary hyphen breaks are repaired from the document's own words** (`line_quality::dehyphenate`, after `join_soft_hyphens`): a line ending `letters-` followed by a line starting with lowercase letters becomes `understanding` / rest-of-line only when the joined word appears whole elsewhere in the same document and the hyphenated compound (`pre-training`) appears nowhere; lines with different PDF origins (a page break) are never joined, the fragment's trailing `.,;:)]!?` moves with it, line count and page citations are unchanged. Measured on `benches/data/xlarge.pdf`: 2315 hyphen-ended lines, 1495 repaired, 23392 -> 23376 lines. Raised `cache::EXTRACTION_VERSION` to 3, `native_index::INDEX_SEMANTIC_VERSION` to 5, `semantic_index::CITATION_VERSION` to 4.
- **Extraction settings are part of cache and failure identity.** `cache::compute_fingerprint` includes `ocr_scanned_pdfs` and `pdf_timeout_seconds` (omitted at their defaults, so old caches stay valid); `failure_log::FailureLog::open_with_context(path, extraction_context(&settings))` stores a context number beside each failure (`(pdf_timeout - 15) << 1 | ocr`) and `known_failure_reason` only matches the same context, so turning OCR on or raising the PDF timeout retries files that failed before. Old databases gain the column on open. The `native_index` stamp (path, size, time) does not include the OCR setting: a PDF indexed without OCR is not re-extracted when OCR is switched on (not changed here).
- **OCR runs page by page and is cached** (`extraction.rs` OCR fallback, `ocr.rs`): image streams are decoded one at a time in file order (`jpeg_image_from_stream`, `raw_flate_image_from_stream`) with a timeout check and a progress callback before each page; decoding all pages first held every page's pixels at once and stayed silent. `ocr::cache_key` is a SHA-256 of the stream dictionary and bytes (plus model sizes and `OCR_CACHE_VERSION`); recognised raw lines (before `line_quality`) are kept in memory and, when `SearchSettings::ocr_cache_dir` is set (`extraction::configure_ocr_cache`, called by the orchestrator), in `<dir>/<key>.txt`. OCR is still only attempted when the text layer has no lines at all; a PDF mixing text pages and scanned pages is not OCR'd. `ocrs` 0.12 does not expose recognition confidence (`TextChar` is character plus box; the CTC score is dropped inside), so nothing here is called confidence except the pre-existing `low_confidence_pdf` heuristic flag.
- **Page provenance** (`pdf_page_map.rs`, feature `semantic-search`) only maps classic-xref PDFs without `/Prev`, `/XRefStm`, `/Encrypt` or object streams; anything else yields no pages (line-only citations). `real_large_pdf_keeps_bounded_page_numbers` (144-page `xlarge.pdf`) passes (release 1.8 s, debug 14 s); the mapper accepts that file (147 content-stream offsets, every xref object parses), so the logged reject (#0127) could not be reproduced and was most likely a time-truncated extraction (a debug build needs half of the old 30 s budget on an idle machine), which is why the test now passes 300 s.
- **Natural-language Find retrieval and judging** (`nl_find.rs`, `lexical.rs`): keyword terms are stemmed (`lexical::terms`, blended = blending), each condition may carry a model-written `example` used only to retrieve, strong candidate files get up to 8 judged passages and weak ones 1, and when nothing is confirmed `Fit::Unconfirmed` closest passages are returned. See `docs/search-natural-language-find.md`.
- **Semantic queries run from an in-memory snapshot** (`semantic_cache.rs`): vectors, chunk lengths and term postings per index file, rebuilt when `semantic_meta.generation` changes. Every write path in `semantic_index.rs` must call `bump_generation`; a new write path that forgets serves stale results. Indexes over `MAX_CACHED_CHUNKS` stream from SQLite instead (`search_streaming`); both paths must stay score-equivalent. Per-document freshness (`std::fs::metadata` per document per query) is still paid on every query.
- **Natural-language Find is rule-based** (`nl_rules.rs`, no model; the `search-llm` crate was removed 2026-11-08, backup of its uncommitted edits is not in git). `parse` only uses the sentence's own words; `judge` needs all key words in one or two neighbouring sentences, un-negated.
- **Retrieval knobs live in `lexical::LexicalConfig`** (stemming, fusion kind, keyword share, RRF constant, BM25 `k1`/`b`). `Default` is the benchmarked setting for passage retrieval over files (score fusion, keyword share 0.35); `LexicalConfig::rrf()` is the older rank fusion kept for the Search-help line ranking in `neural_search::best_lines`. Change a default only with numbers from `examples/retrieval_eval.rs` on all five benchmark sets (`docs/search-benchmarks.md`).
- **`ranking.rs` + `SemanticIndex::rank_features` are measurement-only**: raw per-chunk scores (similarity, BM25, length, place in file) with no extension/freshness filtering, used by `retrieval_eval --rank-experiments` (`examples/support/ranking_experiments.rs`) to score ranking ideas offline. Measured 2026-11-10 and not adopted (each loses on a benchmark set): query-length keyword share, file score = best chunk + decayed next chunks, learned linear fusion, near-duplicate removal (`evidence::select_evidence_with`, opt-in). A copied eval index folder with older path spelling duplicates every document; delete or rebuild it first.

## Re-embedding is skipped for unchanged text (2026-11-08)
`SemanticIndex::index_document_with_pages` stores a SHA-256 of the extracted lines and PDF pages (`semantic_documents.content_hash`). A file whose time or size changed but whose extracted text did not keeps its vectors and only takes the new stamp (no write-counter bump, so cached snapshots stay valid). Test: `unchanged_text_with_a_new_time_keeps_its_vectors`. Extraction still runs for such a file because the Tantivy index stores the stamp inside its documents; gating extraction on a file hash would need a stamp table or a Tantivy field and is not built. SHA-256 is enough: hashing extracted text is milliseconds against seconds of embedding (BLAKE3 measured about 5 times faster than the `sha2` crate here, which does not matter at that scale).

### Per-query cost and embedding reuse (2026-11-08)

- The snapshot search used to `stat` every indexed file on every query, one after another. The scope checks that need
  no file access (folder, hidden, size limit, extension) now run first from the indexed size, and one `stat` per
  remaining file runs across up to 8 threads (`semantic_path_listed` / `semantic_path_fresh`). Results are identical
  (SciFact nDCG@10 0.7518 before and after); only the per-query time drops, and it grows with the index.
- Re-indexing a file reuses the stored vector of every chunk whose text is unchanged
  (`SemanticIndex::stored_chunk_embeddings`); only new or edited chunks go through the model. Chunk boundaries are fixed
  every `CHUNK_CHARS`, so this pays off for appended text (logs, notes) and edits near the end; an insert near the top
  shifts every later boundary and re-embeds the rest. Whole-file reuse by content hash still comes first.
- A document whose extracted text equals another indexed document's (a copy, a renamed or moved file, a second
  revision folder) takes that document's stored chunk vectors (`SemanticIndex::donor_path`, looked up through the
  `semantic_documents_hash` index on `content_hash`, same `citation_version`); no model call. `open` now sets
  `PRAGMA foreign_keys = ON` outside the schema transaction (inside one SQLite ignores it); replacing or removing a
  document relies on the cascade to delete its chunks. Tests: `incremental_updates_embed_only_changes_and_leave_no_orphan_rows`
  (appended line, copy, replaced text, orphan cleanup, no stray chunk rows), `an_interrupted_build_resumes_without_redoing_finished_documents`.
  Build-time invalidation is `(modified, size)` via `is_current`, then the content hash inside
  `index_document_with_pages`; a file whose text changed within the same size and mtime is not seen (accepted, same as the
  exact index). Each document is one transaction, so an interrupted build keeps finished documents and the next run
  embeds only the rest.
- Chunking stays fixed 800 characters. Sentence/line/paragraph-boundary chunking (with and without 100-character
  overlap) lost to it on Cranfield and was not adopted; numbers in `docs/search-benchmarks.md` (lead merges). Do not
  retry without a different idea for what the boundary buys.
- No ANN index above `MAX_CACHED_CHUNKS`: the exact scan is cheap (about 35 ms for 250,000 chunks on one thread, 20 ms
  on 8); IVF needed 15% to 57% of the lists for recall@100 >= 0.98. `examples/ann_probe.rs` reproduces the measurement.
  The limit is memory (1.5 KB per chunk), snapshot build time and the per-file `stat`, not the dot products.
- `search_ml::embed_queries` remembers the last 32 distinct queries.
- Not done, by design: releasing the keyword index before embedding finishes. Embedding runs inside the same per-file
  loop as the exact index build and the semantic version stamp means "complete"; showing results early needs a
  product decision on what "index ready" means (partial semantic results must be labelled), not just a code change.
