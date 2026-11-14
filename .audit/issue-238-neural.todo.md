# Issue #238 implementation checklist

Goal: Complete all 12 Search ML roadmap items with local, reviewable behavior. Keep exact matching unchanged. Keep model/inference in `search-ml`, search/index policy in `search-core`, and UI/state in `app-tui`.

- [ ] Ground current branches, runtime, settings persistence, and UI entry points.
- [ ] Sketch bounded data shapes and module ownership before each cross-layer feature.
- [ ] Add / reuse behavior for all 12 roadmap items; update checklist only with proof.
- [ ] Verify focused tests, `cargo test -p search-core`, and app build/render paths.
- [ ] Audit findings, update project memory, and report remaining constraints honestly.

Exit predicate: all twelve roadmap items have bounded local behavior, tests, and modular ownership; exact query parsing, matching, and result membership remain unchanged.
Scope: twelve independent feature slices plus integration and verification. Model runtime and API are grounded; feedback is implemented. Ask the Index now presents unchanged cited lines per explicit question part. The bundled generator remains available for probes but is not used for answers after it fabricated text on an unsupported question. Bounded page citations cover supported PDFs; real-world corpus validation remains open. Calibrated OCR recognition confidence and alert materiality remain open.
Rigor: high for persisted judgments, generated answers, document access, and saved-search alerts; focused checks per reversible slice.

Roadmap items:
- [x] Context-aware autocomplete from history, folders, types, corpus.
- [x] Plain-language query preview (already implemented; preserve).
- [x] Transparent engineering synonym expansion.
- [x] Hybrid semantic search beside exact search.
- [x] Explainable relevance ranking.
- [x] Optional local relevance feedback with reset/export.
- [x] Source-cited extractive summaries. MiniLM selects up to five unchanged, cited source lines from bounded, evenly sampled document text; asynchronous Search UI cancels stale requests and caps inference to one active worker.
- [~] Question answering. The Index popup uses MiniLM to rank unchanged, cited source lines independently for up to three explicit questions. It shows only source text, with verified PDF page numbers when available. Unsupported PDF forms and OCR lines keep line-only citations. Ranked lines are possible evidence, not verified answers. Verified synthesis or reliable abstention remains open.
- [~] Engineering entity extraction. Bounded candidate extraction, MiniLM token-vector ranking, and asynchronous cited preview suggestions are integrated. On 59 independently worded held-out spans, unrestricted top-one is 41/59; exact-pattern coverage is 59/59; MiniLM restricted to pattern-supported categories is 56/59 versus 54/59 for the first pattern hint. The benchmark covers known candidate spans, not corpus-level false positives or recall.
- [~] Selected-document lookup returns cited related files and exact extracted-line diffs; raw MiniLM cosine is now displayed with an uncalibrated-score warning. Calibrated duplicate/revision classification remains open.
- [~] Local OCR/drawing text discovery with confidence/highlights. Existing bundled OCR finds text; exact search highlights it. OCR provenance now triggers a source-review warning. `cargo test -p search-core --features ocr ocr_extracts_real_text_from_the_real_scanned_pdf_fixture -- --ignored --nocapture` passes. `ocrs` 0.12 and latest 0.13.1 expose text-detection probability, not recognized-character confidence; the detector map requires a separate inference pass.
- [x] Zero-result typo/synonym/proximity suggestions. Typos and synonyms remain explicit user choices; mode-rejection diagnostics now enable an explicit +3-line exact proximity rerun.
- [~] Saved-search alerts report new and updated exact-hit files, with MiniLM distances shown as uncalibrated review signals. Materiality calibration remains open.

Ask the Index reliability checkpoint (2026-11-07):
- A bundled-model probe answered a combined torque/temperature question with only `Applied torque`; separate questions returned `Applied torque` and `120 C`. An unsupported fatigue-life question returned `torsional stress`. See `search-ml/examples/qa_complex_probe.rs`.
- Ask the Index now retrieves each of up to three explicit questions separately and displays only unchanged, cited source lines. It does not publish generated prose or claim the ranked lines prove an answer. A fourth question returns a visible limit error.
- `cargo test -p search-ml --lib bundled_generator` passed 2 tests; `cargo test -p search-core` passed 180 tests; `cargo test -p search-core --features semantic-search` passed 217 tests; focused Search UI suite passed 42 tests. The Search UI also checks source identity and text for two question parts.
- A verified synthesized answer or automatic abstention cannot be claimed from this generator and uncalibrated similarity. Broader held-out question/answer evidence is required before adding either.

