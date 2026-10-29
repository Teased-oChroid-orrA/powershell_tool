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
- New engineering workbenches must have a native-window dashboard launch. Owner preference (2026-10-28). Bushing Stackup is native-only, with engineering calculations in `bushing-solver/src/stackup.rs`; it has no terminal toolbox or terminal transfer.
- `handle_event` (and everything it calls transitively: `app::execute_command`, `toolboxes::search::handle_key`, `model::apply_progress`, etc.) must never touch the terminal, the tokio runtime, the filesystem, or the OS. Anything that needs to is expressed as an `Effect` variant and executed only in `main.rs::execute_effect`. Adding a new side-effecting action means adding an `Effect` variant, not calling out directly from inside the reducer.
- `main.rs` installs no custom panic hook — `ratatui::init()` already installs one that restores the terminal before any panic propagates, and must be called (not a raw `Terminal::new`) for that guarantee to hold.
- Every toolbox `Number` row enters edit mode via `Enter` (prefilled) or by typing a digit/`.`/`-` (fresh buffer), through `widgets/number_edit.rs`'s `number_char`/`handle_buffer_key`. Every edit buffer is a `number_edit::EditBuffer` (text + char-index cursor): `Left`/`Right`/`Home`/`End` move the cursor, typed characters insert at it, `Backspace` removes the char before it and `Delete` the one under it (neither clears the whole buffer); rows render it via `with_cursor()` (`_` at the cursor). Text-entry forms (material add forms, search settings) use the same type with an accept-all filter. A picker-backed `Number` row (Bushing's Bore Diameter/Friction/Bushing ID) must be excluded from type-to-edit in that toolbox's `handle_key`, not in `number_edit.rs`.

## Pitfalls
- **Failure guidance and STEP solids live in the native solid editor** (`src/gpu_viewer/editor/solid/guidance.rs`, `stepops.rs`): the
  card / status-bar fix come from `fea_problem::diagnose` (ADR-020; ranker "Failure fixes" in `Advisors`, same measured gate);
  STEP open, hole / boss / fillet need `--features occt` (prebuilt OpenCASCADE via `cadrum`; set `OCCT_ROOT` to an extracted
  tarball if its download fails). Without the feature those entries are disabled with the reason. A kernel edit renumbers
  `faceN`; `Problem::apply_solid_op` carries supports and loads across it by geometry (`solid::remap_faces`) and renames a lost
  one `removed:faceN`, which the diagnosis offers to retarget / drop. Findings that need a result (`diagnose_result`, e.g. an
  absurd displacement) are added to the same card, cached per result. A remedy row previews itself: `DockItem::row_with_preview`
  sends `(on, off)` actions on pointer enter / leave (`gui/mod.rs::run`), `SolidAction::PreviewFix` lights the surfaces
  `Edit::surfaces` names, presentation only.
- **Thermal studies** (ADR-021) ride the shared row model: `FieldRow::StudyKind`, the Thermal / Fixed temperatures / Heat loads
  sections, `Feature::{Thermal, HeatSupport, HeatLoad}` tree nodes, `BcKind::{FixedTemperature, HeatFlux, Convection}` in the surface
  menu. A result lists its fields with `Solved::fields()` / `default_field()`; never `Field::ALL`.
- **The terminal Workbench is manual by default** (`u` mesh, `r` solve, `a` automatic mode). Tests of the automatic behaviour use
  `auto_state()`.
- **Window tests drive real input**: `gpu_viewer/window.rs` tests click the viewport (`click`, `move_to`, `Input::Key`), including the
  STEP hole workflow under `--features occt`. Prefer them to calling session methods when the thing under test is a pointer or
  key path; `type_text` feeds egui text fields only, an editor prompt takes `Input::Key(Key::Character(..))`.
- **Linker error = check `df -h` first.** `target/debug/incremental` alone reached 14 GB and the build then failed with "No space
  left on device" behind a bare `clang: linker command failed`. Delete `target/debug/incremental` (a cache), use `CARGO_INCREMENTAL=0`
  for long test runs.
- **Disk space**: a bare `cargo build --workspace` hit a real `ENOSPC` (OCR/`burn-wgpu`-adjacent trees are heavy). **Always scope to `-p app-tui`** (`cargo check -p app-tui --bins --lib --tests`, `cargo test -p app-tui --lib`). `target/` is safe to remove and rebuild (see root Global Pitfalls).
- **The GPU render tests are `#[ignore]`d, so a plain `cargo test` never exercises the shaders or pipeline creation.** After any `wgpu`/shader change run `cargo test -p app-tui --lib --locked -- --ignored gpu_viewer` (needs a native adapter). The wgpu 0.19 -> 30 port (phase 26) only surfaced a real defect there: a depth bias on the `LineList` mesh pipeline is invalid in wgpu 30, so mesh lines get their depth offset in `shader.wgsl::vs_mesh`.- Ratatui's `Gauge::ratio` panics on a value outside `0.0..=1.0` — `widgets/gauge_row.rs`'s private `ratio()` helper clamps `percent` before dividing by 100 specifically because progress data arriving from a background task should never be trusted to already be in range; don't bypass it by calling `Gauge` directly elsewhere.
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
WGSL/deformation buffers, `winit` native window and verified free-vibration worker. (Superseded: Workbench `g`, `t` and
`w` all open the solid editor, which runs the static, modal, buckling and transient studies itself; see the Phase 23 section.) Main executable dispatches
`--gpu-viewer <scene.json>` before ratatui initialization. `gpu-viewer` defaults on; terminal-only
builds use `--no-default-features`. Effects/process/window I/O stay in `main.rs`; reducers stay pure.
See `docs/issue-12-phase-18.md` for controls, supported physics, limits and validation evidence.
Earlier no-GPU descriptions above are historical.

