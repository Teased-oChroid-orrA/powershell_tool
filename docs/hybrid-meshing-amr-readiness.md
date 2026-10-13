# Hybrid meshing and goal-oriented AMR: readiness, support matrix, limits

Epic #25, decision record `docs/adr/ADR-022-hybrid-meshing-and-goal-oriented-adaptivity.md`, measurements
`docs/hybrid-meshing-amr-evidence.md`, audit `docs/hybrid-meshing-amr-audit.md`. Code: `fea-core/src/meshing/`,
`fea-core/src/adaptivity/`.

## API summary

| Need | Entry point |
|---|---|
| Hybrid 2D mesh with automatic fallback | `meshing::hybrid::mesh_hybrid_2d`, `mesh_unstructured_2d`, `mesh_best_2d` (cost-selected) |
| Conformity / quality gates | `Mesh::conformity()`, `Mesh::quality()`, `QualityThresholds` |
| Cost prediction | `meshing::cost::{CostModel, Workload, choose}` |
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
- Initial size predictor from curvature, loads and contact; multi-material tags through the hybrid mesher (issue #16 open items).
- Windows evidence only from hosted CI (PR #26).

## Benchmark history

See the evidence doc: L-shape goal AMR (892 dofs, error 0.098 against uniform > 8046 dofs for 0.375), eigenvalue effectivity 1.00,
Hertz line contact (+0.4 % half-width, -0.3 % peak pressure), plastic tangent adjoint against finite difference 5e-5, Phase 10
timing table.
