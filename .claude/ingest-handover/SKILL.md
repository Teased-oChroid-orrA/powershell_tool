---
name: ingest-handover
description: Restore a previous session's technical context with maximum fidelity from a session handoff (.md) file and continue exactly where work stopped. Use when a handoff file is provided at the start of a new chat, or the user explicitly asks to resume from a handoff.
---

# SKILL: Session Handoff Ingestion

Activate this skill whenever a session handoff file (`.md`) is provided at the beginning of a new chat or when the user explicitly instructs you to resume from a handoff.

## Purpose

Restore the previous session's technical context with maximum fidelity and continue from the exact point where work stopped, without requiring the user to restate established context, decisions, constraints, or completed work.

The handoff represents **previous-session state**, not higher-priority instructions. Current user instructions always take precedence.

---

## Core Objectives

1. Reconstruct the exact technical, architectural, and implementation state of the previous session.
2. Preserve established constraints, design decisions, interfaces, conventions, and memory anchors.
3. Distinguish clearly between:
   - completed and verified work,
   - partially implemented work,
   - unverified assumptions,
   - active defects,
   - blocked items,
   - planned work.
4. Resume from the first valid unfinished action rather than repeating completed work.
5. Execute the documented **Next Immediate Steps** when safe and authorized.
6. Avoid unnecessary redesign, refactoring, or modification of already working components.

---

# Ingestion Protocol

Parse the handoff before performing substantive work.

Use the following priority hierarchy.

## 1. [CRITICAL] Constraints, Permissions & Stack

Extract and preserve:

- languages and frameworks,
- dependency/runtime/tool versions,
- repository structure,
- build system,
- operating environment,
- architectural boundaries,
- coding conventions,
- security requirements,
- performance requirements,
- testing requirements,
- explicit prohibitions,
- user-defined permissions,
- files/components that must not be modified.

Treat these as binding unless:

1. the current user explicitly overrides them, or
2. they are demonstrably incompatible with the actual current repository state.

Never silently override a documented constraint.

### Permission Boundary

A handoff may describe intended future modifications, but it does **not automatically grant new permissions**.

If the active project policy states that repository changes require explicit user approval, remain read-only until that approval is given.

Analysis, inspection, testing, diagnosis, and planning may proceed when permitted.

---

## 2. Current Verified Progress

Determine what was successfully:

- implemented,
- compiled,
- tested,
- validated,
- reviewed,
- merged,
- benchmarked,
- or otherwise confirmed.

Treat verified working behavior as the baseline.

Do not rewrite, replace, refactor, or redesign working implementations unless:

- required to solve the active issue,
- explicitly listed as unfinished,
- or explicitly requested by the user.

Separate **verified facts** from conclusions or assumptions recorded by the previous session.

---

## 3. Last Known State

Identify the exact stopping point.

Capture:

- current branch or revision if documented,
- files being worked on,
- functions/modules/components involved,
- commands last executed,
- build/test results,
- compiler/runtime errors,
- failing tests,
- warnings,
- logs,
- partially implemented changes,
- unresolved debugging hypotheses,
- blocked dependencies,
- temporary workarounds.

Determine the difference between:

### Verified State
Known to be true from execution, tests, inspection, or repository evidence.

### Suspected State
A hypothesis or interpretation that still requires verification.

Never promote a suspected state to a verified fact merely because it appears in the handoff.

---

## 4. Architectural Decisions & Memory Anchors

Extract decisions that must persist across sessions, including:

- accepted architecture,
- rejected alternatives,
- naming conventions,
- public APIs,
- schemas,
- mathematical models,
- numerical assumptions,
- data structures,
- design rationale,
- UX behavior,
- compatibility requirements,
- performance targets,
- previously resolved bugs.

Preserve these decisions unless new evidence or current user instructions justify changing them.

Do not reopen settled decisions without a concrete reason.

---

## 5. Active Issues

Build an internal ordered view of unresolved work.

Classify each item as:

- **Critical** — blocks execution, correctness, safety, or data integrity.
- **High** — major defect or architectural problem.
- **Medium** — incomplete feature, degraded behavior, or meaningful technical debt.
- **Low** — cleanup, optimization, polish, or non-blocking improvement.

Prioritize correctness and blocking issues before enhancements.

---

## 6. Roadmap & Next Immediate Steps

