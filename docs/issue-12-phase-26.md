# Issue 12 phase 26: egui front end, smaller ZZ-driven refinement zones, real 3D (plan and progress)

Decisions: `docs/adr/ADR-017-egui-wgpu-front-end-and-real-3d-modelling.md`. Design reviewed by the user as an
artifact (designs A, B, C); chosen: B with a thin step bar from C.

## Steps (each lands green before the next starts)

1. **wgpu 0.19.4 -> 30 port** with no visible change (`render.rs`, `window.rs`, `shader.wgsl`, `overlay.wgsl`).
   Gate: `cargo test -p app-tui -p fea-problem --locked -j4 --no-fail-fast`, `--gpu-solid` smoke prints FEA_GPU_READY,
   headless solid snapshots unchanged, `cargo check -p app-tui --no-default-features --lib --bins`.
2. **Refinement zones** (pure `fea-problem`, no UI): new default radii, edge bands, per-zone element estimate, total
   budget, ZZ-seeded re-derivation after a solve. Tests: zone area share on the reference plate, element estimate vs
   real count, ZZ error not worse after a seeded pass.
3. **Typed panel model** in the `Authoring` seam, filled by the solid and member sessions; unit tests per row kind.
4. **egui host**: egui-winit input, egui-wgpu pass over the existing viewport, theme tokens (light and dark), Ctrl+K palette
   reusing the existing ranking.
5. **Design B screens**: floating tool pill, selection-driven inspector, collapsible tree, history chips, thin step bar;
   sketch, mesh, results and empty states. Headless egui snapshots for review.
6. **3D kernel spike** and follow-up ADR (truck vs OpenCascade), then boolean / STEP / face picking and the 3D size
   field in `fea-core`.

## Progress

- 2026-10-11: version pairing verified (see ADR-017 decision 2).
- 2026-10-11: **step 1 done on macOS (Metal).** wgpu 0.19.4 -> 30.0.1 in `app-tui` (`render.rs`, `window.rs`, `shader.wgsl`).
  Evidence: `cargo test -p app-tui -p fea-problem --locked` all pass (app-tui lib 1070 passed, 8 ignored);
  the 7 `#[ignore]`d GPU tests (`-- --ignored gpu_viewer`) pass, snapshots render; `--no-default-features` check and
  clippy on `gpu_viewer` clean. API changes: `request_device` one argument plus `DeviceDescriptor` defaults, `Option`
  entry points / buffers / bind-group layouts / depth fields, `compilation_options`, `cache`, `multiview_mask`,
  `depth_slice`, `immediate_size`, `TexelCopy*` names, `PollType::wait_indefinitely()`, `get_current_texture()` returns
  `CurrentSurfaceTexture` (no `OutOfMemory`), `present` is `Queue::present`.
  **Defect the port exposed:** the mesh-line pipeline set a depth bias, which wgpu 30 rejects on `LineList` (pipeline
  creation panicked in the GPU tests). Replaced by a `vs_mesh` vertex entry that offsets clip depth by 1e-5.
  Not verified: the mesh-line overlay by eye (no snapshot draws it), a real window / `--gpu-solid` smoke, real
  mouse and keyboard, Windows (DX12 backend) and CI.
- 2026-10-11: **step 2 done** (`fea-problem`, verified): hole zone 1.25 r blending to 2 r, edge supports / loads as bands
  (`RefineRegion { to, blend }`, capsule outline in the viewport), corner 0.03 diagonal; `build::estimate_mesh` (per-zone element
  counts, full-refinement and touched shares) shown in the Mesh panel as a budget; ZZ error zones (`advice::error_zones`,
  `add_zones`) are folded in at the next Generate mesh or Solve as one undoable edit (adding them at once would make the shown
  result look stale). Plate with a Ø56 hole: today's zones fully refine 57 % and touch 100 %, the new ones 10 % and 31 %.
  **Defect fixed on the way:** the "too fine" guard in `sketch_mesh` counted 3 elements per h^2 for quads; the mesher makes
  about 9.5 (tri 3.2), so it undercounted about 3x and ignored holes and zones; it now uses the calibrated estimate.
