# Phase 160: a point search for slivers, and reconnection that respects the solver gate

Issue #63, excluding NEMA17 by owner request. The ABC files are the Phase 156-159 pair (`00006600`, `00007062`); they stay in temporary local storage only. The goal is unchanged: both files through the default solver gate (scaled Jacobian at least 0.1, edge ratio at most 50) and solved, with no gate, tolerance, budget or timeout relaxed. Branch `phase-160-candidate-search`, based on `main` at `ed0abf2`.

## Correction to Phases 158 and 159

The Phase 158 doc's table row "Quality Steiner points kept ... 35,714 (stage ends by itself)", its limits paragraph ("Given unlimited time it ends by itself with 10,670 cells below its 0.2 bound ... whose only candidate point, the circumcentre or centroid, is refused"), and the Phase 159 doc's row "`PLC_QSEC=900`: whole run (stage ends by itself)" are wrong about why the stage stops.

The sliver stage ends at its point cap or at its clock, not because its remaining candidates are refused. With `PLC_QSEC=900` it ends at its point cap (`max_quality_steiner`, 50,000). The cap counts every Steiner point inserted since the stage began: quality points and the segment and sub-facet splits that encroached centres queue. In the last round of the Phase 159 run the stage had 50,551 Steiner points; only about 35 cells per round were refused (mostly a centre that encroaches a segment already below the sliver floor). The 35,714 quality points plus the boundary splits reach the cap. In the default run the 20 s clock ends it first.

## Diagnosis

`plc-run` on the dumped shells (`/private/tmp/abc-issue63/dumps-157/*.body0.plc`, `STAGEDBG=1 PLC_MERGE=1 PLC_REFINE=1 PLC_QSEC=900`), macOS arm64, release.

1. **Circumcentre insertions make new slivers.** On `00006600` the first round placed 215 centres, whose insertions created 22,368 cells under the stage's 0.2 bound (about 100 per point; the count includes cells outside the domain). The second round placed 81 and created 10,749. In the last rounds the rate fell to about 2 per point. Most of the stage's points go to slivers that earlier points made.
2. **Post-stage reconnection makes most of the remaining gate failures.** After the stage, `extract` runs `tet_flip::improve`, which accepts a flip when it raises the minimum radius ratio and does not look at the scaled Jacobian. On `00006600` (with the point search below) the final mesh had 388 cells under 0.1 with the default 6 flip passes, and 27 with `PLC_PASSES=0` (no reconnection). By number of boundary vertices (0 to 4): with flips `[3, 5, 13, 367, 0]`, without `[0, 0, 10, 11, 6]`. 367 of the 388 are cells with three boundary vertices and one sub-facet face, which the flips made.
3. **`00007062` spends its cap on boundary splits.** From the round and `refine why:` lines of the same runs: of the Steiner points the stage inserts on `00007062`, 54% are quality points (27.4k) and 46% boundary splits forced by queued encroachments (23.8k, mostly segment splits), 0.87 boundary splits per quality point against 0.29 on `00006600`. The queue of encroaching centres stays at about 250-300 per round to the end on `00007062`, while on `00006600` the point search drains it from about 300 to about 6. The bad-cell count was still falling when the cap stopped the stage (67,369 to 61,352 over the last 5 rounds); refusals at the sliver floor climb late (9 to 162 per round). A quality point removes about 3.3 bad cells on `00007062` (3.0 on `00006600`). Making the cap count quality points only would relax a budget, so it was not done.
4. **The encroached segments on `00007062` are ordinary boundary splits, not a cascade at the floor.** The segments its centres encroach are mostly at least 16 times `optional_floor` long, and many are input segments. The input has many short, skinny edges (3,321 edges shorter than 5 times its shortest edge, against 22 on `00006600`) and no acute angles.
5. **The 19 cells left on `00006600` come from cap triangles in the input shell.** `plc-run` with the new `PLC_CELLS=1` lists them: every one has a short edge of 0.08 to 0.10 times its longest (about 4e-5 mm), between two vertices on segments, and they sit in five clusters. The graded shell (90,000 triangles) has only nine triangles with an angle under 10 degrees; four of them are caps with angles 4.73, 4.73 and 170.5 degrees (for example triangle 85820, edges 1.97e-3, 1.97e-3 and 3.92e-3 mm), at exactly the four clusters near x = +-8.485, and an 8.55 / 162.9 degree cap lies 0.02 mm from the fifth cluster. Segments meeting at 4.73 degrees, split at equal distances d from the shared vertex (concentric shells), leave splits 2 d sin(4.73 / 2) = 0.0825 d apart: the observed ratio. A cell on such a sub-facet keeps that angle at a corner, which bounds its scaled Jacobian; neither refinement nor flips can raise a sub-facet's angle. The centres of these cells encroach sub-segments already below the sliver floor (0.75 of `optional_floor`, the shortest input segment, 6.47e-4 mm), so the stage refuses them. Removing the caps belongs to the surface stage (the graded chart triangulation), not to this stage; it is not done here.

