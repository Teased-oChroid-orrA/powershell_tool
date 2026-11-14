# gpu_viewer/editor/bushing/ - the Bushing workbench in the native window

> TL;DR: `BushingSession` is the sole Bushing Workbench UI. It uses `BushingState` for inputs, checks, fixes and edge checks, plus a read-only `SolidSession` for the derived FEA model. No engineering math lives here.

## Purpose
Owns: the native Bushing Workbench (inputs, sketches, 3D view, results cards, Send dialog, pickers, undo, the bushing file), and `fe.rs`, the mapping from inputs to a plane-stress `fea_problem::Problem` with one bushing in frictional interference contact.
Does not own: Lamé / press-fit / tolerance / edge-distance formulas (`bushing-solver/`, `edge-check/`; `toolboxes/bushing/{model,advice,edge_check}.rs` bridge them), the FE solve (`fea-core`, `fea-problem`), or shared window drawing (`gpu_viewer/gui`).

## Code Map
| Looking for... | Go to |
|---|---|
| The session, `BushingAct`, `Authoring` impl, edit / undo / save / export, the edge-check worker thread | `mod.rs` |
| Inputs -> `Problem` (housing rectangle, edge distance, bushing, pin load), `layout` | `fe.rs` |
| The saved case (JSON keyed by row label, `NUMBERS`) | `file.rs` |
| Three-pane workbench inputs, results, Send dialog and menus | `cards.rs` |
| 3D housing and bushing geometry and stress colouring | `drawing.rs` |
| Axial and cross-section sketches | `sketch.rs`; shared layout in `gpu_viewer/sketch.rs` |
| Bore / ID / friction / material pickers as value cards | `picker.rs` |

## Contracts
- The inputs are the model; the FE picture follows them (`refresh_fea` hands `SolidSession::set_derived` the new problem, no undo step, not dirty) and is **read-only**: click, select and the boundary popup do nothing. Never let an edit of the derived `Problem` become input; the FE result never feeds the pass / fail checks.
- The FE solve is a manual cross-check, not the verdict: plane stress, the free edge at x = 0, the bore centre `edge_dist` from it, held at the far end, and pin load toward the edge. The 3D view visualizes the housing and bushing, but the FE model omits flanges, countersinks and the housing's real outline. `fe::tests` compares mean fit pressure with the closed form in a wide housing (measured 9267 vs 9025 psi, +2.7 %); the default case solves in about 8 s.
- Quick FE shows mean/peak fit pressure, Lamé pressure and their signed difference only after an explicit run. Input edits mark the result stale. The difference reflects model and discretization limits; it is not a pressure correction.
- `bench()` is `Static` and `benches()` is empty: the Window menu offers no benches here. `key_labels()` renames the colour key (Housing, Support, Load, Bushing); the bushing is coloured as "selected" by `SolidSession::highlight_bushings` (flood fill from its bore, `bushing_nodes`).
- **FEA hand-off** (`handoff.rs`, `BushingAct::{SendDialog,ConfirmSend}`): the dialog currently sends a 2D plane-stress problem. The 3D solid option is disabled pending #241. Keep linked stays on: the case is saved, its derived `Problem` written as `<case>.fea.json`, and the FEA workbench's Send writes back only what changed (the case file is the interface; `watch` takes it in once a second, as one undo step, unless this window has unsaved edits). The window keeps a heartbeat while linked.
- A typed value equal to the current one is not an edit (no undo step, no new mesh). Picker choices and fixes go through the same `checkpoint` / `changed` pair as typed edits.
- Inputs use US customary units (in, lbf, ksi, °F), stated on the Inputs card. New case files declare `in-lbf-ksi-F`; older files without a declaration retain the same interpretation, and files declaring other units are refused.
- Strings that come from the model, advice and edge check pass through `cards::plain`: the window font lacks arrows and several symbols.
- A `Field` whose tone is `Warn` is drawn as an edit box by the window, so an input a failing check points at is marked `! label` instead.

## Pitfalls
- The default case fails the edge-distance (sequencing) check: that is the solver's answer for its own default fixture, not a bug here.
- Picker state (`ReamerPickerState`, `BushingIdPickerState`) loads the user's library from disk when created: build it once per opened picker (`open_picker`), never per frame.
- Tests that want the result drawn must rebuild the scene after the solve (`needs_scene`, `scene()`), as the window does.

- Checks uncertainty fields reuse `BushingState::edge_settings`; `PromptTarget::EdgeCv` routes validation without a solver input, undo checkpoint or new FE mesh. These transient controls are not saved in the bushing case.
- Library management (#242) is `library.rs`: one generic `Manager<C: Catalog>` per library (reamer, material, Bushing ID) behind the object-safe `Manage`, opened by the picker's "Manage library" row. File format, duplicate pruning and conflicts stay in `src/library.rs`. Saves are read back from disk before the model is refreshed (`BushingModel::set_custom_materials` / `set_user_reamers`). A material the case uses cannot be deleted; deleting another shifts the material indices. Shipped reamers are read-only except their Preferred tag. Import filters out untagged copies of shipped entries. Tests redirect the files with `BushingSession::library_dir`; never let a test write the real app-data library.
