# gpu_viewer/editor/preload/ - the Preload Analysis workbench in the native window

> TL;DR: `PreloadSession` is an `Authoring` over `toolboxes::preload_analysis`'s `PreloadModel` (the same rows, solve, bolt catalog, joint templates and report), plus a to-scale three-quarter picture of the joint: each member a revolved double frustum coloured by the member finite-element model's stress, the shank in the bore. No stress is computed here.

## Purpose
Owns: the window front end of Preload Analysis (inspector cards, tree, ribbon, menus, typed edits, undo, the case file, the bolt and joint-template picker cards), the member-FE job (debounce, worker, angle sync) and `drawing.rs`, the picture.
Does not own: any preload, torque, stiffness or stress formula (`fastened-joint-solver` through `PreloadModel::commit_number` / `recompute`), the member FE and its field (`toolboxes/preload_analysis/fe_check::run_with_field`, `fea_problem::joint::member_field` / `MemberField::stress_at`), the rows and labels (`model::field_rows`, `NumberTarget::label`), value text, margins and pass rule (`report::{display_value, margins, passes}`), the report (`report::build_report_text`), the bolt catalog (`model::BOLT_SECTIONS`, `is_hi_lok`), the templates and random joints (`joint_templates::{presets, random, random_seed}`), the mesh builder (`editor/drawing.rs`) or the translucent draw (`render.rs`).

## Code Map
| Looking for... | Go to |
|---|---|
| Session, `PreloadAct`, `Authoring` impl, typed edits, undo / save / export, FE job (`poll`, `fe_current`) | `mod.rs` |
| Inspector cards (one per form section, joint stack, result, torque, stress, deformation, rotation, FE cross-check, drawing note), tree, ribbon, menus, status verdict | `cards.rs` |
| Bolt picker (search field 0, sections, current bolt first) and template picker (presets, Random joint) | `picker.rs` |
| The picture: member shells, cut faces, shank; four FE stress fields; part codes before the FE answers | `drawing.rs` |
| The saved case (JSON: numbers by name, members, mode / tightening / bearing by label, switches) | `file.rs` |
| Session tests | `tests.rs` |
| Input-level test (click Applied Torque, type, open the bolt picker) | `gpu_viewer/window.rs` `the_preload_workbench_*` |
| GPU snapshots (parts, inputs / results dark and light, narrow, picker) | `gpu_viewer/render.rs` `preload_snapshot` (`FEA_SNAPSHOT_PRELOAD_OUT=dir`, ignored) |

## Contracts
- **The window is the only head** (the terminal toolbox was removed 2026-11-03). It wraps `PreloadModel` directly; never re-implement a row, a check or a formula here.
- **The FE does the legwork.** One worker solve (`fe_check::run_with_field`) feeds both the cross-check card and the picture; the cone half angle drawn is `FeResult::equivalent_angle_deg`, else the user's. The picture's member stress is `stress_at * preload`; the shank's is the solver's shank section. Until the answer for exactly this joint arrives the scene is a geometry view in part codes (`drawing::KEY_LABELS`: member, washer, bolt, cut face).
- **Draw order is a contract with the renderer**: the shank and every cut face first, the members' revolved surfaces last; `drawing::scene` returns where they start and `Authoring::translucent` draws them at `drawing::MEMBER_OPACITY` (owner choice 2026-11-03: fixed 35%, no control) so the bolt shows through. Cut faces stay solid so the section reads exactly. Hidden-line styles draw everything solid.
- The shank radius is `min(shank/2, 0.98 x the narrowest bore)` so it never coincides with a bore surface.
- The Result headline counts finite margins only (`report::margins`); with none it says "no margins checked", never "0 / 0".
- `bench()` is `Static`, `benches()` is empty. Export writes `report::build_report_text` (with the FE cross-check when current) beside the case.

## Pitfalls
- **Catalog entries are compared by designation, never by address**: `BOLT_SECTIONS` holds copies of the solver crate's static slices, so `std::ptr::eq` against `HI_LOK_CATALOG` never matched (the Hi-Lok note and "thread only" tag silently vanished). Designations are full strings (`"AN6 (.3750-24 UNF)"`), not `"AN6"`.
- Tests wait for the FE with `poll()` (the window polls every 250 ms); a snapshot must wait for `fe_current()` before shooting results.
