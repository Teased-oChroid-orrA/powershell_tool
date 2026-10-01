# toolboxes/bushing/ — Bushing Workbench toolbox

> TL;DR: Thin bridge over `bushing-solver`/`mechanics-core`; `model.rs` computes nothing itself. Adds material picker, reamer catalog picker (primary way to set Bore Diameter), friction picker, Bushing ID library, fit-type selector. Libraries use `src/library.rs`.

## Purpose
Owns: UI state/key routing/rendering, pickers, and library wiring (reamer, material, Bushing ID) for the bushing solver.
Does not own: any Lamé/press-fit/tolerance math (`bushing-solver/AGENTS.md`).

## Code Map
| Looking for... | Go to |
|---|---|
| pure UI-facing state bridging `bushing-solver` (geometry/tolerance/countersink/enforcement/assembly-thermal inputs, full `BushingOutput` including the per-radius stress field) | `src/toolboxes/bushing/model.rs` |
| `BushingState`, toolbox-local key routing (`d` numbers panel, `e` export) | `src/toolboxes/bushing/mod.rs` |
| rendering (field list + results readout, wide/narrow layout, plain-text report builder) | `src/toolboxes/bushing/view.rs` |
| housing/bushing material picker | `src/toolboxes/bushing/material_picker.rs` |
| full filterable aircraft reamer catalog (primary way to set Bore Diameter - `m` for manual numeric entry) | `src/toolboxes/bushing/reamer_picker.rs` |

## Entry Points
| Task | Start Here |
|---|---|
| Change a Bushing Workbench formula (Lamé/press-fit, tolerance-stack, countersink, bearing/edge-distance, reamer catalog) | `bushing-solver/` (NOT this crate — `toolboxes/bushing/model.rs` only bridges that crate's existing public API into UI state; see `bushing-solver/AGENTS.md`'s own differential-test discipline before changing a formula there) |
| Add/change a Bushing Workbench editable field | `src/toolboxes/bushing/model.rs` (`NumberTarget`/`field_rows`) - a dynamic row list gated by `BushingType`/`IdType`/`CsMode`/enforcement/assembly-thermal state, same pattern as Fastener Holes' `field_rows` |

## Contracts
- Picker-backed `Number` rows (Bore Diameter, Friction, Bushing ID: `Enter` opens a catalog/library) must be excluded from type-to-start-editing in `bushing::handle_key`; see parent Contracts. Add any new picker-backed row to that same exclusion list.
- Material inputs reach the solver as resolved `Material` values (see `bushing-solver/AGENTS.md`); resolve built-in vs library/custom materials in `model.rs` before solving.

## Pass/fail checks and recommendations (`advice.rs`)
- `BushingModel::recompute` also fills `checks` (Pass/Warn/Fail per `CheckKind`) and `recommendations`; the Results pane highlights a failing check's name+value (red, `✗`) and the related input rows. Recommendations are **not** listed inline: Results has a fixed action bar (`Fixes (N)`, `Why OD clamped?`, `Numbers`, `Export` - all mouse buttons) and flagged lines end in `▸` (click to open that check's fixes). `f`/`a` opens the modal Fixes window (`BushingState::advice`; tabs Fixes / Explain, `w` opens Explain); inside it Up/Down select, Enter/`a`/Apply applies the selected fix to the real input fields and jumps the cursor to the first changed one, Esc/Close dismisses. Mouse and keys share `BushingState::perform(BushingAction)`; click regions are published as `MouseRegions::bushing_actions` (window-only while it is open; `scroll_paragraph::render_interactive` maps wrapped/scrolled lines to rects). `advice::explain_tolerance` builds the OD-clamped/infeasible explanation from the live numbers (heading/body pairs).
- Every recommendation is **verified by trial-solving a clone** (`BushingModel::trial`): bisect from the current value toward a limit, then require the failing check to clear (preferring no warning) and *no other check to newly fail*. The advice search calls `recompute_output`, never `recompute` (it would recurse). Do not hand-derive fixes here - the solver stays the authority.
- Changing Fit Type loads `fit_type_preset()` (interference ~0.003xD press / 0.005xD shrink / negative for clearance and slip; tolerance band never narrower than the bore band). Leaving Shrink only turns Install Thermal Assist off if Shrink turned it on.

