# CodeGraph-First Engineering Workflow (full runbook)

Moved out of root `CLAUDE.md` to keep that file within its token budget. The binding rules are summarized in root `CLAUDE.md`; this is the long form.

## Runbook

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

