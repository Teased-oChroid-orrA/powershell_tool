# Phase 179: NIST import 22 to 27 of 33 (shallow chord inversion, wireframe items, cone wedge branch at the apex)

Date 2026-11-16. Same local NIST PMI corpus as Phases 175-178 (not committed). Import only: no OpenCASCADE here, so no parity or solve is measured; `occt_retirement_approved` is untouched.

## What changed

1. **Shallow chord-triangle inversion** (`native_surface.rs`, `AnalyticModel::heal_limit_mm()` in `analytic_step.rs`). A chord triangle beside a curved boundary leaves the surface by up to the chord error, so a sliver whose vertex is nearer the boundary than the chord's sagitta is inverted in space by less than that. A foreign-CAD read (`heal_limit_mm` = the chord) accepts a wrongly signed triangle whose height over its longest edge is at most the heal limit; a strict read (limit 0) accepts none. Admits ctc_05 ap203, ftc_11 e2 and stc_07.
2. **Wireframe and fill items beside the body** (`step_body.rs`). `COMPOSITE_CURVE`, `OPEN_SHELL` and `SHELL_BASED_SURFACE_MODEL` listed beside the solid in its representation enclose no volume and are ignored, like a `GEOMETRIC_CURVE_SET`. `GEOMETRIC_SET` (may hold surfaces) is still refused. stc_07.
3. **Branch after a cone's apex** (`curved_surface.rs`, `periodic_trims_toward`). A boundary through the apex has no angle there; the angle after it was continued from the one before by the nearest branch, so a turn of about half a period was taken the short way even when only the other way closes the loop. The face's chart was then folded: its polar centre fell on the wrong side, `point_curved` cut through the face, and the chord-rule refinement split forever (337k edges, "chart region traversal budget exceeded"). Now, when the boundary turns at the apex and the angle does not return to its start, everything after the last apex is shifted by whole periods until it does. A full cone's seam (out and back along one line: no turn) keeps the waived closure test. Admits ftc_10 e2 and ftc_07 e2.

## Finding: why the tie rules were not enough

Faces #1826 and #4156 of ftc_10 e2 are halves of a cone (180 degrees, a rim arc, two generators). The turn at the apex is half a period to rounding. `unwrap_period_tie` only breaks an exact tie; with the turn a few ulp under half, both tries (`+1`, `-1`) took the same wrong branch and the unclosed result was accepted because the closure test is waived at an apex.

## Proof

- Corpus: **27 of 33** import (was 22 after Phase 178). Volumes against sibling files: ftc_10 e2 188 938.172 against ftc_10 rb 188 937.836 (1.8e-6); ftc_07 e2 1 726 974.6 and stc_07 1 726 960.2 against ftc_07 rd 1 726 641.9 (1.9e-4); ftc_11 e2 5090.680 against ftc_11 rb 5090.766 (1.7e-5). ctc_05 ap203 (12 670 501.7) has no imported sibling.
- Tests: `a_cone_wedge_wider_than_half_a_turn_samples_its_closed_form_volume` (wedges of 180 to 350 degrees, three loop starts, 24 attitudes, two semi-angles, closed-form volume within 2 %); `a_solid_listed_by_several_representations_of_one_context_or_beside_a_curve_set_binds_unambiguously` extended with composite curve, open shell and surface model.
- **Limit of the tests:** the synthetic wedges read correctly with or without the apex fix; I could not reproduce the NIST rounding with generated geometry. The apex fix is evidenced by the corpus (ftc_10 e2 and ftc_07 e2 refused before, import after, volumes above), not by a synthetic test that fails without it. The shallow-inversion rule has no synthetic test either: a sliver needs a real curved face meeting a chord; its evidence is the three corpus files, and the strict read's behaviour is unchanged (limit 0, so `shallow` is false).

## Still refused (6 of 33)

| File | Reason |
|---|---|
| ctc_02 rc, ctc_02 ap203, ctc_05 e1 | tangent spline faces ~1e-2 mm apart, over the 5e-3 geometry tolerance after healing (Phase 177/178). Widening is an owner decision. |
| ctc_02 e2 | "curved chord midpoint is unresolved" on spline face #2893: the chart midpoint of an interior chord coincides with an existing vertex (the insert adds nothing). Traced to a spline (`Param`) face, not a cone; root cause not found. |
| ctc_05 rd | a vertex 1.8e-2 mm off its declared circle (over the heal limit); not investigated further. |
| ftc_08 e1-tg | tessellated only, no B-rep solid. Not importable by design. |
