# toolboxes/pressure_vessel/ — Pressure Vessel model (window-only)

> TL;DR: Thin bridge over `pressure-vessel-solver`/`mechanics-core`; `model.rs` computes nothing itself. Adds thermal stress (solver-side, `pressure-vessel-solver::thermal`), the shared row text and plain-text report (`report.rs`), the material filter and "add material" form, and persisted custom materials. No terminal toolbox since 2026-11-03: the only head is the Pressure Vessel window (`gpu_viewer/editor/vessel/AGENTS.md`), opened from the dashboard.

## Purpose
Owns: UI-facing model state (field rows, values, validation), row text and report, material filter + add-material form, custom-material persistence (`settings-tui-pressure-vessel.json`, file name kept so existing files load).
Does not own: stress/failure/buckling/thickness math (`pressure-vessel-solver`); any drawing or input handling (the window).

## Code Map
| Looking for... | Go to |
|---|---|
| pure UI-facing state bridging `pressure-vessel-solver` (geometry/pressure/material/buckling/thermal inputs, failure-mode/thickness-solve outputs), custom-material catalog | `model.rs` |
| row display text (`display_value`), pass rule (`passes`: an unbounded margin counts), plain-text report (`build_report_text`) | `report.rs` |
| material filter (`visible_indices(model, query)`) + "add custom material" form (`AddMaterialForm`, `ADD_MATERIAL_FIELDS`) | `material_picker.rs` |
| "Minimum wall by requirement": rows and class (`requirements.rs`), the line run's inputs (`line.rs`), walls and the line wall job (`walls.rs`), recompute, margin curve, reactions, equilibrium (`run.rs`), typed edits with one address `Edit` (`edit.rs`) | `requirements.rs`, `line.rs`, `walls.rs`, `run.rs`, `edit.rs` |
| custom-material persistence (`config_path`, `load`, `save_at`, `PersistedMaterial`) | `persistence.rs` |

## Entry Points
| Task | Start Here |
|---|---|
| Change a Pressure Vessel formula (stress, failure mode, buckling, minimum-thickness solve) | `pressure-vessel-solver/` (NOT this crate — `model.rs` only bridges that crate's public API into UI state; it computes nothing itself) |
| Add/change an editable field | `model.rs` (`NumberTarget`/`field_rows`) - every field is a plain `f64` (no toleranced-value concept, unlike Fastener Holes) |
| Change how the window shows or edits it | `gpu_viewer/editor/vessel/AGENTS.md` |

## Contracts
- Every editable field is a plain `f64`; `model.rs` computes nothing itself.
- Thermal stress is superposed into the same four failure checks; the thermal module belongs to `pressure-vessel-solver`, not this crate.
- `report.rs` and `model.rs` are ratatui-free except for `theme::fmt_margin` (plain formatting), so the window reads every row and report string here.

## Pitfalls
- **Never invent a factor, a required margin, an allowable or a density.** Rows are seeded 1.0 / 0.0 (neutral), the tube density is 0 until entered. A material change re-seeds only rows with `edited == false` and clears the allowable overrides (`material_changed`).
- The line run's walls are a job (`LineWallJob`, `line_wall_job` / `set_line_walls`): `walls` is `None` while it runs, and an answer for an older `walls_revision` is dropped. Every `recompute` bumps the revision.
- The code check (`heads: HeadsCheck`, inputs `head: HeadInputs`) is independent of the Lame analysis: `recompute_heads` runs even when `geometry` is invalid (provided thickness is then `None`). S and E are placeholders by design; never default them from the material table. Dimensions are corroded-condition inputs; the provided thickness is the remaining wall.
- Custom materials persist in `settings-tui-pressure-vessel.json`; keep forward-compat discipline (`#[serde(default)]`/`Option<T>`) on any new persisted field.

## Design Rationale
- Bridge only: failure modes, minimum thickness, buckling and thermal stress all come from `pressure-vessel-solver`, so one authoritative implementation.
- Custom materials persist across relaunches in their own settings file rather than editing the built-in table.

## Patterns
### Adding an editable field
1. Add a `NumberTarget` variant (or a cycling/toggle `FieldRow`) in `model.rs`.
2. Add its `FieldRow` to `field_rows`, plus a `row_label` and `field_hint` arm.
3. Read/write it in the model's value getter/setter and feed it into the solver input.
4. Give it display text in `report::display_value`; the window's cards draw from `field_rows`.
5. A new solver output that belongs in the exported report goes in `report::build_report_text`.

## Boundaries
### Always
- Change formulas in `pressure-vessel-solver`, never in `model.rs`.
### Never
- Add a parallel material table here.

## Navigation
Parent: `app-tui/AGENTS.md`. Window: `gpu_viewer/editor/vessel/AGENTS.md`.
