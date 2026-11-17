# Phase 181: NIST import 28 to 29 of 33 (an arc end off its own circle is healed)

Date 2026-11-16. Same local NIST PMI corpus as Phases 175-180 (not committed). Import on the production path (`import_step_native`) only: no OpenCASCADE here, so no parity or solve is measured; `occt_retirement_approved` is untouched. `geometry_tolerance_mm` (chord / 10) and `heal_limit_mm` (chord) are unchanged.

## Root cause (ctc_05 rd)

Refusal: "edge endpoint is not on its declared circle (off by 1.82e-2 mm, radius 6.35)" at edge `#4444` (circle `#1016`, vertices `#2781`, `#2783`). Measured from the file (inch units): vertex `#2781` is on the circle to 1.4e-5 mm; vertex `#2783` is 7.17e-4 inch (1.82e-2 mm) out of the circle's plane and 2.5e-5 inch off its radius. The file's own stated closure uncertainty is 3.669e-3 inch (9.3e-2 mm), so the writer considers this connected.

The Phase 178 heal covers an edge whose curve disagrees with a *face* by more than the geometry tolerance and at most the heal limit. The check of an arc's ends against its *own circle* ran earlier (`analytic_step::samples`), at the geometry tolerance only, so this file never reached the heal: the same size of disagreement was healed in one place and refused in the other.

## Change (`solid-kernel/src/analytic_step.rs`)

An arc end off its circle by more than `geometry_tolerance_mm` and at most `heal_limit_mm` no longer refuses: the arc is sampled from the end's nearest circle angle, the vertex stays the end sample, and the edge is added to the heal set, so the vertex is moved onto every face meeting it and the edge's samples onto its two faces (unchanged `heal`). Beyond the heal limit it refuses as before. `Options::default()` has heal limit 0, so strict reads are unchanged. Lines and ellipses are not changed: no corpus file needs it.

## Proof

- Regression test `analytic_step::a_vertex_off_its_edge_circle_within_the_heal_limit_is_healed_onto_the_faces` (plate with a hole; the rim circle's vertex raised 0.02 mm out of the circle's plane): the strict read refuses with the circle message, a 0.06 mm offset is refused by the foreign-CAD read too, the 0.02 mm one reads with every rim sample on the top plane and the hole cylinder to 1e-7 mm, and the volume equals the unmodified file's to 1e-9 relative.
- Corpus (`native_import_corpus`, run with and without the change in the same build tree): **29 of 33** import (28 before). The other 28 give identical face and triangle counts, sizes and volumes. ctc_05 rd reads as 208 faces, 3,818 triangles, volume 1.26837e7 mm3 (closed, oriented, passes the import's own checks). ctc_05 ap203, a different revision with 156 faces, reads 1.26705e7 mm3 (1.0e-3 apart); it is not the same model, so that is a plausibility check, not parity.

## Still refused (4 of 33)

| File | Reason |
|---|---|
| ctc_02 ap203, ctc_02 rc, ctc_05 e1 | tangent spline faces about 1.0e-2 mm apart: the heal's compromise point is over the 5.0e-3 geometry tolerance. Chord / 9 was tried and does not admit them (below). |
| ftc_08 e1-tg | tessellated only, no B-rep solid; not importable by design. |

## Tried and dropped: geometry tolerance at chord / 9

The owner allowed a trial of `geometry_tolerance_mm = chord / 9` (5.56e-3 mm) in `Options::foreign_cad` on 2026-11-16. Full corpus, same harness: **still 29 of 33, no file gained.**

| File | At chord / 9 |
|---|---|
| ctc_02 rc | the first stalled sample (76, 5.21e-3 mm) passes; sample 91 of the same edge `#13622` stalls at 5.64e-3 mm |
| ctc_05 e1 | sample 7 of edge `#2769` stalls at 6.28e-3 mm (sample 6 was 5.46e-3) |
| ctc_02 ap203 | the heal completes; the read then refuses with "sampled trim polygon self-intersects or touches" |

The earlier figures were each file's first failing sample, not its worst. Admitting rc and e1 would need at least 6.3e-3 mm (chord / 7.9), with no evidence that this is the worst sample either, and ap203 fails at a later stage whatever the tolerance.

It also changes files that already import: 14 of the 29 read with different triangle counts and volumes (edges between 5.0e-3 and 5.56e-3 mm off a face are no longer healed onto it), by up to 3.4e-5 of the volume (ftc_10 e2: 188,938.17 to 188,944.54 mm3).

No gain and a changed baseline: not adopted, the tolerance stays at chord / 10. What these three files need is not a slightly wider tolerance but a treatment of tangent faces with a gap (ctc_02 rc, ctc_05 e1) and of the trim polygon that results once the heal passes (ctc_02 ap203).

