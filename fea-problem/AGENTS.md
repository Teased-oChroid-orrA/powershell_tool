# fea-problem

> TL;DR: Problem-definition layer over `fea-core`: a serialisable `Problem` (2D sketch with holes, extruded to 3D, or an imported Gmsh / Abaqus mesh; material, supports, loads, mesh settings), mesh generation, the solve (optionally adaptive), nodal results and a summary, a raster of any field for a terminal, a text report and `.vtu`; plus `joint::member_compliance`, the FE compliance of a clamped bolt-joint stack. Pure functions of their inputs; no file I/O, no UI. Consumed by the `app-tui` FEA Workbench (`problem` / `solve` / `raster`) and Preload Analysis (`joint`).

## Purpose
Owns: the problem schema and its JSON, edge naming, support / load resolution (names to nodes and faces), the solve driver and its checks, result fields and the summary, the pixel raster, the report text, the templates, the stacked-ring axisymmetric member model.
Does not own: any element, solver or mesher (`fea-core`), any UI or file access (`app-tui` reads and writes the text), material tables (`mechanics-core`).

## Code Map
| Looking for... | Go to |
|---|---|
| `Problem`, `Shape`, `Support`, `Load`, `MeshSpec`, JSON, `edge_names`, validation, editing helpers (`params` / `set_param` / `next_kind`) | `src/problem.rs` |
| Sketch to `Region` to mesh (size function, hole refinement, extrusion, element cap), imported mesh | `src/build.rs` |
| `solve`, `preview`, supports -> `Dirichlet`, loads (incl. the bearing cosine load), result fields, summary, adaptive passes | `src/solve.rs` |
| Field to pixel grid (triangle fill, edges, deformed shape, 3D front face) | `src/raster.rs` |
| Plain-text report, one-line support / load descriptions | `src/report.rs` |
| Interference-fit bushings (`Problem::bushings`): the annulus mesh (`build::add_bushings`), the contact solve (`solve::solve_mesh_contact`), `InterfaceResult` | `src/problem.rs`, `src/build.rs`, `src/solve.rs` |
| Natural frequencies and buckling load factors (`modal`, `buckling`, `run`, `DynSolved`) on the problem's own mesh, supports and loads | `src/dynamics.rs`, `tests/dynamics.rs` |
| Parametric sketch (entities, constraints, LM solver with DOF / conflict report, drag, profile -> `Shape`s, surface names, rule-based constraint inference) | `src/sketch/{mod,solve,profile,infer}.rs`, `tests/sketch.rs` |
| Feature history (Sketch / Extrude / Revolve, planes + frames, suppress, roll-back), `Problem::apply_history`, `History::from_problem` | `src/history.rs`, `src/problem.rs` |
| Rule-based support / load / mesh recommendations (`recommend`, `boundary_options`, `Advice::apply`) | `src/advice.rs`, `tests/advice.rs` |
| `Geometry::Solid` (STEP text + hash-tied cached surface mesh; `solid_mesh`, `Problem::set_solid`, `tessellate_step` needs feature `occt`) | `src/solid.rs`, `tests/solid.rs` |
| Failure diagnosis: `diagnose(&Problem, Option<&str>)` -> class, evidence, ranked `Remedy` (data `Edit`), `Question` (ADR-020) | `src/diagnose.rs`, `tests/failures.rs` |
| Starting problems | `src/templates.rs` |
| Member stack -> axisymmetric FE compliance (`C = 2U/F^2`) | `src/joint.rs` |
| Aircraft frame / truss member problem (stable ids, atomic `Edit`s, `validate`, accepted `solve`) | `src/members.rs`, `tests/members.rs` |
| Closed-form validation | `tests/problems.rs`, `tests/joint.rs` |

