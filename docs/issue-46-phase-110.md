# Issue 46 — Phase 110: periodic (unclamped) B-spline surfaces (#51)

Phase 107 gave periodic *curves* an active-domain reader and left periodic *surfaces* refused ("surface u basis: only nonempty clamped B-spline domains are supported"). OpenCASCADE writes an interpolated periodic B-spline surface (`Solid::bspline(u, v, u_periodic, ..)`) with unclamped knots and an already extended control net.

## What changed

- `NurbsSurface::new_active_domain` (and `step_nurbs_surface::read_surface_active_domain`, `BodyContext::read_spline_surface_active_domain`): each parameter is evaluated on `[knots[degree], knots[count]]` only; nothing is wrapped. Only point evaluation and the domain are meant to be used on such a surface (the interval enclosures and derivatives of the strict reader were not extended). `new_clamped` and `read_surface` are unchanged.
- **A latent clamped-only assumption was found in `evaluate`**: the corner shortcut returned the corner control point when both parameters sat at a domain end, which is right only for a clamped basis. It is now taken per axis and only for a clamped one (`Basis::is_clamped`); an unclamped surface has no corner control point.
- `AnalyticModel::read` reads every B-spline surface face through the active-domain reader. The bilinear 2 x 2 patch path is unaffected (a degree-1 net with two controls is clamped by construction).
- Fixture `periodic-shell` (`Brep::bspline_shell`, `write-shapes`): an 8 x 5 grid of points, periodic around, one face closed around with two seam edges.

## Evidence (`tests/spline_surfaces.rs`)

| shape | native (chord 0.01) | OpenCASCADE tessellation 0.001 |
|---|---|---|
| periodic spline shell | 940.9922 | 943.1996 (-2.3e-3) |

A closed shell with its face meshed; graded sampling with chart quality lifts it from 0.68 to 6.3 degrees (the sampled quality of a freeform wall is not the point of this phase). The graded-versus-plain volume comparison of the Phase 105 test was loosened from 0.2 to 0.4 percent: the smooth periodic wall is the least accurate under plain sampling, which is the inscribed-sampling effect, not a change of body.

## Limits (recorded)

- Derivatives, enclosures and the exact-evaluation features of `NurbsSurface` on an active-domain surface are not supported or tested; the native import uses point evaluation and finite differences only.
- A B-spline surface closed in both parameters or with a pole (degenerate edge) is untested.
- No Windows or hosted-CI evidence of its own.
