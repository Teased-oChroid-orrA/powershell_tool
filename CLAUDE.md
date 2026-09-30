# CLAUDE.md

Context for Claude Code (or any fresh agent) picking up this repository with
no memory of how it got here. Read this before making changes.

**Before modifying code in a subdirectory, read its `AGENTS.md` first** (see
Downlinks below). This file only covers what's true project-wide; every
crate's own contracts, pitfalls, and code map live in its own node.

## Bug-Fix Policy (overrides default scope control for this project)

Any bug or issue discovered while working a user prompt in this repository
must be **fixed in the same session**, not merely documented for later -
even when it is unrelated to the task the user actually asked for, provided
it is a genuine, verified defect (not a speculative "would be nice") and the
fix is safe (no destructive/irreversible action, no scope requiring product
judgment only the user can make).

This is a deliberate, explicit override of the general "don't fix unrelated
things, document and move on" scope-control default (root
`~/.claude/CLAUDE.md`'s engineering-orchestrator routing still applies for
*how* to fix it - investigate, verify, test - just not for *whether*). The
project has already shipped at least one same-root-cause bug in multiple
places (the Windows Caps-Lock/Ctrl-key `KeyCode::Char` case bug, see
`app-tui/AGENTS.md`'s Pitfalls) because a prior fix documented the remaining
instances instead of fixing them, and they stayed broken until the next
session happened to look.

Practical effect:
- If CodeGraph/grep/review surfaces the same bug class elsewhere while fixing
  the one the user reported, fix all confirmed instances in the same pass,
  not just the reported one.
- Still classify and verify each instance before fixing it (don't
  mass-apply an unverified pattern-match) - "fix, don't document" is about
  not deferring known-good fixes, not about skipping verification.
- Still respects "Ask First" / "Never" boundaries in this file and in each
  crate's `AGENTS.md` - fixing everything found does not mean bypassing
  those.
- If a found issue is too large/risky to fix safely in-session (needs a
  product decision, touches a frozen reference tier, requires Windows
  hardware access, etc.), that is the exception: document it clearly
  (`AGENTS.md` Pitfalls or `docs/`) and say so explicitly, rather than
  silently fixing a smaller version of it.

## What this project is

A native Windows desktop app that recursively searches a folder for keyword
filters across `.txt`, `.log`, `.docx`, `.pptx`, `.xlsx`, `.zip` (recursing
into entries, including nested zips), `.rtf`, `.pdf`, and dozens of other
code/config/data extensions, producing an HTML report plus optional CSV/JSON
export. It has grown into a multi-tool "Toolbench" dashboard: Search (the
original, fully-functional tool), a Bushing Workbench and Pressure Vessel
Analyzer (engineering solvers for GS Engineering), and — in the newer UI head
only — a PINN-based Stress Solver.

**The project is mid-migration, three tiers deep: PowerShell -> C#/WinUI ->
Rust.** Each older tier is kept as a working, byte-for-byte-tested reference
during the transition, never deleted, never called into from the newer tier.
See `src/AGENTS.md` for the C#/WinUI tier's rules.

**`app-tui/` (ratatui/crossterm terminal UI) is now the sole actively-
developed GUI head.** Two earlier Rust GUI heads, `app/` (dioxus-native/
Blitz) and `app-egui/` (egui/eframe, including its own PINN/AMR Stress
Solver integration), have been deleted from this repository entirely — a
deliberate decision, not an oversight, made once `app-tui/` reached feature
parity on everything it implements. This is a real, accepted feature loss
in two areas `app-tui/` deliberately does not replicate (a terminal can't
render them the same way): the axial cross-section sketches /
Pressure-Vessel derivation view both former heads had, and `app-egui/`'s
PINN/AMR Stress Solver tool (its actual solver source lives entirely in the
separate sibling repo `NeuralNetwork-Stress-Solver`, untouched by this
removal — `app-egui/` only ever consumed it as a path-dependency library).
If either capability is ever wanted again, it would need a fresh port into
`app-tui/` or a new GUI head — there is nothing left in this repository to
resurrect.

`app-tui/` is additive and partial in its own right (Search Files, Fastener
Holes, Bushing Workbench, Pressure Vessel Analyzer, and Preload Analysis
toolboxes so far; Dupes/Rename/Logs remain placeholders). Fastener Holes and Preload
Analysis are unique to `app-tui/` (no equivalent in `app`/`app-egui`) —
Preload Analysis is a fastened-joint installation/preload mechanics solver
(`fastened-joint-solver/`, a new solver crate: full V-thread torque
equation, uniform-pressure/uniform-wear bearing friction, Brent-root-solved
torque-preload equilibrium, fastener/member elastic compliance including a
numerically-integrated Rotscher pressure-cone member model, nut rotation,
stress state, and service-load/separation/slip checks). Bushing Workbench
and Pressure Vessel Analyzer are ported from both existing heads' own
`bushing-solver`/`pressure-vessel-solver`-backed tools, minus their
cross-section sketches and (for Pressure Vessel Analyzer) derivation view
(visual presentation with no terminal equivalent), plus several
capabilities neither existing head has: thermal stress (a real addition to
`pressure-vessel-solver` itself, not just new UI — see
`pressure-vessel-solver/src/thermal.rs`), a text-only Numbers panel per
toolbox, inline field validation hints, report export, and (Pressure
Vessel Analyzer only) a filterable material picker with a persisted "add
custom material" form. See `app-tui/AGENTS.md`.

## Intent Layer

> TL;DR: Rust-first, GS Engineering "Toolbench" desktop app (search + engineering
> solvers), migrating PowerShell -> C#/WinUI -> Rust, with `app-tui` (terminal
> UI) as the sole active GUI head.
> Find your area in Subsystems, then read its `AGENTS.md`.