## Navigation addendum — native GPU workbench roadmap (2026-10-10)

`src/gpu_viewer/progress.rs` and `transport.rs`: generation stages/step progress, ready handshake
and packed companion streams. Versioned native sessions carry all computed modes; native Tab/[ ]
selects results and M/C/D controls mesh/contour/deformation. Interactive authoring and contextual
ML are newly planned, not shipped: `docs/issue-12-phase-19.md`; current status/handoff remains
`docs/issue-12-claude-handoff.md`.

## Native window: CAD chrome, viewer and editors (issue #12, Phases 20-22)

One native window (`app-tui --gpu-viewer <scene> | --gpu-editor [file] | --gpu-solid [file]`) is the main frontend: an
egui chrome (design B "Canvas" with a thin step strip, ADR-017) around a confined 3D viewport. Everything is a pure,
unit-tested core; `window.rs` maps winit events to egui and to `Input`, runs the egui frame and draws the scene.

- **Chrome (`gpu_viewer/gui/`, feature `gpu-viewer`)**: `Gui::run(RawInput, &GuiInput) -> (FullOutput, Vec<Outcome>)` builds a
  thin top bar (Menu button holding every menu, model title, the **step strip** = the editor's ribbon tabs with a
  done / to-do dot from `Authoring::step_state` and a ring on the step usually taken next, Search commands, theme), a floating
  **tool pill** over the model showing the selected step's ribbon groups, the **feature tree** on the left (foldable to a chip;
  right click on a row = the menu of its entity), the **inspector** (below), a status bar (hover text of the last hovered
  control, else error, else the editor status; "Solving... Ns"), the **command palette** (Ctrl/Cmd+K, F10;
  `menu::palette_entries` fuzzy over every menu, disabled entries stay findable and explain themselves), right-click /
  clicked-entity lists (`Gui::open_context`), the **start card** (`Authoring::start_options`, an empty model offers New part /
  templates / Open) and the selection box. Light and dark palettes follow the system theme (`gui/theme.rs`).
  `Gui::viewport_px` is what the panels leave, less the inspector's width while its cards show: the scene is drawn there
  (`DisplayOptions::viewport`, clear colour from the theme) and the camera aspect and picking use it. The viewport is
  deliberately **not** an egui `CentralPanel` (that would use the space up and egui would then count the pointer over the
  model as over the interface).
- **Inspector and adaptive layout (ADR-018)**: no permanent properties dock. `Authoring::inspector() -> Vec<Card>`
  (`Relevance::{Primary, Context, Info}`, optional `steps`); `Gui::inspector_plan` filters by step, orders by relevance then
  the learned order, and returns `None` when there is nothing to show or the person closed it (Pin overrides). The solid
  inspector (`solid/inspect.rs`) lists only the focused feature's **value** rows (`EditKind::Number/Component/Text`); choice and
  action rows are `Authoring::tree_menu(node)` (right click on a tree row selects it and opens the menu next frame;
  `node_feature` maps tree ids to `Feature` and a test checks every row). Do not add a row that the tree, a menu or the title
  already shows. `editor/adapt.rs` (`Adapt`) learns tool order per group, folded cards, tree open/rail and the usual next step
  from `offer` / `used` counts, persisted in `<app data>/fea-editor/layout/layout.json`; it is presentation only (no gate),
  off with Window > Adaptive layout or `FEA_UI_ADAPT=off`, reset and explained from the Window menu.
- **Model labels**: `Authoring::overlay(cam, viewport size) -> Vec<Overlay>` places text on the model for the current camera (sketch mode:
  boxed dimension values and constraint glyphs H V // _|_ = T o M F, `solid/sketching.rs::sketch_overlay`, stacked so none overprint);
  `Gui::model_labels` draws them clipped to the viewport. Computed in `Viewer::run_gui` from the viewport of the last frame.
- **Pure data the chrome renders**: `menu.rs` (`Entry`, `Item`, `Menu`, `Outcome`, palette search), `menus.rs` (View / Window /
  Help + composition in CAD order), `ui.rs` (`TreeNode`, `RibbonTab`, `DockItem::{Heading, Line, Field, Gap, Legend}`; typed
  `Field` rows carry label and value separately), `results.rs` (loaded results in the tree, the result section, `GuiInput`),
  `text.rs` (wrap, number format, contour colour map).
- **Input routing**: egui sees every winit event first (`egui_winit::State`); a press is a model gesture only when
  `Gui::wants_pointer()` is false (not over a panel, the pill or a popup); a press egui took also swallows its release
  (`egui_press`); the wheel zooms only over the free viewport; while the palette's text field is open every key is egui's;
  while a number is being typed (`Authoring::prompting`) every key belongs to the prompt, even Ctrl+K.
- **Rules stay single-sourced**: ribbon buttons, menu entries and context menus are `menu::Entry`s whose enabled state and
  reason come from the model's own rule engine (`editor::caps::availability` for members, `SolidSession::*_availability`
  for solids). A disabled control shows its reason (hover -> status bar, click -> status text).
