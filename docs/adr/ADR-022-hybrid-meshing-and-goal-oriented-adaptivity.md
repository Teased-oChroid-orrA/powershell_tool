# ADR-022: Hybrid structured/unstructured meshing and goal-oriented adaptivity in `fea-core`

Status: Accepted (2026-10-13), direct user direction (epic #25). Audit: `docs/hybrid-meshing-amr-audit.md`; measurements:
`docs/hybrid-meshing-amr-evidence.md`. Builds on ADR-012 and ADR-013.

## Context

`fea-core` adapts by remeshing from a ZZ energy-norm estimate, in 2D and 3D, with no goal-oriented indicator, no coarsening
policy beyond the size field, no mixed-topology mesher and no solution transfer. Epic #25 asks for two separable systems: a
hybrid structured/unstructured mesher (System B) and a goal-oriented, multicriteria adaptive-refinement framework (System A).

## Decisions

1. **Two new modules, no dependency between them.** `fea_core::meshing` (adjacency and conformity, quality, welding,
   partition, hybrid mesher, smoothing, cost model) and `fea_core::adaptivity` (snapshot, transfer, rollback, then estimators,
   fusion, marking, controller). Estimators never call a mesher; meshers never see an estimator. They meet in a caller-supplied
   size field, as `adapt.rs` already does.
2. **Adaptation backend is size-field remeshing** (the repository's existing architecture), behind the controller's mesher
   callback. Local topology-preserving refinement of structured blocks is not a prerequisite.
3. **2D hybrid mesh = tensor-grid cell classification + welded blocks.** A rectilinear outer boundary is cut into coarse cells
   by its vertices and by the padded boxes of features (holes, reflex corners, caller boxes). The padding is the fixed point of
   `pad = margin * size(padded boundary)`, so a block starts only where the size function has relaxed. Regular cells become
   Quad blocks; the other cells are one or more Delaunay regions whose structured-side edges are pre-split into exactly the
   structured divisions; a weld makes both sides' interface nodes the same nodes. A quad edge meets a triangle edge of the same
   order (Quad4/Tri3, Quad8/Tri6), so the interface is conforming C0 with no constraint equations. If the Delaunay mesher
   splits an interface edge further the structured run's divisions are doubled and the build repeated (bounded). Every result
   is checked for conformity, quality and area; any failure falls back, with the reason in the report, to the unstructured
   mesh. A curved outer boundary is `Unsupported` (cells cannot tile it) and falls back the same way.
4. **Cost, not preference, selects hybrid.** `meshing::cost` predicts the work of a candidate (exact symbolic factor size,
   assembly units) in seconds from rates measured on the machine, for a stated workload; candidates are compared at nominal
   equal accuracy. Anisotropic structured blocks are cheaper but their accuracy is a modelling assumption, so they are
   eligible only on request. Measured result (evidence doc): an isotropic structured block is **not** cheaper than a graded
   Delaunay mesh; an anisotropic one on a slender member with a local feature is (38 % fewer unknowns at equal energy error).
5. **Goal-oriented estimate** (System A, Phases 5 and 8). Two estimators, chosen by measurement (evidence doc, Phase 8):
   the **enriched-dual** estimate (the mesh is refined uniformly once, the dual is solved there, `J(u) - J(u_h) ~ (z_f - P z_h) .
   (f - K u_h)` with nodal sharing among the coarse elements) is the controller's choice for 2D triangle and quadrilateral
   meshes; the **recovered-stress** form below is the fallback for other meshes and the cheap option. A residual-based form with
   a polynomial-fit dual was built, failed (wrong sign), and removed.

   Recovered-stress form (Phase 5): `eta_e = int_e (sigma*(u) - sigma_h(u)) :
   C^-1 : (sigma*(z) - sigma_h(z))` from `a(e_u, e_z) = J(u) - J(u_h)` and Galerkin orthogonality, with the dual `z` from the
   primal factorisation (`K` symmetric). It needs no enriched space, works for every element family and inherits SPR
   improvements; effectivity is measured against closed-form quantities of interest.
6. **Equilibrated stress by vertex patches, row by row** (Phase 6): Raviart-Thomas of degree `p` on the triangles around a
   vertex, one local KKT problem per stress row (rows are independent `H(div)` fields), a constraint row removed only when
   linearly dependent (its leftover right-hand side is the reported compatibility defect), Neumann data from the applied
   tractions, rows free where the displacement component of the vertex is constrained. Symmetry is not enforced: it is an
   estimator, not a certified bound. Stress recovery never smooths across a material interface (SPR+ per material).
7. **Unsupported means explicit.** New code returns `MeshError` / reliability statuses, never a silent substitute. History
   variables are not transferred between meshes (`Unsupported`) until an admissible transfer exists.

## Consequences

- A hybrid mesh is a normal `Mesh` (blocks of Quad and Tri kinds); every solver, estimator and exporter keeps working.
- Interface faces are interior faces, so boundary-based tools (surfaces, SPR boundary handling) see only the true boundary.
- 3D hybrid (hexahedra against tetrahedra) needs a decision between pyramid elements, tie constraints and a vertex-matched
  diagonal-split interface; it is tracked in issue #16 and not decided here.
