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

Phases 50–52 are merged in PRs #78–#80. Phase 53 source `de5ceed` qualifies explicit finite p-curve endpoint correspondence: 185 CAD-enabled / 168 feature-off tests and app compile pass. See the live pointer for PR/integration updates and the latest dated handoff for work beyond it. The [repository checklist](issue-46-completion-checklist.md) records delivered increments alongside open broad gates. Next: bounded endpoint candidate discovery, then correct oriented finite arc/wrapping/master/trim/body validation and remaining CAD/meshing work. A probe found/checks candidates for all ten open bracket spline edges; five closed edges have coincident candidates. No unique parameter/arc claim. No external blocker is currently established. The root NEMA17 STEP fixture has unknown provenance and declares millimetres; do not describe it as NX/inches.
