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
