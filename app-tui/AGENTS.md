# app-tui/ — ratatui Terminal GUI ("Toolbench")

> TL;DR: The sole active GUI head — ratatui/crossterm terminal UI (the earlier `app/` dioxus-native and `app-egui/` egui heads are deleted; no cross-section sketches, derivation view, or PINN Stress Solver exist anywhere in this repo anymore - see root `CLAUDE.md`). Nine real toolboxes: Eccentric Bushing (spin capacity of an offset-bore bushing, `eccentric-bushing`; inputs shared with the Bushing Workbench), Search Files (business logic in `search-core`/`native-search`), Fastener Holes (original `domain/`, unique to this crate), Bushing Workbench (`bushing-solver`/`mechanics-core`), Pressure Vessel Analyzer (`pressure-vessel-solver`/`mechanics-core`, plus thermal stress), Preload Analysis (`fastened-joint-solver`, unique to this crate), Lug Analysis (`lug-solver`, rigid-pin contact FE run on a worker with an auto re-run), an FEA Workbench (define a sketch / extrusion / imported-mesh problem, solve it on `fea-core` through `fea-problem`, colour contour; mesh preview and small solves re-run on a worker), and Material Lookup (browser/compare over `mechanics-core`'s MIL-HDBK-5J handbook, also the lug's and the workbench's material picker). Dupes/Rename/Logs rail slots are placeholders. Bushing/Pressure Vessel/Preload `model.rs` only bridge solver crates into UI state - the toolbox computes nothing itself. Shared UI pattern: field lists grouped under `Header` rows plus a bottom Hint panel (`widgets/hint_panel.rs`, `model.rs::field_hint`); Results panes use `widgets/scroll_paragraph.rs`. Per-toolbox detail is in Code Map and Pitfalls below.

## Purpose
Owns: terminal lifecycle (`main.rs`), the shell chrome (topbar/rail/status bar/command palette/help overlay/toasts, `widgets/`), the app-wide reducer (`app.rs`), shared `library.rs`, and eight toolbox modules under `src/toolboxes/` (each has its own `AGENTS.md`, see Downlinks).
Does not own: search/matching/extraction (`search-core`), the index engine (`native-search`), or any solver math (`bushing-solver`, `pressure-vessel-solver`, `fastened-joint-solver`, `mechanics-core`). Toolbox `model.rs` files only bridge those crates into UI state; Fastener Holes is the exception (own `domain/`).

**Status**: Search Files, Fastener Holes, Bushing Workbench, Eccentric Bushing, Pressure Vessel Analyzer, Preload Analysis, Lug Analysis, FEA Workbench, Material Lookup are real; Dupes/Rename/Logs are placeholders. Pure terminal I/O, no windowing/GPU dependency.

## Code Map

### Find It Fast
| Looking for... | Go to |
|---|---|
| Terminal init/teardown, async event loop, `Effect` execution | `src/main.rs` |
| User-added catalog infrastructure (JSON import/export, exact-duplicate pruning, conflict queue, labels) - backs Bushing's reamer, material, and Bushing ID libraries; reuse for any new library, don't copy | `src/library.rs` |
| `AppState`/`AppEvent`/`Effect`/`handle_event` — the whole decision layer | `src/app.rs` |
| `ToolId`, `NavigationState`, `FocusState`/`FocusArea`, responsive breakpoints | `src/nav.rs` |
| Design tokens (`Theme`), `NO_COLOR` handling, reduced-color fallback palette | `src/theme.rs` |
| Modal overlay state (palette/help/confirm) | `src/modal.rs` |
| Toast queue | `src/notifications.rs` |
| `Command` enum + fuzzy-filter palette state/render (toolbox-scoped - see `Command::scope`) | `src/command_palette.rs` |
| Global, toolbox-agnostic chrome widgets (topbar/rail/status-bar composition, help overlay, spinner, empty-state, gauge); centred-overlay geometry for every popup is `widgets/popup.rs::centered_rect` (do not add a local copy) | `src/widgets/` |
| Byte-size/elapsed-time formatting, extension-breakdown aggregation | `src/format.rs` |

## Key Relationships
- `main.rs` builds one multi-thread tokio runtime, initializes the terminal via `ratatui::init()`, and runs one async loop: a spawned task forwards crossterm's `EventStream` + a 200ms tick into a single `mpsc::unbounded_channel::<AppEvent>`; the main loop drains it, calls `app::handle_event`, executes whatever `Effect`s it returned, and redraws via `widgets::shell::draw`.
- `handle_event` is the **only** place `AppState` (chrome/nav) or `SearchToolState` (the one real toolbox) is ever mutated. It never touches the terminal, the async runtime, the filesystem, or the OS — it returns `Effect` values instead, and `main.rs`'s `execute_effect` is the only code that spawns tasks, writes files, or shells out (`open`/`arboard`). This is deliberately narrower than either sibling head's own pattern: `app/`'s `AppState` is a flat `Copy` struct of Dioxus `Signal<T>` (reactive, auto-rerendering); `app-egui/`'s `SearchUiState` is an `Arc<Mutex<_>>` written by a background task and polled once per immediate-mode frame. Here there is exactly one synchronous mutation point, full stop — see `app.rs`'s own module doc comment.

## External Dependencies
| Dep | Used for | Note |
|---|---|---|
| `search-core` (path) | Search/matching/extraction/report | Same crate `cli/` also consumes; zero changes made to it by this crate |
| `ratatui` | Rendering | Pulls in `crossterm` transitively at a matching pinned version |
| `crossterm` (`event-stream` feature) | Terminal input as an async `Stream` (`EventStream`), merged with the tick timer via `tokio::select!` | |
| `tokio` (`rt-multi-thread`, `macros`, `time`, `sync`), `tokio-util` | Async runtime + `CancellationToken` | Same cancellation mechanism `cli/` also uses over `search-core`'s API |
| `futures-util` | `.next()` on crossterm's `EventStream` (a `futures_core::Stream`, not a tokio-native type) | |
| `open`, `arboard` | Opening a result / copying a path — same OS-integration calls both existing heads already use | |
| `serde`, `serde_json` | `settings-tui.json` persistence | |
| `tempfile`, `chrono` (dev-only) | Test fixtures; `chrono` named directly to construct `DateTime<Local>` values | |

## Design Rationale
| Decision | Why | Rejected |
|---|---|---|
| `AppState`/`SearchToolState` are plain (non-reactive) structs mutated only inside `handle_event`, which returns `Effect`s as data instead of executing them inline | Testability with no terminal/tokio runtime at all (`cargo test -p app-tui --lib` covers the whole decision layer), and a narrower state-mutation surface than either sibling head's pattern | A `Signal<T>`-per-field struct (no reactivity system exists in ratatui to drive it) or an `Arc<Mutex<_>>` polled per frame (works, but has two independent access patterns instead of one) |
| `lib.rs` + a thin `main.rs`, rather than one binary crate | Every module's tests — including `runner.rs`'s real `#[tokio::test]` search-execution tests — run via `cargo test -p app-tui --lib` with no terminal | Putting the event loop and search-runner logic directly in `main.rs`, untestable without actually launching the binary |
| `FocusState::default()` starts on `FocusArea::Rail`, not `Workspace(0)` | Search Files puts a text-entry field at workspace pane 0; defaulting keyboard focus straight into a text field would silently swallow the very first keypress (e.g. `q` typed to quit) as a character instead of a global shortcut — see `nav.rs`'s own doc comment on `FocusState::default` | Defaulting into the first workspace pane (broke 4 Phase-1 shortcut tests when tried) |
| Toolbox-local key routing (`toolboxes/search::handle_key`, see `src/toolboxes/search/AGENTS.md`) lives inside the toolbox module, called from `app::handle_key` before global bindings, rather than behind a `Toolbox` trait | Only one real toolbox exists in this phase; a trait abstraction is worth adding once a second toolbox needs the same treatment, not before | A generic `Toolbox` trait now |
| Settings form (where built) uses one flat field-index `match` rather than a generic field/widget descriptor framework | Same "don't abstract prematurely" reasoning — one toolbox, ~28 fields, a big but simple match is more legible than a generic form engine built for a consumer count of one | A reusable `FormField` widget abstraction |
| The Ctrl+P command palette (`command_palette.rs`) is toolbox-scoped via `Command::scope() -> Option<ToolId>` (`None` = always global, `Some(tool)` = only visible while that toolbox is active), checked in `CommandPalette::matches` alongside the existing text filter | The palette used to be one flat list - every Search-only command (`RunSearch`, `ToggleFastReSearchIndex`, ...) showed up while on the Bushing/Pressure Vessel/Preload Analysis tabs with no way to reach any of *their* own actions from the palette at all | A separate palette per toolbox (more state/rendering to keep in sync); an "active tool first" sort with no actual filtering (still clutters the list, just reorders it) |

## Public API
`lib.rs` exports `app`, `command_palette`, `format`, `library`, `modal`, `mouse`, `nav`, `notifications`, `paths`, `theme`, `toolboxes`, `widgets`; the binary (`main.rs`) is a thin loop over `app::handle_event`.

## Entry Points
| Task | Start Here |
|---|---|
| Add a new global keybinding or overlay | `src/app.rs` (`handle_key`/`handle_modal_key`) + `src/widgets/shell.rs` (render + help-overlay hint list) |
| Add a real tool behind the `Dupes`/`Rename`/`Logs` rail placeholder | `src/nav.rs` (`ToolId::enabled()`) + a new `src/toolboxes/<tool>/` module following `toolboxes/search/`'s (or `toolboxes/fastener_hole/`'s/`toolboxes/bushing/`'s/`toolboxes/pressure_vessel/`'s/`toolboxes/preload_analysis/`'s, if the tool has real engineering math) split (pure `model.rs`/`domain/`, a `mod.rs` owning state + key routing, a `*_view.rs`/`view.rs` for rendering) + wire the match arms in `widgets/shell.rs::draw_workspace` and `app::AppState::workspace_pane_count` |

## Contracts
- `handle_event` (and everything it calls transitively: `app::execute_command`, `toolboxes::search::handle_key`, `model::apply_progress`, etc.) must never touch the terminal, the tokio runtime, the filesystem, or the OS. Anything that needs to is expressed as an `Effect` variant and executed only in `main.rs::execute_effect`. Adding a new side-effecting action means adding an `Effect` variant, not calling out directly from inside the reducer.
- `main.rs` installs no custom panic hook — `ratatui::init()` already installs one that restores the terminal before any panic propagates, and must be called (not a raw `Terminal::new`) for that guarantee to hold.
- Every toolbox `Number` row enters edit mode via `Enter` (prefilled) or by typing a digit/`.`/`-` (fresh buffer), through `widgets/number_edit.rs`'s `number_char`/`handle_buffer_key`. Every edit buffer is a `number_edit::EditBuffer` (text + char-index cursor): `Left`/`Right`/`Home`/`End` move the cursor, typed characters insert at it, `Backspace` removes the char before it and `Delete` the one under it (neither clears the whole buffer); rows render it via `with_cursor()` (`_` at the cursor). Text-entry forms (material add forms, search settings) use the same type with an accept-all filter. A picker-backed `Number` row (Bushing's Bore Diameter/Friction/Bushing ID) must be excluded from type-to-edit in that toolbox's `handle_key`, not in `number_edit.rs`.