## Contracts
- Edge names: outer rect `bottom right top left`; outer polygon `edge1..N` (edge `i` runs from point `i` to `i+1`); outer circle / slot `outer`; hole `i` is `hole{i}` (1-based); an extrusion adds `start` (z = 0) and `end`. An imported mesh uses its own node-set / surface names; a node set with no surface of that name gets the boundary faces whose nodes all belong to it (`Mesh::select_faces_by_nodes`).
- Loads are per the kernel's conventions: `Pressure` acts into the body; `Traction` is force per area; `Force` is a total spread over the edge (`face_measure`: length x thickness plane, `2 pi int r ds` axisymmetric, area 3D); a `Bearing` load is a cosine pressure on the half of a hole facing the force, rescaled so its resultant along the force is exactly `|F|` whatever the hole shape; axisymmetric forces are totals over 360 degrees.
- `Support::Edge` / `Point` fix the listed components to the given values; `Fixed` = all three at 0. A model that leaves a rigid-body motion free is refused by `fea-core` (`Model::check_constrained`), reported as "not fully constrained".
- `Solved.summary.equilibrium_error` compares the support reactions with the assembled loads over the components that must balance (axisymmetric: the axis only, the radial reaction carries the hoop); it must stay below ~1e-8 for a sound model.
- Adaptive passes (`mesh.adapt_passes`, 2D sketches, bushings included): above `target_error` a pass is kept only if its ZZ estimate is lower (a singularity makes the estimate unreliable and a finer mesh must never make the answer worse); at or below it the field coarsens where the error is low and a pass is kept when it still meets the target with < 90 % of the unknowns. A bushed plate re-solves the fit and the pin load from scratch on every new mesh (no state transfer: contact states do not map across meshes), and the bushing is sized from the same field as the plate (`add_bushings`, capped by the wall), so both sides of the fit ask for the same element size along the interface. Measured on the bushed template (zz 0.0055 at the default mesh): 10281 -> 4769 nodes at zz 0.011, torque capacity identical, von Mises -0.2 %, peak pressure +4 % (pointwise pressure scatters), 3.3x the time of one solve.
- `MeshSpec.size` has a hard element cap (`build::MAX_ELEMENTS`): a typo in the size must not freeze the app.
- `joint::member_compliance`: members are rings sharing nodes at interfaces (a lattice with active cells), loaded by equal and opposite uniform pressure over the head / nut bearing annuli; members carry their own Poisson's ratio (Preload assumes 0.3). Different moduli with different Poisson's ratios constrain each other at the interfaces (no closed form); with `nu = 0` the compliances of members in series add exactly.

- **Bushings** (`Problem::bushings`, plane analyses, circular holes only): each is an annulus of its own material (outer circle = the hole, bore possibly offset) in frictional interference contact with the plate; the fit is solved first, the loads continue from it (`fea_core::fit`). The plate's own hole surface is kept as `plate_holeN` and the name `holeN` is re-pointed at the bushing's bore, so a load or support on `holeN` (including `Load::Bearing`, centred on the offset bore) acts on the bushing. A bushed problem is a contact solve (seconds), never run automatically by the Workbench. `Summary::interfaces` carries mean / peak fit pressure, open arc, friction torque capacity and slip share per bushing (integrals: pointwise pressures scatter); the equilibrium error of a fit-only problem is judged against 1e-3 of the transmitted force, of a loaded one it is the Newton tolerance (1e-3 is the test bound).
- `Summary::notes` carries the caution the kernel documents for the elements in use (Tri3 / Tet4: constant strain, poor on pressure and bending problems).

- **Members** (`members::MemberProblem`): truss and Timoshenko frame members over `fea_core::structural` (small displacement, no releases, warping or member buckling). Ids are stable and never reused; `apply(Edit)` is atomic (clone, edit, `validate`, swap). `plane = Some(axis)` makes a flat model (all nodes coplanar, out-of-plane translation and in-plane-axis rotations held, out-of-plane loads refused). `solve()` returns a result only if the backward error is <= 1e-8 and force/moment balance (about the origin, incl. distributed loads) <= 1e-6; frame `uniform_load` is in the member's local axes (`structural::member_axes`). Rotations/moments need a frame member at the node. Unit-agnostic; `unit_note` is display-only.

- **History** (`Problem::history`, ADR-015): when present it is authoritative; `apply_history()` rewrites `geometry` (and sets `Analysis::Solid` + a quad element for a solid feature) and `sketch_mesh` re-reads the history to extrude / revolve and place the part through the plane frame (Front = XY +Z, Top = XZ +Y with sketch y = -Z, Right = YZ +X; revolve about the sketch y axis, x = radius). One solid feature per history. Names (`edgeN`, `holeN`, `bottom..left`, `start`, `end`) come from the profile, deterministic by lowest entity id; `Sketch::surface_names` says which entity has which name. Arcs become polygon edges (`edgeN` per segment). Files without the field load unchanged and files without a history serialise byte-identically (`skip_serializing_if`).
- **Sketch solver**: Levenberg-Marquardt, forward-difference Jacobian, dense; iterates to numerical precision, a constraint counts as violated at residual >= `VIOLATION_TOL` (1e-7), rank threshold 1e-6 relative; `Report::status` Under / Fully / Over(redundant, consistent) / Conflicting, `violated` lists constraint ids. Drag = weak pulls (1e-3) on points. A constraint is added only if `Sketch::accepts` (entities exist, right kinds); callers must roll back a `Conflicting` result themselves (`app-tui` `SketchEdit::settle`).
- `Problem` equality includes `history` on purpose (it is model data); do not put derived or self-changing data in it.

