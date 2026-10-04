# toolboxes/search/ — Search Files toolbox

> TL;DR: Search Files UI over `search-core` (+ `native-search` fast re-search index). Pure `model.rs` state, async `runner.rs`, `settings_view.rs` form, `persistence.rs` for `settings-tui.json`. Business logic lives in `search-core`, never here.

## Purpose
Owns: the Search Files toolbox state, key routing, Run workspace, Settings screen/extension picker, run orchestration, and persistence (settings, recents, presets).
Does not own: search/matching/extraction (`search-core`) or the index engine (`native-search`) - consumed unchanged via `indexing.rs`.

## Code Map
| Looking for... | Go to |
|---|---|
| pure business logic (`SearchToolConfig`/`SearchRunState`/`build_settings`/`apply_progress`) | `src/toolboxes/search/model.rs` |
| `settings-tui.json` persistence, recent searches, presets | `src/toolboxes/search/persistence.rs` |
| match-highlighting + preview metadata | `src/toolboxes/search/preview.rs` |
| async search execution + report writing (real `#[tokio::test]` coverage lives here) | `src/toolboxes/search/runner.rs` |
| `SearchToolState`, pane constants, toolbox-local key routing | `src/toolboxes/search/mod.rs` |
| Run workspace render (path/filters/progress/in-flight/results/preview) | `src/toolboxes/search/run_view.rs` |
| Fast re-search index build/query bridge | `indexing.rs` |
| Settings form, extension picker (`scan_extensions`) | `settings_view.rs`, `extension_picker.rs` |

## Key Relationships
- `toolboxes/search::runner::run_search` mirrors `app/`'s `run_search` orchestration: a raw `tokio::spawn` runs `search_core::orchestrator::run`/`run_candidates` and only ever writes into its own progress channel; the *caller* (`runner::run_search` itself) drains that channel and forwards each report into the app-wide `AppEvent` channel as `AppEvent::SearchProgress`. It lives in the library, not `main.rs`, specifically so it has real `#[tokio::test]` integration coverage against tempdir fixtures (see its own test module) rather than only being exercisable by actually running the binary.
- `toolboxes/search::model::apply_progress` was originally ported from the fuller of the two now-deleted GUI heads' own `apply_progress` implementations (the other dropped `in_flight_files`/`last_completed_result` entirely, a documented parity gap on that head). Root `CLAUDE.md` calls per-file in-flight status "a hard requirement, not a nice-to-have" — this crate's implementation must keep following the fuller behavior. See Contracts.

