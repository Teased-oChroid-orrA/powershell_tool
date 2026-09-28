# app-egui/ — egui/eframe Desktop GUI ("Toolbench")

> TL;DR: Rust/egui+eframe desktop shell — the newer, actively-developed UI implementation of the "Toolbench" multi-tool dashboard, built to escape confirmed Blitz/dioxus-native renderer bugs (`docs/issue-11-phase-14.md`). Hosts 4 real tools (Search, Bushing Workbench, Pressure Vessel Analyzer, Stress Solver) plus 3 inert placeholders (Dupes/Rename/Logs). Own standalone Cargo workspace. UI/wiring only; math and search logic live in sibling crates.

## Purpose
Owns: window/launch (`main.rs`), app shell chrome (auto-hiding rail, topbar, command palette), 4 fully-functional tool UIs, and this crate's own design system (typography/spacing/radii/shadows/icons).
Does not own: search/matching/extraction (`search-core`), fast-index (`native-search`), engineering math (`bushing-solver`/`pressure-vessel-solver`/`mechanics-core`), or PINN/AMR solver internals (`pinn-core`/`pinn-solver` — separate sibling repo, see External Dependencies).

**Migration status vs `app/` (verify — see Boundaries, status changes fast)**: both crates are BOTH actively developed, not a clean supersession. `app-egui` was started to work around confirmed Blitz/dioxus-native renderer bugs (`position:sticky`, `transform` hit-testing, hover-toggle failures — `docs/issue-11-phase-14.md`), but `app/` kept getting real feature commits through 2026-09-08 while `app-egui` has heavier ongoing activity through 2026-09-23, including a PINN Stress Solver tool with no `app/` counterpart. Both `app/`'s and this crate's `bushing.rs` independently import `bushing-solver` and are both live. Check `docs/toolbench-status.md`, `docs/rust-rewrite-status.md`, `docs/app-egui-parity-checklist.md` for current parity before treating `app/` as legacy.

## Code Map

