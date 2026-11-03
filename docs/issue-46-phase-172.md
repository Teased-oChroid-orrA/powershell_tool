# Phase 172: why facets named after a flat face are tilted at a concave edge

Issue #46. Follows [Phase 171](issue-46-phase-171.md), which found that the benchmark load on `screw.step` is off traction x exact area (OCCT path +2.7 %, native -0.4 %) because the volume mesh's facets named after the loaded face include tilted ones. This phase finds where those facets come from. Diagnosis only: no mesher, naming, solver or tessellation change, and no gate, tolerance, budget, cap or timeout changed. `occt_retirement_approved` stays false.

## Diagnostics added

- `fea-problem/tests/facet_naming.rs` (native pipeline, no OpenCascade; ignored, about 5 s in release): a 10 x 10 x 4 box with a R2 x 3 boss on its top. For each flat face it prints the area of the facets named after it against the surface triangles' area, and groups the facets more than 45 degrees off the face's plane by the model faces their corners lie on (within a hundredth of the size, the namer's `FACE_TOL`).
- `fea-problem/tests/occt_vs_native.rs` (`occt` feature, ignored):
  - `flat_face_facet_areas`: the same comparison for every flat face of every model, on both paths;
  - `loaded_face_tilted_facets`: the benchmark's loaded faces only, grouping each facet tilted more than 1 degree by tilt, corner faces and naming rule (all three corners on the face, or the face nearest the centroid).

## Observed

**Built-in models** (`flat_face_facet_areas`, default size, both paths). Facets more than 45 degrees off a flat face's plane occur only where that face meets another along a concave edge:

- `box + boss (native)`: 2.35 mm² on the box top, the face round the boss (2.7 % of its area, on both paths);
- `plate with a boss (OCCT)`: 0.07 mm²;
- `box + boss + hole (native)`: 0.14 mm²;
- `NEMA17 motor bracket`: up to 9.5 mm² (OCCT) and 10.9 mm² (native) on one face (1.6 % / 1.8 %).

Boxes, cylinders, plates with through holes, filleted boxes, the cross-bores, the t-junction, the tilted hole and the vase have none: their flat faces meet their neighbours along convex edges only.

**Box + boss** (`facet_naming`, size 0.877). All 24 steep facets named after the top face are bridges across the concave circle (r = 2, z = 4). Each has two corners on the top plane at r ≈ 2.2 and one on the boss wall at z ≈ 4.37; no corner is on the circle itself. They are tilted about 45 degrees and their centroids, at z ≈ 4.125, are nearer the top face, so the centroid rule names them after it. The top face's named area is still right (-0.007 %), because its flat facets stop short of the circle by about as much.

**`screw.step` slot bottom** (`loaded_face_tilted_facets`, size 0.833). The loaded face 3 is the bottom of the screwdriver slot, and it meets the slot walls along concave edges. Its 6.65 mm² (OCCT) and 5.51 mm² (native) of facets tilted more than 1 degree are of two kinds:

| Corners | OCCT | native | What they are |
|---|---|---|---|
| one on the edge (walls and bottom), one on a wall only, one on the bottom only; tilt 40-90 degrees | 3.29 mm² | 3.13 mm² | facets across the edge, most of them at right angles to both faces: the boundary steps along the concave edge |
| at least one corner on no face (not snapped within a hundredth of the size); mostly under 30 degrees | 3.16 mm² | 2.31 mm² | facets of the bottom with an unsnapped corner |
| other | 0.20 mm² | 0.07 mm² | |

Both paths have the same kinds of facet in about the same amounts. The volume mesher is the same, so the difference comes from the surfaces the two paths hand it.

## Cause

The snap does not place boundary vertices on a concave crease:

- `snap_to_features` keeps a vertex that is already on a wall on that wall at a proven reentrant edge (Phase 139: moving an exact wall point onto the rim was the regression `reentrant_snap_preserves_a_point_already_on_the_wall`);
- `align_near_seams` pulls a vertex onto a seam only within `OFF_SURFACE` (0.05) of the local size, and only when the vertex's facet has two corners on the target face already. In the box + boss the wall corner of a bridge is about 0.43 of the size from the circle.

So the boundary crosses the concave edge with facets that lie on neither face, and `face_of_triangle` names them after the face nearest their centroid. That is the "closed sharp feature is rounded by the snapping within one element" limit stated at the top of `fea-core/src/tetmesh.rs`, here specifically at concave edges. It is not a naming bug: no name makes a bridge facet part of either face, and its area is the mesh's error in following the surface.

## What a fix needs (not attempted)

Removing the bridges means putting vertices on the concave crease. A vertex move alone cannot do it: the bridge facet would collapse, and the moves that `align_near_seams` makes are refused when a facet's area or normal changes that much, as they must be. It needs boundary topology changes (edge splits on the crease, with the new vertices projected onto it), each under the transaction rules of Phases 138-147: manifold boundary, positive embedded cells, CAD face ownership, the quality gate. That changes every solid mesh with a concave edge in the app, on both paths, so it needs its own phases with corpus-wide evidence. It is listed as open work, not done here.

A different remedy would scale each traction so its total force equals traction x the face's surface area. That changes what a traction load means, so it is a product decision; it is not taken here either.

## Limits

- macOS arm64 only; default element size only.
- The second kind of `screw.step` facet (corners not snapped) was counted but not traced to its vertices.
