# Phase 167: feature-off STEP import, volume mesh and static solve on hosted Windows x64

Issue #46, Track C step 4 of `issue-46-abc-and-retirement-gates-plan.md` (step 3 of its narrowed-gate proposal), executing `issue-46-windows-runtime-plan.md`. Branch `phase-167-windows-step-runtime`, based on `main` at `1a59333` (Phase 164 docs, PR #214). OpenCascade stays the production backend; `occt_retirement_approved` stays false. No gate, tolerance, budget or solver setting changed.

## Existing evidence (plan step 1)

The `Workbench qualification` workflow already runs `scripts/qualify.py` on `windows-latest` for every pull request that touches code, as `cargo test --workspace --locked -j 2 --no-fail-fast` without CAD features. Its log for run [41345593545](https://github.com/Teased-oChroid-orrA/powershell_tool/actions/runs/41345593545) (Phase 164, `5a86bb9`, Windows job 124964599240) shows `tests\native_solid.rs` with 3 passed (including `supports_and_loads_follow_their_faces_and_the_edited_solid_solves`) and `tests\solid.rs` with 7 passed; `tests\occt_vs_native.rs` built with no tests, so `occt` was off under workspace feature unification. That run records neither the processor nor the checked-out commit beyond GitHub's merge commit, and no workflow ran any of the 19 same-STEP models feature-off on Windows: `occt_vs_native.rs` is `#![cfg(feature = "occt")]` because it imports every model through OpenCascade to compare. So the focused tests had Windows evidence, the benchmark models did not.

## Change

- `fea-problem/tests/native_step_benchmark.rs` (`#![cfg(not(feature = "occt"))]`): the 14 rows of `occt_vs_native::cases` that need no OpenCascade to build (6 `NativeBrep` shapes, 8 committed STEP files including NEMA17), each imported by `Problem::from_step`, which without the feature is `native_solid::import_step_native` (chord a thousandth of the native bounding-box diagonal, the shared `finish_import` element size), then `solid_mesh` (its 5 % volume bound and named-face refusals) and `solve` on the benchmark of the comparison table: the lowest flat `-z` face fixed, 10 MPa traction on the highest `+z` face. The test asserts that every row imports, meshes and solves with force balance under `1e-6` and a positive minimum scaled Jacobian (criterion 1 of the asserted comparison); the default solver-quality gate (`QualityThresholds::default()` with size progression) is printed per row and counted, not asserted, because NEMA17 is known below it. Since the chord and size come from the native import rather than OpenCascade's, these rows are not the `occt_vs_native` table's numbers. It is a separate file so the parent session's uncommitted Phase 165 edits to `occt_vs_native.rs` do not conflict; the case list duplicates that file's 14 rows and can be shared once both land.
- `.github/workflows/windows-step-runtime.yml` (`pull_request` on `fea-problem`, `solid-kernel`, `fea-core`, `mechanics-core`, the NEMA17 file, Cargo files and itself; `workflow_dispatch` once on `main`): checks out the pull request's head commit (not the merge commit); records `Win32_Processor`, `Win32_OperatingSystem`, `rustc -vV` and the commit; fails if `cargo tree -p fea-problem -e features --no-default-features --locked` mentions `occt`; runs the two focused test binaries and requires `running 3 tests`, `running 7 tests` and two `ok ... 0 failed` results, then the `--exact` solve test requiring `running 1 test` and `1 passed`; builds the benchmark binary in release (`--message-format=json` names the executable), runs it directly with `native_rows --exact --ignored --nocapture`, measures wall time and samples `PeakWorkingSet64` every 0.5 s, and requires exit 0, `running 1 test` and `1 passed`. No build cache: every run compiles from the checked-out source. Evidence goes to the `windows-step-runtime` artifact and the job summary.

Also: root `CLAUDE.md` said the test workflows had all been removed; it now names `qualification.yml` (which never was) and the new workflow. `fea-problem/AGENTS.md` said `tessellate_step` needs `occt` (it falls back to the native pipeline) and now lists the new test. The workflow ignores Markdown changes under its paths.

## Hosted Windows result

Run [41362857299](https://github.com/Teased-oChroid-orrA/powershell_tool/actions/runs/41362857299), job 125016740650, `pull_request` on PR #215, commit `a820c5f588b9b2c2d3d3ea82c37f35045c56276f`, **success**.

- Host: `Intel(R) Xeon(R) Platinum 8370C CPU @ 2.80GHz` (2 cores, 4 logical), Windows Server 2025 Datacenter 10.0.26100 (build 26100), 64-bit, 16.0 GiB visible memory; `rustc 1.99.1 (58cf8f9c3 2026-10-13)`, host `x86_64-pc-windows-msvc`, LLVM 23.1.1; `+crt-static` from `.cargo/config.toml`.
- Features: 503-line feature tree, no `occt`.
- `cargo test -p fea-problem --no-default-features --locked -j 2 --test native_solid --test solid`: 3 passed / 0 failed / 0 ignored and 7 / 0 / 0; `... --test native_solid supports_and_loads_follow_their_faces_and_the_edited_solid_solves -- --exact`: 1 passed, 2 filtered out. 365 s wall including the debug build.
- `cargo test -p fea-problem --no-default-features --locked --release -j 2 --test native_step_benchmark --no-run` (4 min 38 s build), then the binary: 1 passed; **14 models, 14 imported, meshed and solved in force balance, 13 pass the default quality gate**; wall 802.3 s, peak working set 1921 MiB.

| Model | h | tets | DOFs | min sJ | min rr | max er | jump | gate (<0.1) | mesh s | solve ms | umax | U | Rz | equilibrium |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| box (native) | 2.9918 | 32,256 | 146,319 | 0.512 | 0.638 | 2.3 | 1.0 | ok (0) | 0.86 | 7,269 | 1.98304e-3 | 5.89037 | -6000.000 | 1.0e-12 |
| cylinder (native) | 2.2906 | 27,791 | 125,442 | 0.126 | 0.072 | 6.5 | 1.7 | ok (0) | 4.63 | 6,738 | 1.48587e-3 | 2.28847 | -3116.110 | 4.1e-13 |
| box + hole (native) | 0.5085 | 74,404 | 335,580 | 0.125 | 0.124 | 7.2 | 1.5 | ok (0) | 4.73 | 14,152 | 2.47252e-4 | 9.01777e-2 | -969.167 | 4.8e-13 |
| box + boss (native) | 0.8767 | 17,719 | 83,796 | 0.104 | 0.028 | 5.9 | 1.6 | ok (0) | 4.13 | 1,860 | 2.28600e-4 | 1.35143e-2 | -121.156 | 4.7e-15 |
| box + boss + hole (native) | 0.5273 | 83,426 | 375,852 | 0.120 | 0.042 | 7.8 | 1.8 | ok (0) | 18.99 | 19,780 | 2.31473e-4 | 1.31979e-2 | -116.078 | 6.0e-13 |
| box, all edges filleted (native) | 0.9530 | 93,756 | 416,826 | 0.111 | 0.037 | 7.9 | 1.9 | ok (0) | 16.77 | 21,587 | 5.77085e-4 | 2.07625e-1 | -799.006 | 1.3e-12 |
| cylinder, filleted top | 0.6389 | 76,524 | 344,178 | 0.105 | 0.036 | 9.4 | 2.1 | ok (0) | 15.16 | 16,774 | 3.31067e-4 | 5.59258e-2 | -382.104 | 7.7e-13 |
| edit plate (40x30x10, hole) | 1.9979 | 48,939 | 223,047 | 0.120 | 0.044 | 5.8 | 1.7 | ok (0) | 6.51 | 9,733 | 6.54902e-4 | 2.60525 | -11507.439 | 3.8e-13 |
| cross-bore (R8, R3) | 1.2366 | 91,466 | 415,374 | 0.120 | 0.041 | 9.7 | 1.8 | ok (0) | 17.90 | 22,151 | 1.96450e-3 | 1.90375 | -1995.693 | 7.2e-13 |
| cross-bore, filleted | 1.2359 | 91,361 | 414,774 | 0.107 | 0.035 | 9.6 | 1.9 | ok (0) | 28.21 | 21,719 | 1.75380e-3 | 1.51815 | -1771.143 | 1.0e-12 |
| t-junction | 1.0899 | 88,059 | 398,397 | 0.123 | 0.063 | 5.8 | 1.7 | ok (0) | 10.82 | 21,590 | 1.46261e-3 | 8.15627e-1 | -1122.994 | 4.9e-13 |
| tilted hole | 1.9796 | 96,925 | 434,364 | 0.120 | 0.039 | 6.6 | 1.8 | ok (0) | 19.33 | 21,998 | 1.61159e-3 | 5.55791 | -11430.961 | 1.9e-12 |
| vase (spline revolution) | 0.7982 | 76,096 | 345,492 | 0.110 | 0.040 | 10.3 | 1.9 | ok (0) | 31.38 | 19,181 | 1.23542e-3 | 3.80638e-1 | -630.426 | 6.5e-13 |
| NEMA17 motor bracket | 1.6667 | 205,995 | 959,235 | 0.047 | 0.012 | 11.6 | 2.1 | **NO (4)** | 57.20 | 91,866 | 6.71372e-3 | 4.62145 | -2400.000 | 3.6e-13 |

Import took at most 0.46 s per model (the filleted cross-bore). Every reaction equals the applied load on the loaded area (for example box: 10 MPa on 20 x 30 mm = 6000 N).

**Failure attribution.** The one gate failure is NEMA17, 4 elements under scaled Jacobian 0.1 (minimum 0.047), identical on Windows and macOS (below). It is the known native NEMA17 residue (`the_nema17_native_relaxation_starts_from_the_best_round` bounds it at 9 on the OpenCascade-sized path; NEMA17 work is deferred by owner request, Phase 155), not a Windows effect. It still meshes within the volume bound, resolves its named faces and solves in balance.

## Same commit on macOS (local)

Apple M1, 8 GiB, macOS 15.7.2, `rustc 1.96.1` `aarch64-apple-darwin`, own worktree target, the same two commands (the release run with the tree equal to `a820c5f` apart from a doc-comment phase number). Focused: 3 / 0 / 0 and 7 / 0 / 0. Benchmark: 1 passed, the same 14 solved / 13 gate summary, 1746 s, maximum resident set 2.16 GiB; timings were taken while other sessions ran release test suites on the same machine, so they are not a speed comparison.

Ten rows are identical to every printed digit (h, chord, tets, DOFs, quality, displacement, energy, reaction). Four differ in the last digits, from platform floating-point differences in the native sampler and mesher (different `libm` and compiler, as in the graded-torus case recorded in `issue-46-resume.md`):

| Model | macOS arm64 | Windows x64 |
|---|---|---|
| box, all edges filleted | sJ 0.105, rr 0.041, jump 2.0, umax 5.77072e-4, U 2.07620e-1 | sJ 0.111, rr 0.037, jump 1.9, umax 5.77085e-4, U 2.07625e-1 |
| cross-bore (R8, R3) | 91,465 tets, U 1.90374 | 91,466 tets, U 1.90375 |
| cross-bore, filleted | 91,350 tets, U 1.51816 | 91,361 tets, U 1.51815 |
| vase | umax 1.23549e-3, U 3.80652e-1 | umax 1.23542e-3, U 3.80638e-1 |

The largest relative difference in strain energy is 3.7e-5 (vase); every row's gate verdict and the NEMA17 sub-gate count agree.

## Limits

- One hosted host (Xeon Platinum 8370C) at one commit. Hosted Windows runners differ by CPU (AMD EPYC 7763 hosts returned wrong `private-gemm-x86` 0.1.20 products before the 0.1.22 pin); other hardware is not covered. `solve_mesh` verifies each linear result and recomputes on one thread on failure, so a wrong product would refuse rather than pass silently.
- This is `cargo test` with Rust, Cargo and the repository present. It is not the packaged, offline, clean-machine, no-admin acceptance of the staged `app-tui.exe` on Windows 10 1809+ / 11 (plan step 5), which remains open, nor proof on the oldest supported Windows build.
- 14 of the 19 same-STEP models: the 5 OpenCascade-built shapes need the feature to generate their STEP text and cannot run feature-off. The rows use the native import's chord and size, so they say nothing about OCCT/native parity; Phase 165's licensed real-CAD corpus and the 19-model comparison remain the parity evidence.
- The quality gate is reported, not asserted, in the new test; NEMA17 is below it.
- The new workflow runs only on pull requests touching the listed paths and is dispatchable once on `main`.

## Status

Windows native (feature-off) STEP import, volume mesh and static solve: qualified for this host and commit on the 14 built-in native rows and the focused tests. OCCT remains the production backend; retirement remains unapproved.