- 2026-10-11: **steps 3-5 done** (egui chrome, design B + step strip). `gpu_viewer/gui/` (egui 0.36.2 + egui-wgpu + egui-winit),
  `Renderer::paint_egui` / `discard_egui`, typed `DockItem::Field`, `Authoring::step_state` and `start_options`. Removed:
  `chrome.rs`, `dock.rs`, `ribbon.rs`, `layout.rs`, `overlay.rs`, `overlay.wgsl`, the menu state machine and the bitmap-font
  dependency (`font8x8`), the renderer's overlay slots. Evidence: 1034 app-tui lib tests (17 window tests drive the real
  chrome headlessly: tool / step clicks, palette, right-click, typing, wheel, drag, box select, mesh -> solve -> results, start
  card), 7 GPU tests on Metal including egui painting and the discard path, light and dark snapshots reviewed by eye, the real
  binary draws frames for 10 s without a panic. Defect found only by the real window: dropping a frame's texture deltas when no
  surface texture was acquired panicked (fixed, GPU regression test).
  Not verified: real mouse and keyboard on a desktop, Windows, CI. Step 6 (B-rep kernel spike, 3D size field) not started.
- 2026-10-11: **contextual inspector and adaptive layout** (ADR-018, user direction). The right dock is gone: floating cards
  (`Authoring::inspector`) show only the focused feature's value fields plus step cards; choices and actions moved to tree-row
  right-click menus (`Authoring::tree_menu`, `solid/inspect.rs`); duplicate lines removed; `editor/adapt.rs` learns tool order,
  folded cards, tree open/rail and the usual next step, persisted in `layout.json`, switchable, resettable, explainable.
  Defects found on the way: an egui `Area` content is 400 points tall by default (cards clipped until `set_max_height`); the
  inspector covered the pill's buttons and the model (it now sits beside the model in reserved room). Evidence: 1054 app-tui lib
  tests pass (25 window tests, 10 adapt tests, scoping / tree-menu tests), light and dark snapshots reviewed.
- 2026-10-11: **in-viewport dimension values and constraint glyphs** (item-4 list): `Authoring::overlay`, drawn by egui over the
  model (`sketch_overlay` places 12 / 4 / R0.8 / H / V / F on the geometry; unit test + snapshot).
- 2026-10-11: **step 6a done: kernel spike and ADR-019.** truck 0.6: coplanar boss and one round-number hole fail, a tangent
  hole returns a wrong solid, no fillet; OpenCascade 7.8.1 (`opencascade` 0.3, built here in about 25 min with
  `CMAKE_POLICY_VERSION_MINIMUM=3.5`, cmake installed with brew): 40/40 holes in 98 ms, coplanar boss, pocket, fillet, STEP round
  trip all correct. Decision: OCCT behind a `solid-kernel` crate with an off-by-default `occt` feature. Finding: OCCT meshes
  surfaces only, so a tetrahedral volume mesher (size-field driven, which is also the 3D adaptive-refinement core) is built in
  `fea-core` first, testable on analytic solids without OCCT.
