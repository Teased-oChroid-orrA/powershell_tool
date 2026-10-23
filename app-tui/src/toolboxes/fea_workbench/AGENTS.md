# toolboxes/fea_workbench/ — FEA Workbench toolbox

> TL;DR: Define a finite-element problem, see its mesh and solve it on the general kernel. A field list (template, analysis, material, sketch with holes / extrusion or an imported Gmsh / Abaqus mesh, mesh settings, supports, loads) beside a canvas (mesh preview, then a colour contour of any result field, optional deformed shape) and a results readout. The mesh preview and, for a small model, the solve re-run on a worker a moment after any input stops changing. Problems save as JSON; results export as a text report, `.vtu` (ParaView) and a nodal `.csv`.

## Purpose
Owns: UI state and key routing, the field rows and their edits (`model.rs`), the worker-job protocol (ids, debounce, stale-result drop), the canvas and its colour ramp, rendering.
Does not own: the problem schema, meshing, solving, results, rasterising or the report (`fea-problem`), any mechanics (`fea-core`), material data (`mechanics-core` through the Material Lookup browser).

## Code Map
| Looking for... | Go to |
|---|---|
| Rows (`FieldRow`), labels, values, hints, number get / set, polygon / hole helpers | `model.rs` |
| `FeaWorkbenchState`, `activate` (what Enter / Space does per row), editing, `tick`, `start_solve`, `finish_*`, `file_read`, `handle_key` | `mod.rs` |
| Colour ramp, half-block painting, legend colours | `canvas.rs` |
| Layout, field list, canvas (raster cache), readout | `view.rs` |
| Behaviour tests (jobs, edits, files, Caps Lock) | `tests.rs` |

## Bushings (interference-fit contact)
- A `Bushings` section (plane problems with a circular hole): `Add Bushing` presses a default steel bushing into the first circular hole that has none, then one row per parameter (hole number, bore diameter, offset x / y, diametral interference, friction, E, nu) and `Remove`. Removing a hole drops its bushing and renumbers the later ones (`model::forget_hole`). The solve is a contact solve (seconds): the automatic solve skips a bushed problem and it runs on `r`; the results show each bushing's fit pressure (mean / peak), friction torque capacity and open arc, and the equilibrium warning threshold is 1e-3 for a contact problem (1e-6 linear).

## Contracts
- Effects: `RunFeaPreview` (mesh only, key = problem without supports / loads / name, `preview_key`), `RunFeaSolve`, `WriteTextFile` / `WriteTextFileAndOpen`, `ReadTextFile { purpose }`. Events carry a job id; a result whose id is no longer the live job is dropped (`finish_preview` / `finish_solve`).
- `tick` (every `Tick` while the toolbox is active): validate geometry (error shown, nothing started) -> **nothing meshes or solves by itself** (ADR-016): `u` builds the mesh preview once (`mesh_requested`), `r` solves any size (the worker meshes on its own), an edit leaves the mesh and result "out of date". `a` turns on **automatic mode** (`auto_solve`, default off): then a preview starts when the mesh key changed and the inputs settled for 300 ms (the first one at once) and a solve starts when the preview is current, the mesh has at most `AUTO_SOLVE_ELEMENTS` elements and these exact inputs have not failed. A failed solve is not retried until an edit. Tests that need the automatic behaviour build `auto_state()`.
- The canvas shows the solved contour while the result matches the inputs (or no mesh exists), else the mesh preview; a stale result keeps a warning line in Results. The raster is cached in a `RefCell` keyed by size, field, deform flag and `source` (bumped whenever a mesh or result arrives).
- **Natural frequencies and buckling** (`n`, `b`): `fea_problem::dynamics::run` on a worker (`Effect::RunFeaDynamic` / `AppEvent::FeaDynamicFinished`, same job-id protocol as the solve): the first 6 frequencies (needs `Mass Density` in consistent units: inch / psi -> lbf s^2/in^4 = weight density / 386.09) or the first 3 positive load factors of the problem's loads. The canvas then draws the displacement magnitude of the selected mode (`[` `]` pick it, `x` deforms the shape) and Results lists them with the mass share per axis. The display belongs to the inputs it was computed for (`dynamic_shown`: an edit hides it, `r` returns to the static result). Bushed problems and axisymmetric buckling are refused with the reason. Checked in a real terminal (tmux): the cantilever template shows 319 Hz for mode 1 (Euler-Bernoulli 321 Hz) and an axial mode at 4978 Hz (closed form 4972 Hz).
- Keys (both cases, Windows Caps Lock): `u` mesh, `r` solve, `n` natural frequencies, `b` buckling, `[` `]` mode, `a` automatic mode (mesh and solve by themselves), `v` field, `m` mesh lines, `x` deformed shape, `d` full report in Results, `e` report, `p` `.vtu`, `c` nodal results `.csv`, `j` save JSON, `o` open JSON (path prompt; Tab browses directories and JSON files, Esc cancels), PageUp / PageDown scroll Results. Files go to `<app data>/fea-workbench/<problem-name>.{json,txt,vtu,csv}`.
- Support components are numbers or `free`: Space toggles Free / Fixed 0, typing a number prescribes it. Edge rows cycle through `FeaWorkbenchState::names()` (the sketch's edges, or the imported mesh's set and surface names once previewed).

