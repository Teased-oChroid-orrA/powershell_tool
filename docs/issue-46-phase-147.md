# Phase 147: merging a pinned pair beside a three-face corner onto the corner

2026-10-27. Continues [Phase 146](issue-46-phase-146.md) on `main`. OCCT remains production and `occt_retirement_approved` remains false.

## Problem

After Phase 146, NEMA17 keeps 12 (OCCT) and 12 (native) elements under the 0.1 gate, all on the boundary. Phase 146 left the lower gusset corners (+-25, -15, 5) out of reach and gave the reason as "their nearest vertices sit about 1.3 from the corner, beyond half the size, so `corner_faces` never names that corner". The planned next step was to insert a vertex at a model corner that no vertex is within half a size of.

## What the trace shows (`REPAIR_TRACE`, baseline 12/12 confirmed)

The premise of the planned step is wrong, so it was not built.

- On both paths, the stuck cells at the lower corners have the same four vertices: an interior vertex A about 0.42 inside the solid (for example (25.42, -13.69, 5.38)), a vertex B on the slope 0.15 off the slope / gusset-side edge (25.15, -14.80, 5.20), a vertex C on the gusset side face (25.0, -13.41, 6.11), and a vertex D on the slope / gusset-side edge (25.0, -14.93, 5.07). A second cell swaps B for a vertex E on the base-top / gusset-side edge (25.0, -14.70, 5.0).
- D is 0.095 from the corner, well within half its size (1.09). `corner_faces(D)` names all three faces and the corner move of Phase 146 is tried; the retry trace refuses it with `corner#3` (it pushes neighbours under the gate) and, in the chained move, `corner#4` (surface). The figure of 1.3 in Phase 146 is the distance from the interior vertex A to the corner, not from the nearest boundary vertex. Phase 145's own count (two or three vertices 0.1 to 0.24 off each corner) was right.
- The flat cell is A over the boundary face B-C-D. B and D are both pinned and only 0.24 apart (about a fifth of the size), and the face B-C-D straddles the convex slope / gusset-side edge.

## Change (`fea-core/src/meshing/boundary_repair.rs`)

`merge_point(a, b)` gives the point where the edge a-b may merge when neither end can take the other (Phase 145). For two pinned ends it required both to sit on the same model edge (`feature_edge`), so a vertex on a model edge beside a corner vertex could never merge with it. Now, when `feature_edge` refuses either end:

- the corner faces are those of whichever end has three or more faces within half its size (`corner_faces`);
- every face within half the size of either end must be one of them (an end held by another face is not at this corner);
- the target is `surface_point` on all of those faces, which starts from the Gauss-Newton `point_on_faces` (Phase 146), so it is the corner;
- the corner must be within one local size of both ends.

The merge itself is the existing `collapse_at`, unchanged: a pinned end may only go onto a vertex whose nearby faces include its own, the link condition, every remaining element positive, no more elements under the gate and a better worst element, a surface deviation bound of half the usual one for a pinned vertex, and `new_faces_follow_surface` against the faces before the change. No tolerance, gate or model-specific constant was changed or added.

Phase 145 tried a corner merge that required both ends to have the same set of three or more faces, before Phase 146 seeded multi-face surface points with Gauss-Newton; it was never kept. This version also admits an edge vertex beside a corner vertex, which is the pair the trace shows.

## Results (NEMA17, `REPAIR_TRACE`)

| Variant | NEMA17 OCCT | NEMA17 native |
|---|---|---|
| Phase 146 | 12 | 12 |
| Corner merge of a pinned pair (kept) | 12 (identical row) | 11 |

- The OCCT row is byte-identical apart from mesh time: no corner merge was kept on that path.
- On the native path the only behavioural difference is this branch, so at least one corner merge was kept there. The cell that goes is printed without coordinates; its vertex sizes (1.588 / 1.037 / 0.989 / 0.734) match the OCCT cell at (25, 19.44, 39.44), so it is most likely at the upper corner (25, 20, 40). The native mesh has 40 fewer elements (205995).
- At the lower corners the merge is offered and refused by the quality contract: the retry trace shows two extra `collapse#7` refusals after the plain collapses (the merged star would have more elements under the gate or a worse worst element). The lower-corner cells stay on both paths.

## Tests

- `two_pinned_vertices_beside_a_corner_merge_onto_it`: the Phase 146 corner cube with vertex 7 on the edge x = y = 0, 0.1 below the corner (three faces near), and vertex 3 on the same edge 0.6 below (two faces near), both pinned. `merge_point` in either order returns the corner (within 1e-9). With the ends at 0.4 and 1.3 below the corner, the corner is beyond one size of vertex 3 and `merge_point` returns `None`.
- `a_corner_merge_needs_every_face_near_either_end_at_the_corner`: the same pair with a fourth plane z = -0.8 near vertex 3 only: `merge_point` returns `None`.
- `the_nema17_native_relaxation_starts_from_the_best_round` is tightened from `<= 12` to `<= 11`.

## Evidence (macOS arm64, release, `CARGO_INCREMENTAL=0`, serial Cargo)

- Mesh-only 19-model table (`NO_SOLVE=1`, `docs/issue-46-phase-147-parity.txt`): **36/38 rows pass**, the same as Phase 146. 37 rows are identical to `issue-46-phase-146-parity.txt` apart from mesh time. NEMA17 native goes from 12 to 11 elements under the gate; NEMA17 OCCT stays at 12.
- NEMA17 per stage (`REPAIR_TRACE`): OCCT is unchanged (best round 12 before the rebuild stage). Native: after three stuck-region rebuilds it ends at 11 (Phase 146: 12 after three rebuilds).
- `the_nema17_native_relaxation_starts_from_the_best_round` (now `<= 11`), `the_t_junction_repair_keeps_its_best_round` and `connected_plate_boss_rim_passes_solver_gate` pass.
- `cargo test -p fea-core -p solid-kernel -p fea-problem --release --locked --offline -j1 --no-fail-fast`: 1134 passed, 0 failed, 47 ignored (Phase 146: 1132, plus the two new unit tests).
- `cargo test -p app-tui --release --locked --offline -j1`: 1256 passed, 0 failed, 24 ignored (unchanged).

## Limits and what is still open

- NEMA17 (OCCT 12, native 11) still fails the gate, all on the boundary. What remains:
  - the lower gusset corners (+-25, -15, 5): a vertex is already near the corner; moving it there (`corner#3`, `corner#4`) and merging its short edge onto it (`collapse#7`) are both refused by the existing contracts. Inserting a corner vertex would not help, since one is already within 0.1. What would is an operation that re-tiles the face B-C-D across the convex edge and lifts the interior vertex A together; that is untried;
  - upper-corner cells (corner moves refused by `corner#3` / `corner#4`), on both paths;
  - two flat elements with two boundary faces at each slot end on the base bottom;
  - a cluster at the fillet between base and plate (x about 16, y about 20, z about 8.7);
  - a cluster near the corner (-30, 20, 40).
- What generalises: the merge depends only on the model's face distances, and only on pairs that are already pinned beside a corner. What is shown: one fewer cell on one NEMA17 path and every other table row identical. A large effect on other models is not shown.
- Solves were not rerun for this phase (nor for Phases 145 and 146). There is no Windows evidence. `fillet_rim_tagging` still fails with OpenCascade on (pre-existing, `fea-problem/tests/fillet_rim_tagging.rs:53`).