- 2026-10-11: **step 6b: tetrahedral volume mesher** (`fea-core/src/tetmesh.rs`, ADR-019 decision 3). Kuhn lattice, longest-edge
  bisection with Rivara propagation driven by an arbitrary size field (conforming, tested), feature-preserving snapping from
  per-face distances (`FaceHit`), quality-guarded snapping and smoothing, Tet4 / Tet10 with curved boundary midside nodes,
  named outward faces. Evidence (`fea-core/tests/tetmesh.rs`, 6 tests): a box is filled exactly with outward faces; a sphere's
  volume error 0.8% -> 0.15% (h 0.3 -> 0.15), conforming, boundary on the surface; a size field grades the mesh; a bar passes
  the uniaxial patch test for Tet4 and Tet10 to 1e-8; a thick cylinder under pressure vs Lame: 6.7% -> 0.9% radial
  displacement error (h 0.2 -> 0.1) with Tet10. **Defects found while building it:** nearest-surface snapping rounds sharp
  edges into chamfers (a cylinder's bore area came out 13% too large and the error did not shrink with h) -> feature
  snapping onto the edge of the faces a vertex is near; curved Tet10 midside nodes inverted elements -> accepted only while every
  tetrahedron around the edge keeps a positive Jacobian at 11 sample points; snapping slivers (quality 0.007) -> quality
  guard and smoothing (worst 0.1 to 0.16). Limits: thin features under about two elements are not resolved, interior
  sharpness is rounded within an element, worst quality about 0.1.
- 2026-10-11: **step 6c: `solid-kernel` crate** (ADR-019). Always built: `SurfaceSolid` (weld, T-junction repair, closed-manifold
  check, outward orientation, BVH signed distance with angle-weighted pseudo-normals, per-face distances) and `mesh_volume`
  (through `fea-core::tetmesh`). Feature `occt` (OpenCascade 7.8.1, about 20 min to build): `occt::Brep` (extrude a sketch
  profile with holes cut as booleans, union / cut / intersect, fillet, STEP in and out, face-tagged tessellation). Evidence:
  analytic tests (exact box distances at faces, edges and corners; tube meshed to 0.3% / 0.65% volume) and OpenCascade tests
  (40 round-number holes, coplanar boss, pocket, fillet, STEP round trip, a bar solved through the whole pipeline passes the
  patch test, and **a plate with a hole from the kernel gives sigma_max 324.8 against Peterson 322.7 (0.6%)** through
  extrusion, boolean, tessellation, distance queries, an adaptive Tet10 mesh refined at the hole, solve and stress recovery).
  **Defects found:** the `opencascade` crate has no point-classification or distance API (so distances are our own BVH over
  the tessellation, kernel-agnostic); its `Face::normal_at_center` throws a C++ exception on a curved face, which aborts the
  process (so normals come from the tessellation); a face built with a hole wire oriented like the outer wire is invalid and
  triangulates the hole itself (holes are now cut as booleans); meshing faces one by one re-discretises edges and leaves
  T-junctions (the whole shape is meshed once and split into faces by connected components, plus a T-junction repair);
  a padded lattice cut every planar face through its cells (the lattice now covers exactly the bounding box).
  Not done: the problem-schema side (`fea-problem` geometry from a kernel solid, boss / cut / fillet history features),
  the window (face picking, STEP in the Open card), the license confirmation of ADR-019. CI: `.github/workflows/solid-kernel-occt.yml` (manual or on kernel / mesher changes, cached; not yet run on a hosted runner).


## 3D adaptive size field (fea-core `adapt.rs`)

`SizeField3` + `adapted_size_field_3d(mesh, zz, opts)`: the tetrahedral analogue of `adapted_size_field` (error equidistribution on the ZZ
estimate, tet edge from volume, nodal geometric mean, grading relaxation along tet edges, grid-accelerated barycentric lookup with
clamped weights just outside). Hand `|x| field.at(x)` to `tetmesh::Solid::size`. Evidence (`fea-core/tests/tetmesh.rs`): on the thick
cylinder the second mesh puts >1.5x the inner/outer element ratio of the first and the ZZ relative error falls 0.84 -> 0.031.

Bug found by it and fixed: the snapping reach and tolerance in `mesh_solid` came from the global `boundary_size`, so where the size
field asked for much smaller elements several vertices snapped onto one feature point (coincident, unconnected nodes; 37 % radial
error). Reach is now local (longest incident edge, capped at `boundary_size`). Regression test added.
Test-harness note: bind symmetry planes by surface name (`face3`..), not by coordinate - snapped nodes may sit 1e-3 off a plane.

## Kernel binding switched to `cadrum`; `Geometry::Solid` (2026-10-11)

- **Binding**: `opencascade` 0.3 (25 min source build) replaced by `cadrum` 0.9 (prebuilt static OCCT 8.0.1; Windows MSVC included).
  Evidence table and caveats in the ADR-019 addendum. Local change in `solid-kernel/src/occt.rs`; the 5 `tests/occt.rs` and 6
  surface tests pass, plate with hole 0.8 % from Peterson. Found: cadrum's `Face::id` is not unique (the two extrusion caps share
  one) and is session-local; `tessellate` splits shared ids by connected piece and nearest face centre. Its build-script download
  failed in this sandbox ("unexpected end of file") while curl worked; `OCCT_ROOT=<extracted tarball>` is the fallback and the
  offline route. macOS arm64 only: the Windows prebuilt and the CI job (now ubuntu + windows) have not run.