## Pitfalls
- `solve_mesh` runs the kernel's `Model::validate`: errors stop the solve, warnings land in `Summary::notes`. Before 2026-10-11 it
  did not, and a NaN load solved to a silent zero. Bushing (contact) solves skip it on purpose (held only by contact).
- `Geometry::Solid` is solved from its cached surface mesh only; a missing or stale cache (hash of the STEP text) is refused,
  never re-tessellated silently (that needs the `occt` feature). Faces are `face1`.. in the kernel's order. Refinement zones
  and sketch history do not apply to it.
- `diagnose` recomputes its evidence from the problem; the only message text it relies on is the rigid-body wording, pinned by
  `tests/failures.rs`. Change that message and the test and the rule together.
- `MaterialSpec::density` (serde default 0) feeds only `dynamics::modal` (consistent units: inch / psi -> lbf s^2/in^4 = weight density / 386.09); a zero density is an explained error there, never a silent massless structure. Modal and buckling refuse bushed problems (a contact problem) and axisymmetric buckling.
- Random corpora live in `tests/sweep.rs`: linear templates and bolted-joint stacks (fast, in the default run), random bushed plates (`--ignored`, `SWEEP_SEED` / `SWEEP_N`, ~4 min for 24), `every_corpus_case_solves` over `tests/data/*.json` (each file is a problem that once failed the kernel), and `replay_one_case` (`SWEEP_CASE=path`, with `NL_TRACE` / `NL_DEBUG`). Add a failing case to `tests/data` when you fix it.
- `Problem` derives `PartialEq` and the workbench uses it for staleness / debounce: do not add a field that changes on its own (a timestamp, a counter). `Problem::name` is part of it; `app-tui` strips it, the supports and the loads (`preview_key`) when deciding whether the mesh is still valid.
- A node set and a surface can share a name; `named_nodes` prefers the node set, `named_faces` the surface.
- A hole polygon is supported by the schema but the editor does not list its points (edit the JSON); outer polygons are edited point by point.
- `Load::Bearing` and `Force` are plane-only (error for axisymmetric / solid, except `Force`, which works for 3D faces).
- The terminal Workbench edits `geometry` row by row; for a problem with a `history` it would be overwritten at the next `apply_history`, so `field_rows` hides the geometry rows for such problems. Anything new that writes `geometry` must check `history.is_none()` or edit the history.
- Mesh-refinement near holes uses the hole's bounding-circle reach: a long slot refines more widely than its width suggests.

## Boundaries
### Always
- Add or extend a closed-form test in `tests/` for a solver-facing change.
### Never
- Add file I/O or UI here, or put kernel mechanics here (it belongs in `fea-core`).

## Navigation
Parent concepts: `fea-core/AGENTS.md`. Consumer: `app-tui/src/toolboxes/fea_workbench/AGENTS.md`, `app-tui/src/toolboxes/preload_analysis/AGENTS.md`.

## Benchmarks
`benchmark::check(&Solved)` compares an *unmodified* starting template with its closed form (cantilever tip deflection by Timoshenko theory +0.09 %, Lame thick-cylinder bore hoop stress +0.04 %, Heywood net-section Kt of a hole in a finite-width plate +0.6 %); any edit to the problem returns `None`. Shown in the workbench readout and appended to the report. Tolerances (2 / 1 / 3 %) are the closed forms' own accuracy plus the mesh; do not loosen them to pass.

- **Phase 25 additions**: `MeshSpec::adapt_passes` defaults to 2; `MeshSpec::refine` (`RefineRegion`, levels 1-3 = size x0.5/0.25/0.125, <= 16, serde-skipped when empty) is applied by `build::size_function` and clamps the adaptive field; `advice::{refinement_regions, level_order, apply_region}` (rules; `Resolution` = feature size / element size); `Problem::clear_geometry` and `templates::{blank, imported}`; `validate_geometry` now rejects an empty or unbuildable history (`mesh`/`solve` refuse an unbuilt model with the reason). Tests: `tests/refine.rs`.

