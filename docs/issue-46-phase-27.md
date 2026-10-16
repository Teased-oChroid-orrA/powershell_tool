# Issue 46 — Phase 27: source-bound spline curve declarations

Source `69ae967` adds `BodyContext::read_spline_curve`: exact-document edge references resolve before numeric body lookup; the terminal spline declaration uses the body's declared length factor and retains the actual curve source reference. Non-edge and stale context/edge references refuse.

The existing analytic importer and this reader now share one bounded 3D SURFACE_CURVE/SEAM_CURVE wrapper traversal, preserving the 32-entity/cycle refusal and analytic geometry errors. Complex terminal declarations remain available to the spline declaration reader while the strict analytic body reader still rejects unsupported spline geometry. Associated p-curves are not certified by this helper.

The existing endpoint-matching spline replacement on the actual bracket now tests source-bound extraction, retained curve identity, declared mm/metre factors, authoritative endpoint scaling and unchanged knot domains. Both stale-reference directions, non-edge references and a cyclic wrapper refuse. Full native analytic body import still refuses the same exact unsupported curve.

No dependency, tolerance, production backend, main or hosted CI change. Bounded spline sampling/projection, edge/surface locus and trim validation are still required before full spline-body support. General CAD/assemblies, PLC recovery/quality and all P2/P3/final gates remain open. Source-specific final evidence is recorded in [verification](issue-46-phase-27-verification.json).

Final checks at `69ae967c2a3b8fcb271f24c38167fed6d5fd5782`: **106 CAD-enabled / 89 feature-off tests and CAD-enabled app lib/tests compilation pass**, zero failures/ignored checks.