Saved-search alert architecture slice:
- [x] Ground Search persistence, result delivery, and neural embedding boundaries.
- [x] Compare JSON-sidecar and SQLite storage; select versioned JSON sidecars with atomic replacement.
- [x] Implement bounded core alert snapshots using 8-bit bundled MiniLM vectors, scoped settings, sampled evidence fingerprints, and explicit baseline replacement.
- [x] Wire save/capture/check/delete actions into the Index panel with isolated async search and the 32-alert cap; lifecycle test passes.
- [~] Show exact changed paths and MiniLM cosine distances in the Index panel, capped at ten visible rows. Scores remain review signals; no threshold labels changes material.
- [x] Verify a new file and updated hit are reported, repeated checks are idempotent in core, and incomplete runs preserve prior baselines.

Saved-alert implementation checkpoint:
- Core snapshots retain at most 2,000 files and 2 KiB sampled evidence per file; sidecars cap at 2 MiB.
- Exact hit membership remains the source of new/updated file alerts. MiniLM distance is surfaced as a review signal, not a calibrated materiality verdict.
- A Flan-T5-small binary prompt probe predicted `IMMATERIAL` for all 16 hand-labeled changes (8/16, majority baseline); no bundled model supports a materiality verdict.
- Sample fingerprint combines up to three exact-hit contexts and six evenly spaced document lines; source text is not persisted.
- `cargo test -p search-core --features neural-search,semantic-search alerts::tests -- --nocapture`: 4 passed.

Autonomous run steps (poteto-mode):
- **State the exit condition as a checkable predicate before the first iteration (tests green, repro fixed, all N PRs merged, pixel-diff zero).**
- **Pick the wake mechanism using Claude Code's `loop` skill (a built-in, not a pstack skill). An event to watch (CI, a merge, a ref advancing) gets a watcher subagent that wakes you on the event, with a long time-based heartbeat as fallback. No event gets a fixed-interval heartbeat sized to when the result is worth re-checking. Pick each subagent's reasoning level for the job, per the Subagents section.** skip: no event-watching target; execute finite local roadmap and report limits.
- **Each iteration makes the smallest change the evidence justifies, verifies it against the predicate, commits if it advanced, discards changes that didn't help. Belt-and-suspenders that "might help" gets reverted, not left to ride. Sequence the work via the sequence-verifiable-units principle skill, verifying each unit before the next instead of batching checks at the end.**
- **Mid-run discoveries that support the goal are yours. Address broken skills, related bugs, flaky verifiers, review noise, tooling failures, orphaned follow-ups, and fixable drift yourself via poteto-mode. Put out-of-band fixes in their own PR. Do not park reversible work for the human or use `AskUserQuestion`. Surface only irreversible actions, genuine product or preference calls no experiment can settle, or a real dead end. Keep the predicate as the main drive, and return to it after each side fix.**
- **Checkpoint every iteration via the show-me-your-work skill, a row for what changed and whether the predicate moved.**
- **Stop when the predicate is met. A plateau is not a stop, so keep going and pivot your approach to push past it. Surface a genuine dead end rather than spinning, and never relax the predicate to declare victory.**

- Semantic sidecar cleanup now removes deleted files after complete scans. `cargo test -p search-core` passes.

