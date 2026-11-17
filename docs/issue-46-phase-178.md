# Phase 178: native STEP import healing (NIST import 17 to 22 of 33)

Date 2026-11-15. Same local NIST PMI corpus as Phases 175-177 (not committed).

## What it is

`solid-kernel/src/analytic_step/heal.rs`. `Options::foreign_cad(chord)` sets `heal_limit_mm = chord` (default reads: 0, so unchanged). An edge whose declared curve disagrees with a face by more than `geometry_tolerance_mm` (chord/10) but no more than the heal limit is *healed* instead of refused: every sample of the edge and each of its vertices is moved onto the surfaces of all faces that use them (`heal_point`, Levenberg-Marquardt on the signed face distances; a vertex uses every face meeting it, and every edge ending there follows). Samples inserted later by `insert_edge_splits` are healed alike (`healed` map). A heal that would fold or collapse a segment, or move a point farther than the limit, refuses.

## Finding: the stalls were tangent faces with no common point

The first version stalled at 5e-6 to 5e-4 mm residual on six files. Per-face diagnostics showed normal cosines of +-1 and equal-and-opposite signed distances: B-spline faces that are tangent and separated by a micron-scale gap meet nowhere, so an exact-intersection heal cannot converge, and the unregularised step slid the point ~0.1 mm along the tangent faces. Not projection noise.

Fix (no tolerance or limit was loosened):
- `solve_damped` is a truncated eigen solve: directions where the faces' unit normals are within a few degrees of parallel (Gram eigenvalue < 1e-3 of the largest, `TANGENT_EIGENVALUE`) are not corrected.
- A stalled or creeping iteration (step removes < 0.1 % of the distance) is a heal only if every face is within `geometry_tolerance_mm`; otherwise it refuses with the per-face residuals and normal cosines.

## Proof

- Unit: `heal::tests::a_point_beside_a_plane_cylinder_intersection_is_healed_to_the_nearest_circle_point` (residual < 1e-9 on both faces, displacement equals the distance to the nearest circle point to 1e-9); `tangent_faces_with_a_gap_heal_to_the_midway_compromise` (parallel faces 10 mm apart are refused, not healed).
- Fixture: `a_hole_cylinder_off_its_edge_circles_is_healed_onto_the_cylinder_with_the_closed_form_volume` (hole radius 4.008 against edge circles at 4: strict read refuses; foreign-CAD read puts every sample on the cylinder to 1e-7 mm; volume within 2e-3 of the closed form). `a_strict_read_never_heals_and_a_conforming_file_reads_the_same_either_way`.
- Full `cargo test --release -p solid-kernel` and `-p fea-problem` pass. Three `on_surface` unit tests were repaired for the new `limit` argument.
- Corpus: **22 of 33** import (was 17 without heal, 21 with the first heal): ctc_01 rd/ap203, stc_10 and ctc_04 e1 as measured earlier (volumes within 1.5e-4 of sibling files), plus ftc_07 rd (volume 1 726 641.88).

## Still refused

| File | Reason |
|---|---|
| ctc_02 rc, ctc_02 ap203, ctc_05 e1 | tangent spline faces ~1e-2 mm apart: over the 5e-3 geometry tolerance after healing. Correct refusal; whether to widen is an owner decision (Phase 177). |
| ctc_05 ap203 | heals, then a degenerate/inverted boundary triangle (face #572) |
| ftc_07 e2, ftc_10 e2 | chart region traversal budget exceeded |
| ftc_11 e2 | refused before the heal too (edge/surface disagreement 8.2e-3 mm, Phase 177); the heal admits that edge and the file then stops at a degenerate/inverted triangle (face #1199), a later stage, not a regression |
| ctc_02 e2, ctc_05 rd, ftc_08 e1-tg, stc_07 | unchanged from Phase 177 |

ctc_05 e1 also showed a vertex 5.16e-2 mm from its common point, just over the 5e-2 limit: left refused. No OCCT parity or solve measured; `occt_retirement_approved` untouched.
