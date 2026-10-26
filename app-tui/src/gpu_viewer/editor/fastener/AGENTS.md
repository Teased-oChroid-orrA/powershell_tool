# gpu_viewer/editor/fastener/ - the Fastener Hole workbench in the native window

> TL;DR: `FastenerSession` is an `Authoring` over the terminal toolbox's own `FastenerHoleState` (the same model, form rows, results and report), plus a to-scale three-quarter-section drawing of the hole. No dimension is computed here.

## Purpose
Owns: the window front end of Fastener Hole (inspector cards, tree, ribbon, menus, typed edits, undo, the case file) and `drawing.rs`, the picture of the nominal hole.
Does not own: any fit, countersink, area or tolerance formula (`toolboxes/fastener_hole/domain/`, reached only through `FastenerHoleModel::commit_number` / `recompute`), the form's rows and labels (`model::field_rows`, `view::row_label`, `view::display_value`), the report text (`view::build_report_text`), the window or the cards' drawing (`gpu_viewer/gui`).

## Code Map
| Looking for... | Go to |
|---|---|
| Session, `FastenerAct`, `Authoring` impl, typed edits, undo / save / export | `mod.rs` |
| Inspector cards (one per form section, results, drawing note), tree, ribbon, menus, status verdict | `cards.rs` |
| The picture: revolved wall profile per part, plate size, colour codes and key labels | `drawing.rs` |
| The saved case (JSON: choices by label, each dimension `[min, nominal, max]`) | `file.rs` |
| Input-level test (click a dimension, type, switch hole type) | `gpu_viewer/window.rs` `the_fastener_hole_workbench_*` |
| GPU snapshots (regular, countersink dark / light-wireframe, Report step) | `gpu_viewer/render.rs` `fastener_snapshot` (`FEA_SNAPSHOT_FASTENER_OUT=dir`, ignored) |

## Contracts
- **One model for both heads.** Rows come from `model::field_rows` (only input dimensions; the solved countersink dimension is never a row), choices cycle through `FastenerHoleState::toggle_selected`, numbers go through `FastenerHoleModel::commit_number`. Never re-implement a row, a cycle or a formula here. Every calculated value is shown `nominal -minus/+plus` (`model::format_*_band`), labelled with its origin (calculated / derived / transferred / preserved) as the terminal readout tags it.
- A typed value equal to the current one is not an edit (no undo step). A value that parses but makes no valid band (`commit_number` leaves the dimension unchanged) is reported in the status bar and leaves no undo step.
- **The drawing is the nominal hole, to scale; the plate is not an input.** Regular: two stacked parts (Hole 1 above, Hole 2 below, a small gap). Countersunk: the solved primary countersink over its straight hole. Plate size and thickness are picked from the hole only for the picture (the Drawing card says so). The secondary countersink is not drawn. A case that cannot be drawn shows an empty scene (`drawing::empty`), never the last picture, and the Drawing card says why.
- The quarter cut faces the opening Isometric view (`SWEEP_START`); change the two together. Surfaces are shared-vertex grids (`Mesh::grid`) so the window's feature edges are only the real outlines; separate quads per facet would draw every facet edge.
- Colour codes 0..3 map to `key_labels` per hole type. Code 0 (the parts) is not painted in the Wireframe / hidden-line styles (`fs_highlight`), by the viewer's design.
- `bench()` is `Static`, `benches()` is empty. Export writes `view::build_report_text` beside the case (`<case>.report.txt`), the same text as the terminal toolbox's `e`.

## Pitfalls
- The inspector's cards are rebuilt every frame: use `drawing::layout` (no mesh) for the Drawing card, never `drawing::scene`.
- The terminal toolbox's `NumberPart` rows depend on the tolerance mode; a test that edits a `TolPlus` row must be in Nominal +/-Tol mode, a `Min` row in Min / Max mode.
