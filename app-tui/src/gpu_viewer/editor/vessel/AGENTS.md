# gpu_viewer/editor/vessel/ - the Pressure Vessel workbench in the native window

> TL;DR: `VesselSession` is an `Authoring` over `toolboxes::pressure_vessel`'s `PressureVesselModel` (the same rows, checks, minimum-wall solve, material catalog and report), plus a to-scale three-quarter section of the cylinder coloured by its Lamé stress through the wall. No stress is computed here.

## Purpose
Owns: the window front end of Pressure Vessel (inspector cards, tree, ribbon, menus, typed edits, undo, the case file, the material picker card) and `drawing.rs`, the stress contour.
Does not own: any stress, margin, buckling or thickness formula (`pressure-vessel-solver`, reached through `PressureVesselModel::commit_number` / `recompute`, and `stress_at_radius_with_thermal` for the picture), the form's rows and labels (`model::field_rows`, `model::row_label`), value text and pass rule (`report::{display_value, passes}`), margin text and tone (`theme::{fmt_margin, tone_for_margin}`, shared by every toolbox), the report (`report::build_report_text`), the material filter and add-form checks (`material_picker::{MaterialPickerState::visible_indices, AddMaterialForm}`), the window or the cards' drawing (`gpu_viewer/gui`).

## Code Map
| Looking for... | Go to |
|---|---|
| Session, `VesselAct`, `Authoring` impl, typed edits, undo / save / export | `mod.rs` |
| Inspector cards (one per form section, result, checks, minimum wall, through-wall numbers, drawing note), tree, ribbon, menus, status verdict | `cards.rs` |
| "Minimum wall by requirement" table (`requirements_table`), the line run's page (case pills, to-scale drawing with margin curve, table), Requirements / Allowables / Line run / Supports / Point loads / Load cases input cards, Tube / Governing / Reactions result cards | `page.rs` (data types `gpu_viewer/page.rs`, painters `gpu_viewer/gui/page.rs`) |
| Material picker value card (search field 0, add-form fields 1..=6), pick, add | `picker.rs` |
| The picture: revolved wall, end and cut faces, five stress fields (the mesh builder is `editor/drawing.rs`) | `drawing.rs` |
| The saved case (JSON: numbers by name, end condition, material by name, a custom material's properties) | `file.rs` |
| Session tests | `tests.rs` |
| Input-level test (click Wall Thickness, type, open the picker) | `gpu_viewer/window/tests/workbenches.rs` `the_pressure_vessel_workbench_*` |
| GPU snapshots (inputs / results dark and light, narrow, picker) | `gpu_viewer/render/snapshot_tests/analysis_workbenches.rs` `vessel_snapshot` (`FEA_SNAPSHOT_VESSEL_OUT=dir`, ignored) |

## Contracts
- **The window is the only head** (the terminal toolbox was removed 2026-11-03). It wraps `PressureVesselModel` directly. Never re-implement a row, a check or a formula here.
- **A check with no load on it passes** (`report::passes`: margin >= 0, an unbounded margin passes). The verdict, tree and step state all use it.
- A typed value equal to the current one is not an edit (no undo step). Inputs that make no vessel show an empty scene (`drawing::empty`) and the Drawing card says why; never the last picture.
- The picture is a long cylinder drawn `drawing::LENGTH_PER_OD` outer diameters long, only for the picture (the analysis is per unit length). Its fields are `drawing::FIELDS` (hoop first); the session reports `shows_result` true, `geometry_view` false, unit psi.
- The quarter cut faces the opening Isometric view (`editor/drawing.rs` `SWEEP_START`, shared with Fastener Hole).
- **Custom materials persist in `settings-tui-pressure-vessel.json`** (`persistence::config_path`, kept name for existing files; held in `VesselSession::library`) and travel with a case (`VesselFile::custom_material`). A case naming a material this machine lacks and does not describe is reported in the status bar; the rest applies.
- `bench()` is `Static`, `benches()` is empty. Export writes `report::build_report_text` beside the case (`<case>.report.txt`).
- Wall Thickness is nominal. `WallAllowance` in `pressure-vessel-solver::geometry` applies mill undertolerance first, then internal corrosion, at fixed entered OD. Checks and the contour use the remaining wall and enlarged bore. The minimum-wall solve retains its fixed analyzed bore convention; its nominal equivalent includes both allowances and can change OD. It excludes thermal stress and buckling.
- **Requirements and the line run** (case format 4: `requirements`, `material_kind`, `allowables`, `line_run`; older cases load seeded with the default run, also over edited state). `Authoring::page()` is non-empty only while the line run is on: the window draws it over the viewport (left of the inspector) and the 3D vessel is hidden; the cylinder keeps its picture and shows the table in a Results card. Every typed field goes through one `Prompt { edit: Edit }`, applied by `PressureVesselModel::apply_edit` (numbers keep numeric characters, names accept text); each applied edit is one undo step, a switch or a row added or removed is one step (`edit_model`), selecting a case is none. The line wall solve runs on `Worker<Vec<WallOutcome>>` tagged with `walls_revision` (`kick_walls`, started after every change, applied in `poll`).
- **Segment mode** (case format 5: `line_run.segment`; format 4 and older load with it off, a newer file is refused): the Segment card (`page.rs` `segment_card`) cycles each end's condition (`CycleFixity`), toggles axial anchors, adds a valve at an end (`AddEndLoad`) and switches the bound (`ToggleBound`). With the bound the page adds the "Segment bound" table (`bound_table`, pinned / clamped / worse per requirement) and the Results step the Segment bound card; the idealisation statement (`SEGMENT_NOTE`) is shown on the page, the card and the report. See `docs/pressure-vessel-line-run.md`.
- Case format 3 adds the code check's inputs (`head ...` numbers, `head_component`); older cases get `HeadInputs::default()` even over edited state. The "Head / shell code check" card (`cards.rs` `heads_card`) draws `model.heads`; a violated rule shows `report::head_error_text`, never a number.
- Case format 2 adds corrosion allowance (in) and mill undertolerance (%). Format 0/1 cases remain readable with zero loss, including when applied over edited state. Older builds reject format 2 instead of silently ignoring loss.

## Pitfalls
- **Tests must set `s.library = Some(<temp path>)`**: otherwise adding a material writes the person's real `settings-tui-pressure-vessel.json`.
- `PressureVesselModel::default()` reads that library too: a test must not assume the catalog length.
