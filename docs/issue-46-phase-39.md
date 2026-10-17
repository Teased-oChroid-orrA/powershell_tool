# Issue 46 — Phase 39: source-plane parameter maps

Source `fba2f89` adds `PlaneParameterMap` for actual STEP PLANE entities selected through the exact source body/face context. It never treats a cylinder or geometrically reduced spline face as an affine source-UV plane. Both plane parameters are source lengths and scale once by the bound body mm factor; origin placement, raw direction ratios, orthogonalization and frame orientation use outward interval arithmetic.

The immutable map retains face, basis-surface and length-context references and reports world-coordinate boxes rather than pretending rounded points are exact. Stale contexts/identities, parallel or unresolved axes, invalid rectangles and geometry-range/overflow uncertainty refuse. This is a forward source-plane transform, not inverse UV or trim/master validation.

Interval division now divides interval endpoints directly. Reciprocal-first evaluation overflowed for a nonzero denominator such as 1e-310 even when the quotient 1e-310/1e-310 is representable. Direct outward-rounded quotients keep that ratio bounded without weakening zero-denominator/overflow handling. This supports actual tiny direction ratios without an unbounded intermediate reciprocal. Existing interval and native geometry tests remain required.

Three integration groups cover all original bracket plane frames, independent axis-aligned map coordinates and rectangles, mm/metre conversion, 1e-310 direction ratios, stale identities, non-plane rejection, parallel axes and unbounded/invalid queries. An initial filtered command ran one interval unit and zero integration tests; it was corrected to run all 60 library tests plus all three integration tests. The zero-test run is not counted as validation. Environment restart lost the initial build process ID; the restored working tree retained all source changes and checks were rerun.

The combined Phases 39–40 gate is recorded in [verification](issue-46-phase-39-verification.json), avoiding duplicate whole-suite qualification between the transform and its immediate locus-check consumer. No dependency, tolerance relaxation, production switch, hosted workflow dispatch or OCCT retirement.

Final combined source checks: **145 CAD-enabled / 128 feature-off tests and application lib/tests compilation pass**, zero failures/ignored tests.
