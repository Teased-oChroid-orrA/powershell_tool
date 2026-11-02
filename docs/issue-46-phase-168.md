# Phase 168: native STEP import of parts inside product structure, and degree plane-angle units

Issue #46. Follows [Phase 165](issue-46-phase-165.md), which found native refusing 33 of the 35 licensed corpus files at STEP import: 21 on "assembly/placement mapping is not implemented", 3 on "multiple body representations require explicit mapping" and 2 on "ambiguous plane-angle units". Measured against the owner's retirement gate (`issue-46-abc-and-retirement-gates-plan.md`: native must import and mesh every licensed-corpus file OpenCascade reads). No gate, tolerance, budget, cap or timeout was changed. `occt_retirement_approved` stays false.

## What the corpus files actually contain

No file of the corpus has an assembly: no `NEXT_ASSEMBLY_USAGE_OCCURRENCE`, `CONTEXT_DEPENDENT_SHAPE_REPRESENTATION` or `REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION`. All 22 files with a `MAPPED_ITEM` (the 21 placement refusals and `ftc_08_ap242-e1-tg`) hold it only as an item of a `DRAUGHTING_MODEL`, simple or AP242's complex `DRAUGHTING_MODEL` + `REPRESENTATION` + `SHAPE_REPRESENTATION` (+ `TESSELLATED_SHAPE_REPRESENTATION`). Those are PMI annotation views showing the part's representation, not placements of its shape. Each of these files has exactly one `MANIFOLD_SOLID_BREP`, except `ftc_08_ap242-e1-tg`, which has none.

The three "multiple body representations" files (and nine more once the mapped items stopped refusing first) list the solid in its `ADVANCED_BREP_SHAPE_REPRESENTATION` and again in one or more `SHAPE_REPRESENTATION`s of the same context: AP203 validation properties ("shape for validation of solid"), shape aspects, or the source of a PMI view's map. Two AP242 files list a `GEOMETRIC_CURVE_SET` (datum targets) beside the solid.

The degree files assign `( CONVERSION_BASED_UNIT('DEGREE',#m) NAMED_UNIT(#zero exponents) PLANE_ANGLE_UNIT() )` with `#m = PLANE_ANGLE_MEASURE_WITH_UNIT(PLANE_ANGLE_MEASURE(0.0174532925), #SI radian)`. The old rule counted every `PLANE_ANGLE_UNIT` entity in the file, so the radian basis and the degree made two.

## Changes (`solid-kernel`)

- `step_assembly::shape_mapped_items`: a mapped item held only by draughting models places no shape and is left out (`read` and `read_mapped` use only the others). A malformed mapped item, one whose map is not a `REPRESENTATION_MAP`, or one held by no representation refuses.
- `step_body::BodyContext::read_all`: a file with product structure that places shape (assembly usages, the two transformation entities, or shape mapped items) goes through the existing `step_assembly::read`. Each instance is read from a copy of the document moved by its rigid placement (`Inventory::rigidly_moved`: every three-coordinate `CARTESIAN_POINT` moved, every three-coordinate `DIRECTION` turned, two-coordinate p-curve data untouched; the copy has its own fingerprint, so references into the source do not resolve in it). That gives one body per instance; `BodyContext::document` and `placement` expose the copy and the motion. Everything `step_assembly::read` refuses still refuses (cycles, several roots, a placement in both or neither representation, unit mismatch), and so does a solid that no instance places. A `CARTESIAN_TRANSFORMATION_OPERATOR` placement now refuses by name (it may scale or mirror). An identity placement uses the source document unchanged. `BodyContext::read` (single body) now also refuses one solid instanced several times.
- `step_assembly::related_solids`: a part's solid in a representation joined to the part's placement-only `SHAPE_REPRESENTATION` by a plain `SHAPE_REPRESENTATION_RELATIONSHIP` (no transformation) sits at the part's placement, as most exporters write a part. One level; different length units refuse.
- `BodyContext::bind`: several representations listing the solid in one context bind once (they share one coordinate space and unit); different contexts still refuse ("multiple body representations in different contexts require explicit mapping"). A `GEOMETRIC_CURVE_SET` beside the solid is wireframe and is ignored. Any other item (a `GEOMETRIC_SET`, which may hold surfaces, an `OPEN_SHELL`, a mapped item) still refuses.
- `analytic_step::plane_angle_factor`: the plane-angle unit is the one assigned in the body's own representation context: the SI radian, or a conversion-based unit whose `PLANE_ANGLE_MEASURE_WITH_UNIT` gives its size in an explicit SI radian. The declared factor is used exactly (the NIST files declare 0.0174532925, about 4e-10 rad off π/180 at 45°); it is never inferred from the unit's name. Several assigned plane-angle units, a prefixed radian, a non-angle measure, a basis that is not the SI radian or non-zero dimensions refuse. Only a cone's semi-angle needs the factor, so the refusal is raised at the first cone.

