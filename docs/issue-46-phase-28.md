# Issue 46 — Phase 28: bounded rational curve enclosure and sampling

Sources `92bd937` (enclosure) and `166279e` (sampling). This advances the native CAD geometry gate, not full spline-body import or production strategy availability.

## Enclosure contract

`NurbsCurve::enclose` bounds a closed parameter subdomain across its intersected knot spans. It uses the same homogeneous de Boor recurrence as point/Dual evaluation through a minimal arithmetic/blend trait. Existing Dual/Scalar evaluation remains on its original arithmetic path. A private binary64 interval implementation rounds primitive operations outward; denominator subtraction and homogeneous control products are included. Convex blending intersects with the input hull only where active-span coefficients are mathematically in [0,1]. Positive rational weights remain bounded away from zero. Nonfinite/domain/order/work-budget inputs refuse.

Bounds describe the exact real rational curve defined by accepted binary64 controls/weights/knots. They do not recover decimal source precision, prove regularity or injectivity, or validate CAD trims. There is no new dependency, second AD engine, tolerance relaxation or global derivative capability.

## Sampling contract

`nurbs_sample::sample` retains distinct knots and evaluator endpoints, recursively subdividing entire parameter intervals. Each output segment carries a symmetric Hausdorff upper bound in the control coordinate units. Source-bound STEP declarations already use mm; standalone evaluators may use other explicit units. A positive-weight degree-one span traces a straight segment and uses its endpoint-rounding bounds without unnecessary subdivision, including diagonal spans.

For a general interval, distance to the segment is convex; bounding every corner of the curve enclosure bounds every enclosed point. Each corner uses an outward-rounded distance to a computed real convex blend of segment endpoints (the blend need not be the exact nearest point). Endpoint enclosures additionally bound evaluation rounding. A continuous curve inside a segment's epsilon capsule with both endpoints within epsilon of their corresponding segment endpoints has symmetric Hausdorff distance at most epsilon: use the intermediate value of its axial projection for interior segment points, and an endpoint for any uncovered end interval. This requires the existing clamped/continuous/positive-weight curve admission; it is not a simple-curve or edge/surface agreement certificate.

Progress reports cumulative enclosure calls, accepted segments and pending intervals rather than a fabricated percentage. Cancellation is checked before work, at every adaptive interval and before returning the immutable result. Segment, recursion and enclosure budgets, and parameter precision exhaustion refuse atomically; no partial result or implicit tolerance increase is returned.

## Evidence and limitations

Eight new regressions cover outward rounding/subnormal arithmetic/zero denominators; rational-circle analytic enclosure and shrinking subdomains; hidden S-curve excursions despite a chord-aligned midpoint; span/domain/work refusals; analytic circle curve-to-chord and chord-to-arc distances; diagonal straight/C0 knot preservation; independent cubic geometry; and initial/final cancellation plus each resource limit. Existing evaluator/Dual and native pipeline tests remain required.

The development circle case uses 4,094 segments and 24,561 enclosure calls at tolerance 0.001; the four sampling tests complete in 0.04 s locally (not a standalone benchmark or representative CAD throughput/memory claim). Axis-aligned interval bounds lose correlations and over-refine curved spans compared with a tighter rational-Bezier/control-polygon certificate. Do not market this as an optimized production spline mesher. Tiny tolerances/poorly conditioned weights may hit explicit budgets. No sampled-point-only test substitutes for interval acceptance.

Full source-specific tests and app compilation are recorded in [verification](issue-46-phase-28-verification.json). No hosted CI, main modification, production switch or OCCT retirement. Next gates: tighten conservative curve bounds; integrate source-bound edge sampling/endpoint agreement; bound surface patches and projection; validate p-curves/trims and full edge/surface loci before enabling native spline B-Rep import. General CAD/assemblies, PLC recovery/sliver/conditioning, P2/P3 and final parity remain open.

Final checks at `166279e61b964c846cda71f8c4add6be76287305`: **114 CAD-enabled / 97 feature-off tests and CAD-enabled app lib/tests compilation pass**, zero failures/ignored tests.
