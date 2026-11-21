# gpu_viewer/editor/materials/ - the Material Lookup workbench in the native window

> TL;DR: `MaterialsSession` is an `Authoring` over the shared `MaterialLookupState` (`toolboxes::material_lookup`) (query, hits, cursor, marks, compare). Like Search it is a page bench: `Authoring::document()` returns a search bar, the results table and a card with the highlighted material's properties or the comparison of the marked ones, a left **Refine rail** (basis, form, property limits, group) and a bottom **compare basket**. No filtering, sorting, comparing or report text lives here.

## Purpose
Owns: the window front end of Material Lookup (page, Refine rail and basket data, tree of marks, ribbon for the palette, menus), the mapping of clicks to `MaterialsAct`, the property / comparison grids (`view::property_grid`, `view::compare_grid`) as `DocGrid` data.
Does not own: the catalogue, groups, query, sort, basis filter, compare rows and their best marks, the report text (`toolboxes::material_lookup::model`), cursor and mark rules (`MaterialLookupState::{refilter, refilter_from_top, move_cursor, toggle_mark}`), the handbook facts of one material (`widgets::material_detail::{facts, curated_values}`), the page's drawing (`gpu_viewer/gui/document.rs`, `property_grid` for `DocDetail::grid`; the rail and the tray in `gui/document/rail.rs`, the rail's body is `Gui::narrow_block` in `gui/panels.rs`, shared with Search's sidebar).

## Code Map
| Looking for... | Go to |
|---|---|
| Session, `MaterialsAct`, keys, Export, slider values (`document_field`), `Authoring` impl | `mod.rs` |
| The page (`Document`), table rows and columns, `rail` / `basket` / `chips`, property / compare grids, tree, ribbon, menus | `view.rs` |
| Session tests (toolbox filter, sort, basis / group, marks, compare best cells, Export) | `tests.rs` |
| Input-level tests (typed search, header click, double-clicked rows, Compare chip; rail checkboxes, basis pill, row box, basket cross) | `gpu_viewer/window/tests/search_materials.rs` `the_materials_page_searches_sorts_marks_on_double_click_and_compares`, `the_materials_rail_and_basket_answer_clicks` |
| GPU snapshots (properties, facets with a basket, compare, dark / light, narrow) | `gpu_viewer/render/snapshot_tests/model_pages.rs` `materials_snapshot` (`FEA_SNAPSHOT_MATERIALS_OUT=dir`, ignored) |

## Contracts
- **One model.** Rows, values and decimals come from `material_lookup::model` (`format_value`, `density`, `specific_yield_in`, `compare_rows`); the property grid comes from `material_detail::facts` / `curated_values`, the same data the terminal pickers' `material_detail::lines` prints. Never format a material value here that the toolbox formats differently.
- **A changed search starts at the best match** (`refilter_from_top`); every other change keeps the highlighted material where it can (`refilter`).
- **Marks are catalogue indices**, at most `model::MAX_COMPARE`. A row's double click (`DocRow::open`) toggles its mark; Enter marks the highlighted row; with no marks left, Compare turns itself off.
- **No file.** `--gpu-materials` ignores its argument; Export writes `<app data>/reports/material-lookup.txt` and opens it. Tests use `MaterialsSession::with_report(None|Some(tempdir))`, never the app data folder.

- **Facets are the toolbox's.** The rail only draws `model::facet_counts(&Query)`: each option's count is what choosing it would show with the other choices kept (so a basis count ignores the chosen basis, a form count ignores the chosen forms). Group and basis stay one choice each (the chosen option again goes back to All / any basis); forms are many. Form is a class (`model::FORMS`, `form_class`: the first matching word wins, "Bar, forging, tubing" is a forging) over the handbook's ~65 spellings; tubing and "All wrought forms" have no class. A property limit (`Floor`: Fty at least, density at most, Fty/density at least) never passes a material with no value, so the curated typicals drop out under a density limit. Counts are cached per `Query` (`facets_cache`).
- **Sliders report text.** A drag arrives as `document_field(INPUT_FLOOR0 + k, "60")`; the free end (the minimum, or the maximum for density) sends an empty text, which drops the limit. A chip over the table (`DocTable::chips`) or the rail's Clear all takes a choice back.
- **The basket is the marks.** `Document::basket` shows `st.marked` (at most `MAX_COMPARE`) in fixed slots; a row's box (`DocRow::mark`) and a slot's cross both run `ToggleMark`, which also moves the highlight to that row. The rail is left out below 860 px of page width and the tray below 340 px of height.

## Pitfalls
- A click on a row's basket box is decided in the *row's* response (pointer inside `mark_box`), not by a second `ui.interact` over the box: the separate widget never got the click in the window tests.
- After a narrowing is cleared the highlighted material may sit far down the list; the table scrolls it into view one frame later, so a test aims at a row only after an extra `frame()`. `table_row` registers `row` targets only for the first 400 rows.
- `document()` is rebuilt every frame, but its ~1,400 result rows are cached until query, hits, cursor or marks change. The table body is virtualised; cached rows are still cloned each frame.
- The Material Lookup terminal toolbox is gone (2026-11-03), but `toolboxes::material_lookup` state / model / view stay: they are the material picker of the terminal FEA Workbench (`fea_workbench::material_browser`) and the solid editor's materials card.
- Window tests double-click with `gpu_viewer/window/tests/mod.rs`'s `double_click` (a 1 s pause, then two clicks): egui counts a third click within 0.6 s of the click before last as a triple click, and `Response::double_clicked()` answers only a count of exactly two, so two `click()`s right after another click never double-click.
- The table body (`gui/document.rs`, `ScrollArea::show_rows`) places row i at i * (`ROW_H` + item spacing); the rows are packed, so the spacing is zeroed *before* `show_rows`. Zeroed only inside the closure (the state before 2026-11-02), clicks, scroll-to-selected and the scrollbar drifted 10 px per row from the painted rows: a click on a row in view selected one ten rows up and scrolled the list.
- A row's badge (`DocRow::badge`: compare mark here, file type in Search) is drawn before the name, so a name cut at the cell's edge never hides it.
