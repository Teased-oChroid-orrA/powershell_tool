# Phase 146: moving a vertex onto a three-face model corner

2026-10-27. Continues [Phase 145](issue-46-phase-145.md) on `main`. OCCT remains production and `occt_retirement_approved` remains false.

## Problem

After Phase 145, NEMA17 keeps 13 (OCCT) and 12 (native) elements under the 0.1 gate, all on the boundary. About half of them sit at the gussets' three-face corners: (+-25, -15, 5), where the gusset side, the 45 degree slope and the base top meet, and (+-25, 20, 40), where the gusset side, the slope and the plate face y = 20 meet. No mesh vertex lies on these corners; two or three vertices sit 0.1 to 0.24 away.

## Root cause (traced with `REPAIR_TRACE`)

- A boundary vertex within half the local size of two or more model faces whose normals differ is pinned by `fixed()`. `optimise_vertex_with` then asks `feature_edge(v)` for the model edge to slide along. With three or more faces nearby that returns `optimise_vertex#1`, and the vertex is never moved. That refusal fired 440 times in the Phase 145 trace.
- After a first attempt (below), a diagnostic run showed a second defect: `surface_point` reaches a point on several faces by projecting onto one face after the other. Where two faces meet at angle `a`, each sweep shrinks the error by about `cos(a)^2`. At 45 degrees (the slope against the plate face) that is a factor of 2 per sweep, so its 12 sweeps leave about `2e-4` of the start offset, far above the `1e-6 h` it then demands. All 401 corner refusals the verbose trace printed in that run failed this way. The same loss applies to a vertex sliding along a model edge where two faces meet at a shallow angle.

## Change (`fea-core/src/meshing/boundary_repair.rs`)

- `corner_faces(v)`: the faces within half the size of `v`, when there are three or more.
- `move_to_corner(v, faces, push)`: when `feature_edge` refuses a pinned vertex and `corner_faces` finds a corner, the vertex gets one candidate: `surface_point` on all of those faces, which is the corner. A corner more than one local size away is refused: a corner whose faces are each within half the size lies within about 0.87 of it, and a farther one (two of the faces meeting at a shallow angle) is not this vertex's corner. The move is kept only under the existing contracts: no more elements under the gate in the star (unless `push`), `new_faces_follow_surface` against the faces before the move, and `gains` on the star's worst element. Otherwise it is rolled back. Refusals are traced as `corner#1` (no corner point), `corner#2` (already on it), `corner#3` (more elements under the gate), `corner#4` (surface) and `corner#5` (no gain).
- `point_on_faces(x, faces, h)`: Gauss-Newton on the faces' signed distances with the smallest step that zeroes them (the 2 x 2 Gram system for two faces, least squares for three or more). It returns `None` when the faces fix no point (nearly parallel) or it does not converge in 8 steps. `surface_point` starts from it whenever it is given two or more faces. The face-by-face sweep and every acceptance check after it are unchanged, so it now only confirms the point. With one face nothing changes.
- No tolerance, gate or model-specific constant was changed or added.

## Results by step (NEMA17, `REPAIR_TRACE`)

| Variant | NEMA17 OCCT | NEMA17 native |
|---|---|---|
| Phase 145 | 13 | 12 |
| Corner move, face-by-face projection | 13 (identical row) | 12 (identical row) |
| Corner move, Gauss-Newton corner point | 12 | 12 |
| Also seeding every multi-face `surface_point` (kept) | 12 | 12 |

- With the Gauss-Newton point, `corner#1` falls from about 12,400 to about 3,500 refusals (both paths). Most corner points found are then refused by the existing contracts: `corner#3` about 5,400 and `corner#4` about 3,500. `corner#5` (no gain) fires 12 times.
- Where the cells are: on OCCT the stuck cell at the upper corner (25, 19.8, 39.8) is gone and one appears at (-26.05, 20, 40), for a net drop of 1. The lower-corner cells (around (+-25.5, -13.7, 5.3)) remain on both paths. Their vertices sit about 1.3 from the corner, more than half the local size, so `corner_faces` never names that corner for them. This phase does not reach them.
- Capping the corner move at one local size changes no row of the full table (checked by rerunning it).
- Seeding the edge slides as well changes no NEMA17 row. It is kept because the unit test shows the sweep failing on a shallow crease, and because it moves four other rows within the gates (below).

