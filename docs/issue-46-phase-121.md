# Phase 121: native STEP read into the editable B-rep, round hole and boss booleans (#46/#63)

Continues phase 120 (plan table there). The solid editor's two booleans, `SolidOp::Hole` (a cylinder cut through the part) and
`SolidOp::Boss` (a cylinder fused on a flat face), are now done natively on `NativeBrep`, together with the reader that turns an
existing STEP solid into that editable model.

## What was built

* `native_brep_read.rs`: `NativeBrep::from_step_text`. Reuses `step::inventory` + `step_body::BodyContext` (unit binding,
  `RawBody` topology) and reads exact surfaces (PLANE, CYLINDRICAL_, CONICAL_, SPHERICAL_, TOROIDAL_SURFACE) and edge curves
  (LINE, CIRCLE, also behind `SURFACE_CURVE` / `SEAM_CURVE`, `curve_geometry` unwraps them) into millimetres. `EDGE_CURVE.same_sense`
  false is stored along the curve; a `FACE_BOUND` with no outer marker (OpenCASCADE writes only `FACE_BOUND`) is told apart by the
  largest extent, a tie is an error. Any other surface or curve type (B-spline, extrusion, revolution, ellipse) is an explicit error.
* `native_brep_drill.rs`:
  * `NativeBrep::drill(face, centre, radius, depth)`: a hole from a planar face along its inward normal, blind (`Some(d)`, flat
    bottom) or through (`None`), and across cavities (the walk along the axis alternates exit / entry faces; each exit and entry
    face gets a hole loop, each stretch of material a cylinder wall, a blind end a bottom disk).
  * `NativeBrep::boss(face, centre, radius, height)`: a cylinder wall and a top disk on a planar face, the face gaining a hole loop.
  * Method: the tool axis is perpendicular to the start face, so the intersection is decided on the axis. A perpendicular plane in
    reach is classified against the tool circle as inside the face (a crossing), outside it (skipped), or partial (refused: the
    circle touches an edge). Every other face must be provably clear: parallel cylinders exactly (`|R - r| <= gap <= R + r` meets),
    everything else by bounding box (conservative). A failing check is an error, never an approximated shape.
  * Existing faces keep their indices; new faces are appended (hole walls, bottoms, boss wall, top).

## Evidence

* `cargo test -p solid-kernel --test native_brep_boolean --test native_brep --lib`: 115 passed (default build). Closed-form volumes
  through write -> native read -> sample (chord 0.002 mm, tolerance 1e-3): through hole `400 - pi r^2 t`, blind hole with bottom,
  depth equal to the thickness (= through), boss `+ pi r^2 h`, STEP round trip then a hole through boss and block, a C-shaped
  bracket drilled through two arms across the gap (two walls; blind into the gap removes one arm's share; blind into the second
  arm), a plate with a circle hole taking another hole and a boss. Refusals: hole larger than the face, hanging over an edge,
  tangent to an edge, boss over an edge, start face curved, hole overlapping a neighbouring hole.
* Real files: OpenCASCADE-written fixtures (`edit-plate`, `edit-plate-twin-holes`, `frustum`, `cone`, `cylinder-filleted-top`,
  `cylinder-chamfered-top`, `filleted-box`, `torus`, `tilted-cylinder`, `tilted-hole`, `cross-bore`) that the native importer reads
  are read into the editable model, re-written, and re-imported: same face count and volume to 1e-9. The 40 x 30 x 10 OpenCASCADE
  plate takes a native hole from the top and from the bottom with the closed-form volume (1e-3).
* Parity with OpenCASCADE, `cargo test --release -p solid-kernel --features occt --test native_brep_occt -- --nocapture`
  (booleans built as `fea_problem::solid::apply_op` builds them):

  | operation | faces native / OCCT | exact | OCCT reads native | OCCT boolean |
  |-----------|---------------------|-------|-------------------|--------------|
  | through hole r1 in a 10 x 10 x 4 block | 7 / 7 | 387.43363 | 387.45003 | 387.45003 |
  | boss r2 h3 | 8 / 8 | 437.69911 | 437.67431 | 437.67431 |
  | hole through the boss and block | 9 / 9 | 432.20132 | 432.19098 | 432.19098 |

## Limits

* Only cylinders perpendicular to a planar start face. Tilted holes, countersinks, pockets, boolean of two arbitrary solids,
  and a boss that meets another face are refused (the next general step is a surface-surface intersection engine: size L).
* The bounding-box clearance test makes a refusal possible where OpenCASCADE would succeed (a face whose box meets the tool's
  without the faces touching); that is the conservative direction.
* Fillets, chamfers and the wiring into `fea_problem::solid` / `cad::Brep` (the default build still reports the OpenCascade
  requirement) are later phases.
