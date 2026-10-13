# Hybrid meshing + goal-oriented AMR: measured evidence (epic #25)

Every number below was produced by a test or benchmark in this repository on the date given; the command is next to it.
Machine: Apple silicon macOS (release builds for timings). Hosted Windows CI is the only Windows evidence and is tracked in
the phase issues. Appended per phase; nothing here is a projection.

## Phase 2: 2D hybrid mesh (2026-10-13)

`cargo test -p fea-core --test hybrid`

- **Patch test**, Quad8+Tri6 and Quad4+Tri3 meshes with a forced unstructured block, affine displacement on every boundary node:
  interior displacement error < 1e-12, stress error < 1e-9 (relative), both families.
- **Plate with hole** (12 x 4, hole radius 0.6, uniaxial 1000 psi), hole-top stress: fine unstructured reference 3352.1,
  Peterson's finite-width factor gives 3352.6, hybrid (1958 nodes) 3308.0 (1.3 %), unstructured (1962 nodes) 3443.6 (2.7 %).
  Strain energy: reference 2.589051, hybrid 2.587118, unstructured 2.587250.
- **Where hybrid pays** (`hybrid_benefit_on_a_long_bar_with_a_hole`, `--release --ignored --nocapture`; 40 x 4 bar, hole
  radius 0.6, size 0.1 at the hole growing 0.2 per unit to 0.6; reference = unstructured at half the size, 34960 dofs):

  | candidate | dofs | energy error | hole-top stress error |
  |---|---|---|---|
  | unstructured Tri6 | 9076 | 6.5e-6 | 0.7 % |
  | hybrid, isotropic Quad8 blocks (scale 1.2) | 8636 | 4.9e-6 | 1.1 % |
  | hybrid, structured aspect 4 along the bar | 5636 | 5.4e-6 | 1.1 % |

  At sizes 2x and 4x coarser the isotropic hybrid needs *more* unknowns than the graded Delaunay mesh (3272 vs 2538, 2052
  vs 930) and the aspect-4 hybrid fewer only at 2x (2056 vs 2538). Mesh generation is 2-3x slower than plain Delaunay.
  Conclusion recorded in ADR-022: hybrid is chosen by predicted cost, never by default.
- Cell-grid divisions go through size-aware bisection (size within a factor 1.4 across a cell), a padded-feature fixed point and
  per-run division classes; before those the isotropic structured elements were 0.5 of the target size (3x the unknowns).

## Phase 3: quality and cost (2026-10-13)

- Quality-guarded Laplacian smoothing (`smoothing_improves_the_unstructured_part...`): 63 movable nodes of the hybrid plate,
  349 accepted moves, minimum scaled Jacobian 0.482 -> 0.567, mean 0.9276 -> 0.9448; structured elements and boundary
  nodes bit-identical; strain energy changes < 0.1 %.
- Cost model (`predictions_track_measured_solve_times`, `--release --ignored`): calibrated on a 100x100 Quad8 mesh, predicted /
  measured time of assemble+factor+solve is 0.57 (50x50), 0.84 (70x70), 1.27 (140x140): good to a factor of 2 over a 4x range
  and poor on very small meshes (fixed overheads, ratio 0.21 at 20x20), so it ranks candidates that differ by more than ~50 %.

## Phase 5: dual-weighted residual estimate (2026-10-13)

`cargo test -p fea-core --test dwr -- --nocapture`. Plane-strain Lame cylinder (a = 1, b = 2, p = 1000), quarter model; exact
quantities from the closed form. Effectivity = estimated error / true error (exact minus discrete). Below ~1e-5 relative
the closed form is limited by the circle being approximated by quadratic edges, so those rows are informational.

| goal | mesh | h = 0.5 | h = 0.25 | h = 0.125 |
|---|---|---|---|---|
| compliance `f.u` | Tri6 | 1.83 | 1.04 | 0.55 (err 2.7e-6, floor) |
| compliance `f.u` | Quad8 | 2.95 | within [0.5, 2] | within [0.5, 2] |
| mean `sigma_xx`, band `r < 1.6` | Tri6 | 1.26 | 1.10 | 0.64 (floor) |
| mean `sigma_xx`, band `r < 1.6` | Quad8 | 1.17 | 0.57 | 1.60 (floor) |

- The dual solve reuses one factorisation for any number of goals; reciprocity `f.z = J(u_h)` holds to 1e-9, the compliance dual
  equals the primal solution, the dual residual is < 1e-10, the estimate is linear in the goal (2 g1 - 3 g2 to 1e-7), and the
  field is bit-identical on one and many threads (60 x 60 Quad8).