## Mesh and material controls
- Rows added with the epic #25 work (all default to the earlier behaviour): `Mesh Strategy` (also for extrusions: Hex + Wedge), `Auto Size Features` (`mesh.predict_sizes`), `Adaptivity Driver` (ZZ / Equilibrated / Goal-oriented), `Bond Regions` and `Region k Material` (a sketch with extra regions; Enter opens the material browser with `material_target` set), `Material Model` / `Hardening Modulus` (elastic-plastic; hidden with bushings or thermal studies). `n` refines frequencies on the eigenvalue estimate when `Adaptive Passes > 0`. The readout shows the plastic summary; the result has an `Equivalent plastic strain` field.

## Pitfalls
- Number rows accept `e` / `E` / `+` so `2.9e7` can be typed; text rows (mesh path) accept any graphic character.
- `Source` toggles keep the sketch in `saved_geometry` so going back loses nothing; the imported file's text lives in `import` and is matched to the path.
- Templates replace the whole problem (`j` saves first); `set_problem` keeps the view flags (automatic mode, field, mesh, deform).
- Real-terminal check done with `tmux` (170 x 50): true-colour half blocks render; a `TestBackend` test (`tests/rendering.rs`) guards that a contour paints many colours and that no size panics.

## Navigation
Parent: `app-tui/AGENTS.md`. Library: `fea-problem/AGENTS.md`, kernel: `fea-core/AGENTS.md`.

## Navigation addendum — GPU viewer (2026-10-10)

`f`/`F`: native frame/truss member editor (`--gpu-editor`, file `member-model.json` in the workbench folder; see `app-tui/AGENTS.md`).
`g`, `t` and `w` (any case) all open the native solid editor on the current problem (it runs static / modal / buckling /
transient studies from its Simulation menu; `start_animation` / `open_gpu` remain only as unreachable-from-keys helpers kept
by their tests, candidates for deletion with `Effect::OpenGpuScene`). `n`/`b` still compute modes for the text report (`native-solid.json`; closing reads the edits back). Shared editing core for both heads: `model.rs`
(`apply_row_action`, `commit_edit`, `edit_start_text`, `toggle_source`, `apply_material`, `move_selection`).
`src/gpu_viewer/` implements scene conversion/rendering/transient preparation; `main.rs` executes
`RunFeaAnimation` and `OpenGpuScene`. Controls and physics limits: `docs/issue-12-phase-18.md`.

## Navigation addendum — streamed generation/native controls (2026-10-10)

Generation stages and actual step progress: `gpu_viewer/progress.rs`, `FeaAnimationProgress` events
and `view::readout_lines`. Launch stays visible until the matching native readiness event.
`gpu_viewer/transport.rs` owns bounded packed companion streams with legacy JSON compatibility;
`scene::ViewerProject` transfers all computed modes, and the native window switches them on one
graphics device. `t` streams bounded display frames while retaining all 240 numerical steps.
Implementation and remaining interactive editor/ML scope: `docs/issue-12-phase-19.md`.
