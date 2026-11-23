# Epic: assembly feature recognition and joint advice

Status: planned 2026-11-19. Nothing built. Phase A is the start; later phases are gated on its evidence.

## Goal

Given two or more bodies (e.g. two angles, one on the other), recognise what the geometry is -
cylindrical holes, planar faces that overlap, coaxial holes across bodies - and *recommend*
supports, contact and fastener models with a stated reason, the way `fea_problem::advice::recommend`
already does for sketch problems. Advice first: nothing is applied without the user choosing it.

## Verified starting point (2026-11-19)

| Piece | Where | Gap |
|---|---|---|
| Per-face analytic surfaces (`Surface::Plane`/`Cylinder{frame, radius}`/`Cone`/...) | `solid-kernel/src/analytic_step.rs` (`AnalyticModel::surfaces`, private) | no public face-classification API; keyed by STEP face entity id |
| STEP assemblies: every instance read with its rigid placement | `solid-kernel/src/step_assembly.rs`, `step_body::BodyContext::read_all` | `fea-problem` does not call it; `Geometry::Solid` is one solid |
| Face identity across edits | `solid-kernel/src/face_identity.rs` | - |
| Named-surface measurements (area, normal, centroid) | `fea-problem/src/facts.rs` | no axis/radius, no body index |
| Explained advice (Advice, `reason`, `apply`) | `fea-problem/src/advice.rs` | sketch problems only, name/centroid based |
| 2D contact pairs | `Problem::contacts` | refused for 3D (`validate_contacts`) |
| Ties / rigid couplings / contact in kernel | `fea-core/src/mpc.rs`, contact modules | not exposed by the problem layer for solids |
| Bolt stiffness / preload | `fastened-joint-solver`, `fea-problem/src/joint.rs` | axisymmetric member stack only |

Unverified, to settle in Phase A step 1: whether a multi-body STEP can be held by `Problem` today
(expected: no; a body index on faces and a `Vec` of bodies is needed), and whether face ids from
`read_all` are stable per instance.

## Design decisions

1. **Detection is a pure function in `solid-kernel`** (no UI, no solver): classified faces in, candidate
   features and pairs out. Geometry only, in the root frame, in mm, deterministic order.
2. **Advice stays in `fea-problem`** next to `advice::recommend`, consuming detection output. Each item
   carries a reason string with the measured numbers (offset, axis angle, overlap area).
3. **Every candidate carries a confidence class**: `Exact` (analytic match within tolerance),
   `Inferred` (heuristic: pattern, layout), never a silent guess. Inferred items are labelled as such and
   are Phase D, gated on measurement.
4. **Tolerances are explicit constants tied to the model's `geometry_tolerance_mm`**, not tuned to pass
   fixtures. No loosening to make a corpus file pass.
5. **No context cost**: advice is rendered in the existing UI docks; nothing new goes into model context.

## Phases

### Phase A - detection (the start of the epic)

Deliverable: `solid-kernel::features` (name provisional), advice-free, tested.

1. **Spike (read-only).** Confirm the two unverified points above; list which NIST/corpus files are
   multi-instance and which have coaxial holes. Output: a short evidence note appended to this doc.
2. **Face classes.** Public, read-only accessor over the analytic model: per face, body index, kind
   (Plane/Cylinder/Cone/Other), plane (normal, offset) or cylinder (axis point, axis dir, radius), outward
   side, area, bounding box. Cylinders report `Hole` (concave, material outside) vs `Boss` (convex) from
   the face orientation, not from the name.
3. **Holes.** Group a cylinder face (or its split halves: STEP often splits a hole into two faces) into a
   hole feature: axis, diameter, depth, through vs blind (does the hole exit on a face on the far side),
   countersink/counterbore (cone/step coaxial with it).
4. **Coaxial hole sets.** Holes on different bodies whose axes agree within tolerance (axis angle and
   perpendicular offset) and whose bores overlap along the axis -> a `FastenerSite { holes, nominal_diameter,
   clearance_class }`. Diameter checked against a standard bolt/clearance table (inch and metric); a
   mismatch between the two bores is reported, not hidden (e.g. 0.201 in vs 0.250 in is a clearance hole
   in one part and a tapped/press hole in the other).
5. **Contact pairs.** Planar faces on different bodies, anti-parallel normals within an angle tolerance,
   gap below a tolerance, with positive projected overlap area -> `InterfaceCandidate { faces, gap,
   overlap_area, overlap_polygon }`. Overlap via 2D polygon clipping in the shared plane.
6. **Tests.** Closed-form fixtures built with `native_brep` (box, `cylinder_z`, drill ops in
   `native_brep_drill.rs`): two plates with 1, 2 and 4 coaxial holes; offset-by-tolerance and
   offset-past-tolerance negatives; plates touching, plates with a gap, plates overlapping partially;
   rotated and translated placement through `step_assembly`; split-face holes. Plus a run over the
   existing corpus with counts recorded in the evidence note.
7. **Docs.** `solid-kernel/AGENTS.md` node entry; `docs/AGENTS.md` index row.

Exit gate: all fixtures pass; the corpus run is recorded with no unexplained detections; false-positive and
missed-detection counts against hand-labelled files are reported, not just a pass.

### Phase B - advice over detection (sketch of scope; confirm after A)

- Multi-body `Problem` representation (bodies + face ownership) if the spike says it is missing.
- `advice` items: "tie/bond these two faces", "treat holes N and M as one fastener (bolt dia D)",
  "support or load on the bolt hole"; reason strings with the measured numbers.
- Window surface: list candidates, highlight the faces on selection, apply on click (native-window rule;
  needs an approved mockup per `app-tui/AGENTS.md`).

### Phase C - solver exposure (needs product decisions)

- 3D contact pairs on `Problem` (kernel capability exists; `validate_contacts` refuses it today).
- Fastener models: rigid link / beam-to-hole-wall coupling (`mpc.rs`), optional preload from
  `fastened-joint-solver`. Bonded (tie) interface as the cheap first option.
- Cost: 3D contact solves are minutes; keep them manual studies like the 3D bushing.

### Phase D - inference without holes (heuristic, gated)

- Fit fastener layout from flange overlap and symmetry when no holes exist. Ships only if measured on a
  labelled corpus to beat doing nothing; otherwise record the negative result and stop.

## Risks

- Hole faces split by the STEP writer, or modelled as B-spline: classification must fall back to
  `Other` loudly, not mis-classify.
- Assembly placement direction ambiguity (documented in `step_assembly.rs`): detection must run on the
  same frame the mesher uses, or pairs will be wrong.
- Multi-body in `Problem` may touch the frozen face-numbering rules (`remap_faces`, `removed:faceN`);
  do not persist kernel ids.
- Angles (L-sections) have fillets and tapered legs: planar overlap must tolerate fillet-band edges.

## Not in scope

Changing the mesher, touching `fea-core` solvers, applying any advice automatically.

## Phase A step 1 evidence (spike, 2026-11-19)

Probe: `solid-kernel/examples/assembly-probe.rs` (`cargo run --release -p solid-kernel --example assembly-probe -- FILES`), read order as production (`fea_problem::native_solid`): strict, then `Options::foreign_cad(0.05)`. Corpus: the 33 NIST PMI files (not committed); per-file rows in `.audit/assembly-probe-nist.tsv`.

1. **`Problem` cannot hold a multi-body STEP today (observed in source).** `fea-problem/src/native_solid.rs` reads with `AnalyticModel::read`, which refuses more than one body (`tests/step_placement.rs`: "exactly one supported body"); `Geometry::Solid` is one STEP text plus one cache. Nothing in `fea-problem` or `app-tui` calls `read_all`; its only callers are `solid-kernel` examples and tests. Phase B needs a body list on the problem.
2. **Face ids repeat across instances.** `read_all` returns one `AnalyticModel` per placed instance, in assembly order. The fixture `assembly-two-instances.step` is one solid placed twice, so both models carry the same STEP face entity ids. A face key must be `(body index, face id)`, never the id alone. Within one file the body order is deterministic (the product-structure order).
3. **The corpus has no assemblies.** All 32 readable NIST files give `bodies=1` (the 33rd, `nist_ftc_08_asme1_ap242-e1-tg.stp`, is a tessellated solid, not a B-rep). So the corpus yields zero coaxial pairs and zero contact pairs by construction; it can only test hole classification and false-positive rates on single parts. Coaxial and contact detection must be proven on synthetic fixtures (`native_brep` plates, the `step_placement` fixtures); real multi-part CAD for the exit gate is still needed and is not in the repository.
4. **Bug found and fixed on the way: `face_signatures` failed whole models.** A cone face whose first boundary sample is the apex (a drill point, a countersink tip) has no normal there, and `face_signatures` returned `Err("no normal")` for the whole model: 9 of 33 NIST files (`ctc_01/02/04/05`, `ftc_06/07/10` variants). Fix: `probe_point` takes the first boundary sample that has a normal (`face_identity.rs`); test `a_cone_probe_skips_the_apex_and_a_plane_takes_the_first_sample`. After: 32 of 33 sign; the rest is the tessellated file.
5. **Hole candidates (concave cylinders) on single NIST parts:** 0 to 165 per file (for example `ctc_03_ap242-e2`: 45 on 58 cylinders). Not yet grouped into holes; counts above are faces, not features.

