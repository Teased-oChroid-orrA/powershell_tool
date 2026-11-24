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
| The picture: `Frame` (the one framing), the plate before a run with pin, free edge, shear-out planes and edge dimension drawn on it, the solved model by equivalent plastic strain | `scene.rs` |
| The viewport labels (`Authoring::overlay`): load arrow P always, `e = ..`, `D`, planes and free-edge labels only before a result | `mod.rs` |
| Results step: the load-travel plot, the 2D / 3D columns table, the convergence table | `cards.rs` (`curve_plot`, `line_table`, `convergence_card`) |

## Contracts
- **A result is fresh only for the exact inputs it ran for** (`Outcome::inputs == inputs`, `current()`); a stale one is labelled, not drawn.
- **Every edit starts the quick allowables** (closed forms and the elastic field, about 0.3 s) unless a longer run is going; a longer run the person asks for replaces a running quick one. Only `Run 2D` / `Run 2D + 3D` start the contact FE and the FE bracket.
- **A row with no number says why** (`ModeRow::why`); never fill one in. A bolt class gives tension and double-shear ratings only: pin bending and bearing need a pin material or typed strengths.
- The FE bracket refuses an interference or custom-pressure fit (`bracket_refusal`) and says so; the 2D contact FE of the allowables still runs for it. Do not route a press-fit pin through the 3D driver (its contact stage did not converge in trials).
- A cancel (`Esc`, Cancel) sets the solver's interrupt flag: the run ends at the next Newton step with the lines finished so far.
- Library strengths are read from `mechanics_core::materials`; `Fbry` comes from `MaterialExtras` (handbook entries only), so the curated typicals have no bearing-onset row until typed.

- **Both views share one framing** (`scene::Frame::of(inputs)`: hole, ligament and free edge, about 2 D beyond the hole, `FILL` of the view). `reframe` maps the result scene onto it (positions, displacement samples and `gain`), so the camera does not jump when a result replaces the picture, and the overlay positions (`Frame::point`) stay valid for both.
- **The result is drawn to scale** (`scene.gain = 1`): the automatic gain exaggerates the displacement several times and made the hole look larger than the picture's. The frame fits the viewport height (`Camera::project` divides x by the aspect), so tests and snapshots use a landscape camera or a 1500 x 1500 window, not a tall one. The solve is on half the plate; the drawing is the half mirrored about the load line (`scene::mirror`, whole circular hole and pin, symmetric contour; seam nodes shared, orientation kept) and the `HALF_MODEL` caption says so. Draw annotations after mirroring, in full-plate coordinates.
- **Two collapse numbers, two names.** The allowables row "Collapse (contact FE, plane strain)" is `edge-check`'s own contact FE; the FE lines are "(FE bracket)" (`lug-solver`). They agree on the physics (rigid pin, `contact_model::flow_stress`, friction `PlateEdgeInputs::friction`, load at 0.1 D) and differ by discretisation and contact algorithm: measured gap in `docs/plate-edge-workbench.md`. Never merge them into one row.
- **The pin-hole friction reaches the bracket** through `Case::pin_mu` (`picture_case` / `bracket_case` set it from the input). Before 2026-11-23 the bracket was frictionless although the input said 0.12.
- **The plot's three lines use three palette colours** (`line_color`: accent, bad, good; the warm theme's other colours are too close) and plane stress is dashed; onset is a dot without a label, the key names the lines.
- **Every FE line keeps its load-travel curve** (`Refinement::curve`); the plot and the "onset" marks read it. Do not recompute a curve in the cards.

## Pitfalls
- Annotations in the picture are flat triangles (the whole pin disc, both shear-out planes, the e dimension under the hole) at scene z = `-LIFT` (in front of the plate), vertices coded 2 (pin) and 3 (edge, planes, dimension); the geometry view reads codes 0..3 (`KEY_LABELS`), so the scene range must stay `[0, 3]`.
- A 3D run with friction is about two minutes by itself (115 s at the default mesh vs 99 s frictionless): tests that wait for one need a limit above 120 s (`wait_up_to`).
- A full `Run 2D + 3D` is minutes (the 3D solid alone is about 150 s at the default mesh); tests that run it must cancel or use the 2D run.
- The scene colours the solved model by nodal-averaged equivalent plastic strain (`bracket::Fields::plastic`), not stress.
