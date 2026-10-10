# Issue #12 — Phase 15 verified orchestration

2026-10-10 UTC. Validation stays in `validate.rs`, selection/workflow in `strategy.rs`,
and common acceptance records in `report.rs`; no GUI or learned solver replacement.

`solve_refined` combines validated deterministic `solve_adaptive` selection and its at-most-one
direct fallback with the shared `adapt::refine` loop. The caller owns geometry/meshing and must
return rebuilt loads/constraints with each mesh. Every candidate passes numerical and equilibrium
checks; finite ZZ estimates select the best candidate. The budget is explicit, callback failures
propagate, and final unmet ZZ tolerance returns an error including the attempt/budget/check data.
The report lists decisions, tolerances, measured residual/equilibrium/ZZ, and diagnostics. Remeshing
is currently 2D. ZZ is a smooth-solution estimator, not a universal bound near singularities;
nodal-temperature-field ZZ recovery is explicitly unsupported instead of silently omitting thermal
strain. Existing one-way coupling does not request that estimator.

`solve_nonlinear_verified` augments the bounded nonlinear ladder with finite state, strict Completed
termination, no accepted stalls/unfinished AL passes, and a finite final force-scaled residual.
It also requires a caller independent physical verifier; contact/follower/engineering-limit checks
cannot be inferred from Newton convergence. Raw `complete()` remains the historical termination
classification and is not a verification flag. Invalid rung/tolerance input is rejected.

`AcceptanceReport` never accepts empty evidence or nonfinite values/limits, reports all checks and
serializes deterministic diagnostics. `ZzEstimate::relative` uses hypot to avoid squaring overflow;
a truly zero field has zero relative error, invalid norms stay nonfinite and fail acceptance.

Regression coverage includes insufficient refinement budget, achieved ZZ target with rebuilt
meshes, physical-verifier rejection, strict nonlinear residual rejection, malformed dimensions,
nonfinite acceptance requirements/reports, zero/large ZZ norms, and existing fallback/ladder tests.
Full milestone validation and evidence are in the final roadmap handoff.
