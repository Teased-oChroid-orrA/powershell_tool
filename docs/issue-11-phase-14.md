# Issue #11 Phase 14: begin the egui migration (app-egui)

## Why

`dioxus-native`'s Blitz renderer produced three confirmed, source-verified
gaps in one session (Phase 11-13): `position:sticky` is parsed but never
implemented, CSS `transform` is invisible to hit-testing, and a
`:hover`-driven `width` toggle on a `position:absolute` flex child - a
pattern that should work per Taffy's own source - still only "minimized
an insignificant amount" after three attempts. These are real, load-
bearing gaps in ordinary desktop-app patterns (pinned panels, collapsible
sidebars), not edge cases. Decision: migrate to **egui** (MIT/Apache-2.0,
no license conditions, `egui::SidePanel` + real pointer-position hover
solve exactly the pattern that kept failing). Full reasoning and the crate
comparison (egui vs. Slint vs. iced, why `dioxus-desktop`/Tauri were ruled
out) is in the conversation record for this arc; the approved staged plan
is what this phase executes.

## What shipped this phase

New `app-egui` workspace member. `app/` (dioxus-native) is untouched and
stays the shipping build throughout - same coexist-during-migration
pattern this repo already used for its WinUI -> Rust move
(`powershell/`, `src/TextInFilesSearch(.Core)/` are still in the repo
today as that same pattern's artifacts).

- **`theme.rs`** - color tokens ported 1:1 from `main.rs`'s CSS custom
  properties, mapped onto `egui::Visuals`.
- **`main.rs`** - window scaffold, `ToolId` enum, and the app shell
  chrome. The auto-hiding rail - the bug that started this migration -
  is `egui::SidePanel::exact_width`, animated via `Context::animate_bool`
  from real pointer-position hover-testing (`ctx.pointer_hover_pos()`),
  not a CSS pseudo-class or `transform`. It collapses to fully hidden and
  the rest of the app reflows live, because `SidePanel` is real layout,
  not an overlay - both properties Blitz could never deliver.
- **`search.rs`** - Search Files tool. `search-core::orchestrator::run`
  is reused completely unchanged (it already reported progress over a
  plain `tokio::sync::mpsc::UnboundedSender<SearchProgressReport>`,
  never touching a Dioxus `Signal` directly - confirmed by reading it,
  not assumed). The new piece is the async bridge every future
  background-work tool will reuse: one persistent
  `tokio::runtime::Runtime` (created once, lives for the process,
  spawned into via `Runtime::spawn` from inside `update()`) plus an
  `Arc<Mutex<SearchUiState>>` a background task writes into and
  `update()` reads once per frame, polling for repaints only while a
  search is actually running.
- **`pressure_vessel.rs`**, **`bushing.rs`**, **`components.rs`** - both
  engineering tools, wired to `pressure-vessel-solver`/`bushing-solver`/
  `mechanics-core` completely unchanged (pure math, zero Dioxus
  coupling, confirmed by reading their real public APIs before wiring -
  `BushingInputs` derives `Default` and its own doc comment guarantees
  every field this pass doesn't expose reproduces the original
  straight-bushing-only behavior, so leaving them at
  `..BushingInputs::default()` is a real, documented construction, not
  invented). `components.rs` ports the approved mockup's two chosen
  designs: Checks as Stat Tiles, the status rail as a Ladder/Center-
  spine (rank-evenly-spaced node positions, not raw-value-positioned -
  the mockup's first pass showed raw positioning visibly collides
  whenever two margins are numerically close; ranking preserves the
  "how close to the edge" ordering with zero possible overlap).

## Explicit scope cut, not silently dropped

Each tool's *core* workflow is real and functional; the following are
tracked follow-up, not shipped this phase:

- **Search**: the Matching/Scope-and-output/Performance/Fast-re-search-
  index settings sections, HTML/CSV/JSON report export, presets, recent
  searches, drag-drop, desktop notifications, the extension type-to-
  filter picker (uses the engine's built-in default list unconditionally
  for now), multi-root search (`search_paths_extra`).
- **Bushing Workbench**: flanged/countersink OD geometry, internal
  countersink ID geometry, friction/edge-load/install-thermal-assist
  fields, the cross-section visualizer + lightbox, the full worst-case-
  across-tolerance derivation view. Scoped to the straight-bushing case.
- **Pressure Vessel Analyzer**: the labeled engineering cross-section
  sketches from the approved mockup (real hand-authored SVG drafting
  views - porting them means building a resvg rasterization path into
  egui, a substantial follow-up on its own), the 8-step KaTeX derivation
  view.
- **App-wide**: settings/recent-search persistence, native completion
  notifications, the command palette, the context menu, filesystem
  watching (folder-changed-since-search).

## Verification

- `cargo build --workspace`: clean.
- `cargo test --workspace`: all existing suites unchanged and green
  (149 search-core, 43 bushing-solver, and the rest - this phase touched
  zero business-logic crates, only added a new UI-layer consumer of
  them). `app-egui` itself has no unit tests yet - it's UI composition
  calling into already-tested solvers/orchestrator, the same shape
  `app/`'s own UI layer has always had.
- **Not verified**: actual on-screen rendering/interaction. No local GUI
  capability in this environment - same standing limitation as every UI
  phase this whole session. CI (`rust-build.yml`) validates that this
  builds for `x86_64-pc-windows-msvc` and checks for an accidental
  WebView2Loader.dll dependency (irrelevant to egui, but the same
  regression check still makes sense to keep passing) - it does **not**
  verify the UI actually looks or behaves correctly. A real screenshot
  from the user is still the only way to confirm that.
