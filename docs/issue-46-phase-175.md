# Phase 175: the whole-body surface budget stopped real parts at 40 faces

Date 2026-11-15. Native STEP import over the licensed NIST PMI corpus (33 files, `NIST-PMI-STEP-Files.zip` from nist.gov, 13,976,599 bytes, downloaded with owner approval, not committed; NIST allows use without restriction).

## Finding

`AnalyticModel::sampled_surface` refused up front when `faces * face_work > total_work`. At the defaults (2,000,000 and 80,000,000) that is any body of more than 40 faces, whatever the faces need: it charged every face its worst case. 20 of the 33 files stopped there ("surface chart work budget exceeded"). The tests and the ABC harness had been raising `total_work` to 800,000,000 to get around it.

## Change

`solid-kernel/src/native_surface.rs`: each face keeps its `face_work` cap; the body is now charged what its charts produce (vertices + triangles, summed as faces are sampled) against `total_work`, and refuses with the same message when that sum passes it. The vertex and triangle budgets are unchanged.

Tests: `solid-kernel/tests/native_brep.rs` (a 124-face comb prism samples at the default limits with the closed-form volume; a `total_work` below one chart still refuses). `fea-problem/tests/native_import_corpus.rs` is an ignored evidence test over `STEP_DIR`.

## Measured (macOS arm64, release, `import_step_native`, the production path)

Native imports **9 of 33** files (before: 1, `nist_ftc_11_asme1_rb`). Newly importing: `ctc_03` (rc, AP203, AP242), `ftc_08_ap242-e2`, `ftc_09` (rd, AP242), `stc_08`, `stc_09`; each in under 0.6 s, 120 to 270 faces.

The other 24 refuse on classes that were hidden behind the budget (first stage error each):

| Class | Files |
|---|---|
| edge/surface disagreement or spline edge vertex off its curve (3.4e-4 to 6e-3 mm) | ctc_01 (x3), ctc_02 ap203/rc, ctc_05 ap203/ap242, ftc_07 (x2), ftc_10 rb, ftc_11 e2 |
| edge endpoint not on its declared circle/line | ctc_02 e2, ctc_05 rd, ftc_06 e2, ftc_10 e2, stc_10 |
| parallel or unresolved placement directions | ctc_04 rd, ftc_08 rc |
| invalid or unsupported geometric radius | ctc_04 e1 |
| spherical face orientation and extent | ftc_06 rd, stc_06 |
| boundary winds without a chart-closing seam / ambiguous half-turn trim (tolerant read) | ctc_04 ap203, and the second stage of several above |
| one supported body required (product structure) | ftc_08 e1-tg |
| unsupported representation item | stc_07 |

## Tried and dropped

Reading the extent at a coarser chord (0.5 mm) before the final read, to loosen the 5e-3 mm first-read tolerance: it admitted no additional file (still 9 of 33) and only changed which refusal is reported, so it was reverted.

## Limits

Import only: none of the 9 was meshed or solved against OpenCASCADE here (no OCCT prebuilt on this machine), so the retirement gate's mesh and parity criteria are not re-measured. Retirement remains unapproved.
