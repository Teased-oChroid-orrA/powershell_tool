# Phase 174: why the OCCT `screw.step` reaction rose with the crease stage

Issue #46. Follows [Phase 173](issue-46-phase-173.md), which added the concave-crease stage to `fea-core::meshing::boundary_repair` and left one result untraced: on `screw.step`, the OCCT path's benchmark reaction moved from 2.7 % to 3.3 % above the exact load (373.26) while the linear area of its loaded-face facets went down. This phase traces it. Diagnosis only: no production code changed. No gate, tolerance, budget, cap or timeout was changed. `occt_retirement_approved` stays false.

## Change

`fea-problem/tests/occt_vs_native.rs`, ignored diagnostic `loaded_face_tilted_facets`:

- Every facet named after the loaded face is now classified, not only the tilted ones. Flat facets whose corners all lie on the face are one line.
- Each class prints its curved area next to its flat (corner) area. The curved area is the load a unit traction along the benchmark axis puts on the class, computed with `fea_core::loads::assemble`, the same face integration the solve uses (Tri6 facets whose boundary midside nodes `tetmesh::curve_boundary_edges` has moved onto the surface).
- The face's totals end its list.

## Results (macOS arm64, release, `STEP_DIR` = `screw.step` only)

Stage off is `3bf42bc` (main before Phase 173) with this diagnostic copied in. Stage on is the Phase 173 change.

**The reaction is the traction times the curved area of the facets named after the loaded face.** The traction is 10, and on all four runs the curved total matches the Phase 173 reaction to every printed digit:

| Run | Facets | Flat area | Curved area | Reaction z (Phase 173) |
|---|---|---|---|---|
| OCCT off | 240 | 39.04705 | 38.33086 | -383.309 |
| OCCT on | 256 | 38.81164 | 38.55061 | -385.506 |
| native off | 236 | 37.77661 | 37.17338 | -371.734 |
| native on | 254 | 37.42792 | 37.34594 | -373.459 |

So the linear facet area the earlier phases compared is not what is loaded. The two differ by 0.2 to 0.7 mm² here, mostly in the bridge facets.

**What the crease stage changed** (curved area, off -> on):

| Class | OCCT | native |
|---|---|---|
| flat, every corner on face 3 | 31.831 -> 34.565 (+2.733) | 31.771 -> 34.550 (+2.779) |
| steps across the concave edge with face 2 (corners on faces [2, 3], [2], [3]; tilt 40 to 90 degrees) | 32 facets, 2.675 -> 2 facets, 0.165 (-2.510) | 34 facets, 2.606 -> 0 (-2.606) |
| a corner on no face (not within a hundredth of the size of any face) | 3.229 -> 3.226 (-0.004) | 2.227 -> 2.227 (0) |
| other | unchanged | unchanged |
| total | +0.220 | +0.173 |

The stage works the same way on both paths. The steps lie along the wall (most are tilted 80 to 90 degrees) but are named after the bottom by their centroid. Their curved area is only 0.85 (OCCT) and 0.83 (native) of their flat area, because their curved midsides pull them in. Meanwhile the flat facets of the bottom stopped short of the crease. The stage replaces the steps by flat facets that reach the crease. The bottom's named area grows by the strip the steps had left uncovered, net of the steps' area: +0.22 mm² on OCCT and +0.17 mm² on native.

On native this moved the reaction to the exact load (-0.41 % -> +0.05 %). On OCCT it moved away (+2.69 % -> +3.28 %), because OCCT was already above the exact load. Before the stage, the steps' deficit hid part of OCCT's surplus.

**Where OCCT's surplus is.** Stage on, OCCT's named curved area is 1.205 mm² above native's (38.551 against 37.346, exact 37.326). That difference is:

- +0.999 in facets with a corner on no face: OCCT has 24 of them (3.226 mm²) and native 19 (2.227 mm²). On OCCT, 1.675 mm² of these are tilted 20 to 80 degrees, and 1.416 mm² of that has corners on face 1 (on [1, 3], [1, 3, 7] or [1]). No native facet with a corner on no face is tilted more than 20 degrees.
- +0.165 in the 2 steps left at face 2.
- +0.026 in the facets with a corner on face 7: 4 on OCCT (0.595 mm², two of them 50-degree facets at the face 2 / face 7 corner, the same on and off) and 2 on native (0.569 mm²).
- +0.014 in the flat bulk.

The crease stage cannot reach the first two kinds. It takes only boundary edges whose two ends lie on model faces, and these facets have a corner more than a hundredth of the size off every face. That is the residual Phase 173 recorded: a boundary vertex sunk inside the solid next to the crease. The OCCT surface leaves more such vertices along face 3's edges than the native one. Phase 172 found the same among the tilted facets: 3.16 mm² with an unsnapped corner on OCCT against 2.31 mm² on native.

## Limits

- `screw.step` only, at its default size, on macOS arm64. The breakdown was not repeated on the built-in models.
- This finds where the surplus is (facets with an unsnapped corner, on the OCCT path mostly round face 1). It does not explain why the snap leaves those vertices unsnapped, or why OCCT's surface leaves more of them than native's. That needs the snap's refusal trace (`MeshReport::refused`) on these vertices.
- Faces 1, 2 and 7 are named only by their numbers here. Phase 172 identifies face 3 as the slot bottom, meeting the slot walls along concave edges.
