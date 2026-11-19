# Phase 182: NIST import 29 to 32 of 33 (tangent-face heals, tessellated solids)

Date 2026-11-19. Same local NIST PMI corpus as Phases 175-181 (not committed; public domain, nist.gov/document/nist-pmi-step-files). Production path (`import_step_native`).

## Changes

1. `solid-kernel/src/analytic_step/heal.rs`: a stalled compromise between two tangent faces with a gap is a heal when the worst distance is at most `max(geometry_tolerance, heal_limit)` (it was the geometry tolerance only). Test `a_compromise_between_tangent_faces_within_the_heal_limit_is_a_heal`. Admits ctc_02 rc. Strict reads (`heal_limit` 0) are unchanged.
2. `solid-kernel/src/cylindrical_step.rs`: the chart canonicalization allowance is `max(geometry_tolerance, heal_limit)`; the seam closure/width round-off bound scales with chart magnitude and, on a foreign-CAD read, is at least 1e-9 mm (healed vertices lie on their surfaces to 1e-10 relative). Admits ctc_05 e1. Corpus evidence only.
3. `fea-problem/src/native_tessellated.rs` (new): reads a STEP `TESSELLATED_SOLID` (`COMPLEX_TRIANGULATED_FACE` strips and fans, `TRIANGULATED_FACE`) into the face-tagged cache, welds shared positions at 1e-5 mm and flips inward shells. Hooked from `import_step_native`. Admits ftc_08 e1-tg, which OCCT also reads as a solid. Two unit tests.
4. `solid-kernel/src/planar_step.rs`: the "self-intersects or touches" error names bound, segments and points (diagnostic only).
5. `fea-problem/tests/native_import_corpus.rs`: `MESH_TETS=<n>` meshes at the size whose estimate V/(0.1179 h^3) is `n`.

## Evidence

- Native import: **32 of 33** (29 before; log `.audit/nist-import-after.log`). `cargo test --release -p solid-kernel -p fea-problem --locked`: all pass.
- The 27 "element estimate" refusals of imported files are not a native gap: OCCT refuses 32 of 35 corpus files at the default size too (Phase 165). Native meshes every file OCCT meshes.
- Same-size native vs OCCT 8.0.1 (`occt_vs_native`, no solve): ctc_01 rd OCCT 76,028 tets, 25 cells under the 0.1 gate, worst scaled Jacobian 0.025; native 76,111 / 19 / 0.042. ctc_05 rd OCCT 62,509 / 105 / 0.022; native 63,332 / 84 / 0.036. ctc_03 rc: both refuse ("more than 2000000 tetrahedra"). No native-only failure. Both fail the 0.1 gate at these sizes (boundary slivers), so that is a shared limit. ctc_04 rd and ftc_06 rd OCCT sides were not finished (10 to 30 min per ~70k tets on both paths).

## Still refused (1 of 33)

`nist_ctc_02_asme1_ap203.stp`: face #1167 is a B-spline surface closed in v; its trim polygon (bound #1705) crosses the v seam with a straight jump (segment 154, v 1.6 to 668.1), so the sampled polygon "self-intersects or touches". Needs periodic-seam chart unwrapping for closed spline surfaces.

### Tried and dropped: unwrapping the closed spline surface's chart across its seam

Probe: bound #1705's chart points step by about 2 mm in v (1.636 at segment 154), then the next sample is at 668.1: the surface is closed in v (period 666.5 mm of chart) and `SplineSurface::chart_loop` clamps every point into the parameter domain, so a loop leaving the domain at v = 0 re-enters at v = 668. A trial (periodic flag detected from coincident domain ends, `continue_from` unwrapping by whole periods, point evaluation wrapped) removed that jump, and the read then refused with a self-intersection between the loop's last segment (v = -314.6) and its first point (v = +351.9): **the loop winds a full period**. The face is a band closed in v (like a cylinder side), not a patch crossing a seam, so a single unwrapped polygon cannot represent it. It needs a seam-cut band chart (two winding loops joined by seam edges, seam vertices stitched), the way cylinder bands are handled. That is a larger change than this phase, and ctc_02 ap203 is a different revision of a model whose other revisions (rc) import. The trial was reverted; ap203 stays a documented refusal. OCCT reads it (Phase 165 table), so native is 1 file short of full parity on this corpus.