- A reaction sum fixed by equilibrium (true error 0) is estimated at 2e-4 .. 6e-5 of its value: small, not zero.
- Point goals are reported `Degraded` (singular dual); plastic blocks and nodal temperature fields `Unsupported`.

## Phase 6: SPR+ and the equilibrated stress reconstruction (2026-10-13)

- **Defect fixed (Fix-Everything Policy)**: `recover_spr` built every patch from all elements around a node whatever their
  material, so stress was blended across a material interface (`tests/spr_interface.rs`: the plain recovery misses the jump by
  more than 20 % at interface nodes; kept as a test of the premise). `Model::recover_spr_by_material` recovers each material as
  a model of its own (one-sided patches at the interface); `zz_error_field` and the dual-weighted estimator use it. A layered bar
  whose finite-element solution is exact now has a ZZ estimate of 0 to 1e-6 (it was a visible nonzero), and a single-material
  mesh gives the old result bit for bit. `SprDiagnostics` counts own fits / borrowed / lower-degree / plain-average nodes.
- **Equilibrated reconstruction** (`adaptivity::equilibrated`, Tri3 / Tri6, plane stress and strain; `cargo test -p fea-core
  --test equilibrated -- --nocapture`): vertex patches, Raviart-Thomas degree `p` per stress row, KKT local problems.
  Verified on six cases (Tri3, Tri6 plane stress, Tri6 plane strain; clamped or symmetric supports; body force, traction,
  pressure): `div sigma* + f` < 4e-15, normal-traction jump < 3e-14, boundary traction error < 5e-14, compatibility defect
  < 4e-14 (all relative to the stress scale). Row-wise reconstruction is not symmetric: skew fraction 0.2 % (Tri6) to 4.5 %
  (Tri3 clamped), so the result is an estimator, not a certified bound.
- **Estimator quality**, L-shaped plate with a re-entrant corner against a fine Tri6 reference (h = 0.025):

  | h | elements | true energy error | equilibrated (effectivity) | ZZ (effectivity) |
  |---|---|---|---|---|
  | 0.4 | 56 | 1.58e-2 | 2.17e-2 (1.37) | 2.91e-2 (1.84) |
  | 0.2 | 220 | 1.27e-2 | 1.71e-2 (1.35) | 1.86e-2 (1.47) |

  Both rank the worst element first or second. An exact multi-material solution gives an equilibrated estimate of 0 (< 1e-8 of
  the stress scale) without any smoothing across the interface. Curved Tri6 edges are reconstructed on the straight triangle and
  reported `Degraded`; quadrilaterals, nodal point loads, boundary force fields and temperature fields are `Unsupported`.
  The reconstruction is bit-identical on one and many threads.
- **Zones** (`adaptivity::zones`): re-entrant corners are `Singular`, sharp convex corners and hole boundaries
  `GeometricDiscontinuity`, caller lists give contact / perimeter / bearing / buckling zones; the physics estimators a zone calls
  for that do not exist (contact residual, eigenvalue, localised bearing) are listed by `ZoneMap::unmet`.
- **Contact-specific reconstruction prerequisites** (not implemented): the contact traction must enter the patch problem as
  Neumann data on each body, which needs (1) a converged active set and the tangent-transpose adjoint of the contact problem
  (unsymmetric with friction), (2) a smoothed contact-pressure field, because pointwise Gauss-point pressures on non-matching
  deformable meshes scatter (`fea-core/AGENTS.md`, contact pitfalls), (3) master/slave pair consistency in the patch topology.
  They are Phase 9 work.

## Phase 8: the adaptive loop (2026-10-13)

