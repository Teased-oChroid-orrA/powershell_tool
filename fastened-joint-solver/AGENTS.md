# fastened-joint-solver/ — Fastened-Joint Preload Solver

> TL;DR: Zero-UI solver crate for threaded-fastener installation/preload mechanics. Consumed only by `app-tui`'s Preload Analysis toolbox (`toolboxes/preload_analysis/model.rs` bridges it into UI state). Entry point is `solve::compute(&JointInputs) -> Result<JointSolution, ValidationError>`.

## Purpose
Owns: V-thread torque equation, uniform-pressure/uniform-wear bearing friction, Brent-root-solved torque-preload equilibrium, fastener/member compliance (VDI 2230 segments + numerically integrated Rotscher pressure-cone), nut rotation, stress state, service-load/separation/slip checks, uncertainty engines, thread load distribution, thread shear, the VDI 2230 bolt fatigue screen (`src/fatigue.rs`, `Option`-gated `JointInputs.fatigue`) and the fastener thread catalog.
Does not own: any UI, unit selection, or material database (`mechanics-core`).

## Code Map
| Looking for... | Go to |
|---|---|
| Orchestration, `JointInputs`/`JointSolution`, `AnalysisMode` | `src/solve.rs` (`compute`, `validate`) |
| Brent root finder | `src/root.rs` |
| Thread/bearing torque | `src/thread.rs`, `src/bearing.rs` |
| Gauss-Legendre quadrature (cross-check of friction integrals) | `src/quadrature.rs` |
| Compliance (fastener + Rotscher members) | `src/compliance.rs` |
| Stress state / service load / thread shear | `src/stress.rs`, `src/service.rs`, `src/thread_shear.rs` |
| Bolt fatigue (VDI 2230 alternating stress vs endurance amplitude) | `src/fatigue.rs` |
| Worst-case corner search + seeded Monte Carlo | `src/uncertainty.rs` |
| Per-thread spring-coupled load distribution | `src/thread_load_distribution.rs` |
| Thread Workbench tapped-hole elastic FE adapter | `src/thread_hybrid.rs` |
| AN/NAS/MS21250/Hi-Lok thread catalogs | `src/thread_catalog.rs` |

## Entry Points
| Task | Start Here |
|---|---|
| Change the solve/torque-preload equilibrium | `src/solve.rs::compute` |
| Add a fastener to the catalog | `src/thread_catalog.rs` (cite a source; see Pitfalls) |
| Add an input field | `JointInputs` in `src/solve.rs`, then `app-tui/src/toolboxes/preload_analysis/model.rs` |

## Contracts
- Primary solver never uses the reduced `T = K*F*d` coefficient; pitch diameter, lead angle, flank angle stay explicit.
- Advanced analyses (uncertainty, Monte Carlo, thread load distribution, embedment, thread shear) are `Option`-gated inputs, off by default.
- `JointInputs.tightening_from` selects head-side vs nut-side bearing geometry for every torque/friction calc; unset head geometry defaults to the nut-side pair.
- Catalog UNJ minor-diameter uses the ASME B1.15 coefficient, distinct from UN/UNF's ASME B1.1.

## Pitfalls
- `TighteningMember` was once stored but never read by `compute` (dead UI toggle). Any new input must be traced into `solve::compute`, not just the UI.
- Do not fabricate catalog data. MS20004-MS20024 omitted (only MS20004 sourced). Hi-Lok fills thread geometry only (no public clamp-up table). Lockbolts have no path: swaged, not torque-installed.
- Only deliberate model cut: locking-feature prevailing torque as a function of rotation (`FrictionInputs::prevailing_torque` is a constant baseline). Full record in `src/lib.rs` doc comment.
- Thread shear uses the FED-STD-H28 same-material simplified area; no thread-class tolerance data exists here for the external/internal-distinct form.
- `thread_capability.rs` owns the separate Unified axial screen used by Thread Workbench: external/internal FED-STD-H28 shear areas, damage comparison, STI hole limits, and insert manufacturer-rating gates. `thread_catalog.rs` now distinguishes the internal basic minor (`1.082532P`) from the external basic root (`1.226869P`); keep both when changing catalog geometry. Method and source limits: `docs/thread-workbench.md`.
- `thread_hybrid.rs` runs the 1D elastic bolt/parent bar-and-spring FE model for tapped-hole load sharing only. Its uniform parent annulus is an idealization; it does not change `thread_capability` capacities or model coil/key transfer, flank contact, plasticity or rupture.
- `thread_capability.rs` also owns the inch installation catalog: 44 Heli-Coil STI hole/drill records and 16 Howmet solid-insert regular-tap records. Keep STI tap and regular Unified tap data distinct; pilot drills are not strength ratings. Source pages and expansion limits: `docs/thread-workbench.md`.
- HC68-2 chart readings for 1/4-20 and 1/4-28 are advisory rows only; the graph is approximate and does not replace a confirmed matching assembly rating. NASA RP-1228's tapped-hole equation must not be applied to a wire-insert assembly.
- Friction integrals are cross-checked against quadrature; thread load distribution against the continuum sinh solution (differential tests). Keep those tests when touching either.

- Fatigue (`fatigue.rs`): formulas `sigma_ASV = 0.85 (150/d + 45)` MPa (rolled before heat treatment) and `sigma_ASG = (2 - F_Sm/F_0.2min) sigma_ASV` come from the FVA knowledge base page and a TU Magdeburg paper (cited in the module), **not the standard itself**: check the VDI 2230 edition before relying on it. Elastic single bolt: no thermal load, settlement or eccentric clamping. `n` (load introduction) is an input, 0.5 only a mid-range assumption. The app window has no inputs for it yet (`model.rs` passes `fatigue: None`; layout awaits approval, `docs/workbench-redesign/preload.html`).

## Boundaries
### Always
- `cargo test -p fastened-joint-solver`, scoped to the crate (full-workspace builds can exhaust disk; see root Global Pitfalls).
### Never
- Add UI/terminal dependencies, or duplicate Lamé/material math that belongs in `mechanics-core`.

## Public API
`solve::compute(&JointInputs) -> Result<JointSolution, ValidationError>` and `solve::validate`. `root::brent` (Brent root finder). Modules `thread`, `bearing`, `compliance`, `stress`, `service`, `uncertainty`, `thread_load_distribution`, `thread_shear`, `thread_catalog`, `quadrature`, `validation` are public so the UI and tests can reach their types, but callers drive the solver through `compute` only.

## Design Rationale
- One module per physical concern, so each can be differential-tested against an independent evaluation (quadrature for friction integrals, continuum sinh for thread load).
- Brent root solve for torque-preload equilibrium because thread and bearing friction couple through preload.
- Advanced analyses are `Option`-gated, matching the spec's "advanced option, not mandatory for initial UI interaction" wording.

## Patterns
### Adding an advanced analysis
1. New module with its own inputs/outputs and tests (an independent cross-check, not just spot values).
2. `Option<...>` field on `JointInputs`, `Option<...>` result on `JointSolution`.
3. Wire it into `solve::compute`; only when the input is `Some`.
4. Surface it in `app-tui`'s `preload_analysis/model.rs` as an opt-in section.

## Navigation
Parent: root `CLAUDE.md`. Consumer: `app-tui/AGENTS.md`. Sibling pattern: `bushing-solver/AGENTS.md`.
