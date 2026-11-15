# Phase 176: NIST import refusals, second-stage table; a looser edge/surface tolerance admits no file

Date 2026-11-15. Same corpus as Phase 175 (33 NIST PMI files, local copy, not committed). `fea-problem/tests/native_import_corpus.rs` now prints 420 characters of each refusal instead of 150, so the tolerant-read (second-stage) message is visible; before, it was cut off and only the first-stage class was known.

## Second-stage refusals (`Options::foreign_cad`, chord 0.05 mm, tolerance 5e-3 mm), 24 files

| Class (tolerant read) | Files |
|---|---|
| edge/surface or spline-edge/surface disagreement, 5.2e-3 to 1.9e-2 mm | ctc_01 rd/ap203, ctc_02 rc/ap203, ctc_05 ap203/ap242, ftc_07 rd/e2, ftc_11 e2, stc_10 (10) |
| invalid or unsupported geometric radius | ctc_01 e1, ctc_02 e2, ctc_04 e1, ftc_06 e2 (4) |
| edge endpoint not on its declared circle | ctc_05 rd |
| parallel or unresolved placement directions | ctc_04 rd, ftc_08 rc (2) |
| spherical face orientation and extent | ftc_06 rd, stc_06 (2) |
| boundary winds without a chart-closing seam | ctc_04 ap203, ftc_10 rb (2) |
| ambiguous half-turn cylindrical trim | ftc_10 e2 |
| one supported body required | ftc_08 e1-tg |
| unsupported representation item | stc_07 |

## Tried and dropped: tolerance up to the declared uncertainty

Some files declare a distance uncertainty of 0.01 mm (`nist_ctc_01` rd and AP242), so the idea was to admit edge/surface disagreement up to the file's own declaration instead of chord/10. Measured with `foreign_cad` at chord/4 (0.0125 mm, above any declared 0.01 mm): still **9 of 33**. Files only moved to the next refusal. Five of the ten (ctc_01 rd/ap203, ftc_07 rd, ctc_05 ap203, stc_10) now stop at "boundary winds without a chart-closing seam", joining ftc_10 rb and ctc_04 ap203 (7 in all); the other five stay over the tolerance (ctc_02 rc 1.9e-2, ftc_07 e2 2.7e-2, ftc_11 e2 1.4e-2, ctc_02 ap203 and ctc_05 e1 spline samples 1.3e-2 and 1.6e-2). The declared values are also inconsistent (1e-6 in `ftc_10_rb`, which disagrees by 3.4e-4). The tolerance is not the binding constraint; it was reverted and `Options::foreign_cad` is unchanged.

## What is binding

1. "boundary winds without a chart-closing seam": the largest class once the tolerance is out of the way (about 7 files).
2. "invalid or unsupported geometric radius" (4 AP242 files; likely the zero-radius cone class of PR 281, to be re-measured after it merges).
3. Placement directions (2), spherical orientation (2), half-turn trim (1), product structure (1), unsupported item (1).

## Fix: an unseamed cone tip gets a seam to its apex

Cause of "boundary winds without a chart-closing seam" in `ctc_04` AP203 and `ftc_10_rb`: a cone face (semi-angle 59 degrees, a drill point) bounded by one circle that winds once around the axis, the apex inside the face and no seam edge. The chart code reads an apex only on an explicit seam edge. (The other nine tolerance-class files that reach this message in the experiment above are different faces and are not fixed here; the experiment's count of seven mixed the two causes.)

`AnalyticModel::seam_cone_tips` (`solid-kernel/src/analytic_step.rs`, run with the other band-seam passes at read time) cuts such a face along a generator from the circle's first vertex to a new apex vertex, used once each way in one merged loop: the shape of `data/cone.step`. Only cone faces with exactly one bound that turns once (|turns - 1| < 1e-6) are touched; anything else is left to the chart code as before. The apex is `origin - z * radius / tan(semi_angle)`.

Proof: `unseamed_cone_tip_gets_a_seam_to_its_apex_and_samples_like_the_seamed_original` (the OpenCASCADE cone fixture with the seam removed from the loop) gives the same vertex and triangle counts and the same signed volume (1e-9 relative) as the seamed original, and 1.2 % below pi r^2 h / 3, the chord's sampling error at 0.05 mm on r = 5 mm. `cargo test -p solid-kernel` and `-p fea-problem --release` pass.

Measured (`import_step_native`, NIST corpus): **11 of 33**, up from 9: `nist_ctc_04_asme1_ap203` (405 faces, 7016 triangles, 0.6 s) and `nist_ftc_10_asme1_rb` (214 faces, 8470 triangles, 0.3 s). Import only: no OpenCASCADE comparison or solve (no prebuilt here), so volume parity for these two is not measured; retirement remains unapproved.
