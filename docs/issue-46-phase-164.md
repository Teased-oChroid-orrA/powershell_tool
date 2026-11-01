# Phase 164: index the facet meshes a split segment bounds; ABC 00007062's sliver stage finishes at the default clock

Issue #63, excluding NEMA17 by owner request. The ABC files are the Phase 156-163 pair (`00006600`, `00007062`); they stay in temporary local storage only. The goal is unchanged: both files through the default solver gate (scaled Jacobian at least 0.1, edge ratio at most 50) and solved, with no gate, tolerance, budget or timeout relaxed. Branch `phase-164-segment-facets`, based on `main` at `ad42e72` (Phase 163 merged as PR #210, squash `0703b9e`; pointer PR #211).

## Why

After Phase 163, `00007062` was limited by the speed of the sliver stage. At the default 20 s clock it placed 4,413 quality points and left 36,667 cells under 0.1; given 900 s (`ABC_QSEC=900`, a diagnostic only) it placed 27,666 and left 448. A profile of the stage (line-tables build, `sample`, addresses resolved with `atos`) put about 7,557 of about 11,000 samples in `FacetMesh::has_boundary_edge`, inlined into `State::split_segment`: every segment split looked for the facet meshes the segment bounds by testing every mesh (`for k in 0..self.meshes.len()`), and `00007062` has many meshes.

## Change

`plc_conform::State` keeps `vertex_meshes`, the meshes holding each vertex (in their `uv`). It is built from the meshes when the state is made and kept current by `State::own_vertex` after each of the two insertions into a mesh (a segment split and a sub-facet circumcentre); a mesh never loses a vertex, and flips do not change its vertices. `split_segment` tests only the meshes holding the segment's first end, in index order, so the meshes it changes and the order it changes them in are the same as before. A debug assertion compares the result with the scan of every mesh.

`Missing` (the kept missing items of Phase 160) had its own copy of this map, rebuilt from the changed meshes at each presence check; it now uses the state's map, and that rebuild is gone.

No gate, clock, cap or floor changes: the solver gate, the 20 s sliver clock, the 50,000 `max_quality_steiner` cap and its scope, and `optional_floor` are as they were.

## Same insertion sequence

`plc-run` on the dumped shells, `PLC_MERGE=1 PLC_REFINE=1 PLC_SMOOTH=2 PLC_QSEC=900 PLC_SECS=900`, macOS arm64, release:

- `00007062`: the output of `main` (`ad42e72`, built in a separate worktree and target) and of this branch is identical apart from the time: 54,179 Steiner points, 27,666 quality points, 363,285 interior cells, 448 cells under 0.1 (12 with all four vertices on the boundary). The branch ran with `PLC_VERIFY_KEPT=1` (exact cache checks every round) and still took 203 s against `main`'s 309 s without it.
- `00006600`: 26,445 quality points and 297,399 interior cells, as in Phase 163; no cell under 0.1; `PLC_VERIFY_KEPT=1` clean.

## Results

The gate command (`ABC_GRADED_FIRST=1 abc-corpus --one <file> --foreign --graded --steiner 200000 --conform-timeout 300 --merge --quality --refine`), default 20 s clock, macOS arm64, release, one run at a time with nothing else running:

| | Phase 163 (`main`) | Phase 164 |
|---|---|---|
| `00007062` quality points | 4,413 | 27,666 (the 900 s result) |
| `00007062` cells under 0.1 (all-boundary) | 36,667 (4,521) | 448 (12) |
| `00007062` minimum scaled Jacobian / largest edge ratio | 0.0006 / 34.3 | 0.0190 / 29.2 |
| `00007062` solver gate | fails | fails (scaled Jacobian only) |
| `00007062` run time | 260 s | 258 s |
| `00006600` gate / solve | passes / solved | passes / solved (sJ 0.1135, er 7.5, 297,399 tetrahedra) |
| `00006600` run time | 170 s | 130 s |

`00007062` now gets the same mesh at the default clock as with 900 s: the stage ends by itself (nothing left it may do), not by the clock. Its run time does not drop because the stage now does all its work inside the 20 s instead of stopping early.

`cargo test -p solid-kernel -p fea-core --release --locked --offline -j1 --no-fail-fast` on macOS arm64: 927 passed, 0 failed, 41 ignored, no warnings (as Phase 163). The `plc_conform` unit tests and the `plc_conform`, `plc_merge` and `plc_preserving` integration tests also pass in a debug build, where the new assertion compares the index with the scan of every mesh at every segment split.

## The margin is thin

On this machine the stage needs most of its 20 s on `00007062`. With the clock set to 13 s (`PLC_QSEC=13`, a diagnostic) it is stopped after 15,139 quality points and returns to its last kept state (10,224); at 10 s it keeps 7,012. A run under `sample` (which slows the process) was stopped after 21,192. So on a slower or loaded machine `00007062` still stops early, and the result at the default clock depends on the machine. This phase does not change the clock.

A second profile of the stage after the change is flat (about 13,200 samples over 18 s): the candidate-point previews of `better_point` (Delaunay cavity and insphere tests, scaled Jacobians of the new cells) about 31%, point insertions about 20%, full cell classifications (`bad_cells_from_scratch`, needed when the cell cache is empty, as at the start of the stage) about 7%, the facet ball index lookups (`FacetIndex::near`, including building and sorting the candidate list) about 6%. None of these is a single hot spot that can be removed while keeping the insertion sequence identical; fewer candidate points would change the sequence and the mesh.

## Limits / not shown

- `00007062` still fails the default solver gate: 448 cells under 0.1, minimum scaled Jacobian 0.0190 (edge ratio 29.2 is within the limit), so it is not solved. The 448-cell residue is the next item.
- The default-clock result on `00007062` depends on the speed of the machine (see above). No Windows or hosted timing is shown.
- The mesh-only parity table (`occt_vs_native`) was not rerun; this change keeps the insertion sequence, which the two files above show for their shells only.
- The numbers are from one machine (macOS arm64). No 300-file ABC rerun is shown here.
- Hosted checks: Phase 163's `Workbench qualification` run 41339381216, still running when PR #210 was merged, finished with qualify passing on ubuntu-latest, macos-latest and windows-latest.