### Find It Fast
| Looking for... | Go to |
|---|---|
| Entry point, `ToolbenchApp`, `ToolId` enum (7 rail slots, 4 real), nav list, tool dispatch | `src/main.rs` |
| `#![windows_subsystem]` attribute for this binary | `src/main.rs` (top of file, line ~30) |
| Color tokens (ported 1:1 from `app/`'s CSS custom properties) | `src/theme.rs` (`Tokens`) |
| Ctrl/Cmd+K command palette (scoped subset of `app/`'s commands) | `src/command_palette.rs` |
| Search Files tool: async bridge, `SearchUiState`, index-with-progress | `src/search.rs` |
| Windows-specific desktop-notification no-op | `src/search.rs::notify_search_complete` (~line 234) |
| Bushing Workbench tool (6-step stepper) | `src/bushing.rs` |
| Pressure Vessel Analyzer tool (per-frame synchronous recompute, no async needed) | `src/pressure_vessel.rs` |
| Stress Solver tool (PINN training UI, background job via `spawn_blocking`) | `src/stress_solver.rs` |
| Debug-build slow-training warning banner | `src/stress_solver.rs` (~line 1391, `cfg!(debug_assertions)`) |
| Force-directed "brain map" graph view of search results (bipartite file↔filter, hand-rolled pan/zoom) | `src/graph.rs` |
| Engineering cross-section sketches (hand-drawn via `egui::Painter`, not SVG) | `src/sketches.rs` |
| Shared engineering-tool UI (Stat Tiles, Ladder/Center-spine status rail) | `src/components.rs` |
| Shared chrome widgets (`.card`, `.nav-item`, `.step-pill`, `.headline`) ported from mockup CSS | `src/widgets.rs` |
| Settings/recent-search/preset persistence (plain serde JSON, NOT a port of `app/persistence.rs`'s Dioxus-coupled version) | `src/persistence.rs` |
| Design system: type scale + bundled Inter/JetBrains Mono fonts | `src/design/typography.rs` |
| Design system: Lucide SVG icon rasterization (resvg/usvg/tiny-skia) | `src/design/icons.rs` |
| Design system: Button/Segmented/Select/Tooltip/EmptyState/Toast primitives | `src/design/components.rs` |
| Design system: spacing/radius/shadow scales | `src/design/spacing.rs`, `radii.rs`, `shadows.rs` |
| Bundled font/icon binary assets | `assets/fonts/`, `assets/icons/` |

### Key Relationships
- `main.rs` → per-tool module (`search.rs` / `bushing.rs` / `pressure_vessel.rs` / `stress_solver.rs`) → shared `widgets.rs`/`components.rs`/`design/*` — tools never talk to each other directly.
- Each tool module calls into its own sibling math/logic crate (`search-core`+`native-search`, `bushing-solver`, `mechanics-core`, `pinn-core`+`pinn-solver`) and stays framework-agnostic on the other side of that call.

## External Dependencies
| Dep | Used for | Failure Mode |
|---|---|---|
| `pinn-core`, `pinn-solver` (path: `../../../NeuralNetwork-Stress-Solver/crates/...`, features = `["ndarray-backend"]`) | All PINN training, AMR, parametric-model, checkpoint, network-viz, Kt-convergence logic for Stress Solver | **Separate sibling repo**, not part of this repo, no version pin beyond the relative path. A change to its public API (`pinn_core::messages`, `pinn_solver::runner`/`checkpoint`/`parametric_problem`) can break this crate's build with no local diff to explain why — the fix is always in that repo. Don't document its internals here; give it its own Intent Layer if needed. |
| `search-core` (features = `["ocr"]`), `native-search` (path) | Search/matching/extraction/report + fast index | Same crates `app/` uses; `search.rs` must call the `_send`-suffixed generic entry points, not the plain `dyn FnMut` ones (see Contracts) |
| `bushing-solver`, `mechanics-core` (path) | Pure engineering math, zero GUI coupling | |
| `eframe`/`egui` 0.29 (`glow` backend, not `wgpu` directly — see Decisions), `egui_extras` | Rendering | wraps `winit`, no WebView — same "no host-machine runtime dependency" constraint `app/` had |
| `resvg`/`usvg`/`tiny-skia` | Lucide icon SVG rasterization | see Pitfalls |
| `notify-rust` | Desktop completion toast | `[target.'cfg(not(windows))'.dependencies]` only — see Pitfalls |
| `crossbeam-channel` | Training-thread → UI channel (`bounded(1)` latest-value + `unbounded` control) | see Contracts |

## Decisions
| Decision | Why | Rejected |
|---|---|---|
| Own standalone Cargo workspace (`exclude`d from root) | `app`'s fontique/parley needs `windows ^0.58.0`; this crate's wgpu-hal 29.x + burn-wgpu (pinn-solver) need `windows-core ^0.62` — confirmed incompatible via `cargo update -p windows@0.58.0 --precise 0.62.2` failing | One shared root `Cargo.lock` |
| `eframe`/`egui`, replacing dioxus-native/Blitz | `position:sticky` unimplemented, `transform` invisible to hit-testing, a proven `:hover` toggle failed 3x on Blitz — `docs/issue-11-phase-14.md` | New tools on `app/`'s Blitz renderer |
| `notify-rust` cfg'd out entirely on Windows | Its Windows backend needs `windows ^0.61`, incompatible with this crate's `windows-core ^0.62` (via pinn-solver) — same `--precise` check as above | Runtime feature-gating (conflict is at dependency-resolution time) |
| Training via `runtime.spawn_blocking`, never `runtime.spawn` | The PINN loop is a tight synchronous CPU loop, no `.await` points | `tokio::spawn` (starves other tasks) |

## Entry Points
| Task | Start Here |
|---|---|
| Add a real tool behind a placeholder rail slot | `src/main.rs` (`ToolId`) → new `src/<tool>.rs` — see Patterns below |
| Debug a layout bug where content collapses/disappears in a scroll area | Check Pitfalls' `available_size().y` / missing-`ScrollArea` entries first |
| Change persisted settings | `src/persistence.rs` (`PersistedState`, `load`/`save`) |
| Add/change a design-system token (color/spacing/radius/font/icon) | `src/theme.rs` (color) or `src/design/*.rs` (everything else) |
| Debug a Windows-only build or notification issue | `src/search.rs::notify_search_complete` + this crate's `Cargo.toml` `[workspace]` comment (windows-crate conflict) |
| Wire a new PINN capability into the Stress Solver UI | `src/stress_solver.rs` — check the sibling `NeuralNetwork-Stress-Solver` repo's public API first (see External Dependencies) |

## Contracts
- `#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]` (`main.rs` ~line 30) is per-binary-crate, not inherited from `app/`'s own copy. If a third binary target is ever added to this repo, it needs this attribute explicitly too (confirmed real regression class in `app/AGENTS.md`).
- `search.rs::index_one_root_with_progress` must call `search_core::native_index::build_or_update_corpus_index_send` (generic `F: FnMut(..) + Send`), never the plain `build_or_update_corpus_index` (`Option<&mut dyn FnMut(..)>`, unconditionally `!Send`). eframe's async bridge runs on a multi-thread `tokio::Runtime`, which requires the spawned future to be `Send`; an unannotated `dyn FnMut` trait object never carries `Send` even when the concrete closure does. See `search-core/src/native_index.rs:203-220`.
- The training→UI channel (`stress_solver.rs`, `bounded(1)` + `try_send`) intentionally drops intermediate steps under UI-poll contention — a "latest value wins" channel by design, not a bug. Chart marker x-position must use the just-pushed vector index (`self.total_loss.len() - 1`), never the real `report.step`, or markers misalign whenever a step is dropped (~line 1248). The real step is still shown separately in the status card text.
- `tiny_skia::Pixmap::data()` output is premultiplied alpha — feed it only to `egui::ColorImage::from_rgba_premultiplied`, never `from_rgba_unmultiplied`, or anti-aliased icon edges render too dark (`design/icons.rs` ~line 58).
- egui's bundled fallback fonts (Ubuntu-Light/Hack-Regular/NotoEmoji-Regular/emoji-icon-font) must stay installed as fallback entries even though this crate bundles Inter/JetBrains Mono — several glyphs used throughout this UI (✔ ⚙ 🖊 🌙 and others) are confirmed absent from Inter/JetBrains Mono via `fontTools` cmap inspection and only render through the original bundled fonts (`design/typography.rs`, top-of-file doc comment).

## Pitfalls
- **Debug builds are ~20-30x slower for PINN training** (measured: ~2.2s/step debug vs ~93ms/step release on a `single_hole_plate`-shaped config) — `stress_solver.rs` (~line 1391) shows a `cfg!(debug_assertions)`-gated warning banner specifically so a `cargo run` (no `--release`) user doesn't file a false "hang" bug. Don't remove without replacing the warning some other way.
- `ab_glyph` (egui/epaint's text rasterizer) has no variable-font axis support — `design/typography.rs` bundles Inter + JetBrains Mono pre-instanced to static Regular/Medium/SemiBold/Bold weights via `fonttools varLib.instancer` before `include_bytes!`. Dropping in a variable `.ttf` directly renders every weight at the font's single default instance, not the weight you asked for.
- Lucide SVGs use `stroke="currentColor"`; `usvg` has no CSS cascade to resolve that concept, so `design/icons.rs` string-replaces `"currentColor"` with a real hex color before `Tree::from_str` (~line 45) — a raw bundled SVG will silently fail to tint.
- `ui.available_size().y` is unsafe to size a chart/image off of inside an `egui::ScrollArea` — egui 0.29's scroll viewport gives no infinite height on the scroll axis (that path is unreachable in this pinned version), so anything sized off it collapses toward ~1px as sibling content grows. Convention here: size off `available_width()` with a fixed height cap (`stress_solver.rs` ~lines 2552, 2833-2851).
- `stress_solver.rs`'s `step_content` is wrapped in `ScrollArea::vertical().auto_shrink([false, false])` (~line 1451) — the `auto_shrink` arg matters; the default shrinks-to-fit and leaves stacked cards unreachable off-screen.
- **Confirmed still-open gap**: `pressure_vessel.rs`'s `step_content` (line 173) has no `ScrollArea` wrapper at all — same shape `stress_solver.rs` once had and fixed. Verify current source before assuming fixed; apply the same fix if reproducing the off-screen-card symptom.
- `notify_search_complete` (`search.rs` ~213-244) is a deliberate no-op on Windows (`#[cfg(target_os = "windows")]`): `notify-rust`'s Windows backend needs `windows ^0.61`, incompatible with this crate's `windows-core ^0.62`. The non-Windows path is wrapped in `spawn_blocking` + `catch_unwind`, ported from a real confirmed Windows crash (~5s post-search) in an earlier version that fired unconditionally on the runtime task — this file had silently reintroduced that bug once already.
- `main.rs`'s auto-hiding rail hover zone must match the mockup's actual sliver width, not an arbitrary threshold (fix: commit `b21d10b`) — hover-reveal is driven by `ctx.input()` pointer position + `Context::animate_bool`, not CSS.
- `components.rs`'s `TILE_WIDTH` (170.0, ~line 52) must use `set_width` (a cap), never `set_min_width` (a floor) — a screenshot-confirmed bug had tiles stretch to fill the card because `available_width()` inside a nested `Grid` reports the whole card's width; the status rail was pushed/painted over as a result.

## Patterns

### Adding a real tool behind a placeholder rail slot (Dupes/Rename/Logs)
1. Create `src/<tool>.rs` mirroring `pressure_vessel.rs`'s or `bushing.rs`'s stepper/card shape (or `stress_solver.rs`'s async/`spawn_blocking` shape if the tool needs a background job).
2. In `main.rs`: add `mod <tool>;`, add the real render call where the placeholder currently renders for that `ToolId`, and add the variant to `ToolId::enabled()`'s `matches!` so it stops showing as a "Soon" placeholder.
3. If the tool needs persisted settings, extend `persistence::PersistedState` — a field added without a matching `#[serde(default)]` breaks old config files on load (see `persistence.rs`'s own doc comment).

## Boundaries

### Always
- Size scrollable content off `available_width()` with a fixed height cap, never off `available_size().y`, inside a `ScrollArea`.
- Feed `tiny_skia` pixmap data to `from_rgba_premultiplied`, never `from_rgba_unmultiplied`.
- Call `search_core::native_index::build_or_update_corpus_index_send`, not the plain `dyn FnMut` variant.

### Never
- Add a variable-weight `.ttf` directly to `design/typography.rs` — pre-instance static weights with `fonttools varLib.instancer` first.
- Remove egui's bundled fallback fonts — several in-app glyphs only render through them.
- Assume `app/` is dead because `app-egui/` is more active, or vice versa — both get real feature work.

### Verify First
- Migration/parity status vs `app/` — check `docs/toolbench-status.md`, `docs/rust-rewrite-status.md`, `docs/app-egui-parity-checklist.md`, and `docs/issue-11-phase-14.md` before stating either crate is authoritative or legacy; this changes fast and this file may be stale.
- Whether `pressure_vessel.rs::step_content` still lacks a `ScrollArea` — re-check current source (see Pitfalls) before assuming it's fixed or unfixed.

## Downlinks
Leaf node — no children. Depends on sibling nodes: `search-core/AGENTS.md`, `native-search/AGENTS.md`, `bushing-solver/AGENTS.md` (also consumed by `app/`), and the out-of-repo `NeuralNetwork-Stress-Solver` project (no Intent Layer node here — out of scope, see External Dependencies).
