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