## Change

- **Point search for slivers** (`State::better_point`, `picking_points`). For a cell under the bound, the stage previews the cells that inserting its circumcentre would create, and the cells for 14 offsets (the 6 axis and 8 diagonal directions) at 0.3 x min(circumradius, shortest edge), kept only when strictly inside the cell's circumsphere so the cell is still removed (the picking region of Li's sliver removal). Among the candidates that encroach no segment and no sub-facet, it takes the one whose new cells have the best worst scaled Jacobian (the first one on ties), if that beats the centre's. Otherwise the centre stays, with its usual handling (insertion, or queued boundary splits, or refusal). When the centre's preview fails, or the centre's new cells already reach the bound, the centre is used without searching.
- **`Delaunay3::preview`**: the tetrahedra an insertion would create, without changing the mesh. The cavity search was extracted from `try_insert` into `cavity()`, which both use; the insertion cell budget check moved into `cavity()`, so a preview honours it too.
- **`State::facets_encroached_by`**: the sub-facet circumball test, extracted from the inline loop in `refine_quality` so the search and the stage share it.
- **Reconnection by scaled Jacobian.** When `min_scaled_jacobian` is set, flips are judged by the scaled Jacobian alone: `extract` calls `tet_flip::improve_by` instead of `improve`, and the smoothing path calls `optimize_by` instead of `optimize`. Pure scaled Jacobian was chosen over min(scaled Jacobian, radius ratio) because it left fewer cells under 0.1 on both files (table below). The temporary `PLC_FLIPQ_MIN` switch used to compare the two was removed.
- **Speed, without changing the sequence of insertions.**
  - A facet ball index (`FacetIndex`, over a generic `BallGrid`) replaces the linear scan of sub-facet circumballs, and `unit_det` / `tet_scaled_jacobian` no longer allocate.
  - Point location starts from the sliver's own cell: `Delaunay3` takes a walk hint (`insert_near`, `preview(p, near)`, `ids()`).
  - `outside_cells` hashes only faces whose three vertices all lie on sub-facets, and marks blocked faces once, so its flood fill does no hashing; it seeds directly from the cell loop without the per-key owners map.
  - The cavity search uses `sphere_side_positive` (the insphere test alone), since every `Delaunay3` cell is positively oriented by construction.
  - Per-round classification is incremental (`State::cell_cache`, `State::bad_cells`): the inside flags and badness keys are kept per Delaunay id, and only the cells created since the previous round (`Delaunay3::take_created`) are classified, by the same flood as `outside_cells`.
  - The missing sub-segments and sub-facets are kept too (`State::missing`, `State::missing_items`): `Presence::count` logs the edges and faces whose count goes to or from zero, `split_segment` logs segments, and changed facet meshes are logged where the facet index is marked stale (`State::mesh_changed`).
  - `PLC_VERIFY_KEPT=1` asserts, every round and every presence check, that the kept results equal a full recompute. It passed on `00006600` (to convergence) and on `00007062` (35 refine rounds, to the point cap, `PLC_QSEC=300`).
  - The point search tests encroachment first and previews only the candidates that encroach nothing; a candidate's preview stops once its worst new cell is no better than the best so far (the first candidate with the strictly best value wins, as before).
  - Each facet mesh of at least 32 sub-facets keeps a grid of its sub-facet balls (`FacetMesh::ball_grid`, a `BallGrid` rebuilt in `ensure_spheres`). A query scanned about 8,000 balls, 99.9% of them in the 11 largest merged patches (up to 4,480 sub-facets).
  - The rollback copy taken during the sliver stage (`State::rollback_copy`) leaves out the presence table, the missing items and the cell classification, which rebuild themselves when absent; only `extract` reads the copy.
