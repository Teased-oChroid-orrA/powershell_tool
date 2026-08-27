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
