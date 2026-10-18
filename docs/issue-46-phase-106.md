# Issue 46 — Phase 106: surfaces of revolution and of linear extrusion (#51)

Phase 105 ended with "surfaces of revolution / linear extrusion" open. OpenCASCADE writes a B-spline profile turned about an axis as `SURFACE_OF_REVOLUTION` and a B-spline outline pushed along a direction as `SURFACE_OF_LINEAR_EXTRUSION`; both were refused ("unsupported native analytic surface").

## What changed

- **One parametric surface type.** `SplineSurface` (renamed variant `Surface::Param`, formerly `Surface::Nurbs`) now evaluates one of three geometries: a B-spline surface, `Revolution` or `Extrusion`. Everything else (parameter-rectangle chart, multi-start nearest point, `chart_loop` seam continuation, per-sample edge distance check, graded sampling, face identity as `Kind::Spline`) is the Phase 105 code, unchanged.
- **Revolution.** `S(a, t)`: the swept B-spline curve at `t`, turned by `a` in `[0, 2 pi]` about the `AXIS1_PLACEMENT` axis (Rodrigues). The parameter order is STEP's (angle first): the first attempt had it reversed, which flipped the normal and the shell failed the orientation check ("not closed and consistently oriented").
- **Extrusion.** `S(t, h)`: the curve at `t` moved by `h` along the `VECTOR` direction. The surface is unbounded along it, so the height range is the extent of the face's own boundary samples along the direction plus a 5 percent margin.
- `read()` takes the swept curve through the existing B-spline curve reader (`step_nurbs::read_curve`, source scale applied); a swept curve that is not a B-spline is refused with that reason.
- `Brep::lathe_spline` and `Brep::prism_spline` (feature `occt`) and `write-shapes` produce the fixtures `vase` and `wavy-prism`.

## Evidence (`tests/swept_surfaces.rs`, OpenCASCADE 8.0.1)

| shape | native (chord 0.01) | OpenCASCADE tessellation 0.001 |
|---|---|---|
| vase: spline profile (4,0) .. (4.5,20) turned about z | 1501.3212 | 1503.0205 (-1.1e-3) |
| prism: spline outline (0,0) .. (20,0) extruded 8 | 440.7547 | 440.8099 (-1.3e-4) |

Closed shells with every face (3 and 4) meshed; a finer chord converges to the reference. Graded sampling with quality refinement works (vase 14.3, prism 17.6 degrees registry angle at `--grade 25`).

## Limits (recorded)

- **Periodic / unclamped B-spline sweeps are refused**: a closed spline outline (OpenCASCADE's periodic spline) is written with unclamped knots, and the NURBS evaluator supports clamped domains only ("only nonempty clamped B-spline domains are supported"). Supporting it means an unclamped basis in `nurbs.rs`.
- A swept line, circle or ellipse is not handled here (those normally come out as planes, cylinders, cones or tori).
- A revolution face that touches its axis (a pole) has a collapsed chart edge and is refused by the trim check; a face closed in both parameters is untested.
- The extrusion height range comes from the face's boundary samples; a face whose samples span no height is refused.
- No Windows or hosted-CI evidence of its own.
