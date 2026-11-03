# Phase 169: native STEP spline edges that use part of their curve

Issue #46. Follows [Phase 168](issue-46-phase-168.md). After that phase, `screw.step` was the one corpus file that OpenCASCADE meshes and native refuses, with "a spline edge must cover its whole curve". Five corpus files stopped on that check. Measured against the owner's retirement gate (`issue-46-abc-and-retirement-gates-plan.md`). No gate, tolerance, budget, cap or timeout was changed. `occt_retirement_approved` stays false.

## Cause

In `screw.step`, edges #336 and #426 are the two halves of one cubic B-spline (23 control points, domain [-9.753, 9.753]). Both use the whole curve declaration, split at t = 0. The shared vertex lies on the curve to about 1e-14 mm, but it is not an end of the curve. `AnalyticModel::read` required each vertex of a general spline edge to be an end of the curve's domain. STEP does not store trim parameters on an `EDGE_CURVE`: the vertices bound the edge, and an exporter may reuse one curve for several edges.

## Change

`solid-kernel/src/analytic_step.rs`:

- For each vertex of a general B-spline edge (taken in the curve's direction), the reader first checks whether it is the curve's end within the geometry tolerance, as before. If not, `spline_vertex_parameter` locates it inside the curve:
  - The bounded global curve distance (`nurbs_distance::distance`) gives a candidate parameter. The vertex must lie within the geometry tolerance of the curve. The search is cancelled as soon as its lower bound proves the vertex is farther than that, so an off-curve vertex refuses quickly instead of spending the enclosure budget.
  - A distance bracket does not prove the parameter is unique (the Phase 53 note: "do not infer unique parameters from a bounded distance objective alone"). So the curve is also sampled to the chord tolerance over its whole domain. The polyline segments within chord + geometry tolerance of the vertex must form one run, and that run must hold the candidate. A curve that passes the vertex twice refuses ("lies on its curve more than once") rather than picking one pass.
- The edge samples only its interval (`sample_spline_range`). Vertices out of order on the curve (an edge across a closed curve's seam) refuse by name.
- The unused `sample_spline_adaptive` wrapper was removed. The sampler's doc comment now sits on `sample_spline_range`, and `Curve::Spline`'s doc says that it may cover part of its curve. Downstream code already handled such pieces (`split_edge_at_sample` makes them), because every consumer uses the stored sample parameters, not the curve's domain.
- The degree-one, two-point path (`step_spline_edge::sample_declaration`) still requires whole-domain endpoints. No corpus file was seen to need more.

## Tests

- `solid-kernel/tests/curved_edges.rs` `a_spline_edge_that_uses_part_of_its_curve_reads_and_keeps_the_volume`: `cross-bore.step` with spline edge #79 split at 37 % of its domain into two edges that share the curve. The pieces meet at the new vertex and keep the unsplit edge's ends. The volume at a 0.01 mm chord is 5583.774 mm³ against 5583.798 unsplit (relative -4.3e-6). The same split with the vertex moved 0.01 mm off the curve refuses ("neither an end of its curve nor on it").
- `analytic_step` unit test `a_vertex_inside_a_spline_is_located_once_or_refused`: an interior point of a cubic is located; a point 1e-3 mm off refuses; on a degree-one polyline that crosses itself at (1, 0, 0), that point refuses as "more than once" and a point passed once is located.
- Suites, macOS arm64, release: `solid-kernel` 317 passed, 0 failed, 4 ignored; `fea-problem` 220 passed, 0 failed, 7 ignored.

## Corpus

Same 35 files as Phases 165 and 168 (local copy, not committed), mesh only, macOS arm64, release, 1,964 s (two test suites ran alongside it), same command as Phase 168.

Summary: **35 models; OCCT meshed 3, passed the default gate 1; native meshed 2, passed the gate 1**. The OCCT rows, and native's two meshed files (`linkrods`, `ftc_11_rb`), are unchanged.

The five files that stopped on spline coverage:

| File | Strict read now | Tolerant re-read now |
|---|---|---|
| `screw.step` | horn and spindle tori are not implemented | same |
| `nist_ctc_02_asme1_rc` | a spline edge's vertex is neither an end of its curve nor on it | surface chart work budget exceeded |
| `nist_ftc_07_asme1_ap242-e2` | same as above | surface chart work budget exceeded |
| `nist_ftc_11_asme1_ap242-e2` | same as above | edge/surface geometry disagrees by 1.36e-2 mm |
| `nist_ftc_10_asme1_ap242-e2` | edge endpoint is not on its declared line | (not reached) |

The three NIST files whose strict read stops on the new message have spline-edge vertices more than 1e-7 mm off their curves. That is the strict geometry tolerance, and the refusal is correct there. The tolerant re-read (`Options::foreign_cad`, tolerance chord / 10) accepts those vertices and stops later, on the same classes as other NIST files. No file refuses on spline coverage any more.

## What this shows and does not show

- It removes the spline-coverage blocker. All five files get past it: two at the strict tolerance, three under the tolerant re-read.
- It does not move the gate count. `screw.step` is still the one file that OpenCASCADE meshes and native does not. It now stops one stage later, on its head.
  - The head is a spindle torus (major radius 8.25 mm, minor radius 54.87 mm).
  - All three of the faces on it lie on the torus's inner (lemon) part. In the meridian plane, their vertices fit the circle centred at minus the major radius: for example ρ = 10, h = -51.75.
  - Face #874 is a cap bounded by one circle at ρ = 10. Its interior holds the cusp on the axis (h = -54.25), a cone-like apex with a half-angle of about 81 degrees.
  - Native's torus chart is the conformal ring-torus chart (`curved_surface::tube_scale` needs major > minor). A spindle torus needs a new chart for the lemon part and apex handling like the cone's. That is a separate surface feature, not part of this change.
- It was measured on macOS only. The hosted Windows STEP workflow runs on the pull request.

## Follow-ups

- Spindle torus (lemon part with a cusp inside a face), for `screw.step`.
- The remaining native strict-read classes on the corpus, led by edge/surface disagreement and the surface chart work budget (Phase 168 table, updated above for the spline rows).
