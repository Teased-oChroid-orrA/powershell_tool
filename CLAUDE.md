# CLAUDE.md

> TL;DR: Rust-first GS Engineering Toolbench (search + engineering solvers), `app-tui` is the sole GUI head. Fix-Everything Policy applies to every request.

Context for Claude Code (or any fresh agent) picking up this repository with
no memory of how it got here. Read this before making changes.

**Before modifying code in a subdirectory, read its `AGENTS.md` first** (see
Downlinks below). This file only covers what's true project-wide; every
crate's own contracts, pitfalls, and code map live in its own node.

## Fix-Everything Policy (overrides default scope control for this project)

For **every** user request, any genuine, verified bug, defect, or worthwhile
improvement discovered while working - related to the request or not - must
be **worked in the same session**, not merely documented for later. This
includes: bugs, same-root-cause instances elsewhere, stale or wrong docs,
dead/duplicate code, missing regression tests, and clear, low-risk
improvements (correctness, robustness, performance, clarity) that need no
product judgment.

This deliberately overrides the general "don't fix unrelated things,
document and move on" default (root `~/.claude/CLAUDE.md`'s
engineering-orchestrator routing still governs *how* - investigate, verify,
test - not *whether*). Motivating incident: the Windows Caps-Lock/Ctrl-key
`KeyCode::Char` case bug shipped in several places because an earlier fix
documented the remaining instances instead of fixing them.

Rules:
- Classify and verify each finding before fixing it; never mass-apply an
  unverified pattern-match. Same bug class elsewhere -> fix every confirmed
  instance in the same pass.
- Improvements must be real and evidenced (measured, reproduced, or clearly
  demonstrable), not speculative "would be nice" or taste-only rewrites.
- Add or extend a regression test for every fix where practical.
- Report everything fixed beyond the request, separately and briefly, in the
  final summary.
- Still bound by "Ask First"/"Never" below and each crate's `AGENTS.md`.
- Exception: if a finding is too large or risky for one session (needs a
  product decision, touches a frozen reference tier, needs Windows hardware,
  etc.), document it clearly (`AGENTS.md` Pitfalls or `docs/`) and say so
  explicitly - never silently fix a smaller version of it.

## What this project is

A native Windows desktop app that recursively searches a folder for keyword
filters across text, Office (`.docx`/`.pptx`/`.xlsx`), `.zip` (including
nested), `.rtf`, `.pdf`, and dozens of code/config/data formats, producing an
HTML report plus optional CSV/JSON export. It has grown into a multi-tool
"Toolbench" for GS Engineering: Search, Fastener Holes, Bushing Workbench,
Pressure Vessel Analyzer, and Preload Analysis (placeholders remain for
Dupes/Rename/Logs).

**Mid-migration, three tiers deep: PowerShell -> C#/WinUI -> Rust.** Older
tiers are kept as byte-for-byte-tested references, never deleted, never
called from the newer tier (see "Reference-only tiers").

**`app-tui/` (ratatui/crossterm) is the sole active GUI head.** The earlier
Rust heads `app/` (dioxus-native) and `app-egui/` (egui) were deleted on
purpose once `app-tui/` reached parity on what it implements. Accepted
losses: axial cross-section sketches / Pressure-Vessel derivation view (a
terminal cannot render them) and the PINN/AMR Stress Solver (its source is
the separate sibling repo `NeuralNetwork-Stress-Solver`; nothing here
consumes it). Reviving either needs a fresh port; nothing remains to
resurrect. Per-toolbox detail lives in `app-tui/AGENTS.md` and its child
nodes.

## Intent Layer

> TL;DR: Rust-first, GS Engineering "Toolbench" desktop app (search +
> engineering solvers), migrating PowerShell -> C#/WinUI -> Rust, with
> `app-tui` as the sole active GUI head. Find your area below and read its
> `AGENTS.md` **before modifying code there**.

## Downlinks

