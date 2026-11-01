# Phase 162: a band seam drops the loop sample beside its end

Issue #63, excluding NEMA17 by owner request. The ABC files are the Phase 156-161 pair (`00006600`, `00007062`); they stay in temporary local storage only. The goal is unchanged: both files through the default solver gate (scaled Jacobian at least 0.1, edge ratio at most 50) and solved, with no gate, tolerance, budget or timeout relaxed. This phase takes projectmem issue #0005, the 6 cells Phase 161 left on `00006600` at its cone corners. Branch `phase-162-seam-sample`, based on `main` at `56b335e` (Phase 161 merged as PR #206, squash `d796d63`; pointer PR #207).

## Diagnosis

1. **The corner is in the read model, not in grading.** Phase 161 named the corner's sides, 6.47e-4 mm along the circle and 7.81e-3 mm along the generator, and assumed grading would have to split the generator side. `face-probe --foreign` (with `PROBE_EDGES`, both added in this phase) printed the edges of cone faces 647 and 965 as read, before grading: edge 19292, from vertex 12331 to the seam vertex 19291, has three samples, and its middle one lies 6.47e-4 mm from 19291 (x = -8.485928 against -8.485281). The model's geometry tolerance with `--foreign` is 0.005 mm.
2. **Where that sample comes from.** Onshape writes the cone as a band: one face bounded by two circles with no seam edge. `AnalyticModel::seam_unseamed_cylinder_bands` cuts it along a generator from the first loop's vertex to the second loop at the same angle. The second loop has no vertex there, so `vertex_of_loop_at_angle` inserts an exact point of the circle (`insert_edge_splits`) and splits the edge there (`split_edge_at_sample`). The circle (12331 to 12546, 0.66 mm, three samples) had its midpoint sample 6.47e-4 mm from the seam's angle. The split kept that sample, so one piece (19293) has two samples over 0.3295 mm and the other (19292) ends in a 6.47e-4 mm segment. That segment is shared by the cone and the neighbouring face 579, and the cone's chart corner at 19291 gets the 4.73 degree angle seen since Phase 160.
3. **Grading respects that spacing elsewhere.** Since Phase 157 grading never puts a new sample nearer than the geometry tolerance to another. The seam insertion had no such rule: its only snap was a parameter fraction below 1e-9.

## Change

- **`AnalyticModel::drop_samples_beside`** (`analytic_step.rs`): after the seam's end is inserted on a loop's edge, an interior sample of that edge next to it that lies nearer than the geometry tolerance is removed (with its spline parameter). The new sample is an exact point of the curve and takes the old one's place, so the edge keeps its sample count and the chord across the removed sample moves off the curve by less than the tolerance more than before. It runs only when the insertion actually added a sample (`insert_edge_splits` skips a point it cannot evaluate or one that repeats a sample). The cylinder/cone, torus and revolution band seams all end through `vertex_of_loop_at_angle`, so all three get the rule.
- **Regression test** `a_band_seam_drops_the_loop_sample_beside_its_end` (`tests/analytic_step.rs`): the frustum fixture with its cone's seam removed and its base circle turned so that an interior sample lies 0.002 mm round from the seam's angle. It checks that the band is seamed, that no edge has samples nearer than the tolerance, that the base circle keeps its sample count, and that the volume matches the seamed original. Without the change it fails with a 2.0e-3 mm pair on the split edge.
- **`face-probe`**: `--foreign` reads with `Options::foreign_cad(0.05)` as `abc-corpus --foreign` does (without it the probe could not read either ABC file), and `PROBE_EDGES=id,id` prints those edges' samples.
- **`solid-kernel/AGENTS.md`**: the seam-sample rule and its limit; two stale facts corrected (the foreign tolerance is chord / 10, not / 50; torus and revolution bands are seamed).

## Results

`00006600`, the Phase 161 `abc-corpus` command (`ABC_GRADED_FIRST=1 ... --foreign --graded --steiner 200000 --conform-timeout 300 --merge --quality --refine`). macOS arm64, release, serial.

