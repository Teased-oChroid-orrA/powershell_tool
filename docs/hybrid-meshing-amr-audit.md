# Hybrid meshing + goal-oriented AMR: Phase 0 audit (epic #25, issue #14)

Audited on `feature/hybrid-meshing-goal-oriented-amr` at `43403a9` (the epic says "latest main"; `main` is 38 commits behind
the kernel work in PR #13, so the audit and the work start from this head). Every statement below was read in the source
or run; "unverified" marks the rest.

## What the kernel is today

- **Mesh**: `fea_core::Mesh` = nodes + `Block`s (one `ElementKind`, one material each) + named node sets / surfaces. Mixed
  element kinds in one mesh are already legal (one block per kind); assembly colours elements across blocks. Kinds: Tri3/6,
  Quad4/8/9, Tet4/10, Hex8/20/27. No wedge, no pyramid.
- **Meshers**: 2D constrained Delaunay + Ruppert (`delaunay.rs`, size function, curve-aware) converted to Tri3/6 or
  three-quads-per-triangle Quad4/8/9 (`mesh2d.rs`); structured mapped grids of every kind (`generate.rs`); extrude / revolve
  / sweep of quad meshes into hexes (`sweep.rs`); 3D tetrahedra from a signed-distance solid (`tetmesh.rs`: Kuhn lattice +
  Rivara bisection); Gmsh / Abaqus import; B-rep solids in `solid-kernel` (OCCT off by default).
- **Adaptivity**: *remeshing*, not local modification. `recover_spr` + `zz_error_field` (energy norm) -> `adapted_size_field`
  (2D) / `adapted_size_field_3d` -> the caller's mesher on that size field -> re-solve (`adapt::refine`,
  `strategy::solve_refined`). Equidistribution of the ZZ estimate, grading limit, accept/reject on estimated error and DOFs.
  No Dörfler marking, no goal-oriented indicator, no state transfer (every pass re-solves from zero; nonlinear history is not
  carried), no coarsening other than the size field growing.
- **Solvers**: symmetric block-CSR assembly, faer Cholesky / LDL^T, PCG + AMG; `Reduced::structure_only` gives the factor
  size without a numeric factorisation (a cheap cost predictor). Nonlinear Newton / arc-length, contact (AL + Coulomb),
  modal / buckling / transient, thermal. Linear stiffness is symmetric, so the adjoint of a linear problem is the primal
  factorisation re-used; the nonlinear / frictional tangent is not symmetric (adjoint needs `K_T^T`, not implemented).
- **Adjoint machinery**: `sensitivity.rs` has the discrete adjoint `K lambda = dJ/du` with the factorisation shared, for
  design sensitivities. It does not localise anything per element and has no QoI abstraction.
- **Quality**: `validate.rs` flags inverted (non-positive Jacobian) and slender elements; `TriMesh::quality` for 2D triangles.
  No general metric set (scaled Jacobian, aspect, skew, size progression), no smoothing / flipping.

## Capability matrix (epic requirement -> status)

| Epic requirement | Status | Where / gap |
|---|---|---|
| Common mesh interface, element metadata | **Exists** | `Mesh`, `Block`, `ElementKind` tables. Gap: no region / provenance metadata, no adjacency object |
| Adjacency | Partial | `Mesh::boundary_faces` (sorted face keys), node-to-element maps rebuilt locally in `recover.rs`. Gap: shared element/edge adjacency + conformity checker |
| Quality metrics | Partial | Jacobian sign, slenderness. Gap: full metric set + distribution report |
| Structured meshing 2D/3D | **Exists** | `generate::grid`, `sweep`. Gap: automatic detection of mappable regions |
| Unstructured 2D / 3D | **Exists** | `delaunay`, `tetmesh` |
| Region classification, partitioner | Absent | new |
| Structured-unstructured transition | Absent | 2D tri+quad with a shared edge-node set is conforming for Tri3/Quad4 and Tri6/Quad8/9 (same edge order); 3D hex-tet has no wedge/pyramid, needs a decision (below) |
| Mesh smoothing / topology optimisation | Absent | new |
| Cost model | Partial | `Reduced::structure_only` (factor nnz), `report.rs` timings. Gap: model + candidate comparison |
| Size-field initial prediction | Partial | geometry-driven sizes exist only inside the callers' closures; ZZ-driven field exists. Gap: curvature / feature / load proximity predictor |
| ZZ / SPR recovery | **Exists** | `recover.rs`. **Defect**: patches span blocks, so stress is smoothed across material interfaces (epic 5.1 forbids it) |
| SPR+ (interfaces, ill-conditioned patch detection, fallbacks) | Partial | borrow-from-neighbour fallback exists; interface handling and conditioning report missing |
| Equilibrated H(div) stress reconstruction | Absent | new, linear elastic triangles first |
| Goal functionals (QoI) + adjoint | Partial | adjoint solve exists in `sensitivity.rs`; no QoI trait, no localisation |
| DWR localisation, signed + marking indicators | Absent | new |
| Estimator trait, error field, reliability status | Absent | new |
| Normalisation + fusion (sum / max / constraint-preserving) | Absent | new |
| Dörfler marking, budgets, stagnation | Partial | accept/reject by ZZ in `adapt::refine`; no marking |
| Hybrid-aware refinement | Absent | remesh path is the architecture (size field); local topology-preserving refinement is not |
| State transfer | Absent | linear problems do not need it; nonlinear / contact history needs a design |
| Nonlinear / contact / eigenvalue adaptive integration | Absent | kernels exist (`nonlinear`, `contact`, `dynamics`) |
| Deterministic parallel reductions | **Exists as a rule** | `tests/recovery.rs` bit-identity test; new code follows it |
| Benchmarks | Partial | `tests/bench.rs`; analytic cases Kirsch, Lamé, Hertz, Timoshenko exist in `tests/` |

## Verified defects found (Fix-Everything Policy: fixed in this effort)

1. **SPR smooths across material interfaces** (`recover.rs::recover_spr`, `samples_of`): all elements around a node
   contribute regardless of block. Regression + fix: Phase 6 (#20).

## Decisions taken from the audit

- **Adaptation backend = size-field remeshing**, as the repository already does, behind a single `Adaptation` boundary so
  estimator code never touches a mesher and a mesher never sees an estimator (epic section 3). Topology-preserving local
  refinement of structured blocks is kept as a later option, not a prerequisite.
- **DWR without an enriched space**: the dual solve uses the primal factorisation (symmetric linear case). The localised
  estimate is the recovered-stress form `eta_e = int_e (sigma*(u) - sigma_h(u)) : C^-1 : (sigma*(z) - sigma_h(z))`, from
  `a(e_u, e_z) = J(u) - J(u_h)` and Galerkin orthogonality. It works for every element family the kernel has, reuses SPR
  (so SPR+ improvements feed it), and its effectivity is measured against closed-form QoI values. A residual-based DWR on an
  enriched space remains unattempted.
- **2D hybrid first**; the 3D hex-tet question gets its own ADR after an experiment (vertex-matched diagonal-split interface
  vs. tie constraints vs. adding pyramid elements).
- **Typed errors**: new modules return typed errors; the existing `Result<_, String>` surface is not rewritten here.
- **Unsupported configurations fail explicitly** (`Unsupported` reliability status), never silently fall back.

## Baselines

Recorded in the Phase 1 / Phase 10 issues as they are measured (`fea-core/tests/bench.rs`; new epic benchmarks live in
`fea-core/tests/amr_bench.rs`).
