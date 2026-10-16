# Issue 46 — Phase 35: structured spline cancellation

Source `d11d77a` corrects cancellation classification across spline sampling, the shared curve/surface distance engine and source-bound spline-edge wrappers. Callback cancellation now returns the existing `ErrorKind::Cancelled`; invalid geometry remains `Geometry` and exhausted budgets remain `Limit`. Source offsets, messages and final cancellation boundaries remain intact. No unconverged/partial result is returned.

New discriminant assertions reproduced `Geometry` instead of `Cancelled` in two unit tests (two other filtered tests passed, exit 101) and the source-edge integration test (exit 101). All affected paths were corrected; the surface distance API shares the corrected engine. These expected pre-fix failures are recorded separately from final qualification. Existing cancellation-message and atomic-result checks remain, rather than being replaced by type-only checks.

[Final combined verification](issue-46-phase-35-verification.json) covers all Phases 32–35 at the latest source, including the Phase 34 diagnostic compatibility correction. No dependency, tolerance/resource-limit increase, production backend switch, hosted workflow dispatch or OCCT retirement. Work continues through native surface/trim and broader meshing/roadmap gates.

Combined qualification at `2042c2d136829591f03cc7f4ad2d3e8c6198777a`: **130 CAD-enabled / 113 feature-off tests and CAD-enabled app lib/tests compilation pass**, zero final failures/ignored tests.
