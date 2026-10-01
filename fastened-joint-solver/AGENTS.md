# fastened-joint-solver/ — Fastened-Joint Preload Solver

> TL;DR: Zero-UI solver crate for threaded-fastener installation/preload mechanics. Consumed only by `app-tui`'s Preload Analysis toolbox (`toolboxes/preload_analysis/model.rs` bridges it into UI state). Entry point is `solve::compute(&JointInputs) -> Result<JointSolution, ValidationError>`.

## Purpose
Owns: V-thread torque equation, uniform-pressure/uniform-wear bearing friction, Brent-root-solved torque-preload equilibrium, fastener/member compliance (VDI 2230 segments + numerically integrated Rotscher pressure-cone), nut rotation, stress state, service-load/separation/slip checks, uncertainty engines, thread load distribution, thread shear, and the fastener thread catalog.
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
| Worst-case corner search + seeded Monte Carlo | `src/uncertainty.rs` |
| Per-thread spring-coupled load distribution | `src/thread_load_distribution.rs` |
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
- Friction integrals are cross-checked against quadrature; thread load distribution against the continuum sinh solution (differential tests). Keep those tests when touching either.

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
