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
