# gpu_viewer/editor/vessel/ - the Pressure Vessel workbench in the native window

> TL;DR: `VesselSession` is an `Authoring` over the terminal toolbox's own `PressureVesselModel` (the same rows, checks, minimum-wall solve, material catalog and report), plus a to-scale three-quarter section of the cylinder coloured by its Lamé stress through the wall. No stress is computed here.

## Purpose
Owns: the window front end of Pressure Vessel (inspector cards, tree, ribbon, menus, typed edits, undo, the case file, the material picker card) and `drawing.rs`, the stress contour.
Does not own: any stress, margin, buckling or thickness formula (`pressure-vessel-solver`, reached through `PressureVesselModel::commit_number` / `recompute`, and `stress_at_radius_with_thermal` for the picture), the form's rows and labels (`model::field_rows`, `model::row_label`), value text and margin tone (`report::{display_value, fmt_margin, tone_for_margin, passes}`), the report (`report::build_report_text`), the material filter and add-form checks (`material_picker::{MaterialPickerState::visible_indices, AddMaterialForm}`), the window or the cards' drawing (`gpu_viewer/gui`).

## Code Map
| Looking for... | Go to |
|---|---|
| Session, `VesselAct`, `Authoring` impl, typed edits, undo / save / export | `mod.rs` |
| Inspector cards (one per form section, result, checks, minimum wall, through-wall numbers, drawing note), tree, ribbon, menus, status verdict | `cards.rs` |
| Material picker value card (search field 0, add-form fields 1..=6), pick, add | `picker.rs` |
| The picture: revolved wall, end and cut faces, five stress fields (the mesh builder is `editor/drawing.rs`) | `drawing.rs` |
| The saved case (JSON: numbers by name, end condition, material by name, a custom material's properties) | `file.rs` |
| Session tests | `tests.rs` |
| Input-level test (click Wall Thickness, type, open the picker) | `gpu_viewer/window.rs` `the_pressure_vessel_workbench_*` |
| GPU snapshots (inputs / results dark and light, narrow, picker) | `gpu_viewer/render.rs` `vessel_snapshot` (`FEA_SNAPSHOT_VESSEL_OUT=dir`, ignored) |

## Contracts
- **One model for both heads.** The terminal `PressureVesselState` is not used; the window wraps `PressureVesselModel` directly. Never re-implement a row, a check or a formula here.
- **A check with no load on it passes** (`report::passes`: margin >= 0, an unbounded margin passes). The verdict, tree and step state all use it.
- A typed value equal to the current one is not an edit (no undo step). Inputs that make no vessel show an empty scene (`drawing::empty`) and the Drawing card says why; never the last picture.
- The picture is a long cylinder drawn `drawing::LENGTH_PER_OD` outer diameters long, only for the picture (the analysis is per unit length). Its fields are `drawing::FIELDS` (hoop first); the session reports `shows_result` true, `geometry_view` false, unit psi.
- The quarter cut faces the opening Isometric view (`editor/drawing.rs` `SWEEP_START`, shared with Fastener Hole).
- **Custom materials persist where the terminal toolbox keeps them** (`persistence::config_path`, held in `VesselSession::library`) and travel with a case (`VesselFile::custom_material`). A case naming a material this machine lacks and does not describe is reported in the status bar; the rest applies.
- `bench()` is `Static`, `benches()` is empty. Export writes `report::build_report_text` beside the case (`<case>.report.txt`).

## Pitfalls
- **Tests must set `s.library = Some(<temp path>)`**: otherwise adding a material writes the person's real `settings-tui-pressure-vessel.json`.
- `PressureVesselModel::default()` reads that library too: a test must not assume the catalog length.
