# Phase 123: the solid editor on the native pipeline in the default build (#46/#63)

Phases 120-122 built the native modeller (`NativeBrep`: extrude, lathe, STEP write/read, hole, boss, convex fillet). This phase puts
it behind the editor: in a build without `occt`, `fea_problem::solid::{tessellate_step, import_step, apply_op}` now use the
native pipeline instead of answering "needs the OpenCascade build". An `occt` build is unchanged (OpenCascade stays the
implementation there; the native functions remain public for comparison).

## What changed

* `fea-problem/src/native_solid.rs` (new): `tessellate_step_native` (STEP text -> `AnalyticModel` -> face-tagged `SurfaceMesh`;
  strict read first, then the tolerant `Options::foreign_cad` read with chart canonicalization for files from other kernels),
  `import_step_native` (extent from the model's edge samples, cache at a thousandth of the diagonal, start element size shared with
  the OpenCascade import through the new `solid::finish_import`), `apply_op_native` (`SolidOp::Fillet` -> `fillet_all_convex`,
  `Hole` -> `drill` through all, `Boss` -> `boss`).
* The flat face an edit names (`faceN` of the cache) is found in the B-rep by plane and position
  (`NativeBrep::planar_faces_through`, `face_holds`), not by number: a cache made by OpenCascade numbers faces differently from
  the STEP text order the native reader uses. Coplanar faces are told apart by which one holds the face centre.
* `app-tui` `stepops.rs`: the "needs the OpenCascade build" guard is removed (an `occt` build behaves as before). Messages of
  `problem.rs` / `units.rs` that sent the user to the OpenCascade build now say to open the STEP file again.
* Behavioural differences from OpenCascade, by design: native refusals are explicit (non-convex fillet, tilted tools, a boss that
  would meet another face, a hole centred on an existing hole); OpenCascade would attempt them.

## Evidence

* `cargo test -p fea-problem --test native_solid`: 3 passed: a native cube opens as a 6-face problem, takes a boss (+ pi r^2 h),
  a hole through the boss and cube (- pi r^2 * 13) and a 0.5 fillet (26 faces, rounded-cube closed form) with volumes within
  2e-3 / 5e-3; refusals say why (no face99, curved face, negative radius, hole wider than the face); supports and loads on the
  bottom and top follow their faces through a hole in a side face, and the edited solid meshes and solves (equilibrium error
  below 1e-3, stress above the 5 MPa traction).
* `app-tui` `guidance_tests::a_step_solid_opens_gets_a_hole_and_a_dangling_support_is_diagnosed`: in the default build a non-solid
  file is refused ("Could not open"), a native cube opens ("6 faces"), a hole from the Insert prompt makes 7 faces, undo restores 6.
* `cargo test -p fea-problem`: every suite passes (the one test that asserted the old "needs OpenCascade" refusal now asserts a
  reasoned refusal of a STEP text without a solid). `cargo test -p app-tui --lib guidance_tests`: 8 passed. `cargo test -p app-tui
  --lib gpu_viewer`: 367 passed, 3 failed with "timed out waiting for the analysis" (`bushing::tests::solving_shows_the_fit_pressure...`,
  two `eccentric::tests`): background analyses in a debug build on a machine at 97 % disk; they do not touch solids and were not
  re-run against the previous commit, so treat them as unverified-unrelated.

## Limits

* The editor operations are the three it has (`Fillet`, `Hole`, `Boss`); each is limited as phases 121-122 state. The native
  tessellation is a plain per-face chart sampling: fine for the surface preview and the signed-distance volume mesher; it is not the
  graded/conforming path (`AnalyticModel::graded`).
* A cache stored by an `occt` build and opened without it still solves (the stored mesh is used); editing it re-tessellates natively.