### Subsystems

| Area | Location | Node | Status |
|------|----------|------|--------|
| Search/matching/extraction core | `search-core/` | `search-core/AGENTS.md` | Active. Zero GUI deps. |
| Fast re-search index engine | `native-search/` | `native-search/AGENTS.md` | Active. Tantivy-backed. |
| ratatui GUI head (sole active GUI head) | `app-tui/` | `app-tui/AGENTS.md` | Active. Search Files + Fastener Holes + Bushing Workbench + Pressure Vessel Analyzer + Preload Analysis toolboxes; Dupes/Rename/Logs not yet migrated. |
| Bushing press-fit solver | `bushing-solver/` | `bushing-solver/AGENTS.md` | Active. Consumed by `app-tui`. |
| Legacy C#/WinUI app | `src/` | `src/AGENTS.md` | Frozen reference, do not extend. |
| Design/history docs + ADRs | `docs/` | `docs/AGENTS.md` | Append-only historical record + navigation index. |
| CLI | `cli/` | *(no node — small)* | Second `search-core` consumer, proves it's GUI-free-usable. |
| Shared math (Lamé, materials) | `engineering-math/`, `mechanics-core/` | *(no node — small)* | Extracted from `bushing-solver`; no re-export shim, import directly. |
| Pressure vessel solver | `pressure-vessel-solver/` | *(no node — small)* | Sibling pattern to `bushing-solver`. |
| Fastened joint preload solver | `fastened-joint-solver/` | *(no node — small)* | Sibling pattern to `bushing-solver`/`pressure-vessel-solver`; consumed only by `app-tui`'s Preload Analysis toolbox so far. |
| Original PowerShell tool | `powershell/` | *(no node — reference only)* | Never called from Rust; diff-only artifact. |

## Downlinks

| Area | Node | What's there |
|------|------|--------------|
| search-core | `search-core/AGENTS.md` | matching/extraction/orchestrator internals, PDF CID-font pitfall, testing gate |
| native-search | `native-search/AGENTS.md` | Tantivy engine, `ErrorInThread` recovery, FFI (legacy-only) |
| app-tui | `app-tui/AGENTS.md` | ratatui dashboard, AppState/AppEvent/Effect event-reducer pattern, Search Files (partial parity)/Fastener Holes/Bushing Workbench/Pressure Vessel Analyzer/Preload Analysis toolboxes |
| bushing-solver | `bushing-solver/AGENTS.md` | Press-fit/Lamé math, imperial-only unit risk, differential-test-against-TS-original discipline |
| src | `src/AGENTS.md` | Frozen C#/WinUI reference, its own test gate, fixture-sharing contract with search-core |
| docs | `docs/AGENTS.md` | ADR index, epic/issue phase-history index, live-vs-closed doc classification |

## Why the migration happened

WinUI 3 cannot run, build, or be debugged on a non-Windows machine at all —
every UI iteration needed a Windows CI round-trip (tens of minutes each). A
real bug (`EnableMsixTooling=false` silently disabling `resources.pri`
generation) took three blind CI round-trips to diagnose — local reproduction
would have caught it in seconds. Rust was chosen specifically to close that
loop: build, run, and debug the whole app on any platform. Two earlier Rust
GUI heads (`app/`, dioxus-native/Blitz, and `app-egui/`, egui/eframe) were
tried and later deleted once `app-tui/` (a terminal UI, which needs no GUI
toolkit or windowing system at all — the simplest possible way to satisfy
"build/run/debug on any platform") reached feature parity on everything it
implements; see root `CLAUDE.md`'s "What this project is" section above for
what was and wasn't carried forward.

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

