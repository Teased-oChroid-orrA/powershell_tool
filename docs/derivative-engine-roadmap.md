# Derivative engine: follow-up work (not built in epic #39)

Epic #39 shipped in PR #40. These are the limits documented in `docs/derivative-engine.md`; each is a separate, later piece of work. Order is a
suggestion (cheapest and most reused first). Each item needs the same discipline as the epic: exact derivatives of the *computed* response, an
independent reference (finite differences with Richardson steps, closed forms), direct-versus-adjoint agreement, explicit refusal outside the supported
range, and an entry in the support matrix.

- [ ] **Eigenvalue and buckling-factor sensitivities.** `d lambda / dp = phi^T (dK/dp - lambda dK_G/dp) phi / phi^T B phi` for simple eigenvalues; clustered or repeated eigenvalues need the directional (generalised) form and must be flagged. Building blocks exist: exact `dK/dp`, `dM/dp`, and the generic geometric stiffness (`element::geometric_stiffness`). The stress dependence of `K_G` on the parameter (through the prebuckling solve) needs the linear-static adjoint already available.
- [ ] **Transient sensitivities with thermal loads.** The transient and path engines currently refuse thermal loads. Extend with the exact thermal-load derivatives of `linear_sens` (time-dependent temperature, `f(t) = g(t) f_th`).
- [ ] **Coupled thermal-structural geometry and thickness derivatives.** Geometry changes both the conduction matrix and the structural problem; add `dK_T/dx` (exact, generic conduction kernel over `Scalar`) and the chain into the thermal load.
- [ ] **Temperature-dependent conductivity.** Nonlinear conduction: differentiate the converged Picard/Newton fixed point (implicit differentiation with the conduction tangent).
- [ ] **Large-strain plasticity paths.** Finite-strain J2 return map as a `DifferentiableMaterial` (the neo-Hookean kernel shows the pattern); path engine with the deformation-gradient tangent and updated-Lagrangian state.
- [ ] **Transient sensitivities through contact and plasticity.** Nonlinear transient integrator sensitivities with event detection: tag each step with its contact/yield status, propagate the active-branch derivative, report `Generalized` semantics across a status change, refuse the adjoint at a biactive point (the unilateral machinery exists in `complementarity`).
- [ ] **Second derivatives with surface and thermal loads.** Needs an exact (not central-difference) load-geometry derivative: generic `Scalar` face-load kernels.
- [ ] **Anisotropic elasticity in the exact element derivatives.** Generic `Scalar` anisotropic constitutive matrix in `element::stiffness`, `thermal_load`.
- [ ] **3D matrix-free plasticity.** Matrix-free tangent for 3D elements and the 3D plastic kernels (`MatrixFreeTangent` is 2D now).
- [ ] **Models `fea-core` does not have** (viscoelasticity, damage, kinematic hardening): add the material first, then register it as a `DifferentiableMaterial`.
- [ ] **Explicit SIMD**, only if a measurement on the Windows target shows the auto-vectorised dual lanes (cost 0.33 of a plain evaluation per lane) are the bottleneck.
