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
- 3D hybrid: see the addendum below.

## Addendum: 3D hybrid is prismatic (decided with the Wedge6 / Wedge15 elements)

A conforming interface between a hexahedron and a tetrahedron needs pyramid elements (a quadrilateral face has four corners,
a tetrahedron face three), or tie constraints (non-conforming, mortar-style), or a diagonal-split quad that breaks the hex's
face shape. All three add an element family or a constraint layer that every solver, estimator and exporter would have to
honour, for a gain the 2D measurements do not support (isotropic blocks were not cheaper than graded Delaunay).

Decision: **the 3D hybrid is the sweep of the 2D hybrid mesh.** `ElementKind::Wedge6` / `Wedge15` (VTK 13 / 26) are added,
and `sweep::extrude` maps Quad4/8/9 to Hex8/20/27 and Tri3/6 to Wedge6/15, so structured (Hex) and unstructured (Wedge)
regions meet on shared quadrilateral faces: the union is conforming by construction and the 2D interface machinery
(pre-split interface edges, welding) carries over unchanged. General hex-against-tet transitions (pyramids, ties) are
**declined**; a solid that is not a prism-like extrusion keeps the tetrahedral mesher (`tetmesh.rs`, `solid-kernel`).
Evidence: `tests/wedge.rs` (Hex+Wedge conformity, 3D affine patch test to 1e-12 in displacement and 1e-9 in stress, extruded
plane-strain solution equal to the 2D one to 1e-9, exact wedge-grid strain energy), element tests over `ALL_KINDS`.

## Addendum: non-rectilinear outlines, stretch guard, workbench controls

- **Outlines.** The partition no longer requires an axis-aligned outer boundary. Cut-cell style: cells the boundary crosses (or comes
  within one local size of, for angled and curved pieces) are unstructured and the unstructured region's loop uses the true boundary
  pieces, cut at the grid lines. Mapped (transfinite) boundary-following blocks and structured strips along angled edges were not
  built: they need a per-block mapping and a second interface rule, and the cut cells reuse the interface machinery unchanged.
- **Stretch guard instead of graded rows.** A structured cell whose elements would exceed 3x the requested aspect is given to the
  unstructured mesher. Per-cell graded transition rows would need non-uniform interface spacing shared with the unstructured side;
  the guard bounds the same quality loss with the existing uniform interfaces.
- **Multi-material.** Shared boundaries are cut at the corner nodes of both sides and re-meshed until the weld conforms (the
  assumption that both sides place the same nodes was false when a segment is encroached).
- **Workbench.** The controls listed in the readiness document are `Problem` fields with serde defaults, so older files read
  unchanged. Contact AMR through `run_amr_nonlinear` is not exposed for interference fits (measurement in the readiness document).