## Pitfalls
- **Disk space**: a bare `cargo build --workspace` hit a real `ENOSPC` (OCR/`burn-wgpu`-adjacent trees are heavy). **Always scope to `-p app-tui`** (`cargo check -p app-tui --bins --lib --tests`, `cargo test -p app-tui --lib`). `target/` is safe to remove and rebuild (see root Global Pitfalls).
- Ratatui's `Gauge::ratio` panics on a value outside `0.0..=1.0` — `widgets/gauge_row.rs`'s private `ratio()` helper clamps `percent` before dividing by 100 specifically because progress data arriving from a background task should never be trusted to already be in range; don't bypass it by calling `Gauge` directly elsewhere.
- Every render function taking a `Rect` must guard 0-sized areas explicitly (`empty_state::render`, `help::render_overlay`/`render_status_hints` do, each with a `TestBackend` small/zero-size test); ratatui's `Layout` does not guarantee panic-free behavior there.
- **Selectable lists must render through `widgets/scroll_list.rs`** (stateful `List`/`ListState`), never a bare `List::new(items)`: the stateless form always paints from item 0, so the selected row silently scrolls off-screen while `selected` keeps changing (this shipped in Results, Settings lists, extension picker, and the palette). Grep for `List::new(` before adding a list; render palette-style bordered lists via `Block::inner` + `scroll_list::render`.
- **Status-bar hints are contextual** (`shell.rs::contextual_hint`): `s Settings` only on Search/Run/Results pane, `Esc Back` only on Settings. The bar used to be a static 4-hint list and `s` was undiscoverable. New toolbox bindings that are non-obvious should extend that match. Covered by `tests/rendering.rs` "Status bar's contextual hint".
- The exact rendering fidelity of rounded borders / braille spinner glyphs / color depth on the actual shipping target (Windows Terminal/ConHost) has **not** been verified from this development environment (macOS) — `theme.rs`'s reduced-color fallback and `spinner.rs`'s ASCII fallback exist for this reason, but real verification on Windows is still outstanding.
- **Do not use `ratatui_image::picker::Picker::from_query_stdio`**: it spawns a thread blocking on raw stdin that is never cancelled if the terminal does not answer (tmux did not), racing crossterm's `EventStream` and swallowing keystrokes (upstream `ratatui-image` #202). `Picker::halfblocks()` is safe; re-verify upstream before trusting any stdio-querying constructor.
- **Every toolbox Results/readout pane must render through `widgets/scroll_paragraph.rs`** (word-wrap via `hint_panel::wrapped_line_count`, per-toolbox `results_scroll: u16` driven by `PageUp`/`PageDown`, a "N more below" row on overflow). A bare `Paragraph::new(Vec<Line>)` silently truncates wide lines and drops overflow; this shipped in all four solver toolboxes at once. Grep for `Paragraph::new(` over a `Vec<Line>` readout before adding a pane.

