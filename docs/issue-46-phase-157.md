# Phase 157: keep graded edge samples a geometry tolerance apart

Issue #63, excluding NEMA17 by owner request. The two ABC v00 files are the ones Phase 156 used (`00006600_2516c959a43348f89c02cae9_step_000.step`, `00007062_a7b44768091a464bb7beff7f_step_000.step`, archive MD5 `695388be7a278c7798c8c8ae239772ac`). They stay in temporary local storage only; the [ABC publisher](https://deep-geometry.github.io/abc-dataset/) says CAD copyright belongs to the creators.

## Diagnosis

After Phase 156, `00006600` grades and samples, but the 327-face shell had 133,509 vertices, 125,894 triangles with a corner under 18 degrees, and a minimum shell angle of `0.0855°`. Face #537 is a planar annulus 0.477 mm wide (r 16.623 to 17.1 mm) between the 0.15 mm torus fillet bands #937 and #831; all their bounds are circles. About 57,000 adjacent samples on those circles were under 1e-5 mm apart (30,650 at r = 17.1, 26,199 at r = 16.623). PLC on that shell ran out of the default 400k cell budget, and a 20M-cell `plc-run` was cancelled at 120 s with 328 Steiner points.

Two mechanisms in `AnalyticModel::graded` produced those gaps. Faces sharing an edge segment ask for the same split from different charts, so their fractions differ slightly (on the filleted cross bore, 1.2e-5 of a 0.25 mm segment, 2.9e-6 mm). Both points were inserted, since `insert_edge_splits` only merged fractions within 1e-9. And a face's refinement near a small input angle runs a concentric-shell cascade towards the vertex (fractions 0.5, 0.75, 0.875, ... in one round). With two faces doing this, each cascade point appeared twice, down to 2e-8 mm apart. The `floor_mm` check only stops a segment shorter than twice the floor from splitting; it says nothing about how close the new point lands to another sample.

## Change

Grading keeps the chart's split fractions, and spline intervals are still bisected (Phase 156). A new sample is dropped if it would land nearer than the model's geometry tolerance to another sample of its segment: an existing end sample or a split already kept in that round. The edge and its faces only agree to within that tolerance, so closer samples carry no geometry, and their face-chart images can swap order (the Phase 156 overlap refusal). The rule is the model's own tolerance, not a new constant: 1e-7 mm by default, chord/10 for the foreign-CAD reading (0.005 mm at the 0.05 mm chord the ABC runs use), 1e-4 mm in the OCCT fillet tests. No existing sample is moved or merged.

`boundary_splits` now measures the floor on the world chord between the segment's two samples instead of the chart length. Sphere and torus charts are conformal, so chart lengths there are not millimetres (this answers the open question left by the Phase 157 candidate). The parity table below is identical with and without this change.

`abc-corpus` gained `ABC_NOFALLBACK`, which reports the protected recovery's own result instead of falling back to unprotected recovery (the same switch `plc-run` has as `PLC_NOFALLBACK`).

## Rejected candidate: bisect every split

The first Phase 157 candidate (`03492e9`, never merged) bisected every requested segment on the edge's curve. It cleared `00006600` (37,234 vertices, 139 triangles under 18 degrees, minimum `0.2431°`, no edge under 1e-5 mm). But the touched-crate suites failed twice: the bracket still had needles on 11.4% of its triangles (gate 10%), and the frustum's protected PLC no longer refused at the feature-size floor as `curved_step` expects. Bisection throws away the chart's concentric-shell placement, which is what grades the mesh towards small features. Two more variants also failed the bracket gate: clamping each fraction so both parts stay at least `floor_mm` long, then dropping splits nearer than the floor to each other (needles on 14.2%). Clamping without the drop passed the bracket but left 922 edges under 1e-5 mm on `00006600`. Any rule that keeps every part at least `floor_mm` long fails the bracket, because grading there must go below the floor around its 5 µm sliver faces. So the spacing bound has to sit at the model-tolerance scale.

## Re-baselined guard

`spline_surfaces::grading_and_quality_refinement_lift_the_filleted_shells_without_changing_the_body` required the filleted cross bore's graded shell to keep a minimum angle of 0.19 degrees. With this change it is `0.156°`, at (6.214, 4.200, 30), the end vertex of edge #1496 segment 13. That is the same intrinsic end-fillet corner the test comment names. Main reached 0.198 degrees there only by inserting both faces' copies of each cascade split, 2.9e-6 to 2e-8 mm apart under a 1e-4 mm tolerance. The owner decided to re-baseline the bound to 0.15 degrees; the evidence is in the test comment. No other gate, tolerance or budget changed.

