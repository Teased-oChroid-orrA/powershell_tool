# Local knowledge intelligence engine: audit, gap analysis and roadmap (2026-11-10)

Tracking issue: #240. Source directive: `docs/MASTER DIRECTIVE- EVOLVE THIS APPLICATION INTO A LOCAL KNOWLEDGE INTELLIGENCE ENGINE.md`. Related: #238 (local ML roadmap for Search).

Scope and honesty. This is the first-stage audit the directive asks for. It was done by reading code, documentation and existing measurements, and by running a few cheap checks (network denial, existing tests). No source was changed by the audit. Numbers come only from `docs/search-benchmarks.md` and from the checks named below; anything not measured is listed under "Not measured" and is not estimated. The audit covers `search-core`, `search-ml`, `native-search` and the Search window in `app-tui`; the engineering solvers are out of scope.

## 1. Architecture map (what exists)

| Stage | Code | Notes |
|---|---|---|
| Discovery | `search-core/src/file_reader.rs` (`enumerate_files_safely`) | Walk of one root folder per run; extension, hidden-file and excluded-folder filters. No filesystem watcher anywhere in the workspace (grep for notify, fsevent, inotify: none). Freshness is a (modified, size) check on each run. |
| Extraction | `extraction.rs` (3.2k lines), `pdf_page_map.rs`, `ocr.rs`, `line_quality.rs` | Text, Office (docx/pptx/xlsx), zip (nested), rtf, pdf, many code/config formats. Scanned-PDF OCR (ocrs + rten, models bundled). PDF page provenance for citations. |
| Exact index | `native-search/src/engine.rs` (Tantivy), `native_index.rs` | Trigram narrowing plus full text; folder-local index, versioned (`INDEX_SEMANTIC_VERSION`, `ocr-pdfs` stamp). |
| Semantic index | `semantic_index.rs`, `semantic_cache.rs`, `search-ml` | SQLite (`semantic_documents`, `semantic_chunks`, `semantic_meta`). MiniLM-L6-v2 (384 dimensions) bundled in the binary (90 MB), run by rten. Hybrid ranking: normalised cosine plus BM25, keyword share 0.35. In-memory snapshot for query speed. Content hash lets copies and renames reuse vectors. |
| Query understanding | `nl_find.rs`, `nl_rules.rs`, `synonyms.rs`, `neural_search.rs` | Rule-based sentence Find (no model), engineering synonym expansion, corpus-term suggestions. |
| Ranking and explanation | `relevance.rs`, `ordering.rs`, `explain.rs`, `evidence.rs`, `ranking.rs` | Score meters, `relevance_order` (Best match sort), per-span match explanation, evidence selection with near-duplicate removal. |
| Entities | `entities.rs` | Pattern candidates (part numbers, units, standards) with optional model score; not persisted as a graph. |
| Derived views | `app-tui/.../search/mod.rs` | Related documents (`FindRelated`, `CompareRelated`), diff between related files, saved-search alerts (`alerts.rs`), `group_similar_documents` (near duplicates). |
| Cache | `cache.rs`, `semantic_cache.rs`, OCR page cache | Result cache keyed by fingerprint (settings, OCR flag, extraction version). |

File lifecycle today: user picks a folder and runs a search or builds an index; the walk lists files; each file is extracted to lines; exact search matches lines; the optional index stores trigram and full text in `.native-search-index/` inside that folder, and the optional semantic index stores chunk excerpts and vectors in `semantic.sqlite` beside it; results carry file, line, page and citation.

## 2. Findings

### F1. The index is per searched folder, not per machine (architectural limit)
ADR-011 puts the index inside the folder it covers. Consequences: no cross-folder retrieval ("search across unrelated folder structures" is impossible by construction), every root has its own model snapshot and version stamps, and results cannot be joined across roots. This is the single biggest gap against the directive's central vision. It also has a privacy side (F6).

### F2. No relationship layer
There are no stored entities, topics, projects, folders-as-objects or relationships. Related documents are computed on demand by similarity; near-duplicate grouping exists but is not persisted. There is no evidence/confidence model and no graph invalidation. Layers C and F are partly present (semantic, cited passages); layers D and E (relationship, context) are absent; layer B (content intelligence) has entity candidates only.

### F3. No change detection beyond a rescan
No watcher; staleness is discovered at search time. Incremental indexing exists and is correct (identical-content reuse, orphan removal, OCR re-extract once), but nothing keeps an index current in the background.

