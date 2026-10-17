# Issue 46 — Phase 41: whole-interval planar spline p-curve bounds

PR #68 merged into main at `43d8da95d12046c783ee5e29dad8ea25c82f9d37`; its branch was deleted. Continuation uses `feature/native-cad-spline-pcurve-bounds`.

`PlaneParameterMap::spline_pcurve_bounds` binds exact source identity and basis-surface identity, reuses the positive-weight outward de Boor enclosure, then maps its whole UV rectangle through the source plane into world millimetres. Knots and UV controls remain raw; only the plane map applies length conversion. Caller-supplied declaration and knot-span caps remain hard limits, with no partial result. Source errors retain p-curve offsets.

All 15 bracket spline p-curves pass eight interval checks each with independently evaluated containment probes. Budget exhaustion, invalid domains, stale source references and wrong basis surfaces refuse. A separate metre fixture verifies unchanged raw controls/knots and world scaling by 1000 exactly once. Sampling probes are regression evidence, not the mathematical enclosure certificate.

This is a projection/trim prerequisite, not full spline B-Rep import, master agreement, edge association or trim certification. The existing plane-line report remains separately scoped. No new dependency or production backend switch.

[Verification](issue-46-phase-41-verification.json): 147 CAD-enabled / 130 feature-off tests pass, zero failures/ignored; app CAD library/tests compilation passes. Workspace formatting check exposes existing unrelated differences; only changed Rust files were formatted. No hosted CI was dispatched. Continue source cylindrical parameters and curved p-curve agreement, whole-model/master/trim gates, general constrained meshing and remaining P2/P3/final gates.
