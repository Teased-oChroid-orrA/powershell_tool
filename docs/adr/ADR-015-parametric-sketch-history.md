# ADR-015: Parametric sketch + feature history model

Status: Accepted (user direction 2026-10-10: model/problem builder must feel like SolidWorks / NX: planes, sketch
constraints, features, history, BCs on sketch and topology entities)

## Problem

A solid was `Geometry::Sketch { outer, holes, depth, layers }`: one outline of fixed shapes plus circular / polygon
holes. No sketch entities, constraints, planes, feature order, revolve or sweep; supports and loads refer to NAMED
surfaces (`bottom`, `edgeN`, `holeN`, `start`, `end`).

## Decision

1. **A new pure model in `fea-problem`, `Problem::history: Option<History>`, rewrites `geometry`.** `History` is an
   ordered list of `Feature`s (`Sketch{plane, sketch}`, `Extrude`, `Revolve`) with `suppressed` and a roll-back bar.
   `Problem::apply_history()` rebuilds `geometry` (and sets `Analysis::Solid` for a solid feature). Everything
   downstream (mesher, supports, loads, solvers, the terminal Workbench) keeps working on `Geometry` unchanged.
   `history` is serde-default / skipped when absent, `FORMAT_VERSION` stays 1: old files load, new files with no
   history are byte-identical. Rejected: a new `Geometry` variant (every `match` in app-tui and fea-problem would
   change for no gain) and a parallel file format (two sources of truth).
2. **Sketch = entities with stable ids + constraints** (`sketch/mod.rs`): `Point`, `Line`, `Circle`, `Arc` and
   `Coincident, Horizontal, Vertical, Parallel, Perpendicular, Equal, Tangent, PointOn, Midpoint, Fixed` plus driving
   dimensions `Length, Distance, HDist, VDist, Radius, Angle`. Ids are never reused within a sketch; deleting an entity
   cascades to what refers to it. Construction geometry, splines, ellipses and symmetric are out of scope for now.
3. **Solver: Levenberg-Marquardt on the residuals, forward-difference Jacobian, dense algebra** (`sketch/solve.rs`).
   No dependency: sketches are tens of variables. Damping gives the minimum-norm move, so under-constrained sketches
   change as little as possible (the same mechanism implements dragging: weak pulls on dragged points). Reports
   DOF = variables - rank(J), and status `Under / Fully / Over(redundant, consistent) / Conflicting` with the violated
   constraint ids. Rejected: a graph-decomposition (D-Cubed style) solver - needless for this size.
4. **Profile = loops of the solved sketch -> `Shape`s** (`sketch/profile.rs`). Circle -> `Circle`; four axis-aligned
   lines -> `Rect`; other closed chains -> `Polygon` (arcs tessellated, 48 segments / turn). Largest loop is the outer
   boundary, the rest must lie inside it (one outline with holes; islands and separate regions are refused with a
   message). Loop order and start edge are deterministic (lowest entity id) so `edgeN` / `holeN` are stable across
   rebuilds when entities are not reordered. Values are snapped to 1e-10 (no solver noise in names/values).
5. **Planes and frames** (`history.rs`): Front = XY (+Z), Top = XZ (+Y, sketch y = -Z), Right = YZ (+X), all with an
   offset, plus datum planes (origin, normal, x direction). The mesher stays in the sketch frame; `build::sketch_mesh`
   extrudes or revolves (`fea_core::sweep`, revolve axis = sketch y axis, x = radius >= 0) and then maps nodes through
   the plane frame. Names and node sets are unaffected. A plane (2D) analysis needs the Front plane.
6. **One solid feature per history** for now (extrude OR revolve of one sketch, with every outline and hole in that
   sketch). Boss/cut sequences need a boolean kernel we do not have; sweep needs a path sketch and a swept frame and is
   scheduled (`fea_core::sweep` has the general mapping, only extrude / revolve are exposed). Not faked.
7. **Stable topology naming beyond sketch order** (face / edge names that survive reorder, fillets, multiple bodies) is
   NOT solved; BCs bind to the named surfaces above. Recorded as a known limit; revisit when booleans arrive.
8. **Constraint inference is deterministic rules first** (`sketch/infer.rs`): which constraints the geometry already
   satisfies within tolerance (auto relations). A learned ranker may only reorder the allowed set; it ships only past
   the measured gate of `editor/learning.rs` (ADR-014 item 5). No external ML runtime.

## Consequences

* The native window gets a Sketch mode (`gpu_viewer/editor/sketch`) over `Sketch`; features are forms over `History`.
* `Problem` equality now includes `history` (a deliberate part of the model, not a self-changing field).
* The terminal Workbench keeps editing `Geometry`; editing a problem that has a history there desynchronises it
  (documented in `fea-problem/AGENTS.md`; the window rebuilds from the history on open).
