# Hybrid meshing and goal-oriented AMR: readiness, support matrix, limits

Epic #25, decision record `docs/adr/ADR-022-hybrid-meshing-and-goal-oriented-adaptivity.md`, measurements
`docs/hybrid-meshing-amr-evidence.md`, audit `docs/hybrid-meshing-amr-audit.md`. Code: `fea-core/src/meshing/`,
`fea-core/src/adaptivity/`.

## API summary

| Need | Entry point |
|---|---|
| Hybrid 2D mesh with automatic fallback (rectilinear, angled and curved outlines) | `meshing::hybrid::mesh_hybrid_2d`, `mesh_unstructured_2d`, `mesh_best_2d` (cost-selected) |
| Conformity / quality gates | `Mesh::conformity()`, `Mesh::quality()`, `QualityThresholds` |
| Cost prediction | `meshing::cost::{CostModel, Workload, choose}` |
| Initial size prediction | `meshing::predictor::SizePredictor2d` (arcs, re-entrant corners, loads / restraints / contact lines) |
| Multi-material meshes | `meshing::multimaterial::mesh_multi_material_2d`, `mesh_multi_material_unstructured_2d` (conforming weld, shared boundaries cut at both sides' nodes until it matches, or explicit failure) |
| 3D hybrid (prismatic) | `sweep::extrude` of a hybrid 2D mesh: Hex8/20 + Wedge6/15 |
| Linear-static AMR | `adaptivity::run_amr(MeshedProblem, AmrConfig, GoalSpec)` |
| Goal functionals | `adaptivity::goal::LinearFunctional` constructors |
| Estimators | `ZzEstimator`, `EnrichedDwrEstimator`, recovered-stress `dwr`, `EquilibratedEstimator`, `PhysicsEstimator` |
| Fusion / marking | `fusion::fuse` (weighted sum, weighted max, constraint-preserving), `marking::dorfler`, `Budget` |
| Eigenvalue AMR | `eigen::run_eigen_amr` (clusters, MAC tracking) |
| Plasticity / contact AMR | `nonlinear::run_amr_nonlinear` (tangent adjoint, history transfer, warm start, contact protection) |
| Diagnostics | `AmrResult::report_json`, `NlResult::report_json`, `AmrConfig::keep_fields` |

Every failure path returns `MeshError` or a reliability status; there is no silent substitute.

## Support matrix

| Capability | Tri3/Quad4 | Tri6/Quad8/9 | 3D Tet/Hex | Wedge |
|---|---|---|---|---|
| Hybrid mesh | yes | yes | Hex/Wedge via sweep (tested) | yes (tested) |
| ZZ / SPR+ indicator | yes | yes | element rules unit-tested | error rule unit-tested only |
| Recovered-stress DWR | yes | yes | not exercised | not exercised |
| Enriched-dual DWR | yes | yes | no | no |
| Equilibrated H(div) | Tri3 | Tri6 | no | no |
| Eigenvalue AMR | yes | yes | not exercised | no |
| Plastic / contact AMR | 2D | 2D | no | no |

## Known limits (not delivered)

- Goal-oriented error estimates for nonlinear and contact quantities; contact adjoint (unsymmetric with friction).
- History transfer for non-monotone loading; contact states are refused (`Unsupported`).
- Buckling error bound; local topology-preserving refinement and coarsening (adaptivity is remeshing-based).
- 3D enriched dual and 3D equilibrated reconstruction.
- General hex-against-tet transitions (declined, ADR-022 addendum).
- Wedge import from Gmsh / Abaqus (node orderings not verified; none is imported today). Multi-material 3D (sweep keeps block materials but is not tested across an interface).
- The size predictor takes explicit points / lines; the workbench feeds it the sketch geometry and the point loads / restraints (not edge loads, contacts).
- Hybrid mesh: a region too small for the element size to leave regular area falls back (measured 3 of 120 random outlines); multi-material hybrid needs the shared boundaries to coincide exactly.
- Windows evidence only from hosted CI (PR #26): Windows and Linux workspace runs passed on the branch head before the last perf commit.

## Benchmark history

See the evidence doc: L-shape goal AMR (892 dofs, error 0.098 against uniform > 8046 dofs for 0.375), eigenvalue effectivity 1.00,
Hertz line contact (+0.4 % half-width, -0.3 % peak pressure), plastic tangent adjoint against finite difference 5e-5, Phase 10
timing table.

## What the hybrid mesher covers

- **Outlines.** Axis-aligned, angled and curved outer boundaries. The cells the boundary passes through, and those within one local
  size of an angled or curved piece, are unstructured; the loop of the unstructured region follows the true boundary (cut at the
  grid lines), so regular blocks stay a margin inside and a thin layer fills the gap (strips next to such a boundary are
  subdivided to 2.5 sizes). Axis-aligned pieces stay on grid lines and keep their regular cells. Checkerboard corners (two
  unstructured cells and two structured ones around a grid vertex) give up the smaller structured cell.
- **Gradients.** The grid is subdivided by the size field of the structured and boundary cells only (a fine size around a hole used to
  slice the far regular blocks into slivers). A structured cell whose elements would stretch more than 3x beyond the requested
  aspect goes to the unstructured mesher; per-cell graded transition rows were not built, the unstructured cells do the grading.
- **Seeds.** `PartitionOptions2d::extra_boxes` keeps cells unstructured; the problem layer seeds point loads and restraints.
  `SizePredictor2d` is a size function (workbench: Auto Size Features), not a partition input.

## In the application (FEA Workbench)

Controls (2D and, for the strategy and predictor, extruded sketch problems; every one defaults to the previous behaviour):

- **Mesh Strategy** (`mesh.strategy`): Unstructured or Hybrid; a request the region does not suit returns the unstructured mesh and
  adds a result note with the reason. An extruded sketch gives Hex20 + Wedge15. The template plate with a hole at Size At Holes 0.1,
  which used to fall back (size jump 8.1 against 6), now meshes hybrid with jump 1.63.
- **Auto Size Features** (`mesh.predict_sizes`): `SizePredictor2d` over arcs (0.35 rad), re-entrant corners and point loads /
  restraints (a quarter of the element size), grading 0.3; never coarser than without it; the element forecast follows it.
- **Adaptivity Driver** (`mesh.adapt_mode`, with Adaptive Passes > 0): ZZ (default), Equilibrated (the H(div) reconstruction drives the
  loop; triangle elements, Unstructured strategy, plane problems, else refused with the reason) or Goal-oriented (enriched-dual
  adjoint of the compliance fused with the equilibrated / ZZ criterion; the result note names the goal and physics estimators that
  ran). The enriched dual is a goal estimator, so it is reachable through the goal-oriented driver only.
- **Natural frequencies (n)** refine on `eigen::run_eigen_amr` when Adaptive Passes > 0 on a 2D sketch (relative eigenvalue error
  `target_error^2`, the lowest 6 modes followed); the frequency list says how many meshes it took.
- **Bond Regions / Region k Material** (`mesh.bond_regions`, `Problem.region_materials`): the regions of a multi-profile sketch weld
  along shared boundaries (`mesh_multi_material_2d`), each with its own material. Stress at an interface node is the side with the
  larger von Mises; the yield margin is not given with several materials.
- **Material Model / Hardening Modulus** (`material.plastic`, `material.hardening`): J2 small-strain plasticity with linear hardening,
  loads in 8 steps, an equivalent-plastic-strain field and summary; adaptive passes use `adaptivity::run_amr_nonlinear` (recovery
  indicator of the converged plastic state, external work watched from mesh to mesh). Not with bushings, thermal studies, region
  materials or the goal-oriented driver.

Measurements are in `docs/hybrid-meshing-amr-evidence.md` ("Widening the hybrid mesher and the workbench controls").

**Contact AMR is not exposed.** `run_amr_nonlinear` was run on the bushed lug (single frictional-contact solve per mesh): a fit's
contact patch is the whole bore, so the contact-protection rule refines everything (5.4k and 25.8k elements against 1.25k for the ZZ
driver, 110 and 196 s), the controller's force-balance check (rigid master against slave tractions, 1e-3) sees 3-11 % on a
two-pass deformable interface, and the peak stress (0.7 %) and fit force (1e-4) are the same as the ZZ driver's. The ZZ passes
remain the bushed-plate driver. A contact problem with a rigid pin is the case the controller was built for.
