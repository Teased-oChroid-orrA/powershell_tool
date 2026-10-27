# Phase 145: merging an edge that neither end can take at its midpoint

2026-10-26. Continues [Phase 144](issue-46-phase-144.md) on `main`. OCCT remains production and `occt_retirement_approved` remains false.

## Problem

After Phase 144, NEMA17 keeps 23 (OCCT) and 24 (native) elements under the 0.1 gate, all on the boundary. Phase 144 did not check whether they share the t-junction's seam-gap cause.

## Root cause (traced with `REPAIR_TRACE`, verified by the fix)

- The bracket's two gussets (x in [25, 30] and [-30, -25]) rise from the base top (z = 5) at y = -15 with a 45 degree slope. Where the slope meets the base top there is a reentrant crease between two planes, so there is no tessellation seam gap there. The seam-gap cause is not involved.
- 22 of the 47 stuck cells (both paths, both gussets) share one boundary edge 0.079 long, about 0.05 of the local size. Its ends are two boundary vertices at the same lattice x (26.842 or -26.842). One is 0.004 from the crease. The other is 0.074 from it, 0.016 below the base top. Both lie within half a size of both faces, so `fixed()` pins both as feature vertices.
- Collapse refused the edge in both directions. Moving the off-crease vertex onto the on-crease one turns an element inside out (`collapse#5`). The reverse fails the surface check (`collapse#8`). Both vertices are pinned, so no vertex move separates them, and a rebuild keeps the boundary faces exactly, so it cannot remove the edge either.
- The other stuck cells are mostly at three-face corners with no mesh vertex on the corner: the gusset corners (+-25, -15, 5) and (+-25, 20, 40), with two or three vertices 0.1 to 0.24 off each corner. There are also a few at the slot ends on the base bottom and at the fillet between base and plate.

## Change (`fea-core/src/meshing/boundary_repair.rs`)

- `collapse` becomes `collapse_at(u, v, at)`. With `at`, the surviving vertex `v` moves to `at` in the same change:
  - The change is judged over both stars (`u`'s and `v`'s), with the same rules as before: no more elements under the gate than they held, and `better_than` on worst element and penalty. Every new element must be positive and must not stretch an edge beyond 1.3 times the longest old one.
  - The surface is checked against the boundary faces before the move: `new_faces_follow_surface`, plus the deviation of the removed vertex and of the survivor's old position from the new faces around the survivor. The bound is the existing one (half of it for a pinned vertex).
  - With `at = None` the behaviour is unchanged: the same cells, checks and order as before.
- `merge_point(a, b)` gives the target: the midpoint, projected onto the surface for a boundary edge. When both ends are pinned on the same model edge (the same two faces from `feature_edge`), the projection is onto both faces, so the merge lands on the crease. It returns `None` when one end is pinned and the other free (plain collapse covers that), and also for corners, different features, and a boundary vertex paired with an interior one.
- `cleanup_tet` tries the midpoint merge after both plain collapses of every edge have failed, shortest edge first.

## Tried and dropped (negative result)

| Attempt | NEMA17 OCCT | NEMA17 native |
|---|---|---|
| Phase 144 | 23 | 24 |
| Midpoint merge along a shared model edge (kept) | 13 | 12 |
| Also merging pinned pairs whose nearby faces (within half the size) are the same set of three or more, at the midpoint projected onto all of them (a corner) | 13 | 12 |

The corner variant produced byte-identical rows: no corner merge was ever kept. It was reverted. The corner clusters need a different operation, for example inserting a vertex at the model corner.

## Tests

- `boundary_repair::tests::an_edge_neither_end_can_take_merges_at_its_midpoint`: a lattice (`MERGE_SHIFT`, found by a seeded search, with cell volumes adding up to the box's 162) where vertices 21 and 22 are 0.45 apart. Both plain collapses are refused and leave no log or count behind. A target that would turn an element inside out is refused with the mesh untouched. The midpoint merge is kept: the survivor sits at the midpoint, the edge's elements are gone, every element is positive, the volume is unchanged, boundary vertices are unchanged, there are no more elements under the gate in the survivor's star and its worst element is better, and the change is committed. Without model faces a pinned boundary pair has no merge point.
- `fea-problem/tests/occt_vs_native.rs::the_nema17_native_relaxation_starts_from_the_best_round` is tightened from `<= 24` to `<= 12`.

## Evidence (macOS arm64, release, `CARGO_INCREMENTAL=0`, serial Cargo)

- Mesh-only 19-model table (`NO_SOLVE=1`, `docs/issue-46-phase-145-parity.txt`): **36/38 rows pass**, the same as Phase 144. 30 rows are identical to Phase 144. Six passing rows change slightly and still pass the gates: plate with two holes (native), plate with a boss (native), box + boss (native), edit plate (native), cross-bore (native) and tilted hole (OCCT). In those rows the element count drops by 3 to 20, the radius ratio moves by up to 0.003 and the edge ratio by up to 0.7 (edit plate 5.1 to 5.8, plate with a boss 7.2 to 6.7), in either direction.

  | Model | Phase 144 | Phase 145 |
  |---|---|---|
  | NEMA17, OCCT | 23 under the gate, edge ratio 23.4 | 13 (205957 elements, edge ratio 11.3) |
  | NEMA17, native | 24, min sJ 0.047 | 12 (206035 elements, min sJ 0.053) |

- NEMA17 per stage (`REPAIR_TRACE`): the best round before the rebuild stage is OCCT 14 and native 15 (Phase 144: 24 and 27). The rebuild stage then takes OCCT to 13 (1 rebuild) and native to 12 (3 rebuilds). No stuck cell has the 0.079 edge any more. NEMA17 native mesh time fell from 198 s to 156 s.
- `the_nema17_native_relaxation_starts_from_the_best_round` (now `<= 12`), `the_t_junction_repair_keeps_its_best_round` (0 on both paths) and `connected_plate_boss_rim_passes_solver_gate` pass.
- `cargo test -p fea-core -p solid-kernel -p fea-problem --release --locked --offline -j1 --no-fail-fast`: 1128 passed, 0 failed, 47 ignored (Phase 144: 1127, plus the new unit test).
- `cargo test -p app-tui --release --locked --offline -j1`: 1256 passed, 0 failed, 24 ignored (unchanged).

## Limits and what is still open

- NEMA17 (OCCT 13, native 12) still fails the gate, all on the boundary. What remains:
  - three-face corners of the gussets with no vertex on the corner (about half the remaining cells);
  - two flat elements with two boundary faces at each slot end on the base bottom;
  - a cluster at the fillet between base and plate (x about 16, y about 20, z about 8.6);
  - a cluster near the corner (-30, 20, 40).
- The merge covers only pairs pinned on the same model edge, or two free boundary vertices, or two interior vertices. Mixed and corner pairs are not merged.
- Solves were not rerun for this phase: the mesh changed on NEMA17 and slightly on six passing rows. The other limits in Phase 144 are unchanged, including `fillet_rim_tagging` failing with OpenCascade on and the lack of Windows evidence.
