# Phase 126 — bounded ABC chart diagnosis (2026-10-21)

Epic #46 / #63, feature-off native STEP path. This is a diagnostic checkpoint, not OCCT parity or retirement evidence. The worktree's torus-only graded-boundary repair has a synthetic face #339 regression; the earlier broader curved-face split was removed because it exhausted the bracket grading budget.

With `ABC_GRADED_FIRST=1`, `--foreign --graded --steiner 200000 --conform-timeout 0`, the representative ABC file `00000297_52a153d2417747b5926ff9df_step_002.step` and `00004389_478cd71be49144f0bb3ff944_step_000.step` reach PLC recovery. Their `shell`-stage cancellation is expected with the zero-second conform timeout; neither result proves a conforming or solver-usable mesh. The other two diagnosed files still stop at exact chart-polygon validation:

- `00006600_2516c959a43348f89c02cae9_step_000.step`: face #379 is a rational B-spline surface. Bound #1066 has consecutive projected chart samples `[3.0902992217623497, 0]` and `[3.0902992217623497, 0]` near segment 156.
- `00007062_a7b44768091a464bb7beff7f_step_000.step`: face #92 is a surface of revolution. Bound #225 has collinear samples `[0, 22.1474883179823]`, `[115.48623768519036, 22.1474883179823]`, `[111.7608751792165, 22.1474883179823]` at segment 0, so the sampled path doubles back.

The validator still refuses both. Its error now reports the bound, segment and three chart points; a unit regression verified that diagnostic red then green. Do not drop a repeated chart point or simplify the backtrack without proving the source edge geometry, face trim and shared-edge topology. The ABC files remain outside the repo because redistribution rights are not established.

Local macOS arm64 qualification, this worktree's own target, locked offline dependencies: `cargo test -p solid-kernel --no-default-features --locked --offline -j1` passed 292 tests (4 ignored); `cargo test -p fea-problem --no-default-features --locked --offline -j1` passed, including the three `native_solid` tests. The first crate-wide `fea-problem` run was interrupted because RTK suppressed live progress; the direct-output rerun exited successfully. Neither OCCT-enabled parity, Windows builds, nor production per-model quality/efficiency was checked. `occt_retirement_approved` remains false.
