# Phase 142: the boundary repair keeps its best round

2026-10-25. Continues [Phase 141](issue-46-phase-141.md) on `main`. OCCT remains production and `occt_retirement_approved` remains false.

## Defect

`boundary_repair::repair` returned the mesh of its last round. A round can lift one element over the gate and spoil another, so the loop can end worse than a state it passed through. Traced on the t-junction (`REPAIR_TRACE=1 NO_SOLVE=1 CASE='t-junction'`), OCCT tessellation, elements under the 0.1 gate at the start of each round:

    752, 172, 48, 21, 10, 11, 13, 9, 8, 10, 9, 10, 8, 7 (round 13), 8, 8  -> returned 8

The native tessellation's best round equals its end (8), so it does not change.

## Change (`fea-core/src/meshing/boundary_repair.rs`, fe1f4c8)

- At the start of each round whose key (elements under the gate, then all elements worked) is the best so far, the loop takes a `Snapshot` of the `Work` state. After the loop, if the end key is worse than the best, it restores that snapshot. The ranking key is the one the stagnation counter already used. `round_work` and `round_key` factor out the round's work list and key.
- The snapshot holds every per-vertex and per-element table: points, cells, liveness, vertex-to-cell adjacency, boundary flags, vertex state, reference sizes, versions and live count. A first version restored only points, cells and liveness. That left the adjacency describing the abandoned mesh, and the `REPAIR_TRACE` diagnostics then panicked in `tets_with`. The restore also drops the failure memory and the open log.

## Tests

- `fea-problem/tests/occt_vs_native.rs::the_t_junction_repair_keeps_its_best_round` (occt, ignored, about 25 s in release): failed on the old code ("8 elements under the gate"), passes with 7.
- `boundary_repair::tests::restoring_a_snapshot_returns_every_table_to_the_kept_round`: after a committed split and move, the restore brings back the points, cells, adjacency and table lengths, and `tets_with` answers for the restored mesh.

## Evidence (macOS arm64, release, `CARGO_INCREMENTAL=0`, serial Cargo, at fe1f4c8)

- The 19-model same-STEP table, mesh only (`docs/issue-46-phase-142-parity.txt`): **34/38 rows pass**, as in Phase 141. Exactly two rows changed, both improved. All other rows are identical to Phase 141 in element count, minimum scaled Jacobian, gates and sub-gate counts:

  | Model | Phase 141 | Phase 142 |
  |---|---|---|
  | t-junction | 8/8 | 7/8 |
  | NEMA17 motor bracket | 24/33 | 24/29 |

- Solves on the two changed rows, against Phase 141: max displacement moves by at most 3e-5 relative (t-junction OCCT 1.45613e-3 to 1.45612e-3; NEMA17 native 6.71334e-3 to 6.71349e-3), and the reaction and equilibrium are unchanged. The OCCT/native max displacement gap is 0.64% (t-junction) and 0.27% (NEMA17).
- The NEMA17 native solve time was 219 s and 407 s in two runs on the identical (deterministic) mesh, against 116 s in Phase 141. Load average was 73 from builds outside this work. These times measure machine load, not the change; a quiet-machine timing was not made.
- `cargo test -p fea-core -p solid-kernel -p fea-problem --release --locked --offline -j1 --no-fail-fast`: 1122 passed, 0 failed, 47 ignored (Phase 141: 1121, plus the new unit test).
- `connected_plate_boss_rim_passes_solver_gate` passes.
- `cargo test -p app-tui --release --locked --offline -j1`: 1226 passed, 0 failed, 22 ignored.

## Limits and what is still open

- t-junction (7/8) and NEMA17 (24/29) still fail the gate on boundary cells. Retention only keeps what the rounds reach; clearing the rest needs operations the repair does not yet have, or that it refuses.
- Everything else in the Phase 141 Limits list is unchanged: the boss audits on the final build, a fidelity measurement under the region bound, `face_touches_patch` across shallow sharp edges, the unaudited fea-core modules, and Windows evidence.