### Purpose

CodeGraph MCP is installed, initialized, and connected to this project
(`.codegraph/` at the repo root, machine-local — see its own `.gitignore`).

CodeGraph is a required architectural and code-relationship analysis tool
for this project. It must be used whenever understanding relationships
between code elements could affect the correctness, safety, completeness,
or maintainability of a change.

Claude must **not assume that an MCP being connected means it has actually
been used**. For applicable tasks, CodeGraph must be explicitly consulted
before making implementation decisions.

The goal is not to use CodeGraph mechanically. The goal is to use it to
understand the existing system — three migration tiers, one active GUI
head, and several shared solver crates — before changing it.

---

### 1. Mandatory CodeGraph Usage

Use CodeGraph before making decisions involving:

* architecture
* crate/module boundaries (`search-core` vs `native-search`, solver crates
  vs shared math crates)
* dependencies
* call relationships
* data flow
* control flow
* numerical computation flow (Lamé/press-fit/pressure-vessel calculations)
* solver behavior
* convergence/tolerance behavior
* state management (`AppState` in `app-tui`)
* UI-to-backend interactions
* refactoring
* API changes
* removing code
* replacing implementations
* consolidating duplicate functionality
* determining whether an existing module can be reused
* adding a new subsystem
* modifying an existing engineering toolbox (bushing/pressure-vessel
  solvers)
* modifying shared numerical/materials/precision infrastructure
  (`engineering-math`, `mechanics-core`)
* changing behavior that may affect multiple toolboxes

CodeGraph should be treated as the **primary source for understanding code
relationships**, while normal repository inspection remains necessary for
understanding implementation details.

---

### 2. Required Workflow

For any substantial coding task, follow this sequence:

#### Phase 1 — Understand

1. Inspect the current local Git state.
2. Identify the relevant branch and working-tree state.
3. Use CodeGraph to identify the relevant:

   * callers
   * callees
   * types
   * crates/modules
   * dependencies
   * implementations
   * data/control-flow relationships
4. Inspect the actual source files involved.
5. Identify existing functionality that may already solve part or all of
   the problem — check `search-core`, `engineering-math`/`mechanics-core`,
   and `app-tui` before writing new code.

Do not begin implementation merely because a file with a matching name has
been found.

---

#### Phase 2 — Evaluate

Determine:

* What the existing architecture is actually doing.
* Which existing abstractions should be reused.
* Whether the requested functionality already exists in another form (in a
  shared crate, or in the frozen `src/`/`powershell/` reference tiers —
  read-only, never a source to port live code from).
* Whether apparently separate implementations are actually duplicates.
* What code will be affected by the proposed change.
* Whether the proposed change crosses crate/API boundaries.
* Whether there are hidden callers or dependencies.
* Whether tests or other tools depend on the existing behavior.

For numerical code, additionally determine:

* Where inputs originate (unit assumptions — `bushing-solver` is
  imperial-only, see its `AGENTS.md`).
* Where units/conversions are applied.
* Where intermediate calculations occur.
* Where numerical precision is handled.
* Where convergence/tolerance criteria are calculated.
* Where solver state is updated.
* Where results are consumed/displayed.
* Whether multiple tools duplicate the same numerical logic.

---

### 3. Planning Requirements

Before creating or substantially modifying an implementation plan, use
CodeGraph to establish the relevant architecture.

The plan must be based on the **actual repository relationships**, not
assumptions based solely on:

* filenames,
* directory structure,
* text search results,
* documentation,
* or the original task description.

The plan should identify:

* existing modules to reuse,
* modules requiring modification,
* affected callers,
* affected dependents,
* potential duplicate implementations,
* likely regression areas,
* and required validation.

If CodeGraph reveals that the original plan is based on an incorrect
architectural assumption, update the plan.

Do not blindly follow an earlier plan simply because it already exists.

---

### 4. Trust but Validate

Existing code, plans, previous AI-generated work, and developer assumptions
are **inputs to evaluate, not automatically authoritative**.

This applies equally to:

* existing implementation,
* previous Claude Code changes,
* user-created changes,
* generated code,
* documentation,
* architectural assumptions,
* optimization proposals,
* numerical approaches,
* and previous plans.

Use CodeGraph and repository evidence to determine whether those
assumptions are correct.

Do not preserve an existing implementation merely because it already
exists.

Do not replace an existing implementation merely because a new
implementation appears cleaner.

