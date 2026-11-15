# toolboxes/line_run/ - Line Run model (window-only)

> TL;DR: Inputs, display units and the bridge to `pressure_vessel_solver::line_run` for the Line Run window. Pure data, no ratatui. Material constants and allowables are `Option`: never defaulted.

## Purpose
Owns: `Inputs` (what a case file stores), `Units` / `Qty` (display conversion), `Field` (one editable number, table rows by index), the table and toggle edits, `LineRunModel::solve` (builds `LineRun`, `MaterialClass`, `LoadCase`s, runs `analyze`), `report.rs` (report text, station CSV, `location_text`).
Does not own: any action, stress or margin (`pressure-vessel-solver/src/line_run`), the window (`gpu_viewer/editor/line_run/`).

## Code Map
| Looking for... | Go to |
|---|---|
| Inputs, units, `Field`, edits, solve | `model.rs` |
| Report text, station CSV | `report.rs` |
| Tests (closed form through the inputs, SI = US, brittle, case scoping) | `model/tests.rs` |

## Contracts
- **Stored in solver units (in, lbf, psi, degF), shown in `Units`**; a typed value is divided by `Units::factor` once in `set`. The solver never sees display units; `si_entry_converts_once_and_gives_the_same_solve` proves it. Temperature change uses 5/9 (a difference, not an absolute temperature).
- **No default allowable, modulus, expansion or density** (owner rule). A missing one makes `outcome` an `Err` naming it. Do not add a "typical steel" fallback.
- A point load's `case` is `None` (hardware, every case) or `Some(i)`; `remove_case` drops that case's loads and renumbers the rest.
- Moments on point loads and `subdivisions` are not exposed (`PointLoad::moment` is zero; `SUBDIVISIONS` default). Composite / Other classes are not selectable: the solver refuses them.
- `LineRunModel::outcome` is recomputed on every edit (`recompute`); a solve is milliseconds, so there is no worker.
