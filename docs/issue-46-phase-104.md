# Issue 46 — Phase 104: ellipse and B-spline edges, cylinders with holes (#51)

Real parts are not made of lines and circles. Two of the first walls of the native import, found by exporting a handful of ordinary shapes from OpenCASCADE (a block drilled at an angle, a cylinder with a perpendicular branch, a cylinder with a cross bore) and running them through `step-inventory`:

1. a plane cut by a tilted cylinder has `ELLIPSE` edges (`unsupported referenced 3D curve geometry`);
2. two cylinders meeting have a degree-6 `B_SPLINE_CURVE_WITH_KNOTS` as their intersection (`native body supports only full-domain two-control degree-one splines`);
3. the face a hole is drilled into has two bounds (`cylindrical holes/unseamed bands are not implemented`).

## What changed

- **`ELLIPSE` edges.** `P(t) = origin + a cos t x + b sin t y`; the ends are located by their eccentric anomaly, the span follows the edge sense (a closed ellipse is a full turn), and the interior is bisected until every parameter interval's midpoint is within the chord tolerance of its chord. The edge/surface consistency check samples the whole ellipse.
- **General B-spline edges** (rational or not, any degree and knot vector, **whole domain**): the curve's two ends must be the topological vertices (in the edge's sense) within the geometry tolerance, else the edge is refused; the interior is sampled by `sample_spline_adaptive` (every distinct knot, `2 degree` points per span, then bisection until the quarter points and midpoint of each interval are within the chord tolerance). The certified sampler of Phase 8 (`nurbs_sample`, interval bounds) was **not** used: for the 9.5 mm degree-6 intersection curve of the cross bore it needs 134 points at 0.5 mm, **41 547 at 0.05 mm** and exhausts its segment budget at 0.005 mm (measured with a probe), because its error bound converges far slower than the sagitta; the shipped sampler is exact in the evaluation but certifies nothing (stated). The collinear quadratic of `step_spline_edge.rs` that used to be refused is now a spline edge whose samples lie on the segment (the test says so).
- **Grading and the new curves.** `insert_edge_splits` evaluates an ellipse at the interpolated eccentric anomaly and a spline at the interpolated parameter (the parameters of the samples are kept with the curve and updated), so graded sampling (Phase 100) works on them.
- **Cylinder faces with holes.** `sampled_cylindrical_trims` now unwraps every bound of the face, takes the loop of largest area as the outer one and shifts each other loop by whole periods until the exact classifier finds it inside the outer loop (a hole is contractible, so it sits in the chart once). `CylindricalTrims::loops` carries all bounds in source order (`boundary` stays the outer one). A hole that crosses the seam is not a separate bound in a valid B-rep (the seam edge is split and the arcs join the outer wire), and that case is handled by the existing single-wire path.

## Evidence (OpenCASCADE 8.0.1 exports in `tests/data`, `tests/curved_edges.rs`)

| shape | native (chord 0.01) | OpenCASCADE tessellation 0.001 | closed form |
|---|---|---|---|
| 40 x 30 x 20 block, hole r4 tilted (0.5, 0.2, 1) | 22859.6153 | 22858.3494 | 24000 - pi r^2 H / cos = 22858.2 (an inscribed hole leaves the block a little large) |
| cylinder R6 x 30 with a perpendicular branch R4 | 3827.1496 | 3830.4028 | |
| cylinder R8 x 30 with a cross bore R3 | 5583.7599 | 5587.2122 | |

All within 0.1 percent (the inscribed sampling); the tilted hole needs the explicit chart policy `CanonicalizeWithinTolerance` (the exporter's 4e-12 mm roundoff fails the strict single-turn certificate, as for the tilted cylinder fixture of Phase 14). Graded sampling lifts the three shells from 0.96, 0.34 and 0.12 degrees to 25.0, 20.1 and 12.4 degrees (the cross bore's saddle edge is a genuinely acute input angle). 236 solid-kernel tests pass without OpenCASCADE, 259 with.

## Limits (recorded)

- A B-spline edge must cover its curve entirely (the existing rule of the spline reader); a trimmed portion of a longer curve is refused, as is a spline or ellipse edge on a face made of a coplanar spline patch.
- Faces on **spline surfaces** (and surfaces of revolution / linear extrusion) are still unsupported: this phase changes edges and trims only. They are the next wall of any real part and the bulk of the remaining #51 work.
- The adaptive sampler checks three points per interval; a curve that wiggles between them at a scale below the chord tolerance is under-sampled. No Windows or hosted-CI evidence of its own.