Make the decision based on evidence.

---

### 5. Existing Functionality Must Be Reused Where Appropriate

Before creating a new implementation of functionality that may already
exist:

1. Query CodeGraph for related implementations and relationships.
2. Inspect the existing implementation.
3. Determine whether it can be reused directly.
4. If not, determine whether it should be refactored into a reusable
   module.
5. Update existing consumers as appropriate.
6. Only create a parallel implementation when there is a documented
   technical reason.

Avoid parallel implementations of the same engineering or numerical
concept.

Examples include:

* materials calculations
* Lamé/press-fit and pressure-vessel equations
* precision/display rules
* unit conversion
* numerical solvers
* convergence/tolerance detection
* engineering-property calculations
* geometry calculations
* tolerance calculations
* common validation
* shared UI components
* search/extraction logic (must live in `search-core`, never duplicated
  into a GUI head)

Prefer one authoritative implementation with well-defined interfaces.

---

### 6. Duplicate-Code Detection

Before adding substantial new code, use CodeGraph to determine whether
equivalent or overlapping functionality already exists.

After implementation, use CodeGraph again to look for:

* obsolete implementations,
* redundant call paths,
* duplicate calculations,
* unused abstractions,
* bypassed shared modules,
* and functionality that should now be consolidated.

Remove obsolete code when it is safe to do so.

Do not leave old implementations in place merely because they might
theoretically be useful — except the frozen `src/` and `powershell/`
reference tiers, which are intentionally kept as diff-only artifacts (see
"Reference-only tiers" above) and must never be deleted or extended.

Before removing code, verify that it is not required by:

* callers,
* tests,
* configuration,
* public APIs,
* serialization,
* generated code,
* feature flags,
* or planned functionality.

---

### 7. Local Repository Is the Source of Truth

For code analysis and implementation, the **current local repository state
takes precedence over GitHub's remote state**.

This is especially important when:

* local commits have not been pushed,
* feature branches differ from remote branches,
* the working tree contains changes,
* or the user is experimenting locally.

Do not assume GitHub contains the latest implementation.

When reviewing a task involving Git history:

1. Inspect local Git state.
2. Identify local-only commits.
3. Identify uncommitted changes.
4. Compare against the relevant remote branch.
5. Inspect GitHub separately for remote context.
6. Clearly distinguish local code from remote code.

Never overwrite or discard local work merely to synchronize with GitHub
unless explicitly instructed.

---

### 8. CodeGraph Index Freshness

Do not assume that CodeGraph automatically reflects every local
modification.

Before relying on CodeGraph for an important architectural decision,
determine whether its index reflects the relevant current local repository
state.

If the index is stale:

1. Refresh/reindex it when supported.
2. Re-run the relevant CodeGraph queries.
3. Verify important relationships against the actual source files.

If CodeGraph cannot represent a recent change, use direct source
inspection as the fallback.

Never present stale CodeGraph information as current repository truth.

---

### 9. Debugging Requirements

For non-trivial bugs, CodeGraph must be used to trace the relevant
execution path before changing code.

For example, for a solver correctness problem, trace:

```text
GUI input (app-tui)
        ↓
solver construction (bushing-solver / pressure-vessel-solver)
        ↓
unit/geometry validation
        ↓
Lamé constants / stress calculation (engineering-math, mechanics-core)
        ↓
failure-criteria / safety-factor evaluation
        ↓
result formatting/precision
        ↓
report/export
```

For a search/extraction problem, trace:

```text
CLI or GUI request
        ↓
orchestrator (search-core)
        ↓
per-format extraction (docx/pptx/xlsx/zip/pdf/...)
        ↓
keyword matching
        ↓
progress reporting
        ↓
report generation (HTML/CSV/JSON)
```

The actual path will vary by implementation.

Use CodeGraph to determine the real relationships rather than assuming
this structure exists.

Then inspect the actual implementations and validate the behavior
experimentally.

Do not "fix" a numerical or search discrepancy merely by:

* loosening tolerances or convergence criteria,
* increasing iteration/retry limits,
* suppressing warnings,
* masking failures,
* changing thresholds without justification,
* or otherwise making failure less visible.

First determine the underlying cause.

---

### 10. Numerical Engineering Requirements

For numerical and engineering code (`bushing-solver`,
`pressure-vessel-solver`, `engineering-math`, `mechanics-core`), CodeGraph
analysis must be combined with mathematical and numerical validation.

Before modifying numerical behavior, determine:

