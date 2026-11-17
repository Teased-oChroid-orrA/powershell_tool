# Phase 180: NIST import 27 to 28 of 33 (a chord spanning its own midpoint vertex)

Date 2026-11-16. Same local NIST PMI corpus as Phases 175-179 (not committed). Import and meshing on the production path (`import_step_native`, `Problem::from_step` + `solid_mesh`) only: no OpenCASCADE here, so no parity or solve is measured; `occt_retirement_approved` is untouched.

## Classification of the 6 files still refused after Phase 179

| Class | Files | Status |
|---|---|---|
| Tangent spline faces ~1.0e-2 mm apart: the heal's compromise point is 5.1e-3 to 5.5e-3 mm from each face, just over the 5e-3 geometry tolerance (chord / 10) | ctc_02 rc, ctc_02 ap203, ctc_05 e1 | Largest class (3). Fixing it means widening the tolerance, which Phases 176-179 left to the owner and this task forbids; unchanged. |
| "curved chord midpoint is unresolved" on spline face #2893 | ctc_02 e2 | **Fixed here.** |
| Vertex 1.8e-2 mm off its declared circle (over the heal limit) | ctc_05 rd | Unchanged. |
| Tessellated only, no B-rep solid | ftc_08 e1-tg | Not importable by design. |

## Root cause (ctc_02 e2)

`chart_mesh::shape_chart`'s chord rule splits a non-constraint chord whose surface sagitta exceeds the chord error, inserting the surface point at the chart midpoint. Traced by logging each insertion on face #2893 (a spline face; chart rectangle diagonal between boundary vertices 10 and 11, world chord 0.72 mm, sagitta 0.10 mm against 0.05): the midpoint of a chord whose ends are not dyadic in the chart is on the chord only to rounding. The spade triangulation, whose predicates are exact, then inserts it into a face beside the chord instead of splitting the chord, so the old chord stays and now spans the new vertex. The next batch met the same chord with the same midpoint (to 1e-16), the insertion returned the existing vertex and the face refused; before that the chain had inserted vertices at 1/3 and 2/3 of the diagonal over and over (vertices 173 to 180, coincident to 1e-15 in the chart).

Fix (`solid-kernel/src/chart_mesh.rs`): a candidate whose chart point holds a vertex to within 1e-12 of its magnitude (located through `cdt.locate`, comparing the vertices of the vertex / edge / face it falls on) is already split and is skipped; a batch that inserts nothing ends the pass. No tolerance or limit was loosened; every chord whose midpoint holds no vertex is split exactly as before.

## Proof

- Regression test `chart_mesh::tests::a_chord_spanning_its_own_midpoint_vertex_is_split_once` (torus face, chart rectangle 2.3317 x 5.2859, chord rule 0.5): without the change it fails with "curved chord midpoint is unresolved", with it the face triangulates and its triangles cover exactly the rectangle's chart area. The values were found by scanning 1,197 rectangle shapes: 795 failed without the change (unresolved midpoint or chart traversal budget).
- Corpus (`native_import_corpus`): **28 of 33** import (27 before). The other 27 files give identical triangle counts, sizes and volumes to Phase 179's run; ctc_02 e2 reads as 635 faces, 8,806 triangles, volume 4.7159e7 mm3 (closed, oriented, passes the import's own manifold, orientation and volume checks; no sibling file imports to compare with, and no OpenCASCADE here).
- Meshing on the production path (new `MESH=1` mode of the same harness, `solid_mesh` at the import's own size): **1 of 28** imported files meshes (649,241 tets, min scaled Jacobian 0.019, 27 cells under the 0.1 gate, 806 s); the other 27 stop at the element-estimate limit (14 million to 3.1 billion elements against 400,000). That is the thin-walled large-part default-size refusal the owner kept at the 2026-11-01 gate decision, so import is not the limit for the NIST files any more; their meshing needs an explicit size or the owner's reconsideration of that refusal. ctc_02 e2 is one of them (86,398,986 elements).

## Still refused (5 of 33)

The four rows above that are unchanged, 5 files: ctc_02 rc, ctc_02 ap203, ctc_05 e1 (tolerance, owner decision), ctc_05 rd, ftc_08 e1-tg (by design).
