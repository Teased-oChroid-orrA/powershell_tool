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

## Addendum 2026-10-12: hover preview and the usefulness of questions

Two items listed above as not built are built:

- **Hover preview.** `Edit::surfaces(&Problem)` names the surfaces an edit acts on (the added boundary's surface, the surface a
  retarget leaves and reaches, the one a drop frees, `holeN` of a removed hole; none for a mesh size). A dock row can carry an
  `(on, off)` pair of actions (`DockItem::row_with_preview`); the egui layer sends `on` when the pointer arrives and `off` when it
  leaves, and `SolidAction::PreviewFix` lights those surfaces in the preview mesh exactly like a selection. Presentation only:
  nothing is applied, a stale reference lights nothing, applying a fix clears it.
- **Which questions are useful.** A finding that has a question offers the ranker a pseudo-kind `RemedyKind::AnswerQuestion`
  next to its fixes. Taking a fix and answering the question are both recorded as the choice among the same offered set (only
  when recording is on, content-free like every other ranker). The learned order, behind the same measured gate, decides whether
  the card shows the question before the fixes; the rules show the fixes first. Nothing is ever hidden: asking and fixing stay
  offered whichever leads. No simulated prior exists for this choice (it would be my statement of what users do): until a user's
  own sessions pass the gate the order is the rules'.

## Addendum 2026-10-12: the remaining failure classes

Each class below was reproduced as a real failure first; its regression test applies the top remedy and checks the model then solves.

| Class | Reproduced as | Remedy / question | Test (`fea-problem/tests/failures.rs`) |
|---|---|---|---|
| `InvertedElements` | one triangle of an imported Gmsh file written clockwise: `inverted or degenerate elements (block, element): [(0, 1)]` | `Edit::Reorient` (`MeshSpec::reorient`): `fea_core::import::reorient_inverted` turns mirrored elements right side out by the mirror node permutation, every element kind; collapsed or folded ones stay and the class then offers nothing | `a_reversed_triangle_in_an_imported_mesh_...` (the repaired model gives the good mesh's strain energy to 1e-12), `fea-core/tests/import.rs` for all ten kinds |
| `ContactNotConverged` | a bushing modulus of 1 against the plate's 1e7: `the loaded bushing could not be solved: NoConvergence("stagnated ...")` | `MatchBushingModulus` only when the ratio is outside 1e-3..1e3 (a units slip); any other stall gets the evidence and **no** invented remedy | `a_bushing_with_a_units_slip_...` (fast), `a_real_stalled_contact_solve_...` (`--release --ignored`: minutes in a debug build) |
| `ThinFeature` | a slot 1e-5 wide with the default adaptive passes: the refinement ran for minutes against the mesher's 2 million vertex cap | `MAX_VERTICES = 250_000` ends it in seconds with `mesh exceeds ...`; remedies widen the hole to half an element or remove it | `a_hair_thin_slot_fails_fast_...` (both fixes solve) |
| `ScaleSuspect` | a modulus 1000 times too small: nothing fails, the plate stretches past its own size | `diagnose_result` (needs a solved result): displacement above 5 % of the part's size asks whether the modulus is in another unit; the two answers scale it by 1000 either way | `a_modulus_a_thousand_times_too_small_...`, `guidance_tests.rs` |

Also: the contact (bushing) solve now runs the kernel's model check before the contact iteration, minus the findings a bushed model
fails on purpose (`rigid_body`: the bushing is held by contact alone; `no_load`: a fit alone is a real case). A NaN load is refused
at once with its reason.
