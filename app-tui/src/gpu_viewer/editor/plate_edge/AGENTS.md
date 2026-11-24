# gpu_viewer/editor/plate_edge/ - the Plate Edge workbench in the native window

> TL;DR: `PlateEdgeSession` is an `Authoring` over `PlateEdgeInputs` (library or custom plate material, fastener, fit, geometry, mesh mode). Two calculations on a worker (`run.rs`): the allowables by failure mode (`edge_check::modes`) and the 2D/3D FE bearing response (`lug_solver::bracket`). Advisory; the legacy `Fbru + 0.8 p` check stays governing. Design, evidence and limits: `docs/plate-edge-workbench.md`.

## Purpose
Owns: the window front end (the three-pane workbench data: input cards, result cards, the plot under the view, chips and actions, menus, undo, the case file, the report text), turning a case into the two solvers' inputs (`inputs.rs`), the worker job and its staleness rule (`run.rs`), the picture (`scene.rs`).
Does not own: any mechanics (`edge-check/AGENTS.md`, `lug-solver/AGENTS.md`), the material library (`mechanics-core`), the fastener size catalogue (`fastened-joint-solver`), the window or the cards' drawing.

## Code Map
| Looking for... | Go to |
|---|---|
| Session, `PlateEdgeAct` (`Set(Setting)` for a segmented switch, `View(Dim)` 2D / 3D, `Layer(Layer)` Geometry / Plastic strain, `Choice` for the two cyclic rows), prompts, undo, save, `Authoring` impl (`workbench()`; no tree, ribbon or inspector), the quick-run-after-every-edit rule | `mod.rs` |
| Inputs, `Field`/`Prop` numeric rows with validation, library search, fit kinds, `modes_input` / `bracket_case` / `picture_case`, the case file | `inputs.rs` |
| `Job` (worker thread, cancel flag, per-mesh progress), `Outcome`, `FeLine`, which lines a run makes | `run.rs` |
| `workbench_data()`: header chips (units, verdict, advisory), actions (Save case, Report, Run 2D, Run 2D + 3D, Cancel), the view and field switches, the status line, input cards (plate, pin, fit, plate & load, mesh; each opens with a `DockItem::Segmented`), result cards (allowables `Compare`, handbook check, mesh convergence, FE bearing response, limits), the plot card under the view, menus, report text | `cards.rs` |
| How the window draws it: `Look::Boxed`, boxed cards with chevron and tag chip, `segmented_equal`, `compare_ui` (per-column widths: a head sits over the right edge of its figures), `under_pane`, `input_row` (a long value gets up to two thirds of the row, then an ellipsis) | `gui/workbench.rs`, `gui/panels.rs::dock_item`, `ui.rs` (`DockItem::{Segmented, Compare}`, `Look`, `Workbench::{under, view_status}`) |
| The picture: `Frame` (the one framing), the plate before a run with pin, free edge, shear-out planes and edge dimension drawn on it, the solved model by equivalent plastic strain | `scene.rs` |
| The viewport labels (`Authoring::overlay`): load arrow P, `e = ..`, the planes and free-edge labels always, one caption line along the bottom (`HALF_MODEL`, or `PIN_TRAVEL` in a result); `D` only before a result; the pin disc, the dashed shear-out planes (both sides) and the e dimension line with its end ticks (`OverlayKind::Line` / `Dashes`) only in a result, whose scene draws none of them (the picture state has them as scene triangles). Each label is anchored on what it names (`each_result_label_sits_on_the_geometry_it_names` checks the pixels). Drawn by `Gui::model_labels`, which the workbench branch of `gui/mod.rs` must call (it returns before the common path) | `mod.rs`, `gui/mod.rs` |
| The load-travel plot (under the view), the allowables table with 2D FE and 3D FE columns, the convergence table | `cards.rs` (`curve_plot`, `line_table`, `allowables_card`, `convergence_card`) |

## Contracts
- **A result is fresh only for the exact inputs it ran for** (`Outcome::inputs == inputs`, `current()`); a stale one is labelled, not drawn.
- **Every edit starts the quick allowables** (closed forms and the elastic field, about 0.3 s) unless a longer run is going; a longer run the person asks for replaces a running quick one. Only `Run 2D` / `Run 2D + 3D` start the contact FE and the FE bracket.
- **A row with no number says why** (`ModeRow::why`); never fill one in. A bolt class gives tension and double-shear ratings only: pin bending and bearing need a pin material or typed strengths.
- The FE bracket refuses an interference or custom-pressure fit (`bracket_refusal`) and says so; the 2D contact FE of the allowables still runs for it. Do not route a press-fit pin through the 3D driver (its contact stage did not converge in trials).
- A cancel (`Esc`, Cancel) sets the solver's interrupt flag: the run ends at the next Newton step with the lines finished so far.
- Library strengths are read from `mechanics_core::materials`; `Fbry` comes from `MaterialExtras` (handbook entries only), so the curated typicals have no bearing-onset row until typed.

