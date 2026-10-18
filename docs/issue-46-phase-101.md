# Issue 46 — Phase 101: spherical, conical and toroidal faces; a cheap radius-edge recipe; three `plc_conform` defects (#51/#52/#53/#63)

The native STEP pipeline imported planes and cylinders only. The filleted-box target of `solid-kernel/tests/parity_occt.rs` (8 `SPHERICAL_SURFACE`, 12 `CYLINDRICAL_SURFACE`, 24 `CIRCLE` edges; the 32 `B_SPLINE_CURVE_WITH_KNOTS` of that file are the **p-curves** in the spheres' parameter space, not edges: the 3D edges are lines and circles, which were already native) was refused with `unsupported native analytic surface`. It now imports, with cones and tori.

## Charts (`solid-kernel/src/curved_surface.rs`)

Every curved face is triangulated in a plane chart in millimetres, so exact trim classification, the constrained Delaunay triangulation, the quality refinement and the grading of Phase 100 apply unchanged. Each chart is angle-preserving where the surface allows it, and a counter-clockwise chart triangle is the *outward* geometric normal (unit tests assert it per surface):

- **Sphere**: stereographic projection from the antipode of the **face's own centre** (conformal). The centre is the direction of the vector area of the face's largest bound loop on the side the face occupies (`same_sense` decides which; a seam traversed twice cancels). A face must stay within 120 degrees of it, so the source poles and the periodic seam never matter: a hemisphere or a corner patch whose pole is a vertex is ordinary. A sphere's seam edge is a zero-width slit in this chart and is dropped from the boundary (`planar_step::chart_usages`).
- **Cone**: the development into the plane (isometric): slant distance from the apex and the angle `-lon sin(semi_angle)`, periodic with period `2 pi sin(semi_angle)`. A full band is a sector with two straight seam sides, like a cylinder's. The apex is supported (angle undefined there; the closure test is waived at it and the single-turn extent test and the polygon classification stay).
- **Torus**: the conformal coordinates `(R u, R w(v))`, `w(v) = 2 r / sqrt(R^2 - r^2) atan(m tan(v/2))`, periodic in both; a full torus (one face, two seams) is a rectangle.
- Edge/surface consistency (`on_surface`): the distance of the declared locus (the segment, or the whole circle) from the surface, within the geometry tolerance. The cone's semi-angle needs one explicit SI-radian plane-angle unit; degree files are refused, not guessed.
- Interior points of a curved face follow the sagitta (the surface point of a chart edge's midpoint against the chord midpoint, within the chord tolerance); the cylinder keeps its closed-form bound.

Refused with a reason: a full sphere (a vertex-loop face; also beyond 120 degrees of its own centre), horn/spindle tori, a cone/torus face with holes or an unseamed band, a cone point inside the apex tolerance without a closing seam.

## Evidence (OpenCASCADE 8.0.1 exports, `tests/data`, closed forms)

`tests/curved_step.rs` (default features; the STEP files are committed) and `tests/parity_occt.rs` (OpenCASCADE's own tessellation as the reference), chord 0.005 mm, volume against the closed form:

| shape | exact | native | OpenCASCADE tessellation |
|---|---|---|---|
| box 2 x 3 x 4, every edge rounded 0.2 | 23.7017 | 23.6835 (-0.077 %) | 23.6927 |
| cylinder R5 x 10, top edge rounded 1.5 (torus) | 771.2454 | 770.5707 (-0.087 %) | 770.6489 |
| same, top edge chamfered 1.5 (cone) | 753.5895 | 752.9500 (-0.085 %) | 753.0577 |
| frustum R6 -> R3, height 8 | 527.7876 | 527.3852 (-0.076 %) | 527.2475 |
| torus R6 / r2 (one face) | 473.7410 | 472.8504 (-0.188 %) | 472.7175 |
| cone R5 x 8 with its apex | 209.4395 | 209.1662 (-0.130 %) | |
| hemisphere R4 | 134.0413 | 133.8558 (-0.138 %) | |
| capsule (cylinder R3 x 8, two caps) | 339.2920 | 338.6511 (-0.189 %) | |

The inscribed sampling is a little small and never large; the shells pass the facet-intersection check at the default chord. Graded sampling (Phase 100) lifts every one of them from 2.5-8.5 degrees to 23-25.5 degrees (`graded_sampling_lifts_curved_shells_over_the_registry_gate`). The parity matrix no longer pins the filleted box as refused.

## A cheap recipe for radius-edge 2, and three `plc_conform` defects it exposed

- **Graded shell + `protect_encroached: false` + `max_radius_edge: Some(2)`** reaches a radius-edge ratio of 1.96-2.0 on the frustum, cone with apex, cylinder with a rounded end, capsule and torus with 9-244 segment Steiner points and 1000-5000 cells (1-12 s release), against 2400-5900 segment Steiner points and 15 000-38 000 cells for the default protect-everything rule. The graded box and the hemisphere leave one unresolved sliver (reported). `Options::default()` is unchanged.
- **Defect (fixed): quality refinement re-inserted the same point forever.** The circumcentre of a near-flat cell is ill-conditioned; rounded, it can lie outside the exact circumsphere, so inserting it destroys nothing and the same cell (and the same centre, now a duplicate vertex) returns each round until extraction fails with `duplicate point coordinates`. It hit the graded torus, filleted box and rounded cylinder. A centre is now inserted only when `sphere_side` says it is strictly inside its cell's circumsphere; otherwise the cell is counted unresolved.
- **Defect (fixed): unbounded concentric-shell cascade.** The frustum's 69 degree wedge at its base rim makes protect-everything refinement split segments around one vertex down to 7e-15 mm and still "succeed". `Options::min_segment_relative` (default 1e-6 of the bounding diagonal) now stops it with `refinement descends below the feature-size floor around a vertex`; the cheap recipe above handles the same shape (shortest cell edge 5e-2 mm). The slab test of 7-degree diagonals ends at the budget or at this floor.
- **Defect (fixed, Phase 100 follow-up): CAD exporters' coordinate noise.** A cone apex exported at `3.5e-32` mm failed `coordinate is outside the supported arithmetic range`; magnitudes below 1e-25 mm in source points and in sampled circle points are now exact zeros (nothing above it changes: the bracket's shell is bit-identical).

## Not done (recorded)

A full sphere (needs vertex-loop faces and a two-chart split), NURBS/B-spline surface faces and rational B-spline *edge* curves in 3D (the exports above never use them for edges; trimmed spline surfaces keep the exact-domain work of #51), degree plane-angle units, faces with holes on cones and tori, horn/spindle tori. Acute wedges (< 90 degrees) between faces still make the protect-everything refinement cascade (now bounded); the missing-only recipe is the workaround. No Windows or hosted-CI evidence.
