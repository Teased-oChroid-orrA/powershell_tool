# Lug solver upgrade plan (15 items)

Goal: move `lug-solver` closer to a full FEM (accuracy, robustness) with minimal speed cost.
Each item lands with a regression/validation test, is measured against the existing gates
(Lame, Hertz, ring collapse, NACA TN 1503, bushing, angle sweep) and is recorded in
`docs/lug-analysis.md` + `lug-solver/AGENTS.md`. Status is updated as items land.

| # | Item | Phase | Status |
|---|------|-------|--------|
| 13 | Solution self-checks (equilibrium, complementarity, energy) | A verification | done (`Verification`; found the 1e-6E bushing grounding leaked 1.4 % of the load: now 1e-9E, 70 % slower on bushed models until item 12) |
| 5  | Superconvergent stress recovery + ZZ error estimator | A verification | done (SPR + ZZ in `stress.rs`; reported in `Verification.discretisation_error`) |
| 14 | Share K/G/S across angle/clearance/load sweeps (+ angle envelope API) | A speed | done. `solve_sweep`, `collapse_sweep` (parallel) |
| 15 | Parallel Woodbury/tangent products | A speed | partly. 4-accumulator dot, parallel sweeps; profiling shows the cost is the number of tangent factorisations, so parallel Woodbury was not built |
| 6  | Volumetric locking patch test (plane-strain plasticity) | B material | done (no change needed). Exact plane-strain ring collapse within 0.3 %: no locking |
| 2  | Hardening law (multilinear / Ramberg-Osgood) | B material | done. `Hardening`, Ramberg-Osgood, strain-limited collapse; NACA 16 % vs 18 % |
| 3  | Plasticity: radial return + consistent tangent, modified Newton | B material | measured, not built. The collapse is 42 iterations / 0.16 s; a consistent-tangent Newton is slower |
| 11 | Consistent Coulomb friction (AL on tangential part), drop STICK_RATIO | C contact | done. Tangential AL multiplier; old results were 4-8 % low at 45-90 deg |
| 12 | Bushing rigid-body stabilisation by constraint | C contact | done by measurement. 1e-9 E leak 0.016 %, no speed cost once friction is consistent |
| 4  | Mortar/segment bushing-lug interface | C contact | done for the bushing interface (nodal collocation, sawtooth 0.37 -> 0.004); pin contact unchanged |
| 10 | Single path-following controller | C contact | improved, not replaced. Coarse trials, substep before nested, no forced growth: bushed 45 deg 8.5 -> 1.9 s |
| 1  | Deformable elastic pin | D model | done. Elastic pin by `G + A`; exact Lame 0.2 %; does not change limit loads |
| 9  | Thermal strain (dT, CTE mismatch) | D model | done. `Thermal` on `PinSpec` |
| 8  | Through-thickness (layered / generalised plane strain) | D model | done for double shear (`through_thickness`); single shear deliberately excluded |
| 7  | Large-displacement (updated Lagrangian) | D model | done as second order (P-delta) without moving the mesh; beam-column check 1-5 % |

Rules: one item at a time; all `cargo test -p lug-solver` + `-p app-tui` stay green after each;
an item that cannot be shown to help (measured) is reported as such, not merged blindly.

## Follow-up: collapse-load accuracy

Flow-stress rules (selectable, default Ftu) and a finite-strain solver: `docs/finite-strain-collapse.md`. NACA TN 1503
twelve points: (Ftu+Fty)/2 18 % mean error, Ftu 10 %, Ftu(1+e) 4.3 %, finite strain with one failure strain per alloy 1.7 %.
