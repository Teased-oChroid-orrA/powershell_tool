# Phase 144: the reentrant-crease proof within the surface accuracy, and stuck-region rebuilds

2026-10-26. Continues [Phase 143](issue-46-phase-143.md) on `main`. OCCT remains production and `occt_retirement_approved` remains false.

## Problem

After Phase 143 the t-junction keeps 5 (OCCT) and 8 (native) elements under the 0.1 gate where the branch meets the main pipe, and NEMA17 keeps 24 (OCCT) and 25 (native). Phase 143 blamed the repair's path; this phase traced the t-junction cells back to the snap stage instead.

## Root cause at the t-junction (verified)

- On a tessellated model the two faces at an inward (reentrant) crease do not meet exactly. Alternating projection onto both stops between 1e-6 and 1.2e-3 off one of them on the t-junction, at sizes near 1.
- `reentrant_edge` (`fea-core/src/tetmesh.rs`) required both face distances within `1e-3 * eps`, where `eps` is the surface accuracy (a thousandth of the local size). That is about 1e-6 of the size, a round-off tolerance. It therefore never proved the t-junction crease: every stuck vertex came back "unconverged".
- With the crease unproven, the snap fell back to pulling the wall vertex onto the crease line. The quality guard refused part of that pull, so wall vertices ended about 0.1 off the surface (signed distance -0.18 to +0.075), within half a size of both faces. The repair's `fixed()` then pinned them as feature vertices, and the cells around them could not be repaired.
- The previous hypothesis ("keeping only one face leaves no vertex on the crease") was wrong in direction: the vertices were dragged towards the crease, not kept off it.

## Change (d61a794)

### The crease proof within the surface accuracy (`fea-core/src/tetmesh.rs`)

- `reentrant_edge` accepts `|dist| <= eps` for both faces (was `1e-3 * eps`). The sector samples that decide inside/outside move out to `step = 8 * eps`, with thresholds at `step / 2`, so they stay well clear of the seam gap.
- This is a correction of the surface accuracy the proof uses, not a gate change: `eps` is the accuracy the snap already works to everywhere else. A gap wider than `eps` is still not a seam (the new test checks that it fails closed).

### Stuck-region rebuilds (`fea-core/src/meshing/boundary_repair.rs`)

- After the rounds (and the restore of the best round), each element still under the gate gets `Work::rebuild(t)`: a cavity around `t` (its face ring, the cells around each of its edges, or the two-ring) is replaced by the star of one new interior vertex over the cavity's outer faces. The outer faces, mesh boundary faces among them, are kept exactly, so the surface does not change. This reaches caps whose vertices are all pinned, where no vertex move or reconnection that keeps the surface exists.
- The new vertex is placed by pattern search on the star's worst scaled Jacobian, seeded with centroids and the apex that would make each outer face's element regular.
- Kept only when the star leaves **strictly fewer** elements under the gate than the cavity held and the worst scaled Jacobian is not lower (the Phase 143 rule). Up to four passes over the stuck elements. `RepairReport::rebuilds` counts kept rebuilds; `REPAIR_TRACE` prints the count per pass.
- The cavity must be provably fillable by the star:
  - its outer faces, each oriented outward from its cell, form a closed oriented surface: every directed edge once and its reverse once, Euler characteristic 2;
  - every star cell is positive;
  - the star's total volume equals the cavity cells' total (relative 1e-10). Over a closed oriented surface the star's volume is the enclosed volume for any apex, so this fails exactly when cavity cells fold over each other.
