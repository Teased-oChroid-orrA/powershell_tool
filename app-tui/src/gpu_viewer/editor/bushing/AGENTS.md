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
- **Narrow windows fold the side panes into drawers** (`gui/workbench.rs`, shared by every `ui::Workbench`; approved mockup `docs/workbench-redesign/drawer.html`). Below 1100 px the results pane leaves the layout and opens over the views from an edge tab that shows the verdict chip; below 700 px the inputs do the same from a left tab. Same cards, only placed differently; click outside or Esc closes; automatic, no setting. Wide windows are unchanged. Checked by the `bushing_snapshot` shots `bushing-drawer-*`.
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

- **The ~8 s FE solve is not a mesh-size cost; coarsening or adaptive passes will not shorten it (measured 2026-11-14, default case, M1).** Base size /6../2.5 and hole size /3../8 gave 1.9k-2.9k nodes and 7.8-10.8 s (16 s at /8 /8): time does not follow node count, so it is the contact Newton iterations. Mean fit pressure moved < 0.03 % (9233.5-9236.1 psi) and max von Mises < 0.3 %; peak pressure only scatters (13-25 ksi, Gauss-point noise). Contact AMR on a fit was already built and removed (`fea-problem/AGENTS.md`). A real speedup is in the contact solve, and it exists (measured 2026-11-14): chord Newton (`FIT_CHORD_ITERS` = 5, -14 %) and a looser multiplier-pass tolerance on the *loaded* solve (`LOADED_FIT_OUTER_TOL` = 1e-2, 7.8 s -> 3.4 s) in `fea-problem/src/solve.rs`; mean fit pressure moved -0.03 %, max von Mises +0.07 %, the no-load fit is unchanged. The peak pressure remains Gauss-point noise (18-25 ksi across tolerances). Not in `housing_problem`'s mesh numbers.
- Checks uncertainty fields reuse `BushingState::edge_settings`; `PromptTarget::EdgeCv` routes validation without a solver input, undo checkpoint or new FE mesh. These transient controls are not saved in the bushing case.
- Tolerance failures (bore band wider than the interference band, or leaving a bushing OD band under `OD_BAND_FLOOR_IN` = 0.0002 in) are fixed by three verified routes built in `toolboxes/bushing/advice.rs::tolerance_routes`: tighten the bore band from the top (least change; minimum bore size and interference requirement kept), a catalog reamer that holds the band, or widen the interference band. Nothing applies unasked. The old enforcement controls (toggle, lock bore, preserve/shift nominal, capability floor) were removed; old case files still open and ignore them. The 0.0002 floor is the owner's stated assumption for a ground OD.
- Library management (#242) is `library.rs`: one generic `Manager<C: Catalog>` per library (reamer, material, Bushing ID) behind the object-safe `Manage`, opened by the picker's "Manage library" row. File format, duplicate pruning and conflicts stay in `src/library.rs`. Saves are read back from disk before the model is refreshed (`BushingModel::set_custom_materials` / `set_user_reamers`). A material the case uses cannot be deleted; deleting another shifts the material indices. Shipped reamers are read-only except their Preferred tag. Import filters out untagged copies of shipped entries. Tests redirect the files with `BushingSession::library_dir`; never let a test write the real app-data library.
