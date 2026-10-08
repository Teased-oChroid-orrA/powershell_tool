# fea-core

> TL;DR: General finite-element kernel: 2D (plane stress / plane strain / axisymmetric) and 3D solid elements (Tri3/6, Quad4/8/9, Tet4/10, Hex8/20/27), colour-parallel symmetric block assembly, Dirichlet elimination, faer sparse Cholesky (AMD or geometric nested dissection) or PCG + smoothed-aggregation AMG for large 3D, multi-point constraints and reference-point rigid coupling, isotropic and anisotropic elasticity, point / surface (constant or field) / body / thermal loads, nodal stress (averaged or SPR) with a ZZ error estimate, boundary-face extraction, `.vtu` writer, structured mesh generators. Pure function of inputs; no file I/O, no UI. Plan, validation and measured speed: `docs/fea-core.md`, decision: `docs/adr/ADR-012-general-fea-kernel.md`.

## Purpose
Owns: element library, element kernels, assembly, linear solve, loads, stress recovery, mesh generators.
Does not own: any UI, material tables (`mechanics-core`), the lug physics (`lug-solver`: `FeaLug` in `lug-solver/src/fea*.rs` runs the lug on this crate and is the toolbox's main solver; its condensed / finite-strain solvers are the legacy comparison, see `docs/fea-core.md` Phase 7). `lug-solver` depends on this crate, never the reverse. The `app-tui` FEA Workbench (problem definition, mesh preview, contours) is `fea-problem` + `app-tui/src/toolboxes/fea_workbench/` (`docs/fea-core.md` Phase 8).

## Code Map
| Looking for... | Go to |
|---|---|
| Shape functions, Gauss rules, tabulated per element type (VTK node order) | `src/element.rs` |
| Geometry (Jacobian, gradients), stiffness (node-pair and fast component-major), thermal/body loads, strain/stress at a point | `src/kernel.rs` |
| Sparsity pattern, element colouring, parallel scatter assembly, symmetric matvec | `src/assembly.rs` |
| Dirichlet elimination, symbolic + numeric Cholesky (faer), solve | `src/linear.rs` |
| Geometric nested dissection (2D/3D) | `src/ordering.rs` |
| Point loads, edge/face traction and pressure, body force, thermal | `src/loads.rs` |
| `Model` (mesh + pattern), `solve_static`, `solve_static_many` (several cases, one factorisation), `check_constrained`, Gauss and nodal stresses | `src/analysis.rs` |
| Boundary faces / edges of any element mix in load orientation | `src/topology.rs` |
| SPR nodal stress recovery, ZZ error estimate, true-error energy for verification | `src/recover.rs` |
| Multi-point constraints, reference-point rigid coupling, constrained solve | `src/mpc.rs` |
| CSR kit; smoothed-aggregation AMG + PCG | `src/sparse.rs`, `src/amg.rs` (selected by `SolveMethod` in `analysis.rs`) |
| `.vtu` text writer | `src/vtu.rs` |
| Stress at an arbitrary point (`Model::locator`, `Locator::stress_at`) | `src/locate.rs`, `tests/locate.rs` |
| Nonlinear material points (small / finite strain J2, plane stress) | `src/material.rs` |
| Nonlinear assembly, Newton / arc-length driver, `NlState` | `src/nonlinear.rs` |
| Contact: rigid / deformable masters, AL normal + Coulomb friction, BVH, free rigid translation / (2D circle) rotation, traction output (`NlSolution::contact_tractions`) | `src/contact.rs` (`Model::solve_nonlinear_contact`) |
| Follower (non-dead) pressure with its load stiffness | `src/loads.rs::follower_face`, applied in `Ctx::eval` (`nonlinear.rs`) |
| Anisotropic elastic-plastic update (small strain), thermal expansion tensor | `src/material.rs::small_strain_update_aniso`, `src/mesh.rs` (`Aniso::alpha`, `Elastic::with_alpha_tensor`, `thermal_strain`) |
| Bushing-in-housing interference-fit contacts + the speed/tightness `Tuning` shared by `lug-solver` and `eccentric-bushing` | `src/fit.rs` |
| 2D region geometry (lines + arcs, holes, named segments) | `src/geometry.rs` |
| Delaunay + Ruppert triangulation with a size function | `src/delaunay.rs` |
| Region -> Tri3/Tri6/Quad4/8/9 mesh with curved boundary nodes, named sets and surfaces | `src/mesh2d.rs` |
| ZZ-error-driven size field for adaptive remeshing | `src/adapt.rs` |
| Extrude / revolve quad meshes into hex meshes | `src/sweep.rs` |
| Gmsh MSH and Abaqus INP import | `src/import.rs` |
| Structured grids of any element type through a coordinate map (curved boundaries, node sets, surfaces) | `src/generate.rs` |
| Closed-form validation (patch tests, Lame plane strain/axisymmetric/3D, thermal, reactions) | `tests/linear_static.rs` |
| Timoshenko cantilever, Kirsch plate, pressurised sphere | `tests/classic.rs` |
| SPR exactness, ZZ effectivity | `tests/recovery.rs` |
| MPC / rigid coupling vs beam theory | `tests/mpc.rs` |
| Iterative solver vs direct, mesh-independent iteration counts | `tests/iterative.rs` |
| Anisotropic law (equivalence, rotation, patch, thermal, bending, frame invariance) | `tests/anisotropic.rs` |
| Nonlinear: uniaxial analytic, thick cylinder / sphere limit loads, Hill, finite-strain collapse | `tests/nonlinear.rs` |
| Contact: Hertz 2D/3D, friction, shrink fit, patch test, contact + plasticity | `tests/contact.rs` |
| Unstructured meshes vs Lame / Kirsch, multi-hole regions | `tests/mesh2d.rs` |
| Adaptive vs uniform refinement, size field | `tests/adapt.rs` |
| Sweeps vs 2D / axisymmetric solutions | `tests/sweep.rs` |
| Gmsh / Abaqus import and round trip | `tests/import.rs` |
| Speed baseline (ignored tests; the faer-vs-lug-solver gate lives in `lug-solver/tests/sparse_speed_gate.rs`) | `tests/bench.rs` |

## Contracts
- Long contact solves are steered through `NlOptions`: `interrupt` (`Interrupt`: wall-clock deadline and/or shared `AtomicBool`, checked at every Newton iteration and step; ends with `Stop::Interrupted`, not `complete()`), `observer` (`StepObserver`, called after every converged load step with a `StepEvent`: live progress). `NlOptions` is `Clone`, not `Copy`.
- `stick_slip_guard` (on in `Tuning::friction()`, off elsewhere) is for frictional contact whose Newton iteration cycles. **Root cause found (eccentric bushing, 2026-10-08)**: points at the edge of a contact patch, at ~zero pressure, enter and leave the contact from one iteration to the next (`NL_TRACE` shows the active set of the first contact changing between iterations); each flip jumps the residual by more than the tolerance, so Newton alternates forever, and which way it falls depends on round-off (the case passed on macOS and failed on Linux CI). When 8 iterations bring no 3 % gain of the best residual the solve **holds the active set** (`ContactSet::hold_active_set`: a point is in contact iff it was at that iterate, its pressure following the smooth branch; released when the solve ends) and gets 15 more iterations: every case that had failed all attempts (fine mesh, concentric thick wall, soft parts, the reported 8856 lbf case) then converged on the first attempt, 2-10 x faster. Only if that also stagnates: with `stall_tol` the lowest iterate is accepted when its residual is below `stall_tol` x force scale (`NlSolution::stalled_solves`; the force scale includes large constant forces, so check what the answer must still satisfy), else the solve fails (`STAGNATED`, 3 step cuts). `step_memory` starts each Newton step from the last accepted backtrack fraction: it rescued one case and ruined another: a later attempt's strategy, never a default.
- Node order is VTK (corners, edge midsides, face centres, volume centre); a mesh writes to `.vtu` without renumbering. Boundary faces/edges are listed counter-clockwise seen from outside (2D: body on the left): `Pressure(p)` pushes into the body.
- Units are free (the kernel is unit-free); conventionally inch / psi / lbf. Plane problems carry a thickness in `Physics`; axisymmetric loads and stiffness are for the full 360 degrees (`x` = radius).
- Isotropic law is `sigma = lambda tr(eps) I + 2 mu eps` in every case; plane stress uses `lambda* = E nu / (1 - nu^2)`. Thermal: `sigma_th = -(3 lambda + 2 mu) alpha dT` (plane stress: `2 lambda* + 2 mu`).
- Matrices are the **lower block triangle** of `d x d` node blocks (`BlockMatrix`, `Pattern`); a node no element touches still gets a diagonal block and must be constrained.
- Assembly writes disjoint blocks per colour class through a raw pointer (`assembly.rs`, one `unsafe` block): do not break the invariant "elements of one colour share no node".
- Constrained dofs are eliminated, never penalised (Dirichlet in `linear.rs`, multi-point constraints by `T^T K T` in `mpc.rs`; extra dofs `>= mesh.n_dofs()` carry reference-point translations/rotations and have no stiffness of their own).
- Anisotropic `Elastic` (`aniso: Some`) uses the B-matrix kernel and `Layout::NodeBlock`; `e`/`nu` are then only a Voigt-average display value, so never read `mat.e`/`mat.nu`/`lambda()`/`mu()` outside the isotropic branches (`stiffness`, `stiffness_fast`, `thermal_load`, `stress_from_strain`, `complementary_energy` all branch on `aniso`). Plane problems need a material symmetric about `z`; axisymmetric axes are `(r, z, theta)`.
- `solve_static` / `solve_static_many` refuse a model that leaves a rigid-body motion of any connected piece free (`Model::check_constrained`: the rigid modes against the constrained dofs): a singular system can factorise anyway and return the answer plus an arbitrary rigid motion. A node no element touches is ignored by the check and still needs its own constraint. Reference-point / MPC solves (`mpc.rs`) and the nonlinear driver do their own constraining and do not call it.
- `SolveMethod::Auto` factors (faer) except 3D above `AUTO_ITERATIVE_DOFS` (150k free dofs), where it runs PCG + AMG; an unconverged iteration is an `Err`, never a wrong answer.

## Pitfalls
- Serendipity and quadratic faces: **consistent nodal forces of a uniform traction are not equal** (corners carry negative share on Quad8/Hex20). Apply end loads as `SurfaceLoad::Traction` on a face, never as equal nodal forces (a test did this wrong and showed +4.6 % tip deflection).
- Plane strain free thermal expansion is `(1 + nu) alpha dT`, not `alpha dT` (`eps_zz = 0`).
- Hex8 under self weight is only exact to ~5e-7: the exact field needs `x^2` terms trilinear elements lack.
- Tri3 (CST) and Tet4 are poor on pressure-driven problems (Lame at 16x16: 2 % displacement error); they are in the library for meshers, not recommended for results.
- Ordering: AMD wins in 2D; geometric nested dissection wins 2-3x on trilinear hexahedra, ties on quadratic hex/tets. `Ordering::Auto` analyses both in 3D and keeps the sparser factor. `faer` solve is parallel by default (global rayon parallelism).
- Dev profile is optimised (`[profile.dev.package.fea-core]` in the root `Cargo.toml`).
- Speed: element stiffness is not the bottleneck (< 1 % of a 3D linear solve); the factorization is. A register-tiled GEMM-style stiffness kernel and `pulp` runtime SIMD dispatch (x86 Windows builds are SSE2-only by default) are deferred until the Phase 3 nonlinear loop re-assembles every Newton iteration and a profile shows assembly matters.

- SPR needs `nt + 2` samples per patch (a two-triangle Tri6 patch has 6 samples for 6 unknowns and interpolates noise); boundary nodes take an interior neighbour's polynomial (one-sided boundary patches are inaccurate). The ZZ estimate is only meaningful (effectivity ~1) on smooth solutions; near a stress concentration the coarse-mesh estimate overshoots.
- Mesh grading toward a hole: an exponent above ~1.2 makes the Jacobian vanish at the hole edge and ruins the first element ring (found validating Kirsch).
- AMG: 2D and strongly anisotropic-aspect meshes converge slowly (~100 iterations); a strength-of-connection filter cuts iterations but raises setup (coarsening stalls into a big coarse factorization), so default is no filter. `AmgOptions` is the tuning surface.
- Test-design lesson: a quadratic displacement field is only an FE-exact solution if its `-div(sigma)` is applied as body force.

- Meshing: boundary vertices come from the geometry's curves (not the chord); `Region` outer loop is CCW and holes CW whatever order is given. The mesher conforms segments by splitting (never by flipping), so every boundary subsegment is a triangle edge; a line piece splits at the midpoint of its actual chord, an arc piece at the curve point. Ray casting must use one closed polyline (resampling each segment independently leaves a rounding gap that swallows a crossing).
- Quad meshes come from splitting each triangle into three quads (Catmull-Clark style): quad edge ~ half the triangle edge, so `adapt::adapted_size_field(.., quad_split = true)` triples the element area to recover the triangle size.
- Importers verify quadratic node order geometrically (midside nodes within 0.3 element sizes of the position their corners imply); keep that check when adding element types, and build test files from the format's own edge/face definitions, not from the permutation table under test.
- Hex sweeps cannot touch the revolution axis (`x = 0` nodes would create zero-length edges); use axisymmetric elements there.

- Nonlinear: `Block::plasticity` (`J2`) switches a block to the plastic path; the linear `solve_static` ignores it. Elastic blocks in a nonlinear model use the linear kernels (small strain), so a *finite-strain* analysis needs `J2::elastic_finite()` on its elastic blocks. Stress/tangent conventions: `S` = Cauchy (small) / first Piola-Kirchhoff (finite), `A = dS/dH` as a 9x9 `[3i+J][3k+L]`; the 4th-order tangent contracts with the shape-function gradients (plus the hoop entry `u_r / r` for axisymmetry). Plane-stress convergence of `sigma_zz = 0` must be scaled by the *elastic* stress scale, never by the yield stress (an elastic law has `sigma_y = 1e30`; that bug shipped to the first test run).
- Displacement control needs the tangent predictor (done in `Ctx::newton`); arc-length uses a secant predictor (the sign of `K^-1 f` is meaningless where the tangent is singular) and falls back to LDL^T when Cholesky fails; load control must NOT fall back (an indefinite tangent there is an unstable equilibrium and Newton would converge to it).
- The nonlinear driver factorizes with direct faer only; do not point it at the AMG path without a plan for indefinite tangents.

- Contact: a slave point's gap/normal come from the exact master (rigid analytic or Newton projection onto the quadratic face), weights from the *reference* slave surface. The collocation rule matters: full Gauss on quadratic edges oscillates (over-constrained), nodal is smooth against rigid masters in 2D and 3D, reduced Gauss is right for deformable masters (nodal fails the patch test there). Mean pressure, transmitted force and contact extent are reliable; pointwise Gauss-point pressures on non-matching deformable meshes scatter.
- Friction: keep `eps_n`, `eps_t` near `E / h`; the symmetrised sliding tangent is indefinite, so contact analyses factor with LDL^T fallback and freeze the sliding friction in the tangent. The slip history must be seeded for points touching at the start (`initial_state`) and committed for points within the activation gap, not only active ones, or the first step transmits no friction.
- Contact needs load control; rigid masters move by `shift + lambda * travel`; the matrix pattern is extended once with `ContactSet::pattern_pairs` from the initial proximity (`margin`): a contact reaching a node pair outside it is an `Err`.
- Free rigid translations (`RigidMaster::with_free_translation`): the translation components are extra unknowns after the mesh dofs; `ContactSet::eval` fills `ExtraOut { rq, kuq, kqq }` and `Ctx::newton_step` condenses them with a Schur complement over the existing factorization of `K_uu`. Residual sign: `r_y = -(force on slave)`, force on master `= sum r_y`, `R_q = sum r_q - lambda load` with `r_q = -r_y`. A free body with no contact is singular (error), so start it touching (`shift`). The first tangent of a step uses the near-contact stabiliser and is deliberately not consistent; every later one is exact.
- **Never take a central difference of `max(0, .)`** (found cross-checking the lug): the contact tangent differentiates the smooth active branch (`PointCtx::smooth`). Use `NL_FDCHECK=1` (`NL_FDH`, `NL_TRACE`, `NL_DEBUG`) to compare any analysis' tangent with finite differences; expect ~1e-11 from the second iteration on.
- `Loads::block_body` loads one block only (a pin inside an assembly of bodies); `Loads::body` loads all blocks.
- Editing hazard (cost one lost test module): never replace a file tail with `s[:i] + new` when the original had content after `i`; check `git diff --stat` / test counts after scripted edits.

- **Contact additions (lug integration)**: `ContactSpec::with_overlap` (an interference fit: gap = projected gap - overlap, deformable masters); `solve_nonlinear_contact_from` + `Start` (continue from a converged state, e.g. the fit, with a shorter displacement vector for bodies added later); `Loads::ground` (weak springs `(dof, k)` that hold a body only contact holds; leak `k u`); `NlOptions::{stop_on_plateau, stop_on_fall, stall_tol, first_step}`. `stop_on_plateau` reads travel not steps (`load_flat`: gained < 1.2 % of the top over the last 5 % of the travel; also a failed step at < 4 % gain is a limit state), because tiny post-cut steps made any load look flat.
- **Friction tangent**: a smooth-branch evaluation can see `p < 0` at a point on the active-set boundary: the capacity is `max(mu p, 0)` (a negative one flipped the traction and 0/0 made the tangent NaN: frictional runs near full load died with `ZeroPivot`). `factor_ldlt` retries an exactly zero pivot with a `1e-10..1e-6` diagonal shift and refuses a non-finite tangent. Adding the `mu dp` coupling to the symmetrised sliding tangent **stalls** Newton, and a load-ratio predictor made friction worse (both tried, reverted).
- **Speed facts** (10k-dof 2D lug): faer is sequential below 40k free dofs (`parallelism_for`; its threads cost more than they save), the assembly keeps rayon; the 2D plastic tangent uses `accumulate_tangent_2d` (5x fewer operations); AMD beats geometric nested dissection ~2x in 2D; chord Newton did not help once evaluation was cheap; `NL_PROFILE=1` prints evaluation / factorisation / solve time.

- **Kernel upgrades of the eccentric-bushing effort** (record: `docs/fea-core.md` Phase 9, evidence in `docs/eccentric-bushing.md` section 9):
  - **Arc-length works with deformable contact** (not with free rigid masters): after every converged arc step the AL passes run at the fixed load, every step starts from a load-controlled Newton polish, and a step that overshoots `lambda_max` is replaced by a load-controlled step that lands on it exactly. `Start::lambda` continues a run that already applied part of its load; `NlOptions::stop_at_force` presses a body in by prescribed displacement until the first master carries a target force (an overshooting step is halved).
  - **The frictional tangent is made consistent by defect correction**: the factorised matrix stays symmetric with sliding friction frozen; the exact tangent minus it (the `mu dp` coupling and the unsymmetric part, per frictional point, `ContactStats::defect`; likewise the follower-load stiffness, `Eval::load_defect`) is applied by `refine_consistent` (iterative refinement over the existing factorisation, <= 6 back-solves). Restricting the defect to sliding points was tried: Newton stalls again. Not available with free rigid translations / rotation (the Schur complement assumes symmetry: near the friction limit a moment-controlled run slows).
  - **Follower pressure** (`Loads::followers`): the load follows the deformed surface; verified against the exact Hencky compression (to 1e-9, 2D and 3D) and finite differences of the load stiffness. LDL^T is used for any run with followers (the symmetric part of the load stiffness is indefinite well before the body is).
  - **Anisotropic elasticity with J2 plasticity** (small strain; finite strain stays isotropic) solves the return mapping for stress and multiplier together (the flow direction changes during the return because `D n` is not parallel to `n`); with an isotropic stiffness it reproduces the isotropic law to 1e-9. Thermal expansion tensor for anisotropic materials; plasticity with a temperature change is still refused.
  - Convergence is measured against the elastic internal force (`Eval::f_elem`) and an absolute floor `1e-10 |f_ext|`; a run whose bodies are held by nothing is refused (`check_not_floating`); `NlSolution::ground_leak` is the net force the weak grounding springs carry; a node pair outside the contact matrix pattern retries with doubled margins (4 times).
  - **The closest-point projection's convergence tolerance is round-off aware** (`eps |x| / |tau|`); a fixed `1e-14` made contact points silently leave contact once coordinates were of order one (a hole at (1, 1) failed, at the origin it solved): `a_contact_solution_does_not_depend_on_where_the_model_sits`.
  - The tangent predictor of prescribed displacements is evaluated without the near-contact stabiliser (`ContactSet::set_stabilise`), which would drag an approaching body along.
  - Debug aids: `NL_TOPRES=1` prints the four largest initial-residual dofs (node, component, position, displacement) of every step; `NL_PROFILE=1` splits evaluation into element assembly and contact.

## Boundaries
### Always
- Add or extend a closed-form / convergence-rate test for every numerical change (`tests/linear_static.rs`); speed claims need a `tests/bench.rs` measurement.
### Never
- Add UI, material tables, or file I/O here (the `.vtu` writer must return a `String`).
- Weaken a test tolerance to make a failing case pass; find the cause.

## Navigation
Plan and measured numbers: `docs/fea-core.md`. Sibling solvers: `lug-solver/AGENTS.md`, `edge-check/AGENTS.md`.
