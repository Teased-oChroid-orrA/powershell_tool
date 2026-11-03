# toolboxes/preload_analysis/ — Preload Analysis model (window-only)

> TL;DR: Thin bridge over `fastened-joint-solver::solve::compute`; unique to this crate. Sections: Tightening / Friction / Bearing Geometry / Fastener / Joint Stack / Service Load & Slip / Material Strength Limits / Uncertainty / Thread Load Distribution. The bolt catalog auto-fills thread geometry from sourced catalogs. No terminal toolbox since 2026-11-03: the only head is the Preload window (`gpu_viewer/editor/preload/AGENTS.md`), opened from the dashboard; it owns the bolt/template pickers, the FE worker and all input handling.

## Purpose
Owns: UI-facing model state (field rows incl. member add/remove, bolt catalog `BOLT_SECTIONS`), the FE cross-check job (`fe_check.rs`), joint templates, row text and report (`report.rs`).
Does not own: any preload mechanics (`fastened-joint-solver/AGENTS.md`).
`PreloadModel::matching_bolt` reports "Custom" once any catalog-filled field is hand-edited; picker `Esc` leaves everything untouched. Hi-Lok fills thread geometry only; Lockbolts have no path. Compare catalog entries by designation, never `std::ptr::eq` (a static slice copied from another crate is a different allocation).

## Code Map
| Looking for... | Go to |
|---|---|
| pure UI-facing state bridging `fastened-joint-solver` (thread/friction/member-stack/tightening-mode/uncertainty inputs, full `JointSolution`) | `src/toolboxes/preload_analysis/model.rs` |
| row display text (`display_value`), solver status label, margins / `passes`, plain-text report (`build_report_text`, with the FE cross-check when current) | `src/toolboxes/preload_analysis/report.rs` |
| sectioned AN/NAS/MS/Hi-Lok fastener catalog (`model::BOLT_SECTIONS`, `PreloadModel::select_bolt`); the picker card is the window's `editor/preload/picker.rs` | `src/toolboxes/preload_analysis/model.rs` |
| finite-element member-compliance cross-check (cone `C_m` vs an axisymmetric FE on `fea-core`, load fractions, equivalent cone half angle): `FeInput` / `FeResult`, `run`, `run_with_field`; worker and debounce in `PreloadSession` (`start_fe`, `poll`, `sync_fe_angle`) | `src/toolboxes/preload_analysis/fe_check.rs`; `gpu_viewer/editor/preload/mod.rs`; FE model in `fea-problem/src/joint.rs` |

## Entry Points
| Task | Start Here |
|---|---|
| Change a Preload Analysis formula (thread/bearing torque, compliance, nut rotation, stress, service load, uncertainty, thread load distribution, settlement, thread shear) | `fastened-joint-solver/` (NOT this crate — `toolboxes/preload_analysis/model.rs` only bridges that crate's `solve::compute` into UI state) - see that crate's `lib.rs` doc comment for the one remaining deliberately-scoped-out feature and the fastener-catalog sourcing/exclusion record before assuming something is missing by mistake |
| Add/change a Preload Analysis editable field | `src/toolboxes/preload_analysis/model.rs` (`NumberTarget`/`field_rows`) - member-stack fields are indexed (`NumberTarget::MemberThickness(usize)` etc.), added/removed via `ToggleAddMember`/`ToggleRemoveMember` up to `MAX_MEMBERS` |

## Contracts
- The FE member-compliance cross-check is a read-only second opinion in `Member Stiffness: Pressure Cone` mode (default). In `Finite Element` mode (Advanced section) the solver is fed the cone half angle whose compliance equals the FE one (`FeResult::equivalent_angle_deg`, bisected to 1e-6 and tested to reproduce the FE `C_m`): one solver path, no second solve, no solver change. `PreloadModel::fe_angle_deg` is set only by `PreloadSession::sync_fe_angle` (window) for exactly the current joint (cleared while pending/failed/stale, falling back to the user's cone angle, labelled in Inputs and Results). `FeInput` deliberately excludes the solver angle, so applying it never re-triggers the FE job. `PreloadSession::start_fe` starts the worker 400 ms (`FE_WAIT`) after the joint stops changing (the first at once); `current(&input)` returns a result only for exactly the inputs shown, so a stale result is never displayed as current. Members are assumed Poisson's ratio 0.3 (`fe_check::MEMBER_NU`); the report export includes the cross-check when it is current.
- `model.rs` only bridges `solve::compute`; member-stack fields are indexed (`NumberTarget::MemberThickness(usize)` etc.), added/removed up to `MAX_MEMBERS`.
- `Tightening From` (Nut/Bolt Head) must reach `JointInputs.tightening_from`; it was once a dead toggle.

## Joint templates (`joint_templates.rs`)
- `joint_templates.rs` (pure) holds preset stack-ups (washer/plate/shim/doubler/fitting layers, head-to-nut) and a seeded random generator; `PreloadModel::apply_template` turns one into ordinary editable fields (members incl. washers as members, bearing radii from the washer annulus or ~1.3 d face, shank length = grip, nut data, K=0.2 / 50% of 125 ksi typical torque+preload) and re-solves. All sizes are *typical*, not drawings. `MAX_MEMBERS` is 6 because washers are members. `MemberUi::name` is a label for the stack diagram only.
- The picker is the window's value card (`editor/preload/picker.rs`). Same seed = same random joint (`random_joints_are_always_valid_and_analyzable` sweeps 300 seeds - keep it green when changing the generator).

## Pitfalls
- `field_rows` hides the expert rows behind `FieldRow::AdvancedSection` (`PreloadModel::advanced_open`, collapsed by default): friction model, member stiffness model, thread detail, member outer diameters, Uncertainty, Thread Load Distribution. Hidden values still feed the solve. Tests that look for those rows must set `advanced_open = true`.
- Before assuming a feature is missing, read `fastened-joint-solver/src/lib.rs`: scope cuts and catalog sourcing exclusions are deliberate.

## Public API
Crate-internal. `model.rs`: `PreloadModel`, `Mode`, `FieldRow`, `NumberTarget`, `field_rows(&model)`, `row_label`, `field_hint`, `MemberUi`, `cycle_tightening_from`, `cycle_bearing_model`, `BOLT_SECTIONS`. `report.rs`, `fe_check.rs`, `joint_templates.rs` as above.

## Design Rationale
- Bridge only: the whole engine is `fastened-joint-solver`, which has no UI dependency.
- Catalog selection fills ordinary editable `Number` rows (and reports "Custom" once edited away), so the catalog never hides an overridable value.
- Member stack is a variable-length list (1-4 members) with its own add/remove actions, so `field_rows` takes the model.

## Patterns
### Adding an editable field
1. Add a `NumberTarget` variant (or a cycling/toggle `FieldRow`) in `model.rs`.
2. Add its `FieldRow` to `field_rows`, plus a `row_label` and `field_hint` arm (the hint feeds the bottom Hint panel).
3. Read/write it in the model's value getter/setter and feed it into the solver input.
4. Give it display text in `report::display_value`; the window's cards draw from `field_rows`.
5. Advanced analyses (uncertainty, thread load distribution, embedment, thread shear) are `Option`-gated in the solver; mirror that as an opt-in section here.

## Boundaries
### Always
- Read `fastened-joint-solver/src/lib.rs` scope notes before treating something as missing.
### Never
- Compute any mechanics in `model.rs`.

## Navigation
Parent: `app-tui/AGENTS.md`. Window: `gpu_viewer/editor/preload/AGENTS.md`. Solver node: `fastened-joint-solver/AGENTS.md`.
