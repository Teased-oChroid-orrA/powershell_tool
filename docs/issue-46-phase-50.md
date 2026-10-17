# Issue 46 — Phase 50: bounded quadratic sampler acceleration

Continuation from merged PR #77 and tracking-only main `caa0178`, on `feature/native-cad-bezier-sampling`.

The quarter-circle sampler at unchanged tolerance 0.001 previously required 4094 segments and 24561 enclosure work units. A regression requiring the same error bound with a 64-segment budget failed before this change (exit 101). Positive-weight, Bezier-separated quadratic spans now use their rational Cartesian control hulls, subdivided with outward homogeneous arithmetic and intersected proven convex hulls. Distance to a convex segment is convex: control-box corner distance bounds therefore bound the complete curve; endpoint enclosures retain the symmetric Hausdorff argument. Other degrees/knot layouts retain the existing conservative enclosure path.

Child nets use an enclosure of the actual representable split factor, rather than assuming exactly one half. Tests include a three-ULP parameter range whose midpoint is asymmetric, non-dyadic domains, analytic arc/chord distances, cubic hidden excursions, C0 knots, hard budgets and initial/final cancellation. Targeted six sampler tests pass. The quarter-circle case now uses 32 segments / 189 work units, with no tolerance change. Work counters count three processed proof boxes per interval, not literal calls to the original curve enclosure routine. Cached control nets increase per-task memory; this is a measured work reduction for a pilot curve, not a universal memory/throughput claim.

[Full qualification](issue-46-phase-50-verification.json): 177 CAD-enabled / 160 feature-off tests, zero failures/ignored/warnings; app CAD library/tests compilation passes at source `1306f235aa42839c046985391d26dcecfff9449b`. No new dependencies or hosted CI. OCCT remains production; no full spline trim/master/body or general constrained-meshing claim.

External unoptimized pilot (same program, one call each; cache uncontrolled): before 0.046489935 s, after 0.000465503 s; observed error bound after 0.0006699820683854008, below unchanged 0.001. These isolated timings do not establish representative production speedup.
