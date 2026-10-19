# Phase 122: native fillet of every edge of a convex polyhedron (#46/#63)

`SolidOp::Fillet` (the editor's "round every edge") needs `Brep::fillet_all`. The native version covers the shapes where the
answer has a closed form and an exact construction; everything else is refused with the reason (phases 120, 121 carry the rest of
the modelling plan).

## Method

For a convex solid `S` whose vertices each join exactly three faces (a box, any extruded convex polygon, a cut box), rolling a ball
of radius `r` over all edges gives the Minkowski sum of the inward offset `S_r` (each face plane moved in by `r`) with a ball:

* each original face stays on its plane, cut to the polygon of tangent points `V' + r n` (`V'` a vertex of `S_r`);
* each edge becomes a cylinder of radius `r` along the edge of `S_r`, an arc of the two face normals, bounded by two lines
  (tangent to the faces) and two circle arcs;
* each vertex becomes a sphere patch of radius `r` centred at `V'`, bounded by three of those arcs.

`V'` is the solution of the three shifted plane equations of its vertex. Checks before building: planar faces without holes and
straight edges only; every vertex inside every face plane (convex); three faces per vertex and two consistently oriented faces per
edge; the offset vertices inside all shifted planes and every offset edge still pointing the way the original does (so a radius that
collapses the body is an error, not a bad shape). Every loop is oriented by the sign of its Newell area vector against the face's
outward direction, so orientation is checked, not assumed.

Module: `solid-kernel/src/native_brep_fillet.rs`, `NativeBrep::fillet_all_convex(radius)`.

## Evidence

* `cargo test -p solid-kernel --test native_brep_fillet` (default build): 4 passed.
  * 2 x 3 x 4 box, r = 0.2: 26 faces (6 + 12 + 8), 24 vertices, 48 edges; sampled volume (chord 0.002) within 1e-3 of the
    rounded-box closed form, never above it; the largest radius allowed on a 2 mm side is below 1 (1.0 is refused, 0.99 builds).
  * Triangle, hexagon and trapezoid prisms against the Steiner formula `V + A r + M r^2 + 4/3 pi r^3` computed from an independent
    2D offset of the polygon: within 2e-3, face counts `(n + 2) + 3n + 2n`.
  * A rounded box written to STEP and read into the editable model keeps its planar faces on their original planes: a hole
    drilled afterwards (phase 121) has the closed-form volume.
  * Refusals: a non-convex L prism ("not convex"), a plate with a circular hole, a cylinder, radius 0 / negative / NaN.
* OpenCASCADE parity (`cargo test --release -p solid-kernel --features occt --test native_brep_occt -- --nocapture`):

  | shape | faces native / OCCT | closed form | OCCT reads native | OCCT `fillet_all` |
  |-------|---------------------|-------------|-------------------|-------------------|
  | box 2 x 3 x 4, r = 0.2 | 26 / 26 | 23.70169 | 23.69492 | 23.69493 |
  | hexagon prism R3 x 4, r = 0.5 | 38 / 38 | (Steiner in the default test) | 91.3647 | 91.3647 |

## Limits (honest)

* Convex, three faces per vertex, planar only. Concave edges (an L-bracket's inside corner), edges next to cylinders (a plate with
  holes, a bushing), vertices of degree four or more, variable radius, and filleting only some edges are not covered. Those need a
  rolling-ball engine with torus / blend patches on curved faces and vertex blends of mixed convexity: size L, and the biggest risk
  of the whole modelling plan (OpenCASCADE itself fails on many such parts).
* `round_edges_at_height` (cylinder ends: torus fillet, cone chamfer) is not done; it needs arc segments in `NativeBrep::lathe`.
* Not wired into `fea_problem::solid::apply_op` / `cad::Brep` yet.
