# Issue 46 — Phase 43: unsigned planar circle carrier agreement

PR #70 merged at `8e7e4e5` and its branch was deleted. Continuation branch: `feature/native-cad-circular-carrier-validation`.

`PlaneParameterMap::verify_circle_carrier` validates exact source face/edge/p-curve associations through the shared bounded surface-curve wrapper lookup. It compares an actual 3D CIRCLE against a parameter CIRCLE on an actual PLANE. Original source placements and lengths use outward intervals, retaining declared master/reference identities.

The unsigned whole-circle Hausdorff bound is centre separation plus radius difference plus maximum radius times aligned unit-normal separation. The minimal rotation between aligned plane normals has operator norm 2 sin(theta/2), equal to their Euclidean separation; pairing rotated circle points bounds both directed distances. A normal sign is selected only with certified alignment. Opposite natural normals can describe the same unsigned carrier and remain visible in the report. Parameter phase/speed and finite arc selection are not inferred.

All 13 bracket planar circle associations pass unchanged 1e-7 mm tolerance. Incorrect centre/radius/3D normal, invalid tolerances and stale documents refuse. A reversed source normal retains carrier agreement and changes reported orientation. Shared association extraction preserves the prior 148 finite plane-line checks. A duplicated word in a placement diagnostic was corrected.

[Verification](issue-46-phase-43-verification.json): 154 CAD-enabled / 137 feature-off tests pass, zero failures/ignored; app CAD library/tests compilation passes. No hosted CI dispatched or new dependency. OCCT remains production. Full master/finite-arc/spline/trim/body, general constrained meshing/quality and P2/P3/final gates remain open.