- **Crossterm's Windows Console backend flips `KeyCode::Char` case from Shift XOR Caps Lock, even for Ctrl-modified keys** (Unix always reports Ctrl+letter lowercase). Root cause of the nav-shortcut bug (`6270fed`, `search::is_plain_char`) and Ctrl+P/Ctrl+Q failing under Caps Lock. **Every single-letter binding must use `'x' | 'X'` alternation or `is_plain_char`**; a bare `KeyCode::Char('x')` is a latent Windows-only bug. All existing production bindings (bushing, pressure_vessel, preload_analysis `d`/`e`/`m`, extension picker `a`/`n`, PV material picker `n`) were fixed this way, each with a Caps-Lock regression test; tests that build `KeyEvent`s directly will not catch the bug unless an uppercase variant is added explicitly.

- **Debug log**: `src/debug_log.rs` writes `toolbench-debug.log` into the launch folder (cwd; falls back to the exe folder, then temp; `TOOLBENCH_DEBUG=0` disables) - environment banner, index stages with timings, failures aggregated by extension + error text, search narrowing counts, panics (frames only). **Privacy contract: never any file/folder name, path, search term or file content** (`log()` also redacts path-like text as a backstop; a test asserts a build+failure leaves none of them in the log). First thing to ask a Windows user for on any index/search report; palette command "Open debug log". Log counts, sizes, timings, extensions and OS error codes only.
- **Text-input rows must use `widgets::input_line::line`**, not a bare `Paragraph` of `label + text + "_"`: a long path pushed the cursor off the row (the reamer-import overflow bug). It keeps the tail + cursor visible and drops the label before squeezing the text.
- **The Windows exe icon comes from `build.rs` (`winresource`), host-Windows only.** Verified compile of the non-Windows path only; the embed itself must be checked on a CI-built `app-tui.exe`. See `docs/deployment-rust.md` "Windows .exe icon".

