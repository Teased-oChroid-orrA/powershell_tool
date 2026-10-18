# Resume the native CAD and meshing statement of work

This is the current entry point for Claude Code or Codex. Historical phase reports and dated handoffs are append-only evidence; their old branch/approval statements do not override subsequent user instructions.

## Establish current state

1. Read root `CLAUDE.md`, then the relevant crate and `docs/AGENTS.md` instructions.
2. Run `git status --short --branch` and inspect local commits before fetching or switching. Preserve uncommitted work; do not reset/overwrite it or create an unsolicited worktree.
3. Read [the live execution pointer](issue-46-current.json). It records the last qualified source, tests, phase report, branch/PR integration state, last confirmed main merge and next work. `origin/main` may additionally include tracking-only commits; the pointer does not attempt to record its own commit hash.
4. Read the latest dated addenda in [the Claude handoff](issue-46-claude-handoff.md), plus the linked phase verification manifest. Inspect actual Git/PR state; a work-in-progress checkout may be ahead of the last qualified increment.
5. Read the [repository completion checklist](issue-46-completion-checklist.md), [epic #46](https://github.com/Teased-oChroid-orrA/powershell_tool/issues/46) and the relevant child issues. Checked scoped milestones are delivered; unchecked broad priority gates remain incomplete. Phase numbers count increments, not completed priority tiers.

## Scope and authorization

The user authorized continuing all remaining epic/addendum items, fixing verified defects as they arise, and merging completed qualified increments into main before proceeding on a new branch. Keep commits and a draft PR referencing the applicable issues, record exact qualification, and update the checklist/handoff at integration. Preserve unrelated user branches/PRs. No unnecessary hosted GitHub CI; tracking-only edits need none.

Priority order: P0 dependency audit/isolation, robust predicates/topology, verified telemetry and deterministic compatibility registry; P1 native B-Rep import/validation/healing, general constrained tetrahedralization and sliver/solver quality; P2 curved high-order, hybrid and metric anisotropic adaptation; P3 motion/ALE/history, advanced cost and optional learning; final verified production parity and separately approved OCCT retirement. Production still uses OCCT. Do not remove it or claim general native import/PLC meshing based on scoped diagnostic tests. Defer learned predictions until representative verified benchmarks and held-out evidence exist.

The full original [epic](epic-native-cad-advanced-meshing-source.md) and [addendum](epic-native-cad-autotuning-source.md), [dependency plan](issue-46-execution-plan.md) and [support/evidence matrix](issue-46-native-support-matrix-2026-10-16.md) preserve the requirements. `issue-46-tracking.json` is the historical initial plan, not live execution state. Issue #12 handoff records earlier work and historical branch restrictions; resume the present statement of work here.

## Verification workflow

Use the pinned/available Rust toolchain and locked dependencies. For native CAD kernel changes run focused meaningful regressions, then:

```sh
cargo test -p solid-kernel --features occt --locked --offline -j1
cargo test -p solid-kernel --no-default-features --locked --offline -j1
cargo check -p app-tui --features occt --lib --tests --locked --offline -j1
```

Cloud tools are retained in `/workspace/toolchains`: set `CARGO_HOME=/workspace/toolchains/cargo`, `RUSTUP_HOME=/workspace/toolchains/rustup`, `OCCT_ROOT=/workspace/toolchains/occt/occt-8_0_1_rev3-x86_64_unknown_linux_gnu`, `CARGO_INCREMENTAL=0`, and prepend `$CARGO_HOME/bin:/workspace/toolchains/link-tools` to PATH. These paths are specific to this Linux workspace. Inspect disk space before builds; use one Cargo process at a time. Reuse qualification for unchanged source; do not rerun builds solely for docs. For other crates follow their own required checks. A compile/test run does not establish desktop rendering or clean-machine Windows/macOS acceptance.

Capture command exit codes and actual test counts, failures, ignored tests, warnings, numerical/geometry tolerances and limitations at the exact committed source. Never relax tolerances to pass. Existing `/workspace/epic-evidence` logs are supplemental machine-local evidence; committed phase manifests/handoffs must contain enough information to resume without that directory. GitHub checklists must be updated as scoped milestones finish, without closing broad gates prematurely.

## Current milestone

Phases 50–84 are merged into `main` (PRs #78–#112; #83 and #97 closed automatically when their base branches were deleted, their commits arrived through the next PR). Full-workspace result at `main` `ddba394` (macOS arm64, `cargo test --workspace --locked --offline --no-fail-fast`): **2831 passed, 0 failed, 87 ignored**. None of Phases 54–78 ran on Windows or hosted CI.

Delivered since Phase 64 (each phase doc states its evidence and limits): smoothing, planar boundary sliding, metric-aware smoothing and edge collapse (65–68, #53/#58); generic shell cleanup, weld, registry v3 and one-call `prepare_shell` (69–71, #52/#53/#54/#64); mesh motion with rollback (72, #59); reconnect-and-snap-again boundary layer and the outward-orientation fix (73, #55); full total-cost objective with limits and break-even (74, #60); AMR feedback (75, #62); cluster-weld defeaturing (76) and opt-in surface Steiner refinement (77) (#52/#53); verified prism-tetrahedron transition (78, #56); hexahedron-tetrahedron transition and the no-pyramid decision (79, #56), plus the benchmark research record ([issue-46-benchmarks.md](issue-46-benchmarks.md), #60-#62); AMR-advice measurement on a plate with a hole, no gain (80, #62); perfect-oracle bound on any start-size advice (81, #62); held-out cost-model validation and the iterative-regime flag (82, #60); termination boundary of constrained recovery, gate kept (83, #53); Tet4/Tet10 convergence and the off-surface snapping diagnosis on a curved body (84, #55).

Open, in the order the evidence points to:
- The sampled NEMA17 bracket still fails the registry's 18 degree input-angle gate (smallest angle 0.0123 degrees at Phase 64, 0.735 after defeaturing, 1.446 with opt-in refinement; the limiting triangles lie along a thin curved band) and constrained recovery on it still exceeds its Steiner budget. Fixes must stay generic; the bracket is only an example.
- #55: validity-guarded exact CAD-parameter projection and a convergence study; #51: loop-level master/trim/body agreement, spline-surface faces, healing, assemblies and persistent associations; #56: a boundary-constrained tetrahedral mesher that follows `transition::interface_diagonal` (hex and prism transitions exist; no pyramid element by decision, Phase 79); #58: good quality at strong anisotropy, curved boundaries and AMR integration; #59: tangential sliding, ALE and the link between motion, transfer and remeshing triggers.
- #60-#62: run the benchmarks listed in `issue-46-benchmarks.md` (Kirsch hole, Mode I crack, Lamé control, NAFEMS LE1/LE10) (the Phase 75 AMR feedback gave an identical result to the default start on the L-plate benchmark, so no time-to-accuracy gain is shown), learning waits for such data, and the autotuner qualification; #63: native production parity and the separately approved OCCT retirement (`occt_retirement_approved` stays false; OCCT remains production).

PRs stay open for owner review unless the owner asks for a merge; the agent never rewrites history or force-pushes (root `CLAUDE.md`).
