# toolboxes/lug_analysis/ — Lug Analysis toolbox

> TL;DR: Thin bridge over `lug-solver`: editable lug/bushing/pin/load fields, an automatic background re-analysis (elastic contact FE + plane-strain plastic collapse) **on the general kernel by default** (legacy / compare selectable), a collapsible mesh section with a brief mesh-size test and recommendation, margins against the material allowables, and a bore profile. Materials (lug and bushing) come from the Material Lookup browser (overlay).

## Purpose
Owns: UI state, key routing, the analysis pipeline wrapper (`model::run`, caching of the condensed model), margin checks, rendering, the report.
Does not own: any mechanics (`lug-solver`), material data (`mechanics-core`), the browser (`../material_lookup/`).

## Code Map
| Looking for... | Go to |
|---|---|
| Fields (`FieldRow`, `NumberTarget`), `LugUiModel::input`, `run`, `evaluate` (checks/notes), `report_text` | `model.rs` |
| `LugAnalysisState`, `tick` (debounced auto-run), `finish`, key routing | `mod.rs` |
| Field list + Results readout + bore profile, material overlay | `view.rs` |

## Contracts
- **Solver choice** (`SolverChoice`, part of `LugInput`): `Kernel` (default, `lug_solver::fea::FeaLug`), `Legacy` (condensed `LugModel` / `FiniteLug`), `Compare` (kernel result shown, the legacy one beside it in a table: hoop, von Mises, pressure, travel, patch, collapse). The elastic analysis and the collapse run side by side on two threads, each on its own copy of the cached models (merged afterwards). A kernel failure falls back to the legacy solver with a note (`LugRun.solver` is the one that produced the answer); the only known unsupported case is a meshed elastic pin with friction on an oblique (full) model. The kernel runs finite-strain collapse for every load direction and bushing; legacy only axial unbushed. Pin bending always uses the legacy `LugModel` (built lazily).
- **Mesh section** (`FieldRow::MeshSection`, collapsed by default; Enter / Space / a single click toggles, `app.rs::handle_lug_analysis_click`): density preset (clears the typed override), `Elements Around Bore` (12-240), `Max Growth Ratio`, `First Layer Aspect`, `Contact Refinement`, and `Mesh Size Test` (`mesh_test.rs`, `Effect::RunLugMeshTest`): the elastic case (and, with the plastic limit load on, the collapse) at 24-120 elements on the kernel, frictionless and in parallel; recommends the coarsest size from which every finer one stays within 2 % hoop, 1 % travel, 10 % pressure, 3 % collapse of the finest; Enter on the row applies it. The result is keyed to geometry/material/pin/load/bushing (`mesh_test::signature`), not to the mesh fields.
- The analysis runs on a worker (`Effect::RunLugAnalysis` -> `AppEvent::LugAnalysisFinished`); `tick` (called from `AppEvent::Tick` while this tool is active) starts a run when the inputs have been unchanged for 250 ms (the very first run starts at once), drops a job for older inputs, and never reruns inputs that just failed.
- `CachedModel` (the condensed lug) rides along in the effect/event so editing only pin/load/friction skips meshing; its key is geometry + (E, nu) + mesh (incl. the quantised auto-refinement) + symmetry + bushing. It also keeps the plane-strain twin and the last collapse (`LimitKey`: pin, angle, flow stress): the collapse does not depend on the applied load, so a load edit reuses it.
- The plastic collapse uses a selectable flow stress (`FlowRule`, default `Ftu`; `(Ftu+Fty)/2` is `MaterialLimits::flow_stress` -> `edge_check`'s rule, `Ftu (1 + elongation)` uses the handbook elongation or an override). Measured on NACA TN 1503 (`lug-solver/tests/validation_naca_tn1503.rs`): mean 18 % low / 10 % low, never high / 4.3 % with 75S up to 10 % high. Ultimate Model = Finite strain (`lug_solver::FiniteLug`, true stress-strain from Fty/Ftu/elongation + a failure strain from the material library, `docs/failure-strain-library.md`, typed values override) replaces the rule for axial loads without a bushing; otherwise the flow-rule collapse is shown with a note. Its result is cached (`FiniteKey`) like the collapse. A failure to find it is a note, never a lost elastic result.
- Turning the bushing on resets the pin diameter to just under the bushing bore (and back to just under the hole when off); the pin must stay within 1.2x of the surface it bears on.
- Margins use `Fbru` only where `mechanics_core::materials::fbru_at_edge_ratio` gives one; otherwise the bearing row is Info with the reason (never a guessed allowable). Elastic stresses above yield are labelled indicators, not capacity.
- Every single-letter binding is `'x' | 'X'` (Windows Caps Lock). Readout paragraphs are trimmed per line: start table rows with text, not padding.

## Pitfalls
- The `Advanced` section (`FieldRow::MeshSection`, `mesh_open`) holds mesh, solver, plasticity, nonlinearity and pin-bending rows; collapsed by default. Results start with a `PASS/REVIEW/FAIL` verdict line, then Checks, then the contact/ultimate details.
- `Head Shape = Round` derives edge and corner radius from the width; the Edge/Corner rows only exist in Custom mode. Toggling Custom copies the round values so nothing jumps.
- Oblique/transverse loads use the full lug and a clamped far end: the response depends on `Model Length` (a note says so).
- Do not present the elastic peak hoop as a failure load: the ultimate margin comes from the plastic collapse (Plastic Limit Load).
- The optional analyses (Pin Body, Temperature Change, Ultimate Model = hardening with a Failure Strain, Geometric Nonlinearity, Pin Bending with a Clevis Load Offset) are all part of `LugInput`, so two equal inputs still give one result; the hardening law (`LimitKey.ductility`) and the pin spec are part of the collapse cache key. The "Solution check" row is the solver's `Verification` (equilibrium, energy, penetration, friction cone, ZZ mesh error); a Warn above 10 % mesh error says to try Fine mesh.
- The ultimate note quotes the NACA TN 1503 statistics of the solver in use (`naca_statistics`): kernel `Ftu` is from 13 % low to 2 % high, so the old "never high" no longer holds.
- Pin bending reports no pin allowable (no pin material strengths in the library): stresses only. Single shear is not offered (see `lug-solver/AGENTS.md`).

## Navigation
Parent: `app-tui/AGENTS.md`. Solver node: `lug-solver/AGENTS.md`. Write-up: `docs/lug-analysis.md`.