- **`Geometry::Solid { step, cache }`** (`fea-problem/src/solid.rs`): STEP text is the source of truth, `cache` is the face-tagged
  surface mesh tied to it by an FNV-1a hash. Solving needs only the cache (`solid_mesh` -> `SurfaceSolid` -> `mesh_volume`, Tet4 or
  Tet10 from the element choice, `MAX_TETS` cap) so a build without `occt` solves; `tessellate_step` / `Geometry::solid_from_step`
  need the `occt` feature and fail with a clear message otherwise. A stale or missing cache is refused with the reason, never
  solved. Faces are `face1`.. in the kernel's face order. `Problem::set_solid` drops the sketch history. serde stays tagged and
  backward compatible (old files and every template round-trip). Refinement zones and sketch history features do not apply to a
  stored solid (zones are sketch-plane; boss / cut / fillet history features are the next step and need `occt`).
  Tests: `fea-problem/tests/solid.rs` (4 default + 1 `occt`: kernel plate with hole through STEP text to a solve).
- `app-tui`: every exhaustive `Geometry` match handles `Solid` (names, tree row "STEP solid (N faces)", no hole tool, no
  history conversion, Source row label). No UI to open a STEP file yet.

## Failure guidance, slice 1 (ADR-020, 2026-10-11)

- **Defect found and fixed (Fix-Everything):** `fea-problem` never ran the kernel's `Model::validate`. A NaN load solved to a
  silent zero; "no load" gave a silent zero. `solve_mesh` now runs it: errors stop the solve with the reason, warnings join
  `Summary::notes` (the low-order-element warning is skipped there, `notes()` already has it). Contact (bushing) solves are
  not covered (a bushing is held only by contact, the rigid-body check would be wrong there).
- **`fea-problem/src/diagnose.rs`**: `diagnose(&Problem, Option<&str>)` -> `Diagnosis { class, summary, evidence, remedies,
  questions }`; `Edit` is data (`Advice` boundary/mesh size, `Retarget`, `Drop`). 11 tests in `tests/failures.rs` reproduce
  real failures (no supports, one roller, a mistyped surface, a mesh over the limit, a NaN load) and assert that applying the
  top remedy makes the failing solve succeed. Open: the rest of the failure classes, the coarse-mesh hint (size 1e6 solves
  silently), UI, recording, ranker wiring. UI mock in ADR-020 awaits the user's approval.

## UI for guidance, STEP workflow, build fix (2026-10-11, later)

- **Build failure on the user's machine**: the linker failed with "No space left on device" behind it on this machine (179 MB
  free; `target/` 31 GB, 14 GB of it `debug/incremental`). Freed regenerable data (obsolete `target/OCCT`, incremental, scratch
  spikes); both `cargo build -p app-tui` and `--features occt` link. Run `df -h` first if the linker fails again; use
  `CARGO_INCREMENTAL=0` for long test runs.
- **Guidance UI** (ADR-020 accepted): card + status-bar fix + undo + recording + fourth ranker. Snapshot test
  `guidance_snapshot` (`FEA_SNAPSHOT_EDITOR_OUT=dir cargo test -p app-tui --lib -- --ignored guidance_snapshot`) renders it in
  the real renderer; looked at (dark theme) and correct. New failure classes `MeshTooCoarse`, `HoleOutsideOutline`; found and
  fixed: the mesher numbered holes from 0 ("hole 0 is not inside") while the surfaces are `hole1`..; a mesh size of 1e6 solved
  without a word.
