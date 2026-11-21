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

**`face_holds` fixed (2026-11-20).** The finding above is resolved in `solid-kernel/src/native_brep_drill.rs`: `polyline` sampled a full circle's last point as `cos / sin` of 2 pi, which is off the start vertex by rounding, so the one-edge loop of a pocket floor was not closed and a ray through that vertex (the floor's axis) missed the closing crossing for radii of about 3.7 mm and up. The ends of every sampled edge are now its exact vertices. Regression tests: `the_centre_of_a_blind_hole_floor_is_inside_it_at_every_radius` and `a_bore_can_be_drilled_in_the_floor_of_a_wide_counterbore` (`solid-kernel/tests/native_brep_boolean.rs`, a 12 mm counterbore with a bore in its floor against the closed-form volume); the counterbore test in `fea-problem/tests/assembly_advice.rs` now uses an 8 mm counterbore.