## Phase A results (2026-11-19)

Built: `solid-kernel/src/features.rs` (`classify_faces`, `find_holes`, `find_sites`, `find_interfaces`, `detect`), `tests/features.rs` (14 closed-form fixtures), `examples/assembly-probe.rs` (corpus run). `cargo test --release -p solid-kernel`: all pass.

Decisions and findings:
- **A hole wraps the full circle.** Concave cylinders are mostly not holes: an inside pocket-corner fillet is a concave quarter-turn cylinder, and a slot end is a half turn. On the NIST corpus 2946 concave cylinder faces give 1052 holes. The wrap is the union of the angular intervals covered by consecutive boundary samples (`arcs_of`), not the arcs at one axial position: a bore with an oblique rim (elliptical edge) would be missed by the latter. A bore split into half faces is one hole because the union of the halves covers 2 pi.
- **Evidence for the rule:** the export variants of one part now agree on the hole count (ctc_01 ap203 / ap242-e1 / rd: 10 / 10 / 10; ctc_02: 94 / 94 / 94; ctc_03: 15 / 15 / 15; ctc_04: 60 / 60 / 60), where face counting gave 45 / 50 / 45 for ctc_01.
- **Invariants held:** every concave cylinder face of a full bore lands in exactly one hole; area of a planar face is of the sampled polygon (a curved boundary reads inside by at most perimeter x chord tolerance; a rectangle is exact); a contact needs `|gap| <= geometry_tolerance_mm` and common area above `tol^2`.
- **Size table.** `size_matches` carries ISO metric (M3 to M24: tap drill, nominal, ISO 273 close / normal / loose) and unified inch (#4 to 1/2: tap, nominal, close, free). The inch values are transcribed from the usual Machinery's Handbook tables and **need checking against the current handbook before release**. Match window `TABLE_MATCH_MM` = 0.05 mm (half the coarsest printed step).
- **Verified by test:** 5.0 mm over 6.6 mm is an M6 joint (tap drill over normal clearance), 0.201 in over 0.250 in is 1/4-20 (tap over nominal), 5.0 over 7.8 mm has no common size and is reported as a mismatch.

Limits (not hidden):
- **No assembly in the corpus.** The 32 readable NIST files are single parts, so sites and interfaces are proven on `NativeBrep` fixtures and one STEP assembly fixture only; the exit gate's hand-labelled false-positive and missed-detection counts against real multi-part CAD cannot be produced here. Needs a licensed multi-part STEP set.
- Slots (obround) are not holes and are not recognised as slots; tilted (oblique-axis) assemblies are tested through x-axis fixtures and the rotated `assembly-two-instances` placement, not an oblique-axis fixture.
- A site needs bores whose axial ranges touch or overlap within tolerance: a washer, shim or air gap between plates breaks it. Interfaces likewise need contact within tolerance.
- Cone and sphere bottoms other than a drill point, and cones that are not concave, are not described; B-spline faces are `Other`.
- Circles smaller than the sampling chord (bores under about 0.1 mm) can have two-sample edges whose angle step is ambiguous.
- The model's polygon boundary must be simple for the overlap area; a face that is not is listed in `Features::unresolved`, never dropped.

Not done: Phases B to D; the multi-body `Problem` representation; a native window.

## Phase B step 1: read-only advice (2026-11-19)

Built: `fea-problem/src/assembly_advice.rs` (`recommend(&Features)`, `recommend_step(step)`), `fea-problem/tests/assembly_advice.rs` (3 tests, pass). Items: `Tie` (an interface: faces, gap, shared area), `Fastener` (coaxial bores: bodies, bore faces, smallest common standard size, diameter mismatch flag) and `Unresolved` (interface pairs whose common area could not be computed, so they are never dropped). Each carries a reason with the measured numbers; faces are `FaceKey { body, face }`, body numbers in the text are 1-based. Fixture check: a 5.0 mm bore under a 6.6 mm bore, stacked 40 x 30 mm plates, gives one `Tie` (no gap, 1166 mm²) and one `Fastener` (M6, mismatch flagged, 20 mm of axis).

Decision: the items are read-only and no `Problem` is edited. A `Problem` holds one body (`Geometry::Solid` is one STEP text), and there is no tie or fastener on it to apply until the multi-body representation and the solver exposure of Phase C exist. Building the multi-body `Problem` first would change `Geometry::Solid` consumers across `solid.rs`, `solve.rs`, `facts.rs`, `units.rs` and the face-numbering rules (`remap_faces`, `removed:faceN`) without any solver using it; that is held for Phase C's product decisions.

Not done: "support or load on the bolt hole" advice (needs a body to carry the boundary); the multi-body `Problem`; the native window (needs an approved mockup; none exists yet, so none was built); the inch fastener table check; the exit-gate corpus.

## Phase B step 2: multi-body viewer mesh (2026-11-19)

The Assembly window mockup was approved (solid window, new Assembly step between Model and Mesh; feature tree with Bodies / Detected groups; advice inspector card with Fastener sites and Interfaces groups; a Why card showing the item's reason; Exploded toggle; selecting an item highlights its faces in the accent colour; no Apply button while the advice is read-only). The viewport needs more than one body, so this step builds the mesh behind it.

Built: `fea-problem/src/assembly_mesh.rs`: `assembly_mesh(step, chord) -> AssemblyMesh` (vertices, triangles, per-triangle `FaceKey { body, STEP face id }`, triangles per body, chord) and `AssemblyMesh::triangles_of(&[FaceKey])` for highlighting. It reads with `AnalyticModel::read_all`, so bodies, order and root frame are those `features::detect` uses; `with_bodies` (strict read, then the tolerant foreign-CAD read with chart canonicalization, as `native_solid::sample` does for one body) is shared with `assembly_advice::recommend_step`. It is not a `Problem` and nothing is persisted: the face ids are valid for that file only. Tests (`tests/assembly_mesh.rs`, 3 pass): `assembly-two-instances.step` gives 2 bodies, equal positive signed volumes (a rigid placement keeps the volume), different bounding boxes, the same 6 face ids in each body kept apart by the body index; a box gives 12000 mm³; bad text is an error.

Limits: the only multi-body fixture is one box placed twice, so a stacked bolted pair is covered by the advice tests (detection) and the mesh tests (placement) separately, not by one file; no polish of the exploded offset (a window-side transform, not a mesh property) yet. Not done: the window itself (tree, card, Exploded toggle, highlight drawing) in `app-tui`.

## Phase B step 3: the Assembly Joints window (2026-11-19)

Built: `app-tui/src/gpu_viewer/editor/assembly/` (`mod.rs` session and `Authoring` impl, `scene.rs` the multi-body picture, `cards.rs` tree / inspector / ribbon / menus, `tests.rs`), `app-tui --gpu-assembly [file.step]` (`EditorKind::Assembly`), dashboard tile "Assembly Joints" (12 tiles). Layout as the approved mockup: feature tree left (Bodies, Detected: Holes / Fastener sites / Interfaces, Advice), inspector right with the "Assembly advice" card (groups Fastener sites, Interfaces, and Check only when a pair is unresolved; each row is `AssemblyAdvice::label()` with the measured `detail()` dimmed under it), a "Why" card (the item's `reason()`, one sentence per line) and a Summary card, status bar "N bodies, n fastener site(s), n interface(s), detected in X s". Selecting an advice item or a body (tree, inspector row, Up / Down walk the list, Esc clears) lights its faces in the selected colour; Exploded (key E, default on, ribbon and View menu) moves each body away from the assembly's centre by 0.8 x its centre's offset. Read-only: no Apply, nothing written, no `Problem`.

Deviation from the mockup, stated plainly: the mockup put an "Assembly" step between Model and Mesh inside the solid window. The solid window edits one `Problem`, and a multi-body view cannot live in it without making `Problem` multi-body (held for Phase C), so this is its own window like the Fastener Hole window, with one ribbon step "Assembly". Opening a file is File > Open (Ctrl/Cmd+O) or the argument; with none the window starts empty with an "Open STEP assembly" option. A file that cannot be read or is not STEP leaves the window empty with the reason in the status bar.

Also: `AssemblyAdvice::Fastener` gained `diameters` and `span`, `AssemblyAdvice::detail()`, and `assembly_advice::detect_step`. New fixture `solid-kernel/tests/data/assembly-stacked-plates.step` (two instances of `edit-plate.step`, stacked, coaxial 8 mm holes: one Tie and one Fastener). Shared fix found on the way: `gui/panels.rs` drew a `DockItem::Line` with an action and `focused = true` without any selection fill (`frame_when_inactive(false)` overrode what `Button::selectable` sets), so no focused clickable row ever showed as selected; it now does (the screenshot of the Assembly window caught it).

Verified: `cargo test --release -p app-tui --lib` 1294 pass (12 new `assembly` unit tests: load counts, inspector rows and Why card, tree selection, key walk and redraw, selected-face colour code, exploded separation and the single-body zero offset, empty / unreadable file, ribbon and menus); the ignored `render.rs::assembly_snapshot` ran on this Mac's GPU (`FEA_SNAPSHOT_ASSEMBLY_OUT`) and its dark and light images were read back: both bodies visible, the selected bore / interface face red, nothing clipped, selected rows highlighted in tree and inspector.

Limits: (1) a Tie whose faces are internal (bodies joined, not exploded) shows nothing red in a shaded view; explode or use Wireframe to see it. (2) No picking of a face on the model; selection is by tree, list or keys. (3) The model is the viewer mesh (0.1 mm chord class), not an analysis mesh. (4) The window's right status text still reads "Saved | result 1/1" (generic chrome of the shared window). (5) Only Windows hardware can show the real-app run on the target platform; macOS here. Not done: Phases C and D, the multi-body `Problem`, support / load advice on a bolt hole.

## Phase B follow-ups: one read, holes without a mate (2026-11-20)

Built: `fea_problem::assembly_mesh::read_assembly(step, chord) -> AssemblyRead { features, mesh }` reads the bodies once (`with_bodies`, strict then tolerant) and both detects and samples them; the Assembly window's `load()` uses it, so the STEP is parsed and read once, not twice, and the features and the mesh always come from the same read. `detect_step` and `assembly_mesh` are unchanged. New advice `AssemblyAdvice::Unmated { body, faces, diameter, closure, size, reason }`: a hole of a body in a file with at least two bodies that belongs to no fastener site, whose diameter matches a standard size (`solid_kernel::features::size_matches`, so an arbitrary diameter is not listed), and that is not an enclosed cavity. A counterbore or other section of a stepped bore is not reported when another section is in a site (same body, parallel axis, touching ranges, within `DEFAULT_CHORD / 10`, the strict read's geometry tolerance; `Features` does not carry the models' own). `size` names every size the diameter matches ("M5 or M6 or #10-24" for 5.0 mm). The window shows it as the third inspector group "Holes without a mate", a Detected tree node, a Summary field and a status-bar count.

Measured (`cargo run --release -p fea-problem --example assembly-load-bench -- solid-kernel/tests/data/assembly-stacked-plates.step 301`, two reads and one read interleaved so load hits both; the Mac was shared with other builds, so the medians are noisy and the minimum is the cleaner figure): minimum 6.2 ms to 4.55 ms (0.72 of the old time), median ratio 0.54 to 0.63. An earlier one-off baseline of 31.8 ms was taken while a release build ran and is not comparable.

Not done: a face on the model cannot be picked; a counterbore section on a differently placed axis than the bore (beyond the tolerance) is reported as unmated; the inch size table still needs a handbook check. Finding for `solid-kernel` (not changed here): `NativeBrep::face_holds` is false at the centre of a drilled pocket's floor when the pocket radius is 4 mm or more (3.5 mm is true), so a counterbore of radius 4 mm or more cannot be built with `drill` from the floor.

**`face_holds` fixed (2026-11-20).** The finding above is resolved in `solid-kernel/src/native_brep_drill.rs`: `polyline` sampled a full circle's last point as `cos / sin` of 2 pi, which is off the start vertex by rounding, so the one-edge loop of a pocket floor was not closed and a ray through that vertex (the floor's axis) missed the closing crossing for radii of about 3.7 mm and up. The ends of every sampled edge are now its exact vertices. Regression tests: `the_centre_of_a_blind_hole_floor_is_inside_it_at_every_radius` and `a_bore_can_be_drilled_in_the_floor_of_a_wide_counterbore` (`solid-kernel/tests/native_brep_boolean.rs`, a 12 mm counterbore with a bore in its floor against the closed-form volume); the counterbore test in `fea-problem/tests/assembly_advice.rs` now uses an 8 mm counterbore.

## Phase A exit gate: synthetic hand-labelled corpus (2026-11-20)

**This is synthetic, not real CAD.** Every part is a `NativeBrep` (boxes, polygon / arc outlines extruded, drilled holes) written by this repository's own STEP writer and read back by its own reader. The counts below show that the detector agrees with labels I wrote for geometry I built; they say nothing about the false-positive or missed-detection rate on real multi-part CAD from other exporters, which is still unmeasured (no such set is in the repository).

Built: `solid-kernel/tests/features_corpus.rs`. `compose` writes several parts into one STEP assembly (one product per part, `NEXT_ASSEMBLY_USAGE_OCCURRENCE` + `ITEM_DEFINED_TRANSFORMATION` per instance, as `assembly-two-instances.step` does), so `AnalyticModel::read_all` places the bodies as it does for a file. Each body's placed vertices are checked against the read body, so a body-order or placement fault cannot pass as a detection. 39 cases, each run under 4 global placements of the whole assembly: axis-aligned, oblique A (axis (1, 2, 3), 37 deg), oblique B (axis (-2, 1, 0.5), 112 deg), oblique C (hole axes taken to (0.95, 0.2, 0.24), x-dominant, which is where `stable_basis` switches its reference direction, plus 53 deg of spin); 156 case runs. Some cases also place a part inside the assembly (a plate turned 180 deg about z, one flipped over 180 deg about x so its blind hole opens on the joint face). Ground truth is data next to each case: bore sections (body, axis point, radius, through / blind), fastener sites (position, bodies, fastener size, mismatch flag), contact interfaces (body pair, common area to the closed form, gap). Detections are matched to labels by position and radius, not by count. A label marked `limit` is a documented limit: expected missed, and the test fails if it is ever found, so this section is updated.

Classes (cases): stack of 2 and 3 plates with 1 / 2 / 4 coaxial holes (6); L-section on a plate, 1 and 2 holes (2); hole offset 0.5 tol, 2 tol, 0.2 mm (3); touching, gap 0.5 tol, overlap 0.5 tol, gap 2 tol, 0.5 mm air gap blank and holed, side-by-side face contact, edge contact, edge contact with a 0.5 tol strip, stacked blank (10); shim with and without a hole (2); counterbore over a through hole, blind hole not reaching the joint, tapped blind hole from the joint face (3); partial overlap with one hole pair inside the overlap (1); tapped / clearance pairs 5.0 over 6.6, 6.6 over 5.0, 6.6 / 6.6, 5.0 over 7.8, 0.201 in over 0.250 in, three plates 6.6 / 6.6 / 5.0 (6); turned and flipped plates (2); slotted plate, rounded-corner window, slot end over a round hole, slot plus round hole (4). The slot and the window are extruded outlines with arcs (a local builder; `NativeBrep::extrude` has only polygons and full circles); the window is through, not a pocket with a floor: its four quarter-turn concave cylinders are the same faces a filleted pocket corner gives.

Result, per placement (identical in all four placements, per class: true positives / false positives / missed; `lim` = documented-limit labels missed as documented):

| Class | Holes | Sites | Interfaces |
|---|---|---|---|
| stack of plates | 35 / 0 / 0 | 14 / 0 / 0 | 9 / 0 / 0 |
| L-section on plate | 6 / 0 / 0 | 3 / 0 / 0 | 2 / 0 / 0 |
| offset vs tolerance | 6 / 0 / 0 | 1 / 0 / 0, 1 lim | 3 / 0 / 0 |
| touching vs gapped | 10 / 0 / 0 | 3 / 0 / 0, 1 lim | 5 / 0 / 0 |
| shim and air gap | 5 / 0 / 0 | 1 / 0 / 0 | 4 / 0 / 0 |
| counterbore and blind | 7 / 0 / 0 | 2 / 0 / 0 | 3 / 0 / 0 |
| partial overlap | 4 / 0 / 0 | 1 / 0 / 0 | 1 / 0 / 0 |
| tapped vs clearance | 13 / 0 / 0 | 6 / 0 / 0 | 7 / 0 / 0 |
| placed instances (turned, flipped) | 6 / 0 / 0 | 3 / 0 / 0 | 2 / 0 / 0 |
| slot and corner fillets | 3 / 0 / 0 | 1 / 0 / 0, 1 lim | 4 / 0 / 0 |
| **Total per placement** | **95 / 0 / 0** | **35 / 0 / 0, 3 lim** | **40 / 0 / 0** |

Over the 156 runs: 380 holes, 140 sites, 160 interfaces true positive; 0 false positives; 0 missed outside the documented limits; 12 documented-limit misses (3 labels x 4 placements); 0 mislabelled sites (every site's fastener size and mismatch flag matched, including M6 tap over clearance, 1/4-20, no common size, and the counterbore over a through hole reading as M3 without mismatch because the wide section is not coaxial-and-touching across bodies). Each slot / window case also asserts the expected number of concave cylinder faces exist (2 and 4), so "not holes" is shown on faces that are really there.

Documented limits (labels the detector misses by design, truth in the physical sense):
1. A 0.5 mm air gap between plates with coaxial holes: a bolt spans it, the detector sees no site (a site needs axial ranges that touch within tolerance). The shim with a coaxial hole is found (3 bodies); a solid shim correctly gives no site.
2. Holes offset by 0.2 mm: a real misalignment a clearance hole absorbs, beyond the model's geometry tolerance (1e-7 mm here), so no site. Offsets of 0.5 tol are sites and 2 tol are not (a convention test, labelled by the tolerance, not physics).
3. A bolt through a slot end over a round hole: a half turn is not a bore, so slots are not fastener locations (already noted under Phase A limits).

**Bug found and fixed.** A zero-area edge contact of two rotated bodies was reported as an interface. `find_interfaces` accepted any common area above `tol * tol` (1e-14 mm2 for the default tolerance), which is under the rounding noise of a placed body: two boxes touching along an edge, placed by the x-dominant oblique pose, read a common area of 5.7e-14 mm2 and produced an `Interface` (axis-aligned and the other two poses happened to read exactly 0). The rule is now common area above `tol * length`, `length` the longer face diagonal: a strip no wider than the tolerance along a shared edge is within tolerance and not a contact. This is not a loosened tolerance in the sense of the epic's decision 4 (the tolerance is the same; the area it implies is). Regression: `edge contact only` under oblique C and the new `edge contact, strip 0.5 tol wide` case (an overlap strip of 5e-7 mm2 that the old rule accepted in all four placements); both fail with the old rule and pass now. Everything else already passing (tests/features.rs, the app-tui Assembly tests are not touched by this and were not re-run) is unchanged. With the foreign-CAD tolerance (5e-3 mm) the same rule drops patches below about tol x diagonal (0.5 mm2 on a 100 mm face); real contact patches are larger.

Not verified / not done: real multi-part CAD (the exit gate's real-data half remains open); anything under `Options::foreign_cad` (this corpus reads at the default tolerance 1e-7 mm, so the chord / healing path of real files is not exercised); a pocket with a floor (the window is through); fillets on L-section inside corners (`NativeBrep` cannot build a concave fillet except by the arc outline builder, and the L has sharp inside corners); non-planar mating faces; a bolt hole in a leg of the L (only the foot is drilled); the inch size table against the handbook (unchanged from Phase A).

## Exit gate, real data: vetted public models (2026-11-20)

Sources were vetted for provenance and licence before anything was downloaded; each download was approved by the owner, checked against the published size (and MD5 where the host publishes one), listed before extraction and kept outside the repository (models are not committed; only counts and attribution are recorded here).

| Source | Size | Licence / terms | Used |
|---|---|---|---|
| Zenodo 14869539, "Test models and test results to evaluate CAD assembly modules capabilities to generate component interfaces" (Leon, Boussuge, Giannini, Monti, Lupinetti, Bonino, Pernot, Raffaeli; DOI 10.5281/zenodo.14869539; supplement to Computer-Aided Design 2025, DOI 10.1016/j.cad.2025.103911) | 5 files, 8 MB; read the PDF (5.5 MB, MD5 matched) and the surface zip (44,059 B, MD5 `ad6002394b9b5ed268640c6658f7b1e0` matched) | CC BY 4.0: attribution required | Yes: the 5 surface-interface files |
| NIST "Bracket Assembly" = CAx-IF AS1, `as1-oc-214.stp` (STEP AP214, Open CASCADE / Datakit 2008), `https://www.mbx-if.org/library/as1-oc-214.zip` | 82,774 B zip (MD5 `195d7fa1df276bc1e5adf07e96237358`, no published checksum), 441,968 B file | NIST download page: "can be used without any restrictions", acknowledgement appreciated; terms on the mbx-if.org host not read | Yes |
| NIST MTC box assembly (`nist-cad-models-mtc-assembly`) | 15.9 MB | NIST: unrestricted | Downloaded, held only NX, SolidWorks, JT and SAT (no STEP), deleted unread |
| Fusion 360 Gallery Assembly Dataset (8,251 assemblies, STEP plus labelled holes and contacts) | 18.8 GB in 11 archives of 1.1 to 2.3 GB | "non-commercial research purposes" only, binds a for-profit employer, no whole-dataset redistribution, Autodesk may withdraw | Not downloaded: the licence is the owner's decision; its holes are labelled by Autodesk's feature recogniser, not by hand |
| AutoMate (Zenodo 7776208, CC0) | 34.9 GB | CC0 | Not downloaded: no hole or bolt labels, 13.2 GB of per-part STEP |

**Zenodo surface set (5 files, 2 bodies each).** The set tests CAD systems' contact-constraint solvers: its components sit at arbitrary relative locations until the user constrains them, so no contact exists in the files as delivered. Result: 0 interfaces, 0 sites, 0 unresolved in all five (cone, sphere, torus, plane, cylinder pairs): no false positive. Cone and sphere files hold no concave cylinder. The cylinder pair (convex R40, concave R40) gives 1 blind hole (the concave cup), the torus pair 2 through holes (4 concave cylinder faces); neither hole count was inspected against the part geometry and the PDF gives none. This set cannot show true positives and the PDF holds no hole ground truth.

**AS1 (the canonical bolted assembly).** First result: `read_all` refused it, `usage #1137: cannot tell which placement belongs to the component and which to the assembly`. Cause: Open CASCADE writes the part's origin placement `#11` as an item of both the component's and the assembly's representation in every usage (`ITEM_DEFINED_TRANSFORMATION(#11, #15)`, `#11` in both, `#15` only in the assembly's), and the rule of `step_assembly.rs` rejected any placement held by both even when only one assignment fits. **Bug fixed**: the pair is now decided when exactly one assignment has its source in the component and its target in the assembly (which is also what the standard's `rep_1`/`rep_2` order says here); both or neither still refuse. Regression tests: `tests/step_placement.rs` (`a_placement_held_by_both_representations_is_decided_when_only_one_way_fits`; the old "held by both" refusal now uses a case where both placements are in both representations, plus a neither case).

After the fix AS1 reads as 18 bodies (5 distinct solids, 13 usages) and gives 0 holes, 0 sites, 16 interfaces, 0 unresolved in 0.08 s. The interfaces were checked by hand against the geometry: the plate (180 x 150 x 20) has the two L-brackets on its top face (2 interfaces, 4765.6 mm2 each), a nut under it at each of 6 bolt positions (6, 222 mm2), 6 bolt heads against the bracket undersides (6, 97.7 mm2) and 2 rod-end nuts against the brackets' sides (2, 221.9 mm2): 2 + 6 + 6 + 2 = 16, every gap 0. Hand count equals detected count: no false positive and no miss among interfaces on this file.

**Correction (2026-11-20, same day): 0 holes and 0 sites on AS1 is a miss, not a correct result.** An earlier reading here took the file to be all planar because it holds no `CYLINDRICAL_SURFACE` or `CIRCLE` entity. It does hold round geometry: 28 of its 53 `ADVANCED_FACE`s are rational B-spline surfaces (`B_SPLINE_SURFACE(1,3,...)` with weights 1, 1/3, 1/3, 1), and all 28 are exact half-cylinders (control points checked: 26 of radius 5 mm, 2 of radius 7.5 mm, each a semicircle extruded along its axis). The exporter writes every cylinder as two rational B-spline halves. `features::classify_faces` classifies only analytic surfaces (`Surface::Plane`, `Cylinder`, `Cone`), so a B-spline face is `FaceGeometry::Other` and no bore or fastener site can form from it. Which of the 28 faces are concave (bores) and which convex (bolt shanks, rods, nut outsides) was not determined. Recognising a rational quadratic or cubic Bezier B-spline as an exact cylinder (weights, control polygon, tolerance) is a feature with its own numerical proof and a false-hole risk, so it is recorded here as a limit, not done.

Limits: AS1 proves interfaces, placement reading and nested assemblies on real CAD, not hole or site detection (its round faces are B-splines, see the correction above); no real, hand-labelled bolted model with round holes has been obtained (the Fusion 360 Gallery set is the one that would, under its licence). 16 of 16 is one file, not a rate.

## B-spline cylinders scored on real CAD (2026-11-20)

Built (branch `feat/assembly-bspline-cylinders`): `SplineSurface::fit_cylinder(tol)` (`solid-kernel/src/curved_surface.rs`) takes a rational or plain B-spline surface that is a right circular cylinder to within the model's geometry tolerance (a 17 x 17 parameter grid plus the knots; axis from the two most different normals; least-squares circle; refused when the normals are nearly parallel, the radius or axis drifts, or any sample is off the circle). It is sampled evidence, not a proof. `classify_faces` uses it for `Surface::Param`; the side comes from the surface normal at a boundary sample times the face sense. Tests: two unit tests (AS1's cubic half; a quarter turn, a plane, an ellipse and a bulge are refused), `a_bore_written_as_a_rational_spline_cylinder_is_the_same_hole_as_the_analytic_one` and `a_spline_cylinder_whose_face_sense_is_reversed_is_a_convex_shank_and_never_a_hole` (`tests/features.rs`). `assembly-probe` gained `PROBE_VERBOSE` (cylinder faces by side and radius, holes, interfaces, sites) and `PROBE_FACES` (one line per cylinder face with wrap, range, axis).

**NIST AS1 (`as1-oc-214.stp`, 441,968 B, MD5 of the zip `195d7fa1...` matched).** Before: 0 holes, 0 sites (the correction above). After: 18 bodies, 160 faces, 70 cylinder faces (44 concave r = 5.000 mm; 14 convex r = 5.000 mm; 12 convex r = 7.500 mm), 22 holes, all through, all 10 mm diameter (8 bodies with one bore 3 mm long, two bodies with four bores 10 mm long, one body with six bores 20 mm long), 8 fastener sites (all M8, no diameter mismatch), 16 interfaces, 0 unresolved. The 16 interfaces are the expected 16. So the sides came out right (a convex shank read as concave would have made false holes and moved the interface set): the 44 concave faces are 22 split bores, the 26 convex faces are the rods, bolt shanks and nut or head outsides. The Zenodo surface set is unchanged (0 interfaces, 0 sites; its parts are unconstrained, see above).

**Fusion 360 Gallery assembly dataset, archive `a1.0.0_04.7z` (1,202,111,575 B, MD5 `07f253af...`; used for non-profit research, licence is non-commercial research).** Only `.step` and `.json` were extracted; nothing is committed. 498 of the archive's 753 assemblies were run before the job was stopped (the run is slow on large files); aggregate counts only. The JSON `holes` come from Autodesk's recogniser, not a hand label, and units are cm (x 10 for mm).

| | count |
|--|--|
| body files read / not read | 5969 / 1248 |
| `assembly.step` read / not read (of 498) | 273 / 220 (5 more gave no assembly result, one of them a 900 s timeout) |
| Autodesk holes in the readable bodies | 4133 |
| matched by diameter (0.02 mm), per body | 2722 |
| not matched | 1411 |
| detector holes with no Autodesk hole of that diameter | 1298 |

Why the 1411 are not matched (150 randomly chosen bodies with at least one miss re-run with `PROBE_FACES`, 405 missed holes): 390 (96%) have only partial concave cylinder faces of that diameter: 381 of at most a half turn (a notch or slot end on a rim: one body is a ring of 32 scallops of an 82 degree arc around a 15 mm disc) and 9 of more than a half turn. A hole needs the full circle (decision above), so these are correctly not holes; Autodesk's recogniser lists them as round holes. 10 have no concave cylinder of that diameter (the Autodesk diameter of a countersunk or tapered hole is the cone's, the bore itself is found). 5 are count differences, not misses: stacked full-circle sections of one bore (a 21 mm bore interrupted by a 4 mm gap is read as one hole, Autodesk lists three; three 0.4 mm stacked rings of a 2.67 mm bore are one hole). No missed true hole was found in that sample, so the sample shows no detector defect; it also cannot show a false positive, since the 1298 extra detector holes are not judged (Autodesk lists fewer holes than there are full-circle bores) and would need hand labels.

Contacts: over the 273 readable assemblies Autodesk lists 979 contacting body pairs (any face type within 0.1 mm) and the detector 845 body pairs with a planar interface. These are different definitions and the body indices could not be mapped across files, so they are totals, not a check.

Reader refusals seen on this set (not detector findings; most frequent first, over body and assembly files): `native context scope requires a supported body` (981, a body that is not a closed supported solid), a sphere without an enclosed area (89), a part with 0, 2 or 3 solids (59: an empty Fusion component or a component with several bodies refuses the whole assembly; `read_all` places one solid per part by design), an edge sample off its spline surface (50), a spline edge's vertex not on its curve (44), a mapped item of no representation (32), a solid not placed by the product structure (32), an unsupported body kind (24), declared edge and surface geometry disagreeing (24), an edge endpoint off its line (21), a spherical face reaching too far from its centre (21), the 2,000,000 entity budget (16). 220 of 498 assemblies therefore give no detection at all because one body refuses. Not changed here: these are reader scope decisions (one solid per part) and, for the spline and sphere cases, kernel limits that need their own evidence.

Limits: sampled evidence for a spline cylinder (a fit within tolerance at 17 x 17 points and the knots, not a proof over the face); a spline whose control polygon is a cylinder only after a re-parameterisation is still recognised only if the samples are on the circle; a B-spline cone, sphere or torus is still `Other`; the Autodesk comparison is by diameter and cannot show position agreement.

## Phase C slice 1: bonded ties on a Problem (2026-11-21)

Decisions (owner, 2026-11-20): the first Phase C slice is bonded ties only, and ties are stored by face geometry, not by kernel or STEP ids.

Built: `fea-problem/src/ties.rs`, `SolidCache::bodies`, `Problem::ties`; tests `fea-problem/tests/ties.rs` (9) and unit tests in `ties.rs` (2).
- **Representation.** `Geometry` is unchanged. A multi-body `Geometry::Solid` is a cache whose `bodies` lists the faces of each body; face tags run on from one body to the next, so supports and loads keep naming `faceN` over the whole assembly. `import_step_native` / `tessellate_step_native` fall back to `assembly_mesh::assembly_cache` (one `read_all`) when the single-body read refuses and the file holds two or more bodies; a file that does not keeps the single-body reader's reason. `finish_import` takes a wall only between faces of one body (an air gap between plates is not a wall).
- **Tie.** `Tie { a, b, gap }` over `FaceSig { body, centroid, normal, area }` in the problem's units (`convert_units` scales them). A tie resolves to cache faces at use by body, normal, plane (1e-3 of the diagonal), centroid (a tenth of the face) and area (a quarter), so an edited or re-exported file resolves or is refused by name, never wrong silently. `Problem::apply_tie(&AssemblyAdvice::Tie)` computes the signatures from the advice's `FaceKey`s on the problem's own STEP text and does not store a duplicate. The Assembly window stays read-only (a window change needs an approved mockup).
- **Solve.** `solid_mesh` meshes each body on its own and merges (node and face-name offsets); the size and volume checks run per body. In `solve_mesh` the smaller face of each tie is the slave: each of its surface nodes, within `|gap| + 0.1 h` of the master face and inside one of its element faces (a tolerance of 1e-6 on the barycentric coordinates), is constrained component by component to the master element's interpolation (3 nodes linear, 6 nodes quadratic: Tet10 midside nodes join too), through `fea_core::mpc::Constraints` and `solve_static_constrained`. A tie on a face that overlaps its partner only partly joins the overlap only (unit test). A slave dof that a support prescribes keeps the support and is reported in the result's notes. A tie that joins no node is an error. The kernel's per-component `rigid_body` check is skipped when ties exist (each body alone looks unsupported); a stack that is free as a whole fails the singular solve as "not fully constrained". The constrained solver reports no residual, so the result is held to the reaction-balance check (1e-6).
- **Refused, not ignored:** plasticity, thermal studies, contact pairs, bushings, and modal / buckling with ties; solid edit operations on an assembly.

Proof (`cargo test --release -p fea-problem --test ties`, `--lib ties`): two stacked 1 x 1 x 1 bars, Poisson ratio 0, held at the base and pulled at the tip, move `sigma L / E` of the whole stack to 1e-6 with Tet4 and Tet10 (a uniform axial state is exact in both meshes, so the tie is checked to rounding, not to a mesh tolerance); the same two bars from a STEP assembly (`compose`d from `NativeBrep` boxes) go from `recommend_step` through `apply_tie` and a unit change to the same closed form; the untied stack is refused; a tie across a gap the tie does not carry joins no node; weights reproduce the slave node positions exactly.

Limits: one material for the whole assembly (per-body materials are the next small step); planar faces only (the advice's `Tie` is planar); the ties are bonded, so a joint that can open or slide is Phase C's contact slice; no fastener model or preload yet; the nearest-element search over master faces is a brute-force scan with a bounding-box filter (fast at the mesh sizes the 400,000-element cap allows, not indexed); equal-area faces take the first as the slave; the stress at the edge of a partial overlap is singular, as for any bonded step, so read it as a trend; no hardware run on Windows (macOS here, CI covers the tests).

## Phase C slice 2: contact ties on a Problem (2026-11-21)

Built on slice 1's representation: `Tie.friction: Option<f64>` (`None` bonded, `Some(mu)` a contact pair with Coulomb friction `mu`; the key is absent from a bonded tie's JSON). `ties::contact_joints` resolves contact ties to `faceN` by the same signature match; `solve::joints` joins them with the 2D contact pairs and the existing `pair_setup` / `pair_specs` / `solve_mesh_contact` path (two passes, so neither mesh is privileged) solves them, in Tet4 and Tet10. `Problem::apply_contact(advice, mu)` stores one from a `Tie` advice item. The 3D contact margin is two faces (`FIT_MARGIN_FACES_3D`) plus the largest gap.

Decisions: every tie of a problem is bonded or every tie is contact, because the nonlinear contact solver takes no multi-point constraints; the combination is refused by name. A contact tie reads `ContactResult` (normal force, tangential force, contact share) from the same code as the 2D pairs.

Defect found and fixed on the way: the solver's engaging overlap (`touch`, 1e-5 of a face, written for sheared frictional stacks) offsets the faces of a frictionless pair too. On two stacked bars pushed by 50 psi the tip moved +1.55e-6 (the wrong way) instead of -3.448e-6: the whole 5e-6 offset, 145 % of the push's own shortening. A frictionless pair now takes no overlap. Measured after: both faces of the joint and the tip match `sigma L / E` to 4e-7 relative in Tet4 and Tet10, contact share 1, normal force equal to the push to 5e-6, equilibrium error 5e-7 or less. `tests/contact.rs` (2D pairs, 8 tests) is unchanged and passes.

Proof (`cargo test --release -p fea-problem --test ties`, 15 tests): frictionless push closed form in Tet4 and Tet10; frictional (mu 0.3) push carries the load, no shear and no slip, tip within 0.5 % (the remaining offset is the engaging overlap, 5e-6 against a 3.4e-3 shortening); a pull that opens the joint, and a joint that starts a gap apart, are refused with "the contact could not be solved" and why, not solved wrongly; validation, JSON round trip, unit change, and a STEP assembly from `recommend_step` through `apply_contact` to the closed form.

Limits: a frictional pair keeps the engaging overlap, so a small load reads an offset of 1e-5 of a face (remove it only with a measured replacement for the sheared-stack case it exists for); contact carries a push only, and a pull or an initial gap leaves a body held by weak springs alone, so the load control cannot find the contact (NoConvergence, reported); no bonded and contact ties together; planar faces only; one material; no fastener model or preload; 3D contact is a manual study (Tet4 frictionless 2 s, frictional 7 s, Tet10 frictional 41 s on the 1 x 1 x 2 stack); no Windows hardware run.


## Phase C follow-ups 1 and 2: per-body materials, bonded and contact ties together (2026-11-22)

Owner approved four follow-ups (per-body materials; mixed bonded and contact ties; gap-closing contact that carries a pull; fastener with preload), in that order. Items 1 and 2 are one change: item 2's tests are built on item 1's stacks.

**1. Per-body materials.** Reuses `Problem::region_materials` (body `k > 0` is `region_materials[k - 1]`, `Problem::region_material(k)` falls back to `material`). `solid_mesh_of_geometry` meshes body `k` with `p.region_material(k).elastic()`, and `merge_bodies` keeps each body's own block. Validation for a solid assembly: at most `bodies - 1` entries, each `elastic().validate()` ("N body materials for M bodies", "body K material: ..."). Defect found and fixed on the way: the strain fields (`MaxPrincipalStrain` and the other strain readouts) read `p.material` for every node, so a node of a second material got a strain from the wrong modulus; `solve::node_moduli` now reads `(E, nu)` from the block that owns each node (the same defect existed for sketch regions of different materials). SPR recovery is node-based and bodies of an assembly share no node, so it never averages across two materials; the contact penalty already used the smallest modulus; the yield margin is skipped when materials differ.
Proof (`tests/ties.rs`, "Per-body materials", 5 tests): two bonded unit bars, Poisson ratio 0, moduli ratio 3 and 0.25, Tet4 and Tet10: joint `sigma L1/E1` and tip `sigma (L1/E1 + L2/E2)` to 1e-6, strain energy `F^2/2 (L1/E1 + L2/E2)/A` to 1e-6; frictionless contact push, same closed form to 1e-5; strain fields read `sigma/E` of the owning body (base `PULL/E1`, tip `PULL/E2`) to 1e-6; equal body material and none gives bit-identical displacements (differential); validation, unit change and JSON round trip.

**2. Bonded and contact ties together.** The contact solver (`fea-core::nonlinear`) had no elimination. New in `fea-core`: `Loads::couplings: Vec<Coupling { terms: Vec<(dof, coef)>, k, offset }>`, a penalty energy `k/2 (sum c_j u_j - offset)^2` in the nonlinear driver (force `k c (c.u - offset)`, rank-one tangent, matrix pattern extended for the coupled node pairs, `check_not_floating` treats a coupling as joining its nodes, a dof the mesh lacks is refused by name). The convergence floor `abs_tol` includes `k offset` of every coupling, so a case that balances to round-off with nothing else applied (the future bolt preload) converges instead of chasing the force scale of round-off. `ties::bonded_couplings` turns the bonded ties' equations (`ties::equations`, the same node search and weights as the exact path) into couplings of stiffness `BOND_PENALTY (1e6) x E_max x mesh size`; `solve_mesh_contact` passes them, and `pair_setup` treats bodies a coupling joins as one body so the weak ground springs go only on bodies nothing holds. A problem with only bonded ties still takes the exact elimination (`solve_static_constrained`); the validation refusal "bonded ties and contact ties do not combine" is removed. The result carries the note "Bonded ties next to contact ties are held by a stiff penalty rather than eliminated."
Why a penalty and not exact: the contact driver has no multiplier or elimination for general equations; a penalty carries a pull (a contact cannot) and its error is measured, below.
Proof: `fea-core/tests/coupling.rs` (4): two blocks joined by node-pair springs, tip = `2 sigma L/E + sigma/k0` to 1e-9; `offset` is the unloaded relative displacement (no load: block B moves rigidly by exactly the offset, A stays); a stiff coupling reproduces the bonded bar to 1e-5; a missing dof is refused. `fea-problem/tests/ties.rs` (3 new): two independent columns in one problem, column A bonded and **pulled** (the joint carries a tension), column B a frictionless contact and pushed, materials 2E, 0.5E, 4E, joint and tip closed forms (measured relative error: bonded column 1e-8 to 6e-8, contact column 4e-7 to 7e-8, Tet4 and Tet10), contact force = push to 1e-5, reactions = loads; a three-bar column (bonded then contact) matches the series closed form to 6e-7 and the exact-elimination solve of the bonded pair to 6e-7 (the penalty error: theoretical bound `h / (1e6 L)` = 5e-7); a tie that joins no node is refused in the contact solve too.

Limits: a bonded tie next to contact is a penalty, accurate to about `h / (BOND_PENALTY L)` of a displacement (6e-7 measured on the unit bars), not exact; the element stress at a bonded edge reads that penalty; the Workbench has no rows for body materials or ties yet (set `region_materials` / `ties` in the problem JSON or through the API; a solid assembly shows no rows in the terminal Workbench at all); contact still carries a push only (items 3 and 4); no Windows hardware run (CI covers the tests).


## Phase C follow-up 3: a contact that starts a gap apart (2026-11-22)

Interpretation (settled by the owner decision below): "gap-closing contact" is built; "a contact that carries a pull" is not, because a unilateral contact cannot carry tension (it opens). Tension passes through a bonded tie, which item 2 now allows next to contact, and through a fastener (item 4).

Built: a body that no support or bond holds, a gap away from a body that is held (through any chain of contact ties, either side loose), used to be held only by the weak ground springs, so load control could not walk it across the gap (NoConvergence, recorded by the old refusal test). `pair_setup` now places each such body: a breadth-first pass from the held bodies moves a body across a joint by the joint's gap along `b`'s outward face normal (`ContactJoint::normal`), cumulatively along a chain (a joint with gap <= 0 carries the shift on), and the solve starts from that displacement (`fea_core::Start`, load factor 0) so the contact is engaged at the start. The weak springs of a shifted body become one-dof `Coupling`s about the shifted position (a spring about zero would leak `k shift` of force: measured 0.14 lbf of 5e4 for two gaps of 2e-3); `check_not_floating` counts a one-dof coupling as a grounding spring. Results report the true displacement (the shift included).
Proof (`tests/ties.rs`, 6 new, the old "starts open is refused" test replaced): gap 1e-3 between two bars (materials E and 3E), push 5e4 psi, Tet4 and Tet10: lower face of the joint `-sigma/E`, upper face `-(sigma/E + gap)`, tip `-(sigma (1/E1 + 1/E2) + gap)` to 4e-7 relative (the closed form), contact force = push to 1e-5; three bars with two gaps of 2e-3 close one after the other (`-((k + 1) sigma/E + k gap)` per bar top to 3e-6); the loose body may be the lower one (fixed top, push up across the gap, 5e-7); frictional pair (mu 0.3) closes and carries the push, no shear and slip, tip within 0.11 % (the engaging overlap of 1e-5 of a face); a gap behind a bonded tie closes (8e-7); a pull after the gap still opens the contact and is refused with the reason. `fea-core/tests/coupling.rs`: a one-dof coupling carries a free plate rigidly to its offset (1e-9).

Limits: planar faces (the shift is one translation along the tie's normal; a curved or tilted pair of faces is not placed); a loose body joined to two held bodies by gaps that disagree is placed by the first joint reached; negative gaps (overlap) are not moved; the equilibrium check of a three-bar chain reads up to 1.5e-6 (the contact solve's own 1e-6 residual against its force scale).

**Owner decision (2026-11-22): contact deliberately carries no pull.** Asked whether "contact that carries a pull" meant (A) tension through a bonded or fastened path next to unilateral contact or (B) a contact that sticks in tension up to a bond strength, the owner chose pull via fastener only: plain contact stays push-only, tension crosses an interface only through a bonded tie (follow-up 2) or the preloaded fastener (follow-up 4), and there is no bond-strength input. A sticking-in-tension contact is not built and must not be. The refusal "a contact between assembly bodies carries only a load that presses the bodies together" is therefore the intended behaviour, not a gap.


## AutoMate check: user-authored mates on real assemblies (2026-11-21)

The last open exit-gate item, "a hand-labelled real model with round holes and a bolted site". Source, vetted before any download and fetched with the owner's approval: AutoMate (Jones, Hildreth, Chen, Baran, Kim, Schulz, ACM TOG 40(6), 2021; Zenodo 7776208, DOI 10.5061/dryad.2547d7wvw; CC0 1.0): public Onshape assemblies with parts as STEP and **mates the users made** (type, the two parts, two mate frames), 255,211 assemblies, 34.9 GB. Not downloaded whole: `scripts/automate/` reads the two zip directories (130 MB) and the chosen members by HTTP byte ranges. The NIST hole test case (HTC, NIST says unrestricted use) was looked at and is Creo / NX / CATIA natives plus a PDF with no STEP, so it cannot be read here.

Sample: 400 of 105,588 small assemblies (JSON 5 to 40 KB, seed 7); 91 have all parts as STEP, 2 to 10 parts and at least one mate; 90 prepared (Zenodo answered two parts with HTTP 429). 660 placed parts. Reproduce with `scripts/automate/README.md`; models are not committed.

Labels come only from the mates: **a bolted joint** is one body whose mates on a single axis line join two other bodies that each have a bore on that line, the fastener body carrying a convex cylinder that fits the bores (no wider than the narrower, 0.05 mm) and overlaps both. Of 98 axis-coincident joints only **17** pass this (the rest are mates made on a hole's circle centre with no shank: a sleeve, a plate-to-plate hole alignment).

Results:
- **Reader**: 44 of 660 parts (6.7 %) cannot be read: spline / B-spline limits of the native reader (unresolved edge endpoints 12, spline vertices out of order across a seam 10, swept-surface spline shape 7, a compound of several bodies 4, edge / surface disagreement 5, other 6). Epic #46 work, not touched here.
- **Detector** on the 616 read: 1083 holes, 198 sites, at most 0.09 s per assembly.
- **Bolted joints**: 17 labelled; 11 have both bores a full turn. Before this change 9 of the 11 were found; the two misses were bores separated by an axial gap (3 mm and 30 mm) with the pin running through both. **Rule added** (`features::find_sites`, `FastenerSite::bridged_by`): two coaxial bores of different bodies that do not touch are joined when a convex cylinder of a third body, at least half a turn, no wider than the narrower bore, spans the gap between them; applied only to bores the touching pass left apart. Result: **11 of 11** found (3 of them across a gap). Fixtures in `tests/features.rs`: pin spans a gap (site, `bridged_by` names the body), no pin (no site), pin too wide, too short, off axis (no site), narrower pin (site), pin through touching bores (no bridge recorded). The synthetic corpus guard (`tests/features_corpus.rs`, "air gap breaks a site" with no pin) still holds. The advice text names the bridging body.
- **Not found, by the hole rule (6 of the 17)**: a bore on that axis is not a full turn (half faces of 3.14 rad, arcs of 1.68 and 1.85 rad, several partial faces summing to less than a turn); not inspected one by one. Two other bores among the 98 axis-coincident joints (not among the 17) are 97 % and 99.5 % of a turn with a gap of 0.36 mm and 0.7 mm in the boundary (a slit: the gap sits at one angle, in a 1.84 mm and a 22 mm radius bore). By the rule a hole is a full turn, so they are not holes. Whether a split bore is a bolt hole is a product decision, left as it is.
- **Size table**: 1 of 11 found sites matched a standard size, 10 of 11 report differing bore diameters: Onshape models use arbitrary dimensions, so the table cannot be validated on this data.
- **Negative results kept**: (1) the 28 labelled joints whose bores did not touch: 25 had no fitting shank at all (not bolts), so a bridge without a shank would have joined unrelated holes; only a shank bridges. (2) A "pin in a hole" proxy (mate axis on a concave cylinder of one body and a convex one of the other, 261 of 295 mates) was dropped: 140 of those 261 have a boss wider than the bore (standoffs, collars), so it does not separate right from wrong and says nothing about side classification.

Limits: the sample favours small assemblies; the labels are what users mated, not a designed ground truth, so recall is measured only on the 17 strict joints and precision is not measured (no label for a site nobody mated: 8 of the 11 bridged sites found in the 90 assemblies are unlabelled and unverified); 6.7 % of parts do not read.

## Near-closed bores and the Fusion 360 Gallery check (2026-11-22)

Owner decisions (2026-11-22): the Fusion 360 Gallery Assembly Dataset may be used (personal, non-commercial use; its licence is non-commercial research only and forbids redistribution, so nothing from it is committed); a bore that covers 97 % or 99.5 % of a turn is a **modelling error, not a hole**: the full-turn rule for what counts as a hole stays, and such bores are shown as candidates the person looks at and repairs, after which they count as holes.

**Built.** `features::find_near_closed` / `NearClosedBore` / `Features::near_closed` (`solid-kernel/src/features.rs`): a concave cylinder group (the one pass `bore_groups` behind `find_holes`) covering `NEAR_CLOSED_MIN_TURN` = 0.9 or more of a turn but not the whole turn. It records the would-be `Hole` (body, faces, axis, radius, range, ends), the covered fraction, the widest uncovered angle interval, `gap_point` (on the bore wall in the middle of the gap, halfway along the bore) and `gap_width` (arc length). `AssemblyAdvice::NearClosed` (`fea-problem/src/assembly_advice.rs`) carries it into the advice: label "Body N bore Ø.. mm is 97.0 % closed: possible modelling gap", the reason gives the covered turn, the missing width and the gap's coordinates; it is listed after the holes without a mate. The Assembly Joints window shows it under Check (the warning group), counts it in the status bar, the Summary card and the Detected tree node, and highlights the bore's faces through the existing selection path. The gap itself is not drawn as a marker: its point is in the Why text. Tests: `solid-kernel/tests/features.rs` (97 % and 99.5 % are candidates and not holes, the gap point's place and angle, the threshold against slot ends, fillets and notches, a repaired bore is a hole), `fea-problem/tests/assembly_advice.rs`, `app-tui/src/gpu_viewer/editor/assembly/tests.rs`.

**No repair is offered, because none exists.** The repo has no user-triggered mesh-heal action. The only heal is `solid-kernel/src/analytic_step/heal.rs`, which moves the samples of an imported STEP edge onto the faces that use it (within `Options::heal_limit_mm`) while reading; it cannot close a gap in a bore. The owner's proxy did not ask for one to be built and none was. The advice text therefore says only "close it in the CAD model and open the file again".

**Threshold.** The partial bores (half a turn or more) by the turn they cover, and how many sit on a designed full hole (so the reader or the CAD export split a full bore short of a turn: a false candidate):

| Turn covered | AutoMate (97 assemblies) | Fusion a1.0.0_04 (718 assemblies) | Fusion on a designed hole |
|---|---|---|---|
| 0.5 to 0.6 | 143 | 364 | 70 |
| 0.6 to 0.7 | 5 | 39 | 23 |
| 0.7 to 0.8 | 14 | 36 | 11 |
| 0.8 to 0.9 | 4 | 56 | 32 |
| 0.9 to 0.95 | 1 | 29 | 1 |
| 0.95 to 0.99 | 6 | 9 | 0 |
| 0.99 to 1 | 4 | 0 | 0 |

(Counts include repeated instances of one part.) Below 0.9 more than half of Fusion's partial bores are designed full holes the reader split, so a lower threshold would flood the list; from 0.9 up only 1 of 38 is. AutoMate has a gap between 0.865 and 0.936. `NEAR_CLOSED_MIN_TURN` = 0.9: 11 candidates in AutoMate, 38 in 17 Fusion assemblies. The candidates are not all errors: AutoMate's six at 0.95 or more are slits with gaps of 0.07 to 1.02 mm (the 97 % and 99.5 % cases are 0.36 mm and 0.70 mm), while Fusion's gaps run 0.8 to 10.6 mm and most look like designed slits (split collars, clamp slots; 8.6 mm in a 29 mm bore 309 mm long). That is why the item says "possible modelling gap" and says it may be a designed slit.

### AutoMate re-run (same scripts, a different sample)

The fetch was repeated with the owner's approval; Zenodo rate-limited (HTTP 429) other parts than the first time, so the sample is a different one: 97 assemblies (first run 90), 48 unread parts. 20 labelled bolted joints (first run 17), 14 with both bores a full turn (first run 11), all 14 found as sites, 6 of them across a gap by the bridge rule (first run 3 of 11). Numbers are not comparable with the first run beyond "the rule held": the labels and limits are those of "AutoMate check" above.

### Fusion 360 Gallery check (separate numbers)

Source: Fusion 360 Gallery Assembly Dataset a1.0.0 (Autodesk AI Lab; licence above), archive `a1.0.0_04.7z` only (1.2 GB, 751 of about 8,251 assemblies; a 7z cannot be range-read, so a whole archive was fetched, with the owner's yes naming file, source and size). `scripts/fusion/` places every body from `assembly.json`'s occurrence transforms (the dataset's own `assembly.step` does not read through the multi-body path: "mapped item … of no representation") and scores against the **designed Hole features** (`holes`: type, diameter, origin, direction), which are a designer's labels, not mates. 721 assemblies placed (30 skipped for more than 80 body instances, 2 without JSON); 718 scored (3 timed out at 180 s), 8,710 body instances of which 1,937 (22.2 %) do not read (the reader's spline limits, epic #46). Detector: 4,496 holes, 499 sites, at most 2.3 s per assembly.

- **Designed holes**: 5,396 labelled; 3,875 on bodies that were read; 2,006 matched by axis (1 degree, 0.1 mm) and diameter (0.05 mm): 51.8 % of those on read bodies. The 1,869 misses on read bodies, by what the model has at that axis and size: a cylinder wrapping only part of a turn, 740 (a hole cut at the body's edge or a notch: by the full-turn rule not a bore); no cylinder of that size on that axis, 1,129 (for 724 of these a same-size parallel bore exists elsewhere on the body); a single face wrapping the whole turn that is not a hole, 0: **no full designed bore was missed by the detector**. The label's frame was tried both ways (root frame 1,975, component frame placed by the instance 1,988, either 2,006): neither explains the 1,129. **Unverified:** the labels may record the feature where it was made in the design timeline, not where the finished part has it.
- **Designed coaxial pairs** (two designed holes of different bodies on one axis with one diameter): 3,208, 2,877 with both bodies read; 319 found as a site (11.1 %). Read this as a ceiling check on a weak label (the same position question as above), not as the detector's recall.
- **Bridge rule**: 126 bridged sites in the 718 assemblies, 83 of them on a designed hole's axis. No label says which bridges are real; the rule's own limit (a shank is required) stands.
- **Near-closed bores**: see the table above.

Limits: one of eleven archives; Fusion assemblies are small, mostly one designer's; the labels are hole features, not joints (the 1,710 dataset joints were not scored: their geometry is entity references to the Fusion model, not to the STEP faces); no dataset labels a modelling gap, so the precision of "possible modelling gap" is unmeasured (what is measured is the share of partial bores that are designed full holes); the 1,129 unexplained misses mean the recall figure is a lower bound of unknown looseness. Negative results kept: a threshold of 0.8 (32 of 56 partial bores in 0.8 to 0.9 sit on a designed hole); summing the wraps of faces at different depths made 22 half rings read as full turns (the scorer now counts one face).

## Phase C follow-up 4: a preloaded fastener (2026-11-22)

Owner decisions (2026-11-22): pull crosses an interface only through a bonded tie or this fastener (contact stays push-only); the preload is a force, or a torque that `fastened-joint-solver`'s own torque-preload equilibrium turns into a force (no maths repeated); the bolt stiffness is that solver's `fastener_axial_compliance` (VDI 2230 additional lengths 0.4 d at the head and 0.4 d3 at the thread included); geometry from the size table and the detected site.

**Model.** `Problem::fasteners: Vec<Fastener>` (`fea-problem/src/fastener.rs`): an axis (`axis_point`, `axis` from the nut toward the head), the axial coordinates of the head and nut bearing planes, a table size (`"M6"`, `"1/4-20"`; thread, pitch and clearance from `solid_kernel::features::size_info`, new), optional head and nut bearing diameters, a `Preload::Force` or `Preload::Torque`, friction and an optional bolt modulus. The bolt is one `fea_core::Coupling` between the area-weighted mean axial displacement of the head's bearing faces and the nut's (the outer faces on the two planes, facing outward along the axis, within the bearing radius of it; weights sum to one, consistent for quadratic faces), with the bolt's stiffness `k_b` and an unloaded span chosen to carry exactly the preload when clamped. `solve_mesh_contact` runs two stages, the same pattern as the bushing fit: (1) bolt cut, the clamp force `P` on the head and nut faces, the members compress (`delta_P`); (2) bolt reconnected with offset `o = delta_P - P / k_b`, the service loads applied from that state (`start_from`). `Summary::fasteners` reports preload, stiffness, tension and the installation torque; the force scale of the equilibrium check includes the preload (it closes on the nut's own nodes, so no reaction shows it). `convert_units` scales every length, force, torque, modulus; JSON round-trips; fasteners need a linear-elastic structural assembly with several bodies and refuse bushings and 2D contact pairs.

**Proof** (`fea-problem/tests/ties.rs`, unit bars, nu = 0, uniform axial state, bolt on the whole face; two bars of `E` and `ratio E`, nut at the fixed base, head on the top; Tet4 and Tet10, bonded and frictionless-contact interface, M6 and M10 with bolt moduli 10 and 50 times steel's so that Phi is 0.2 and larger; `k_m` the series stiffness of the bars, `Phi = k_b / (k_b + k_m)`):
- `k_b` equals the first-principles `1 / sum(L / (E A))` (head 0.4 d, shank, thread 0.4 d3) to 1e-16.
- Preload alone: head `-P / k_m` to 3.3e-7, bolt tension `P`.
- Pull `F` on the head below separation: bolt tension `P + Phi F` to 1.7e-6, head `-P / k_m + F / (k_b + k_m)` to 1.2e-6 of its two terms (it is their small difference), contact clamp `P - (1 - Phi) F` to 7e-6 of `P`. The contact bound is the solve's own 1e-6 of the force scale; the bonded interface (a penalty, follow-up 2) is as accurate as there.
- At 0.95 of the separation load `F = P / (1 - Phi)` the clamp is 5 % of `P` and the forms still hold (tension to 1e-6).
- Torque preload: the torque the joint solver needs for 1e4 matches Shigley's closed form `T = P [d2/2 (l + pi mu d2 sec a) / (pi d2 - mu l sec a) + mu r_m]` (uniform bearing pressure) to round-off; that torque given back returns the force to 1e-6; a rougher bolt needs more.

**Negative result, recorded: a pull past the separation load is not solved.** Past `F = P / (1 - Phi)` the contact must release mid-solve and the bolt alone carries the pull (`T = F`, the upper bar unstressed, head `F / k_b - P / k_m - P / k_b`). The contact driver does not converge through that release: frictionless Tet4 at 1.2 x the load (4 and 20 steps, with the bar's sides held against sliding), frictionless Tet10, frictional (mu 0.3, "stagnated") all end in `NoConvergence` (residual about 6e-3), and at 1.02 x the load the solve never ended (killed after 75 CPU minutes). The cause is not found; the same refusal as a pull on a plain contact (follow-up 3) applies and its hint names a loose body. Nothing was loosened to make it pass. A pull past separation needs the contact driver to release a pair whose pressure falls to zero (fea-core work, `docs/fea-core.md`); a bonded interface carries any pull.

**Assumptions** (listed in the result's notes where they are used): the size table has no head or washer dimension, so the bearing diameter of head and nut is 1.6 x the shank diameter (hex and socket heads of ISO 4014 / 4762 run 1.55 to 1.7 d for M5 to M12) unless given; the bolt is steel (200 GPa, nu 0.29) unless a modulus is given; thread and bearing friction 0.15 (the Preload Analysis default), used only for a torque; the head and nut planes are the outer faces of the outermost plates (a counterbore or washer seat lower down is not found; give the plane); head and nut bearing nodes are those of the mesh faces whose centroid is within the bearing radius of the axis (a coarse mesh gives few); the member stack passed to the joint solver is one placeholder member, its member compliance is not used.

**Limits.** No `AssemblyAdvice::Fastener` -> `Fastener` link yet: the advice carries bore faces, diameters and span but no axis (adding fields would also break exhaustive matches on the variant), so a front end builds the `Fastener` from `FastenerSite` (axis, span, `fastener` size). No app-tui rows for fasteners, ties or body materials. The bolt carries axial force only (no shear or bending share, no thread stripping, no bolt stress check). A fastener needs a contact or bonded tie between the clamped bodies to transmit the clamp.
