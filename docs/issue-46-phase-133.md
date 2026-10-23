# Issue 46 Phase 133: grading progress report

`AnalyticModel::graded` now leaves `GradeReport::converged` false when a round requests boundary splits but exact-curve insertion adds no edge samples. Previously it reported convergence in that stalled case, although the documented condition for convergence is that no face asks for another split. The sampled model and refusal gates are unchanged.

`cargo test -p solid-kernel --no-default-features --locked --offline -j1 --test chart_grade` exited 0 on macOS arm64: 1 test passed (27.96 s). `git diff --check` passed. The focused suite exercises ordinary grading, not a stalled split; no redistributable two-face STEP fixture exists for that branch. The ABC `00006600` near-coincident spline-edge projection remains refused. This correction supplies no OCCT parity, solver-quality, or Windows runtime mesh/solve evidence. `occt_retirement_approved` remains false.

The full `cargo test -p solid-kernel --no-default-features --locked --offline -j1` suite also exited 0: 297 passed, 4 ignored across 39 suites.
