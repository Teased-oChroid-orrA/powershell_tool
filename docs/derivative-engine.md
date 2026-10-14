# Unified adaptive derivative and sensitivity engine

Epic #39 (stories #27-#38), code in `fea-core/src/derivative/`, branch `feature/adaptive-derivative-engine` (draft PR; nothing is merged to
`main` without approval). Spec: `docs/epic-derivative-engine.md`. Audit and baselines: `docs/derivative-engine-audit.md`.

## Architecture

| Layer | Modules | Role |
|---|---|---|
| Scalars | `scalar` (trait `Scalar`), `dual` (`Dual<T,N>`, nestable), `hyperdual`, `complex`, `tape` (`Tape<T>`, `Rev`) | one generic kernel body, many derivative modes |
| Kernel drivers | `ops` (`jacobian_forward`, `jvp`, `vjp`, workspaces), `verify` (`Probe`, Taylor tests, smoothness classification) | derivative of a `SmoothKernel`; refusal of non-analytic maps for complex step |
| Policy | `policy` (registry, `DefaultPolicy`, `select_sensitivity`), `kernels` (`DerivativePolicyEngine::with_fea_kernels`) | which method for which kernel, with a reported reason, a cache and a trace |
| Constitutive | `constitutive` | return maps differentiated by AD, implicit differentiation of converged local systems, plane-stress condensation, trial vs committed |
| Elements | `element` | stiffness / mass / geometry generic over `Scalar` for every element kind |
| Linear systems | `linalg` (`TangentSolver`, CG, GMRES, operators), `matrix_free` | factorised / Krylov solves, transposed solves through an unsymmetric defect |
| Global sensitivities | `linear_sens`, `path`, `transient`, `second_order` | linear static, path-dependent plasticity (direct + discrete adjoint), Newmark/HHT-alpha (direct + checkpointed adjoint), Hessians |
| Contact | `contact`, `complementarity` | contact states, exact tangent, penalty-contact sensitivities, unilateral contact by a primal-dual active set, directional derivatives at biactive points, softplus-regularised law |

Method choice is data driven: each kernel registers `DerivativeCapabilities` (requests, order, smoothness, scalars, thread safety, determinism,
exactness); `DefaultPolicy` filters by compatibility (smoothness, verification-only, memory budget, `verify_against`) then takes the cheapest by
calibrated cost. An explicit override is honoured only if compatible. `select_sensitivity` picks direct / adjoint / matrix-free from parameter count,
output count, step count and factorisation reuse. `PathSensitivityEngine::sensitivities` applies it.

## Support matrix

| Capability | Status | Verified by |
|---|---|---|
| Forward / reverse / hyper-dual / complex step | all element kinds, J2 and hyperelastic kernels | `derivative_core`, `derivative_linear`, `derivative_constitutive` |
| Linear static sensitivities (modulus, nu, thickness, coordinates), exact shape derivatives, hybrid meshes | yes (surface-load geometry dependence stays a central difference of the load assembly) | `derivative_linear` |
| Path-dependent small-strain J2 plasticity, direct and discrete adjoint, Recompute/Store Jacobians | Tri/Quad 2D, plane strain / stress | `derivative_path` (vs finite differences 2e-5) |
| Matrix-free tangent (cached / recompute), CG / GMRES / Newton-Krylov | yes, bitwise deterministic over thread counts | `derivative_operators`, `derivative_bench` |
| Penalty contact incl. friction (unsymmetric tangent) | fixed active set exact; transitions flagged | `derivative_contact` |
| Unilateral contact | classical derivative on a strictly complementary branch; one-sided directional derivatives at biactive points (LCP); adjoint refused at a kink | `derivative_complementarity` |
| Regularised (softplus) contact law | AD differentiable; converges to the unilateral solution as rho -> 0 | `derivative_complementarity` |
| Transient Newmark / HHT-alpha (consistent and lumped mass, Rayleigh) | direct, adjoint, checkpointed adjoint (bitwise equal to stored) | `derivative_transient` |
| Hessian-vector products and Hessians (linear static) | forward-over-reverse and reverse-over-forward agree; nodal loads only | `derivative_second_order` |

## Semantics and limits (read before trusting a number)

* Derivatives are of the **computed** response (discrete). They agree with finite differences of the same computation to the finite-difference
  accuracy (1e-5 to 1e-9 relative in the tests), and with each other (direct vs adjoint) to ~1e-9..1e-12. There is **no** universal 1e-15 claim; AD of a
  linear dependence is exact to rounding (measured 2.6e-16), a Newton solve converged to 1e-13 limits the rest.
* `DerivativeSemantics`: Classical / ActiveBranch / Generalized / Approximate. Across a contact status change, a yield-surface kink or a friction-cone
  boundary the response is not differentiable; the engine returns the active-branch derivative with a flag or the one-sided derivatives, never a silent
  classical one.
* Out of scope (documented, not attempted): eigenvalue / buckling sensitivities, large-strain plasticity paths, transient sensitivities through contact or
  plasticity, second derivatives with surface/thermal loads, anisotropic elasticity in the exact element derivatives, 3D matrix-free plasticity.
* Contact adjoint for frictional contact is available through the unsymmetric tangent solve of the penalty problem; the unilateral problem's adjoint is
  frictionless only.

## AD implementation decision

Own implementation (no new dependency). Considered: `num-dual` (forward only, fixed width, no tape or reverse mode), `autodiff`/Enzyme (nightly LLVM
plugin; cannot satisfy the stable-toolchain, offline-build, Windows constraints), tape-based crates (global tapes conflict with the required local,
bounded, thread-safe tape and deterministic memory). The generic `Scalar` trait keeps the kernels backend-agnostic, so a library can still be adopted
behind it. Measured cost and memory: `docs/derivative-engine-audit.md`.

## Configuration and diagnostics

`DerivativeContext` (smoothness, memory budget, purpose, determinism, override, verification set, difference fallback), `CostCalibration` (refit it with
`derivative_bench -- --nocapture`), `DerivativePlan::report()` (method, cost, memory, reasons, rejected alternatives), `DerivativePolicyEngine::drain_trace()`,
`PathReport` / `TransientReport` / `LinearReport` (solves, factorisations, stored bytes).

## Reproducing

`cargo test --release -p fea-core --lib` plus `--test derivative_{core,constitutive,path,linear,operators,contact,complementarity,transient,second_order,bench}`.
