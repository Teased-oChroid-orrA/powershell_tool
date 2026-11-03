# Phase 171: the benchmark's loaded area on `screw.step`, face by face

Issue #46. Follows [Phase 170](issue-46-phase-170.md), whose benchmark solve of `screw.step` gave native a reaction 3.0 % below OpenCASCADE's (371.7 against 383.3) and put it down, unmeasured, to native sampling small circles to the chord alone. This phase measures it. Measurement only: no solver, mesher or tessellation change; no gate, tolerance, budget, cap or timeout changed. `occt_retirement_approved` stays false.

## Method

`fea-problem/tests/occt_vs_native.rs` gains `benchmark_face_areas` (ignored, `CASE` / `STEP_DIR` as for `table`). For each path it takes the benchmark's fixed and loaded faces by the same rule as `measure` and prints, per face:

- the area of the surface triangles handed to the mesher, against the exact B-rep face area (OpenCASCADE `BRepGProp::SurfaceProperties` through `Brep::faces`; each tessellated face is paired with the exact face whose centre is nearest its area centroid; all pairings were within 7e-3 mm);
- the boundary polygon of the face (segments, longest);
- then, after meshing the volume at the benchmark size, the area of the volume mesh's boundary facets named after the face (corner nodes), how much of it is tilted more than 1 degree from the face's plane, and how much has a corner off the plane.

Run on `screw.step` (local copy, not committed), macOS arm64, release, OpenCASCADE 8.0.1 (cadrum's prebuilt), 305 s.

## Result

| | Exact | OCCT | native |
|---|---|---|---|
| Loaded face 3 (z = 2.936), surface triangles | 37.3257 | 37.3012 (-0.07 %), 11 boundary segments | 37.2707 (-0.15 %), 8 segments |
| Loaded face 3, volume-mesh facets | | 39.0471 (+4.6 %), 240 facets, 6.645 tilted > 1 deg | 37.7766 (+1.2 %), 236 facets, 5.506 tilted |
| Fixed face 10 (z = -34.56), surface triangles | 52.2936 | 51.8855 (-0.78 %), 29 segments | 51.4377 (-1.64 %), 20 segments |
| Fixed face 10, volume-mesh facets | | 51.7192, 0.698 tilted | 51.3310, none tilted |
| Reaction z (Phase 170 solve) / traction x exact area (373.26) | | 383.31 (+2.7 %) | 371.73 (-0.4 %) |
| Compliance U / R² | | 1.4563e-6 | 1.4545e-6 (-0.12 %) |
| umax / R | | 5.578e-6 | 5.591e-6 (+0.23 %) |

(Areas in mm², reaction in N, traction 10 MPa.)

## What this shows

- The hypothesis of Phase 170 is wrong for the load. The loaded face's surface triangles agree between the paths within 0.08 % and lie within 0.15 % of the exact area: chord sampling is not where the 3 % comes from.
- The difference is in the volume mesh. The solve loads the boundary facets named after the face, and `tetmesh` names a facet by the face all its corners lie on, else by the face nearest its centroid. The snapping rounds the face's sharp edges within one element (a stated `tetmesh` limit), and the tilted rounding facets that go to face 3 add 6.6 mm² on the OCCT path and 5.5 mm² on the native one. The volume mesher is the same on both paths; what differs is the surface each one snaps to near those edges.
- Against the exact load, native is the closer of the two: OCCT's path applies 2.7 % more force than traction x exact area, native's 0.4 % less.
- With the load taken out, the two paths agree: compliance U / R² within 0.12 %, umax per unit reaction within 0.23 %. The lower native umax and strain energy of Phase 170 are the lower load.
- The reaction is 1.6 % (native) to 1.8 % (OCCT) below 10 x the facet area measured from corner nodes. The facet area does not use the Tet10 midside nodes moved onto the surface; that this accounts for the gap was not measured.
- Chord sampling does show on the fixed face: native's surface triangles are 1.64 % below the exact area against OCCT's 0.78 % (20 boundary segments against 29). That is the same chord effect as the 0.91 % volume deficit in Phase 170. It does not change the benchmark's force, which is set at the loaded face.

## Limits

- One model, one element size, macOS only.
- The facet naming at rounded sharp edges is a property of the lattice mesher on both paths. It is recorded as a pitfall in `fea-problem/AGENTS.md` and not changed here: changing how `tetmesh` names or recovers sharp-edge facets moves every solid result in the app and needs its own phase with its own evidence.

## Follow-ups

- Replace Phase 170's follow-up "chord policy for small circles: measure the loaded-face area": measured, the load gap is facet naming. A native chord-plus-angle rule for small circles would still close the fixed-face and volume deficits (1.64 % and 0.91 % against OCCT's 0.78 % and 0.63 %); it is a tessellation change with its own evidence to collect.
- Facet naming at rounded sharp edges in `tetmesh` (both paths): a load on a face bounded by sharp edges carries a few per cent more or less than traction x exact area.
