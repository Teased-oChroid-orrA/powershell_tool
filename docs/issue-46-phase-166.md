# Phase 166: editor-operation capability matrix, native against OpenCASCADE

Issues #46 / #63, Track C step 2 of `issue-46-abc-and-retirement-gates-plan.md` (the editor capability matrix the retirement gate lists as a prerequisite; Phase 165 measured STEP import on a licensed corpus). Branch `claude/elated-cerf-e39929`, based on `main` at `1a59333` and merged with `main` at `08ace1d` (Phases 165 and 167; source unchanged by that merge). No gate, tolerance, budget or approval flag was changed; `occt_retirement_approved` stays false.

## What the application exposes

Every solid-editor operation a user can reach, found from `fea_problem::solid::SolidOp`, `Problem::apply_solid_op`, the native editor (`app-tui/src/gpu_viewer/editor/solid.rs`, `solid/stepops.rs`, `solid/ui.rs`) and the `occt` code paths:

- **File > Open a `.step` file**: `Problem::from_step` -> `solid::import_step` (read, tessellate, default element size).
- **Re-tessellate a stored STEP text**: `solid::tessellate_step`.
- **Insert > Hole in face...**, **Boss on face...**, **Fillet edges...**: `SolidOp::{Hole, Boss, Fillet}` through `Problem::apply_solid_op` -> `solid::apply_op`. The menu entries are the same in both builds; there is no `cfg(feature = "occt")` in the editor UI.
- **Supports and loads follow their faces across an edit**: `solid::remap_faces` (geometric, no kernel).
- **Undo / redo of an edit**: editor history.
- **Write-back of the edited STEP text**: the new STEP text and its cache are stored in the model (`.fea.json`). There is no separate "export STEP file" action.
- **Sketch history features** (Sketch, Extrude, Revolve, Sweep): built by `fea-problem` itself (`history.rs`, `fea_core::sweep`), meshes only, no B-rep and no kernel. They behave the same in both builds.

The feature switch selects the kernel in exactly three functions of `fea-problem/src/solid.rs`: `tessellate_step`, `import_step`, `apply_op`. Without `occt` they call `fea_problem::native_solid`; with `occt` they call OpenCASCADE through `solid_kernel::cad::Brep`. Nothing else in `fea-problem` or `app-tui` production code calls OpenCASCADE.

## Matrix

"Supported" means a test proves it; anything without a test is listed as unsupported. Test names are `file::test`.