- The first version checked only that every edge lay in two faces and the Euler characteristic. Its unit test failed: on the folded `CHAIN_SHIFT` lattice (cell volumes adding to 162.346 against the box's 162) a rebuild was kept that grew the total volume to 172.146, so the star overlapped its neighbours. A positive Jacobian per star cell does not prove a tiling. With the orientation and volume checks every rebuild on that lattice is refused.

## Tried and dropped (negative results)

| Attempt | t-junction OCCT | t-junction native |
|---|---|---|
| Phase 143 | 5 | 8 |
| Crease-line vertices (boundary triangles tagged with both faces) take the crease point in the reentrant branch | 12 | 9 |
| Repair `fixed()` exempts vertices beside a proven reentrant edge | 10 | 2 |
| The tolerance fix alone (kept) | 0 | 0 |

The `fixed()` exemption never fired at the stuck vertices, because the crease proof failed there for the root cause above. After the tolerance fix it is not needed.

## Tests

- `tetmesh::tests::reentrant_snap_holds_where_the_two_faces_miss_each_other_by_a_tessellation_gap`: two faces missing each other by 1e-4. Before the fix a wall point at `[0, 0, 0.05]` was pulled to `[0, 0, 1e-4]`; after it, wall points stay and a point between the faces goes to the right wall. A gap of 0.01 (wider than the accuracy) still fails the proof.
- `boundary_repair::tests::a_rebuild_keeps_the_cavity_surface_and_is_kept_only_when_it_lowers_the_count`: an unfolded lattice (`REBUILD_SHIFT`, cell volumes add up to the box's; found by a seeded search) with 3 elements under 0.1. Two rebuilds are kept, each with strictly fewer elements under the gate, every element positive, the total volume unchanged, no existing vertex moved, committed. One is refused and leaves the mesh, the log and the report untouched.
- `boundary_repair::tests::a_rebuild_over_a_fold_is_refused`: on the folded `CHAIN_SHIFT` lattice all 5 rebuilds are refused. The first version kept one there and grew the volume (above).
- `fea-problem/tests/occt_vs_native.rs::the_t_junction_repair_keeps_its_best_round` (occt, ignored, about 23 s) now meshes the t-junction through both tessellations and requires 0 elements under the gate (was OCCT only, `<= 5`).
- `fea-problem/tests/occt_vs_native.rs::the_nema17_native_relaxation_starts_from_the_best_round` (occt, ignored, about 3.5 minutes) is tightened from `<= 25` to `<= 24`.

## Evidence (macOS arm64, release, `CARGO_INCREMENTAL=0`, serial Cargo, at d61a794)

- Mesh-only 19-model table (`NO_SOLVE=1`, `docs/issue-46-phase-144-parity.txt`): **36/38 rows pass** (Phase 143: 34/38). Every row except t-junction and NEMA17 is identical to Phase 143 in elements, dofs, scaled Jacobian, radius ratio, edge ratio, size jump, gates and sub-gate counts.

  | Model | Phase 143 | Phase 144 |
  |---|---|---|
  | t-junction, OCCT | 5 under the gate | 0 (20304 elements, min sJ 0.120) |
  | t-junction, native | 8 | 0 (20665 elements, min sJ 0.123) |
  | NEMA17, OCCT | 24 | 23 (206002 elements) |
  | NEMA17, native | 25 | 24 (206072 elements) |

- NEMA17 per stage (`REPAIR_TRACE`): the best round before the rebuild stage is OCCT 24, native 27. The snap fix alone costs native 2 against Phase 143's 25. The rebuild stage takes OCCT to 23 (1 rebuild) and native to 24 (3 rebuilds). Rerun after the orientation and volume checks were added: the same 23 / 24 with the same 1 / 3 rebuilds, so each of those rebuilds passes the fill proof. Two runs are identical.
- The t-junction, boss and other rows never reach the rebuild: they have no element under the gate after the rounds.
- Solves with this change, against Phase 143: t-junction OCCT umax 1.45503e-3 (1.45634e-3), native 1.44638e-3 (1.44686e-3), OCCT/native gap 0.60 %. NEMA17 OCCT 6.73138e-3 (6.73131e-3), native 6.71352e-3 (6.71349e-3). The three boss models are identical to Phase 143 including umax. These solves ran before the rebuild's fill checks were added; the meshes they used are identical to the rerun meshes (same element counts, dofs and quality columns).
- t-junction mesh time 12 s / 10 s (OCCT / native), was 18 s / 41 s.
- `the_t_junction_repair_keeps_its_best_round` and `the_nema17_native_relaxation_starts_from_the_best_round` pass at d61a794. `connected_plate_boss_rim_passes_solver_gate` passes; `tests/tetmesh` 9 passed, `tests/tetmesh_repair` 4 passed.
- `cargo test -p fea-core -p solid-kernel -p fea-problem --release --locked --offline -j1 --no-fail-fast`: 1127 passed, 0 failed, 47 ignored (Phase 143: 1124, plus the three new unit tests).
- `cargo test -p app-tui --release --locked --offline -j1`: 1256 passed, 0 failed, 24 ignored (Phase 143's 1226 / 22 predates the Search and Fastener Hole workbench merges, PRs #175 and #176; this phase changes no app-tui code).
- Solve and mesh times were taken under outside load and are not compared beyond the t-junction mesh time.

## Limits and what is still open

- NEMA17 (OCCT 23, native 24) still fails the gate, all on the boundary. Its stuck cells were not examined for the seam-gap cause; the rebuild only lowers the count it reaches.
- The snap fix alone makes NEMA17 native's best round worse (27 against 25); only the rebuild brings it below Phase 143. The repair remains sensitive to its path.
- The rebuild's cavity set is fixed (face ring, edge shells, two-ring); larger or shaped cavities were not tried.
- `fillet_rim_tagging` still fails whenever the OpenCascade feature is on (pre-existing; Phase 143 Limits). Everything else in the Phase 143 Limits list is unchanged, including Windows evidence.
