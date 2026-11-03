# gpu_viewer/editor/materials/ - the Material Lookup workbench in the native window

> TL;DR: `MaterialsSession` is an `Authoring` over the shared `MaterialLookupState` (`toolboxes::material_lookup`) (query, hits, cursor, marks, compare). Like Search it is a page bench: `Authoring::document()` returns a search bar, the results table and a card with the highlighted material's properties or the comparison of the marked ones. No filtering, sorting, comparing or report text lives here.

## Purpose
Owns: the window front end of Material Lookup (page, tree of groups and marks, ribbon for the palette, menus), the mapping of clicks to `MaterialsAct`, the property / comparison grids (`view::property_grid`, `view::compare_grid`) as `DocGrid` data.
Does not own: the catalogue, groups, query, sort, basis filter, compare rows and their best marks, the report text (`toolboxes::material_lookup::model`), cursor and mark rules (`MaterialLookupState::{refilter, refilter_from_top, move_cursor, toggle_mark}`), the handbook facts of one material (`widgets::material_detail::{facts, curated_values}`), the page's drawing (`gpu_viewer/gui/document.rs`, `property_grid` for `DocDetail::grid`).

## Code Map
| Looking for... | Go to |
|---|---|
| Session, `MaterialsAct`, keys, Export, `Authoring` impl | `mod.rs` |
| The page (`Document`), table rows and columns, property / compare grids, tree, ribbon, menus | `view.rs` |
| Session tests (toolbox filter, sort, basis / group, marks, compare best cells, Export) | `tests.rs` |
| Input-level test (typed search, header click, double-clicked rows, Compare chip) | `gpu_viewer/window.rs` `the_materials_page_searches_sorts_marks_on_double_click_and_compares` |
| GPU snapshots (properties and compare, dark / light, narrow) | `gpu_viewer/render.rs` `materials_snapshot` (`FEA_SNAPSHOT_MATERIALS_OUT=dir`, ignored) |

## Contracts
- **One model.** Rows, values and decimals come from `material_lookup::model` (`format_value`, `density`, `specific_yield_in`, `compare_rows`); the property grid comes from `material_detail::facts` / `curated_values`, the same data the terminal pickers' `material_detail::lines` prints. Never format a material value here that the toolbox formats differently.
- **A changed search starts at the best match** (`refilter_from_top`); every other change keeps the highlighted material where it can (`refilter`).
- **Marks are catalogue indices**, at most `model::MAX_COMPARE`. A row's double click (`DocRow::open`) toggles its mark; Enter marks the highlighted row; with no marks left, Compare turns itself off.
- **No file.** `--gpu-materials` ignores its argument; Export writes `<app data>/reports/material-lookup.txt` and opens it. Tests use `MaterialsSession::with_report(None|Some(tempdir))`, never the app data folder.

## Pitfalls
- `document()` is rebuilt every frame for all ~1,400 rows; the table body is virtualised, but adding per-row work costs on every frame.
- The Material Lookup terminal toolbox is gone (2026-11-03), but `toolboxes::material_lookup` state / model / view stay: they are the material picker of the terminal FEA Workbench (`fea_workbench::material_browser`) and the solid editor's materials card.
- Window tests double-click with `window.rs` tests' `double_click` (a 1 s pause, then two clicks): egui counts a third click within 0.6 s of the click before last as a triple click, and `Response::double_clicked()` answers only a count of exactly two, so two `click()`s right after another click never double-click.
- The table body (`gui/document.rs`, `ScrollArea::show_rows`) places row i at i * (`ROW_H` + item spacing); the rows are packed, so the spacing is zeroed *before* `show_rows`. Zeroed only inside the closure (the state before 2026-11-02), clicks, scroll-to-selected and the scrollbar drifted 10 px per row from the painted rows: a click on a row in view selected one ten rows up and scrolled the list.
- A row's badge (`DocRow::badge`: compare mark here, file type in Search) is drawn before the name, so a name cut at the cell's edge never hides it.