* the complete calculation path,
* shared versus tool-specific calculations,
* input/output relationships,
* unit handling (imperial-only risk — see `bushing-solver/AGENTS.md`),
* precision handling,
* convergence/tolerance behavior,
* failure modes,
* and downstream consumers (`app-tui`).

Do not infer mathematical correctness from code structure alone.

Validate important numerical changes using appropriate:

* analytical cases,
* known solutions,
* limiting cases,
* dimensional checks,
* independent calculations,
* differential tests against the original TypeScript/C# implementation
  (`bushing-solver`'s documented discipline),
* regression tests,
* or benchmarks.

Where a change is intended to improve convergence or performance, measure
it.

---

### 11. Engineering Toolbox Requirements

For each engineering toolbox (bushing press-fit, pressure vessel):

1. Use CodeGraph to identify related existing modules.
2. Search for existing calculations that can be reused.
3. Identify shared infrastructure (`engineering-math`, `mechanics-core`).
4. Determine whether the functionality belongs in:

   * the toolbox,
   * a reusable numerical module,
   * a reusable engineering module,
   * or shared infrastructure.
5. Implement only the minimum toolbox-specific logic necessary.
6. Verify that shared functionality remains centralized.

The toolbox should not independently reimplement calculations that belong
in shared engineering modules.

---

### 12. Precision and Numerical Display

Precision handling is a shared concern.

Before adding or modifying precision/display behavior:

1. Use CodeGraph to identify all existing precision-related
   implementations.
2. Determine which components perform mathematical calculations.
3. Determine which components perform display formatting.
4. Keep mathematical precision separate from presentation precision.
5. Preserve full precision for intermediate calculations.
6. Apply display rounding only at the appropriate final-display boundary.
7. Avoid duplicating precision policy inside individual toolboxes.

Changes to the shared precision system must be evaluated for downstream
effects on `app-tui`.

---

### 13. Materials and Shared Engineering Libraries

Before implementing a calculation requiring material properties:

1. Use CodeGraph to locate existing materials functionality.
2. Determine how existing tools consume it.
3. Determine whether it is already sufficiently modular
   (`engineering-math`/`mechanics-core` were already extracted from
   `bushing-solver` for this reason — import directly, no re-export shim).
4. If it is not appropriately modular, refactor it into a self-contained
   reusable module where justified.
5. Update existing consumers to use the authoritative implementation.
6. Do not create a second materials database or calculation path.

The same principle applies to other shared engineering functionality.

---

### 14. Verification After Implementation

After a substantial implementation, use CodeGraph again.

Verify:

* affected callers,
* affected dependents,
* newly introduced relationships,
* obsolete relationships,
* duplicate implementations,
* unintended bypasses of shared modules,
* and architectural consistency.

Then run the appropriate tests and validation (`cargo test -p search-core`
at minimum; `app-tui`'s own build/rendering verification where UI changed).

CodeGraph verification does **not** replace testing.

Testing does **not** replace CodeGraph analysis.

Both provide different forms of evidence.

---

### 15. When CodeGraph Cannot Be Used

If CodeGraph is:

* disconnected,
* unavailable,
* stale,
* unable to index the relevant code,
* unable to answer the required relationship query,
* or otherwise unusable,

do not fabricate CodeGraph results.

Instead:

1. State internally/briefly that CodeGraph could not provide the required
   analysis.
2. Fall back to direct repository inspection and other available tools.
3. Continue only when sufficient evidence can be obtained another way.
4. If the task depends critically on information CodeGraph should
   provide, flag that limitation before making a high-risk architectural
   change.

Do not claim that CodeGraph was consulted when it was not.

---

### 16. Mandatory CodeGraph Checkpoint

For every substantial task, before implementation, answer these questions:

* Did I use CodeGraph?
* Did I use it on the relevant local repository state?
* Did I identify callers/dependents of the code I intend to change?
* Did I check for existing implementations that should be reused?
* Did I identify potentially affected components?
* Did I inspect the actual source after the graph analysis?
* Did I validate important CodeGraph findings against the source?

If the answer to any applicable question is no, perform the missing
analysis before proceeding.

---

### 17. Final Engineering Principle

The preferred workflow is:

**Understand → CodeGraph → Inspect → Validate → Plan → Implement → Test →
CodeGraph Re-check → Clean Up → Final Validation**

Do not use:

**Guess → Search for a convenient file → Modify → Hope nothing else
depends on it**

CodeGraph is a required part of understanding the architecture, but it is
**not an authority on correctness**.

The authoritative result comes from combining:

* CodeGraph relationships,
* actual source code,
* Git history,
* tests,
* numerical validation,
* benchmarks where applicable,
* and the stated requirements.

Use all of these together to make engineering decisions.

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