## Patterns
### Basic / Advanced form sections
Bushing Workbench, Lug Analysis, Preload Analysis and Eccentric Bushing show the rows a first run needs and put the expert rows behind a collapsed `FieldRow::AdvancedSection` (Lug: `MeshSection` / `mesh_open`; others: `advanced_open` on the model/UI state), rendered as `-- ▸ Advanced --`, toggled by Enter / Space / click like any row. The section row sits at the end of the basic rows and the advanced rows follow it in `field_rows`, so the mouse row mapping (`list_row_regions*`, built from `field_rows`) stays in sync with no extra code. Hidden rows still feed the solve; tests that look for an advanced row must open the section first. Toolbox block titles use `widgets::title::toolbox_title` (drops trailing key hints to fit the width) and the status bar drops the toolbox's own hints whole before `? Help` / `q Quit`. Results lead with a one-line verdict (Lug `PASS/REVIEW/FAIL`, Eccentric `HOLDS/SPINS/THIN WALL`, Preload preload + solver status, Bushing `REVIEW`/`OK`).

### Adding a real tool behind a placeholder rail slot (`Bushing`/`PressureVessel`)
1. Add a `src/toolboxes/<tool>/` module mirroring `toolboxes/search/`'s split: a pure `model.rs` (zero ratatui/crossterm imports), a `mod.rs` owning the tool's own state struct + toolbox-local key routing (returning `(bool, Vec<Effect>)` the same way `search::handle_key` does), and a `*_view.rs` for rendering.
2. In `src/nav.rs`: add the variant to `ToolId::enabled()`'s `matches!`.
3. In `src/app.rs`: add the new toolbox's state as an `AppState` field, extend `workspace_pane_count` for the new `ToolId`, and route its `handle_key` call the same way `ToolId::Search` is routed in `handle_key`.
4. In `src/widgets/shell.rs::draw_workspace`: render the new tool's view instead of falling through to the `empty_state`/"Coming soon" placeholder.
5. If it needs persisted settings, give it its own `settings-tui-<tool>.json` (or extend the existing file's top-level struct) with the same `#[serde(default)]`/`Option<T>` forward-compatibility discipline as `toolboxes/search/persistence.rs`.

## Boundaries

### Always
- Scope `cargo` invocations to `-p app-tui` in this crate (see Pitfalls — disk space).
- Route new side effects through the `Effect` enum (`app.rs`) and execute them only in `main.rs`/`runner.rs` — never call the filesystem, the OS, or spawn a task from inside `handle_event`.

### Never

### Verify First
- Current test count/coverage (`cargo test -p app-tui --lib` and `cargo test -p app-tui --test rendering`) before citing a specific number anywhere — it changes every phase.

## Downlinks
Per-toolbox nodes (read the one for the toolbox you are changing):

| Toolbox | Node |
|---|---|
| Search Files | `src/toolboxes/search/AGENTS.md` |
| Fastener Holes | `src/toolboxes/fastener_hole/AGENTS.md` |
| Bushing Workbench | `src/toolboxes/bushing/AGENTS.md` |
| Pressure Vessel Analyzer | `src/toolboxes/pressure_vessel/AGENTS.md` |
| Preload Analysis | `src/toolboxes/preload_analysis/AGENTS.md` |
| Eccentric Bushing | `src/toolboxes/eccentric_bushing/AGENTS.md` |
| Lug Analysis | `src/toolboxes/lug_analysis/AGENTS.md` |
| FEA Workbench | `src/toolboxes/fea_workbench/AGENTS.md` |
| Material Lookup | `src/toolboxes/material_lookup/AGENTS.md` |

Sibling nodes: `search-core/AGENTS.md`, `native-search/AGENTS.md`, `bushing-solver/AGENTS.md`, `fastened-joint-solver/AGENTS.md`, `lug-solver/AGENTS.md`. `pressure-vessel-solver` has no node.

## Navigation addendum — issue #12 GPU extension (2026-10-10)

User-authorized direct `wgpu` companion viewport: `src/gpu_viewer/` owns bounded scene conversion,
WGSL/deformation buffers, `winit` native window and verified free-vibration worker. `g` opens current
static/modal/buckling results; `t` computes a time history. Main executable dispatches
`--gpu-viewer <scene.json>` before ratatui initialization. `gpu-viewer` defaults on; terminal-only
builds use `--no-default-features`. Effects/process/window I/O stay in `main.rs`; reducers stay pure.
See `docs/issue-12-phase-18.md` for controls, supported physics, limits and validation evidence.
Earlier no-GPU descriptions above are historical.