- **`editor/authoring.rs` (`Authoring` trait)**: what the window needs from an editor (act/key/click/select_at/scene/poll,
  tree, properties, ribbon, menus, context menu, status). Implemented by the member `Session` (`editor/session/ui.rs`) and
  the solid `SolidSession` (`editor/solid.rs`, `editor/solid/ui.rs`). `EditorAction` is the shared action enum
  (`Solid(SolidAction)` for solids). Add a new model kind by implementing the trait, nothing else in the window changes.
- **Solid editor**: edits `fea_problem::Problem` with the *same* rows, labels, hints and JSON as the terminal Workbench
  (`fea_workbench::model`: `field_rows`, `row_label/value`, `apply_row_action`, `commit_edit`, `toggle_source`, `apply_material`,
  `move_selection`). Mesh preview and solve run on `editor/worker.rs` (one job at a time, stale/cancelled answers dropped;
  bushed contact solves are explicit only). Surfaces (`bottom`/`right`/..., `edgeN`, `holeN`, `start`/`end`, imported names)
  are picked in the viewport (`pick_surface`) or the tree; a click assigns the picked name to the focused support/load edge.
  Preview colours: 0 body, 1 support, 2 load, 3 selected/edited. Results use `SceneSource::from_solved` (all 11 fields).
  Workbench key `w` writes `native-solid.json` and opens it; closing reads it back (`NativeEditorFinished`).
- **One window** (superseded the separate result viewer, ADR-014): Workbench `g`/`t`/`w` open the solid editor; its
  Simulation menu runs the studies (`editor/solid/study.rs`). `--gpu-viewer <file>` still opens a saved result project.
  `ViewerProject::append` merges scenes; the feature tree's Results folder lists them and their fields.
- Keys: viewer `Tab`/`[ ]` result, `F`/Shift+`F` field, `M C D`, `H`, Space/arrows/`+ -`/`R`, Ctrl/Cmd+K or F10 palette, right-click context
  menu. Member editor: tools `1`-`6`, `P G Z`, `E` edit, Enter solve, Esc cancel, `B`/`T`, `K`, `V`, Ctrl/Cmd+Z/Shift+Z/S/O/N.
  Solid editor: Up/Down rows, Enter edits/activates, Left/Right cycles, digits start an edit, Del removes the entry,
  Ctrl/Cmd+Enter solves, `V` results/model, Esc cancels/clears.
- **Advisor loop (opt-in, `editor/learning.rs`, Tools menu of the member editor)**: *Record* (off by default) appends
  content-free `Recorded{session, Context, allowed, taken}` lines to `<app data>/fea-editor/advisor/events.jsonl`; *Train
  from recordings* (explicit) splits sessions deterministically (~20 % held out), fits a `CountModel`, saves `model.json` with
  a `Report` (rules vs learned top-1/top-3, paired sign test); *Use learned ranking* (explicit) is refused unless
  `Verdict::Eligible` (>= 20 sessions, >= 100 held-out events, z >= 2 top-1 gain, no top-3 regression). The advisor may only
  reorder actions the rules allow; `FEA_EDITOR_ADVISOR=off` wins over everything; `FEA_EDITOR_MODEL` loads a bare model as a
  developer override. No default-on path without a measured real-data report; no real data exists yet.
- **Testing without a window**: `window.rs` tests drive `Viewer::input(Input)` and queue egui events (`test_events`), running the
  chrome twice per frame (the second pass has the measured sizes) and clicking controls by label through
  `Gui::find_target` (step chips are "<name> tab"). They cover tool and step clicks, viewport clicks, right-click menu, the
  palette, typing, wheel, drag/orbit, box select, solid mesh -> solve -> results, benches, the start card. GPU tests
  (`-- --ignored gpu_viewer`, needs an adapter; a plain `cargo test` never runs them) cover viewport confinement and egui
  painting; `editor_snapshot` / `solid_snapshot` / `sketch_snapshot` / `hud_snapshot` dump raw RGBA of the real chrome in both
  themes for visual review (`FEA_SNAPSHOT_EDITOR_OUT=dir`; PNG via a small python zlib script, then Read it). Real-mouse
  behaviour on a desktop is still verified by hand only; Windows is hosted-CI only.
- Pitfalls: `FullOutput` panics when dropped with unapplied texture deltas (`paint_egui` clears them; headless tests call
  `drop_without_applying_deltas`); egui's own `wants_pointer_input` is false while a button is down, so the window tracks
  `egui_press` itself; the Authoring `properties()` rows are plain data, so keep label and value in a `DockItem::Field`
  rather than padding a string; docks are resizable panels (no more "vanish when narrow").