| | Phase 161 (`main`) | Phase 162 |
|---|---|---|
| Cells under scaled Jacobian 0.1, `ABC_QSEC=900` | 6 | 1 |
| Cells under 0.1, default 20 s clock | 12 (desktop load) | 1 (load average about 2.5-3; two runs, identical) |
| Minimum scaled Jacobian | 0.0481 | 0.0773 |
| Largest edge ratio | 12.6 | 7.5 |
| Smallest shell angle (degrees) | 4.734 | 6.389 |
| Quality Steiner points | 28,055 (`ABC_QSEC=900`); 25,986 at the clock | 26,445 (converges inside the clock) |
| Tetrahedra | 312,245 (`ABC_QSEC=900`) | 298,152 |
| Shell | 44,529 vertices, 89,070 triangles | 44,415 vertices, 88,842 triangles |
| Solver gate | fails | fails |

The two cone corners are gone. The last cell (scaled Jacobian 0.0773, at (-4.075, -1.008, 15.590), `plc-run` vertex kinds `SIQS`: one input vertex, two on boundary segments and one interior quality point) is not at a corner: the shell triangles within 0.2 mm of it are identical to Phase 161's, where the sliver stage resolved that spot. So it comes from the sliver stage's insertion sequence, which changed with the shell elsewhere (projectmem #0003), not from the surface. It was not chased here.

A third default-clock run, made while a test build ran on the same machine, ended with 16,810 quality points and 112 cells: the stage needs about 26,000 points and the clock ends it early under load. That run is not counted.

`00007062` at the default clock: identical to Phase 161 (58,843 cells under 0.1, minimum scaled Jacobian 0.0006, edge ratio 61.3, smallest shell angle 2.280 degrees). None of its band seams lands beside a loop sample, so it is untouched.

`cargo test -p solid-kernel -p fea-core --release --locked --offline -j1 --no-fail-fast` on macOS arm64: 927 passed, 0 failed, 41 ignored, no warnings (Phase 161's 926 plus the new test).

Mesh-only parity table (`CASE='' NO_SOLVE=1 cargo test -p fea-problem --features occt --release --locked --offline -j1 --test occt_vs_native -- --ignored --nocapture table`): all 38 rows are identical to Phase 161's (ignoring mesh time); 36 pass the gates and the two failures are NEMA17, deferred by owner request. No fixture in the table has a band seam beside a loop sample. No solves ran.

Hosted checks: the `Workbench qualification` result for this PR is recorded in the issue-46 pointer (`docs/issue-46-resume.md`, `docs/issue-46-current.json`) when the PR is merged.

## Negative results

- **Taking the edge's end vertex as the seam's end.** The same gap can occur between the seam's target and an edge's end vertex, which cannot be dropped. Snapping the seam to that vertex when it is nearer than the tolerance (the angle checks of the band seams accept a seam end that far off) made the cone chart refuse the face ("cannot certify a single-turn chart of a cone/torus face") in the regression fixture turned by 0.002 mm at the vertex: the seam then leaves the generator. It was removed. That case still leaves a piece shorter than the tolerance; it is not observed on the two ABC files.

## Limits / not shown

- Neither file passes the default solver gate, so neither is solved. `00006600` has 1 cell under 0.1, from the sliver stage; `00007062` is far from the gate.
- The 20 s sliver clock, the 50,000 point cap and the gate thresholds are unchanged. `ABC_QSEC`, `PLC_QSEC` and `PLC_SECS` are diagnostics.
- An end vertex within the tolerance of the seam's target is not handled (negative result above).
- A read-only audit of the other sample-insertion paths found no guard against a sub-tolerance pair where a torus or revolution seam's two ends nearly coincide, or where `sample_spline_range` forces a one-step segment across a tiny knot span. These were found by reading only, not reproduced, and are not changed.
- The numbers are from one machine (macOS arm64). No Windows evidence and no 300-file ABC rerun are shown here.
