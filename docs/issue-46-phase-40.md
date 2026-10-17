# Issue 46 — Phase 40: finite plane/line p-curve locus agreement

Source `b35c27c` adds `PlaneParameterMap::verify_line_locus`. Exact-document body face/edge/p-curve references must agree; the edge must bound the face and the p-curve must be associated through its source surface-curve wrapper on that source PLANE. Both primitive 3D LINE and parameter LINE representations are required; other geometry classes refuse explicitly.

Original source direction ratios, placement/length intervals and source vertex coordinates define the proof. Source vertices are projected onto each exact line carrier. Upper bounds independently cover source-to-3D, source-to-p-curve and 3D-line-to-p-curve distances at the finite edge endpoints. Distance to a line is convex along a segment, so the endpoint maxima bound the complete finite 3D segment. This does not claim agreement between two entire infinite lines. Source 3D extent/sense must also be certified.

The report retains face/edge/p-curve/wrapper identities, three pairs of physical-mm error bounds, the natural-direction dot interval and declared master/reference. `checked_pcurve_is_master` is an identity comparison only; it is **not** complete master validation. Other associations, natural parameter rates/offsets, curved/periodic loci and general trims remain unqualified. The report is a standalone scoped result, not an automatic promotion of the native body's full STEP-validation status or a solver verification token.

Actual-bracket tests certify **148 finite plane/line associations at unchanged 1e-7 mm tolerance**. Fresh metre-normalized context data also passes without changing physical tolerance. A displaced p-curve, contradictory source edge sense and stale contexts/references refuse. The three earlier plane-map groups remain passing; no assertion or resource limit was relaxed. Targeted six-test runner time was 0.03 s, not a standalone performance/peak-memory benchmark.

[Combined source verification](issue-46-phase-40-verification.json) covers Phases 39–40 and existing curve/surface/STEP/solver bridges. No new dependency, hosted workflow dispatch, production native strategy or OCCT retirement. Next: source cylindrical parameter semantics and remaining primitive/curved p-curve loci, then whole-model/master integration with aggregate budgets, general trim/body gates and the remaining meshing roadmap.

Final combined source checks: **145 CAD-enabled / 128 feature-off tests and application lib/tests compilation pass**, zero failures/ignored tests.