Current iteration verified:
- Entity preview now presents MiniLM's highest-scoring category only among exact-pattern hints, preserving raw six-class scores and candidate spans. Held-out probe (59 spans): constrained rank 56/59 vs first pattern hint 54/59; unrestricted neural top-one 41/59. `cargo test -p search-core` passed (180 passed, 8 ignored); neural feature suite passed (198 passed, 8 ignored); `cargo check -p app-tui --lib --features neural-search,semantic-search` passed (367 crates); focused entity UI test passed (1). Corpus-level precision/recall remains unverified.
- Generated Q&A now uses a compact question/context Flan-T5 prompt; caller passes ranked source lines and retains citations only for lines admitted by the token budget. `cargo test -p search-ml` passed (2 tests); `cargo test -p search-core --features semantic-search` passed (212 passed, 8 ignored). The initial app-tui test build exhausted disk with debug info; a later stripped-debug Search UI test passed, and `cargo check` passed. Actual rendered UI remains unverified.
- Relevance sorting keeps found order until matching scores arrive. `cargo test -p app-tui --lib --features neural-search relevance_sort_ -- --nocapture` passed (2 tests).
- Scoped semantic retrieval honors live roots, extension selection, hidden-file policy, file-size limit, and excluded folders. `cargo test -p search-core --features semantic-search,neural-search` passed (189 passed, 8 ignored); cited-hit UI test passed.
- Saved-alert persistence round-trip and the async exact-search UI lifecycle test pass. The Index panel shows the latest ten new/updated paths with explicitly uncalibrated cosine distances. `cargo test -p search-core` passes (180 passed, 8 ignored). Materiality calibration remains open.
- Exact results remain immutable. Ranking only permutes their indices; semantic hits remain separately cited.
- Workspace-wide `cargo fmt --check` is not a usable gate here: baseline files across workspace are unformatted. Format only files touched, then inspect `git diff --check`.
- Zero-result typo correction is now implemented as an explicit suggestion and ranked with other candidates by MiniLM. Proximity suggestions remain open. Focused core and app tests pass.
- Entity extraction now runs off the Search UI thread for the selected full-document preview. Results are bound to preview generation and path, cancelled with preview changes, limited to five visible suggestions, and cited by source line. Similarity is labeled as a ranking signal, not confidence. Focused Search tests passed (33); default `cargo test -p search-core` passed (180, 8 ignored).
- The default core test failure from the neural-only `entity_rank_probe` example was fixed with Cargo `required-features = ["neural-search"]`.
- A `cargo fmt -- <paths>` attempt formatted the workspace. Restored 547 tracked files that were clean at turn start. Pre-existing dirty Rust files may retain formatter-only churn; tracked as project-memory issue #0095 and not committed.
- Evidence brief: Search's Index menu shows up to three complete cited excerpts, favors distinct files, caps total quote text at 2,400 Unicode characters, and labels them as possible evidence rather than answers. This is partial progress only. It lacks a separate question field, page references, and generated answers.
- Remaining work includes complete cited summaries/Q&A, validating entity categories on held-out data and broadening beyond the selected preview, duplicate/revision comparison, OCR confidence/highlighting, proximity suggestions, and saved-search alerts.

Current slice: engineering entity review signals; classifier evaluation next
- [x] Add bounded MiniLM centroid/MMR selection of unchanged source lines with line citations.
- [x] Add asynchronous Index-panel action, selected-file identity checks, cancellation, and a single active summary worker.
- [x] Verify input cap, sample cap, source citations, empty/cancelled input, stale selection behavior, unchanged exact results, and UI presentation model.
- [ ] Inspect the Index popup in a running native Search Workbench; the app launches, but this session could not open the Search tile through the automation surface.
- Proof: `cargo test -p search-core` (180 passed, 8 ignored); neural/semantic core suite (208 passed, 8 ignored before adding the empty/cancelled case); focused summary core tests (3 passed); full app suite (1,283 passed, 31 ignored before the cancellation regression); two focused app summary tests passed after resource/cancellation fixes; `git diff --check` passed.
- Cross-review found and fixed an oversized preview copy and overlapping cancelled inference. Project-memory issue #0107 is closed with regression evidence.

Explainable relevance feature checkpoint:
- **Blocking first steps.** Confirm search-ml embedding API, search-core neural feature gate, SearchSession async event loop, and existing result sort path.
- **Independent workstreams.** One cross-layer feature; core scorer and UI action share a score shape, so parallel writes would race.
- **Shared mutable state.** Keep exact results immutable; score only a compact snapshot and accept it only for the matching query/run.
- **Smallest safe decomposition.** One owner across search-core scorer and app-tui presentation; verify score ordering, explanations, and unchanged exact result set.

Semantic async scope race closed:
- Semantic completion events now carry query plus each root/index/scope snapshot; poll accepts only if the active request and current configuration still match.
- Scope snapshot includes index location, output folder, roots, extension policy, exclusions, hidden-file policy, and size limit.
- Regression confirms settings changed before event polling discard hits and stop request progress.
- `cargo test -p app-tui --lib --features neural-search,semantic-search gpu_viewer::editor::search::tests::`: 31 passed.
- Fixture now indexes live mtime/size, matching the freshness guard.