## Tests

Committed synthetic fixtures (`solid-kernel/tests/data/README.md`; no downloaded file): `pmi-view-single-part.step` (the edit plate with a NIST-style draughting view at a shifted placement), `placed-single-part.step` (one box instance turned a quarter about z and moved to (10, 5, 0), its solid in an ABSR related to the part's placement-only SR), `cone-degrees.step` (the cone with its semi-angle in degrees). `tests/step_placement.rs`, 9 tests:

- the PMI-view plate reads vertex-for-vertex like the bare plate (simple and AP242 complex view); the same mapped item in a shape representation moves the plate to x 100..140, y 50..60, z -30..0, in a document with its own fingerprint;
- the placed box lands on x 9..10, y 5..7, z 0..3, every vertex the placement of its source coordinates, and samples;
- both assembly fixtures read two models at the two expected extents, and the single-body read refuses them;
- the degree cone samples to within 1e-12 mm of the radian cone;
- refusals: orphan / malformed / non-map mapped items; a scaling operator; a related representation in metres; a self-referencing usage (a cycle: no root); a placement in both representations; a self-mapping representation; a solid no instance places; a basis that is not the SI radian, a length measure, non-zero dimensions, a milliradian, two assigned plane-angle units, none assigned; a `GEOMETRIC_SET` beside the solid;
- a second same-context representation of the solid and a curve set beside it bind unchanged.

`tests/step_assembly.rs` gains a draughting-view case (two parts, mapped item ignored); its "both usages and mapped items" case now places the mapped item in a shape representation. `tests/step_body.rs` now expects a same-context duplicate to bind and a different-context one to refuse. `tests/analytic_step.rs`'s stray mapped item now refuses as "is not a REPRESENTATION_MAP".

`cargo test -p solid-kernel -p fea-core -p fea-problem --release --locked --offline -j1 --no-fail-fast` on the branch merged with `main` at `2ec523f`: 135 test binaries, 1,156 passed, 0 failed, 48 ignored.

## Corpus rerun

Same 35 files as Phase 165 (local copy, not committed), mesh only, macOS arm64, release, 834 s, with the Phase 165 harness (`STEP_DIR=<corpus> NO_SOLVE=1 cargo test -p fea-problem --features occt --release --locked --offline -j1 --test occt_vs_native -- --ignored --nocapture table`).

Summary: **35 models; OCCT meshed 3, passed the default gate 1; native meshed 2, passed the gate 1**, as in Phase 165. The OCCT rows are unchanged. The two files native meshes (`linkrods`, `ftc_11_rb`) are unchanged in elements and quality.

### Per-file native transitions (strict read)

Phase 165 recorded refusal classes, not files. The per-file column below is reconstructed: the 21 mapped-item files had the placement refusal (it was the first check), `ftc_08_ap242-e1-tg` the single-body check, and the other files' first refusals are unchanged by this phase except where noted. A run with only the placement and unit changes showed the class counts of Phase 165 file for file (3 "multiple body representations", 2 placement directions, 2 geometry disagreements, 2 plane-angle: `ftc_06_rd`, `ftc_07_rd`).

| File | Phase 165 | Phase 168 |
|---|---|---|
| `linkrods.step` | meshed | meshed (unchanged) |
| `nist_ctc_01_asme1_ap203` | placement | edge/surface disagree 4.17e-3 mm |
| `nist_ctc_01_asme1_ap242-e1` | placement | edge/surface disagree 2.30e-3 mm |
| `nist_ctc_01_asme1_rd` | edge/surface disagree 4.17e-3 mm | same |
| `nist_ctc_02_asme1_ap203` | placement | edge/surface disagree 6.67e-6 mm |
| `nist_ctc_02_asme1_ap242-e2` | placement | edge endpoint not on its circle |
| `nist_ctc_02_asme1_rc` | multiple body representations | spline edge must cover its whole curve |
| `nist_ctc_03_asme1_ap203` | placement | edge endpoint not on its line |
| `nist_ctc_03_asme1_ap242-e2` | placement | surface chart work budget |
| `nist_ctc_03_asme1_rc` | edge endpoint not on its line | same |
| `nist_ctc_04_asme1_ap203` | placement | surface chart work budget |
| `nist_ctc_04_asme1_ap242-e1` | placement | invalid or unsupported geometric radius |
| `nist_ctc_04_asme1_rd` | placement directions | same |
| `nist_ctc_05_asme1_ap203` | placement | edge/surface disagree 5.79e-4 mm |
| `nist_ctc_05_asme1_ap242-e1` | placement | edge/surface disagree 7.16e-4 mm |
| `nist_ctc_05_asme1_rd` | multiple body representations | edge endpoint not on its circle |
| `nist_ftc_06_asme1_ap242-e2` | placement | edge endpoint not on its circle |
| `nist_ftc_06_asme1_rd` | plane-angle units | spherical face orientation and extent disagree |
| `nist_ftc_07_asme1_ap242-e2` | placement | spline edge must cover its whole curve |
| `nist_ftc_07_asme1_rd` | plane-angle units | edge sample 2.83e-5 mm off its spline surface |
| `nist_ftc_08_asme1_ap242-e1-tg` | exactly one body (no solid) | same |
| `nist_ftc_08_asme1_ap242-e2` | placement | surface chart work budget |
| `nist_ftc_08_asme1_rc` | placement directions | same |
| `nist_ftc_09_asme1_ap242-e1` | placement | surface chart work budget |
| `nist_ftc_09_asme1_rd` | multiple body representations | surface chart work budget |
| `nist_ftc_10_asme1_ap242-e2` | placement | spline edge must cover its whole curve |
| `nist_ftc_10_asme1_rb` | edge/surface disagree 3.42e-4 mm | same |
| `nist_ftc_11_asme1_ap242-e2` | placement | spline edge must cover its whole curve |
| `nist_ftc_11_asme1_rb` | meshed | meshed (unchanged) |
| `nist_stc_06_asme1_ap242-e3` | placement | spherical face orientation and extent disagree |
| `nist_stc_07_asme1_ap242-e3` | placement | unsupported representation item (an `OPEN_SHELL` in a representation of the solid) |
| `nist_stc_08_asme1_ap242-e3` | placement | edge endpoint not on its line |
| `nist_stc_09_asme1_ap242-e3` | placement | surface chart work budget |
| `nist_stc_10_asme1_ap242-e2` | placement | edge endpoint not on its circle |
| `screw.step` | spline edge must cover its whole curve | same |

Strict-read classes now: edge/surface disagreement 7, surface chart work budget 6, spline edge coverage 5, edge endpoint off its circle 4, edge endpoint off its line 3, placement directions 2, spherical orientation 2, geometric radius 1, spline-surface edge sample 1, unsupported representation item 1, no solid 1. The tolerant re-read mostly stops on the chart work budget or the geometric radius. **No file refuses on product structure, representation binding or plane-angle units any more except `stc_07` (a surface item beside the solid, deliberately still refused) and `ftc_08_ap242-e1-tg` (no solid).**

## What this shows and does not show

- It removes the import blocker Phase 165 named: 33 of 35 files now pass product structure, representation binding and units (8 before), and the 31 of them native still refuses stop at the geometry checks.
- It does not move the gate count: native still meshes 2 files, OCCT 3. Against the owner's bar, `screw.step` (OCCT meshes it, native refuses on spline edge coverage) is the one corpus file OCCT meshes and native does not. The other 32 files OCCT reads but refuses to mesh at the import's default size (30) or as non-manifold tessellations (2); native now refuses them at geometry checks instead of structure.
- It does not test real multi-instance assemblies from an outside exporter (the corpus has none); assembly placement is covered by the synthetic fixtures and `step_assembly`'s OpenCASCADE cross-check (feature `occt`). The production path (`fea-problem::native_solid`) reads a single body, so a file with several instances still refuses there with "exactly one supported body".
- macOS only; Windows not run.

## Next

The geometry classes above, starting with the ones that block the most files: edge/surface disagreement at 1e-6..4e-3 mm (7; NIST exporters' declared geometry is looser than the strict tolerance, and the tolerant read then exhausts the chart budget), the chart work budget (6), and spline edges that do not cover their whole curve (5, including `screw`, the only file OCCT meshes and native does not).
