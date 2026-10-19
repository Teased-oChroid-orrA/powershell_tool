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

Step 5 moves 56 files past the reader but none of them conforms: they stop at the conform stage (needle triangles of plain sampling). It is kept as an explicit option, not the default, and its value shows only once the conform stage improves.


Steps 1 to 4 keep the strict 1e-7 mm tolerance; step 5 is the only one that changes a tolerance, and it is an explicit option (`Options::default()` stays 1e-7 mm). Its justification is measured: of 54 files refused for edge/surface disagreement, the deviations ran from 1.8e-7 to 1.1e-3 mm, all far below the 0.05 mm facetting error of the sampled shell, and the files declare an unusable uncertainty (1e-17).

### Graded fallback is not worth it

`--graded` retries a failed body from `AnalyticModel::graded` with 25 degree chart quality. With 300 s per file and 6 parallel jobs it produced 49 conforming files (2 more than plain sampling) and 65 timeouts. The plain shells that fail have a median minimum angle of 0.3 degrees; grading does not rescue them within budget on this corpus. Not a fallback to ship.

### Classes remaining (plain sampling, `--foreign`)

| stage | files | dominant causes |
|---|---|---|
| model | 83 | edge sample off a spline surface 32 (9 are 1.0 to 2.2e-3 mm, just above the new tolerance; 23 are 0.14 mm or more, up to 5e5 mm: suspect the closest-point search, not yet examined), unsupported `EDGE_LOOP` encoding 10 (other vertex loops), edge/surface disagreement above 1e-3 mm 10, spline edge not covering its curve 7, horn/spindle torus 6, unsupported body kind 4 |
| sample | 53 | cone/torus bands 21 (torus needs a meridian-arc seam and a choice of which of the two arcs bounds the face), cylinder bands the heal cannot cut 14, trim polygon self-intersects 7, inverted triangle 4 |
| shell (conform) | 103 | Steiner budget exceeded 82, feature-size floor 12, cancelled at 30 s 9; median shell minimum angle 0.3 degrees (conforming files: median 2.4, up to 45) |

## Limits

300 files from one chunk, sampled by position: not random over the dataset and not a robustness proof. Plain sampling is the baseline; graded sampling was measured once (above). No OCCT parity measured (the harness runs without OCCT). No tet quality or solve was checked on the 11 successes.