Current slice: engineering entity extraction
- [x] Confirm issue scope and existing inference boundary.
- [x] Compare two bounded entity-extraction designs against a small MiniLM probe. The 8/12 model count is descriptive only; the claimed 9/12 baseline was hard-coded, so comparison is inconclusive. See `.audit/entity-eval-2026-11-07.md`.
- [x] Expose normalized contextual MiniLM token vectors with source byte ranges; reject over-limit input instead of silently truncating it. `cargo run -p search-ml --example token_probe` passed; see `search-ml/examples/token_probe.rs`.
- [ ] Validate neural category ranking with a larger independent held-out span set against a real pattern-extractor baseline; current probe is 9/12 top-one and 11/12 top-two.
- [~] Extract cited engineering entities from selected full-document previews asynchronously without changing exact matching; broader corpus discovery remains open.
- [x] Present bounded reviewable candidates with source lines; label neural similarity as a ranking signal, not confidence.
- [ ] Verify entity categories, offsets/citations, resource bounds, offline use, and exact result identity.

Other verified correction during this iteration:
- Semantic citations now appear and open only while their saved request key matches the current query and search scope. Feature build passed; runtime check remains open.
- Restored seven search-core files changed only by recursive formatting after byte-for-byte comparison with formatted `HEAD` sources. Existing edits in other files remain intact.

Local relevance feedback complete:
- Added opt-in Relevant / Not relevant labels for current exact results; feedback reweights existing neural relevance signals and exposes baseline scores.
- Profile stores at most 256 labels and <=64 KiB, with no queries, file paths, filenames, excerpts, or embeddings. Reset disables and clears; export creates a new user-selected file and refuses overwrite.
- Process-wide persistence lock shares updates across Search windows. Exact result membership remains unchanged.
- Proof: `cargo test -p search-core --features neural-search relevance::tests` (3 passed); app feedback persistence tests (3 passed); Search workbench tests with `neural-search,semantic-search` (32 passed); feedback UI test confirms exact paths unchanged.
- Limitation: feedback learns broad factor preferences, not query-specific labels; no quality gain claimed without a held-out relevance set.

Question answering checkpoint:
- Data shape: each `CitedAnswer` carries an unchanged answer line, its complete source excerpt/citation, and raw MiniLM cosine similarity.
- MiniLM ranks every line in up to three bounded evidence excerpts, in batches of 128; a late relevant line cannot be sampled away. UI calls them extractive answer candidates and warns similarity is not confidence.
- Exact search and its result membership remain unchanged.
- Proof: late-line regression passed; focused Search UI test passed after batching; `cargo test -p search-core` (180 passed, 8 ignored); neural/semantic Search Core suite (211 passed, 8 ignored); full neural/semantic `app-tui` library suite (1,248 passed, 31 ignored).
- Remaining: generated answer prose and page-level PDF citations need a decoder model and page-aware PDF extraction. No such model is bundled.

Engineering entity review checkpoint:
- The selected-file preview now labels the MiniLM cosine leader separately from deterministic pattern hints when hints are ambiguous or conflict with the neural leader.
- Raw cosine scores and their order remain unchanged; the UI still labels similarity as a ranking signal, not confidence.
- Proof: focused entity presentation test passed; all 40 Search Workbench tests passed with `neural-search,semantic-search`; `git diff --check` passed.
- Runtime screenshot remains unverified: `cargo run -p app-tui -- --gpu-dashboard` started (`FEA_GPU_READY`), but the computer-use surface did not expose the new window.
- Entity accuracy remains open pending independent held-out data; current small probe is not validation.

PDF citation design phases:
- [x] Ground extraction, exact matching, semantic indexing, and answer citation flow; code trace confirms semantic corpus indexing bypasses the exact-result cache.
- [x] Compare a bounded classic-xref scanner with `lopdf`; reject eager parsing because aggregate memory is not capped.
- [x] Keep page provenance as optional semantic-side metadata; exact extracted strings and cache remain unchanged.
- [x] Implement capped extraction origins, page-homogeneous chunks, SQLite migration, and optional page labels.
- [x] Verify synthetic multi-page provenance, old SQLite migration, exact extraction behavior, and feature-off/core builds.
- [x] Record test evidence and the supported-subset/OCR limits in `.audit/pdf-page-citations-grounding.md`.
- [ ] Test additional real-world PDF generators and the large corpus; unsupported mappings remain safe and line-only.