| Area | Location | Node | Status |
|------|----------|------|--------|
| Search/matching/extraction core | `search-core/` | `search-core/AGENTS.md` | Active. Zero GUI deps. |
| Fast re-search index engine | `native-search/` | `native-search/AGENTS.md` | Active. Tantivy-backed. |
| ratatui GUI head | `app-tui/` | `app-tui/AGENTS.md` + per-toolbox `app-tui/src/toolboxes/{search,fastener_hole,bushing,pressure_vessel,preload_analysis}/AGENTS.md` | Active. Sole GUI head. |
| Bushing press-fit solver | `bushing-solver/` | `bushing-solver/AGENTS.md` | Active. Consumed by `app-tui`. |
| Fastened joint preload solver | `fastened-joint-solver/` | `fastened-joint-solver/AGENTS.md` | Active. Consumed by `app-tui` Preload Analysis. |
| Pressure vessel solver | `pressure-vessel-solver/` | *(no node)* | Sibling pattern to `bushing-solver`. |
| Shared math (Lamé, materials) | `engineering-math/`, `mechanics-core/` | *(no node)* | No re-export shim; import directly. |
| CLI | `cli/` | *(no node)* | Second `search-core` consumer. |
| Design/history docs + ADRs | `docs/` | `docs/AGENTS.md` | Append-only record + index. |
| Legacy C#/WinUI app | `src/` | `src/AGENTS.md` | Frozen reference, do not extend. |
| Original PowerShell tool | `powershell/` | *(reference only)* | Never called from Rust. |

## Why the migration happened

WinUI 3 cannot run, build, or be debugged off Windows, so every UI iteration
needed a tens-of-minutes CI round-trip. Rust closes that loop: build, run,
and debug on any platform. `app-tui/` (a terminal UI, no windowing system)
is the simplest way to satisfy that, which is why the two GUI-toolkit heads
were dropped.

## Reference-only tiers — hard boundary

