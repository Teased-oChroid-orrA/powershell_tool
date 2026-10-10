# ADR-013: Eigenvalue, transient and strategy-selection methods of `fea-core`

Status: Accepted (user direction 2026-10-08: an adaptive solver that starts from the most complete formulation and reduces cost only where it can prove the accuracy; evidence recorded in `docs/fea-core.md` Phase 11)

## Problem

`fea-core` was a statics kernel. Frequencies, buckling loads and transient response need an eigensolver for the sparse
pencil `K phi = lambda B phi` and a time integrator, and the solver is to choose its own cheapest sufficient strategy
without ever trading accuracy for speed. The build constraint stands: pure Rust, offline, `win-x64`, no installed runtime.

## Decision

1. **Eigenproblems on the existing faer factorisation, written here** (`fea-core/src/dynamics.rs`). Shift-and-invert
   operator `T = (K - sigma B)^-1 B` (one Cholesky / LDL^T of `K - sigma B`, then back-substitutions), two interchangeable
   solvers, one verifier:
   * block subspace iteration with Rayleigh-Ritz (robust, ~`1.5 nev` solves per iteration);
   * Lanczos with full reorthogonalisation in the `B` (modal) or `K` (buckling) inner product, plus **deflated
     restarts that look for a missed copy of a multiple eigenvalue** (a single-vector Lanczos process can miss one; square
     columns, circular plates and every symmetric structure have them).
   * Every returned pair carries the backward-error residual `|K phi - lambda B phi| / (|K phi| + |lambda| |B phi|)`
     (less a `100 eps max(K_ii)` round-off allowance, which is all a rigid-body mode of a free structure can reach); a pair
     above the tolerance is an `Err`, never a result. Convergence behaviour of the iteration is never the acceptance test.
   * `EigMethod::Auto` = Lanczos, from `tests/bench.rs::modal_analysis_cost` (Phase 11): 1.2x (6 modes) to 2.6x (20 modes)
     faster than subspace iteration on 3k-11k dof solids with the completeness check included.
   * Buckling: the pencil `K phi = lambda (-K_G) phi`, `K_G` from the linear stress state of the reference load
     (plane stress / strain and 3D solids; axisymmetric refused, its hoop terms are not implemented). Only positive
     factors are returned; a reference load with none is an error.
2. **Rejected**: `ndarray-linalg` / ARPACK / BLAS bindings (break the offline, no-runtime requirement); `scirs2-sparse`
   (LOBPCG, IRAM, generalized pencils in pure Rust, but a very large dependency tree, its own sparse formats, and it
   cannot reuse our factorisation, which is the whole cost; also unproven for our verification requirement);
   `lanczos` / `eigenvalues` crates (no generalized pencils, no shift-and-invert, no completeness guarantee). LOBPCG with the
   AMG preconditioner remains the candidate when a 3D model is too big to factor (above `AUTO_ITERATIVE_DOFS`); not needed
   so far, and no consumer needs it.
3. **Transient**: Newmark average acceleration (`alpha = 0`) or HHT-alpha (`-1/3 <= alpha <= 0`), Rayleigh damping, constant
   step, one factorisation of the effective matrix. Validated against exact modal superposition of the same semi-discrete
   system: second order in `dt`, energy conserved to round-off when undamped, HHT never gains energy. Mass: consistent, or
   HRZ-lumped (positive for every element type; row-sum lumping gives negative masses for the serendipity ones).
4. **Strategy selection (the adaptive solver)**: a strategy is a *reduction* of a reference formulation (see
   `docs/fea-core.md` Phase 11): coarser tolerance, cheaper solver path, lower-order element, larger step. The selector
   (a) runs or estimates the reference quantity of interest, (b) applies the cheapest reduction a documented rule allows,
   (c) verifies it against the requested accuracy with an *independent* check (a-posteriori residual, ZZ error estimate,
   energy balance, a coarse reference run), (d) on failure restores the previous, more complete strategy. Every decision is
   a `SolveEvent` (`fea-core/src/report.rs`) and appears in the JSON report; nothing is substituted silently.