- **Both views share one framing** (`scene::Frame::of(inputs)`: hole, ligament and free edge, about 2 D beyond the hole, `FILL` of the view). `reframe` maps the result scene onto it (positions, displacement samples and `gain`), so the camera does not jump when a result replaces the picture, and the overlay positions (`Frame::point`) stay valid for both.
- **The result is drawn to scale** (`scene.gain = 1`): the automatic gain exaggerates the displacement several times and made the hole look larger than the picture's. The frame fits the viewport height (`Camera::project` divides x by the aspect), so tests and snapshots use a landscape camera or a 1500 x 1500 window, not a tall one. The solve is on half the plate; the drawing is the half mirrored about the load line (`scene::mirror`, whole circular hole and pin, symmetric contour; seam nodes shared, orientation kept) and the `HALF_MODEL` caption says so. Draw annotations after mirroring, in full-plate coordinates. A result's pin is an `OverlayKind::Disc` overlay (centre and radius from `scene::pin_disc`: start position minus 0.1 D), not scene geometry: a scene vertex takes the contour's colours and cannot be neutral.
- **Two collapse numbers, two names.** The allowables row "Collapse (contact FE, plane strain)" is `edge-check`'s own contact FE; the FE lines are "(FE bracket)" (`lug-solver`). They agree on the physics (rigid pin, `contact_model::flow_stress`, friction `PlateEdgeInputs::friction`, load at 0.1 D) and differ by discretisation and contact algorithm: measured gap in `docs/plate-edge-workbench.md`. Never merge them into one row.
- **The pin-hole friction reaches the bracket** through `Case::pin_mu` (`picture_case` / `bracket_case` set it from the input). Before 2026-11-23 the bracket was frictionless although the input said 0.12.
- **The plot's three lines use three palette colours** (`line_color`: accent, bad, good; the warm theme's other colours are too close) and plane stress is dashed; onset is a dot without a label, the key names the lines.
- **Every FE line keeps its load-travel curve** (`Refinement::curve`); the plot and the "onset" marks read it. Do not recompute a curve in the cards.

## Window contracts (the mockup layout)
- **A workbench branch of `Gui::build` returns early**, so everything the common path draws over the model has to be called there too. `model_labels` was missing: the pin disc, the load arrow and every caption were silently dropped in this window. `tests::the_workbench_window_paints_the_pin_disc_and_the_plate_captions_over_the_view` runs the real `Gui` headless and checks the painted circle and texts; keep it.
- **The 2D | 3D and Geometry | Plastic strain switches are the only view state** (`Dim`, `Layer`); `shows_result()` = layer Strain, a current result, and a line for `dim`. A finished run sets both.
- **Folding**: no card starts folded; every boxed card has a chevron and folds on a click.
- **Not offered, because the engine does not compute it**: von Mises, shear and bearing-pressure fields (`lug_solver::bracket::Fields` has the mesh, displacement and plastic strain), plate width and an inclined load (not inputs of `modes_input` / `bracket_case`: "Load direction" is a read-only row), FE numbers for any row but bearing yield and collapse. The pin source stays Class | Material | Custom (the engine distinguishes them), not the mockup's Library | Custom.
- Visual evidence: `plate_edge_snapshot` writes inputs, 2D result and 2D + 3D result shots in both themes (`docs/plate-edge-workbench.md`).

## Pitfalls
- Annotations in the picture are flat triangles (the whole pin disc, both shear-out planes, the e dimension under the hole) at scene z = `-LIFT` (in front of the plate), vertices coded 2 (pin) and 3 (edge, planes, dimension); the geometry view reads codes 0..3 (`KEY_LABELS`), so the scene range must stay `[0, 3]`.
- A 3D run with friction is about two minutes by itself (115 s at the default mesh vs 99 s frictionless): tests that wait for one need a limit above 120 s (`wait_up_to`).
- A full `Run 2D + 3D` is minutes (the 3D solid alone is about 150 s at the default mesh); tests that run it must cancel or use the 2D run.
- The scene colours the solved model by nodal-averaged equivalent plastic strain (`bracket::Fields::plastic`), not stress.
