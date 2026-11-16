# Phase 177: NIST import 14 to 17 of 33: a mislabelled outer bound, and cone wedges through their apex

Date 2026-11-15. Same local NIST PMI corpus as Phases 175-176 (not committed). `fea-problem/tests/native_import_corpus.rs` now also prints each file's signed mesh volume, so two files that carry the same part can be compared.

## Fix 1: a single declared outer bound that is the hole (stc_06)

`nist_stc_06_asme1_ap242-e3` has a planar face (#412) whose only `FACE_OUTER_BOUND` is a circle of radius 10.3 mm and whose `FACE_BOUND` is the 11-edge loop around it: the labels are swapped. `AnalyticModel::sampled_planar_trims` refused it ("declared outer bound is geometrically nested") because it required a single declared outer to be the geometric one. The loop roles are already decided by the geometry alone (`planar_step::classify`: one enclosing loop, unnested holes, crossings refused), so the label check carried no information the geometry did not, and it refused a file whose geometry is consistent. The check is removed; a face whose loops do not nest into one outer and holes still refuses in `classify`.

Proof:
- `a_single_declared_outer_that_is_the_hole_reads_like_the_correct_labels` relabels the hole bounds of the OpenCASCADE plate-with-hole fixture (#262, #369) as the only `FACE_OUTER_BOUND`; the sampled triangle count and signed volume equal the correctly labelled file's exactly. It fails without the change with the corpus message.
- `stc_06` is the same part as `ftc_06 rd` (both 144 faces, size 0.466): native volume 3 322 034.96 against 3 321 974.57 mm^3 (1.8e-5 relative, a different file written by a different export), so the relabelled face is the right face.

## Fix 2: `seam_cone_tips` skipped loops that already contain the apex (ctc_01 e1, ftc_06 e2)

Phase 176's `seam_cone_tips` selects a cone face whose single bound turns once around the axis. A wedge of a cone (two generators from the apex and a half-circle arc, a drill point cut by a plane) has the apex as a vertex of its loop; the apex has no angle, so its turn count is meaningless and came out as one. The pass then added a second seam to the apex, the loop visited its first vertex and the apex twice, and `classify` refused it ("sampled trim polygon self-intersects or touches"). The three faces found (#937, #3150, #3379) are in `ctc_01` e1, `ctc_02` e2 and `ftc_06` e2. A loop with a vertex at the apex is now left alone, so the chart code reads its own apex and the half-turn arc as before.

Proof:
- `a_cone_wedge_through_its_apex_samples_its_closed_form_volume` (a hand-written half-cone solid: three faces, a zero-radius `CONICAL_SURFACE`) samples within 2 % of the exact pi r^2 h / 6 (the chord costs about 1.2 %). It fails without the guard with the corpus message.
- `ftc_06` e2 (the same part again) now imports at 3 321 854.14 mm^3, 3.6e-5 from `ftc_06 rd` and 5.4e-5 from `stc_06`.
- `ctc_01` e1 imports (117 faces, volume 14 663 770 mm^3); no second native file carries that part (its other versions are over the tolerance), so it has no differential check. `ctc_02` e2 is not admitted: it moves on to "curved chord midpoint is unresolved" (one file, not traced).

Tried and dropped: a roundoff-scale test for the half-turn tie in `unwrap_period_tie` (the 180 degree arc is `period / 2` only to rounding). It was not the cause (the extra seam was) and changed nothing, so it was not kept.

## Result

Native import reads **17 of 33** (was 14): added stc_06, ctc_01 e1, ftc_06 e2. Import only: no OpenCASCADE here, so no parity or solve is measured; `occt_retirement_approved` is untouched.

## What is left (16 files), and the tolerance finding

| Class | Files |
|---|---|
| edge/surface or spline-edge/surface disagreement over chord/10 (5e-3 mm) | ctc_01 rd 5.42e-3, ctc_01 ap203 5.42e-3, ftc_07 rd 5.17e-3, stc_10 5.43e-3, ctc_02 ap203 6.75e-3, ctc_05 ap203 7.73e-3, ctc_05 e1 6.65e-3, ftc_07 e2 5.87e-3, ftc_11 e2 8.2e-3, ctc_02 rc 1.9e-2, ctc_04 e1 4.8e-2, ctc_05 rd (endpoint off its circle, 1.8e-2) |
| curved chord midpoint unresolved | ctc_02 e2 |
| chart region traversal budget exceeded | ftc_10 e2 |
| product structure (needs exactly one supported body) | ftc_08 e1-tg |
| unsupported representation item | stc_07 |

Phase 176 recorded that a looser edge/surface tolerance admitted no file; that was true while the cone-tip and wedge faults were present and is no longer. Measured now with the tolerance at 0.0125 mm (chord/4, a temporary experiment, reverted): **21 of 33 import**, 4 more than now (ctc_01 rd, ctc_01 ap203, ftc_07 rd, stc_10), each needing only 5.17e-3 to 5.43e-3 mm, 3 to 9 % above the current chord/10. They agree with files already imported: ctc_01 rd/ap203 volume 14 661 029 mm^3 against 14 663 497 for ctc_01 e1 (1.7e-4, a second check of Fix 2), stc_10 188 877 against 188 937 for ftc_10_rb (3.2e-4). The next four (ctc_02 ap203 6.75e-3, ctc_05 e1 6.65e-3 and ap203 7.73e-3, ftc_07 e2 5.87e-3) stop at 1.3e-2 to 1.6e-2 on spline samples or at other faces. The declared uncertainties cannot justify a per-file value: `ftc_07 rd` declares 1e-6 mm and disagrees by 5.2e-3. Whether to move `Options::foreign_cad` (chord/10, ABC-corpus-calibrated, doc comment on the option) is an owner decision about the accepted geometric error (a whole-chord class of 22 ABC files stays out at any chord/4), so it is not changed here.