| Operation | Native (feature off) | Native evidence | OpenCASCADE (feature on) | OpenCASCADE evidence |
|---|---|---|---|---|
| Open STEP | **Partial.** One manifold solid of planes, cylinders, cones, spheres (not a full sphere), tori and B-spline surfaces / curves. Refused with the reason: assemblies and placement mapping, several body representations, ambiguous plane-angle units, inconsistent geometry, a full sphere. Phase 165: 2 of 35 licensed real-CAD files read. | `fea-problem/tests/native_solid.rs::a_native_cube_becomes_a_problem_and_gets_a_boss_a_hole_and_fillets_with_the_exact_volume`; `native_solid.rs::every_native_edit_of_every_fixture_succeeds_or_says_why` (27 of 30 OpenCASCADE-written fixtures open; the 2 assemblies and the full sphere are refused with reasons); `solid-kernel/tests/curved_step.rs::sphere_cone_and_torus_shells_have_the_closed_form_volume_to_the_chord_tolerance`, `::a_full_sphere_is_refused_with_a_reason`; `solid-kernel/tests/step_assembly.rs::ambiguous_unsupported_and_inconsistent_structure_is_refused_with_a_reason`; app: `app-tui/.../solid/guidance_tests.rs::a_step_solid_opens_gets_a_hole_and_a_dangling_support_is_diagnosed` (native block: a non-solid file is refused, a cube opens), `solid/models_tests.rs` (browser open, native block) | **Supported.** Multi-solid files and every Phase 165 file read (30 of 35 then refused by the mesh-size estimate, 2 tessellations not closed). | `fea-problem/tests/solid.rs::a_kernel_solid_goes_from_step_text_to_a_solve`, `solid.rs::editing::a_step_cube_becomes_a_problem_and_gets_a_hole_a_boss_and_fillets_with_the_exact_volume`; `solid-kernel/tests/occt.rs::step_files_round_trip_the_faces_and_the_volume`; Phase 165 table |
| Re-tessellate stored STEP | Supported (same text gives the same cache; an unreadable text is refused). | `native_solid.rs::the_native_tessellation_agrees_with_the_cache_it_replaces` | Supported. | `solid.rs::a_kernel_solid_goes_from_step_text_to_a_solve` |
| Hole through a flat face | **Partial.** A round through hole perpendicular to a flat face whose axis meets only planes normal to it (across cavities) or faces provably clear of it. Refused: a curved start face, a tool overlapping a face edge or another hole, a cross bore, a hole wider than the face, any solid with B-spline, ellipse or complex edge curves (it cannot be read for editing). | `native_solid.rs::a_native_cube_becomes_a_problem_...`, `::supports_and_loads_follow_their_faces_and_the_edited_solid_solves`, `::a_native_refusal_names_the_face_as_the_editor_does`, `::every_native_edit_of_every_fixture_succeeds_or_says_why` (holes cut in the filleted box and the tilted cylinder); `solid-kernel/tests/native_brep_boolean.rs::a_through_hole_removes_pi_r2_t`, `::a_hole_through_two_arms_crosses_the_gap`, `::a_hole_drilled_into_an_opencascade_plate`, `::unsupported_or_unsafe_requests_are_refused_not_approximated`; app: `guidance_tests.rs` native block | Supported (any boolean cut OpenCASCADE accepts). | `solid.rs::editing::a_step_cube_...`, `solid.rs::naming::boundary_conditions_follow_their_faces_through_a_hole_a_boss_and_fillets`; app: `gpu_viewer/window.rs::a_hole_is_cut_in_a_face_picked_with_the_mouse_and_a_support_on_another_face_survives_it`, `guidance_tests.rs` occt block |
| Boss on a flat face | **Partial.** A round boss on a flat face that meets no other face. Refused as for the hole, plus a boss that would meet another face. | `native_solid.rs::a_native_cube_becomes_a_problem_...`, `::every_native_edit_of_every_fixture_succeeds_or_says_why` (bosses on the plates, chamfered / filleted cylinders, filleted box, tilted cylinder); `native_brep_boolean.rs::a_boss_adds_pi_r2_h_and_holes_follow_through_the_step_text`, `::a_plate_with_holes_takes_more_holes_and_a_boss_beside_them` | Supported. | `solid.rs::editing::a_step_cube_...`, `solid.rs::naming::a_boss_nearly_as_wide_as_the_loaded_face_leaves_the_load_on_the_remaining_ring` |
| Fillet every edge | **Partial.** Only a convex polyhedron whose vertices each join three faces, exact (lines, arcs, cylinders, spheres). Refused: a non-convex solid, a face with a hole, any curved face or edge, a vertex of another degree, a radius the offset body does not survive. | `native_solid.rs::a_native_cube_becomes_a_problem_...` (closed-form rounded-box volume); `solid-kernel/tests/native_brep_fillet.rs::a_box_with_every_edge_rounded_matches_the_closed_form`, `::rounded_prisms_match_the_steiner_formula`, `::what_is_not_a_convex_polyhedron_is_refused`; `fea-problem/tests/fillet_rim_tagging.rs` (native rounded box); app: `guidance_tests.rs` native block (fillet of a holed cube refused in the status, model unchanged) | Supported (OpenCASCADE `fillet_all`). | `solid.rs::editing::a_step_cube_...` (26 faces), `solid.rs::naming::boundary_conditions_follow_...` (fillets then a solve); `solid-kernel/tests/native_brep_occt.rs::native_fillet_all_against_opencascade` |
| Supports / loads follow faces | Supported (kernel-independent). | `solid.rs::remap_faces_follows_a_renumbering_and_reports_a_removed_face`, `native_solid.rs::supports_and_loads_follow_their_faces_and_the_edited_solid_solves` | Supported. | `solid.rs::naming::*` |
| Undo / redo of an edit | Supported. | `guidance_tests.rs` native block | Supported. | `guidance_tests.rs` occt block |
| STEP text write-back | Supported for what the native modeller builds (AP214, planes / cylinders / cones / spheres / tori, lines and circles). | `solid-kernel/src/native_brep_read.rs::a_native_text_reads_back_to_the_same_shape`; `native_brep_boolean.rs::opencascade_files_read_into_the_editable_model_and_round_trip`; `native_brep_fillet.rs::a_filleted_shape_goes_through_the_step_text_and_back` | Supported. | `solid-kernel/src/cad.rs::adapter_preserves_surface_tags_and_step_roundtrip`; `native_brep_occt.rs::opencascade_reads_what_the_native_writer_writes` |
| Sketch / Extrude / Revolve / Sweep | Supported, no kernel involved. | `fea-problem/tests/sketch.rs::history::extrude_rebuilds_geometry_and_meshes_in_place`, `::history::revolve_makes_a_solid_of_revolution`; `tests/swept.rs::a_bent_sweep_has_the_pappus_volume_and_lands_on_the_path` | Same code. | Same tests |

### OpenCASCADE operations the application does not expose

`solid_kernel::occt::Brep` also has general booleans (union, cut, intersect of arbitrary shapes), chamfers and fillets of selected edges (`round_edges_at_height`), cone / sphere / torus primitives, spline lathes and prisms, a B-spline shell, and STEP read / write by path. Only tests and examples call them. They are not rows of the editor matrix; if the editor ever exposes one, it needs a native row first.