### F4. Single English embedding model
MiniLM-L6-v2, 384 dimensions, English-trained. The directive asks for multilingual consideration. The model bake-off already measured bge-small, arctic-embed-xs, e5-small, gte-small and int8 variants and bge-base (see `docs/search-benchmarks.md`); none was adopted except as opt-in Refine, which is now removed. No multilingual model has been measured.

### F5. Intelligence measured and mostly negative: do not repeat
The existing evidence matters for the roadmap. On Cranfield, SciFact, NFCorpus, MS MARCO and TREC-COVID the shipped hybrid (mean nDCG@10 0.5092, Recall@100 0.724) beat dense-only (0.4657). Measured and not adopted: query-length fusion, document-level scoring, heading boost, near-duplicate removal, learned fusion, boundary-aware chunks, IVF approximate nearest neighbour, bge-small int8, two cross-encoders. bge-base rerank-rrf won all five sets but was dropped with Refine by owner decision. Any new retrieval idea must beat the current hybrid on all five sets.

### F6. Privacy and network audit (evidence)
- No HTTP/socket client in the dependency graph used at runtime. A scan of `Cargo.lock` for network-capable crates finds only `minreq` (with `rustls`, `rustls-webpki`, `webpki-roots`), pulled by `cadrum`, the OpenCASCADE binding used by `solid-kernel` behind the off-by-default `occt` feature (build-time prebuilt download). Search crates do not depend on it.
- Source grep (`TcpStream`, `UdpSocket`, `std::net`, `tokio::net`, `reqwest`, `ureq`, `http(s)://`) over `search-core`, `search-ml`, `native-search`, `app-tui`, `cli`: no network use; the one URL is a citation string in `line_quality.rs`.
- Models are `include_bytes!` in the binary (MiniLM, tokenizer; OCR models bundled). No downloader, update checker, telemetry or crash reporter found (keyword scan).
- Executed check: under macOS `sandbox-exec` with `(deny network*)` (control: `curl` fails with "Could not resolve host"), the 19 semantic-index tests, which run real embedding inference and indexing, pass.
- Gap: `.native-search-index/` and `semantic.sqlite` are written inside the user's folder and hold extracted text (excerpts) and vectors unencrypted. A folder that is synced, backed up or committed publishes that content. Not a network leak, but the directive asks to evaluate protection at rest. Logs use path redaction (`redact_paths`) in places; a full log audit is open.
- Not yet done: monitoring outbound connection attempts of the full app (not only the test binary) on Windows; the only Windows evidence is hosted CI.

### F7. Storage
SQLite for vectors and chunks with a foreign-key cascade, an `embedding_version` stamp that wipes on mismatch, `citation_version`, content hash index and `ensure_column` additive migrations: sound and reproducible from files. Tantivy has no in-place migration (outdated index is deleted and rebuilt, documented in `native_index.rs`). Vectors are stored as float BLOBs in the same table as excerpts; memory per chunk about 1.5 KB with snapshot; streaming path above 250,000 chunks. No integrity check, backup or corruption recovery beyond rebuild. No FTS5 or graph tables; SQLite can hold a relationship graph without a new database system.

### F8. Performance (existing evidence only)
Query: Cranfield (1.4k chunks) 17 ms hybrid, SciFact (5.2k) 26 ms after the snapshot (before: 172 and 638 ms). Exact search: literal 437 to 71 ms, whole word 5236 to 181 ms on 3000 files x 150 lines (this Mac, release). OCR rerun with page cache 34.7 s to 0.09 s. No measurement above ~5k chunks of corpus or above 3000 files; none for indexing throughput at 10k, 100k, 500k or 1M files; none for memory, startup or Windows.

### F9. Strengths to preserve
Exact search semantics and cited lines; deterministic fallbacks; live progress reporting (hard requirement in `CLAUDE.md`); versioned, reproducible derived data; the five-set evaluation harness (`retrieval_eval.rs`) and the "measure before adopting" rule.

## 3. Intelligence gap analysis against layers A-F

