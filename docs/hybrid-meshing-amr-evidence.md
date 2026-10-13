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