`cargo test -p fea-core --test amr -- --nocapture`. L-shaped plate (re-entrant corner, clamped left edge, 100 per unit
length downward traction on the lower arm's free end, Tri6, plane stress), quantity of interest = smooth-weighted mean of
`sigma_yy` around (1.5, 0.5) (a bump of radius 0.45); reference value from a uniform h = 0.02 mesh (18.421).

- **Which dual estimate.** Three were measured against the true error on uniform meshes and on an adapted one
  (`J_ref - J_h`; estimate signed the same way):

  | mesh | true | recovered-stress DWR | residual DWR, cubic-fit dual | enriched-dual DWR |
  |---|---|---|---|---|
  | h = 0.4 | -3.134 | -5.232 | +0.085 | -2.255 |
  | h = 0.2 | -1.822 | -1.701 | +0.063 | -1.098 |
  | h = 0.1 | -0.531 | -1.036 | +0.035 | -0.344 |
  | h = 0.05 | -0.181 | -0.418 | +0.012 | -0.157 |
  | adapted, 1086 dofs | -0.164 | -2.054 | +0.013 | -0.245 |

  The recovered-stress estimate is dominated by the re-entrant corner (1.26 of the -2.05 sits in one element) where SPR is not
  superconvergent, so on adapted meshes it does not fall with the error. The residual estimate with a polynomial-fit dual has
  the wrong sign: the Galerkin-orthogonality test of its residual assembly passes (it is exact), so the fault is the dual
  reconstruction; it was removed. The enriched dual (one uniform h-refinement, the goal rebuilt there) keeps the sign and
  tracks the error within a factor 0.6-1.5 (0.68-0.73 along the adaptive run). It is the controller's estimator wherever the
  mesh can be refined (2D triangles and quadrilaterals); other meshes fall back to the recovered-stress form.
- **Goal-oriented against uniform refinement** (tolerance 0.5 absolute): the loop stops after two adaptation steps at 892
  unknowns with a true error of 0.098; uniform refinement has an error of 0.375 at 8046 unknowns.
- **Strategies** (same problem, goal tolerance 0.2): adjoint only 892 dofs, goal error 0.098, physics error 7.1 %; physics only
  2744 dofs, goal error 0.056, physics error 4.1 %; constraint-preserving fusion with a 2 % physics requirement 10918 dofs, goal
  error 0.067, physics error 1.9 %. The physics requirement, not the goal, sets the cost near a re-entrant corner.
- **Stops are explicit and tested**: budget (`max_dofs`), cycle limit, mesher failure, a mesher that silently drops the
  load (caught by the load-resultant check) or the restraints, an unreachable physics tolerance under a singular-zone size cap
  and a growth limit of 3 per cycle (without it every element became mandatory and the mesh grew fourfold per cycle).
- **Hybrid and unstructured meshers** behind the same loop (12 x 4 plate with a hole, `sigma_xx` at the hole top, reference
  J = 2059.16): both stop converged with a physical error under 0.4 %; the estimate sees the discretisation error of the mesh it
  is given, not the approximation of the circle by the boundary elements (true errors 6.8 and 3.6 at the first mesh against
  estimates of -0.8 and +0.2), so that end-to-end check is on the quantity, not on the effectivity.

## Phase 9 (part 1): eigenvalues, and two kernel defects the eigenvalue check exposed (2026-10-13)

`cargo test -p fea-core --test eigen --test dynamics --test recovery -- --nocapture`.

- **Defect 1: simplex consistent mass.** `kernel::mass` integrated `rho N_a N_b` with the stiffness rule: one point for Tri3 /
  Tet4 (a rank-one element mass) and a degree-2 rule for Tri6 / Tet10 (the products are degree 4). Frequencies of triangle and
  tetrahedron models carried a quadrature error larger than the discretisation error (axial bar, Tri6, h = 0.2: the eigenvalue
  error was 2.49e-5, of which 1.1e-5 was quadrature; after the fix 1.36e-5). `ElementKind::mass_table` now gives collapsed Gauss
  rules exact for the mass products (4 / 9 points on triangles, 27 / 64 on tetrahedra); tensor-product elements were already
  exact. The same kernel serves the heat-capacity matrix and the mass objective of `optimize.rs`.
- **Defect 2: error integrals sampled at superconvergent points.** `zz_error_field` and `stress_error_energy` integrated the
  recovered-minus-finite-element stress with the stiffness Gauss points, where the finite-element stress is (nearly)
  superconvergent, so they saw a fraction of the error between those points (for the axial-bar mode: 0.06-0.2 of the energy
  error). They use `recover::error_rule` now (4-5 Gauss points per direction, collapsed on simplices); with it the ZZ estimate
  equals the true stress-energy error (ratio 0.8-1.25 on five element families, `tests/recovery.rs`).
- **Eigenvalue estimate** (`adaptivity::eigen`): `w_h - w ~ sum eta_e^2` with `eta_e` the recovered-stress energy error of the mass-normalised
  mode, no dual solve. Axial bar (Tri6, E = rho = 1, nu = 0, mode 0 / mode 1), estimate / true error: 1.35 / 1.37 (h = 0.4),
  1.03 / 1.48 (0.2), 1.05 / 1.13 (0.1), 1.01 / 1.01 (0.05), 1.00 / 1.00 (0.025). The same identity on Quad9 grids holds to four
  digits (`delta lambda` 8.085e-5 against `a(e, e)` 8.114e-5).
- **Loop**: following the second axial mode to a relative tolerance of 2e-4 takes three adaptations (376 dofs); every cycle's
  mode is recognised by the modal assurance criterion (MAC >= 0.99995). A square with rollers on all sides and nu = 0 has the
  triple eigenvalue pi^2: the three modes form a cluster, the estimate is the cluster's and `Degraded`, and the cluster is followed as a
  unit. A mesher that changes the physics (both ends clamped) makes the followed mode unrecognisable and the run stops with
  `ModeLost`.
- Buckling: the same indicator marks, but the geometric-stiffness term of the same order is not estimated, so a buckling value is
  never presented as an error bound (documented in the module; no buckling loop is built).

## Phase 9 (part 2): plasticity and contact (2026-10-13)

`cargo test -p fea-core --test nonlinear_adapt -- --nocapture`. Supported formulation, stated: small-strain J2 plasticity (the
tangent adjoint also covers finite strain through the same assembly, untested), monotone proportional loading for the warm
start, frictionless or frictional contact for the loop. Goal-oriented *error estimates* of nonlinear and contact quantities
are not provided; their evidence is the change from mesh to mesh, reported as such.

- **Tangent-transpose adjoint** (`adaptivity::nonlinear::tangent_adjoint`; the driver now keeps `NlSolution::prev_state`, the
  state at the start of the last accepted step, because the consistent tangent of a return map needs it). Plate with a hole,
  4 % of the points yielded, goal = mean end displacement: `dJ/d(load scale) = z . f_ext` is 1.722297e-2 against a central
  finite difference of the nonlinear solve of 1.722210e-2 (relative difference 5e-5).
- **Stress read-out** (`Model::nl_gauss_stresses`, from the one constitutive path `plastic_update_at` that the element kernel
  now also uses): the section force of the converged plastic state is within 1.7 % of the applied load.
- **History transfer is approximate.** Interpolating plastic strain (inverse distance over the 4 nearest old Gauss points of
  the block) keeps the state admissible (non-negative equivalent plastic strain, never above the old maximum) and the driver
  re-equilibrates it in 6 iterations (19 from scratch), but the result differs from the from-scratch solution by 0.34 % and,
  evaluated with the exact fine displacements, the transferred history leaves a residual of 3.1e3 where *ignoring* the history
  (a virgin state, one-step return map) leaves 15: for monotone proportional loading the plastic state is a function of the
  final strain and history interpolation is worse than no history. `warm_start_displacement` carries the displacements only
  and takes the load in one step: 6 iterations against 19, and the same answer to all printed digits (1.499601e-2).
  Non-monotone histories have no better transfer here; contact states are refused (`Unsupported`).
- **Plastic loop** (`run_amr_nonlinear`, recovered-stress indicator of the nonlinear state): 528 -> 1988 -> 5976 -> 17932 dofs,
  end displacement 1.4920e-2 -> 1.4996e-2 -> 1.5005e-2 -> 1.5006e-2 (changes 5e-3, 6e-4, 1e-4), converged on the stated
  changes. With the displacement warm start the adapted meshes cost 19 Newton iterations in total instead of 68, same physics
  (end displacement agrees to 5e-3 between the two runs).
- **Contact: Hertz line contact** from a mesh that cannot see the patch (610 dofs, one active point). Contact protection (elements
  near the active points must be at most half-width / 5, mandatory otherwise) refines it: 610, 1720, 2524, 3536, 5702, 11352, 17008
  dofs; contact force 3753 -> 3501 -> 3486 -> 3485.8 (changes 6.7 %, 0.1 %, 0.6 %, 0.1 %, 0.0 %), converged. Against the Hertz
  formulas evaluated with the FE force: contact half-width 0.02855 against 0.02842 (+0.4 %), peak pressure 155615 against 156161
  (-0.3 %). Every cycle verifies that the contact tractions balance the master force (`sum (p n + t) w + F_master`, below
  1e-9).
- **Limits found**: the half-width of a symmetric half model is the full extent of its active points (the loop uses extent / 2
  as the half-width of the *active region*, which makes the resolution rule twice as strict there); the recovered-stress
  indicator at off-Gauss points of a plastic element uses the history of the nearest Gauss point; no adjoint estimate for
  contact (unsymmetric with friction; the plastic adjoint above excludes contact states explicitly).

## Phase 10: profile, scaling, diagnostics

Measured with `cargo test -p fea-core --release --test amr_bench -- --ignored --nocapture` (macOS arm64, release). Time per call
at about 13k dofs on the L-shape:

| Stage | Time |
|---|---|
| sparse solve (faer Cholesky) | 32 ms |
| ZZ / SPR recovery indicator | 9-13 ms |
| recovered-stress DWR (dual from the primal factor) | 39-43 ms |
| enriched-dual DWR (uniform refine + dual solve) | 320-346 ms |
| equilibrated H(div) reconstruction | 350-600 ms (1150 ms before the Schur solve and the edge-flux precomputation) |
| hybrid mesh generation | 158 ms (unstructured 129 ms) |
| state transfer (nodal interpolation) | 19.5 ms |

Findings: the equilibrated estimator was the slowest stage; the patch KKT systems are now solved by a Schur complement with
iterative refinement and the edge fluxes are precomputed per edge instead of per patch (about 2-3x). The enriched-dual estimate
costs about ten primal solves, which is why the controller fuses it with cheap indicators and runs it only on the cycles that
need a goal estimate. Hybrid generation is slightly dearer than unstructured meshing and pays only when the structured blocks
are anisotropic (see Phase 3).

Determinism: `tests/amr_bench.rs::deterministic_counts` fixes the L-shape cycle sizes (270, 472, 892 dofs); parallel code never
reduces floats across threads.

Diagnostics: `AmrResult::report_json` / `NlResult::report_json` (`adaptivity/report.rs`) serialise the per-cycle record (dofs,
elements, goal value and estimate, effectivity, marked count, per-estimator reliability, stop reason) and, with
`AmrConfig::keep_fields`, the per-element fields of each cycle (`CycleFields`) for post-mortem plotting.

Phase 11 follow-up (prismatic 3D hybrid): Wedge6 / Wedge15 elements and `extrude` of Tri elements. `tests/wedge.rs`: a
Hex+Wedge extrusion of the 2D hybrid mesh is conforming, passes the 3D affine patch test (displacement 1e-12, stress 1e-9) and
reproduces the 2D plane-strain solution to 1e-9; exact strain energy on a wedge grid for both orders.

Size predictor (`meshing/predictor.rs`): sources (arcs by `r * arc_angle`, re-entrant corners found from the interior angle, points,
lines) graded at rate `g`; the field is `g`-Lipschitz by construction (test over a point grid). Plate with a hole, Tri6, same base
size 1.2: strain energy error against a fine reference (h = 0.04) 2.1e-4 with the predictor (698 nodes) against 9.2e-4 uniform (391
nodes). The two meshes do not have equal unknowns, so this shows the predictor puts elements where the error is, not an equal-cost
gain. The error of one nodal stress is not a usable judge at this size (+1.4 % with the predictor, +0.2 % uniform: nodal noise).

Multi-material (`meshing/multimaterial.rs`): two-material bar (E and 4E, nu = 0), Linear and Quadratic: conforming union, elongation
equal to the series-spring value to 1e-9. Cells touching a shared boundary are forced unstructured so both sides place the same
interface nodes; without that, the hybrid parts did not weld (7 and 15 hanging nodes) and the function fell back to unstructured
for every part, with the reason reported.

Phase 10 addenda (same run, 13354 dofs): the solve splits into assemble 3.5 ms + symbolic 11.7 + factor 14.5 + back-solve 1.0
(32 ms). A controller cycle spends solve 30 ms, goals (enriched DWR) 318, physics (equilibrated) 387, fusion 0.5, marking+sizing
1.0, meshing 203 (a complete remesh: adaptivity is remeshing-based, there is no local remeshing to profile), checks 7. The
element-data loop of the equilibrated reconstruction now runs in parallel with an ordered collect and a max reduction (order
independent, so results are identical): 299 -> 261 ms.

Factorization reuse, evaluated and not done: the coarse dual solve (31 ms) refactors the stiffness the primal solve factored
(32 ms in total). Reusing the factor would save about 30 ms of a cycle that costs about 1 s, and `Solution` does not keep the
factor; the cost does not justify widening that type. The enriched dual solves on a different (refined) mesh and cannot reuse it.
Allocation and cache behaviour were not studied beyond the equilibrated precomputation above.