| Layer | State | Gap |
|---|---|---|
| A Filesystem | Path, size, modified, content hash (semantic index only) | No stable file id across renames at machine level, no folder objects, no version links |
| B Content | Lines, pages, entity candidates | Entities not stored or resolved; no topics, dates, project ids |
| C Semantic | Chunk vectors, hybrid ranking, cited passages | One English model; no folder/document embeddings; no multilingual |
| D Relationship | Computed similarity and near-duplicate groups | No stored edges, evidence or confidence; no versions, references, project discovery |
| E Context | None stored | Folder and neighbour context not used in ranking |
| F Knowledge | Cited passages (extractive) | Cross-file synthesis removed on purpose (Ask removed by owner); keep extractive and cited |

Note on directive part VIII (local LLM reasoning): the owner has just removed Ask and generated answers ("Search shows cited lines only, nothing generated"), after a local probe showed the bundled small model dropped a question part and invented an unsupported answer. The roadmap therefore makes multi-document answers extractive and cited first, and treats any generative step as a separate, owner-approved decision.

## 4. Benchmark and evaluation plan

Baseline already exists for retrieval (section F5, F8). Additions needed before any change, all with the existing harness conventions (datasets outside the repo, no invented numbers):
1. Retrieval: keep the five BEIR-style sets; add a cross-folder set built from the owner's representative files (private, never committed) with labelled relevant files per question.
2. Relationship: a small hand-labelled set of file pairs (same project, version, duplicate, unrelated); report precision and recall per relationship type and per evidence class (verified, metadata, statistical).
3. Version detection: pairs from real revision chains; accuracy.
4. Scale: synthetic corpus generator for 10k, 100k, 500k, 1M files measuring indexing throughput, query latency, resident memory, index size on this Mac; Windows via CI runner only as a secondary check.
5. Privacy: `sandbox-exec` network denial for the full app on macOS (extend the test above beyond the library tests); outbound-connection monitor; Windows firewall-blocked run when hardware is available.
6. Reasoning (only if a generative step is ever approved): citation correctness and hallucination rate on a fixed question set.

## 5. Roadmap

Each item lists limitation, change, expected benefit, complexity, privacy, test and rollback. Order is by priority; each phase is independently shippable and gated by the baselines above. Tracked as checkboxes in #240.

### P0: correctness, privacy, architecture
- **P0-1 At-rest exposure of in-folder indexes (F6).** Option to place indexes in the app data folder (ADR-007 vs ADR-011 revisit) with the in-folder location kept as default for compatibility; document the risk in the Index card. Complexity low. Test: index location switch keeps results identical. Rollback: setting off.
- **P0-2 Full-app offline proof.** Script and CI step running the built app's search and index flows under network denial plus a connection monitor; record result in this doc. Low complexity, no behaviour change.
- **P0-3 Integrity and recovery.** `PRAGMA integrity_check` on open for `semantic.sqlite`, rebuild on corruption with a clear message; backup not needed because everything is derivable. Low complexity.

### P1: search quality and context
- **P1-1 Machine-level catalog.** One SQLite catalog in app data listing known roots, files (stable id, path, size, mtime, content hash) and per-root index locations, built from existing per-folder indexes; enables cross-folder search and renames across roots. Medium complexity; additive; exact search unchanged. Test: cross-folder queries over two roots equal the union of per-root results; no change to single-root results.
- **P1-2 Stored entities with evidence.** Persist `entities.rs` candidates (kind, normalised value, file, line/page, extractor, confidence, class) in the catalog; entity resolution by normalisation only at first. Enables "files sharing part number X". Measure on the labelled pair set before using in ranking.
- **P1-3 Context-aware ranking, measured.** Folder and neighbour context as a small, capped prior; adopted only if it loses on none of the five sets and wins on the cross-folder set.
- **P1-4 Multilingual embedding evaluation.** Measure one multilingual small model on the five sets plus a non-English set; adopt only under the existing rule (int8 and 384 dimensions preferred; version bump handles reindex).

### P2: performance, indexing, scale
- **P2-1 Scale baselines (section 4.4)** first; no optimisation without them.
- **P2-2 Background freshness.** Opt-in watcher (notify crate: new dependency, needs approval) that queues changed paths into the existing incremental update; progress reporting preserved.
- **P2-3 Selective and lazy embedding.** Embed on demand by folder priority and extension, background enrichment with visible progress; measure recall impact.
- **P2-4 int8 vectors** (already measured as quarter memory at recall 0.995 on the ANN probe) for indexes above the snapshot limit.

