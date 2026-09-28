# app/ — Dioxus-native Desktop GUI ("Toolbench")

> TL;DR: Rust/Dioxus desktop shell built on `dioxus-native` (Blitz/WGPU/winit, NOT WebView) — a tool-switcher rail hosting Search (full), Bushing Workbench (full), Pressure Vessel Analyzer (full), plus 3 inert placeholders. UI-only; all business logic lives in sibling crates.

## Purpose
Owns: window/launch, the dashboard shell (rail + topbar + stage), and 3 fully-functional tool UIs (Search, Bushing Workbench, Pressure Vessel Analyzer).
Does not own: search/matching/extraction (`search-core`), fast-index (`native-search`), engineering math (`bushing-solver`/`pressure-vessel-solver`/`mechanics-core`) — app/ only orchestrates and renders those.

**Current status vs `app-egui/`** (verify before assuming either is authoritative): `app-egui` is a separate Cargo workspace (`../app-egui/`, excluded from the root workspace over a `windows`-crate version conflict) built to replace `app/`'s renderer — see root `CLAUDE.md`'s "Why dioxus-native" section and `docs/app-egui-parity-checklist.md`. Per this repo's coexist-during-migration convention (same treatment `powershell/` and the C# app got), `app/` stays untouched as the shipping build until app-egui reaches parity — it is not dead code. Evidence as of this writing: last commit touching `app/` was 2026-09-08 vs. `app-egui/` 2026-09-23 (heavy, ongoing activity); the parity checklist shows Search/Bushing mostly DONE for app-egui, which has also grown an entire new tool (a neural-network "Stress Solver") with no `app/` equivalent. Check `docs/app-egui-parity-checklist.md`'s open rows before treating `app/` as legacy.

## Code Map

### Find It Fast
| Looking for... | Go to |
|---|---|
| Entry point, window/icon setup, dashboard shell (rail/topbar/stage), `ToolId` enum, hand-written SVG icons | `src/main.rs` |
| Hand-rolled launch sequence (replaces `dioxus_native::launch_cfg`) | `src/main.rs::mod launch` |
| All app state (`Signal<T>` per setting) + run/cancel/preset/index async logic | `src/state.rs` (`AppState`) |
| Search tool UI: `SettingsPanel`, `ResultsPanel`, shared `Expander`/`Dropdown`/`NumberField`/`MaterialField`/`CheckGauge` | `src/components.rs` |
| Bushing Workbench tool (steps, sketches, derivation view) | `src/bushing_workbench.rs` |
| Bushing cross-section SVG rendering (data-URI SVGs) | `src/bushing_visualizer.rs` |
| Pressure Vessel Analyzer tool | `src/pressure_vessel_workbench.rs` |
| File preview pane | `src/preview.rs` |
| Ctrl/Cmd+K command palette | `src/command_palette.rs` |
| Right-click context menu (no native API on this renderer) | `src/context_menu.rs` |
| Drag-and-drop workaround (winit `ApplicationHandler` wrapper) | `src/drag_drop.rs` |
| Filesystem watcher (folder-changed-since-search hint) | `src/fs_watch.rs` |
| Settings/recent-search/preset persistence (JSON) | `src/persistence.rs` |
| `data:` URI image loading (Bushing's LaTeX formula images) | `src/net_provider.rs` |

## Public API
| Export | Notes |
|---|---|
| `state::AppState` | Flat `Copy` struct of `Signal<T>` — pass by value into components/async tasks, never wrap in `Arc` or a context provider |
| `main::ToolId` | `Search \| Bushing \| PressureVessel \| Dupes \| Rename \| Logs`. Runtime-only `Signal`, not persisted (always defaults to `Search`). `docs/toolbench-status.md` is stale — it describes only `Search` as real; Bushing and PressureVessel are real tools now too |
| `components::{Expander, Dropdown, NumberField, MaterialField, CheckGauge}` | Shared widgets reused by both engineering workbenches — extend here, don't fork |

## External Dependencies
| Dep | Used for | Note |
|---|---|---|
| `search-core`, `native-search` (path) | Search/matching/extraction/report + fast index | app/ is UI-only over these |
| `bushing-solver`, `pressure-vessel-solver`, `mechanics-core` (path) | Engineering math for the two workbenches | pure math, zero Dioxus coupling |
| `dioxus` (`"native"`, not `"desktop"`) | Blitz/WGPU/winit renderer | see Decisions |
| `blitz-shell` (`"data-uri"`), `blitz-html`, `blitz-dom`, `blitz-traits` | Renderer internals app/ reaches into directly for the hand-rolled launcher/net-provider | version-pinned; the Pitfalls below were confirmed against blitz-dom 0.2.4 specifically — re-verify against source on a version bump |
| `rfd`, `open`, `arboard`, `notify`, `winit` | Folder picker, report opening, clipboard, fs watch, event loop | |
| `notify-rust` | Desktop completion toast | `[target.'cfg(not(windows))'.dependencies]` ONLY — its Windows backend needs `windows ^0.61`, conflicting with this workspace's `windows ^0.58` (fontique/Blitz's font stack); cfg'd out at compile time on Windows, not just an OS capability check |

