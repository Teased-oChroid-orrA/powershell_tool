# Phase 161: leave a boundary diagonal unsplit when its split carries no geometry

Issue #63, excluding NEMA17 by owner request. The ABC files are the Phase 156-160 pair (`00006600`, `00007062`); they stay in temporary local storage only. The goal is unchanged: both files through the default solver gate (scaled Jacobian at least 0.1, edge ratio at most 50) and solved, with no gate, tolerance, budget or timeout relaxed. This phase works on the surface stage, the graded chart triangulation, which Phase 160 diagnosis item 5 named as the source of the cells left on `00006600`. Branch `phase-161-boundary-diagonal`, based on `main` at `4074bf2` (Phase 160 merged as PR #204, squash `46d2968`; pointer PR #205).

## Diagnosis

Items 1-5 were found with temporary logging in `chart_mesh.rs`, removed before the change.

1. **Where the caps are.** Phase 160 left 19 cells under scaled Jacobian 0.1 on `00006600`, from cap triangles in the graded shell. The caps sit on faces 647 and 965, cones with a 3 degree half-angle. Their chart is unrolled, so chart distances equal world distances.
2. **The corner.** Corner V (chart index 291) is a 90 degree chart corner. Its boundary neighbours are sample 290, 6.47e-4 mm away along the circle edge, and sample 292, 7.81e-3 mm away along the generator.
3. **How the cap is made.** `shape_chart`'s chord loop always split a curved "boundary diagonal" (an interior edge between two boundary samples) at its chart midpoint, whatever its sagitta. The midpoint of the diagonal 290-292 lands 3.2e-4 mm off the long side 291-292, which gives a triangle with angles 4.73, 4.73 and 170.5 degrees. atan(6.47e-4 / 7.81e-3) = 4.73 degrees, the angle observed in Phase 160. The diagonal's sagitta is of the order of 1e-8 mm: the split point lies on the chord to far below the model's geometry tolerance (0.005 mm for `--foreign`), so it adds a vertex and no geometry.
4. **Grading does not remove it.** `chart_mesh::boundary_splits` never splits a segment shorter than 2 x `floor_mm` (0.2 mm), so the side 291-292 stays unsplit and the cap stays.
5. **Why the forced split exists.** It was added with the chart mesher (commit `0932371`) for a shallow cylindrical strip whose four corners all lie on the neighbouring plane: there the unsplit internal chord would overlap that face.
6. **The forced split also keeps curved faces accurate.** The first version of this change (from the previous session) forced the split only when both ends of the diagonal lie on one other face. It failed three `solid-kernel` tests: the native cylinder's volume was off by 1.3e-3 of the exact value (bound 1e-3, `native_brep::a_cylinder_and_a_box_match_closed_form`), the misaligned-circle band's volume by 5.5e-3 (bound 2e-3), and the bracket PLC's segment count changed. So the split matters wherever its point moves the surface by more than the tolerance, not only where a chord can overlap a plane.
7. **Grading reads the forced splits.** `boundary_splits` runs the same chord loop before its refinement (with boundary segments allowed to split), so the interior points the forced splits add steer which edge samples grading adds. Sparing them there too (variant A below) coarsened the graded edges of `00007062`.

## Change