- Terminal-only builds (`--no-default-features`) still compile `controller`, `menu`, `menus`, `results`, `text`, `ui`, `editor`, `scene`, `transport`.
- Number rows accept `e` / `E` / `+` so `2.9e7` can be typed; text rows (mesh path) accept any graphic character.
- `Source` toggles keep the sketch in `saved_geometry` so going back loses nothing; the imported file's text lives in `import` and is matched to the path.
- Templates replace the whole problem (`j` saves first); `set_problem` keeps the view flags (auto-solve, field, mesh, deform).
- Real-terminal check done with `tmux` (170 x 50): true-colour half blocks render; a `TestBackend` test (`tests/rendering.rs`) guards that a contour paints many colours and that no size panics.
- Mouse in the native window: left drag orbits (or drags a sketch point / box-selects with Shift), **middle drag or right drag pans** (`PanGesture`, `Camera::pan`, an NDC shift added after the projection in `shader.wgsl` and `Camera::project`/`unproject_to_plane`), the wheel zooms **about the cursor** (`Viewer::zoom_at` keeps the model point under the cursor fixed; zoom range 0.1..400). A right click (release without travel) opens the context menu **on release**: opening it on the press let the press egui was still handling close it at once (`floating_list` closes on any press outside its rect), which showed as a flashing menu. The test `a_right_click_the_egui_also_sees_leaves_the_menu_open_...` feeds egui the secondary press, which the older right-click test did not.
- Sketch lines keep a constant **pixel** width: `Authoring::set_view` hands the camera and viewport to the session every frame, `SolidSession::set_sketch_pixel` rebuilds the sketch scene when the pixel size changes by more than 3 %, and `sketch_scene` sizes tubes in pixels. World-space tubes scaled with the zoom, so a thin profile never separated into two lines.
- Extrude / Revolve / Sweep are on the Features tab (Solid group), not on the Sketch tab; they need the sketch finished first (`feature_availability`).

## Native window: benches, palette, selection (Phase 23, 2026-10-10)

Records: `docs/issue-12-phase-23.md`, `docs/adr/ADR-014-native-cad-process-model.md`.
- **Benches** (`gpu_viewer/bench.rs`): `Members`, `Static`, `Modal`, `Buckling`, `Transient`. `Window > Bench:` switches; solid
  benches share one `SolidSession` (`Authoring::switch_bench`), crossing to/from members loads the other file beside the
  current one (`editor::open`, `bench_file`) and is refused while dirty. A bench contributes only its own Simulation
  menu/ribbon group. New bench = `Bench` variant + `Authoring` impl or session mode.
- **Studies** in the solid editor: `SolidAction::RunStudy(kind)`, `Study` valid for one `Problem` (stale => hidden), `[`/`]`
  cycle modes, scenes from `SceneSource::from_dynamic` / `transient::run`. Hole tool: `solid/sketch.rs` (`SolidTool`).
- **Selection-driven editing**: member Support/Load tools set `EditorState::popup`; `Authoring::take_popup` returns the
  applicable list (presets: `SupportPreset`); the window opens it at the click. Solid surface click does the same
  (`take_popup_flag`). Shift-click toggles (`click_with`), Shift-drag is a box (`Pointer::press_as`, `box_select`,
  `camera::pick_box`, `solid::surfaces_in_box`); groups delete/move as ONE undo step (`EditorState::commit_all`).
  Solid 3D faces are picked front-most per pixel (`depth_at`), not by face-average depth.
- **Palette**: Ctrl/Cmd+K / F10 open the command palette over every `Entry` incl. disabled-with-reason (`menu::palette_entries`,
  `fuzzy`); the Alt+letter mnemonics and open-menu letters of the bitmap chrome are gone (the Menu button is mouse-driven).
- **Pick radius** scales with the viewport (`camera::pick_radius`); working-plane offset is a property row
  (`PropertyKind::PlaneOffset`). Closing with unsaved edits warns once (`request_quit`).
- **Advisor** is per bench (`learning::Store::for_bench`); only Members has rules, other benches list the menu disabled with
  the reason. Do not share recordings between benches.

## Native window: parametric sketch, feature history, recommendations (Phase 24, 2026-10-10)

