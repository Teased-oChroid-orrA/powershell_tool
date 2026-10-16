# Issue 46 — Phase 34: declared line/surface locus validation

Source `4ee9b41` fixes a reproduced existing native analytic geometry bug. With default tolerance 1e-7 mm, vertices at plane offset 0.75e-7 could be individually within tolerance of a line at 1.5e-7 and of the plane at zero. The old surface check evaluated only those nearby vertices, incorrectly accepting the declared line outside surface tolerance. The new expected-error regression failed before the fix (exit 101).

The finite declared line locus between projected endpoint parameters now participates in the whole-locus surface check. Original endpoint/surface residual checks remain independently required, preventing a nearby line from hiding vertices outside surface tolerance. Plane and cylindrical-generator variants are verified, along with the inverse endpoint-outside/line-inside case. Circle-specific full-locus checks remain; no tolerance increase or world-point relocation occurs.

An initial combined qualification reached the existing malformed-radius test and failed its stable `disagrees` diagnostic assertion (28 tests passed / 1 failed in that suite). The stricter early endpoint check correctly rejected the model but used different wording. Follow-up `2042c2d` preserves the mismatch diagnostic family and reports the residual numerically; the existing assertion is unchanged. The failed run is preserved separately. [Final combined verification](issue-46-phase-34-verification.json).

Combined qualification at `2042c2d136829591f03cc7f4ad2d3e8c6198777a`: **130 CAD-enabled / 113 feature-off tests and CAD-enabled app lib/tests compilation pass**, zero final failures/ignored tests.
