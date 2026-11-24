# gpu_viewer/editor/plate_edge/ - the Plate Edge workbench in the native window

> TL;DR: `PlateEdgeSession` is an `Authoring` over `PlateEdgeInputs` (library or custom plate material, fastener, fit, geometry, mesh mode). Two calculations on a worker (`run.rs`): the allowables by failure mode (`edge_check::modes`) and the 2D/3D FE bearing response (`lug_solver::bracket`). Advisory; the legacy `Fbru + 0.8 p` check stays governing. Design, evidence and limits: `docs/plate-edge-workbench.md`.

## Purpose
Owns: the window front end (inspector cards, tree, ribbon, menus, undo, the case file, the report text), turning a case into the two solvers' inputs (`inputs.rs`), the worker job and its staleness rule (`run.rs`), the picture (`scene.rs`).
Does not own: any mechanics (`edge-check/AGENTS.md`, `lug-solver/AGENTS.md`), the material library (`mechanics-core`), the fastener size catalogue (`fastened-joint-solver`), the window or the cards' drawing.

## Code Map
| Looking for... | Go to |
|---|---|
| Session, `PlateEdgeAct`, prompts, undo, save, `Authoring` impl, the quick-run-after-every-edit rule | `mod.rs` |
| Inputs, `Field`/`Prop` numeric rows with validation, library search, fit kinds, `modes_input` / `bracket_case` / `picture_case`, the case file | `inputs.rs` |
| `Job` (worker thread, cancel flag, per-mesh progress), `Outcome`, `FeLine`, which lines a run makes | `run.rs` |
| Cards (plate, pin, fit, geometry, mesh, run, allowables, FE, convergence, limits), status line, report text, tree, ribbon, menus | `cards.rs` |
| The plate before a run (graded quarter plate) and the solved model by equivalent plastic strain | `scene.rs` |

## Contracts
- **A result is fresh only for the exact inputs it ran for** (`Outcome::inputs == inputs`, `current()`); a stale one is labelled, not drawn.
- **Every edit starts the quick allowables** (closed forms and the elastic field, about 0.3 s) unless a longer run is going; a longer run the person asks for replaces a running quick one. Only `Run 2D` / `Run 2D + 3D` start the contact FE and the FE bracket.
- **A row with no number says why** (`ModeRow::why`); never fill one in. A bolt class gives tension and double-shear ratings only: pin bending and bearing need a pin material or typed strengths.
- The FE bracket refuses an interference or custom-pressure fit (`bracket_refusal`) and says so; the 2D contact FE of the allowables still runs for it. Do not route a press-fit pin through the 3D driver (its contact stage did not converge in trials).
- A cancel (`Esc`, Cancel) sets the solver's interrupt flag: the run ends at the next Newton step with the lines finished so far.
- Library strengths are read from `mechanics_core::materials`; `Fbry` comes from `MaterialExtras` (handbook entries only), so the curated typicals have no bearing-onset row until typed.

## Pitfalls
- A full `Run 2D + 3D` is minutes (the 3D solid alone is about 150 s at the default mesh); tests that run it must cancel or use the 2D run.
- The scene colours the solved model by nodal-averaged equivalent plastic strain (`bracket::Fields::plastic`), not stress.