- **STEP in the editor** (feature `occt` of `app-tui`, which links a prebuilt OpenCASCADE): File > Open takes `.step` / `.stp`
  (`Problem::from_step`: cache at 1/1000 of the bounding-box diagonal, element size a twelfth of it); Insert > Fillet all
  edges / Hole in selected face / Boss on selected face (`fea_problem::solid::SolidOp`, kernel booleans, exact-volume tests:
  hole 0.2 %, boss 0.5 %). Each is one undoable edit; supports and loads keep their `faceN` names, which the kernel renumbers,
  so the guidance diagnoses the dangling ones (unknown surface -> retarget / drop). Faces are the pickable surfaces of the
  preview mesh (`face1`..). Without the feature the same menu entries are disabled with the reason.
- **Not done** (each needs more than this session had): stable topological naming across boolean edits (OCCT history map;
  `cadrum::Solid::iter_history` is the hook), sweep / islands, native arc `Shape`, datum-plane picker, construction geometry and
  splines, thermal problem type, terminal Workbench auto-preview / auto-solve, hover preview of a fix, viewport picking of a
  face for the hole / boss by click on the real GPU window (the preview scene's picking is the existing surface picking and
  was not exercised with a real mouse), Windows and hosted CI (the `solid-kernel-occt` job has never run), the LGPL notice
  check before release.

## Session 3 (2026-10-12): stable face naming, Windows CI finding

- **Stable topological naming (done).** `fea_problem::solid::remap_faces(old, new)` matches faces across a solid edit by geometry
  (same flatness, plane and normal for flat faces, centroid within a third of the face size, area within x20; best match first,
  each new face once). `Problem::apply_solid_op` renames face supports/loads through it; a face the edit removed becomes
  `removed:faceN`, which `diagnose` reports as an unknown surface (no retarget is offered: too far from any real name). Chosen
  over `cadrum::Solid::iter_history` because kernel face ids are session-local, not unique for extrusion caps and lost by STEP;
  geometry is stable across all of that and needs no kernel (the function is tested without `occt`). Tests:
  `fea-problem/tests/solid.rs` (`remap_faces_*`, `naming::*`: hole/boss on a side face and through the loaded face, fillet, solve
  after).
- **Windows CI finding (hosted run 38236448142).** `solid-kernel` `--features occt` links on ubuntu but fails on windows with
  `unresolved external symbol __imp_*`: cadrum's prebuilt MSVC OpenCascade is `/MD`, the workspace forces
  `-C target-feature=+crt-static` (`.cargo/config.toml`, the self-contained-exe rule). The workflow now (a) checks the code on
  windows against the prebuilt with `-crt-static` (not the shipping configuration) and (b) has a manual job building OCCT from
  source (`occt-source` feature) under `crt-static`, the configuration the shipped exe needs. Until (b) is green the Windows
  exe with `occt` is not shippable without bundling the VC++ runtime DLLs app-locally.

### Sketch features closed this session (2026-10-12)

The "Known limits" of `issue-12-phase-24.md` that were open are now built; each has tests (`fea-problem/tests/{sketch,swept}.rs`,
`fea-core/tests/sweep.rs`, `app-tui` `sketching_tests.rs`).

