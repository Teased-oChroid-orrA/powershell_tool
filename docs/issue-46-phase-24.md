# Issue 46 — Phase 24: verified local spline differentials

Source `9269808` reuses fea-core's existing Dual/Scalar arithmetic in the shared homogeneous de Boor evaluator. Curve tangents and tensor-surface partials are derivatives with respect to the original knot parameters. This adds no global CAD-edit, mesh or solver sensitivity capability and no second AD engine.

Endpoints report one-sided derivatives from inside the domain; exact internal C0 knots refuse differential evaluation. Authoritative endpoint/corner coordinates remain unchanged. Surface differential work charges three lanes against the existing evaluation budget. Computed normals use stable direction normalization; zero/parallel partials and sphere poles refuse normals, although pole point evaluation remains valid. Computed directions are not regularity certificates.

Eight differential regressions check independent analytic rational tangents, planes/cylinders/spheres, endpoint sides, knot/length/common-weight transformations, existing Taylor verification (order >1.9), C0 refusal, singular normals and unchanged work limits. Existing curve/surface value tests remain passing.

An additional verified normalization defect was reproduced: `[1e-300,1e300,0]` silently lost its nonzero first component. The expected-error regression failed before the fix. The shared native direction helper now refuses nonzero component underflow; ordinary vectors at extreme uniform scales remain supported. Native analytic frames reuse this helper without changing tolerances.

Final checks at source `92698088cd1300a17fafee37d62da885486ae5e0`: **104 CAD-enabled tests, 87 feature-off tests, and CAD-enabled application lib/tests compilation pass**, with zero final failures/ignored tests. The pre-fix reproduction is recorded separately. [Verification](issue-46-phase-24-verification.json).

No dependency, production backend, main, hosted CI or OCCT retirement change. Full spline-backed B-Rep import remains explicitly refused pending context/placement binding, bounded sampling/projection and trim/locus validation. General CAD/assemblies, PLC recovery, quality and P2/P3/final gates remain open. Next: share the strict single-body representation/unit-context binding before adding more native geometry integration.