### P3: relationship graph and advanced intelligence
- **P3-1 Relationship edges in SQLite** (`relationship(src, dst, type, confidence, evidence_json, class, created, source_version)`), types FILE_IN_FOLDER, SAME_ENTITY, DUPLICATE_OF, VERSION_OF, SIMILAR_TO first. Every edge carries evidence and class; only verified or metadata edges are shown as fact; statistical edges are labelled and thresholded. Invalidated by file content hash change. Test: relationship precision and recall on the labelled set; edges must not lower retrieval metrics when used.
- **P3-2 Project discovery** by clustering over shared entities, folder lineage and similarity, shown as suggestions with evidence, never auto-applied.
- **P3-3 Version chains** from name patterns, mtime order and near-duplicate similarity; accuracy on real chains.
- **P3-4 Code intelligence** (symbols, imports) using a parser crate; evaluate need first.

### P4: discovery, answers, UX
- **P4-1 Extractive multi-document brief:** cited passages grouped by file and folder, no generation. Reuses existing evidence selection.
- **P4-2 Result explanations** extended with relationship reasons (shared entity, same folder, near duplicate) from stored edges.
- **P4-3 Related-file panel across roots** using the catalog.
- **P4-4 Knowledge gaps** (missing references) only once edges exist and precision is shown.
- **P4-5 Generative summaries or answers:** not planned; revisit only on owner request with a measured hallucination rate.

### Challenged assumptions
- A knowledge graph is justified only if P3-1 evaluation shows retrieval or navigation gains; edges that do not improve the metrics are not shipped.
- A larger embedding model or reranker is not assumed to help: Refine's win was real but cost latency and was removed by the owner; reranking returns only as an explicit opt-in with measurement.
- Visual understanding beyond OCR (diagram models) is not recommended until OCR of screenshots is measured on real screenshots.

## 6. Not measured
Scale above 5k chunks / 3000 files; indexing throughput; memory and startup; Windows performance; full-app outbound-connection monitoring; query plans and SQLite lock behaviour under concurrent write; log content audit; multilingual quality. These are baselines to establish in phase P2-1 and P0-2, not estimates.

## 7. Approval
Per the directive, no architectural change starts before the owner approves this roadmap. Approval is a checkbox in #240.

## 8. Progress log

### 2026-11-10: P0-2 offline proof (done)
`scripts/offline_proof.sh` (macOS) runs the Search test binaries under `sandbox-exec` with `(deny network*)`, after a control (`curl` must fail), and then checks the sandbox log for denials attributed to our binaries. Result on this Mac: `search-core` with `semantic-search,ocr` (271 lib tests plus integration tests, real embedding inference, indexing and OCR), `native-search` (32) and `app-tui` lib (1277: Search window flows, index, semantic, alerts) all pass with the network denied; the log shows no denied network attempt by any of them (the only denials in the window were system processes such as `findmylocateagent`). Limits: this exercises the library and the window model through tests, not the GPU window itself; Windows has no equivalent run yet (needs a firewall-blocked run on hardware). The first run also exposed a flaky OCR test that raced on a process-wide cache setting; it now uses an explicit-folder helper (`ocr::read_page` / `write_page`).

### 2026-11-10: P0-3 damaged semantic index (done)
`SemanticIndex::open` runs `PRAGMA quick_check` and recreates a damaged `semantic.sqlite` empty (test `a_damaged_database_is_rebuilt_empty_and_a_sound_one_is_kept`).

### 2026-11-10: P2-1 scale baseline, first part (10k and 100k files; exact search and fast index only)
Method: `search-core/examples/exact_bench.rs` (release build, this Mac, disk-backed temp folder, 40 folders, 150 lines of about 12 words per file, so about 12 KB per file; synthetic text; `/usr/bin/time -l` for peak memory of the whole process including file generation). One run each, not repeated: treat differences under about 20 percent as noise. Not an SLA.

| Measure | 10,000 files | 100,000 files | Ratio |
|---|---|---|---|
| Cold full scan, sequential | 1.14 s | 24.4 s | 21x |
| Cold full scan, parallel (writes cache) | 0.45 s | 5.8 s | 13x |
| Warm re-run, result cache hit | 85 ms | 891 ms | 10x |
| Fast index build | 19.6 s | 203.5 s | 10x |
| Index narrow (walk + freshness + trigram query) | 129 ms | 1.82 s | 14x |
| Verify candidates (about 1% of files) | 9 ms | 62 ms | 7x |
| Peak resident memory (whole run) | 392 MB | 1,536 MB | 3.9x |