## Tolerances, catalog snapping, alternatives (added later)
- Toleranced dimensions are ONE compact row (`FieldRow::Tol(TolGroup)`) instead of separate `+tol`/`-tol` rows; the global `Tolerance Entry` row (`ToleranceMode`) switches between `+plus / -minus` and `min .. max` for all of them (like Fastener Holes). The model still stores nominal + plus + minus; `commit_tolerance_text` parses (one value = symmetric, signs pick the side, `..`/`/`/space separate; Min/Max keeps the nominal when it lies inside). The Bushing ID has **no** tolerance band by design.
- Recommendations that change a drilled/reamed size (`IdBushing`, and `BoreDia` only ever *smaller*) are snapped to the nearest real catalog size on the passing side (`BushingModel::catalog_sizes` = built-in aircraft reamers + the user's reamer library, synced via `sync_user_reamers`); a bore change also takes the reamer's tolerance and widens the interference band to cover it. If no catalog size passes, the fix says it needs custom tooling. Every recommendation carries `impact` (bore line first, then metrics that moved, then newly-warning checks), computed from the trial solve; several alternatives per failure are offered, including ones that leave the bore alone.
- Results show Δ interference from service temperature and from install thermal assist (solver `delta_thermal`, `assembly_thermal_delta`, `install_delta`).

## Drill bit catalog
- `bushing_solver::drills` embeds the AFT Fasteners drill chart (`bushing-solver/data/drill_bit_catalog.csv`: fractional, #1-#107, A-Z, metric; two source typos corrected, see its module doc) with RapidDirect's "most common" sizes flagged. The Bushing ID picker lists the user's saved sizes first, then every drill (`[common]` tag like the reamer list's `[preferred]`; filter by label, decimal, `letter`/`metric`/`common`...). Fix recommendations may snap an ID to a drill; a **bore** never snaps to a drill (it is reamed) - `advice::bisect_fix` filters `source == "drill"` for `BoreDia`.

## Pitfalls
- Reamer catalog, material library, and Bushing ID library are `src/library.rs` consumers; conflict/duplicate handling lives there, not here.
- `m` inside the reamer picker reaches manual numeric entry; Bore Diameter's `Enter` opens the catalog (a real bore is reamed to a real size).

## Public API
Crate-internal. `mod.rs`: `BushingState`, `handle_key(&mut BushingState, KeyEvent) -> (bool, Vec<Effect>)`, `PANE_MAIN`/`PANE_COUNT`. `model.rs`: `FieldRow`, `NumberTarget`, `field_rows`, `row_label`, `field_hint`, and `cycle_*`/`label_*` pairs for each enum row (bushing type, ID type, end constraint, countersink mode, `FitType`). Pickers: `material_picker`, `reamer_picker`, `friction_picker`, `bushing_id_picker`, each with a `*_persistence` module.

## Design Rationale
- Bore Diameter opens the reamer catalog instead of a free-typed number: a real installed bore is reamed to a real tool size. This replaced a "type, then open Nearest Reamer" flow.
- Reamer, material, and Bushing ID catalogs share `src/library.rs` rather than three import/export implementations.
- `model.rs` stays a bridge: all Lamé, tolerance, and countersink math is in `bushing-solver`/`mechanics-core`.

## Patterns
### Adding an editable field
1. Add a `NumberTarget` variant (or a cycling/toggle `FieldRow`) in `model.rs`.
2. Add its `FieldRow` to `field_rows`, plus a `row_label` and `field_hint` arm (the hint feeds the bottom Hint panel).
3. Read/write it in the model's value getter/setter and feed it into the solver input.
4. Render nothing extra: `view.rs` draws from `field_rows`. Add a validation or readout line only if the result needs one.
5. Enum rows (`FitType`, bushing/ID type, ...) need a `cycle_*` and `label_*` pair; the row list is dynamic, gated by type, countersink mode, enforcement, and thermal state.
6. A picker-backed number row must be excluded from type-to-edit in `handle_key`.

## Boundaries
### Always
- Change formulas in `bushing-solver` (follow its differential-test discipline), never here.
- Route new catalogs through `src/library.rs`.
### Never
- Add a second material or Lamé path to this toolbox.

## Navigation
Parent: `app-tui/AGENTS.md` (crate-wide contracts: `Effect` reducer rule, `Number`-row editing, Caps-Lock key patterns, scroll widgets, disk-space `-p app-tui` scoping). Solver node: `bushing-solver/AGENTS.md`.
