# Issue 46 — Phase 44: cylindrical finite-line and circular-carrier agreement

PR #71 merged at `3e06cf1`, old branch deleted. Continuation: `feature/native-cad-cylinder-line-agreement`.

Source-bound cylinder maps now verify exactly constant-raw-U generator LINE p-curves against finite source 3D lines and exactly constant-raw-V LINE p-curves against whole source 3D circle carriers. Original direction ratios decide constant coordinates; even a 1e-310 nonzero slope refuses instead of being rounded into a generator or circle. Physical units remain explicit. Generator U uses the existing bounded trigonometric map; circular carrier comparison is independent of starting angle and does not select finite arcs.

`step_curve_agreement` owns shared finite-line and unsigned whole-circle numerical proofs/reports. `step_pcurve` owns bounded exact-source edge/face/wrapper association and original direction ratio reads. Plane compatibility report aliases remain available. This removes duplicated numerical acceptance paths between surface types.

Actual bracket evidence: 28 cylinder generators and 24 cylinder circle carriers pass unchanged 1e-7 mm tolerance; all prior 148 plane lines and 13 plane circle carriers still pass. Displaced generators, axial circle shifts, nonconstant raw slopes, wrong carrier classes, contradictory line sense and stale documents refuse. These cover 213 analytic primitive associations; the remaining 15 spline declarations have enclosures, not locus/master certificates. No aggregate full-body promotion is inferred.

[Verification](issue-46-phase-44-verification.json): 158 CAD-enabled / 141 feature-off tests pass, zero failures/ignored; app CAD lib/tests compilation passes. One development extraction omitted a still-needed plane placement import; compiler E0425/E0308 were corrected before qualification. No tolerance/assertion/limit changes or hosted CI dispatch. OCCT remains production. Continue spline/carrier agreement, finite arc/master/trim/body gates and general constrained meshing/quality/P2/P3/final gates.
