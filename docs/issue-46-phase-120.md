# Phase 120: native modelling begins - owned analytic B-rep, STEP write, profile extrude and lathe (#46/#63)

Owner decision: every OCCT-only editor operation gets a native implementation. This phase carries the plan, the shared data
model (`solid-kernel/src/native_brep.rs`) and the first operations: STEP export, extrusion of a sketch profile and revolution of
a polyline. Booleans, fillets / chamfers and the editor wiring follow in later phases.

## Plan (what the application uses, and the native replacement)

Consumers: `fea_problem::solid` (`tessellate_step`, `import_step`, `SolidOp::{Fillet, Hole, Boss}` via `apply_solid_op`), the
solid editor's sketch extrude (`Profile`), `app-tui/.../editor/solid/stepops.rs`. The facade is `solid_kernel::cad::Brep`
(`#[cfg(feature = "occt")]`).

| Op | OCCT signature (`occt::Brep`) | Native representation | Proof strategy | Order | Size |
|----|-------------------------------|-----------------------|----------------|-------|------|
| STEP write | `to_step_text`, `write_step` | `native_brep::NativeBrep` (analytic surfaces, line / circle edges, oriented loops) -> `native_brep_step` AP214 text | round trip through the native reader (face count, signed volume); OCCT reads the native text (`--features occt`) | 1 (done) | S |
| Profile extrude | `extrude(&Profile, z0, depth)` | `NativeBrep::extrude`: planes, cylinders for circle loops, holes | closed-form area x depth; reader volume; OCCT parity | 2 (done) | S |
| Lathe (polyline) | `lathe_*` are spline only; polyline revolve is what bushings / sleeves need | `NativeBrep::lathe`: cylinder / plane / cone faces | Pappus closed form | 2 (done) | S |
| Primitives | `box_from_corners`, `cylinder` (cone done via lathe; sphere, torus, apex cones need pole handling) | `NativeBrep::box_from_corners / cylinder_z / lathe` | closed form | 2 (done for box, cylinder, frustum) | S (sphere / torus / apex: M) |
| Tessellate | `tessellate(deflection)`, `solid(deflection)` | already native: written STEP -> `AnalyticModel::sampled_surface` (face tags = source faces) | volume vs closed form | 1 | wiring only (M with `fea_problem` switch) |
| Boolean (planar + cylinder: Hole, Boss, cut/union of boxes and cylinders) | `boolean(op, &other)` | B-rep face splitting on the owned model (plane/cylinder/plane-plane intersections), then loop rebuild | closed-form volumes; differential vs OCCT | 3 | L |
| Fillet all / round / chamfer at height | `fillet_all`, `round_edges_at_height` | rolling-ball strips: cylinder / torus / cone / sphere corners on planar+cylindrical solids | Minkowski closed form (`rounded_box`), existing parity matrix | 4 | L |
| Editor wiring | `cad::Brep` facade | `cad::Brep` backed by `NativeBrep` when `occt` is off | `fea-problem` / `app-tui` tests | 5 | M |

## What was built

* `native_brep::NativeBrep`: vertices, edges (`Curve::Line` / `Curve::Circle`), faces (`Surface::{Plane, Cylinder, Cone, Sphere,
  Torus}`, same-sense flag, outer loop, hole loops of oriented edge uses). Orientation convention: every loop runs
  counter-clockwise seen from outside the material (holes clockwise), each edge forward once and reversed once.
* `NativeBrep::extrude(&Profile, z0, depth)`: polygon and circle outer loops, any number of polygon / circle holes; input winding and
  the sign of `depth` do not matter; degenerate input (< 3 points, zero side, zero area, non-positive radius, zero depth) is refused.
* `NativeBrep::lathe(&[[r, z]])`: a closed (r, z) polyline about the z axis. A constant-radius side is a cylinder, a constant-z
  side a plane (disk or annulus), anything else a cone (positive half-angle; the axis flips when the radius shrinks with z). A vertex
  on the axis is allowed only between sides on the axis or in a plane; a cone apex / sphere pole is refused (explicit error, next).
* `native_brep_step::NativeBrep::to_step_text`: AP214 skeleton as OpenCASCADE writes it, millimetres, bare `LINE` / `CIRCLE` edge
  curves (no p-curves needed by the importer), shortest round-trip reals.
* Closed forms (not reading the shape): `extrude_volume`, `lathe_volume` (pi * integral r^2 dz), `profile_area`.

## Evidence

* `cargo test -p solid-kernel --test native_brep` (default build, no OCCT): 7 passed. Round trips through `inventory` ->
  `AnalyticModel::read` -> `sampled_surface`: the extruded L profile has 8 faces and volume 12 to 1e-9; clockwise input and a
  downward depth give 24 to 1e-9; a plate with a polygon and a circle hole has 11 faces and volume within 1e-3 of
  `(48 - 4 - pi) * 0.25` at chord 0.001 mm; cylinder, frustum, washer with a conical wall and a stepped flare match Pappus within
  2e-3 (chord 0.002 mm; an inscribed sampling is a little small). A mutation (hole wall `same_sense: true`) makes the plate test
  fail with "native sampled surface is not closed and consistently oriented", so the tests see orientation errors.
* `cargo test --release -p solid-kernel --features occt --test native_brep_occt -- --nocapture` (OCCT builds locally): OpenCASCADE
  accepts the native text and finds the same face count and volume as for its own shape of the same construction:

  | shape | faces native / OCCT | exact | OCCT reads native | OCCT built |
  |-------|---------------------|-------|-------------------|------------|
  | extruded L | 8 / 8 | 12 | 12.00000 | 12.00000 |
  | plate, circle + square hole | 11 / 11 | 10.80365 | 10.80417 | 10.80417 |
  | box | 6 / 6 | 24 | 24.00000 | 24.00000 |
  | cylinder R2 x 3 | 3 / 3 | 37.69911 | 37.67431 | 37.67431 |
  | frustum R6 -> R3 x 8 | 3 / 3 | 527.78757 | 527.56761 | 527.56761 |

  (OCCT's own tessellation at 0.002 mm is the same on both, so the native text is the same solid.)

## Limits

* Revolution is a polyline only: no arcs (so no fillet-like profiles, spheres, tori), no apex or pole vertices.
* No face-order compatibility with OCCT: the face numbering of a shape is the construction order here (`faceN` of the editor is
  by construction order, so stored supports / loads on a native solid are named by this order).
* Not wired into `fea_problem` / `cad::Brep` yet (the default build still says "needs the OpenCascade build").
