# Phase 158: required sub-facet splits are not refused at the sliver floor

Issue #63, excluding NEMA17 by owner request. The ABC files are the Phase 156/157 pair (`00006600`, `00007062`); they stay in temporary local storage only.

## Diagnosis

After Phase 157, PLC on the `00006600` shell conforms, but with `--quality --refine` the sliver stage (`Options::min_scaled_jacobian`) reported 0 quality Steiner points, and the mesh failed the solver gate with 50,820 cells under a scaled Jacobian of 0.1, all of them tetrahedra of four boundary vertices.

`plc-run` on the dumped shell with `STAGEDBG=1` showed one refinement round, 209 inserted centres, then a rollback ("Steiner point budget exceeded") when the stage's 20 s clock ran out; the rollback returns to the snapshot taken when the stage started, so every quality point was discarded. With a 900 s clock the result was the same: after 209 points the stage spent the whole budget without adding another point.

Instrumenting the segment queue showed the cause. With the Steiner count frozen at 611, one sub-facet and one segment were requeued more than 8 million times. The sub-facet (`30315`) was missing from the triangulation and in `required_tris`; its circumcentre encroached a segment between two sliver-stage Steiner points, 4.49e-4 mm long, below the sliver floor (`optional_floor`, the shortest input segment: 6.47e-4 mm). The segment loop refuses a split below that floor unless the segment is in `required_segments`; the sub-facet loop then queued the same sub-facet and segment again. Refusal is meant for splits only a sliver asked for. For an optional sub-facet the existing code already stops (`refused_tris`, the livelock the `plate` sliver test names); the required branch had no such exit and no way to make the split happen.

## Change

When a required sub-facet's circumcentre is rejected and a segment is split first (encroached, or the nearest boundary segment when the centre leaves the facet or sits on its boundary), that segment becomes required too. Recovering a missing sub-facet is part of conformity, not quality, so its split is not subject to the sliver floor. It is still subject to the hard floor (`min_segment_relative`); a required split there fails with the floor error, and the sliver stage rolls back to its last snapshot as before.

`conform_attempt`'s set-up of the recovery state moved into `initial_state` (no behaviour change) so that a unit test can build that state directly.

`abc-corpus` now reports `quality_steiner` and `unresolved` (cells the stage left below its bound); `plc-run` gained `PLC_REFINE=1` (the `--refine` recipe) and `PLC_QSEC`, and reads `PLC_SECS` once (reading it in the progress callback took about a quarter of a profiled run).

## Results

New lib test `plc_conform::tests::a_required_sub_facet_gets_its_split_below_the_sliver_floor`: a box whose required face triangle's circumcentre is the midpoint of the face diagonal, with the sliver floor above every segment. With the change the diagonal is split at once; without it the test fails after 5 s with the budget error, as the ABC run did.

No fixture in `solid-kernel/tests/data` reaches this state: a scan of the 51 default and foreign-CAD readings that sample (27 of the 30 files) gives identical quality points and cell counts with and without the change.

`cargo test -p solid-kernel -p fea-core --release --locked --offline -j1 --no-fail-fast` on macOS arm64: 920 passed, 0 failed, 41 ignored, no compiler warnings. Only `abc-corpus` and the `plate` sliver test enable `min_scaled_jacobian`, so the mesh-only parity table (which does not) is unaffected.

ABC runs: `ABC_GRADED_FIRST=1 cargo run --release --locked --offline -j1 -p solid-kernel --no-default-features --example abc-corpus -- --one <file> --foreign --graded --steiner 200000 --conform-timeout 300 --merge --quality --refine`.

| `00006600`, fallback path | Phase 157 | Phase 158 (20 s sliver clock, the `--refine` default) | Phase 158, `ABC_QSEC=900` (diagnostic) |
|---|---|---|---|
| Quality Steiner points kept | 0 | 2,505 | 35,714 (stage ends by itself) |
| Tetrahedra | 120,209 | 135,020 | 384,381 |
| Cells under scaled Jacobian 0.1 | 50,820 | 31,400 | 483 |
| Minimum scaled Jacobian | 0.000 | 0.002 | 0.035 |
| Largest edge ratio | 11,119 | 57.8 | 13.9 |
| Solver gate | fails | fails | fails (scaled Jacobian only) |

`00007062` is unchanged (151,161 tetrahedra, 96,023 cells under 0.1, scaled Jacobian 0.000): its sliver stage ends by its clock without this livelock.

## Limits and next steps

Neither file passes the default solver gate or is solved. With the 20 s clock the stage still ends by a rollback to an earlier snapshot. Given unlimited time it ends by itself with 10,670 cells below its 0.2 bound (483 below the gate's 0.1) whose only candidate point, the circumcentre or centroid, is refused. Two follow-ups are open: the stage's speed (a profile of the corrected stage is still to be taken; the earlier profile was taken inside the livelock), and trying more than one candidate point per remaining bad cell (offset centres, points on the cell's faces and edges, moves of interior vertices), keeping the candidate that most improves the local worst cell. No gate, tolerance, budget or timeout was changed. The 300-file ABC rerun, Windows and broader solves remain open.