## Results

New lib test `chart_grade::tests::grading_keeps_new_samples_a_geometry_tolerance_apart` grades four redistributable fillet/torus files with the foreign-CAD reading. It asserts that no new sample lies within half the geometry tolerance of its neighbour; the half allows for the chart fraction following the curve only to first order. With the spacing forced to zero the test fails (`cross-bore-filleted edge #1333: new samples 5.055e-4 mm apart (tolerance 0.005)`); with the rule it passes.

`cargo test -p solid-kernel -p fea-core --release --locked --offline -j1 --no-fail-fast` on macOS arm64: 919 passed, 0 failed, 41 ignored, no compiler warnings.

The mesh-only parity table (`CASE='' NO_SOLVE=1 cargo test -p fea-problem --features occt --release --locked --offline -j1 --test occt_vs_native -- --ignored --nocapture table`) passes 36 of 38 rows; the two failures are NEMA17, as before. All 38 rows are identical to the same table run on `main` `bb54f14`. Compared with `docs/issue-46-phase-149-parity.txt`, several fillet rows' minimum scaled Jacobian fell from about 0.12 to 0.101 - 0.11 (still above the 0.1 gate). Since `main` gives the same rows, that drift comes from Phases 150 - 156, not from this change; it leaves those rows with a thin margin.

ABC runs used `ABC_GRADED_FIRST=1 cargo run --release --locked --offline -j1 -p solid-kernel --no-default-features --example abc-corpus -- --one <file> --foreign --graded`, with `--conform-timeout 0` for the shell A/B and `--steiner 200000 --conform-timeout 300 --merge --quality --refine` for PLC and the solver gate.

| `00006600` | Phase 156 | Bisect-all (rejected) | Phase 157 |
|---|---|---|---|
| Vertices / triangles | 133,509 / 267,030 | 37,234 / 74,480 | 44,993 / 89,998 |
| Triangles with a corner < 18° | 125,894 | 139 | 70 |
| Minimum shell angle | 0.0855° | 0.2431° | 4.733° |
| Edges under 1e-5 mm | 67,036 | 0 | 0 |
| Volume (mm³) | 2159.687 | 2159.210 | 2159.200 |

OCCT tessellation gives 2159.396 mm³ at deflection 0.05 and 2162.651 at 0.001, so the volume changes are within the chord-tolerance class.

With `--quality --refine`, PLC on the Phase 157 shell **conforms** for the first time. The default run takes the unprotected fallback: 36 Steiner points, 120,209 tetrahedra, volume error 1.1e-13. With `ABC_NOFALLBACK=1` the protected recovery conforms too: 138,570 Steiner points, 502,430 tetrahedra, volume error 4.1e-14 (7 minutes for the whole run, grading included). A plain `plc-run` of the dumped shell (20M cells, no quality options) conforms in 236.6 s. Both meshes fail the unchanged default solver gate: minimum scaled Jacobian 0.000 (need 0.1), largest edge ratio 11,119 (fallback) or 19,556 (protected) against a limit of 50, and 50,820 or 391,856 bad cells, all touching the boundary. No solve ran.

| `00007062` | Phase 156 grading | Phase 157 |
|---|---|---|
| Shell vertices / triangles | 110,215 / 220,430 | 55,008 / 110,016 |
| Minimum shell angle | 0.0316° | 2.904° |
| Triangles with a corner < 18° | not recorded | 3,881 |
| Volume (mm³) | 15,642.890 | 15,642.860 |
| PLC | protected cancelled at 120 s (17,644 Steiner); unprotected refused at the feature-size floor | conforms on the fallback path: 4,961 Steiner points, 151,161 tetrahedra, volume error 7.5e-14 (about 4 minutes for the whole run, grading included) |
| Solver gate | not reached | fails: scaled Jacobian 0.000, edge ratio 301.6, 96,023 bad cells (25,885 touching the boundary) |

The `00007062` result answers the handoff's request for a different strategy than a longer timeout: its 0.0316° input angle was itself a grading artifact, and it is gone without relaxing the 18° input gate.

## Limits and next steps

Both ABC files now stop at the default solver gate, after a conforming mesh. Neither is solved. The next stage is sliver quality: the bad cells on `00006600` all touch the boundary, so they come from boundary sliver tetrahedra, not from the interior. Raising `max_steiner`, the quality time budget or the gate thresholds is not supported by these results. The isolated 300-file ABC diagnostic in `issue-46-abc-and-retirement-gates-plan.md` (Track A/B step 4) was not rerun: that file set is not on this machine. The parity table was mesh-only; no solves ran. Windows was not run. OCCT retirement remains unapproved.
