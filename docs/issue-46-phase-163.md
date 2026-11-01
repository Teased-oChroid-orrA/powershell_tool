# Phase 163: interior smoothing in the sliver recipe; ABC 00006600 solves

Issue #63, excluding NEMA17 by owner request. The ABC files are the Phase 156-162 pair (`00006600`, `00007062`); they stay in temporary local storage only. The goal is unchanged: both files through the default solver gate (scaled Jacobian at least 0.1, edge ratio at most 50) and solved, with no gate, tolerance, budget or timeout relaxed. Branch `phase-163-refine-smoothing`, based on `main` at `c998c83` (Phase 162 merged as PR #208, squash `9261d4d`; pointer PR #209).

## Diagnosis of the last `00006600` cell

Phase 162 left one cell under 0.1 on `00006600` (scaled Jacobian 0.0773 at (-4.075, -1.008, 15.590)). A local trace of the sliver stage (an env-gated print in `refine_quality` of every bad cell and placed point within 0.03 mm of that spot; not committed) on the Phase 161 and Phase 162 shells, `PLC_MERGE=1 PLC_REFINE=1 PLC_CELLS=1 PLC_QSEC=900 PLC_SECS=900 plc-run`, showed:

1. **Phase 162:** the sliver [5851, 63040, 63234, 64798], where 5851 is an input vertex, has scaled Jacobian 0.0036. It appears in round 20 and is refused in every round to the end (65 refusals): its point encroaches the sub-segments [5851, 63040] and [5851, 64798], both 7.8125e-3 mm, and `drive` refuses a split a sliver asked for when the sub-segment is shorter than `optional_floor`. Post-stage reconnection lifts the spot to 0.0773, the last cell.
2. **Phase 161:** the same sub-segments were split down to 4.883e-4 mm before the floor refused them; the cell left there was 0.0941, and reconnection took it over 0.1.
3. **Why the floor moved.** `optional_floor` is the shortest input segment of the whole shell (`segments_input_shortest` in `plc_conform.rs`). The shortest shell edge is 6.47e-4 mm on the Phase 161 shell (the seam piece Phase 162 removed) and 8.89e-3 mm on the Phase 162 shell. Removing one short piece elsewhere raised the floor everywhere by at least 13 times.

A floor taken locally cannot help: any local shortest segment is at least the global one, now above 7.81e-3 mm. Splitting those sub-segments would need a lower floor, which this phase does not do.

## Change

The interior smoothing that `plc_conform` already offers (`Options::smooth_rounds`: the post-extraction flip pass alternated with `fea_core::meshing::tet_smooth`, boundary vertices fixed, judged by the scaled Jacobian since Phase 160) was set to 0 in `abc-corpus --refine` since the recipe was introduced (PR #168), although its comment said "then flips and smoothing". No measurement of that choice is recorded. Smoothing moves the interior quality point of the stuck cell, which the floor does not constrain.

- **`abc-corpus --refine`** now sets `smooth_rounds = 2` (`ABC_SMOOTH` still overrides it). The header comment says so.
- No library default changes: `plc_conform::Options::default()` keeps `smooth_rounds: 0`, and no non-test library caller sets the sliver recipe (`hybrid.rs` uses the radius-edge recipe only). The solver gate, the 20 s sliver clock, the 50,000 point cap and the floor are unchanged.

## Choosing the number of rounds

The Phase 162 gate command (`ABC_GRADED_FIRST=1 abc-corpus --one <file> --foreign --graded --steiner 200000 --conform-timeout 300 --merge --quality --refine`), default 20 s clock, `ABC_SMOOTH` varied. macOS arm64, release, serial, nothing else running.

| Rounds | `00006600` | sJ | er | time (s) | `00007062` cells under 0.1 | sJ | er | time (s) |
|---|---|---|---|---|---|---|---|---|
| 0 (Phase 162) | fail, 1 cell | 0.0773 | 7.5 | 111 | 58,843 | 0.0006 | 61.3 | 219 |
| 1 | pass, solved | 0.1126 | 7.5 | 133 | 43,980 | 0.0006 | 56.9 | 220 |
| 2 | pass, solved | 0.1135 | 7.5 | 162 | 36,667 | 0.0006 | 34.3 | 253 |
| 3 | pass, solved | 0.1135 | 7.5 | 177 | 34,635 | 0.0026 | 36.6 | 287 |
| 6 | pass, solved | 0.1135 | 7.5 | 249 | 32,962 | 0.0037 | 36.6 | 412 |

Two rounds is the fewest that both solves `00006600` at its saturated minimum (0.1135, the same as 3 and 6 rounds) and brings `00007062`'s largest edge ratio under the gate's 50. More rounds buy little on `00007062` and cost about 40 s each. With `plc-run` and `PLC_QSEC=900`, three rounds also leave no cell under 0.1 on the Phase 162 shell (minimum 0.1135).

## Results

The gate command without `ABC_SMOOTH` (so the new default of two rounds), default 20 s clock, macOS arm64, release, serial:

| | Phase 162 (`main`) | Phase 163 |
|---|---|---|
| `00006600` cells under 0.1 | 1 | 0 |
| `00006600` minimum scaled Jacobian / largest edge ratio | 0.0773 / 7.5 | 0.1135 / 7.5 |
| `00006600` solver gate / static solve | fails / not run | **passes / solved** (`gate=pass`, `solve=ok`) |
| `00006600` tetrahedra, run time | 298,152, 111 s | 297,399, 170 s |
| `00007062` cells under 0.1 (all-boundary) | 58,843 (5,702) | 36,667 (4,521) |
| `00007062` minimum scaled Jacobian / largest edge ratio | 0.0006 / 61.3 | 0.0006 / 34.3 |
| `00007062` solver gate | fails | fails (scaled Jacobian only) |
| `00007062` run time | 219 s | 260 s |

The shell, the recovery and the sliver stage are unchanged (same Steiner and quality-point counts, 26,445 on `00006600`); only the post-extraction pass differs. The solve is the harness's static solve (one end fixed, a unit transverse load on the other).

`cargo test -p solid-kernel -p fea-core --release --locked --offline -j1 --no-fail-fast` on macOS arm64: 927 passed, 0 failed, 41 ignored, no warnings (as Phase 162). No library code changed; the suites check that the example still builds and nothing else moved.

The mesh-only parity table (`occt_vs_native`) does not use the `abc-corpus` recipe and was not rerun.

Hosted checks: Phase 162's `Workbench qualification` run 41330669874, still running when PR #208 was merged by owner decision, finished with qualify passing on ubuntu-latest, macos-latest and windows-latest. This phase's run is recorded in the issue-46 pointer when its PR is merged.

## Limits / not shown

- `00007062` still fails the default solver gate (scaled Jacobian 0.0006, tens of thousands of cells under 0.1), so it is not solved. Its smallest shell angle (2.28 degrees) and the boundary-split budget remain open; smoothing cannot move its all-boundary slivers (`badb`).
- Smoothing runs after the recovery and is not bounded by `--conform-timeout`; it adds about 50 s on `00006600` and 35 s on `00007062`. The 300-file survey's 150 s process limit may therefore time out more files; the 300-file rerun was not made here.
- The last-cell diagnosis is from one trace on one machine; the trace print is not committed. The floor's dependence on the shortest input segment of the whole shell is unchanged and remains a way for a shell fix elsewhere to change the sliver stage's result.
- The numbers are from one machine (macOS arm64). No Windows evidence and no 300-file ABC rerun are shown here.
