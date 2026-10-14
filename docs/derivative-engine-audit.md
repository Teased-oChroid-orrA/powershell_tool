# Derivative engine: Phase 0 audit (epic #39)

What derivative-producing code `fea-core` had before the epic, what the new engine reuses, and the measured baselines. Issue #27.
Design and use: `docs/derivative-engine.md`. Spec: `docs/epic-derivative-engine.md`.

## Derivative-producing paths found

| Area | Where | How it differentiates | Verdict |
|---|---|---|---|
| Linear static parameter sensitivities | `fea-core/src/sensitivity.rs` | direct / adjoint solve; `dK/dp` by element-level **central differences** for coordinates, closed form for modulus scale | coordinate derivative was semi-analytic (step-dependent, ~1e-6 relative); kept as an independent reference. Replaced for exactness by `derivative::linear_sens` (dual numbers through the element Jacobian) |
| Plasticity tangent | `material.rs` `J2` | hand-derived consistent tangent of the radial return | exact for J2 + table hardening; kept, now an `Analytic` registry entry and the reference for the AD tangent |
| Nonlinear solve | `nonlinear.rs` | Newton with that tangent; contact tangent with a small unsymmetric friction defect (`K + D`) | correct; the defect blocks are what the new `TangentSolver` consumes for transposed solves |
| Goal-oriented adaptivity | `adaptivity/{dwr,enrich,nonlinear}.rs` | adjoint solves with the (symmetric) tangent, own Cholesky/LDLT calls | duplicated the tangent-solve logic; now routed through `derivative::linalg::TangentSolver` (commit `b87841b`) |
| Transient | `dynamics.rs` | Newmark / HHT-alpha, one factorisation of the effective matrix | no sensitivities existed; added in `derivative::transient` |
| Modal / buckling | `dynamics.rs` | eigen solvers | eigenvalue AMR uses its own error estimate; eigenvalue *sensitivities* are out of scope (see limits) |
| Sparse transposes | `assembly.rs` `BlockMatrix` | stored as the symmetric lower triangle | transposed solves on a symmetric K are free; the unsymmetric defect is handled outside the matrix |

No automatic differentiation, no complex step, no hyper-dual, no matrix-free tangent, no Hessian, no path-dependent adjoint existed.

## Reused

Element kernels (`element.rs`, `kernel.rs`) as the reference for the generic `Scalar` element code (differential tests against them), `Reduced`
(ordering, factorisation), `loads::assemble`, `Model::df_dp` (geometry dependence of surface loads), `J2` tangent, `adaptivity::run_amr*`.

## Baselines (this machine, release build; `cargo test --release -p fea-core --test derivative_bench -- --nocapture`)

| Measurement | Value |
|---|---|
| Quad8 element stiffness, plain `f64` | 9.9 us |
| same with `Dual<f64,1>` / `x4` / `x8` | 13.8 / 23.0 / 32.6 us (a lane adds ~0.3 of a plain evaluation) |
| same recorded on the reverse tape | 141 us, 22 024 nodes, 24 B per node (529 KB); one sweep 120 us |
| complex step / hyper-dual | 1.75x / 3.3x a plain evaluation |
| `dK/dE`, AD error vs closed form `K/E` | 2.6e-16 (central difference, h = 1e-4 E: 9.9e-13) |
| allocations per Quad8 stiffness | 11 plain, 11 with dual numbers (lanes live on the stack) |
| plastic plate, 982 dofs, 4 steps, 5 parameters | one solve 25 ms; adjoint sweep 17 ms; direct 20 ms; central differences 250 ms (2 solves per parameter) |
| matrix-free tangent, 3384 dofs | assembled matvec 158 us (317 KiB); cached 120 us (796 KiB); recompute 712 us (129 KiB) |

The policy's `CostCalibration` defaults were refit from these numbers (tape recording was assumed 2x and is ~13x).

## Findings fixed on the way (Fix-Everything)

* the biactive LCP of the unilateral contact sensitivities had a sign error in the dual-feasibility term (found by the finite-difference test: 7 % off);
* the regularised-contact Newton judged its residual against the applied force, which is zero for displacement-driven problems (never converged);
* an always-true condition in `matrix_free.rs`.
