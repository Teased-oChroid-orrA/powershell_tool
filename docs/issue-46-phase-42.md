# Issue 46 — Phase 42: bounded source cylindrical parameters

PR #69 merged at `f0a72c979aeb440f23388ddd2c045c674c6e6c95`, old branch deleted. Continuation branch: `feature/native-cad-cylinder-parameter-bounds`.

`CylinderParameterMap` requires an actual source CYLINDRICAL_SURFACE bound to a body face and exact-document context. The assigned plane-angle unit must be a unique explicit unprefixed SI radian; missing, ambiguous, prefixed and other angular units refuse. Raw U is radians, raw V scales by the assigned length factor. The source radius and placement origin scale once. Shared outward-rounded `SourceFrame` replaces duplicated plane placement arithmetic; the existing planar line and spline tests continue to pass.

For U in [-8,8], fixed 32-term outward Taylor polynomials enclose sine/cosine without libm or approximate pi range reduction. Lagrange remainders use outward |U|^64/64! (cosine) and |U|^65/65! (sine); real derivatives have magnitude at most one. Results intersect the exact [-1,1] range. Wider U domains refuse; no seam snapping. Rectangle bounds contain every untrimmed cylindrical point; they do not select trims or prove edge/master/carrier agreement.

Tests cover every bracket cylinder, point and whole-rectangle probes, analytical rotated placement, tiny reference direction, physical metre conversion with unscaled angles, stale references, malformed/range queries, and unsupported angular units. Independent libm values are regression oracles only. Point enclosure widths stay below 1e-7 mm on the fixture without changing acceptance tolerances. A development fixture initially moved a surface independently of its edges and correctly failed full-body validation; the fixture now reads unchanged radius from the original body. The final checks retain all assertions.

[Verification](issue-46-phase-42-verification.json): 151 CAD-enabled / 134 feature-off tests, zero failures/ignored; app CAD lib/tests compilation passes. No hosted CI or new dependency. No new production-native capability or OCCT retirement. Next: curved p-curve/carrier agreement, whole-model/master/trim gates, full spline B-Rep, general constrained recovery/quality and later roadmap gates.

Tracking correction: GitHub epic #46 had been manually marked completed while child issues #47–64 remained open and broad gates unfulfilled. It was reopened to represent the current work accurately; no child gate was closed by these subset qualifications.
