# gpu_viewer/editor/lug/ - the Lug Analysis workbench in the native window

> TL;DR: `LugSession` is an `Authoring` over the toolbox's own `LugUiModel` (the same rows, solve, checks, mesh-size test and report), plus the lug flat in the xy plane on the kernel's finite-element mesh, contoured by the nodal field the solve read its results from. No stress is computed here.

## Purpose
Owns: the window front end of Lug Analysis (inspector cards, tree, ribbon, menus, typed edits, undo, the case file, the material pickers), the analysis job (settle delay, worker, stale answers, failure memory), the mesh-test job and `drawing.rs`, the picture.
Does not own: any lug formula or check (`lug-solver` through `toolboxes/lug_analysis/model::{run, evaluate_with}`), the rows, labels, value text and hints (`model::{field_rows, row_label, display_value, field_hint}`, `NumberTarget::label`, `LugUiModel::validation_hint`), the report (`model::report_text`), the mesh-size test (`mesh_test::{run, signature}`), the material catalog and its search (`material_lookup::model::{catalog, filter_sort}`), the field (`lug_solver::LugField`, `FeaLug::analyze_field`) or the mesh builder (`editor/drawing.rs`).

## Code Map
| Looking for... | Go to |
|---|---|
| Session, `LugAct`, `Authoring` impl, typed edits, undo / save / export, analysis and mesh-test jobs (`poll`, `current_run`) | `mod.rs` |
| Inspector cards (one per form section, Result, Contact, Stress, Ultimate, Finite, Bushing, Pin bending, Comparison, Notes, Bore profile, Load-travel curve, Model, Drawing), tree, ribbon, menus, status verdict | `cards.rs` |
| Lug / bushing material picker (search field 0, 12 shown, current first) | `picker.rs` |
| The picture: Q9 -> 8 triangles, half model mirrored; von Mises / hoop / displacement; part codes before the field | `drawing.rs` |
| The saved case (JSON: numbers by key, materials by name, choices by label, switches, overrides) | `file.rs` |
| Session tests | `tests.rs` |
| Input-level test (click Pin Load P, type, open the material picker) | `gpu_viewer/window.rs` `the_lug_workbench_*` |
| GPU snapshots (parts, inputs / results dark and light, narrow, picker) | `gpu_viewer/render.rs` `lug_snapshot` (`FEA_SNAPSHOT_LUG_OUT=dir`, ignored) |

## Contracts
- **One model.** The window wraps `LugUiModel` directly; never re-implement a row, a check or a formula here.
- **The picture is the solve's field.** `LugRun::field` is `Some` on the kernel path only; the legacy solver gives none and the picture stays the parts (lug, bushing, pin at its contact side, clamped end: `drawing::KEY_LABELS`). vm, hoop (about the hole centre) and |u| are mirror-invariant, so a mirrored node copies its original's values; nodes on `y = 0` are shared and the mirrored triangles are wound the other way.
- Geometry facts: the hole centre is the origin, an axial load (angle 0) points to -x, the clamped end is at x = model length, a half model is y >= 0, the Front view looks from +z.
- **Jobs**: a job for older inputs is never cancelled (its `CachedModel` is wanted); an answer is applied only if its input equals the inputs shown, the cache is kept either way. A failed input is not retried until an edit. A new job starts once the inputs have sat unchanged 250 ms (the first at once).
- A typed number is committed on a clone first; if the model keeps the old value the status says "<label> not changed: v is out of range." and no undo step is made. Toggling the Advanced settings row (`MeshSection`) is not an edit.
- `bench()` is `Static`, `benches()` is empty. Export writes `model::report_text` of the current run to `<case>.report.txt`. No translucency (owner: Preload cone only).

## Pitfalls
- Tests and snapshots must poll (`poll()`) until `current_run()` is in before asserting results; the kernel run takes seconds in debug (the session tests turn the plastic collapse off to stay fast).
- Repeated labels (e.g. "Bushing" is a section title and a row) make `target(&v, "X")` hit the first; use `find_target_after("<card>", "<label>")` in window tests.
