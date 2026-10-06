# ADR-012: General FEA kernel (`fea-core`) alongside the special-purpose solvers

Status: Accepted (direct user direction)

## Problem

`lug-solver` is a validated nonlinear contact FE but special-purpose: one lug outline, a hardcoded star
O-grid, 2D only, a rigid analytic pin, fixed boundary conditions, a lug condensed onto its bore dofs.
The goal is a general finite-element kernel (arbitrary 2D/3D geometry, loads and supports, nonlinear
materials and contact), with accuracy and speed first.

## Decision

* New crate `fea-core/`: elements (2D and 3D), assembly, solvers, loads, post-processing. `lug-solver` and
  `edge-check` keep their validated kernels until `fea-core` matches them on accuracy (all existing tests)
  and speed; `lug-solver` then becomes a preset on top of it.
* **faer 0.24 (pure Rust) + rayon** for sparse Cholesky and parallelism. Rejected: hand-written sparse
  Cholesky (lug-solver's `sparse.rs`, 2.2-2.4x slower at 20k-80k dofs, equal at 3k, measured in
  `fea-core/tests/bench.rs`); BLAS/LAPACK/MKL bindings (breaks "no runtime installed, offline win-x64").
* 3D geometry: extrusion/revolution of 2D meshes plus Gmsh/Abaqus mesh import; no native tetrahedral mesher
  (months of work on its own).
* 2D meshing is native (`delaunay.rs`): conforming Delaunay + Ruppert refinement with a size function and exact
  curve midpoints on arcs, written here because it needs a spatial size field, arc-aware boundary splitting
  and adaptive remeshing from the ZZ estimate. It uses the pure-Rust `robust` crate (Shewchuk's exact
  `orient2d` / `incircle`, no dependencies) so degenerate inputs cannot corrupt the triangulation.
  Rejected: a general CDT crate (uniform area limit only, chord-split boundary) and hand-rolled floating-point
  predicates.
* Library first; a general FEA toolbox in `app-tui` is a later, separate step.

* Material nonlinearity: the hardening law lives in `mechanics-core` (`Hardening`), one copy shared by
  `lug-solver` (which re-exports it) and `fea-core`, avoiding a dependency cycle. Finite strain is the
  logarithmic-strain Hencky J2 of the existing `lug-solver/finite.rs` with an *analytic* consistent
  tangent (divided differences of the isotropic tensor function) replacing its forward-difference one;
  the old finite-difference tangent remains only as the test oracle.

* Contact is Gauss-point-to-surface with rigid analytic or deformable masters, augmented Lagrangian
  normal and Coulomb friction, tangent by central differences of each point's closed-form local
  residual (consistent without hand-derived second variations). Rejected for now: mortar segment
  integration (large) and full unsymmetric Alart-Curnier Newton (needs an unsymmetric factorization;
  the symmetric quasi-Newton treatment with moderate penalties is validated against Hertz, Lame and
  Coulomb sliding). The tangent of an active point differentiates the smooth branch (a central
  difference of `max(0, .)` halves the stiffness of lightly loaded points).

* Rigid masters can translate freely: the translation components are extra unknowns after the mesh dofs,
  condensed by a Schur complement in the Newton step (a force-controlled pin).

* Phase 5 outcome: `lug-solver` depends on `fea-core` (`FeaLug`), not the reverse. The kernel reproduces the
  condensed solver (frictionless travel to <1e-4, finite-strain collapse to 0.04 %, meshed elastic pin
  to 1e-4) but is 40-300x slower on the lug, so the speed gate for retiring the condensed / finite solvers
  failed and they remain the interactive default.

## Consequences

* Two solvers coexist for the lug (Phase 5 of `docs/fea-core.md`): the condensed fast path is the
  interactive default and `fea-core` is the independent cross-check; revisit only if the kernel's lug
  solve gets within a small factor of the condensed one.
* New external dependencies (faer, rayon, dyn-stack, robust and their trees) in a crate the TUI does not yet use.
  Build-time only: the built app stays self-contained.

## Update (kernel as the main lug solver)

The gate "match accuracy and speed before switching" was met for the interactive use: the kernel matches the
condensed solver (Lame press fit exact, NACA TN 1503 reproduced, the condensed answers within 1-3 % on hoop stress,
travel and collapse) and, after the speed work of `docs/fea-core.md` Phase 7 (a NaN friction tangent, a soft penalty,
sequential faer for small systems, a 2D plastic tangent), solves the toolbox's default cases in 0.3-10 s on a worker
thread. `lug-solver`'s condensed and finite-strain solvers are kept, not deleted, as a user-selectable comparison
(`Solver: Legacy` / `Compare`) and as the fallback for the one case the kernel cannot run. Consequence: the kernel
is the reference for friction (path-resolved, penalty independent) and its NACA statistics differ from the legacy
solver's by about 3 % (documented where the toolbox quotes them).