Records: `docs/issue-12-phase-24.md`, `docs/adr/ADR-015-parametric-sketch-history.md`. The model lives in `fea-problem`
(`sketch/`, `history.rs`, `advice.rs`); this crate only drives it.
- **Pure core** `gpu_viewer/editor/sketcher/` (`SketchEdit`): tools (select, line, polyline, rectangle, circle, arc,
  dimension), snapping (point > on-entity > horizontal/vertical alignment > grid), rubber-band `preview`, auto-constraints,
  dimensions (`set_dimension` rolls back a conflicting value), `drag`, `fix_selected`, `selection_constraint` (the single
  source for a constraint button's enabled state *and* applying it). Every mutation goes through `settle`: re-solve, and
  restore the backup when the result is `Conflicting`. Positions are sketch-plane coordinates; no pixels, no I/O.
- **Glue** `gpu_viewer/editor/solid/sketching.rs`: `Sketching` (open sketch feature, view frame, grid), pixel -> plane via
  `Camera::unproject_to_plane(.., 2, 0)`, undo (`SolidSession::edit` clone-diff; a point drag is ONE undo step through
  `drag_before`), the sketch scene (`editor::scene::Builder` tubes: blue = under-defined, teal = fully defined, red =
  selected, yellow = preview/hover), property / ribbon / menu / tree content, `typing` for dimensions and feature
  parameters (the first key typed replaces the suggested value). The preview mesher is paused while sketching
  (`maybe_start_preview`). `Finish sketch` = `try_edit` -> `Problem::apply_history`; failure keeps the sketch open and
  says why. Selected sketch entities become the selected *surfaces* (`Sketch::surface_names`), so the ordinary
  Support/Load actions act on sketch entities.
- **Window seam**: `Authoring::{hover, grab, drag, release_grab, take_view}` (defaults no-op). `Viewer::grabbing` stops a
  press on a draggable sketch point from orbiting; `take_view` is polled in `sync_editor_scene`.
- **Rows**: a model with a `history` has NO geometry rows (`model::field_rows`) and no Outline/Hole tree nodes and the Hole
  tool is unavailable: editing the geometry row by row would be silently undone by the next rebuild. Edit the history.
  The terminal Workbench shows the same rows (an empty Geometry section for such a model).
- **Recommendations / learning**: `fea_problem::advice` (rules, shown in the properties panel, applied through
  `SolidAction::Advice(i)` which recomputes the list: a stale index is refused), `editor/rank.rs` (generic gated ranker:
  offered set from the rules, learned order only after the measured gate, recording off by default and content-free,
  files `<advisor>/static/{constraints,boundary}*`), `solid/advising.rs` (situations, popup ordering, Advisor menu).
  Unit tests pass `cfg!(test)` -> no app-data store; `Advisors::in_dir` is the test seam.
- Pitfalls: sketch curves are tube geometry (no line primitive, no in-scene text), so constraint glyphs and dimension
  annotations are listed in the panel, not drawn in the viewport; `Sketch` entity ids are never reused (history tree and
  `Edit(id)` rely on it); `SolidAction` is `Copy`, keep new payloads `Copy`.

### Phase 25 (ADR-016, `docs/issue-12-phase-25.md`)

- **Nothing meshes on its own**: `SolidAction::GenerateMesh` sets `mesh_requested`; `poll` meshes only then. States are
  "not generated" / "out of date" (`preview_summary`). Tests and snapshots that need a mesh dispatch `GenerateMesh` first
  (`tests.rs::meshed`). While no mesh is current `outline_scene` (sketching.rs) draws the sketches / outline.
- **Model lifecycle** `solid/models.rs`: `NewPart`, `OpenTemplate(i)`, `OpenFile` (path prompt: `.json` problem or other =
  mesh), `ClearModel`; all refused while sketching/solving and each is one undoable edit via `start_model`. `new_sketch` on
  a model without history is refused (never a silent convert). `remove_feature` always succeeds; an unbuilt model fails
  `validate_geometry`, which mesh/solve/hints use.
- **Mesh density** `solid/refining.rs`: rules in `fea_problem::advice::refinement_regions`; zones are `MeshSpec::refine`;
  rings are `refine_rings` -> `Builder` tubes added to the outline scene and appended to the preview scene (the mesh
  scene's positions stay first, so `pick_positions` never reaches a ring); ring scalar codes sit BETWEEN the state codes.
- **Learning** `solid/corpus.rs` (simulated prior, deterministic, policies written out), `editor/rank.rs` (weighted
  `Model`, `Evidence::{Real, Simulated}`, `train(recorded, corpus)`, auto refit in `Ranker::record`). Three rankers:
  constraints, boundary, refinement (`Advisors`). Any change to a policy needs the coverage tests in `corpus_tests.rs`.
  `do_advisor_op` runs each op over all three rankers (`each!`).
- Pitfall: `rank::Ranker::record` takes `&mut self` and returns the refit summary; ignore it with `let _` only where no
  panel shows `last_fit`.

## Native window: display styles, backgrounds, un-meshed body (2026-10-14)

- **Display style** (`gpu_viewer/display.rs`, `ViewState::style`, `Command::SetStyle`): Shaded, Shaded with edges, Hidden lines visible
  (HLV), Hidden lines removed (HLR), **Wireframe (the default)**. Edges are the model's *feature edges* (`Scene::feature_edges`: boundary
  edges plus creases over 35 degrees, so a flat face is its outline whatever its mesh), never the mesh's triangles; the mesh is the
  separate `Mesh edges` overlay (`show_mesh`) drawn over the body. Render passes per style (`Renderer::draw_display`): background
  gradient pass, then the surface (colour for Shaded*, depth-only `depth_pipeline` for HLR/HLV, none for Wireframe), HLV's dimmed hidden
  edges (depth `Always`), the visible edges (`LessEqual`, pulled toward the camera in `shader.wgsl::vs_mesh`), the mesh overlay. A result
  keeps its contour: `ViewState::style_for` turns a style without a surface into Shaded with edges while a result is shown, and the
  View menu disables those styles with the reason. Lines are 1 px (wgpu `LineList`); thicker edges need quads.
- **Background** (`display::Background`, View menu presets + `Custom colours...` panel `Gui::background_picker`): theme, black, white,
  grey, grey-to-white and black-to-grey gradients, custom solid or gradient. Style and background persist in
  `<app data>/fea-editor/display.json` (`DisplaySettings`; only `window::launch` attaches it, tests never touch the user's file).
  Edge and mesh line colours follow the background's luminance (`display::line_colors`).
- **Un-meshed body**: a 3D sketch-based model (extrude / revolve / sweep) builds `fea_problem::solve::body_preview` on its own worker
  (`SolidSession::body`, keyed by `body_key`: geometry only) so the solid is visible before *Generate mesh*; the analysis mesh then
  replaces it (`shown_mesh`). Picking and box select work on the body. A failing body is remembered (`body_failed`) and not retried
  every frame. It is a coarse mesh drawn through the same scene path, so `Authoring::shows_body_only` suppresses the mesh overlay
  while it is shown: the mesh lines appear (on top of the body) only once the real mesh exists.
- **Context menu** (`menus::context`) offers what the selection can do: entries that cannot run are dropped (the menu bar and the palette
  keep them and say why), Contour/Deformation appear only for a result, and the list is as wide as its widest entry
  (`Gui::list_width`; `ui.available_width()` inside an `Area` made it screen-wide). Place a new floating panel away from the tool pill (top)
  and the inspector (right).
- Render checks: `-- --ignored display_styles_paint_what_cad_says`, and the RGBA dumps `body_styles_snapshot` (`FEA_SNAPSHOT_BODY_OUT`)
  and `context_menu_snapshot` (`FEA_SNAPSHOT_MENU_OUT`) for a visual review.
- **Surface menu** (`SolidSession::surface_menu`, one list for a face's right-click, a tree row and the click popup): the boundary conditions
  that fit the selected face(s) (ranked), then what is already applied there (edit / remove). Adding a pressure or pin load opens its
  magnitude prompt at once with `Prompt::fresh` (the first key typed replaces the placeholder). Styles without a surface still fill
  state-coloured faces (`fs_highlight`, geometry view only). The background pass covers the whole window, not just the viewport: a
  gradient that stopped at the viewport rectangle left a darker strip behind the right-hand dock.
- **Value cards and load cases** (`solid/cards.rs`, `ui::ValueCard`, `Authoring::value_card/card_field`, `Gui::value_card`): a face menu
  entry that needs numbers (`Hold directions...`, `Pressure...`, `Force...`, `Traction...`, `Pin load...`, `Gravity / body force...`, a
  case's switches and factors, and the *edit* forms of a support or load) opens a card beside the menu; nothing is written until
  `Apply` (one undo step), `Cancel`/Esc leaves the model alone, Enter applies, a bad number keeps the card open with the reason. The first
  field opens focused with its text selected, so the first keys replace the suggestion. `Fixed` and `Slides on the face` stay
  immediate (`BoundaryKind`), so the advisor's recording is unchanged (`record_boundary_choice`). A solid has no rotations: the
  directions card has x, y, z only (and a prescribed displacement per held direction). `fea_problem::cases` holds load cases:
  `Problem::cases` / `active_case`, entries are included at factor 1 unless a case says otherwise, `Problem::resolved()` is what a
  solve sees (`solve` and `dynamics::run` resolve it and keep the full problem in the result, so stale checks and saved files still
  see the cases). Removing a support or load must go through `Problem::remove_support/remove_load` so cases keep pointing at the same
  entries. The terminal Workbench has no case rows; it still solves the active case.
- **Analysis kinds, live motion, glyphs, library, forecasts (2026-10-14)**: the Simulation tab/menu lists every analysis in one click
  (`analysis_entries`: the benches Static / Modal / Buckling / Transient via `UiAction::SwitchBench`, and `SolidAction::SetStudy`
  for Structural / Thermal / Thermal stress). A static result swings between undeformed and deformed (`ViewState::swing`,
  `DisplayOptions::swing`, View > Animate deformation); mode shapes and the transient move by themselves. The viewer draws the
  deformation exaggerated (`Scene` gain = 8 % of the part at the peak): the result card says "drawn xN the true size" and View >
  True scale (x1) shows the real shape. Supports and loads are glyphs over the model (`solid/glyphs.rs`, `OverlayKind::{Load, Support}`,
  painted by `Gui::model_labels`): arrows along the load with the magnitude, a hatched triangle on each supported face with the held
  directions; they follow the camera and the active case and are not drawn while sketching, in a study view, or without a mesh. The
  material library is a value card (`CardForm::Materials`, search + group + sort over `material_lookup::model`); applying a material
  also sets its mass density (`model::mass_density`: handbook weight density, or the curated table's, over g = 386.089), because
  dynamics and gravity need it and the old material's density was silently kept. Results export from File (report, `.vtu`, `.csv`,
  beside the model file). Checks before a solve: `fea_problem::diagnose` forecasts the displacement from the loads, supports and section
  (`predicted_displacement`, 5 % of the part's length, a bar/cantilever estimate for sketch geometry) and `diagnose_mesh` counts the
  rigid-body motions the supports leave free on the shown mesh (`free_rigid_motions`); a free motion blocks Solve with the reason
  (`predicted_blocker`) and both appear as the primary "Check before solving" card. These are deterministic physics checks; the learned
  ranker only reorders their remedies.
- **Legend, geometry view, units, conversions (2026-10-14)**: a result, a mode shape or a transient shows a legend bottom right of the model (`Gui::contour_legend`: field name and unit over a colour bar with five values; click the name, or F, for the next field; a mode shape has the same 13 fields as a static result, `SceneSource::from_dynamic` + `DynSolved::mode_fields`); the model itself shows a colour key instead. `Authoring::geometry_view()` is true only for the model (preview or body scene): only then may a wireframe / hidden-line style be used, the surface lit and the state faces highlighted (`DisplayOptions::shade`); everything else (a result, a mode, a transient, sketch tubes) is drawn shaded with edges whatever style was chosen (`ViewState::style_for(scene, geometry)`, the View menu greys the others with the reason). A folded tree's chip and the tool pill never overlap (`Gui::chip_rect`, the pill starts right of the chip). Support / load glyphs are drawn in a Background layer under the docks and cards, clamped into the viewport, and glyphs on one spot merge (supports) or fan out (loads). The Units card (`CardForm::Units`, tree row "Units: ...") converts or declares the unit system; cards, the legend and the material library (`model::apply_material` uses `Problem::units`) follow it. A support or load changes kind from its face menu or tree row (`SolidAction::ConvertLoad` / `ConvertSupport`; point loads and supports use the picked face, else the nearest). The before-solve findings include the units check, and the forecast for imported meshes (from the shown mesh) and STEP solids (from the tessellation).
- **Colour key (2026-10-14)**: the model's state colours (body, support, load, selected) are the user's: `ViewState::key_colours`, `Command::{SetKeyColour, ResetKeyColours}`, persisted in `display.json` (`DisplaySettings::key_colours`, an older file reads as the contour-map defaults). The shader blends `Params.key` between neighbouring state codes exactly as the contour map does (`key_colour`, flag bit 2 of `frames.w`, set only for a geometry view), the highlight pass and the support / load glyphs use them too. The key and the result legend are movable egui areas (kept inside the viewport); their text must not be selectable (`selectable_labels = false`) or it takes the drag that moves them. Each swatch is `color_edit_button_srgb`: click it for the picker. Their positions persist too: `ViewState::panel_pos` (`[key, legend]`, fractions of the viewport, `Command::SetPanelPos` on `drag_stopped`, `DisplaySettings::panel_pos` with `#[serde(default)]`, `display::sanitize_panel_pos` clamps/drops non-finite). `default_pos` only applies to an area egui has no state for, so a store attached after a frame was drawn (tests) needs `memory_mut(|m| m.reset_areas())`; `launch` attaches before the first frame.
- **Dashboard (2026-10-14)**: Bare `app-tui.exe` opens the Toolbench start window on Windows; `--gpu-dashboard` (alias `--gui`) opens it on every platform, and `--tui` retains terminal launch on Windows. The window has one tile per workbench (`dashboard::WORKBENCHES`), a filter box and the recent files (`dashboard::Recents`, `<app data>/fea-editor/recent.json`, newest first, 12 kept, one per workbench+file). Pure data in `gpu_viewer/dashboard.rs`, drawing in `gui/dashboard.rs` (`Gui::run_dashboard` returns `DashEvent::Open`), `Viewer::dashboard` swaps the whole chrome for it (no model, no scene drawn). Process model is ADR-014's: a **window** workbench (`Opens::Window(EditorKind)`) is its own `app-tui --gpu-solid|--gpu-editor|--gpu-search|... [file]` child; a **terminal** workbench is `app-tui --toolbox <slug>` (`ToolId::slug`, read by `main::toolbox_arg`) started in a new terminal (`dashboard::terminal_command`: `cmd /C start` on Windows, `x-terminal-emulator -e` on Linux, a temp `.command` script via `open` on macOS). The dashboard only records what it started; it shows no state of a model it cannot see. Adding a workbench = one `WORKBENCHES` row (its `id` is the toolbox slug for terminal ones; a test checks it).
- **Search workbench window (2026-10-25)**: `app-tui --gpu-search [folder]` (`EditorKind::Search`, no default file: it keeps the saved search folder; the dashboard's Search tile now opens it, `Opens::Window`). Node: `gpu_viewer/editor/search/AGENTS.md`. First **page bench**: a bench without a 3D model returns `Authoring::document()` (pure data in `ui.rs`: `Document`, `DocInput`, `DocTable`, `DocDetail`, `DocPanel`, `DocModal`, `DocHelp`), drawn by `gui/document.rs` in the free area; `Authoring::has_document()` makes the chrome drop the tool pill, step strip, inspector and the viewer's View tab / menu, and makes the window skip the scene and model gestures (`Viewer::page_mode`). Edits come back as `Outcome::DocField`. The terminal Search toolbox is unchanged in behaviour; its event reducer and effect executor moved into the library (`toolboxes::search::apply_event`, `toolboxes::search::exec::execute`) and both heads call them.
- **Fastener Hole workbench window (2026-10-25)**: `app-tui --gpu-fastener [file]` (`EditorKind::FastenerHole`, default file `fastener-hole-case.json`; the dashboard's Fastener Hole tile now opens it, `Opens::Window`; `app-tui --toolbox fastener-hole` still opens the terminal toolbox). Node: `gpu_viewer/editor/fastener/AGENTS.md`. A model bench over `FastenerHoleState` with a to-scale three-quarter section of the nominal hole; the toolbox is unchanged except a few items made `pub(crate)` / `pub` for the window (`FastenerHoleState::toggle_selected`, `view::{row_label, display_value}`, `FastenerHoleModel::set_toleranced`). The shared snapshot helper `render.rs::draw` now lights the model in the colour key's colours and honours the view's display style, as the window does (`shade`, `key_colours`, `style_for`); snapshots taken before drew it unlit in the contour map.
- **Bushing workbench window (2026-10-14)**: `app-tui --gpu-bushing [file]` (`EditorKind::Bushing`, default file `bushing-case.json`, opened from the dashboard's Bushing tile) is the terminal Bushing toolbox in the native window, with the housing and the bushing drawn and solved by the FEA kernel. Node: `gpu_viewer/editor/bushing/AGENTS.md`. The terminal toolbox is untouched and still the owner of the inputs, solver bridge, checks, fixes and edge check; the window reuses `BushingState`, and a few of its items became `pub(crate)` for that (`activate_selected`, the pickers' `visible` lists, `view::{row_text, display_value}`, `BushingModel::set_number`, `edge_check::table_rows`). **Eccentric Bushing window (2026-10-14)**: `app-tui --gpu-eccentric [file]` (`EditorKind::Eccentric`, default `eccentric-case.json`, the dashboard's Eccentric Bushing tile): over `EccentricState`, with the bushing inputs read from a Bushing case file (`EccentricFile::bushing_case`) and the solved housing, bushing and pin drawn from the solver's `.vtu` fields. Node: `gpu_viewer/editor/eccentric/AGENTS.md`. Fit results (here and in the Bushing window) open at true size (`Scene::gain = 1`). **Contact between bodies in the FEA workbench (2026-10-14)**: `Problem::contacts` (see `fea-problem/AGENTS.md`) has rows in both FEA front ends (`FieldRow::{ContactA, ContactB, ContactFriction, ContactRemove, ContactAdd}`, a "Contact" section that appears for a plane sketch with more than one region), a tree folder and `Feature::Contact` in the window, an "Add contact" ribbon button and Insert menu entry (`model::new_contact` picks the facing surfaces of the first two bodies), focused pairs light both surfaces, and the result card, the terminal results and the report list what each pair transmitted. The bushing's fit contact was already there; this is the general pair. **Hand-off with the FEA workbench (2026-10-14)**: `gpu_viewer/editor/handoff.rs`. A Bushing or Eccentric case opens in the FEA workbench (ribbon "Edit in the FEA workbench"; the case is saved first) as its derived `Problem` in `<case>.fea.json` (an FEA file that differs is kept as `.previous.json`; a window that already has the FEA file open is left alone), and the FEA workbench sends the edited model back (Model ribbon group "Send": To Bushing / To Eccentric, kept short because the tool pill must fit a narrow window; Tools menu: "Send to the Bushing workbench" / "Send to the Eccentric Bushing workbench"). Files are the interface: `<fea>.link.json` says which case files belong to an FEA file (default names `<stem>.bushing.json` / `<stem>.eccentric.json`), and a window touches `<file>.open` every 2 s while it has a linked file open (`is_open`: fresher than 6 s), so a sender updates the file and opens a window only when none has it. **The case takes only what changed** between the model its inputs derive and the model sent (`bushing_from_problem`, `eccentric_from_problem`): bore, bushing ID, interference, friction, materials (by catalog name; one the catalog lacks is reported, never guessed), thickness, pin load, edge distance, width, and for the eccentric case the offset, load angle (from the offset line) and boss. Tolerances, countersinks, minimum wall and pin data are not in the FEA model and stay. A window whose case file changes on disk takes it in as one undo step when it has no unsaved edits, otherwise says so; the eccentric window also reloads its Bushing case. Only the first hole, bushing and bearing load are read; a model that is not a bushing in a housing is refused with the reason (`send_availability`). New seams in the window: `Authoring::benches()` (a workbench that is one study offers no benches) and `Authoring::key_labels()` (what the colour key's four colours mean), `SolidSession::set_derived` (take a problem derived from elsewhere, no undo or dirty mark) and `SolidSession::highlight_bushings`.

## Bushing Stackup native workbench

`--gpu-bushing-stackup` opens the native-only page. It has no terminal toolbox or cross-routing. Pure engineering types and calculation live in `bushing-solver/src/stackup.rs`; native session, cached result revisions and typed transfer live in `gpu_viewer/editor/stackup*.rs`. Coordinate and wall definitions are recorded in `docs/bushing-stackup.md`.
