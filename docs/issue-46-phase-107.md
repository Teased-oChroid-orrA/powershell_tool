# Issue 46 — Phase 107: periodic (unclamped) spline edges and sweeps (#51)

Phase 106 refused a closed spline outline ("only nonempty clamped B-spline domains are supported"): OpenCASCADE writes a periodic spline with an unclamped knot vector and already extended control points.

## What changed

- `step_nurbs::read_curve_active_domain` and `BodyContext::read_spline_curve_active_domain` read such a curve with `NurbsCurve::new_active_domain`: evaluated on `[knots[degree], knots[controls]]` only, **no periodic wrapping or control extension is inferred** (the rule `nurbs.rs` already documents). The swept curve of a revolution/extrusion (Phase 106) and every B-spline edge curve of `AnalyticModel::read` use it; `read_curve` (clamped) and the strict `step_spline_edge` contract are unchanged. The edge rule stands: the curve's two ends must be the topological vertices within the geometry tolerance.
- **Defect fixed on the way** (`SplineSurface::from_geometry`): the last grid node was `lo + (hi - lo) * n / n`, which can round a ulp above `hi`; the curve evaluator then refuses it ("B-spline parameter outside its finite active domain"). The end node is now exactly `hi`.
- Fixtures (`write-shapes`, `Brep::prism_closed_spline`, `Brep::lathe_closed_spline`): `blob-prism` (a closed periodic spline outline extruded 6 mm: a wall closed in its curve parameter) and `spline-ring` (a closed periodic spline section turned about z: **one face closed in both parameters**, two seam edges).

## Evidence (`tests/swept_surfaces.rs`)

| shape | native (chord 0.01) | OpenCASCADE tessellation 0.001 |
|---|---|---|
| blob prism | 487.5904 | 487.9805 (-8.0e-4) |
| spline ring (closed in both parameters) | 648.4622 | 650.2975 (-2.8e-3) |

Closed shells, every face meshed. The ring's larger deficit is the inscribed sampling of a small-radius section at chord 0.01.

## Limits (recorded)

- A **periodic B-spline surface** (unclamped surface knots; `NurbsSurface::new_clamped`) is still refused: only curves gained the active-domain reader.
- A revolution that touches its axis (pole) remains refused by the trim check.
- No Windows or hosted-CI evidence of its own.
