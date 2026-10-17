# Issue 46 — Phase 59: facet tie alignment, robust extraction and radius-edge refinement

Stacked on Phase 58 (PR #86). All changes are in `solid-kernel` (`plc_conform`, `delaunay3`) plus the `step-inventory` flags.

Fixes found while exercising Phase 58 on harder inputs:
- **Cospherical-tie divergence.** A facet's planar Delaunay triangulation and the tetrahedralization pick different diagonals of exactly cocircular quads (grids, right angles); the "absent" sub-facets were then refined forever toward a point. `FacetMesh::flip_toward` flips interior edges to the other diagonal when both new triangles are faces of the tetrahedralization; refinement only sees what flipping cannot fix.
- **Extraction leak.** Seeding the outside flood from hull faces that are not sub-facets let a rounding-created sliver hull face mark every cell outside. Inside/outside now starts from the cells on the inner side of each outward sub-facet (exact `orientation`) and spreads across non-facet faces.
- **Facet insertion robustness.** The planar Bowyer–Watson cavity is now the connected set from the triangle holding the point (or, for a split edge, the triangle on that edge), so rounding cannot produce an empty or non-star cavity; a circumcentre the facet mesh refuses splits the nearest boundary sub-segment instead.

New: `Options::max_radius_edge` inserts circumcentres of interior tetrahedra above the bound (worst first, batched, circumcentres that encroach a sub-segment or sub-facet split those instead; unresolved cells are counted). The report adds `quality_steiner`, `max_radius_edge_ratio` and `unresolved_bad_cells`. `step-inventory --conform-radius-edge B` exposes it.

Evidence: a 3x3x1 slab with bound 2 goes from radius-edge 2.179 to 1.967 (17 to 114 cells) with exact volume 9 and no unresolved cells; `solid-kernel`: 202 passed with `--features occt`, 185 with `--no-default-features`, 0 failed/ignored.

Known limits (pinned by tests, not fixed): a thin 4x4x0.5 slab recovers but radius-edge refinement diverges toward the corners where side-wall diagonals meet the box edges at 7 degrees (it refuses with a `Limit` error, as does the NEMA17 sampled shell from Phase 58). The bound only constrains radius-edge ratio; slivers are not addressed.