- **`AnalyticModel::forced_diagonals`**: the rule for which boundary diagonals of a face's chart are split whatever their sagitta. A diagonal is spared only when both hold: its split point lies within the model's geometry tolerance of the chord (the split carries no geometry), and it cannot overlap a neighbouring face (its two ends share no other face, and neither end lies, within the tolerance, on a plane of a face the other end touches). A seam's samples count the face itself as a neighbour, so a chord between two of them is always split. The neighbour sets are computed on the first diagonal within the tolerance, so a face without one never scans the body (the review of the first version had noted that a per-call scan of every face's bounds makes a model's triangulation quadratic in its face count).
- **`shape_chart` / `triangulate`** take the rule as `forced(a, b, sagitta)`. The chord loop now evaluates the split point (and its sagitta) before the chord tests; for a cylinder that is one more surface evaluation per interior edge and pass, with the same tests after it.
- **Only the final chart triangulation uses the rule.** `boundary_splits` (grading) passes a rule that forces every boundary diagonal, as before, so the graded edge samples are unchanged by this phase.
- **Regression tests** (`chart_mesh::tests`): `a_boundary_diagonal_left_unsplit_leaves_no_cap` reduces the `00006600` corner to a 6.47e-4 x 7.81e-3 rectangle on a cylinder: forcing the split gives 5 vertices and a corner angle above 170 degrees; the rule gives 4 vertices and no angle above 90 degrees. `a_boundary_diagonal_is_spared_only_within_tolerance_and_off_shared_faces` reads a cylinder at foreign tolerance: a chord between two samples of one circle (both on that end's plane) and a chord between the seam's ends are forced whatever their sagitta; a chord across the band is spared at a sagitta equal to the tolerance and split at twice it. The second test fails on the first version (item 6), which did not split seam chords.
- **Bracket snapshot**: `validated_plc_preserves_every_bracket_constraint_and_source_face` counted 1,467 PLC segments and 978 facets; with the change the bracket's shell (read at the default tolerance of 1e-7 mm) has 12 fewer void splits, 1,431 segments and 954 facets. Its real assertions are unchanged and pass (every facet edge is a segment, `check_intersections` accepts the shell, every facet maps to its source face).

## Results

`00006600`, `ABC_QSEC=900` (a diagnostic: the method's result given time, not the default run). macOS arm64, release, serial, nothing else heavy on the CPU (desktop load average about 2.5).

| | `main` (Phase 160 doc) | variant A (rule in grading too) | Phase 161 (rule in the final chart only) |
|---|---|---|---|
| Cells under scaled Jacobian 0.1 | 19 | 6 | 6 |
| Minimum scaled Jacobian | 0.0621 | 0.0481 | 0.0481 |
| Largest edge ratio | 13.6 | 12.6 | 12.6 |
| Quality Steiner points | 28,259 | 27,425 | 28,055 |
| Tetrahedra | 315,048 | 305,490 | 312,245 |

The 6 cells are the same in both variants (`plc-run` with `PLC_CELLS=1 PLC_QSEC=900 PLC_SECS=900` on the dumped shell). They sit in two clusters at x = +8.4853 (z 9.74) and x = -8.4853 (z -7.23), the corners of the cone faces, and each has a short edge 0.08 of its longest: the ratio of the corner's own sides, 6.47e-4 / 7.81e-3 = 0.083. Their bad corner is the 4.73 degree corner of the boundary itself, not a split point; no cell is left at the other two clusters of Phase 160. Removing them needs a graded generator sample near the corner, which `boundary_splits` refuses below 2 x `floor_mm`; that is not done here.

ABC gate at the default 20 s sliver clock, the Phase 160 `abc-corpus` command, run serially, with `main` (built from `4074bf2` on the same machine in the same hour) beside it:

| | `00006600` main | `00006600` Phase 161 | `00007062` main | `00007062` variant A | `00007062` Phase 161 |
|---|---|---|---|---|---|
| Quality Steiner points kept | 25,659 (clock rollback) | 25,986 (clock) | 3,161 | 4,341 | 4,413 |
| Tetrahedra | 295,115 | 296,621 | 178,242 | 174,111 | 179,692 |
| Cells under scaled Jacobian 0.1 | 28 | 12 | 79,179 | 56,128 | 58,843 |
| of which all four vertices on the boundary | 0 | 0 | 12,423 | 5,600 | 5,702 |
| Minimum scaled Jacobian | 0.0212 | 0.0481 | 0.0007 | 0.0005 | 0.0006 |
| Largest edge ratio | 13.6 | 12.6 | 164.1 | 267.2 | 61.3 |
| Smallest shell angle (degrees) | 4.733 | 4.734 | 2.904 | 1.239 | 2.280 |
| Solver gate | fails | fails | fails | fails | fails |

The ABC runs used the final rule before its neighbour lookup was made lazy (the same rule, computed on demand); the suites and the parity table ran on the final code.

At the default clock `00006600` does not converge with either build under this load: the stage needs about 28,000 quality points, and the clock ends it near 26,000 (`main` rolled back to the same 25,659 points and 28 cells that Phase 160 recorded under load). Variant A converged in the clock (6 cells) because its stage needs fewer points; that run was not repeated. The `00007062` numbers are also clock-limited (the stage is far from converging), so they move with the number of points the clock allows; the comparison shows the direction, not a settled value.

Variant A was dropped because of `00007062`: its edge ratio and smallest shell angle got worse than `main`. Phase 161 improves the cells under 0.1 and the edge ratio on both files against `main`; its smallest shell angle on `00007062` is still below `main`'s (2.280 against 2.904 degrees), from the final chart triangulation, since the graded edges are now the same as `main`'s. Where that angle sits was not located.

`cargo test -p solid-kernel -p fea-core --release --locked --offline -j1 --no-fail-fast` on macOS arm64: 926 passed, 0 failed, 41 ignored, no warnings (Phase 160's 924 plus the two new tests).

Mesh-only parity table (`CASE='' NO_SOLVE=1 cargo test -p fea-problem --features occt --release --locked --offline -j1 --test occt_vs_native -- --ignored --nocapture table`), run with the change and then with `main`'s `chart_mesh.rs` on the same tree: 36 of 38 rows pass the gates on both; the two failures are NEMA17, deferred by owner request. 33 rows are identical (ignoring mesh time). Four native rows differ slightly and still pass: the filleted box (93,762 to 93,753 tetrahedra, minimum scaled Jacobian 0.110 to 0.111), the cylinder with a filleted top (51,823 to 51,806, 0.101 both), the filleted cross-bore (90,953 to 91,014, 0.103 both) and the t-junction (20,588 to 20,584, 0.124 both). The native NEMA17 row goes from 3 to 4 cells under 0.1 (it fails on both). No solves ran.

Hosted checks: the `Workbench qualification` result for this PR is recorded in the issue-46 pointer (`docs/issue-46-resume.md`, `docs/issue-46-current.json`) when the PR is merged.

## Negative results

- A post-pass of Lawson flips in `triangulate`, judged by the 3D angle, left `00006600` at 19 cells: every flip at the caps was refused (chart orientation, no gain, or a constraint edge). It was removed.
- A Python prototype that flipped coplanar pairs on the PLC dump reached 0 cells, but only by flipping across CAD face boundaries, which a per-face chart cannot do.
- Skipping the forced split on faces 647 and 965 only gave 7 cells under 0.1. It is specific to this model, so it was not kept.
- Skipping every forced split broke the PLC (edge ratio 1405).
- Forcing the split only where both ends share a neighbouring face (the first version) gave 6 cells and 121,563 tetrahedra at `ABC_QSEC=900` on `00006600`, but failed the three tests of diagnosis item 6 and left seam chords unsplit. A read-only review also found that it would leave a chord unsplit between two coplanar faces split by an imprinted edge (one end on each); the plane test in the final rule covers that case.
- Variant A (the rule in grading as well): see the table; dropped for the `00007062` regression.

## Limits / not shown

- Neither file passes the default solver gate, so neither is solved. `00006600` keeps 6 cells under 0.1 given time (12 at the default clock under desktop load) at the cone corners; `00007062` is far from the gate.
- The 20 s sliver clock, the 50,000 point cap and the gate thresholds are unchanged. `ABC_QSEC`, `PLC_QSEC` and `PLC_SECS` are diagnostics.
- The overlap test covers neighbouring planes by geometry and every neighbouring face by topology. A chord whose ends lie on two different curved faces that coincide (a curved face split in two by an imprinted edge) is covered only if its sagitta is above the tolerance; such a face pair is not handled.
- `forced_diagonals` scans the body's faces and edges for a face only when that face has a boundary diagonal within the tolerance (found lazily on the first one), so the scan runs for a few faces, not every face. Its cost was not measured on a large model.
- The numbers are from one machine (macOS arm64). No Windows evidence and no 300-file ABC rerun are shown here.
