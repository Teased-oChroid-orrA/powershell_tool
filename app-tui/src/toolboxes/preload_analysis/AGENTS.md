# toolboxes/preload_analysis/ — Preload Analysis toolbox

> TL;DR: Thin bridge over `fastened-joint-solver::solve::compute`; unique to this crate. Sections: Tightening / Friction / Bearing Geometry / Fastener / Joint Stack / Service Load & Slip / Material Strength Limits / Uncertainty / Thread Load Distribution. Bolt picker auto-fills thread geometry from sourced catalogs.

## Purpose
Owns: UI state/key routing (incl. member add/remove, `d` numbers, `e` export), rendering, `bolt_picker.rs`.
Does not own: any preload mechanics (`fastened-joint-solver/AGENTS.md`).
`PreloadModel::matching_bolt` reports "Custom" once any catalog-filled field is hand-edited; picker `Esc` leaves everything untouched. Hi-Lok fills thread geometry only; Lockbolts have no path.

## Code Map
| Looking for... | Go to |
|---|---|
| pure UI-facing state bridging `fastened-joint-solver` (thread/friction/member-stack/tightening-mode/uncertainty inputs, full `JointSolution`) | `src/toolboxes/preload_analysis/model.rs` |
| `PreloadAnalysisState`, toolbox-local key routing including member add/remove (`d` numbers panel, `e` export) | `src/toolboxes/preload_analysis/mod.rs` |
| rendering (field list + torque/deformation/rotation/stress/service-load/slip/uncertainty readout, wide/narrow layout, plain-text report builder) | `src/toolboxes/preload_analysis/view.rs` |
| sectioned AN/NAS/MS/Hi-Lok fastener catalog picker (auto-fills thread geometry + shank diameter) | `src/toolboxes/preload_analysis/bolt_picker.rs` |
| finite-element member-compliance cross-check (cone `C_m` vs an axisymmetric FE on `fea-core`, load fractions, equivalent cone half angle): input / result types, `run`, state in `PreloadAnalysisState::fe`, `tick` / `finish_fe` | `src/toolboxes/preload_analysis/fe_check.rs`, `mod.rs`; FE model in `fea-problem/src/joint.rs` |