## Tests

- `a_vertex_near_a_three_face_corner_moves_onto_it_and_then_stays`: the cube [-1, 0]^3 in six elements, in the corner of the octant x, y, z <= 0, with the corner vertex pulled 0.15 to 0.25 inside. `feature_edge` refuses it with `optimise_vertex#1`. `optimise_vertex` then moves it onto the corner (to within 1e-9), the worst element improves, the cells tile the cube again (volume restored exactly), and a second call is refused with `corner#2`.
- `a_corner_far_beyond_the_size_is_not_the_vertex_corner`: three faces within half the size of the corner vertex, two of them 10 degrees apart, meet 1.7 away. The move is refused with `corner#1` and nothing changes.
- `a_corner_move_that_would_crush_an_element_is_refused_and_leaves_the_mesh_alone`: the same cube with the vertex on the edge x = y = 0 placed 0.1 below the corner. The move is refused with a `corner#` reason, and positions, cells and counters are unchanged.
- `a_surface_point_on_faces_at_a_shallow_angle_reaches_their_edge_and_corner`: faces x = 0, y = 0 and a plane 15 degrees off y = 0. `point_on_faces` and `surface_point` reach the corner, `surface_point` on the two shallow faces reaches the nearest point of their edge, and parallel faces give `None`.
- `the_nema17_native_relaxation_starts_from_the_best_round` stays at `<= 12`: the native count did not change. There is no OCCT-path NEMA17 bound to tighten.

## Evidence (macOS arm64, release, `CARGO_INCREMENTAL=0`, serial Cargo)

- Mesh-only 19-model table (`NO_SOLVE=1`, `docs/issue-46-phase-146-parity.txt`): **36/38 rows pass**, the same as Phase 145. 33 rows are identical apart from mesh time. Four passing rows change slightly and still pass: plate with two holes (native, +8 elements, min sJ 0.121 to 0.120), cross-bore (OCCT +14 elements, radius ratio 0.041 to 0.036; native +11 elements, min sJ 0.120 to 0.114), and vase (native, +3 elements). NEMA17 OCCT goes from 13 to 12 elements under the gate (205952 elements); NEMA17 native is unchanged at 12.
- NEMA17 per stage (`REPAIR_TRACE`): the best round before the rebuild stage is OCCT 12 (Phase 145: 14), so the rebuild stage is not needed; native is unchanged (15 before rebuilds, 12 after 3 rebuilds).
- `the_nema17_native_relaxation_starts_from_the_best_round` (`<= 12`), `the_t_junction_repair_keeps_its_best_round` and `connected_plate_boss_rim_passes_solver_gate` pass. The two OCCT tests and app-tui ran before the one-size cap was added; the cap leaves every table row identical, including both of their models.
- `cargo test -p fea-core -p solid-kernel -p fea-problem --release --locked --offline -j1 --no-fail-fast`: 1132 passed, 0 failed, 47 ignored (Phase 145: 1128, plus the four new unit tests).
- `cargo test -p app-tui --release --locked --offline -j1`: 1256 passed, 0 failed, 24 ignored (unchanged).

## Limits and what is still open

- NEMA17 (OCCT 12, native 12) still fails the gate, all on the boundary. What remains:
  - the lower gusset corners (+-25, -15, 5): their nearest vertices are about 1.3 from the corner, beyond half the size, so no corner move is tried. Reaching them would need a vertex inserted at the corner, or a wider reach that is checked against the neighbouring vertices;
  - upper-corner cells where the corner point exists but the move would push more elements under the gate (`corner#3`) or break the surface contract (`corner#4`);
  - two flat elements with two boundary faces at each slot end on the base bottom;
  - a cluster at the fillet between base and plate (x about 16, y about 20, z about 8.6);
  - a cluster near the corner (-30, 20, 40).
- The corner move tries one candidate (the corner itself). It does not search along the corner's edges.
- What generalises: both changes depend only on the model's face distances and normals, not on this model. `point_on_faces` applies wherever two or more faces meet at a shallow angle. What is shown: one fewer cell on one NEMA17 path, and no gate lost on the other 18 models. A large effect on other models is not shown.
- Solves were not rerun for this phase. The other limits in Phase 145 are unchanged, including the lack of Windows evidence.
