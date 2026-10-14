# gpu_viewer/editor/bushing/ - the Bushing workbench in the native window

> TL;DR: `BushingSession` is an `Authoring` over the terminal toolbox's own `BushingState` (inputs, solver, checks, fixes, edge check) plus a read-only `SolidSession` that draws, meshes and solves the housing and bushing as an FEA `Problem` derived from the inputs. No engineering math lives here.

## Purpose
Owns: the window front end of the Bushing workbench (inspector cards, tree, ribbon, menus, pickers as value cards, undo, the bushing file), and `fe.rs`, the mapping from the inputs to a plane-stress `fea_problem::Problem` with one bushing in frictional interference contact.
Does not own: any Lame / press-fit / tolerance / edge-distance formula (`bushing-solver/`, `edge-check/`; the terminal toolbox's `model.rs`, `advice.rs`, `edge_check.rs` bridge them and are reused as they are), the FE solve (`fea-core`, `fea-problem`), the window or the cards' drawing (`gpu_viewer/gui`).

## Code Map
| Looking for... | Go to |
|---|---|
| The session, `BushingAct`, `Authoring` impl, edit / undo / save / export, the edge-check worker thread | `mod.rs` |
| Inputs -> `Problem` (housing rectangle, edge distance, bushing, pin load), `layout` | `fe.rs` |
| The saved case (JSON keyed by row label, `NUMBERS`) | `file.rs` |
| Inspector cards per step, tree, ribbon, menus | `cards.rs` |
| Bore / ID / friction / material pickers as value cards | `picker.rs` |

## Contracts
- The inputs are the model; the FE picture follows them (`refresh_fea` hands `SolidSession::set_derived` the new problem, no undo step, not dirty) and is **read-only**: click, select and the boundary popup do nothing. Never let an edit of the derived `Problem` become input; the FE result never feeds the pass / fail checks.
- The FE view is a display and a cross-check, not the verdict: plane stress, the free edge at x = 0, the bore centre `edge_dist` from it, held at the far end, the pin load toward the edge. Flanges, countersinks and the housing's real outline are not drawn. `fe::tests` compares the FE mean fit pressure with the closed form in a wide housing (measured 9267 vs 9025 psi, +2.7 %); the default case solves in about 8 s, so a solve is manual and only the mesh follows the inputs.
- `bench()` is `Static` and `benches()` is empty: the Window menu offers no benches here. `key_labels()` renames the colour key (Housing, Support, Load, Bushing); the bushing is coloured as "selected" by `SolidSession::highlight_bushings` (flood fill from its bore, `bushing_nodes`).
- A typed value equal to the current one is not an edit (no undo step, no new mesh). Picker choices and fixes go through the same `checkpoint` / `changed` pair as typed edits.
- Strings that come from the model, advice and edge check pass through `cards::plain`: the window font lacks arrows and several symbols.
- A `Field` whose tone is `Warn` is drawn as an edit box by the window, so an input a failing check points at is marked `! label` instead.

## Pitfalls
- The default case fails the edge-distance (sequencing) check: that is the solver's answer for its own default fixture, not a bug here.
- Picker state (`ReamerPickerState`, `BushingIdPickerState`) loads the user's library from disk when created: build it once per opened picker (`open_picker`), never per frame.
- Tests that want the result drawn must rebuild the scene after the solve (`needs_scene`, `scene()`), as the window does.
