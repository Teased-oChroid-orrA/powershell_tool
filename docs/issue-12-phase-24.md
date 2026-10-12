# Issue 12 - Phase 24: parametric sketcher, feature history, recommendations (2026-10-10)

Decision record: `docs/adr/ADR-015-parametric-sketch-history.md`. Started from the handoff "parametric history-based CAD
sketcher + ML constraint / FEA recommendations". Everything is uncommitted on `codex/fea-core-roadmap` (on top of
`ae05501`); nothing was committed or pushed.

## What the user asked, and what exists now

| Request | State |
|---|---|
| Plane selection to start a sketch (XY / XZ / YZ, datum planes) | Front / Top / Right with offset: UI (`Insert > Sketch on ...`, `Features` tab). Datum planes exist in the model (`Plane::Datum`, tested) but have no UI yet. |
| Sketch entities and geometric constraints (coincident, tangent, parallel, perpendicular, horizontal / vertical, equal, point-on, midpoint, fixed) + dimensions (length, distance, h/v distance, radius, angle) | Model + solver + UI. h/v distance have no UI tool (Distance does). |
| Fluid entity creation | Line, polyline, rectangle, circle, arc (centre, start, end); snapping (points, entities, h/v alignment, grid); rubber band; shared endpoints; auto h/v and point-on; dimension by typing (first key replaces the value); drag points with constraints held. |
| Feature ops Extrude / Revolve / Sweep | Extrude and Revolve (editable depth / layers / angle / segments, suppress, roll back, delete). **Sweep is not built** (ADR-015 item 6): needs a path sketch + swept frame; `fea_core::sweep` has the general mapping, only extrude / revolve are exposed. |
| FEA boundaries / loads on 2D sketch entities and 3D topology | Selecting sketch edges / holes selects the mesh surfaces they will have (`Sketch::surface_names`), so Support / Load act on sketch entities; in 3D, faces are picked as before. Vertices: `Support::Point` / `Load::Point` still by coordinates (no vertex pick). |
| ML-assisted smart constraints | Rule-based inference of constraints the geometry already satisfies (`sketch/infer.rs`) shown as numbered suggestions; opt-in local recording and a gated learned *reordering* (`editor/rank.rs`). **No model ships and no real recordings exist**; the learned order cannot switch on until a model passes the measured gate on held-out sessions (>= 20 sessions, >= 100 held-out events, sign test z >= 2, no top-3 regression). |
| Recommendations: mesh controls, load placement, boundary conditions from problem type, geometry, intent | `fea_problem::advice`: support far from the load, load / pin load far from the support, element size for the thinnest section, hole refinement, layers through a thin extrusion; each with its reason, applied on click (undoable); the surface popup orders support / load kinds by intent (`boundary_options`), optionally learned. **Thermal advice does not exist**: the `Problem` schema has no thermal-conduction problem type (only a uniform temperature change). |

## Verification (2026-10-10)

* `cargo test -p app-tui -p fea-problem` (all suites): 1164 passed, 0 failed; app-tui lib alone 1041 (12 sketch-core, 17 sketch-session, 6 advisor, 10 rank tests are new); `-p app-tui --test rendering` 38; `-p fea-problem` all suites incl. `tests/sketch.rs` (29) and `tests/advice.rs` (6).
* Numerical proofs (`tests/sketch.rs`): rectangle DOF 4 -> 0 with fixed corner + two lengths; redundant (agreeing) vs conflicting classification; tangent line / circle / circle to 1e-9; triangle by lengths + angle; drag keeps constraints; every template converts to a history that rebuilds the *same* geometry and edge names; Top-plane part lands at the right coordinates and keeps its surface names; revolve extents; profile names are real mesh surfaces.
* `cargo clippy -p app-tui -p fea-problem --lib --tests`: no warning in `gpu_viewer/` or `fea-problem` (other toolboxes have older warnings); `cargo check --no-default-features` warning-free.
* Rendered check: `FEA_SNAPSHOT_EDITOR_OUT=dir cargo test -p app-tui --lib sketch_snapshot -- --ignored` (sketch view with ribbon, panel, tree).
* Window smoke: `./target/debug/app-tui --gpu-solid <file with history>` opens and stays up 8 s with empty stderr.
* NOT verified: real mouse / keyboard on the desktop (no Accessibility permission in the sandbox; the pixel -> plane path is covered by headless `Viewer::input`-level session tests only), Windows, hosted CI.

## Known limits / open items (Fix-Everything: these were too large or need a decision)

* No Sweep; one solid feature per history (no boss / cut sequences, no booleans); islands and separate regions are refused with a message.
* Arcs become polygon edges named `edgeN` (48 per turn): a supported *arc* edge is several surfaces (select them with Shift / box). A native arc `Shape` would touch every `Shape` match.
* Topology names are order-based (lowest entity id first): reordering entities or deleting the first line renumbers `edgeN` and can leave supports pointing at the wrong edge. Stable names need a topological naming scheme (ADR-015 item 7).
* Constraint glyphs and dimension annotations are not drawn in the viewport (scene is triangle-only, 8x8 ASCII font): they are listed in the property panel; entities are blue (under-defined) / teal (fully defined) as a whole, not per entity.
* No datum-plane or face-plane picker, no construction geometry, no splines / ellipses / symmetric, no h/v distance tool.
* The terminal Workbench shows an empty Geometry section for a model with a history (edit it in the native window).
* Earlier open list (`docs/issue-12-claude-handoff.md`): dead terminal helpers, other benches (lug / bushing / eccentric / preload / thermal / material), GUI polish (IME font, node ids, load arrows, view cube, export, copy/paste/mirror, units, autosave, file dialogs), advisor rules for member-model data beyond the existing ones.

## Next steps (suggested order)

1. Real-desktop pass over Sketch mode (drag, snap feel, entry box), fix what it shows.
2. Per-entity DOF colouring + in-viewport constraint glyphs (needs line / marker geometry or CPU HUD drawing).
3. Sweep (path sketch on a second plane, parallel-transport frame, `fea_core::sweep` mapping) and datum / face plane picking.
4. Topological naming that survives reorder; native arc `Shape`.
5. Thermal problem type in `fea-problem`, then thermal advice.