| Item | What it is now |
|---|---|
| Native arcs | `Shape::Outline { segs: Vec<Seg> }` (`Seg::{Line, Arc, Spline}`): exact arcs, one `edgeN` per edge (was 48 polygon edges per turn). The mesher places nodes on the arc (`fea-core` `Curve::Arc`). |
| Construction geometry | `Sketch::construction`: solved, dimensionable, never part of the profile or its closure / branch checks. |
| Islands and separate outlines | `Profile::regions` / `Geometry::Sketch { regions }`: a loop inside a hole is the outline of another region; each extra region is meshed alone and appended as a separate body, surfaces `r2/...`, `r3/...`. Crossing outlines are refused. A bushing needs a one-region sketch. |
| Sweep | `FeatureKind::Sweep { profile, path, layers }`: profile carried along an open, tangent-continuous chain of lines and arcs on a rotation-minimizing frame (closed form, `fea-problem/src/path.rs`); `fea_core::sweep::sweep_along`. Volume = Pappus to 2e-3, straight path = extrusion. The path must leave the profile along its +normal. |
| Datum / face planes | `History::planar_faces()`: caps and straight sides of the extruded body as outward datum planes; `Sketch on face '...'` in the Features ribbon and Insert menu. |
| Splines | `Entity::Spline { pts }` (centripetal Catmull-Rom through points) and `Seg::Spline`; Spline tool (`B`; Enter or a second click on the last point finishes). Meshed as a fine chain of pieces of one surface; not allowed in sweep paths. |

Fixed on the way: an extrusion of depth exactly 2 pi was treated as a closed revolution (nodes merged); JSON floats lost the last
bit on a save / load (`float_roundtrip`); the C# reference workflow and csproj looked for `native_search.dll` in a directory
the workspace migration emptied.

**Windows-only wrong 3D solves: found and fixed.** `random_linear_problems_solve_and_balance` (45k-dof Hex27 case, reactions 12 % short
of the loads) and `solid-kernel`'s plate-with-a-hole test (peak stress 40 % low) failed on some hosted Windows runners and
nowhere else, with identical wrong numbers each time. The sweep was not a race: `fea-problem/tests/consistency.rs` run on six
sampled runners showed the stiffness matrix exact (`K * translation` 1e-16) and the *direct* solution wrong (`K u = f` off by
0.31) on every `AMD EPYC 7763` host, right on `EPYC 9V74`, `9V45` and the Intel `8370C`. Cause (upstream, found by search, then
confirmed here): `private-gemm-x86` 0.1.20, the matrix-product kernel under faer, returns wrong entries for its threaded product
on that CPU; 0.1.22 fixes it ([report](https://github.com/tachsin/photonoxide/pull/192)). `Cargo.lock` now pins 0.1.22; the same
test on 5 hosts including three 7763s then gave the correct, identical residual (3.6e-13) everywhere.

Kept as guards, because the next defect of this kind will not announce itself: `Model::solve_direct_many` checks its own relative
residual and repeats a bad solve on one thread (`Solution::fallback` says so), `fea_core::sequentially`, and `fea-problem`'s
`solve_mesh` verifies residual and force balance (against the total force magnitude) before it returns a result, refusing one
that still fails. A repeat shows up in `Summary::notes`.

### Terminal Workbench is manual by default (2026-10-12)

ADR-016 decision 1 (nothing meshes or solves on its own) now holds in the terminal Workbench too, which had its own
auto-preview and `auto_solve` default on. `u` generates the mesh, `r` solves, an edit leaves both out of date; `a` is an opt-in
automatic mode (the old behaviour). Test: `nothing_meshes_or_solves_by_itself_until_asked`; the tests of the automatic behaviour
run in `auto_state()`.

### Thermal studies (2026-10-12)

`Problem::study` / `Problem::thermal`, ADR-021, `fea-problem/src/thermal.rs`; closed-form checks in `fea-problem/tests/thermal.rs`
(linear bar, flux + convection, generation, constrained thermal stress), UI in both front ends (`FieldRow::StudyKind`,
Thermal section, tree nodes, surface menu kinds), `diagnose` class `NoThermalAnchor`.

### Windows findings from the hosted Windows run (2026-10-12)

Running the whole workspace on a hosted Windows runner (`.github/workflows/windows-tests.yml`) found, besides the sweep flake
above: the fast re-search walker entered `.native-search-index` (a leading dot hides a folder only on Unix), so the index
build counted its own `meta.json` (fixed in `file_reader::enumerate_files_safely`, which now never enters that folder); two
tests that assumed a `/` path separator and a Unicode ellipsis.