## Entry Points
| Task | Start Here |
|---|---|
| Change a Preload Analysis formula (thread/bearing torque, compliance, nut rotation, stress, service load, uncertainty, thread load distribution, settlement, thread shear) | `fastened-joint-solver/` (NOT this crate — `toolboxes/preload_analysis/model.rs` only bridges that crate's `solve::compute` into UI state) - see that crate's `lib.rs` doc comment for the one remaining deliberately-scoped-out feature and the fastener-catalog sourcing/exclusion record before assuming something is missing by mistake |
| Add/change a Preload Analysis editable field | `src/toolboxes/preload_analysis/model.rs` (`NumberTarget`/`field_rows`) - member-stack fields are indexed (`NumberTarget::MemberThickness(usize)` etc.), added/removed via `ToggleAddMember`/`ToggleRemoveMember` up to `MAX_MEMBERS` |

## Contracts
- The FE member-compliance cross-check is a read-only second opinion in `Member Stiffness: Pressure Cone` mode (default). In `Finite Element` mode (Advanced section) the solver is fed the cone half angle whose compliance equals the FE one (`FeResult::equivalent_angle_deg`, bisected to 1e-6 and tested to reproduce the FE `C_m`): one solver path, no second solve, no solver change. `PreloadModel::fe_angle_deg` is set only by `PreloadAnalysisState::sync_fe_angle` for exactly the current joint (cleared while pending/failed/stale, falling back to the user's cone angle, labelled in Inputs and Results). `FeInput` deliberately excludes the solver angle, so applying it never re-triggers the FE job. `tick` (every `Tick` while the toolbox is active) starts `Effect::RunMemberFe` 400 ms after the joint stops changing (the first at once); `current(&input)` returns a result only for exactly the inputs shown, so a stale result is never displayed as current. Members are assumed Poisson's ratio 0.3 (`fe_check::MEMBER_NU`); the report export includes the cross-check when it is current.
- `model.rs` only bridges `solve::compute`; member-stack fields are indexed (`NumberTarget::MemberThickness(usize)` etc.), added/removed up to `MAX_MEMBERS`.
- `Tightening From` (Nut/Bolt Head) must reach `JointInputs.tightening_from`; it was once a dead toggle.

## Joint templates (`joint_templates.rs`, `template_picker.rs`)
- `joint_templates.rs` (pure) holds preset stack-ups (washer/plate/shim/doubler/fitting layers, head-to-nut) and a seeded random generator; `PreloadModel::apply_template` turns one into ordinary editable fields (members incl. washers as members, bearing radii from the washer annulus or ~1.3 d face, shank length = grip, nut data, K=0.2 / 50% of 125 ksi typical torque+preload) and re-solves. All sizes are *typical*, not drawings. `MAX_MEMBERS` is 6 because washers are members. `MemberUi::name` is a label for the stack diagram only.
- Window: `t` or the `Joint Template` row; Up/Down choose, Enter apply, `g` next random, `b` previous, Esc close; every row/button is a mouse target (`MouseRegions::template_actions`, double-click applies). Same seed = same random joint (`random_joints_are_always_valid_and_analyzable` sweeps 300 seeds - keep it green when changing the generator).
- Results show the head-to-nut stack picture (`stack_diagram`) for any stack, template or hand-built.

## Pitfalls
- `field_rows` hides the expert rows behind `FieldRow::AdvancedSection` (`PreloadModel::advanced_open`, collapsed by default): friction model, member stiffness model, thread detail, member outer diameters, Uncertainty, Thread Load Distribution. Hidden values still feed the solve. Tests that look for those rows must set `advanced_open = true`.
- Before assuming a feature is missing, read `fastened-joint-solver/src/lib.rs`: scope cuts and catalog sourcing exclusions are deliberate.

## Public API
Crate-internal. `mod.rs`: `PreloadAnalysisState`, `handle_key(&mut PreloadAnalysisState, KeyEvent) -> (bool, Vec<Effect>)` (member add/remove, `d`, `e`), `PANE_MAIN`/`PANE_COUNT`. `model.rs`: `PreloadModel`, `Mode`, `FieldRow`, `NumberTarget`, `field_rows(&model)`, `row_label`, `field_hint`, `MemberUi`, `cycle_tightening_from`, `cycle_bearing_model`. `bolt_picker`: catalog picker.

## Design Rationale
- Bridge only: the whole engine is `fastened-joint-solver`, which has no UI dependency.
- Catalog selection fills ordinary editable `Number` rows (and reports "Custom" once edited away), so the catalog never hides an overridable value.
- Member stack is a variable-length list (1-4 members) with its own add/remove actions, so `field_rows` takes the model.

## Patterns
### Adding an editable field
1. Add a `NumberTarget` variant (or a cycling/toggle `FieldRow`) in `model.rs`.
2. Add its `FieldRow` to `field_rows`, plus a `row_label` and `field_hint` arm (the hint feeds the bottom Hint panel).
3. Read/write it in the model's value getter/setter and feed it into the solver input.
4. Render nothing extra: `view.rs` draws from `field_rows`. Add a validation or readout line only if the result needs one.
5. Advanced analyses (uncertainty, thread load distribution, embedment, thread shear) are `Option`-gated in the solver; mirror that as an opt-in section here.

## Boundaries
### Always
- Read `fastened-joint-solver/src/lib.rs` scope notes before treating something as missing.
### Never
- Compute any mechanics in `model.rs`.

## Navigation
Parent: `app-tui/AGENTS.md` (crate-wide contracts: `Effect` reducer rule, `Number`-row editing, Caps-Lock key patterns, scroll widgets, disk-space `-p app-tui` scoping). Solver node: `fastened-joint-solver/AGENTS.md`.
