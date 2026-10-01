# toolboxes/pressure_vessel/ — Pressure Vessel Analyzer toolbox

> TL;DR: Thin bridge over `pressure-vessel-solver`/`mechanics-core`; `model.rs` computes nothing itself. Adds thermal stress (solver-side, `pressure-vessel-solver::thermal`), Numbers panel (`d`), report export (`e`), filterable material picker + persisted custom materials.

## Purpose
Owns: UI state/key routing/rendering, material picker, custom-material persistence (`settings-tui-pressure-vessel.json`).
Does not own: stress/failure/buckling/thickness math (`pressure-vessel-solver`). No cross-section sketch or derivation view exists (terminal has no equivalent).

## Code Map
| Looking for... | Go to |
|---|---|
| pure UI-facing state bridging `pressure-vessel-solver` (geometry/pressure/material/buckling/thermal inputs, failure-mode/thickness-solve outputs), custom-material catalog | `src/toolboxes/pressure_vessel/model.rs` |
| `PressureVesselState`, toolbox-local key routing (`d` numbers panel, `e` export) | `src/toolboxes/pressure_vessel/mod.rs` |
| rendering (field list + inline validation hints + results readout: governing/classification/spec summary/min-thickness/checks/thermal-and-buckling notes/Numbers panel, wide/narrow layout, plain-text report builder) | `src/toolboxes/pressure_vessel/view.rs` |
| filterable material picker + "add custom material" form | `src/toolboxes/pressure_vessel/material_picker.rs` |
| custom-material persistence (`settings-tui-pressure-vessel.json`) | `src/toolboxes/pressure_vessel/persistence.rs` |

## Entry Points
| Task | Start Here |
|---|---|
| Change a Pressure Vessel Analyzer formula (stress, failure mode, buckling, minimum-thickness solve) | `pressure-vessel-solver/` (NOT this crate — `toolboxes/pressure_vessel/model.rs` only bridges that crate's existing public API into UI state; it computes nothing itself) |
| Add/change a Pressure Vessel Analyzer editable field | `src/toolboxes/pressure_vessel/model.rs` (`NumberTarget`/`FIELD_ROWS`) - every field is a plain `f64` (no toleranced-value concept, unlike Fastener Holes) |

## Contracts
- Every editable field is a plain `f64` (no toleranced-value concept, unlike Fastener Holes); `model.rs` computes nothing itself.
- Thermal stress is superposed into the same four failure checks; the thermal module belongs to `pressure-vessel-solver`, not this crate.

## Pitfalls
- Custom materials persist in `settings-tui-pressure-vessel.json`; keep forward-compat discipline (`#[serde(default)]`/`Option<T>`) on any new persisted field.

## Public API
Crate-internal. `mod.rs`: `PressureVesselState`, `handle_key(&mut PressureVesselState, KeyEvent) -> (bool, Vec<Effect>)` (`d` Numbers panel, `e` export), `PANE_MAIN`/`PANE_COUNT`. `model.rs`: `PressureVesselModel`, `FieldRow`, `NumberTarget`, `field_rows()`, `row_label`, `field_hint`, `format_for_edit`. `material_picker` and `persistence` handle custom materials.

## Design Rationale
- Bridge only: failure modes, minimum thickness, buckling and thermal stress all come from `pressure-vessel-solver`, so every toolbox shares one authoritative implementation.
- A text Numbers panel (`d`) stands in for the skipped derivation view, which has no terminal equivalent.
- Custom materials persist across relaunches in their own settings file rather than editing the built-in table.

## Patterns
### Adding an editable field
1. Add a `NumberTarget` variant (or a cycling/toggle `FieldRow`) in `model.rs`.
2. Add its `FieldRow` to `field_rows`, plus a `row_label` and `field_hint` arm (the hint feeds the bottom Hint panel).
3. Read/write it in the model's value getter/setter and feed it into the solver input.
4. Render nothing extra: `view.rs` draws from `field_rows`. Add a validation or readout line only if the result needs one.
5. A new solver output needs a readout line in `view.rs` and, if it belongs in the exported report, the plain-text report builder.

## Boundaries
### Always
- Change formulas in `pressure-vessel-solver`, never in `model.rs`.
- Render the readout through `widgets/scroll_paragraph.rs`.
### Never
- Add a parallel material table here.

## Navigation
Parent: `app-tui/AGENTS.md` (crate-wide contracts: `Effect` reducer rule, `Number`-row editing, Caps-Lock key patterns, scroll widgets, disk-space `-p app-tui` scoping). 
