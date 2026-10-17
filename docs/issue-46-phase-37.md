# Issue 46 — Phase 37: explicit finite active-domain spline evaluation

Source `6c96b31` addresses a concrete p-curve foundation gap in the root bracket: complex curve #237, associated with PCURVE #235, is a closed quadratic with extended, unclamped knots. The existing strictly clamped constructor correctly refused it. A read-only source audit finds 228 p-curves (200 line, 13 circle, 15 complex spline geometries); this is motivation for parameter-space decoding, not proof of their master-representation agreement.

`NurbsCurve::new_active_domain` explicitly accepts a complete extended knot/control representation and evaluates only `[knots[degree], knots[control_count]]`. It does not synthesize controls, wrap parameters periodically, infer closure/continuity across a seam, or expand existing acceptance/resource limits. Clamped construction and all existing STEP/body defaults stay strict. Shared basis validation and de Boor/interval/Dual arithmetic are reused.

Clamped curves retain exact endpoint controls. Extended/unclamped curves evaluate the actual basis at active-domain endpoints; these need not equal their first/last control points. Endpoint partials remain one-sided from inside the active domain. Sampling and global curve distance now partition only active knot spans, excluding extended inactive tails; outside-active parameters refuse even when they lie inside the complete raw knot extent.

Three regressions compare a uniform extended quadratic against independent polynomial values/tangents, check actual non-control endpoints and clamped-default refusal, exercise active-domain enclosure/sampling/distance and discontinuity refusals, and evaluate the actual bracket #237 parameter curve from its original 2D control data (embedded with z=0, without mm conversion). Its endpoint agreement is checked at 1e-9 in parameter coordinates; no periodic wrapping/regularity or source p-curve agreement is claimed.

The actual-file test decodes fixture controls using existing scalar/knot helpers; a production source p-curve reader is still required, with explicit two-dimensional representation contexts and no guessed length scaling. Native body support remains the earlier verified linear-curve/convex-planar-face subset. General extended/periodic spline bodies, inverse UV and p-curve/master semantics remain gated.

No dependency, tolerance/resource-budget relaxation, production backend switch, hosted workflow dispatch or OCCT retirement. [Verification](issue-46-phase-37-verification.json) records full source-specific tests/application compilation. Next: share bounded spline declaration decoding with a parameter-space point decoder; parse p-curve contexts and preserve source/geometry/representation identities, then verify mappings against 3D loci rather than treating declaration extraction as CAD success.

Final source checks: **135 CAD-enabled / 118 feature-off tests and application lib/tests compilation pass**, zero failures/ignored tests.