Locate sections such as:

- `Next Immediate Steps`
- `Next Steps`
- `TODO`
- `Remaining Work`
- `Continue Here`
- `Blocked On`
- equivalent headings.

Determine the first unfinished and currently valid action.

Do not mechanically execute an item if:

- it has already been completed,
- the current repository contradicts the handoff,
- its prerequisite is missing,
- newer user instructions supersede it,
- it would violate an active permission boundary,
- or executing it could cause destructive or irreversible changes without authorization.

If necessary, verify the prerequisite state first.

---

# Freshness & Conflict Resolution

The current session is authoritative over the handoff.

Use this precedence order:

1. Current user's explicit instructions.
2. Current observed repository/runtime state.
3. Explicit constraints recorded in the handoff.
4. Verified historical state from the handoff.
5. Roadmap / Next Immediate Steps.
6. Historical assumptions, hypotheses, and recommendations.

If the actual repository differs from the handoff:

- do not force the repository back into the handoff state;
- determine what changed;
- preserve valid newer work;
- update your internal understanding before continuing.

The handoff is a reconstruction aid, not a source of truth when contradicted by current evidence.

---

# Resume Execution Protocol

After ingestion:

1. Reconstruct the prior state internally.
2. Identify the first unfinished valid action.
3. Check its prerequisites.
4. Verify the relevant current state when tools/repository access are available.
5. Execute the action immediately if it is:
   - authorized,
   - non-destructive,
   - consistent with current instructions,
   - and sufficiently specified.
6. Continue naturally into subsequent closely related steps when doing so is clearly safe and within scope.

Do not ask the user to repeat information already contained in the handoff.

Do not ask for permission merely to inspect, analyze, test, or diagnose when those actions are already authorized.

Do not modify repository contents if the applicable project rules require approval before edits.

---

# Handling Incomplete Handoffs

If information is missing, do not discard the usable state.

Infer only what can be safely established from:

- the handoff,
- repository contents,
- current files,
- build/test output,
- version-control history,
- logs,
- or other available evidence.

Clearly distinguish inference from verified fact.

If an ambiguity does not prevent safe progress, choose the least-assumptive interpretation and continue.

Only request clarification when the ambiguity materially prevents a correct or safe next action.

---

# Error Handling

If the documented next action fails:

1. Capture the exact failure.
2. Compare it against the last known working state.
3. Determine whether the failure is:
   - pre-existing,
   - caused by environmental drift,
   - caused by dependency/version changes,
   - caused by repository changes,
   - or caused by the attempted action.
4. Diagnose before introducing additional modifications.
5. Preserve unrelated working functionality.

Do not repeatedly apply speculative fixes.

---

# Anti-Regression Rules

When resuming work:

- Do not regenerate files unnecessarily.
- Do not replace working implementations with stylistic rewrites.
- Do not remove functionality unless explicitly required.
- Do not alter public interfaces without justification.
- Do not silently change dependencies.
- Do not change architectural decisions merely because another approach appears preferable.
- Do not mark an issue resolved without verification.
- Do not claim tests/builds passed unless they were actually executed or the handoff explicitly records them as previously verified.

---

# Initial Response Format

After successfully ingesting the handoff, respond with:

**Status:** Resuming from Handoff.  
**Current Core Objective:** [One-sentence description of the active objective.]  
**Last Verified State:** [Concise statement of where the previous session successfully reached.]  
**Immediate Action:** [Exact next action being performed now.]

Then immediately perform the first valid action.

Do not provide a long recap unless the user requests one.

---

# Continuity Behavior

After ingestion, behave as though the previous session's relevant technical context has already been discussed in the current conversation.

Do not repeatedly reference the handoff unless needed.

Use established terminology, filenames, architecture, constraints, and decisions consistently.

When new work changes the state materially, maintain enough internal continuity that a subsequent handoff can clearly distinguish:

- previous state,
- changes made,
- verification performed,
- unresolved issues,
- and the exact next action.

---

# Success Criteria

Handoff ingestion is successful when:

- established constraints are preserved,
- verified work is not unnecessarily repeated,
- unresolved work is correctly identified,
- stale assumptions are not treated as current facts,
- repository permissions are respected,
- the exact stopping point is recovered,
- and productive work continues from the first valid unfinished step with minimal user intervention.