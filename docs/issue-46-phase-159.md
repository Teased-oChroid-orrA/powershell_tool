# Phase 159: incremental edge and face presence in the PLC recovery loop

Issue #63, excluding NEMA17 by owner request. The ABC files are the Phase 156-158 pair (`00006600`, `00007062`); they stay in temporary local storage only.

## Diagnosis

After Phase 158 the sliver stage on the `00006600` shell no longer livelocks, but it is slow: in its 20 s clock it placed 4,345 quality points before the clock ended it (and the rollback returned to the last snapshot), and given unlimited time (`PLC_QSEC=900`) a `plc-run` of the dumped shell took 405 s, about 330 s of it in the sliver stage.

A macOS `sample` profile of that stage showed about three quarters of its time in `State::tet_edges_and_faces`: every round of `drive` (one per batch of splits or quality points, 747 rounds on this shell) rebuilt the set of every edge and face of the whole triangulation from all cells (969,000 tetrahedra, hull cells included, by the end of the stage) to check which segments and sub-facets are present.

## Change

`Delaunay3` now also records the tetrahedra the most recent insertion created (`last_added`, beside the existing `last_removed`). `plc_conform` keeps a `Presence` table, every edge and face of the triangulation with the number of tetrahedra that have it, built once at the first exact presence check (`State::ensure_presence`) and then updated by `State::insert` from the removed and created cells of each insertion. `drive` reads that table instead of rebuilding it. `FacetMesh::flip_toward` takes the same face table.

The table is exact, not a cache that can go stale: every change of the triangulation goes through `State::insert`, and `State` clones (the sliver stage's snapshots) carry their table with them.

## Results

New lib test `plc_conform::tests::presence_counts_follow_insertions`: 200 insertions into a box, after which the incrementally kept table equals the one rebuilt from all cells.

`plc-run` on the dumped `00006600` shell (`STAGEDBG=1 PLC_MERGE=1 PLC_REFINE=1 PLC_SECS=900`), macOS arm64, release:

| | Phase 158 | Phase 159 |
|---|---|---|
| 20 s sliver clock: quality points placed before the clock ends the stage | 4,345 | 15,151 |
| 20 s sliver clock: whole run | 128 s | 100 s |
| `PLC_QSEC=900`: whole run (stage ends by itself) | 405 s | 136 s |
| `PLC_QSEC=900`: result | 35,714 quality points, 10,670 unresolved, 384,381 interior cells | identical |

With `PLC_QSEC=900` both versions print the same 36 `refine round` lines (bad-cell count, Steiner count and quality count of every round), and the same final report: the change alters speed only, not the sequence of insertions.

`cargo test -p solid-kernel -p fea-core --release --locked --offline -j1 --no-fail-fast` on macOS arm64: 921 passed, 0 failed, 41 ignored, no compiler warnings.

ABC gate, `ABC_GRADED_FIRST=1 cargo run --release --locked --offline -j1 -p solid-kernel --no-default-features --example abc-corpus -- --one <file> --foreign --graded --steiner 200000 --conform-timeout 300 --merge --quality --refine` (the default 20 s sliver clock; both bodies take the fallback path):

| | `00006600` Phase 158 | `00006600` Phase 159 | `00007062` Phase 158 | `00007062` Phase 159 |
|---|---|---|---|---|
| Quality Steiner points kept | 2,505 | 12,455 | not recorded | 3,173 |
| Tetrahedra | 135,020 | 204,584 | 151,161 | 177,420 |
| Cells under scaled Jacobian 0.1 | 31,400 | 3,380 | 96,023 | 85,052 |
| of which all four vertices on the boundary | not recorded | 66 | not recorded | 9,333 |
| Minimum scaled Jacobian | 0.002 | 0.023 | 0.000 | 0.0004 |
| Largest edge ratio | 57.8 | 21.2 | not recorded | 164.1 |
| Solver gate | fails | fails (scaled Jacobian only) | fails | fails (scaled Jacobian and edge ratio) |

In both files the stage still ends by its clock and returns to its last snapshot (`unresolved 0` is the snapshot's count, not a converged stage). On `00006600` only 66 of the 3,380 cells under the gate have four boundary vertices, so most of what is left there is not the flat boundary sliver the Phase 157 report found (all 50,820 cells then).

## Next speed hot spot

A profile of the Phase 159 sliver stage (about 11 s of samples) puts about a third of its time in `refine_quality`'s test of each candidate centre against the circumballs of the sub-facets (`FacetMesh::ensure_spheres` and a linear scan of every sub-facet of each facet mesh whose bounding box contains the centre), and about 12% in `outside_cells`, which classifies every tetrahedron once per round. Neither was changed here.

## Limits

Neither ABC file passes the default solver gate or is solved. No gate, tolerance, budget or timeout was changed; `PLC_QSEC` is a diagnostic only.