## Entry Points
| Task | Start Here |
|---|---|
| Add/change a search setting | `src/state.rs` (`AppState` field + `build_settings`) → `src/components.rs` (`SettingsPanel`) |
| Add a form control (checkbox, dropdown, collapsible section) | `src/components.rs` — reuse `Expander` or `Dropdown`, use `oninput` (see Pitfalls; never `onchange`, never a raw `<details>` or `<select>`) |
| Add a real tool behind a currently-placeholder rail slot (Dupes/Rename/Logs) | `src/main.rs` (`ToolId`, nav list, `.stage` switch) + a new `src/<tool>.rs` module, following `bushing_workbench.rs`'s shape |
| Debug a layout/interaction bug that "should work" | Check Pitfalls below first — several ordinary CSS/DOM patterns are silently unsupported on this renderer |
| Change persisted settings/presets | `src/persistence.rs` (`PersistedState`, `save`/`apply`/`build_snapshot`) |
| Touch the launch sequence, window/event-loop, or drag-drop | `src/main.rs::mod launch` + `src/drag_drop.rs` (read both — they're coupled) |

## Decisions
| Decision | Why | Rejected |
|---|---|---|
| `dioxus` `"native"` feature, not `"desktop"` | `wry` hardcodes `browserExecutableFolder=null` → no app-local WebView2 bundling possible; violates project's "no host-machine dependency" requirement | `"desktop"` (wry/WebView2) — full verification trail + CI's WebView2Loader.dll regression check: root `CLAUDE.md` |
| Fixed-size pagination (`RESULTS_PAGE_SIZE = 50`, `components.rs`), not list virtualization | Scroll position is never forwarded to app code, and native wheel-scroll is consumed by blitz-shell before `onwheel` fires — computing "which slice is visible" is architecturally impossible here, not just unbuilt | Scroll-based virtualization |
| `AppState` is a flat `Copy` struct of `Signal<T>` fields | `Signal<T>` is itself `Copy` — the idiomatic single-window Dioxus pattern | Context-provided struct / nested state tree |
| Custom `ContextMenu`/`CommandPalette` components instead of framework event types | `oncontextmenu` exists in dioxus-html but this renderer never dispatches it — right-click only arrives as `MouseButton::Secondary` on an ordinary mouse event | Native `oncontextmenu` |

## Contracts
- Numeric `<input>` handlers must call `.set()` only inside `if let Ok(v) = evt.value().parse() { ... }`, never with a fallback `else`. A controlled input's `value` re-renders every signal change, so a default-on-every-keystroke fights the user's typing mid-edit (real, fixed bug).
- `#![windows_subsystem = "windows"]` (`main.rs`) is per-binary-crate, not inherited by a sibling binary target. `app-egui` shipped once without it (real regression: a console-owner window that killed the whole process on close) — check explicitly if a third binary target is ever added.
- `folder_changed_since_search` (`AppState`) is reset to `false` only inside `run_search`'s start — resetting it elsewhere silently breaks the fs-watch "folder changed" hint.

## Pitfalls (dioxus-native / blitz-dom 0.2.4 renderer — confirmed by reading blitz-dom source; full citation trail in root `CLAUDE.md`)
- **Event types that never fire/dispatch on this renderer**: `onchange` (no `Change` variant in `blitz-traits::events::DomEventData` — use `oninput` for everything, including checkboxes); `oncontextmenu` (defined by dioxus-html but never dispatched — right-click arrives as an ordinary mouse event, use `onmousedown` + check `evt.trigger_button() == Some(MouseButton::Secondary)`, see `context_menu::maybe_open_context_menu`).
- **Native widgets only cosmetically implemented — always use this crate's custom replacement, never the raw element**: `<details>`/`<summary>` has two gaps (no click-to-toggle, and even a toggling `open` attribute wouldn't hide content — native collapse-on-close is engine-level layout behavior blitz-dom never reimplements) — use `components::Expander` (signal-driven `open` + `if open() {...}` body). `<select>` renders every `<option>`'s text flattened with no popup (`blitz-dom-0.2.4/src/form.rs` has no select-widget implementation) — use `components::Dropdown` (renders inline in document flow, not `position: absolute/fixed`, when open).
- **CSS properties parsed but not actually implemented**: `position: sticky` behaves exactly like `static` (pin via a bounded-height `overflow-y: auto` *scrolling sibling* instead, see `.settings-column`/`.results-column`); `transform` is invisible to hit-testing (an element hidden via `transform` still absorbs hover/click at its untransformed box — animate `width`/`height`/`max-height` for hover-driven show/hide instead); `filter: blur()`/`backdrop-filter` are never painted (blitz-paint reads only `.opacity` — approximate a glow with a radial gradient with a transparent outer stop, see `.ambient-glow*`).
- No scroll-position API reaches application code, and native wheel-scroll is consumed before `onwheel` fires — this is why results use fixed-size pagination, not virtualization (see Decisions). Don't attempt to compute a "visible slice" here.
- **Two resource/event streams silently dropped by `blitz-shell` unless intercepted**: `WindowEvent::DroppedFile`/`HoveredFile`/`HoveredFileCancelled` never reach the Dioxus app (fixed by `drag_drop.rs`'s `winit::ApplicationHandler` wrapper); `net_provider: None` silently no-ops `<img src="data:...">` (Blitz falls back to a true no-op `DummyNetProvider` — fixed by `net_provider::data_uri_only`, which resolves `data:` URIs only, never swap in the full HTTP-capable `blitz-net::Provider` without a real need for remote fetches).

## Patterns

### Adding a real tool behind a placeholder rail slot
1. Create `src/<tool>.rs` exposing a `#[component] pub fn <Tool>Workbench(...) -> Element`, mirroring `bushing_workbench.rs`'s step/state shape (or `state::AppState`'s pattern if it needs persisted settings).
2. In `src/main.rs`: add `mod <tool>;`, render the real component in the `.stage` match arm that currently renders `PlaceholderTool` for that `ToolId`, and drop that `ToolId`'s `"Soon"` pill from the nav item.
3. If the tool needs settings that outlive a session, extend `persistence::PersistedState` and its own state struct together — a field added to only one silently never round-trips.

## Boundaries

### Always
- Use `oninput` for every form control, including checkboxes — never `onchange`.
- Route new business logic into `search-core` or the solver crates, not into `app` — keep this crate UI-only.

### Never
- Add a raw `<details>`/`<summary>` or `<select>` — use `components::Expander`/`Dropdown`.
- Assume `app/` is dead because `app-egui/` is more active — check `docs/app-egui-parity-checklist.md` first.

### Verify First
- Re-verify the "no host-machine dependency" WebView2 constraint (root `CLAUDE.md`) before ever switching `dioxus`'s feature back to `"desktop"`.

## Downlinks
Leaf node — no children. Depends on sibling nodes: `search-core/AGENTS.md`, `native-search/AGENTS.md` (fast re-search index), and `bushing-solver`/`pressure-vessel-solver`/`mechanics-core` if documented.
