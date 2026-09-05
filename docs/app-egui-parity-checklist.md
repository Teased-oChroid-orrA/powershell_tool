# app-egui parity checklist

Single source of truth for what `app-egui` (the egui/eframe migration
target) still owes vs. `app/` (the dioxus-native app it's replacing) and
vs. the approved mockup artifact. Supersedes the deferred-item prose in
`docs/issue-11-phase-14.md`/`-15.md` — those stay as historical record,
but this table is what gets checked and updated, not them.

**Standing rule (see `CLAUDE.md`): no chrome/cosmetic-only `app-egui`
commit lands while a `P0` row below is OPEN**, unless the user explicitly
asked for cosmetic work specifically. A row moves to DONE only once
either a real user screenshot confirms it, or it's backend-only and
covered by `cargo test -p search-core`.

**The approved mockup artifact is now saved locally** - fetched in full
via its `claude.ai/code/artifact/...` URL, not just glanced at through a
screenshot crop. Its raw CSS/HTML/JS (rail, cards, tiles, ladder, sketch
SVGs, exact hex colors, exact class dimensions) is the actual design
spec, not a reconstruction from pixels - re-fetch it before redesigning
any chrome piece rather than guessing from how a screenshot happens to
look. A whole pass of "fix the ladder" earlier in this project's history
redesigned it away from the real spec (centered/alternating spine →
single left column) specifically because no one had the actual artifact
to check against - see `components.rs::ladder`'s doc comment for the
full story and the corrected CSS-accurate version.

**`ui.set_min_width(...)` does not cap `ui.available_width()` for nested
content - only `ui.set_width(...)` does.** This caused TWO separate,
screenshot-confirmed layout bugs in one session (`widgets::side_by_side`'s
left column, and `components.rs::tile`'s width) where content several
levels deep read an ambient/inherited width instead of its real column
width, in both cases visually destroying the layout (a sketch pane
swallowing the persistent status rail's column entirely; Stat Tiles
stretching to fill an entire card and overflowing the window). Neither
was caught by `cargo check`/`cargo test` - only running the actual
binary and screenshotting it surfaced them. If a `set_min_width` call
exists purely to stop content from collapsing (not to allow it to grow),
audit whether it should be `set_width` instead.

**Static code review is not sufficient for this crate - actually run
`cargo build -p app-egui && ./target/debug/app-egui` and screenshot it**
before claiming a layout/rendering fix works. This environment has a
real display (`screencapture` works); multiple fixes in this session
that looked correct on paper (re-read three times, reasoned through
egui's layout model) turned out wrong or incomplete only once actually
rendered - the "not independently verified on-screen" disclaimer this
project has carried since Phase 14 was true every time it was written,
not a formality.

## Search (P0 — the app's whole reason for existing)

| Item | Status | Closed by |
|---|---|---|
| Folder picker, filter text, run/cancel, live progress, results list | DONE | Phase 14 |
| HTML report export + open | DONE | f1dbb29 |
| Desktop completion notification | DONE | f1dbb29 |
| Native drag-drop | DONE | ca285af |
| Settings persistence (search_path/filters_text/parallel only) | DONE | ca285af |
| Command palette (Ctrl/Cmd+K) | DONE | d279bd3 |
| Matching section (match mode, whole-word, regex, proximity lines, live regex-error validation) | DONE | this pass |
| Scope/output section (exclude filters+scope, exclude folders, group-by, extension picker+custom-add, CSV/JSON export) | DONE | this pass |
| Performance/robustness section (throttle limits, max file size, PDF/file timeout, retries, OCR toggle, cache path, dry run) | DONE | this pass |
| Fast re-search (native index) toggle - real trigram-candidate routing, not just a UI checkbox | DONE | this pass |
| Fast re-search: manual "Build/update index" + "Rebuild from scratch" buttons, live per-file status, existing-index detection message | DONE | this pass |
| Fast re-search: auto-update the index after every completed search (was query-only before; the index never actually got refreshed) | DONE | this pass |
| Settings defaults survive an old/partial settings file - `SearchFieldsSnap`'s `#[derive(Default)]` silently zeroed every Performance-section field (throttle/timeouts/retries/max-file-size) whenever the whole `search` JSON key was missing; screenshot-confirmed real bug | DONE | this pass |
| Status-rail ladder matches the approved artifact's real CSS (fetched and read directly, not guessed): gradient spine (danger→border→good), two-line labels (bold value, name below), 232px rail width, SAFE/LIMIT tags, long real check names truncated (the mockup's own names were always short enough to never need it) | DONE | this pass |
| Multi-root search (`search_paths_extra`) | DONE | this pass |
| Presets + recent searches | DONE | this pass |
| Desktop-notification crash fix (unconditional `notify_rust` call on the async task - the exact bug `app/`'s own `notify_search_complete` doc comment records as a real, confirmed Windows crash, reintroduced here before this pass) | DONE | this pass |
| Context menu on results | OPEN | |
| Filesystem watching (folder-changed-since-search) | OPEN | |
| Index saved at the OUTPUT folder instead of inside the search folder, with conflict detection if one already exists there | DONE | this pass - "Index location" picker, existence+last-built-time detection for BOTH locations, warn-then-confirm before touching an existing index. New scope beyond `app/`'s design (ADR-011 covers `SearchFolder` only). |
| Output-folder placement gave every root its OWN subdirectory, not one shared index - a real bug found by a direct user question: two unrelated searches pointed at the same output folder used to silently merge into one corpus. Fixed: keyed by a stable hash of the root path (`sanitized_root_key`, tested) | DONE | this pass |
| Search progress matches the artifact's own named design ("Search progress = Timeline" - literally in its banner comment): a vertical Scan → Search timeline with connecting line and done/active/pending dots, not a flat "X/Y files" progress bar | DONE | this pass |
| Interactive "brain map" for results (`graph.rs`): a real bipartite node-link graph (file nodes, matched-filter nodes, edges for every real match), not decoration - force-directed layout, drag-to-pin, scroll-to-zoom, drag-background-to-pan, hover tooltips. Alternate view next to the list via a "List / Brain Map" toggle, not a replacement. A real label-collision bug (filter labels centered inside their own small node circle, clipped by whichever neighboring node's circle got drawn after and overlapped it) was found and fixed by screenshot - filter labels now sit below their node like file labels do, never centered on top of it | DONE | this pass |
| Tantivy index-writer crash resilience - a real, user-reported random `IndexError: An index writer was killed.` mid-large-index (confirmed against tantivy's own source: `send_add_documents_batch` checks `index_writer_status.is_alive()` and raises exactly this text once a background segment-writer thread has died; the SAME `IndexWriter` instance is permanently dead after that, only a fresh `index.writer(budget)` recovers). Fixed via `engine.rs`'s `with_writer_retry`: on `TantivyError::ErrorInThread`, transparently rebuilds the writer and retries once; `native_index.rs`'s indexing loop now tolerates a per-document/per-commit failure (`CorpusIndexOutcome::failed_files`) instead of aborting the whole run | DONE | this pass |
| Index memory/size - writer budget 50MB→100MB (`WRITER_MEMORY_BUDGET`); real index-size stat (recursive `dir_size_bytes`) surfaced in the build-summary message | DONE | this pass |
| Index-build progress rendered in the "Fast re-search index" dropdown instead of the Search Timeline - a real, user-reported wrong-location bug. Fixed: `index_build_timeline_stage` prepends an Active/Done stage into `timeline()`'s stage list; the dropdown's duplicate inline status label removed | DONE | this pass |
| Bushing/Pressure-Vessel sketch labels not updating as inputs change - confirmed isolated to the UI/sketch layer only (the solver/calculations were never affected - user explicitly asked this be verified). Root cause: `bushing_head_on`/`pv_head_on`/`pv_side_view` had hardcoded illustrative label strings (e.g. `"6.00 in"`, `"5000 psi"`) instead of the real solved values `bushing_side_view` already used correctly. Fixed by threading a real `BushingSketchCtx`/new `PvSketchCtx` (rebuilt every frame from live input/output values) into every previously-hardcoded label | DONE | this pass |

## Bushing Workbench (P1)

| Item | Status | Closed by |
|---|---|---|
| Repair/Geometry/Material/Fit/Analysis/Results steps, sketches, Stat Tiles, Ladder status rail | DONE | Phase 15 |
| Flanged OD geometry, friction, edge-load, install-thermal-assist | DONE | Phase 15 / 9d0b776 |
| Status rail persistent across every step (not just Results) | DONE | this pass |
| Column-width clash on narrow windows | DONE | this pass |
| Minimum neck wall (in) - a real, independent `BushingInputs::min_wall_neck` field the UI hardcoded equal to straight wall instead of exposing (confirmed missing against the artifact's own Analysis step, not a guess) | DONE | this pass |
| Housing vs. bushing hatch material differentiation in sketches (crossed hatch angle + warm/cool line tint per the artifact's own `h1`/`h2` SVG patterns - the port used the same angle and pure-white lines for both, reading as one undifferentiated blob) | DONE | this pass |
| Sketch always drew the flanged shape regardless of the "Flanged" checkbox - a real "doesn't reflect the part being analyzed" bug, confirmed by a direct user report | DONE | this pass |
| Sketch's bore had a sharp square corner - a real reference part's own STEP file (`/Users/nautilus/Downloads/231110746__.stp`, user-provided) confirmed a genuine 45° lead-in chamfer at both bore ends (`CONICAL_SURFACE` half-angle exactly π/4); added as a real geometric feature, not just a cosmetic notch | DONE | this pass |
| Force arrow ("F", applied load) above the assembly, matching a user-provided reference engineering drawing | DONE | this pass |
| Full STEP-file-driven parametric rendering (auto-deriving the sketch from an arbitrary CAD file) | OPEN - explicitly out of scope: real B-rep interpretation needs a CAD kernel, not something to hand-roll. The one reference STEP file's dimensions/chamfer angle were extracted by grepping its plain-text entities (`CYLINDRICAL_SURFACE`/`CONICAL_SURFACE` radii, `CARTESIAN_POINT` Z-extents) and used to inform this pass's proportions, not parsed as geometry. |
| Real 3-way head-type selector (Slug/Countersunk/Flanged, reusing `bushing_solver::geometry::BushingType` directly) - a real user correction: the sketch previously modeled itself after a general reference photo and drew two countersinks on opposite ends of the member, which isn't any of the three real repair-bushing types. Member geometry now adapts per type (countersunk gets a matching counterbore sized from the SAME solved `cs_solved_od` the head itself uses; flanged/slug get the correct flush relationship), and `bushing_length` vs. `housing_len` drives a real flush/protrusion/recess rendering with an annotation, never silently drawn flush | DONE | this pass |
| Lower-end (always opposite the head) and head-top-edge edge chamfers, both real editable dim-range + angle fields (defaults .007-.015 in @ 45° and .010-.015 in @ 0°/square-relief per the user's own confirmed convention) - drafting-only, explicitly not fed into `bushing_solver`'s margin calculations (disclosed in the UI caption) | DONE | this pass |
| Countersunk-head external geometry (`ext_cs_dia/depth/angle`) exposed as real editable UI fields for the first time - `BushingType::Countersink` existed in the solver but was unreachable from the UI (only a `Flanged` checkbox existed). Default `ext_cs_dia` was initially copied verbatim from a solver unit-test fixture built around a 0.5 in bore (0.6 in) - screenshot-confirmed too small relative to this tool's own ~0.876 in default bore (the "head" rendered smaller than the installed OD); fixed by scaling the default proportionally (1.05 in) | DONE | this pass |
| Internal countersink ID geometry (`IdType`/`cs_mode`/`cs_dia`/`cs_depth`/`cs_angle`) | DONE | this pass - real 3-way `CsMode` picker + spec-table rows, matching the external countersink treatment |
| Reamer picker on the bore field (`bushing_solver::reamers::nearest`) - a real feature the prior `app/` dioxus-native implementation had that never got ported at all | DONE | this pass |
| Real countersink dia/depth/angle mode selection for BOTH internal and external countersinks - a real, previously-shipped bug: the external mode was hardcoded to `CsMode::DiaAngle` while "Countersink depth" still rendered as a freely-editable field, so an edit to it was silently ignored by the solver (depth is the DERIVED dimension in that mode). Fixed by a real 3-way picker + spec-table rows that show the solver's actual solved value (read-only) for whichever dimension the active mode makes derived | DONE | this pass |
| Full tolerance-capture spec tables (Dimension/Nominal/Tol−/Tol+/Range/Source) for bore, interference, and both countersinks - ported from `app/src/bushing_workbench.rs`'s `PlainSpecRow`/`CsSpecRow`. Repair/Fit/Geometry form cards widened 280px→420px to fit them (disclosed, real layout change, not silent) | DONE | this pass |
| "Auto-tighten bore tolerance to meet target interference" enforcement checkbox (`EnforcementPolicy`) | DONE | this pass |
| `RangedValue`-based Results-step output displays (`wall_straight_range`, `pressure_range`, etc. - how computed OUTPUTS vary across the tolerance band, not input capture) | OPEN - genuinely separate, larger feature (result presentation vs. input capture) from everything above |
| Section-cut identifier ("A—A" tick marks + arrows on the head-on view, "SECTION A-A" caption under the side view) tying the two views together, matching real drafting-sheet convention | DONE | this pass |
| Fillet/radius callouts at the head-to-sleeve transition (Flanged/Countersunk; a slug has no shoulder to fillet) - labeled via leader line, not a literal rounded corner (this sketch's polygon primitives don't do per-corner rounding, a disclosed simplification) | DONE | this pass |
| Shaded pseudo-isometric preview (`bushing_isometric`) alongside the 2D views - a fixed-angle ellipse/wall silhouette with flat 2-tone shading, explicitly NOT a real 3D render (egui has no 3D pipeline outside `PaintCallback`), labeled "(SCHEMATIC)" so it's never mistaken for one | DONE | this pass |
| Two real, screenshot-confirmed label collisions found and fixed while verifying this pass: the countersunk fillet callout overlapping the head-type label, and the flanged fillet callout overlapping the (wide, far-left-anchored) flange dimension label - fixed by giving each head type's fillet callout its own clear vertical lane instead of one shared y | DONE | this pass |

## Pressure Vessel Analyzer (P1)

| Item | Status | Closed by |
|---|---|---|
| Geometry/Pressure/Material/Buckling/Results steps, sketches, Stat Tiles, Ladder status rail | DONE | Phase 15 |
| Status rail persistent across every step (not just Results) | DONE | this pass |
| Column-width clash on narrow windows | DONE | this pass |
| 8-step derivation view (KaTeX-equivalent) | OPEN | egui has no LaTeX renderer — needs a design decision (plain formatted derivation vs. a minimal math-rendering approach) before starting |
| Section-cut identifier + "SECTION A-A" caption + shaded pseudo-isometric preview (`pv_isometric`) - same treatment as Bushing's, reusing `Sketch::section_cut`/`section_caption` | DONE | this pass |

## App-wide (P2)

| Item | Status | Closed by |
|---|---|---|
| Rail/nav chrome, theme toggle, mockup-fidelity widgets (`card`, `stepper`, `nav_item`, `headline`) | DONE | Phase 15 |
| Card sizing / oversized-title / tile-chip bugs from first live screenshot | DONE | 9d0b776 |
| Context menu (app-wide, not just Search) | OPEN | see Search row above |
| Filesystem watching | OPEN | see Search row above |
| Modern input styling app-wide - a real, user-reported "ugly"/unstyled look across every `DragValue`/`TextEdit` in every tool, not a subjective nitpick: egui's own default widget rounding (2px) and cramped padding read as flat, unstyled boxes next to this app's 6-8px card/button rounding everywhere else, and per-row `ui.horizontal(|ui| { label; input })` layouts put every field's input box at a different x offset depending on that row's own label length - never lining up into a clean column. Fixed at the theme level (`theme.rs`'s `Visuals::widgets.*.rounding` + `main.rs`'s `Style::spacing.button_padding`, both matching the approved mockup's own `.field input` CSS) so every input app-wide picks it up with no per-call-site changes, plus a new `widgets::num_field`/`styled_number` shared pair (label ABOVE input, `.field label` sizing/color) that replaced Bushing's and Pressure Vessel's own separate, inline-label `num_field` copies | DONE | this pass |
| Six Unicode symbols used throughout the UI (⌀ ✕ ⬔ ✓ ✎ ☽) don't exist in ANY of egui's bundled default fonts (`Ubuntu-Light`/`Hack-Regular`/`NotoEmoji-Regular`/`emoji-icon-font` - confirmed via direct `fontTools` cmap inspection, not assumed) and rendered as tofu boxes app-wide, worst in Bushing/Pressure-Vessel dimension labels (⌀, 10 sites) and the Bushing/Rename/dark-mode-toggle nav icons. Fixed by substituting each for a confirmed-present equivalent (Ø, ×, ■, ✔, 🖊, 🌙) rather than bundling a new font asset - see `CLAUDE.md` for the full table and the "verify glyph coverage before using an uncommon Unicode symbol" lesson | DONE | this pass |
| Windows console-window regression - a blank cmd window opens alongside the real GUI window, and closing it kills the whole process (console-owner-process semantics). `app/src/main.rs` (the dioxus-native predecessor) already carries `#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]` for exactly this reason; `app-egui/src/main.rs` never got it when scaffolded as a new binary target - a real regression a user hit on a real Windows machine. Fixed by porting the same attribute, release-builds-only (debug builds keep the console for stdout/stderr) | DONE | this pass |
| Design System Epic Phase 1: `design/` module (`typography.rs`/`spacing.rs`/`radii.rs`/`shadows.rs`) + bundled fonts. App-wide default typeface switched from egui's bundled `Ubuntu-Light`/`Hack-Regular` to real Inter (proportional, 4 static weights: Regular/Medium/SemiBold/Bold) and JetBrains Mono (monospace, 2 weights) - both OFL-1.1, instanced to static weights from the Google Fonts variable sources via `fonttools varLib.instancer` (`ab_glyph`/epaint has no variable-font axis support) and bundled via `include_bytes!` (fully offline, no CDN). egui's original bundled fonts stay installed as fallback - `CLAUDE.md`'s glyph-coverage note (✔ ⚙ 🖊 🌙 missing from Inter/JetBrains Mono, confirmed via `fontTools` cmap check before adopting) still holds and was re-verified against the new faces, screenshot-confirmed no tofu boxes anywhere. `widgets::card` gained a real elevation shadow (`design::shadows::raised()` on `egui::Frame`'s existing `shadow` field, previously unset) and `card_title`/nav-item/field rounding now read from the named `design::radii`/`design::typography` scale instead of ad hoc literals (same values, now named) | DONE | this pass |
| Design System Epic Phase 2: `design/components.rs` (Button variants, Segmented control, Select, Tooltip, EmptyState, Toast) - all six wired to real call sites, not left as dead code. Every mutually-exclusive chip row app-wide (Search's match mode/exclude scope/group by/results view, Bushing's head type/ID type/end constraint/both countersink modes, PV's closed/open ends) now renders as one bordered `segmented` pill group instead of independent `selectable_label` rects with no shared container - the one genuinely new visual treatment this phase introduces. Every material/index-location `ComboBox` now uses the label-above `select_field` wrapper instead of `ComboBox::from_label`'s label-beside layout, matching every other field in the app. "Run search" is now the real accent-filled Primary button; "Rebuild from scratch" is the real Danger variant (it deletes the existing index - genuinely destructive). Search's "No results yet" text is now the shared `empty_state` component. `ToastQueue` is real (auto-expiring, screenshot n/a - not triggered in this pass's verification since it needs a completed index build) and wired to fire once on the index-build running-\>done edge. Screenshot-verified: Match mode/Group by segments, Index location select, Primary/Danger buttons, empty state | DONE | this pass |
| Design System Epic Phase 3: real light theme (`Tokens::LIGHT` was a `Tokens::DARK` placeholder for several phases - now a genuinely distinct palette, accent/accent_strong deliberately deeper than dark mode's bright cyan for WCAG-AA-ish contrast on a light ground; `visuals()` picks `Visuals::light()`/`dark()` base via a new `Tokens::dark_mode` flag, not just color overrides on a dark base). Icon system: 6 real Lucide vector icons (search/settings/cylinder/copy-check/pencil-line/chart-column, ISC-licensed, `assets/icons/`) replace the tool-rail's Unicode glyphs, rasterized via `resvg`/`usvg`/`tiny-skia` - dependencies `Cargo.toml` declared since early in this crate's history but that no code anywhere had ever actually called into until now. Elevation: `components::tile`/`status_rail` (Stat Tiles, status rail) and the command palette/every ComboBox popup now use the same named `design::shadows` scale `widgets::card` already had, closing the "every card" gap. Screenshot-verified in BOTH themes: nav icons render correctly on dark and light, light theme reads cleanly with correct contrast throughout (segmented control, Danger/Primary buttons, empty state, index-location select) | DONE | this pass |
| Design System Epic Phase 4: command palette extension - 4 new commands (`BuildIndex`/`RebuildIndex`/`OpenLastReport`/`ToggleResultsView`), each delegating to a real, already-existing `SearchTool` method (`trigger_build_index`/`trigger_rebuild_index`, new `open_last_report`/`toggle_results_view`) rather than duplicating logic. Brain-map/Timeline items the epic's Phase 4 also named were already covered by the Fixes tier | DONE | this pass |
| Design System Epic Phase 5: zebra-striped spec tables (Bushing's countersink 3-row spec tables - `zebra_stripe` paints a subtle tint behind odd rows, single-row tables like Bore/Interference are unaffected since striping has no meaning there) and a real fade-in transition on step switch (Bushing/PV `step_content`, `ctx.animate_bool` + `Ui::set_opacity`, keyed per-step so it plays once per step per session rather than every revisit - a disclosed scope cut, not a bug). Focus states were audited via direct `egui::Widgets::style` source read, not guessed: `Response::has_focus()` already routes to the `active` `WidgetVisuals` this app's `theme.rs` already accent-tints - Phase 1's theme work already covers this, no new code needed. High-contrast theme not built - the epic marks it optional ("if still wanted") and nothing in this app's own use signals a need for a 4th/5th theme yet; disclosed as a deliberate scope cut, not dropped silently | DONE | this pass - build/tests clean; the zebra-stripe/fade visuals specifically were verified by code review + successful compilation rather than a fresh screenshot (this pass's live-desktop verification became unreliable mid-session - see session notes - a real index build was accidentally triggered on a harmless, already-indexed folder as a side effect of a missed click, not a data-loss risk) |
| Design System Epic Phase 6 (hardening/verification-only: confirm bundled fonts/icons introduce no network path, profile startup/memory) | OPEN - approved, not yet started |
| Brain-map roadmap features: click-to-open/right-click menu, hover preview of the actual matched line, type-to-highlight filter box, reset/fit-to-view, folder clustering, color-by toggle, export-as-image, remember pinned layouts | OPEN - approved, not yet started |