### Not available in either build

B-rep healing, IGES, persistent topology (`solid-kernel/src/cad.rs::availability_is_not_full_cad_parity`), exact trims. OpenCASCADE does not heal the two non-manifold tessellations of Phase 165 either. `cad::capabilities()` describes the OpenCASCADE backend only (with the feature off it reports that backend unavailable); it is not the application's matrix and nothing in the application reads it.

## Refusal audit with the feature off

Every native operation was run on every OpenCASCADE-written fixture of `solid-kernel/tests/data` (30 files; `native_solid.rs::every_native_edit_of_every_fixture_succeeds_or_says_why`): 27 files open, the 2 assemblies and the full sphere are refused at import with the reason; of 100 edits, 13 succeed and 87 are refused with a reason. **No panic and no silent fallback was found**: a native build has no path to OpenCASCADE, a refused edit leaves the model unchanged (`apply_solid_op` returns before it writes), and the editor shows the error in the status bar (`stepops.rs::run_prompt_op`).

Two defects in how refusals read were found and fixed (`fea-problem/src/native_solid.rs::refusal`):

1. **Wrong face named.** The native modeller names faces by its own index from 0 ("the tool may touch face 6"); the editor names them `face1`.. by the cache, and a cache made by OpenCASCADE numbers them differently again. A user looking for "face 6" found the wrong face, or none. Each `face <index>` in a native refusal is now renamed to the cache face it is, by `remap_faces` against a native tessellation of the same text (computed on the refusal path only). Regression: `native_solid.rs::a_native_refusal_names_the_face_as_the_editor_does` (a fillet of a holed cube names a face normal to z; a cross hole names the bore, the only curved face). It runs in both builds, so it also covers a cache made by OpenCASCADE.
2. **Unsupported edit not identified as such.** A solid the native importer reads but the native modeller cannot edit (B-spline, ellipse or complex edge curves) answered every edit with a bare "#83: unsupported complex curve". Every native-modeller refusal now starts "without OpenCASCADE, this solid cannot be edited:" / "the edges cannot be filleted:" / "this hole cannot be cut:" / "this boss cannot be added:". Regression: `native_solid.rs::a_solid_the_native_modeller_cannot_edit_is_refused_for_every_edit` (the spline `wavy-prism` fixture; in the feature-off build the problem is also checked unchanged), and the editor status in `guidance_tests.rs`.

Face checks shared with OpenCASCADE ("face7 is curved: pick a flat face", "the solid has no face99") are unchanged.

Stale documentation fixed: `app-tui/AGENTS.md` said STEP open / hole / boss / fillet need the `occt` feature and are disabled without it; `fea-problem/AGENTS.md` said `tessellate_step` needs it. Both now describe the native default.

## Commands and results

macOS arm64, release.

```
cargo test -p fea-problem -p solid-kernel --release --locked --offline -j1
OCCT_ROOT=<prebuilt occt-8_0_1_rev3> cargo test -p fea-problem --features occt --release --locked --offline --test solid --test native_solid --test fillet_rim_tagging
OCCT_ROOT=<prebuilt occt-8_0_1_rev3> cargo test -p solid-kernel --features occt --release --locked --offline --lib cad
OCCT_ROOT=<prebuilt occt-8_0_1_rev3> cargo test -p solid-kernel --features occt --release --locked --offline --test occt --test native_brep_occt --test native_brep_boolean --test native_brep_fillet
cargo test -p app-tui --release --locked --offline -j1 --lib a_step_solid_opens_gets_a_hole
```

- Feature off, `fea-problem` and `solid-kernel`: 526 passed, 0 failed, 10 ignored (serial Cargo, no warnings).
- Feature on, `fea-problem` `solid` / `native_solid` / `fillet_rim_tagging`: 12 + 6 + 6 passed, 1 ignored (a diagnostic). The three new `native_solid` tests also pass here, where the cache comes from OpenCASCADE.
- Feature on, `solid-kernel` lib `cad`: 3 passed; `occt` / `native_brep_occt` / `native_brep_boolean` / `native_brep_fillet`: 5 + 3 + 8 + 4 passed.
- `app-tui`, feature off: `a_step_solid_opens_gets_a_hole_and_a_dangling_support_is_diagnosed` passed (the native fillet refusal reaches the status bar and the model is unchanged).

## What this shows and does not show

- It shows which editor operations a build without OpenCASCADE performs, on which shapes, and that everything else is refused with a reason that names the right face and says the limit is the native build.
- It does not make native editing complete. For retirement the native editor still lacks: holes and bosses that meet curved faces or cross other features, fillets of anything but a convex polyhedron, and editing any solid with spline, ellipse or complex edge curves (most real CAD parts). Native STEP import (Phase 165) is the larger gap. These stay gaps until proved.
- It does not cover Windows. The OpenCASCADE application-level window test (`window.rs`) was not run in this phase.
