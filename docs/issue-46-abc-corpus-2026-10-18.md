# Issue 46 — Phase 114: ABC corpus survey of the native STEP pipeline (OCCT-retirement criterion 1)

Goal (owner, 2026-10-18): zero dependence on OpenCASCADE. `docs/issue-46-occt-retirement-readiness.md` criterion 1 needs a measured success rate of the native read -> sample -> conform pipeline on many real STEP files; before this phase there were two (both failed). This phase adds the survey harness and the first measurement. It measures read/mesh *robustness*, not accuracy: accuracy still comes from the closed-form and NAFEMS tests.

## Data and licence handling

- Source: the ABC dataset (https://deep-geometry.github.io/abc-dataset/), STEP chunk `abc_0000_step_v00.7z` (10 000 models, 1.5 GB). Sample: every 33rd file of the chunk, 300 files.
- The dataset page states: "The copyright of the CAD models is owned by their creators" and refers to Onshape Terms of Use 1.g.ii. It does not say whether redistribution is permitted. **No model is stored in the repository**: the files live outside it, and this document carries aggregate counts only. Local evaluation is unaffected.

## Harness

`solid-kernel/examples/abc-corpus.rs`: `cargo run --release -p solid-kernel --no-default-features --example abc-corpus -- DIR OUT.csv [--limit N] [--timeout S] [--conform-timeout S]`. One child process per file (a panic, abort or hang costs one row). Stages: `inventory` -> `model` (`AnalyticModel::read`) -> `sample` (`sampled_surface`, plain per-face sampling, no grading) -> `shell` (`plc_conform::conform`) -> `conform` (success: conforming tetrahedralization whose divergence-theorem volume matched). The CSV records the stage reached, the refusal text, the shell's minimum angle, and timings.

## Result

| stage reached (300 files) | before the fix | after the fix |
|---|---|---|
| refused at STEP inventory | 6 | 6 |
| refused reading the model | 267 | 188 |
| refused sampling the surface | 7 | 86 |
| conform failed | 9 | 9 |
| **conforming tetrahedralization** | **11 (3.7 %)** | **11 (3.7 %)** |

Failure classes at the model stage, before the fix: multiple `FACE_OUTER_BOUND` on one face 136 (45 % of all files), more or fewer than one `MANIFOLD_SOLID_BREP` 102, unsupported `EDGE_LOOP` encoding 9, edge/surface disagreement 9, edge off a spline surface 7, spline edge not covering its curve 3, edge endpoint off its line 1.

### Fix in this phase

Onshape writes the *hole* loops of a face as `FACE_OUTER_BOUND` as well. `step_topology` refused every such face ("multiple explicit outer bounds") although the loop roles are decided geometrically anyway (`planar_step::classify`, order and winding do not matter). The count check is removed; the declared-vs-geometric consistency check in `planar_step` now applies only when exactly one bound is declared outer. Regression test `hole_loops_declared_as_outer_bounds_read_like_face_bounds` (relabels every hole loop of the edit-plate fixture; it failed with the exact production message before the change). Effect: 79 files move from `model` to `sample`; none new reaches `conform` yet, because they hit the next classes below.

### Classes after the fix (rank by frequency; next work)

| class | files | detail |
|---|---|---|
| not exactly one `MANIFOLD_SOLID_BREP` (101 files have 2 to 23 bodies, 1 has none) | 102 | multi-body parts: needs a body-per-region meshing design |
| edge/surface geometry disagrees by N mm | 36 | tolerance question (never loosen to pass): decide per file whether the CAD is inexact or the reader is |
| unsupported `EDGE_LOOP` encoding | 19 | reader gap |
| cylindrical boundary winds without a chart-closing seam | 48 | at `sample` |
| surface chart work budget exceeded | 27 | at `sample` |
| spline edge must cover its whole curve | 13 | |
| holes / unseamed bands on cones and tori | 10 | |

### Registry angle gate (benchmarks D.4)

Among the 11 files that conformed, the plain shell's minimum angle was 0.23, 0.49, 1.17, 1.23, 1.27, 3.37, 4.57, 8.26, 10.43, 12.99 and 45.0 degrees: `plc_conform` terminated on 10 shells below the registry's 18 degree gate (mean 1.1 s). Of the 9 that failed in `conform`, 8 hit "Steiner point budget exceeded" (minimum angles 0.23 to 3.29 degrees) and 1 the feature-size floor. So the gate refuses shells the conformer can mesh; whether its removal is safe needs a tetrahedral quality check on these 10, which this phase does not do. The gate is unchanged.

## Progress within the phase (same 300 files)

Reader changes, in order, with the plain-sampling result after each (`--foreign` from step 5; the strict default is kept):

| step | reader change | conforming | refused reading the model | refused sampling | conform failed |
|---|---|---|---|---|---|
| 0 | baseline | 11 (3.7 %) | 267 | 7 | 9 |
| 1 | several `FACE_OUTER_BOUND` per face allowed | 11 | 188 | 86 | 9 |
| 2 | multi-body documents read as one model per solid (`AnalyticModel::read_all`) | 16 | 141 | 116 | 20 |
| 3 | unseamed cylinder bands (the through-hole wall) get a synthesized seam; harness uses `CanonicalizeWithinTolerance` | 37 | 141 | 77 | 38 |
| 4 | pole `VERTEX_LOOP` bounds on spheres and cones skipped; cone bands; a vertex is inserted on the second circle when the two circles start at different angles (edge split in every face that uses it) | **47 (15.7 %)** | 139 | 22 | 85 (1 timeout) |
| 5 | step 4 read with `Options::foreign_cad` (edge/surface agreement to chord / 50 = 1e-3 mm) | 47 | 83 | 53 | 103 (8 timeouts) |

| 6 | step 5 with the conform budget raised from 4000 to 200 000 Steiner points (harness `--steiner`), 100 s per shell | **70 (23.3 %)** | 83 | 54 | 77 (56 cancelled at 100 s, 21 feature-size floor; 10 timeouts at 150 s) |

Step 6 changes a resource limit, not a tolerance: `conform` still verifies every result by exact set membership and the divergence-theorem volume. The 70 meshes have a relative volume error of at most 1.7e-13 (median 5e-15); they took a median 3.0 s, 90th percentile 37.8 s, at most 97.4 s, with a median 3 169 Steiner points (90th percentile 9 884, most 25 598). The 23 files gained over step 5 were all refused only for the 4000-point budget. Mesh quality (radius-edge ratio, slivers) was not measured: only that the shell is conformed.

| 7 | step 6 with an incremental sub-facet encroachment refresh in `plc_conform` (profiled: `refresh_mesh` was 63 % of the time, O(sub-facets x vertices) after every insertion) | **83 (27.7 %)** | 83 | 55 | 62 (33 cancelled at 100 s, 29 feature-size floor; 11 timeouts at 150 s) |

Step 7 changes no result, only time: the slowest file of step 6 went from 42.3 s to 21.8 s alone with the identical Steiner count (15 482), cell count (49 543) and volume error. All 83 meshes have a relative volume error of at most 1.7e-13 (median 5e-15), a median of 2.9 s per file (90th percentile 47 s, at most 142 s under 8 parallel jobs); 63 of them are single-body parts, the others have 2 to 9 bodies; the faces per file have a median of 19 (at most 213).

| 8 | step 7 with a local-cavity bulk Delaunay seed in `fea-core`, cached sub-facet circumspheres with a per-facet ball box, a point grid for the encroachment ball queries, and an edge pre-check in `has_boundary_edge` | **89 (29.7 %)** | 83 | 55 | 56 (16 cancelled at 100 s, 32 feature-size floor; 4 timeouts at 150 s); 6 files refused by the STEP entity / byte budget (`inventory`, already present in earlier steps) |

Step 8 changes no result, only time. All 83 files of step 7 give the identical triangle count and volume; six more finish inside the 100 s budget. Time over those 83 files: total 1298 s to 595 s, median 2.9 s to 1.4 s. The slowest file of step 7 (`00004686_…`, 15 482 Steiner points, 49 543 cells) went from 21.7 s to 4.2 s alone with the identical Steiner count, cell count and volume error. Profile before (`sample`): 65 % of the time was `fea_core::meshing::tet_delaunay::tetrahedralize` rebuilding the whole topology (every cell tested against every point, `TetTopology::new` revalidating) for each of the 1 500 input points plus the 8 enclosing points; the rest was `note_vertex` recomputing every sub-facet circumcentre per vertex, `has_boundary_edge` scanning every sub-facet of every facet per split, and O(vertices) scans in `segment_encroached` / `tri_encroached`.

| 9 | step 8 with `Options::merge_coplanar` (harness `--merge`): adjacent coplanar input triangles become one polygonal facet, so the diagonals of a planar face are no longer segments | **108 (36.0 %)** | 83 | 57 | 46 (25 feature-size floor, 19 cancelled at 100 s; 2 timeouts at 150 s); 6 files refused by the STEP entity / byte budget |

Step 9 loses no file of step 8 and gains 19. Over the 89 common files the total time falls from 1055 s to 208 s (median 1.4 s to 0.39 s) because the needle diagonals no longer drive refinement; the 108 meshes have a relative volume error of at most 1.4e-13. **Root cause established first** (`PLC_TRACE`-style trace of the splits before the floor error, then the input geometry around the apex): of 25 floor failures whose cascade was classified, 11 ran along an input edge between two exactly coplanar triangles, i.e. a diagonal of a planar CAD face that the sampler had to create, not a feature (a 2667 x 762 x 101.6 mm bar with a ramped end: the diagonal of each long side wall meets the wall edges at 1 to 8 degrees, and every shell split near its end vertex encroached the neighbouring shell until the 1e-6 floor); 14 ran along an edge between nearly flat facets of a curved face (dihedral 0.2 to 21 degrees, two with 90 to 104 degrees), which merging cannot touch. Merging fixed the first class (the 10-vertex bar went from the floor error to 88 Steiner points), and some of the second. Merging is exact-planarity only up to a tolerance (every vertex within 1e-9 of the neighbour's plane relative to its edge length, and the patch flat to 1e-8 of its diagonal as a whole so that a chain of tiny bends cannot add up to a curved patch). `make_delaunay` (Lawson flips of the merged patch) and the Bowyer-Watson insertion in a non-convex patch changed no result on the 76 corpus shells I compared with and without them (Steiner totals identical), so they are kept as the construction's preconditions, with a property test for the first.

| 10 | step 9 with `Options::unprotected_fallback` (protect-everything recovery that exhausts its budget or reaches the floor is repeated once with `protect_encroached: false`), `protected_max_steiner` 30 000, a multi-level grid for segment encroachment, hashed per-facet tables, incremental sub-facet spheres, flush of coordinates below 1e-30 | **138 (46.0 %)** | 83 | 60 | 8 shell (all cancelled at 100 s) + 5 file timeouts at 150 s |

Step 10, measured with 6 jobs on a machine under a load average of 40-90 from other work, so the times are not comparable with earlier steps. Loses no file of step 9 and gains 30; the 108 meshes of step 9 have the identical Steiner and cell counts (the fallback only runs after a failure), the 138 have a relative volume error of at most 1.6e-12. **Root cause proved** on the 23 floor shells (dumped shells, `plc-run`, a trace of the last 40 splits before the error): the segment that hits the floor has no encroaching vertex; it was queued by a rejected sub-facet circumcentre of a needle sub-facet of a sampled curved band (e.g. a full-height triangle with a 1.1 degree corner at an input vertex that also carries a 101 degree dihedral), and each split of it queues the neighbours the same way. Splitting only what the exact presence check finds missing (`protect_encroached: false`) conforms 22 of the 23 (and every other shell that conformed protected, 58 of 58), with 10x fewer Steiner points (44 544 to 4 217 over the 58 common shells) and 6x less time, at a worse radius-edge ratio (median 10.0 against 2.1; none of the unprotected results is refined). The 19 cancelled shells are the same cascade, not a slow convergence: protected recoveries that finish use at most 25 895 points (3 of 377 above 20 000), the cancelled ones were still adding points after 8 000 to 62 000 and no sub-segment scale marks them (their shortest split is 1e-3 to 1e-4 of the diagonal), so a point-count limit is the only deterministic cut; unprotected, 25 of 30 such shells conform in under 30 s. Speed (profile with `sample`): per vertex `note_vertex` scanned every segment (now a multi-level grid, exact: debug builds compare it with the scan) and `FacetMesh::insert` rebuilt a BTreeMap of the whole facet; 4587 went from 11.2 s to 5.6 s and 3729 from 4.8 s to 3.1 s CPU with identical counts.

Remaining (13): `00008910` (features of 6e-6 mm between planes that float32 rounding duplicated, below the 1.9e-4 mm floor: a reader/heal problem; unprotected also does not end), four giant shells (`8712`, `1023`, `8052`, `99` body 7: 6 000 to 24 000 input vertices; unprotected needs more than 60 000 Steiner points at about 1 000 per second, dominated by `Delaunay3::try_insert`; `1584` is a 1 M-triangle shell whose bulk Delaunay does not poll the cancel callback), and file timeouts of multi-body files whose bodies now take longer (`528`, `3729`, `1518`, `33`) which finish in 10 to 20 s per body on an idle machine.

Step 5 moves 56 files past the reader but none of them conforms: they stop at the conform stage (needle triangles of plain sampling). It is kept as an explicit option, not the default, and its value shows only once the conform stage improves.


Steps 1 to 4 keep the strict 1e-7 mm tolerance; step 5 is the only one that changes a tolerance, and it is an explicit option (`Options::default()` stays 1e-7 mm). Its justification is measured: of 54 files refused for edge/surface disagreement, the deviations ran from 1.8e-7 to 1.1e-3 mm, all far below the 0.05 mm facetting error of the sampled shell, and the files declare an unusable uncertainty (1e-17).

### Graded fallback is not worth it

`--graded` retries a failed body from `AnalyticModel::graded` with 25 degree chart quality. With 300 s per file and 6 parallel jobs it produced 49 conforming files (2 more than plain sampling) and 65 timeouts. The plain shells that fail have a median minimum angle of 0.3 degrees; grading does not rescue them within budget on this corpus. Not a fallback to ship.

### Classes remaining (plain sampling, `--foreign`)

| stage | files | dominant causes |
|---|---|---|
| model | 83 | edge sample off a spline surface 32 (9 are 1.0 to 2.2e-3 mm, just above the new tolerance; 23 are 0.14 mm or more, up to 5e5 mm: suspect the closest-point search, not yet examined), unsupported `EDGE_LOOP` encoding 10 (other vertex loops), edge/surface disagreement above 1e-3 mm 10, spline edge not covering its curve 7, horn/spindle torus 6, unsupported body kind 4 |
| sample | 53 | cone/torus bands 21 (torus needs a meridian-arc seam and a choice of which of the two arcs bounds the face), cylinder bands the heal cannot cut 14, trim polygon self-intersects 7, inverted triangle 4 |
| shell (conform), default budget | 103 | Steiner budget exceeded 82, feature-size floor 12, cancelled at 30 s 9; median shell minimum angle 0.3 degrees (conforming files: median 2.4, up to 45) |
| shell (conform), 200 000 budget, after steps 8 and 9 (`--merge`) | 46 | feature-size floor 25 (edges between nearly flat facets of curved faces, acute dihedrals), cancelled at 100 s 19 (remaining hotspots: `note_vertex` scans every segment and sub-facet per vertex, `refine_quality` scans every sub-facet per bad cell, cross-bore hybrid 128 s), timeouts 4 |

## Limits

300 files from one chunk, sampled by position: not random over the dataset and not a robustness proof. Plain sampling is the baseline; graded sampling was measured once (above). No OCCT parity measured (the harness runs without OCCT). No tet quality or solve was checked on the 11 successes.

## Phase 120: reader and sampling refusals (same 300 files, plain sampling, `--foreign --steiner 200000 --conform-timeout 100 --merge`)

| stage | before (run 14) | after |
|---|---|---|
| refused reading the model | 83 | 41 |
| refused sampling | 57 | 62 (24 more files reach it) |
| conforming tetrahedralization | 108 | **121** (no file lost) |

Each root cause was established with an independent check before the fix.

1. **Extrusion height range (the big "edge sample N mm off the spline surface" class, 0.14 mm to 5e5 mm).** `swept_surface` took the height range of a `SURFACE_OF_LINEAR_EXTRUSION` from the absolute position of the face's samples along the direction, but the surface is `C(t) + h d` with `h` relative to the curve: any profile not through the origin was refused (the failing distance equalled the profile's offset along `d`, e.g. 64.79 for a curve at z = 64.8). Fix: the range is the samples' spread minus the curve's own extent along `d`. Test `an_extruded_face_away_from_the_origin_reads_like_the_one_at_the_origin` (translated fixture; the old code gives the production message).
2. **Nearest point on a spline surface (the remaining class).** A dense brute-force search and a descent from its best node both gave distance ~1e-14 for samples `closest` put 0.4 to 6 mm off: the surfaces are screw threads (6 to 50 turns over the parameter range), aliased by the fixed 24 x 24 start grid, so the 6 nearest nodes all lay in one valley of the distance. Fixes in `curved_surface.rs`: the grid is refined per direction (up to 192 cells) while a line of it turns sharply at 4 or more nodes or has a segment above a twelfth of the extent; a search ending above 1e-4 mm is repeated from the grid's local minima of the distance (one per valley), and above 5e-3 mm from a best-first subdivision of the grid by bilinear patch distance (2000 expansions). Foreign-CAD samples that are legitimately 1e-3 mm off do not pay for it. Tests `a_point_on_a_many_toothed_surface_is_found_on_it` (fails with 1.94 mm on the old search), `a_point_off_a_many_toothed_surface_is_still_off_it`.
3. **Tolerance policy.** `Options::foreign_cad` is now chord / 10 (5e-3 mm at 0.05) instead of chord / 50. Reasoning in its doc comment: a boundary that disagrees with its surface by `e` adds at most `e` to a facet error of up to the chord. Measured on the 59 files still refused for edge/surface disagreement at chord / 50: 11 need up to 2.5e-3 mm, 18 up to 5e-3 mm, 22 up to 5e-2 mm (a whole chord: not admitted). The harness gained `ABC_TOL_MM` and `ABC_MODEL_ONLY` for such sweeps.
4. **Point bounds.** A `VERTEX_LOOP` face bound is skipped on every surface (not only sphere / cone poles) while the face keeps an edge-made bound: 9 of the 10 "unsupported EDGE_LOOP encoding" files are Parasolid acorn vertices on tori and revolved faces. A face of points only is still refused.
5. **Torus bands.** A torus face of two loops and no seam (fillet ring: both loops circle the axis; pipe bend: both circle the tube) now gets a circle-arc seam (`torus_seam`): of the two arcs between the loops, the one leaving the first loop towards the left of the face (normal x loop direction) and arriving at the second loop from its left. A wrong side cannot close: with both bounds reversed the shell is refused ("not closed and consistently oriented", tested). 21 of 21 files with this refusal moved on (12 to the conform stage, 2 conform; the rest reach other refusals).
6. **Seam heal on ellipse and spline loops.** `split_edge_at_sample` split only circle and line edges, so every cylinder band whose loops are ellipses (oblique cuts) or splines kept its two bounds ("cylindrical boundary winds without a chart-closing seam", 12 files). Ellipse pieces keep the curve; spline pieces keep the curve and their own sample parameters. The heal also accepts faces with further non-winding bounds (holes) beside the two winding ones. Tests `unseamed_cylinder_band_with_ellipse_loops_is_seamed_by_splitting_an_ellipse`, `unseamed_torus_band_gets_a_meridian_seam_and_samples_like_the_seamed_original`, `point_vertex_loop_bounds_are_skipped_unless_they_are_all_a_face_has`.

Gained conforming files: 13 (the 121 includes no file of run 14 lost). Timings vary with machine load (another agent was running): files near the 100 s conform limit can flip either way and are not attributable to the reader.

### Classes remaining after Phase 120 (62 sampling, 41 reader refusals)

Sampling: degenerate / locally inverted triangle 20 (not yet traced), trim polygon self-intersects 18, cylindrical single-turn certification 8, boundary winds without seam (cone/torus with more than two bounds or other shapes) 5, cone/torus single-turn 4, curved chord midpoint 4. Reader: horn / spindle tori 7 (needs a chart for major <= minor), swept curve complex spline 4, unsupported surface 4, unsupported body kind 4, spline-surface samples off by 0.4 to 2 mm 4 (not yet traced), edge/surface 3, sphere orientation 2, full sphere 2.
