# edge-check

> TL;DR: Independent cross-checks of the bushing edge-distance requirement (stress superposition / tabulated allowables / bushing-and-housing contact FE; the plane-stress FE is kept for validation tests) behind one `EdgeModel` trait. Pure function of an `EdgeInput`; zero dependency on `bushing-solver`; zero external crates. Full write-up: `docs/edge-distance-crosscheck.md`.

## Purpose
Owns: the edge-distance models, the runner (margins, minimum edge distance, Monte Carlo, report), the small FE and least-squares solvers they need.
Does not own: the legacy `Fbru + 0.8 p` check (stays in `bushing-solver/src/solve.rs`, untouched) or any UI (`app-tui/src/toolboxes/bushing/edge_check.rs` adapts the bushing model to `EdgeInput` and renders the report).

## Code Map
| Looking for... | Go to |
|---|---|
| The trait every model implements (`EdgeModel` -> `Response`) | `src/model.rs` |
| Registry (add/remove a model here) | `src/models/mod.rs::default_models` |
| Elastic analytic field (complex potentials + collocation) | `src/analytic.rs` |
| FE solver (Q9, banded Cholesky, point stress recovery) | `src/fem.rs`, `src/linalg.rs` |
| Failure modes from any stress field (shear-out / splitting / first yield) | `src/field.rs` |
| Tabulated allowables | `src/models/allowable.rs` |
| Bushing + housing contact FE (penalty contact, friction, fit-then-pin, J2 housing) and its edge-distance/fit table | `src/contact.rs`, `src/models/contact_model.rs` |
| Elastic-plastic limit load (J2, plane strain, displacement control, Anderson-accelerated initial stress) and its parallel edge-distance profile | `src/plastic.rs`, `src/models/plastic_model.rs` |
| Run, min-edge search, Monte Carlo, report types | `src/runner.rs`, `src/mc.rs` |
| Proof the models are right | `tests/verification.rs`, unit tests in `analytic.rs`/`fem.rs` |

## Contracts
- Free edge is `x = 0`, bore centre `(edge, 0)`, load acts along `-x`; stresses in psi, lengths in inches, per-unit-thickness loads divide by `thickness`.
- `FieldResponse` caches unit-load stress tensors at fixed probe points; `margins(loads, strength_scale)` must stay a cheap linear recombination (Monte Carlo calls it thousands of times).
- Every model must charge the fit pressure at 100% against the edge-failure modes (allowables: `P + p D t` in shear-out; bearing is deliberately not reduced). `TargetResult::capacity_lbf` (bisection on pin load, model-agnostic) is the load a check can carry; keep it consistent with `margin` (`margin >= 0` iff `capacity >= load`). Audit table in `docs/edge-distance-crosscheck.md`.
- Shear-out is the mean tangent-plane shear (a limit load assuming full redistribution), not an elastic peak. Do not add an elastic-peak bound at the bearing-limit load: it is dominated by the bearing stress and fails at every edge distance (tried).
- A model that cannot evaluate a case returns `Err`; the runner reports it and the rest still run. Never silently drop a model.

## Pitfalls
- Contact FE (`contact.rs`): keep the penalty at ~100 E/a (1000 E/a makes the Newton iteration cycle between contact sets and the factorisation ill-conditioned); keep the constraint penalty only ~1e4x the largest stiffness (1e9x loses digits); the far-face reaction is a follower load of the (stiff) pin contact, so its derivative must stay in the iteration matrix (Sherman-Morrison) or the iteration cannot converge; the pin only exists after the fit step (before it the closing bore would press on it). Friction slip limits lag one load step. Reuse a factorisation across steps while at most `REUSE_DIFF` contact points changed. Collapse is the load plateau: its noise is 1-2 %, do not assert tighter.
- Plastic collapse under *load* control is ambiguous (no equilibrium past the limit looks like slow convergence): `plastic.rs` prescribes the loaded-point displacement and watches the load plateau. Its limit load varies ~1-2 % with the convergence tolerances; do not tighten asserts below that.
- The plastic model is plane strain on purpose (plane-stress bearing crush masks the edge); raising the flow stress in a "bearing zone" was tried and rejected (result tracked the zone size).
- Q9 midside nodes must sit at element midpoints; a graded (quarter-point-like) midside node makes the Jacobian vanish at the bore and the stress there is garbage (found the hard way: `fem.rs` grades element *boundaries* only).
- Compare FE and analytic on a half-plane-sized plate with a mesh fine enough for that plate; on a huge plate with the default mesh the FE is under-resolved and the gap is discretisation, not physics.
- The half-cosine pin load has a slope kink at +-90 degrees: the analytic fit converges only algebraically for it (~0.5 % of peak stress at the default `FitSpec`); fit and full-cosine cases are exact to machine precision.
- Build the crate optimised even in dev (`[profile.dev.package.edge-check]` in the root `Cargo.toml`): the TUI runs it synchronously on a key press.

## Navigation
Consumer: `app-tui/src/toolboxes/bushing/AGENTS.md`. History/validation: `docs/edge-distance-crosscheck.md`.
