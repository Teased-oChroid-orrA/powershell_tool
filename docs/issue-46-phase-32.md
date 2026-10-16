# Issue 46 — Phase 32: source-bound spline edge sampling

Source `ed9757d` adds immutable `SampledSplineEdge` results, with subsequent shared-declaration reuse/diagnostic corrections qualified at the final source recorded below. The source body/edge/document context selects the spline and declared mm factor. Natural-parameter samples, topological edge direction, VERTEX_POINT references, control declarations, length context, normalized source endpoints and endpoint-distance upper bounds remain distinct and inspectable. No endpoint snapping or inferred trim occurs.

The complete clamped spline endpoints must agree with their corresponding source vertices under the existing geometry tolerance (default 1e-7 mm). Interval endpoint bounds include real-curve versus binary64 evaluation uncertainty. Sampling retains the existing whole-interval Hausdorff bound, progress, cancellation and resource limits; a final commit callback can still cancel atomically. Errors preserve actual point offsets or add the source edge offset to runtime diagnostics.

Three source integration groups use the real bracket with an endpoint-matching linear spline replacement. They verify exact-document/source/order metadata, mm/metre scaling without knot changes, both stale-reference directions, reversed valid sense, wrong orientation, inconsistent controls, unsupported interior trims, cancellation and invalid/work-limited requests. A fourth group added in Phase 33 covers body admission. This stage alone does not certify curve simplicity, p-curves, surface loci or a B-Rep.

Targeted tests passed at this increment. One combined full regression gate validates Phases 32–35 together, avoiding redundant complete suite rebuilds between closely related integration changes. See [verification](issue-46-phase-32-verification.json). OCCT stays production; general curved/piecewise spline-body and trim gates remain open.

Combined qualification at `2042c2d136829591f03cc7f4ad2d3e8c6198777a`: **130 CAD-enabled / 113 feature-off tests and CAD-enabled app lib/tests compilation pass**, zero final failures/ignored tests.