5. **Learning**: allowed only as a *prior over starting strategy* (initial step size, ordering, solver path), trained on the
   decision log, and only if the prediction costs less than the saving. A prediction never replaces the verification in (c);
   a wrong prediction costs one retry, not a wrong answer. A learned model is not shipped before it beats the rule-based
   selector on a held-out corpus.

## Outcome (2026-10-08)

* Lanczos with deflated restarts is the default; a rejected Lanczos run falls back to subspace iteration and says so (`Modal::fallback`).
* Buckling uses a shifted operator with a Cholesky-proven admissible shift (bending states give factors of both signs; the unshifted pencil did not converge).
* The learning question was answered by measurement (`docs/fea-core.md` Phase 11, "Machine learning: evaluated before it was built"): no learned component.
* Sensitivities were built by the adjoint route described below, checked against global finite differences; the optimiser is an optimality-criteria sizing method over block moduli.

## Consequences

* New code in `fea-core`: `dense.rs`, `dynamics.rs`, `report.rs`; `Block::density`, `Mesh::set_density`.
* The eigensolver is as large as the factorisation allows (a 3D model that does not factor does not get modes); the
  iterative path is not used for indefinite or shifted systems.
* Sensitivity analysis (adjoint for linear statics: `dJ/dp = -lambda^T (dK/dp u - df/dp)`, one extra solve per functional)
  is the chosen route when it is built; it needs element-matrix derivatives that are cheap by finite difference of the
  element kernel and exact for shape parameters only through the kernel Jacobians. Not built in this ADR's scope.

## Source-status addendum (2026-10-09, issue #12)

The original decision and consequences above are retained as history. At the audited commit
`d5748135188cd6f196a2f6310ea9934251260f93`, sensitivities **are implemented** in `sensitivity.rs`
(linear displacement/compliance and modal eigenvalues), and `optimize.rs` implements block-modulus
sizing by optimality criteria. `tests/sensitivity.rs` checks against global finite differences;
`tests/optimize.rs` checks the series-bar closed form. Coordinate derivatives currently use central
differences of the element kernels, not an exact symbolic shape derivative.

The strategy decision describes the intended programme, not a universal implemented reduction
engine. `strategy.rs::solve_adaptive` selects direct/iterative linear statics and verifies residual,
force balance and an optional ZZ requirement. `solve_nonlinear_ladder` requires a caller-provided
acceptance check; it does not independently supply all physical checks. Eigen acceptance and fallback
live in `dynamics.rs`. Mesh refinement remains the caller-owned `adapt::refine` loop. There is no
automatic element-order or time-step reduction shared across all analyses.

Heat conduction is implemented separately in `thermal.rs`; a nodal temperature field can drive
structural loads and stress recovery. This is **one-way** coupling, not a generalized coupled field/DOF
system. Phase 12 audit, checks and follow-up are recorded in [issue-12-phase-12.md](../issue-12-phase-12.md).

## Roadmap implementation addendum (2026-10-10, issue #12)

On `codex/fea-core-roadmap`, `fields.rs` now supplies generalized translational/rotational/thermal
mapping, constraints and sparse symmetric assembly; `coupling.rs` explicitly supplies one-way
thermal-to-structural transfer. This does not claim monolithic/two-way multiphysics.
`strategy.rs::solve_refined` orchestrates bounded verified linear solves with a caller-owned mesher;
`solve_nonlinear_verified` adds strict completed/finite/residual/caller-physics acceptance to a
bounded strategy ladder. `report.rs::AcceptanceReport` provides analysis adapters and machine-readable
finite acceptance records. Effective residuals accompany structural transient histories; energy
checks remain conditional on the load/damping regime. No universal time-step/order adaptation or
learned production selector is claimed. Independent benchmarks and limitations are in
[Phase 13](../issue-12-phase-13.md), [Phase 15](../issue-12-phase-15.md),
[Phase 17](../issue-12-phase-17.md) and the final Claude Code handoff.