Reading: index build is linear (about 2 ms per file here, 10x files = 10x time). Narrowing with the index is 14x slower at 100k, and it is the directory walk (689 ms) plus the freshness check against the index (1,077 ms) that dominate, not the trigram query (7 ms): at this size an index search (1.8 s) is only about 3x faster than a cold parallel scan (5.8 s) and slower than a warm result-cache hit (0.9 s). The cost that grows with the corpus is the per-search freshness pass over every file, which a background watcher (P2-2) or a persisted directory snapshot would remove. Sequential cold scan is 4x slower than parallel at both sizes (the `parallel` default question). Peak memory 1.5 GB at 100k comes from the whole process (generation, result cache, index writer); which part belongs to search is not separated yet.

Still open for P2-1 (box stays unticked): 500,000 and 1,000,000 files (a 12 KB-per-file corpus would need 6 GB and 12 GB of the 16 GB free, so these need a smaller-file corpus or an external disk), the semantic index at scale (embedding throughput, snapshot memory above the 250,000-chunk limit), startup time, Windows, and memory attributed per stage.

### 2026-11-10: P0-1 private index location (done)
`IndexLocation::AppData` ("App data (private)") is a third choice for the fast index location, after Search folder (default, unchanged) and Output folder. The index (Tantivy files and `semantic.sqlite`, which hold extracted text and vectors) then lives under `<app data>/indexes/<root key>` instead of inside a folder that may be synced, backed up or committed; the root key is the same readable-prefix-plus-hash used for Output folder, so roots never collide. With no app data folder it falls back to the in-folder location. The setting's hint now says what the index contains. Existing settings files still load (old values parse; the new one is additive). The default stays in-folder for compatibility; making private the default is an owner decision. Tests: `app_data_location_is_private_per_root_and_cycles_back`, updated cycle test; `app-tui` suite 1278 passed.


### 2026-11-10: P1-1 machine-level catalog (library done; not yet wired into the Search window)
`search-core/src/catalog.rs`: a SQLite catalog (`roots`, `files`) holding each registered folder, the directory of its existing fast index, and per-file path, size, mtime and content hash. `sync_root` fills `files` from the root's own index (`all_document_metadata`) and takes content hashes from its `semantic.sqlite` when present. `search_all` opens every registered root's index and unions the hits (best score first, ties by path); a root whose index cannot be opened is skipped and named in the result. `duplicate_groups` lists files with equal hash across roots. The catalog is derived and additive: per-folder indexes stay the source of truth (ADR-011), single-root search does not touch it, a damaged catalog file is recreated empty, and `remove_root` leaves the root's index on disk. Tests: `cross_root_search_equals_union_of_per_root_results_and_sync_lists_files` (two roots; cross-root hits equal the union of per-root hits), skipped-root and damaged-catalog test, duplicate-group test; search-core (semantic, ocr) 274 passed, clippy clean for the module. Limits: no UI or catalog location yet (callers pass a path; app data is the intended home), scores from separate Tantivy indexes are not strictly comparable across roots (the order is a merge, not a calibrated rank), and the renames-across-roots use case needs hashes, which exist only for roots with a semantic index.

### 2026-11-10: P1-1 wired into Search (catalog filled by index builds; "Also search all indexed folders")
Correction to F1: Search already searches several folders in one run (`search_paths_extra`, "more folders" setting; `runner::run_search` loops roots and merges results), so cross-folder exact search was never impossible; what was missing was a machine-level memory of which folders exist. Now every finished, non-cancelled index build registers its folder and index directory in `<app data>/catalog.sqlite` and refreshes that folder's file list (best effort, logged, never fails the build; tests do not touch the real catalog). The File menu has "Also search all indexed folders": it fills "more folders" with the other catalogued folders that still exist and tells the user how many; Find then runs the existing multi-root search, so results are the same ones the per-folder runs return. Tests: `a_built_index_is_recorded_in_the_catalog_with_its_files`, `also_indexed_folders_reports_what_it_did_and_never_adds_the_main_folder`; `app-tui` lib 1280 passed. Not verified: the menu item was not screenshotted in the GPU window, and `Catalog::search_all` (index-only cross-root hits) has no UI user yet.