`native-search = { path = "../native-search" }` is a direct dependency — fast re-search indexing (`toolboxes/search/indexing.rs`) uses `search_core::native_index::build_or_update_corpus_index_send` (the `Send`-bounded variant, required because this crate's tokio runtime is multi-threaded) and `NativeSearchEngine::trigram_candidate_paths` for query-time narrowing.

## Decisions
| Decision | Why | Rejected |
|---|---|---|
| Extension catalog is a fresh folder scan (`extension_picker::scan_extensions`) rather than a call into `search-core`, and shows only extensions actually present rather than a static built-in list | `search-core` exposes no public "just enumerate, don't extract" API shaped for this; scanning the real folder is also a genuine improvement over both existing heads (which only offer a static default list) | Reusing a private `search-core` walk function (not exported); keeping the plain comma-separated text field as the only option |
| Extension picker's filter (`/`) narrows an *already-scanned* list rather than triggering a fresh scan per keystroke, and `Enter` while filtering both filters and can add a not-yet-present extension in one step | Scanning is the (relatively) expensive part; filtering an in-memory `Vec<String>` is free, and reusing one key for "narrow" + "add if not found" avoids a second dedicated "add custom" keybinding | A live re-scan per keystroke; a separate `Ctrl+A`-style "add custom" binding distinct from the filter's own `Enter` |
| Fast re-search (`index.enabled`) makes a search self-sufficient: `runner.rs::narrow_via_index` builds the index first when it is missing/empty (no `meta.json`)/unreadable (progress streams like a manual build, run cancel cancels it), uses an existing one as-is, and - if the folder changed since the build - emits `AppEvent::IndexStale` so the UI offers an incremental update *after* the search ends | The user asked for it (reverses the earlier "never auto-create" rule). Correctness never depends on the prompt: `narrow_candidates` always includes new/changed files | Silently rebuilding/updating without asking; blocking the search on the update prompt |

## Entry Points
| Task | Start Here |
|---|---|
| Add/change a Search Files setting | `src/toolboxes/search/model.rs` (`SearchToolConfig` field + `build_settings`) → wherever it's editable in the UI (`run_view.rs` for Path/Filters; the Settings view, if present — check current state, see Boundaries) → `persistence.rs` (`PersistedSearchSettings`, with `#[serde(default)]`/`Option<T>` for the new field) |
| Change progress/live-run behavior | `src/toolboxes/search/model.rs` (`apply_progress`, `SearchRunState`) — preserve full `in_flight_files`/incremental-`results` fidelity, see Contracts |
| Change the async search/report-writing flow | `src/toolboxes/search/runner.rs` — has its own `#[tokio::test]` suite against tempdir fixtures; extend those tests alongside any behavior change |
| Debug settings not persisting/loading correctly | `src/toolboxes/search/persistence.rs` (`PersistedSearchSettings`/`PersistedFile`, `load`/`save`) — check the `#[serde(default)]`/`Option<T>` shape on any new field first |

## Contracts
- `toolboxes::search::model::apply_progress` must keep consuming the FULL `search_core::models::SearchProgressReport` — `in_flight_files` overwritten wholesale every report, `last_completed_result` appended into `results` (case-insensitive dedup by `full_name`) whenever its status is `Hit` — never collapsed into a coarser aggregate-only update. This is root `CLAUDE.md`'s "per-file in-flight status is a hard requirement" invariant, and the one place this crate could silently regress to `app-egui/`'s known-thinner behavior.
- `toolboxes::search::run_or_notify` is the single entry point for starting a run (both the Path/Filters field's Enter key and the command palette's "Run search" command call it) — do not call `start_run` directly from a new call site, or the two invocation paths can drift into inconsistent failure messaging (this happened once already during Phase 2/3 wiring: the palette path skipped the "enter a path first" toast until unified).
- Text-field key routing (`toolboxes/search::edit_buffer_key` and the `PANE_PATH`/`PANE_FILTERS` match arms in `handle_key`) must only consume `KeyCode::Char` when `key.modifiers.is_empty()` (or `SHIFT` only) — a `Ctrl`-modified character must fall through unconsumed so global bindings (Ctrl+P above all) keep working while a text field has focus. Enter is handled by the caller (submits), never inside the shared buffer-editing helper, because closing over `state` for both the buffer mutation and a submit callback in the same expression does not borrow-check (a real error hit while building this: "closure requires unique access to `*state` but it is already borrowed") — keep Enter's `run_or_notify(state, ...)` call as a separate match arm ahead of the generic edit helper, not folded into it.

## Pitfalls
- **Post-search summary (`diagnostics.rs`)**: `runner::run_search` fills a `SearchDiagnostics` and sends `AppEvent::SearchDiagnostics` just *before* a successful `SearchFinished` (never on cancel/error); `run_view` draws it in the ticker box once the run and any index build are done. It must stay counts/sizes/timings only - the same text is written to the debug log, which is path-free by design. Index health is read off the async workers (`spawn_blocking`).
- `runner::tests::cancelling_mid_run_stops_promptly_and_reports_cancelled` used to be intermittently flaky (failed roughly 1 in 5-8 full-suite runs): it raced a fixed `sleep(1ms)` against real orchestrator work over a 20-tiny-file fixture, and on a fast/loaded run the whole search could finish before the sleep elapsed, so cancellation landed too late and the run reported success instead of `Cancelled`. Fixed by synchronizing on the first real `AppEvent::SearchProgress` event instead of a guessed duration - `orchestrator::run` unconditionally sends an `is_enumerating` progress report before any file work starts, so waiting for it is a deterministic "the run is genuinely in flight" signal with no risk of hanging. Verified stable across 20 consecutive full-suite runs after the fix. If any other test in this crate uses a fixed `sleep` to race a background task, prefer the same pattern (wait on real evidence via the event/progress channel) over tuning the duration.

## Filter groups (per-group match modes)
The Filters field is parsed by `model::parse_filter_groups`: `;` separates groups, a trailing `[any]`/`[all]`/`[near N]` tag sets that group's mode (untagged = the global Settings mode/proximity); `[not]` makes it an exclusion (a file containing any of its filters is dropped), `[not line]` only ignores those lines. The primary is the first non-exclusion group. The group summary/tip is always on the hint row (run status lives in the progress panel, so it can never hide it). `house ; floor, two [near 3]` = "house" per the global mode AND "floor"+"two" within 3 lines. No `;` = one group, identical to before; no extra screen rows (idle hint row shows the parsed summary). Engine side: `SearchSettings::filter_groups` (`FilterGroup`), matched in `search-core/src/matching.rs::apply_line_matching` - every group must have a hit and pass its own mode; exclusion groups (`FilterGroup::exclude`) are pooled by scope with the global excludes - file scope is a separate pre-pass, line scope feeds every pass (`CompiledMatchState::build`); hits are merged for highlighting; cache fingerprint includes groups. Index narrowing still uses the primary filters only, which stays a safe superset. `;`/trailing-`[...]` are not escapable in regex filters.

## Fast index contracts (verified by `runner.rs`/`native_index.rs` tests)
- **The index is only a candidate pre-filter and must never hide a real match.** `narrow_candidates` (a) uses the literal trigram query for plain filters, but for `use_regex` only the provable literal chunks (`regex_literals::required_literal_chunks`) - the raw pattern as a literal dropped real matches; no provable literal => full scan; (b) intersects with a fresh walk under the *current* extension/exclude/hidden settings; (c) always includes files missing from the index or whose mtime/size differ (added/edited after the build).
- **`IndexBuildFinished` is always sent**, even if the build task panics (`build_or_rebuild_index` runs the build in its own task and maps a `JoinError`). A build is only "ready" after the finished index is reopened and a document looked up (`verify_built_index`).
- Query-time opens must not take the Tantivy writer lock: `NativeSearchEngine` creates its writer lazily on first write.
- A failed commit loses its whole batch (writer is dropped). `build_or_update_corpus_index` keeps the staged batch and replays it (`commit_with_replay`, 4 attempts); only a permanent failure moves those docs from `indexed` to `failed`. Binary / no-text files (e.g. scanned PDFs) are stored as empty docs (`no_text_count`), never failures, so they are not reported stale.
- Staleness (`NarrowOutcome::is_stale`) = in-scope files that are missing/changed in the index and under `max_file_size_mb`, plus indexed docs whose file is gone. Declining the update prompt is per search; it is not sticky.
- Run progress is `widgets/progress_bar.rs` (2 rows: gradient 1/8-cell bar + stats line that sheds trailing segments to fit); stats = status text, files/s, ETA, elapsed from `SearchRunState`. Index progress owns that panel and the in-flight box (full wrapped path) while `index_run.is_building`; the index status row, toasts and confirm dialogs wrap rather than truncate (`hint_panel::wrapped_height`).
- Build is cancellable (`Command::CancelIndexBuild`); a cancel commits what was indexed and reports `cancelled`.
- Windows-only index failures are not reproducible on the dev machine: read `toolbench-debug.log` (preflight probes the index folder with a real write, flags UNC/OneDrive/long paths, logs OS error codes; never paths or file names) before theorizing.

## Boundaries
### Always
- Preserve `apply_progress`'s full `in_flight_files`/incremental-`results` fidelity when touching Search Files progress handling.
### Never
- Call `search_core::orchestrator::run`/`run_candidates` with `progress: None` from this crate — that's `cli/`'s deliberate headless shortcut ("a terminal can't easily redraw anyway" — except this crate is exactly the head that can); always thread a real progress channel.
- Call `native_index::build_or_update_corpus_index` (the plain, non-`Send` variant) from this crate — always use `build_or_update_corpus_index_send`, since this crate's tokio runtime is multi-threaded.
- One build path: `Build / update` updates an existing index incrementally; an index that cannot be opened/reopened (`NsStatus::CorruptIndex`) is wiped and rebuilt from scratch once (`indexing::build_or_rebuild_index`). There is no separate "rebuild" command or force flag.
- `open_report_when_done` is honored in `app.rs` (`ReportWritten` -> `Effect::OpenPath`); it also forces HTML output even when `export_html` is off.
- Start a second index build while one runs (one Tantivy writer per index folder): `start_index_build` and `run_or_notify` both refuse while `index_run.is_building`.

## Settings View
`toolboxes/search/settings_view.rs` exists: a full scrollable form over every
`SearchToolConfig` field (`s` from the Results pane to open, `Esc` to leave -
only when nothing is mid-edit, since `Esc` also cancels an in-progress field
edit). One flat field-index `match` (`FIELDS: &[FieldDef]` + parallel
`field_value_string`/`apply_text_edit`/`apply_toggle`/`bool_field(_mut)`
functions keyed by array index) rather than a generic field/widget
abstraction - deliberate, since only one toolbox needs a settings form so
far (see the "don't abstract prematurely" note elsewhere in this file).
Text/number fields use an explicit edit-mode (`Enter` to start, prefilled
with the current value; `Enter` commits, `Esc` cancels) - invalid numeric
input on commit is silently ignored, leaving the previous value in place,
rather than blocking the whole form. Bool/enum fields toggle/cycle
immediately on `Space` or `Enter`, no edit mode. Extensions have their own
modal picker (`extension_picker.rs`): `Enter` on the Extensions field fires
`Effect::ScanExtensions`, which walks the configured search folder
(`scan_extensions`, mirroring `search-core::file_reader::enumerate_files_safely`'s
walk semantics) and opens a checkbox catalog of only the extensions actually
present - a real improvement over both `app/`'s and `app-egui/`'s static
built-in extension lists, not a scope cut.

The Settings screen also has two more sections - Recent searches and Saved
presets - reached by `Tab`/`Shift+Tab` (a `Section` enum: `Fields` ->
`Recents` -> `Presets` -> wraps). `Enter` on a recent search copies its
`search_path`/`filters_text` onto the live config; `Enter` on a preset calls
`PersistedSearchSettings::apply_to`, applying every field. Neither section
starts a run or touches disk by itself - applying only changes what a
subsequent Run would use, mirroring both existing heads' "apply preset"
semantics. Both sections collapse (Fields-only) below a ~12-row minimum
height rather than attempting a cramped 3-way split. In the Presets section, `a` starts naming a new preset
(`view.naming_preset`, own text-edit mode, `Enter` commits and pushes a
`SavedPreset` snapshot of the live config, `Esc` cancels), `r` renames the
selected preset in place (`view.renaming_preset`, same text-edit mode
prefilled with the preset's current name, `Enter` commits by mutating that
preset's `name` rather than pushing a new one), and `d` deletes the
selected preset - `a`/`d` ported from `app/`'s
`save_current_as_preset`/`delete_preset`; `r` has no equivalent in either
existing head. All three mutating actions return `Effect::PersistSearchSettings`
to flush `settings-tui.json`.

## Public API
Crate-internal. `mod.rs`: `SearchToolState`, `ToolboxScreen`, `PANE_PATH`/`PANE_FILTERS`/`PANE_RESULTS`/`PANE_COUNT`, `handle_key`, `run_or_notify` (the single run entry point), `start_run`, `request_cancel`, `start_index_build`. `runner::run_search` drives `search-core` and forwards progress as `AppEvent::SearchProgress`. `persistence`: `PersistedSearchSettings`.

## Design Rationale
See Decisions above for the index, extension-catalog, and runner choices. In short: `model.rs` stays pure so `runner.rs` can have real `#[tokio::test]` coverage, and the index only narrows an existing on-disk index, never builds one as a side effect.

## Patterns
- **Add a setting:** `SearchToolConfig` field plus `build_settings` in `model.rs`, then the editing UI, then `PersistedSearchSettings` with `#[serde(default)]`/`Option<T>` (see Entry Points).
- **Add a side effect:** new `Effect` variant executed in `main.rs`/`runner.rs`, never inline in `handle_key`.
- **Start a run from a new place:** call `run_or_notify`, not `start_run`.

## Navigation
Parent: `app-tui/AGENTS.md` (crate-wide contracts: `Effect` reducer rule, `Number`-row editing, Caps-Lock key patterns, scroll widgets, disk-space `-p app-tui` scoping). Depends on `search-core/AGENTS.md` and `native-search/AGENTS.md`.
