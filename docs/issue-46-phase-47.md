# Issue 46 — Phase 47: bounded single-winding quadratic circle traces

PR #74 merged at `a7c035c`, old branch deleted. Continuation: `feature/native-cad-single-winding-curves`.

The optional `require_single_winding` contract strengthens full circle coverage. It requires the existing exact closure, nonzero projected radius and strict consistent angular derivative, at most four active Bezier spans, and a certified open half-plane containing all homogeneous XY controls of each span. The half-plane direction uses the sum of the two unweighted endpoint vectors in the source circle frame; every control dot product must have a strictly positive lower interval bound.

Positive rational Bernstein blending then keeps the entire span in that open half-plane, bounding its monotone angular change strictly below pi. At most four spans bound total absolute angular change strictly below 4pi. Exact closed nonzero monotone coverage has a positive integer number of full turns, so the total must be exactly one turn. This uses no approximate pi, atan2 or sampled angle test.

Coverage alone continues to allow multiple windings and reports `single_winding=false`; it is not silently upgraded. The new stronger certificate conservatively refuses more than four spans even if a longer curve might have one winding. The whole-body diagnostic explicitly requests the stronger result. Finite arc selection, other trim intersections, complete master/body semantics and general spline import remain separate gates.

All 15 bracket spline associations pass the stronger check at unchanged 1e-7 mm. A constructed six-span double traversal has full circle coverage, passes one-sided/symmetric deviation, and is refused by both the stronger certificate and whole-body audit. Inconsistent option combinations refuse. All prior closure-gap, knot-layout, budget, cancellation and source tests remain passing.

[Verification](issue-46-phase-47-verification.json): 167 CAD-enabled / 150 feature-off tests, zero failures/ignored; app CAD library/tests compilation and real `--pcurve-audit` run pass (228 associations, 45 spline spans). No new dependency, development failure, hosted CI or production backend switch. Next: general source spline surface parameter/curve bounds and finite/master/trim integration, then general CAD/PLC/quality/P2/P3/final gates.
