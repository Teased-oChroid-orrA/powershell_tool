# Phase 155: align nearby volume-mesh vertices with CAD face seams

Issue #63, with NEMA17 deferred by owner request. The 1 mm filleted-box load-face area failed on OCCT tessellation because 56 top-tagged boundary triangles crossed the flat/fillet seam. Their 28 curved-side vertices lay about 0.032 mm from the seam, and whole-triangle tagging gave the flat top 80.42372283704822 mm² instead of its exact 80 mm². The native tessellation gave 80.106664382864 mm².

## Choice and guard

A localized output-mesh probe moved those 28 OCCT vertices to the exact seam and found 80.0000 mm² top area with no new *corner* scaled-Jacobian failure. That probe did not establish curved Tet10 validity or traction integration, because the already-built midside nodes were not updated. A boundary-diagonal flip looked possible from the surface triangles, but none of the 28 crossing pairs belonged to one tetrahedron; the existing single-tet peel cannot perform it. The surface-constrained PLC prototype also remained below the solver quality gate ([Phase 153](issue-46-phase-153.md)).

The production mesher now aligns only a vertex on a straddling boundary triangle whose trimmed-face foot lies within 0.05 local edge lengths, is on both source and target CAD faces within the surface accuracy, and is on the solid. The move occurs after boundary repair and before Tet10 midside construction. It is kept only when incident boundary triangles preserve their prior face ownership and orientation, at least one crossing becomes conforming, and no incident tetrahedron falls through the existing scaled-Jacobian gate. The final snap counts and repair quality/volume fields are recomputed after this pass.

## Local evidence

The focused 1 mm top-face area is now 80.000000000000 mm² through OCCT and 80.045837112095 mm² through native tessellation. The existing three-size fillet regression passes on both paths with a tightened 0.1% area limit at 1 mm. The 1 mm loaded traction test passes on both paths; it exercises Tet10 midside construction and the solve. The filleted cross-bore still has zero sub-gate cells on both paths (minimum scaled Jacobian 0.105 OCCT and 0.103 native), and the t-junction repair regression passes. The 18-model, 36-row mesh-only comparison, with NEMA17 temporarily omitted from the diagnostic table, had 36 rows meeting the unchanged quality gates. The temporary table filter was removed. The table did not run static solves.

The focused `fea-core` snap unit tests passed (10), and `tetmesh_repair` passed (4, with 2 ignored). The final `fillet_rim_tagging` file passed on both native and OCCT paths (6 active tests each, with 1 ignored). The full touched-crate suites were not run, following the owner's localized-first test instruction.

## Hosted qualification and limits

PR #193 for Phase 154 failed hosted qualification on macOS and Ubuntu at the known 1 mm area assertion (80.42372283704822 mm²); Windows did not run that OCCT assertion. All three jobs also reported that the source changed during qualification. The qualification script now prints before and after Git status on that condition so the generated path can be identified in the next run. It still fingerprints untracked files, including new source files. The cause of the hosted status change has not yet been confirmed.

This pass repairs nearby crossings that can move onto a proven shared seam while holding local quality. It does not prove that every CAD seam is represented by the volume mesh; a crossing farther from a seam still needs conforming topology work. The ABC chart and small-angle refusals, Windows native-only mesh and solve, general native/OCCT parity, and the separately approved OCCT retirement remain open. `occt_retirement_approved` stays false.