- **A facet flip drops the kept cell classification.** `drive`'s `flip_toward` re-triangulates sub-facets without creating cells, so the next round classifies every cell from scratch (found in review; `PLC_VERIFY_KEPT` would only catch it in a run with a flip).
- **Diagnostics.** `refine_quality` prints, with `STAGEDBG` only, a `refine why:` line per round (refusal reasons, points placed, bad cells they created, queued splits). `plc-run` gained `PLC_PASSES` (flip passes, 0 for none), `PLC_SMOOTH` (smoothing rounds, as `abc-corpus`'s `ABC_SMOOTH`) and `PLC_CELLS` (each cell under 0.1 with its vertex kinds, edge lengths and corner heights), and prints the cells under 0.1 by number of boundary vertices and of sub-facet faces.

## Results

New lib test `plc_conform::tests::the_sliver_point_search_takes_a_better_point_than_the_centre`: a unit box whose faces are 6 x 6 grids with 250 seeded interior points; for every sliver inside the domain whose search moves away from the centre, the point is strictly inside the circumsphere, encroaches no segment or sub-facet, and its previewed worst new cell is strictly better than the centre's; at least one sliver moves.

First version of the search (before the skip when the centre is already good), `PLC_QSEC=900`:

| | `00006600` Phase 159 | `00006600` point search | `00007062` point search |
|---|---|---|---|
| How the stage ends | point cap | converges (no cap) | point cap |
| Quality Steiner points | 35,714 | 27,858 | 27,434 |
| Cells under 0.2 left at stage end | 10,670 | 268 | 60,560 |
| Stage time | about 61 s | about 150 s | 482 s run |

Flip metric, `PLC_QSEC=900`, cells under scaled Jacobian 0.1 in the final mesh:

| | radius ratio (old) | no flips | scaled Jacobian (chosen) | min(sj, rr) |
|---|---|---|---|---|
| `00006600` cells under 0.1 | 388 | 27 | 19 | 19 |
| `00006600` by boundary vertices (0 to 4) | `[3, 5, 13, 367, 0]` | `[0, 0, 10, 11, 6]` | `[0, 0, 9, 10, 0]` | not recorded |
| `00007062` cells under 0.1 | 4,091 | not run | 833 | 845 |
| `00007062` by boundary vertices (0 to 4) | `[455, 269, 579, 2759, 29]` | not run | `[9, 29, 167, 614, 14]` | not recorded |

With the final code and unlimited time (`PLC_QSEC=900`):

| | `00006600` | `00007062` |
|---|---|---|
| How the stage ends | converges | point cap |
| Quality Steiner points | 28,259 | 27,417 |
| Rounds | 510 (89 refine rounds) | not recorded |
| Cells left unresolved at stage end | 69 | 60,542 |
| Cells under 0.1 in the final mesh | 19 | not recorded with the final code |

Stage time on `00006600`, `PLC_QSEC=900`, measured from the first to the last `refine round` line with nothing else on the CPU. Every step gives the identical sequence: 28,259 quality points, 510 rounds, cells under 0.1 by boundary vertices `[0, 0, 9, 10, 0]`.

| Step | Stage time |
|---|---|
| Baseline (point search and scaled-Jacobian flips, with the facet ball index and allocation-free `unit_det` / `tet_scaled_jacobian`) | 72 s |
| Point location from the sliver's own cell | 67 s |
| `outside_cells`: hash only all-sub-facet faces, mark blocked faces once | 53 s |
| `outside_cells`: seed from the cell loop, no owners map | 47 s |
| `sphere_side_positive` in the cavity search | 43.5 s |
| Incremental per-round classification | 38.3 s |
| Kept missing items | 26.6 s |

The later steps were timed with other processes on the CPU (load average 3 to 7 on 8 cores), so each comparison ran its two builds back to back, alternately, on the same shell:

| Step | Stage time |
|---|---|
| Point search stops a preview at the centre's value | 27.1 s (no faster than 26.6 s) |
| Encroachment first, then previews, stopping at the best so far | 24.2, 25.8, 23.8 s against 27.1, 26.4, 25.1 s |
| Ball grid per large facet mesh | 20.4 s |
| Rollback copy without the rebuildable caches | 18.9 s |

A profile of the stage at 47 s: `preview` 21%, `outside_cells` 18%, the presence check in `drive` (collecting missing sub-facets and segments) 9%, `insert` 6%, `facets_encroached_by` 6%, `State` clone and drop 3%. At about 24.6 s `facets_encroached_by` had grown to 33% (every candidate is now tested); at 20.4 s it was 10%, and `preview` 17%, `insert_near` 15%, `Presence::count` 9% and `State` clone and drop 10% led.

ABC gate at the default 20 s sliver clock, `ABC_GRADED_FIRST=1 cargo run --release --locked --offline -j1 -p solid-kernel --no-default-features --example abc-corpus -- --one <file> --foreign --graded --steiner 200000 --conform-timeout 300 --merge --quality --refine`, run serially with nothing else on the CPU. The "before speed work" column was taken before the speed work above; the "final" columns are with the final code.

| | `00006600` Phase 159 | `00006600` Phase 160, before speed work | `00006600` Phase 160 final | `00007062` Phase 159 | `00007062` Phase 160 final |
|---|---|---|---|---|---|
| Quality Steiner points kept | 12,455 | 6,727 (clock rollback after 8,053) | 28,259 (stage converges inside the clock) | 3,173 | 2,076 (clock rollback after 3,161) |
| Tetrahedra | 204,584 | 161,296 | 315,048 | 177,420 | 167,352 |
| Cells under scaled Jacobian 0.1 | 3,380 | 3,117 | 19 | 85,052 | 87,029 |
| of which all four vertices on the boundary | 66 | 343 | 0 | 9,333 | 17,944 |
| Minimum scaled Jacobian | 0.023 | 0.0161 | 0.0621 | 0.0004 | 0.0022 |
| Largest edge ratio | 21.2 | 16.3 | 13.6 | 164.1 | 67.3 |
| Solver gate | fails | fails | fails (scaled Jacobian only) | fails | fails |

The "final" `00006600` run is marginal. Its stage needs about 19 s to converge on an idle CPU, against the 20 s clock. Rerun with the final code (including the review fix below) at a load average of about 2.5 from desktop processes, the clock fired after 28,143 quality points and the stage rolled back to its last checkpoint at 25,659: 295,115 tetrahedra, 28 cells under 0.1 (none with all four vertices on the boundary), minimum scaled Jacobian 0.0212, edge ratio 13.6, gate fails on the scaled Jacobian only. The same result came from three runs, and from the code before the review fix too, so it is the clock and not the fix. `plc-run` with `PLC_QSEC=900` on the dumped shell still gives the identical sequence with the review fix (28,259 quality points, 510 rounds, `[0, 0, 9, 10, 0]`).

Before the speed work, the 20 s clock reached fewer quality points on `00006600` than Phase 159 did, because each point now costs a search; the cells under 0.1 fell only slightly (3,380 to 3,117), the minimum scaled Jacobian fell, and the edge ratio improved.

`cargo test -p solid-kernel -p fea-core --release --locked --offline -j1 --no-fail-fast` on macOS arm64: 924 passed, 0 failed, 41 ignored. That run compiled before the review fix below; `cargo test -p solid-kernel` rerun with it: 303 passed, 0 failed, 4 ignored (fea-core is unchanged by it). The run also caught a wrong test: the new `sphere_side_positive` check in `tet_topology` used a negatively oriented cell; the test now uses the positive one, and the function is unchanged. The `plate` sliver test (`sliver_refinement_removes_most_cells_under_the_solver_gate_and_terminates`) uses `smooth_rounds 5` with `min_scaled_jacobian` set, so `extract` now calls `tet_smooth::optimize_by` with the scaled Jacobian: both its flips and its smoothing judge cells by the scaled Jacobian, not by radius ratio. Its result is part of the suite run above.

Naming note: on this path `FlipOptions::target` (default 0.4) and the `FlipReport` fields `min_radius_ratio_*` and `cells_below_target_*` are in scaled-Jacobian units, despite their names. This changes nothing numerically; doc comments in the code say so.

Hosted checks: the `Workbench qualification` result for this PR is recorded in the issue-46 pointer (`docs/issue-46-resume.md`, `docs/issue-46-current.json`) when the PR is merged.

## Negative results

- Scoring the outside cones of a candidate was rejected: a candidate that encroaches no sub-facet cannot have a cavity that crosses one, so its new cells are all on one side of the boundary.
- Correction: testing encroachment before the preview was first rejected here (the sub-facet ball scan was the stage's hot spot in the Phase 159 profile), and a later timing (31.2 s against 26.6 s) seemed to confirm it, but that timing ran beside another process at full CPU. Timed alternately, it is faster (above) and it is kept.
- Stopping a preview's scan once its worst new cell reaches the centre's value made no measurable difference on its own (27.1 s against 26.6 s); it is kept because the encroachment-first search uses the same stop with the best value so far.
- The point search alone does not bring `00007062` under its cap: its boundary splits, not its quality points, use most of the budget.
- A wider or looser candidate rule would barely help `00007062`: in each round only 0 to 15 of the roughly 150 to 340 queued slivers had any non-encroaching picking point. Not tried further.
- min(scaled Jacobian, radius ratio) as the flip metric: no better than pure scaled Jacobian on `00006600` (19 against 19) and slightly worse on `00007062` (845 against 833), so it was dropped with its switch.

## Limits / not shown

- Neither file passes the default solver gate with the final code, so neither is solved. `00006600` converges inside the 20 s clock on an idle CPU only (a loaded CPU rolls it back to 25,659 points and 28 cells under 0.1, above) and fails on the scaled Jacobian only (19 cells under 0.1, minimum 0.0621; edge ratio 13.6 is within the limit); those cells come from the cap triangles of its graded shell (diagnosis item 5). `00007062` fails on both measures (87,029 cells under 0.1, edge ratio 67.3): its 20 s clock rolls back after 3,161 quality points, and the stage is still far from the gate there.
- The 20 s sliver clock and the 50,000 point cap (`max_quality_steiner`) are unchanged. `PLC_QSEC`, `ABC_QSEC` and `PLC_SECS` are diagnostics; numbers taken with them show what the method reaches given time, not what the default run delivers.
- When the 20 s clock ends the stage, the log says "Steiner point budget exceeded": `quality_overdue` reuses `BUDGET_MESSAGE`. A reader of the log cannot tell the clock from the point cap by that message; it was not changed here.
- The search uses a fixed set of 14 offsets; no optimisation over the picking region, and no vertex moves.
- The cap triangles of the graded `00006600` shell (diagnosis item 5) are not removed: that is surface-stage work.
- The numbers are from one machine (macOS arm64). No Windows evidence, no 300-file ABC rerun, and no solve of either file are shown here.
