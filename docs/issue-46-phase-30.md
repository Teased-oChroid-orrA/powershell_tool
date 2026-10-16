# Issue 46 — Phase 30: tensor surface patch enclosures

Source `d3e81dc` adds `NurbsSurface::enclose` over a closed U/V rectangle. Both validated bases use the shared outward-rounded homogeneous recurrence; weights are retained through both directions and projection occurs only after the tensor evaluation. Active Cartesian control-net hulls retain positive-weight convexity. The result bounds the exact real surface defined by accepted binary64 data, not every rounded evaluator output.

A shared `geometry_bounds::Bounds3` now holds curve/surface boxes; the existing public `nurbs::CurveBounds` name remains a compatibility re-export. No second interval or derivative engine is introduced.

Patch count and aggregate two-bound interpolation work are charged before each patch against explicit limits and the existing constructor work cap. Two-bound work is a structural budget, not a CPU instruction/time estimate. Nonfinite/outside/reversed domains and exhausted budgets return no partial enclosure. Zero-area domains and C0 endpoint-neighbour patches remain covered; no seam/parameter snapping or tolerance increase occurs.

`Basis::span_range` uses binary searches to locate intersected knot spans, including endpoint-only neighbours. Curve enclosure now shares this path. Local distance/sampling queries no longer linearly scan unrelated knots before each recurrence. The mathematical interval and budget semantics remain unchanged; no representative speedup is claimed without a benchmark.

Three new test groups cover independent plane/cylinder/sphere references, pole/corner domains, constant nets with unequal weights, invalid domains, patch/interpolation-work refusals and all four patches meeting a two-direction C0 knot. Existing curve enclosures/sampling/distance/Dual tests remain required.

An initial test incorrectly required every rounded evaluator output to lie inside the exact-real geometric box: a cylinder evaluation returned `z=3.0000000000000004` while the exact upper boundary is `z=3`. The initial run failed (48 pass / 1 fail); a subsequent focused run reproduced the same failure. The test now uses an independent analytic reference, consistent with the stated exact-real contract. No enclosure expansion or numerical acceptance change was made. Logs preserve the failure and final corrected run.

This is not a surface regularity/injectivity, trimmed-domain or unique-projection certificate. Full spline-backed B-Rep import remains refused. Next: bounded surface distance/projection candidates and source-bound shared-edge endpoint/locus checks, followed by p-curves/trim validation and body admission. General CAD/assemblies, PLC recovery/refinement/cavities/regions, sliver/conditioning and P2/P3/final gates remain open.

No new dependency, hosted CI, production backend switch, main change or OCCT retirement. [Source-specific verification](issue-46-phase-30-verification.json).

Final checks at `d3e81dcda5ab418753a4e1779980294a9c00ac2f`: **121 CAD-enabled / 104 feature-off tests and CAD-enabled app lib/tests compilation pass**, zero final failures/ignored tests. Initial reference-test failures remain recorded separately.
