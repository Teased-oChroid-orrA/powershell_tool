# Phase 184: closed B-spline bands, refusal wording, mesher profile

Date 2026-11-19. Follow-up to Phases 182 and 183 (OpenCascade retired).

## Native import: 33 of 33 NIST files

`nist_ctc_02_asme1_ap203.stp` was the last refusal. Two faces of it are B-spline bands that close on themselves in one parameter (a periodic spline) and are bounded by two loops that each wind once around it, with no seam edge:

- #1167 (`B_SPLINE_SURFACE_WITH_KNOTS`, degree 1 across two rows, closed in v);
- #1473 (a rational surface, degree 2 across three rows, closed in u).

Changes:

- `seam_unseamed_spline_bands` (`solid-kernel/src/analytic_step.rs`) cuts such a band along the iso-curve of the closed parameter between the two loops, like the cylinder, torus and revolution bands. The seam is a `Curve::Spline` of the exact iso-curve (`NurbsSurface::iso_curve`, a homogeneous combination of the control rows through the fixed parameter's basis), sampled with the shared chord rule.
- `SplineSurface::closed_band` (end curves coincide within tolerance), `parameters`, `parameter_range`, `iso_curve`.
- The chart of a closed band is unwrapped (`chart_loop` may continue a sample one period either way; `param_of_chart` folds the closed coordinate back into the domain), so the boundary of the cut face closes in the chart without winding.

Evidence: `STEP_DIR=<ap dir> cargo test --release --test native_import_corpus -- --ignored --nocapture` gives `ok: 487 faces, 8992 triangles, volume 47165580.175`; the `rc` and `ap242` variants of the same part give 47155247 and 47159446 (0.02 percent). Unit test: `an_iso_curve_is_the_surface_along_one_parameter`.

Limits: the seam and chart changes have no committed STEP fixture (the NIST corpus is a local download); the corpus run is the end-to-end evidence. Only bands with exactly two winding bounds are cut.

## Refusal wording

Native solid-edit refusals no longer start "without OpenCASCADE" (stale since Phase 183); they read `<what cannot be done>: <why>`. Tests and comments that described an `occt` feature were updated. Older phase docs keep their wording (append-only record).

## Mesher profile

A sampling profile (`sample`) of the `ctc_01 rd` volume mesh (single thread, 100 percent in `boundary_repair::repair`): about 70 percent of the time is in `SurfaceSolid::faces_near` (BVH `within` and `closest_on_triangle`, SipHash map, allocation) and `signed_distance`, called from the boundary state checks of the repair. Behaviour-preserving changes: `within` returns the closest point it already computed (no second `closest_on_triangle`), a small vec replaces the hash map, a stack array replaces the heap stack.

Timing (measured): same-time A/B, two single-thread runs side by side on one machine, `ctc_01 rd`, `MESH_TETS=3000`: before 652.2 s, after 505.9 s (22 percent faster), with identical output (14,015 tets, volume 14809467.610, min scaled Jacobian 0.025, 83 cells under the gate). Earlier note, kept for the record: no speed-up is claimed from the first, contended run. The only baseline is 685 s for `ctc_01 rd` (14,015-tet case, `MESH_TETS=3000`); the changed build measured 979 s while a workspace build competed for the CPU, which is not comparable. The larger lever, still open, is fewer distance queries in `boundary_repair`'s before/after surface snapshots (each boundary face state does one `signed_distance` and one `faces_near` per sample, four samples).

## Proof

`cargo test --workspace --release --offline --locked --no-fail-fast -j4`: 3,574 passed, 0 failed, 131 ignored (`.audit/p184-tests.log`).

## Addendum: memoised boundary face states

`Work::boundary_face_state` (`fea-core/src/meshing/boundary_repair.rs`) is a pure function of the face's corner and apex coordinates and `h`, yet every candidate operation re-measured each kept face in its `before` and `after` snapshots (four `signed_distance` plus `faces_near` samples each). It is now memoised by those exact coordinates (bit patterns), cleared at 2^20 entries; `measure_boundary_face` is the unchanged computation.

Measured: same-time A/B, two single-thread runs side by side, `ctc_01 rd`, `MESH_TETS=3000`: main 382.3 s, memoised 117.4 s (3.3x faster; absolute times are lower than the 652/506 s above because nothing else was loading the machine). Output identical (14,015 tets, volume 14809467.610, min scaled Jacobian 0.025, 83 under the gate). Proof: `cargo test --release -p fea-core -p fea-problem -p solid-kernel` (see PR).

## Addendum 2: profile of the remaining mesher time, query cache, allocations

`sample` of `ctc_01 rd` on main after the face-state memo (all in `boundary_repair::repair`, single thread): `closest_on_triangle` 22 percent, `Bvh::within` 19 percent, malloc/free 26 percent (all callers), `scaled_jacobian` 5 percent, `Bvh::nearest` 5 percent, `boundary_faces` 4.5 percent.

A counter in `faces_near` showed 17 million queries about only 1.12 million distinct points (cells of the query radius: 831): candidate operations in a round measure the same points repeatedly, and neighbouring faces share edge midpoints. Changes, all behaviour-preserving:
- `repair()` wraps `ctx.sdf` and `ctx.face_dists` in per-run exact-point caches (bit patterns of the point; both are pure; cleared at 2^20 entries each). No call site changed.
- `boundary_faces` builds each face key as a fixed `[usize; 3]` instead of a `Vec` per tetrahedron face.
- `Bvh::nearest` uses a stack array instead of a heap `Vec` per query.

Measured: same-time A/B, `ctc_01 rd`, `MESH_TETS=3000`, main (memoised states) 117.9 s, this change 52.3 s (2.25x); output identical (14,015 tets, volume 14809467.610, min scaled Jacobian 0.025, 83 under the gate). Against the unmemoised main of the previous addendum (382.3 s in its own side-by-side run) the case is 52 s; the runs were on different days of load, so only the within-pair ratios are claims.

Negative result: replacing the `BTreeMap` that `boundary_faces` builds (and the snapshot maps) with a sorted `Vec` and binary search, which the profile above suggested (about 24 percent of busy time in `boundary_faces` and its map), gave no speed-up: same-time A/B on `ctc_01 rd`, `MESH_TETS=3000`, 103.2 s (main) against 106.7 s (sorted `Vec`), identical output. Reverted. Not retried: the cost is probably the sort and lookups rather than the allocations, but that was not measured.
