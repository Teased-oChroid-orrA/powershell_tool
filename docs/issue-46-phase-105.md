# Issue 46 — Phase 105: faces on rational B-spline surfaces (#51)

Phase 104 ended on "faces on spline surfaces are still unsupported". OpenCASCADE writes a fillet along an intersection curve (a rounded cross bore, a rounded branch, a rounded tilted hole) as one `B_SPLINE_SURFACE_WITH_KNOTS` + `RATIONAL_B_SPLINE_SURFACE` face. This phase imports them natively.

## What changed

- **`Surface::Nurbs(Arc<SplineSurface>)`** (`analytic_step.rs`, `curved_surface.rs`). A `B_SPLINE_SURFACE_WITH_KNOTS` face first tries the existing flat bilinear `PlanarPatch`; a 2 x 2 degree-1 net that is not exactly flat keeps its refusal ("not exactly coplanar", `tests/step_nurbs_surface.rs`), and any other net becomes a `Nurbs` surface.
- **Chart.** The parameter rectangle scaled by the mean metric of a 24 x 24 grid (`X = su (u - u0)`, `Y = sv (v - v0)`), so chart lengths are roughly millimetres; it is not conformal. Interior points evaluate the exact surface (`point_curved`; a chart point a few ulp outside the rectangle is clamped, which boundary splits need). The normal is `S_u x S_v` from central differences (the exact evaluator's derivatives refuse at C0 knots, and these surfaces are C0 at every interior knot).
- **Locating a world point** (`SplineSurface::candidates`, `refine`): damped Gauss-Newton from the six nearest grid nodes, the distinct results within a micron of the best kept.
- **Seams** (`SplineSurface::chart_loop`, used by `sampled_planar_trims`). A fillet ring is closed in one parameter, so a point on the seam has two chart images and the nearest-point search alone cannot tell them apart. Each sample of a boundary loop now takes the image nearest the previous sample's chart point; the first sample tries each of its images and the loop of least chart length wins. The seam edge appears twice in the loop (once per side), at `v = v0` and `v = v1`, giving the expected rectangle.
- **Edge/surface check.** An edge of a spline-surface face must lie on the surface along its samples (a per-sample distance, since an arc of a circle between samples need not), within `geometry_tolerance_mm`. OpenCASCADE's intersection-curve approximations deviate about 1e-5 mm from the fillet surface, so these files need `--geometry-tolerance 0.0001` (`step-inventory` takes `--canonical-surface`; `shell-sampling-probe` gained `--tol`).
- `face_identity` signs a spline face as `Kind::Spline` (position by bounding box, neighbours, edge and loop counts); `native_surface` takes the face normal from the chart.

## The defect that blocked the first attempt

All three shapes failed with "an edge sample is 0.15-0.44 mm off the face's spline surface". The surface evaluation and the edge association were right (edge #621 lies on the `u = 0` row of surface #899 within the 0.0016 mm resolution of a probe). The nearest-point search was wrong: the surface is closed in `v` (`v = 0` and `v = 19.0268` are the same ring), the grid tie chose `v` max, and a clamped descent from there cannot cross the seam to the true nearest point at `v = 0.276`. Multi-start refinement fixes the distance, and the continuation above fixes the trim chart.

## Evidence (`tests/spline_surfaces.rs`; OpenCASCADE 8.0.1 files in `tests/data`)

| shape | native (chord 0.01) | OpenCASCADE tessellation 0.001 |
|---|---|---|
| cylinder R8 x 30, cross bore R3, fillets 0.5 | 5574.7804 | 5578.1288 (-6.0e-4) |
| cylinder R6 x 30, branch R4, fillets 1.0 | 3806.6071 | 3810.0425 (-9.0e-4) |
| 40 x 30 block, tilted hole, fillets 0.5 | 22835.2629 | 22835.2431 (+9e-7) |

(inscribed sampling is a little small). All three are closed shells with a triangle on every source face (9, 14 and 29 faces). A renumbered re-export of the cross bore associates every face exactly, spline faces included (`tests/face_identity.rs`). Graded sampling with chart quality works on them: the branch goes from 0.44 to 21.3 degrees, and the cross bore's spline fillet face has no corner under 13.7 degrees. `filleted-box` (planar patch / cylinder / sphere fillets) is unchanged.

Also fixed in passing: `shell-sampling-probe --canonical` was parsed and ignored (now sets the chart policy), and three warnings in tests (unused import, two needless `mut`).

## Limits (recorded)

- The chart is the parameter rectangle. A face whose trim loop wraps a closed parameter in a way the nearest-image continuation cannot follow (a hole around the seam, a face closed in both parameters) is not tested; a failed projection or a self-touching polygon refuses the import.
- The nearest-point search seeds from a 24 x 24 grid and six starts. A feature narrower than a grid cell can yield a wrong local minimum; the per-sample edge distance check then refuses rather than accepts, but the interior normal would not be checked.
- The worst corners of the graded shells (0.2 degrees at a torus/plane pair of the end fillet of the cross bore; 0.35 degrees at the acute ellipse edges of the tilted hole) are on tori, planes and the Phase 104 ellipse edges, not on spline faces.
- Surfaces of revolution and of linear extrusion, exact (non-sampled) trims, and B-rep healing of sliver faces (the NEMA17 bracket) are still open. No Windows or hosted-CI evidence of its own. A shape import takes 0.6-1.4 s in release (nearest-point search per boundary sample).
