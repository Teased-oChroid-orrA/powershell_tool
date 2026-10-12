# ADR-020: Failure guidance - diagnosis, ranked remedies, intent questions

Status: **Accepted** (UI mock approved by the user 2026-10-11; core, card, status-bar fix, undo, recording and ranker wiring built and tested).

## Context

A failed solve today ends in one string in the status bar ("the model is not fully constrained (2 of 3 rigid-body motions are
held): add supports"). The user asked for: (a) ranked resolutions after a failure, applied with one action; (b) hints during
creation that prevent the failure; (c) questions about intent when the model alone cannot say; (d) one mechanism, not one per
toolbox or problem type.

Evidence gathered by reproducing real failures (`fea-problem/tests/failures.rs`):
- Failures are plain strings. Parsing them for numbers is brittle; the state of the problem is the reliable evidence.
- **The kernel's model check (`fea-core/src/validate.rs`, stable issue codes) was never called by the problem layer.** A NaN
  load solved "successfully" to zero displacement and a model with no load gave a silent zero. Fixed: `solve_mesh` now runs
  `Model::validate`; errors stop the solve with their reason, warnings (e.g. "no load ... the solution is zero") join the
  summary notes. Regression tests added.
- A coarse mesh (size 1e6) still solves without a word. That is a prevention hint (not built yet), not an error.

## Decision

1. **Diagnose from state, not from text.** `fea_problem::diagnose::diagnose(&Problem, Option<&str>) -> Vec<Diagnosis>`.
   With the solver's error it classifies the failure; with `None` it is the prevention pass and reports only what the model
   state already shows. The error text only says that a failure of a kind happened (one pinned substring for "rigid-body
   ... constrained", asserted by a test); every number in the evidence is recomputed from the problem.
2. **Remedies are data.** `Remedy { kind, label, why, edit }` with `Edit::{Advice(Boundary | MeshSize), Retarget, Drop}`
   applied by `Edit::apply(&mut Problem)`. Data can be previewed on a clone, recorded, ranked and undone; a closure cannot.
   Applying goes through the editor's history so it is undoable (UI step).
3. **Questions are data.** `Question { id, text, options: [(label, Edit)] }`, offered only when the answer changes which
   remedy is right and the model cannot say (first: "Which surface is held in reality?").
4. **The rules decide what is offered and its order; learning may only reorder, behind the existing measured gate.**
   `FailureClass` and `RemedyKind` are `Copy + Ord` so they are exactly the situation / candidate labels of the generic
   `editor/rank.rs` (`Model<S, C>`, `Store`, the held-out sign test, `Evidence::Real` vs `Simulated`). No new learning code.
   A ranking shown in the UI must say **"measured"** (passed the gate on the user's own sessions), **"simulated prior"**
   (corpus only) or **"rules"** (no model); a learned order is never presented as knowledge it does not have.
5. **Generic across problem types by construction.** The mechanism is the three data types plus `rank.rs`; each problem type
   provides its own `diagnose` (structural: `fea-problem` now; member models, bushing, lug... implement the same shape when
   they want it). The presentation (cards, status bar) is written once against the data types. No central registry until a
   second implementer exists.
6. **Built**: classes `NoSupports`, `UnderConstrained`, `UnknownSurface`, `MeshTooFine`, `MeshTooCoarse`, `HoleOutsideOutline`,
   `NonFiniteInput`, `Other`; remedies add fixed/roller support, retarget to the closest surface name, drop a reference,
   coarsen / refine the mesh, remove a hole (renumbering the bushings and bearing loads that named later holes). Proof used
   for every remedy: apply it to the failing model and the solve that failed now succeeds (`tests/failures.rs`).
   UI (`app-tui/src/gpu_viewer/editor/solid/guidance.rs`): inspector card "Solve failed" (primary) after a failure or "Check"
   (context) while building; a status-bar button applying the best fix; each fix and each question answer is one undoable
   edit; the card states the order's basis ("rules" / "simulated prior" / "measured"). The fix choice is recorded only with
   recording on (class and fix kind only) into a fourth `Ranker<FailureClass, RemedyKind>` ("Failure fixes") that obeys the
   same measured gate and Advisor menu as the others. No simulated corpus exists for it, so its order is "rules" until the
   user's own sessions pass the gate. Not built: inverted / flat elements and contact non-convergence (no real reproduction
   of either through the problem layer yet), thin features below the mesh size, unit / scale mismatch (needs a question),
   viewport preview of a fix on hover, learning which questions are useful.

## UI proposal (needs approval; nothing built)

```
Status bar (failure):  x Not fully supported - 1 fix ready          [Fix: Fix 'left'] [Why?] [Ask me]

Inspector card "Solve failed" (Relevance::Primary while the failure stands)
 +--------------------------------------------------------------+
 | Not fully supported                                          |
 | The supports leave the model free to move or turn.           |
 |   held: left: x                                              |
 |                                                              |
 | Fixes                                         ranking: rules |
 |  1. Fix 'left'                      [Apply]  (preview shows) |
 |     a rigid-body motion is still free; 'left' is farthest... |
 |  2. Hold 'left' with a roller       [Apply]                  |
 |                                                              |
 | Which surface is held in reality?                            |
 |   ( ) bottom  ( ) top  ( ) hole1                    [Apply]  |
 +--------------------------------------------------------------+
Prevention: the same card, titled "Check", appears in the Context group while building (no red status), and a surface
named in a support/load that does not exist is marked in the tree. Apply = one history step ("Fix 'left'"), Undo reverts it.
Hover on [Apply] highlights the surface in the viewport (preview).
```

## Consequences

- One mechanism for all failure kinds; adding a rule means adding a class/remedy and a regression test that reproduces the
  failure and shows the remedy fixes it.
- The recording of offers and choices uses `rank::Store`; it stays off by default and content-free (class and remedy kind
  only, no names or values).
- Because evidence is recomputed from state, a diagnosis never disagrees with the model the user is looking at.
- Risk: the one pinned substring for under-constraint; the test that pins the message is the guard.