Neither `powershell/` nor `src/` (the C#/WinUI app) has a runtime or build
dependency from the active Rust stack, and nothing in Rust calls out to
either. **Never** add a PowerShell invocation, a C#/.NET reference, or any
shell-out to either from Rust code. **Never** add new features to `src/` —
if something's missing from the Rust port, port it into `search-core`/
`app-tui` instead. They exist only so behavior can be diffed against if a
discrepancy is ever suspected. See `src/AGENTS.md` for the full rule set and
test gate.

## Global Invariants

- **Target environment (do not relax without discussion)**: Windows 10
  1809+ / Windows 11, `win-x64`. No internet access, no admin rights, no
  pre-installed runtime of any kind required on the machine running the
  *built* app. Build-time internet access (crates.io/NuGet restore in CI) is
  fine — only the published, running application must be fully
  self-contained and offline-capable.
- **Live progress reporting is a hard requirement, not a nice-to-have**: this
  app exists partly because the original PowerShell tool's PDF processing
  would go silent for many seconds with no way to tell "still working" from
  "stuck." Any change to search/extraction internals must preserve per-file
  progress reporting, a background ticker during parallel runs, and per-file
  in-flight status — never collapse this into a simpler "start/done" event
  model. Full detail in `search-core/AGENTS.md`.
- **A PINN/AMR Stress-Solver tool no longer exists anywhere in this repo.**
  It was previously integrated into the now-deleted `app-egui/` GUI head,
  which consumed the actual solver as a path dependency on a separate
  sibling repository, `NeuralNetwork-Stress-Solver` — that sibling repo,
  and its PINN training/AMR/Kt-convergence internals, are untouched by
  `app-egui/`'s removal, but nothing in this repo consumes it anymore.

## Global Pitfalls

- **Shared Cargo target directories can exhaust disk space** — a real
  incident: `.shared-cargo-target` (24GB) + `powershell_tool/target` (13GB)
  together filled a disk. Watch usage on this workspace, especially before
  long training/build runs, and don't assume `cargo clean` is unnecessary
  just because a build "worked last time."
- **(Historical, C#-era, still worth knowing if you ever script an
  archive/export of this repo)**: an XML comment containing `--` once broke
  a `.csproj` file outright, and a `zip -x "*.git*"` packaging command once
  silently excluded the entire `.github/` folder via substring wildcard
  matching. Use exact path exclusions, not bare substring wildcards, and
  diff the result.

## CodeGraph-First Engineering Workflow

CodeGraph (`.codegraph/` at the repo root, machine-local) is the primary
tool for code relationships. Full runbook: `docs/codegraph-workflow.md`.
Binding rules:

- **Use it before deciding**, not just when connected: query callers,
  callees, types, and dependents before architecture, API, refactor, removal,
  or shared-crate changes (`engineering-math`, `mechanics-core`, solver
  crates, `search-core`). Then read the actual source; the graph is not an
  authority on correctness.
- **Check freshness.** If the index may be stale, reindex and re-query, or
  fall back to source. Never present stale graph output as current, and never
  claim to have used CodeGraph when it was unavailable.
- **Local repo is the source of truth** over GitHub; never discard local work
  to sync.
- **Reuse before writing.** Look for existing implementations (materials,
  Lamé/press-fit/pressure-vessel math, precision/display, unit conversion,
  solvers, search/extraction in `search-core`) and keep one authoritative
  path. Re-check afterward for duplicates and bypassed shared modules.
- **Debug by tracing the real path** (GUI input -> solver -> validation ->
  stress/failure -> formatting -> export; or request -> orchestrator ->
  extraction -> matching -> progress -> report). Never "fix" a numerical or
  search discrepancy by loosening tolerances, raising iteration limits, or
  masking failures; find the cause first.
- **Numerical changes need numerical proof**: analytical/limiting cases,
  dimensional checks, differential tests (bushing-solver's discipline), and
  a measurement if the goal is speed or convergence. Keep full precision in
  calculations; round only at the display boundary; one precision policy.
- **Verify after**: re-check callers/dependents, then run tests. Neither
  replaces the other.

## Branching

Work happens on `main`. Use a short-lived sub-branch only for genuinely risky or long-running work, and delete it once merged. Do not leave stale branches on the remote.

## Diagnosing Windows-only failures

The app writes `toolbench-debug.log` into the folder it was launched from (`app-tui/src/debug_log.rs`). Ask for it first for any index/search report that does not reproduce on macOS.

## Boundaries

### Always
- Run `cargo test -p search-core` before considering any `search-core`
  change done (zero GUI dependency, runs anywhere).
- Actually run/screenshot `app-tui` before claiming a layout or rendering
  fix works — `cargo check`/`cargo test` alone cannot verify a rendered
  TUI, and a `TestBackend` unit test can miss real-terminal-only failure
  zones (see `app-tui/AGENTS.md`'s Pitfalls for a confirmed example). A
  real `tmux` session has been used for this before; don't assume
  `TestBackend` coverage alone proves a rendering fix without checking.

### Ask First
- Deleting anything under `docs/` — it's an append-only historical record
  (see `docs/AGENTS.md`).

### Never
- Add a PowerShell invocation, C#/.NET reference, or shell-out to `src/` or
  `powershell/` from Rust code.
- Add new features to `src/` (the frozen C#/WinUI reference app).
- Re-derive or duplicate the PINN/AMR narrative (now entirely in the
  separate `NeuralNetwork-Stress-Solver` sibling repo, with no consumer left
  in this one — see "What this project is" above) into this repo's docs.
  (This root file itself used to violate this rule — over 1400 lines of
  PINN/AMR epic history accumulated here before an earlier restructuring.
  Don't let it happen again: durable per-crate facts go in that crate's
  `AGENTS.md`; one-off epic narratives go in `docs/issue-N-*.md`, never
  here.)

## Architecture Decisions

Formal ADRs live in `docs/adr/` (ADR-001 through ADR-011, covering the
Rust/native-search architecture boundary, Tantivy adoption, extraction
architecture, and index-location decisions). See `docs/AGENTS.md` for the
full index mapping decision topics to ADR numbers.

## Entry Points

| Task | Start Here |
|------|------------|
| Change search/matching/extraction logic | `search-core/AGENTS.md` |
| Change the fast re-search index | `native-search/AGENTS.md` |
| Change the terminal (app-tui) GUI | `app-tui/AGENTS.md` |
| Change bushing/pressure-vessel solvers | `bushing-solver/AGENTS.md` |
| Understand why an architecture decision was made | `docs/AGENTS.md` -> `docs/adr/` |
| Understand a past epic's implementation history | `docs/AGENTS.md` -> `docs/issue-N-*.md` |
| Run the C#/WinUI reference app's own tests | `src/AGENTS.md` |
