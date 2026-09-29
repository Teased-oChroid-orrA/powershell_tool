# Toolbench: multi-tool dashboard shell

`app/` is becoming a dashboard shell ("Toolbench") that hosts multiple
independent tools behind one left-hand tool-switcher rail, not a single-
purpose search window - the search app (this repo's whole reason for
existing) is the first, fully-functional tool inside it. Built from a
reviewed artifact preview (a static HTML/CSS/JS mockup, approved before
any real code changed) - see "What shipped" below for what carried over
exactly and what didn't.

## Architecture

- `app/src/main.rs`'s `App()` now renders `.app-shell` > `.shell` (a flex
  row) > `.rail` + `.main`, instead of the old flat `.title-bar` +
  `.main-grid` stack. `.rail` is the tool switcher (brand mark, nav list,
  add-tool stub, theme toggle - moved here from the old title bar).
  `.main` is `.topbar` (active tool's title/subtitle + the command-
  palette trigger) + `.stage` (the active tool's content).
- `ToolId` (`main.rs`) is a plain enum (`Search`, `Dupes`, `Rename`,
  `Logs`) behind a runtime-only `Signal` - not persisted. The mockup this
  was built from didn't demonstrate remembering the last-open tool across
  a relaunch, and defaulting to `Search` (the one real tool) on every
  launch is the more predictable behavior anyway; add persistence later
  if that changes.
- `Search`'s content is the exact same `SettingsPanel`/`ResultsPanel`/
  `PreviewPane` three-pane resizable layout this app already had -
  moved inside `.stage`, not rebuilt. Every existing feature (fast
  re-search index, OCR toggle, drag-and-drop, command palette, context
  menu, filesystem watching) is unchanged.
- The other three tools (`Dupes`/`Rename`/`Logs`) render `PlaceholderTool`
  - a small shared component (title/description/icon props) showing a
  "Coming soon" pill and description, not three duplicated blocks of
  markup. Nothing behind them is implemented; clicking their nav item
  only swaps `.stage`'s content, same interaction as a real tool.
- Icons are hand-written inline SVG (`icon_search`/`icon_dupes`/
  `icon_rename`/`icon_logs`/`icon_plus`/`icon_sun`/`icon_moon`/
  `icon_brand` in `main.rs`), matching this app's existing "no icon-font/
  sprite-sheet dependency" approach - a handful of paths costs nothing
  extra to bundle. The brand mark (rail header) and theme toggle
  (previously plain text "GS"/☀/☾) were upgraded to real vector icons in
  the same pass - not just new icons for new nav items.

## What shipped exactly as the reviewed mockup showed

- Rail layout: brand block (mark + "Toolbench" / "GS Engineering"), a
  "Tools" section label, four nav items (Search Files active by default;
  Duplicate Finder/Batch Rename/Log Analyzer each with a "Soon" pill), an
  "Add tool" stub button, and the theme toggle - all in the same order,
  same visual hierarchy, same copy as the artifact preview.
- Topbar shows the active tool's real title + one-line description,
  swapping live as the rail selection changes.
- Placeholder tools: same icon, same "Coming soon" pill, same title, same
  description copy as the mockup, verbatim.
- Color tokens are the app's own existing `--bg`/`--accent`/`--glass-*`
  etc. custom properties (`.app-shell[data-theme="dark"/"light"]`) -
  the mockup was itself built from these exact values in the first place
  (see the artifact's own CSS comment), so no new palette was introduced
  anywhere in this pass.

## What's deliberately not in this pass

- The "Add tool" button is inert (no click handler) - the mockup didn't
  specify what it should do (a picker? a plugin system?), and guessing
  that shape wasn't part of the reviewed preview.
- `Dupes`/`Rename`/`Logs` have zero real logic - purely the placeholder
  card. Building any of them is a separate, future task each, not
  implied by "match the mockup."
- No persistence of which tool was last open (see `ToolId`'s doc comment
  above for why).

## A third dashboard shell: `app-tui/` (ratatui/crossterm)

A terminal-UI Toolbench dashboard (`app-tui/`, new root-workspace member)
is being built alongside `app/` and `app-egui/`, not replacing either -
same rail/topbar/workspace/status-bar shell shape as this document
describes for `app/`, reimplemented for a terminal renderer. As of this
writing:

- **Search Files**, **Fastener Holes**, **Bushing Workbench**, **Pressure
  Vessel Analyzer**, and **Preload Analysis** are the migrated toolboxes.
  Search Files has live progress (aggregate percent + per-file in-flight
  status, matching `app/`'s full `SearchProgressReport` fidelity, not
  `app-egui`'s thinner subset), incremental results, preview with match
  highlighting, cancellation, HTML/CSV/JSON export, per-result actions
  (open/copy path/reveal folder/export hits), and fast re-search indexing
  (native-search/Tantivy-backed, including a per-extension checkbox
  catalog seeded by scanning the actual search folder - a capability
  neither existing head has). Fastener Holes is a regular/countersunk
  fastener-hole design and analysis tool unique to this crate (no
  equivalent in `app`/`app-egui`). Bushing Workbench is ported from
  `app`'s/`app-egui`'s own `bushing-solver`-backed tool (full straight/
  flanged/countersunk geometry, tolerance-stack, margin-of-safety, install-
  force, and aircraft reamer catalog) minus their cross-section sketch -
  visual presentation with no terminal equivalent - plus a full per-radius
  hoop/radial/axial stress-field breakdown neither existing head exposes as
  plain numbers. Pressure Vessel Analyzer is ported from `app`'s/
  `app-egui`'s own `pressure-vessel-solver`-backed tool (full geometry/
  pressure/material/buckling inputs, full failure-mode and minimum-
  thickness results) minus their cross-section sketches and KaTeX
  derivation view - visual presentation with no terminal equivalent.
  Preload Analysis is a fastened-joint installation/preload mechanics
  solver (`fastened-joint-solver`, a new solver crate) unique to this
  crate - full V-thread torque equation, uniform-pressure/uniform-wear
  bearing friction (both cross-checked against numerical quadrature),
  Brent-root-solved torque-preload equilibrium, piecewise-exact fastener
  axial/torsional compliance plus a numerically-integrated Rotscher
  pressure-cone member compliance, nut rotation/torsional twist, full
  stress state at three sections, service-load/separation/slip checks, and
  both spec-required uncertainty engines (deterministic worst-case corner
  search and seeded Monte Carlo sampling, over the same five tolerance
  bounds), the advanced per-thread spring-coupled load-distribution solve
  (opt-in, validated against the continuum closed-form solution), separate
  head-side/nut-side bearing geometry (`Tightening From` now genuinely
  changes the solve), embedment/settlement, and a thread-shear margin check
  - one deliberate cut remains (locking-feature prevailing torque as a
  function of rotation, labeled a future enhancement in the toolbox's own
  originating spec - see `fastened-joint-solver/src/lib.rs`'s doc comment).
  A sectioned bolt picker covers five sourced catalogs - AN3-AN20, NAS
  tension/shear bolts, NAS machine bolts, MS21250, and Hi-Lok HL18 pins
  (UNJ thread form, reduced major diameter) - auto-filling thread geometry
  from the correct ASME B1.1 (UN/UNF) or ASME B1.15 (UNJ) basic-dimension
  formulas per catalog; Bushing Workbench's own Bore Diameter field is
  similarly reamer-catalog-driven (`toolboxes/bushing/reamer_picker.rs`)
  rather than a free-typed decimal with a separate "nearest reamer" lookup.
  All four engineering toolboxes' (Bushing/Fastener Holes/Pressure Vessel
  Analyzer/Preload Analysis) field lists are grouped under section headers
  with a bottom per-field "Hint" panel, dynamically sized to the wrapped
  hint text; Search Settings keeps its existing flat, tab-grouped field
  list but gained the same per-field Hint panel. All five toolboxes are
  unit/integration-tested.
- Duplicate Finder, Batch Rename, and Log Analyzer are inert rail
  placeholders only - nothing behind them is implemented yet.
- Both keyboard and mouse navigation are supported (click/scroll on the
  rail, workspace panes, results/field rows, and every modal).

See `app-tui/AGENTS.md` for the crate's own architecture (`AppState`/
`AppEvent`/`Effect` event-reducer pattern, module map, testing approach).
This does not change either `app/`'s or `app-egui/`'s status above - both
remain fully active, shipping heads.
