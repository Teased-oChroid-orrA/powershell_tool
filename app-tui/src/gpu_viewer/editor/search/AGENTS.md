# gpu_viewer/editor/search/ - the Search workbench in the native window

> TL;DR: `SearchSession` is an `Authoring` over the terminal toolbox's own `SearchToolState` (run, index, report, settings, recents, presets). It is a bench without a 3D model: `Authoring::document()` returns a page (search bar, live progress, results table, highlighted matches) that `gui/document.rs` draws where a model would be. No searching, matching or extraction lives here.

## Purpose
Owns: the window front end of Search (page, tree, popovers, value cards, menus), its own tokio runtime and event channel, the results view order (filter / sort / selection), the native folder picker call.
Does not own: running a search, the index, reports or settings persistence (`toolboxes/search`: `run_or_notify`, `apply_event`, `exec::execute`, `persistence`), the settings table (`toolboxes/search/settings_view.rs::settings()`), the page's drawing (`gpu_viewer/gui/document.rs`).

## Code Map
| Looking for... | Go to |
|---|---|
| Session, `SearchAct`, run / cancel / index, `poll`, `Authoring` impl | `mod.rs` |
| The page (`Document`), table rows, detail blocks, tree, popover cards, the Find guide (`find_help`), ribbon (palette only), menus | `view.rs` |
| Session and guide tests (real search on a tempdir, filter / sort / keys, settings, picker, presets, every guide example parses) | `tests.rs` |
| Input-level tests (typed Find + Enter, row click, popovers, no docks) | `gpu_viewer/window.rs` tests `the_search_page_*` |
| Where the two cards go (side by side / stacked, order, split; drop and splitter geometry) | `gpu_viewer/gui/page_layout.rs` (pure, unit-tested); stored as `display::PageLayout` |
| GPU snapshots (results dark / light, narrow, running, guide, options, exclude row with moved cards, exclude guide) | `gpu_viewer/render.rs` `search_snapshot` (`FEA_SNAPSHOT_SEARCH_OUT=dir`, ignored) |

## Contracts
- **One run path for both heads.** Events go through `toolboxes::search::apply_event`, effects through `toolboxes::search::exec::execute`, a run starts through `run_or_notify`. The terminal's `app.rs` / `main.rs` call the same functions; never re-implement a reducer arm or an effect here. Per-file in-flight status (root `CLAUDE.md` hard requirement) is drawn from `run.in_flight_files` every poll.
- **Settings are the terminal's.** `SearchToolState::default()` loads `settings-tui.json`; option rows are `settings_view::settings()` rows (label, hint, parse, kind), so a setting added there appears here. Settings are written on every change and on drop. Tests use `SearchSession::in_memory()`: no load, no save, and a folder picker that answers "cancelled" (never a real dialog).
- **Exclude is a row of the page, not an option.** The `Exclude` chip (`SearchAct::ToggleExclude`) shows a second bar row (`Document::extra_inputs`, `INPUT_EXCLUDE`) with its own guide (`exclude_help`) and a segmented scope choice (`DocInput::choices`: Whole file / Matching lines). Exclude filters and Exclude scope are never in the Options popover. Hiding the row does not turn exclusion off: while terms are set, the chip stays lit and names them, so an applied exclusion is always visible. The row opens shown when the saved settings (or an applied preset) exclude something.
- **The cards move and resize.** Each card's grip (dots + title) drags it to the nearest side of the room (left / right / top / bottom; the drop is previewed), the other card takes the rest; the splitter between them sets the shares (double-click: default). The result is `Command::SetPageLayout`, kept in `display.json` (`DisplaySettings::page_layout`). A narrow room still stacks and a short one still shows the list alone, whatever the stored layout says (`page_layout::split`).
- **No docks.** A page bench gets no tool pill, no step strip and no inspector (`Gui::build` checks `Authoring::has_document`); options live in bar popovers (`Document::panels`: Options, Index, Report), the file-type picker is a modal (`Document::modal`), the Find syntax guide is a popover behind the label link (`DocInput::help`; an example click fills Find). The window skips scene drawing, orbit / zoom / pick and viewer keys in page mode (`Viewer::page_mode`).
- **Both folders are an input plus Browse.** The folder to search (bar) and the output folder (Report popover, `DocPanel::inputs`, `INPUT_OUTPUT`) are typed or chosen with the same picker (`SearchSession::pick`); the output picker starts in the output folder, else the searched one. Output folder is never a settings row or value card (`Edit(14)` and the ribbon open the picker). Empty output folder: the report lands in the working folder, Export matches beside each file.
- **Browse is the OS folder picker** (`rfd`, feature `gpu-viewer`): modal, on the window thread (macOS requires it). It blocks the event loop while open; the background search keeps running and its events are folded in on the next poll.
- The results table shows files with a hit, in found order until a column is clicked; `selected` is an index into `run.results` (append-only during a run), so selection survives new results. The detail shows at most `MAX_BLOCKS` lines; Export matches writes all.
- Both guides must stay true to the parser: `tests::every_example_of_the_exclude_guide_is_a_valid_exclusion` checks the Exclude guide; `tests::every_example_of_the_find_guide_parses_the_way_the_guide_says` parses every example and checks the tag semantics it describes. Change the guide and the parser together.

## Pitfalls
- `Authoring::document()` is rebuilt every frame (all visible rows as strings); the table body is virtualised (`ScrollArea::show_rows`), but a run with tens of thousands of hit files will cost per frame. Measure before adding per-row work.
- Strings from the terminal toolbox may carry glyphs the window font lacks (`⚠`, `…`); pass them through `view::plain`.
- A short window drops the stacked detail (both need `MIN_STACKED`) and the figure tiles; this is deliberate, not a missing feature.
