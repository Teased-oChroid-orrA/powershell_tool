# Phase 173: concave-crease recovery in the volume mesher

Issue #46. Follows [Phase 172](issue-46-phase-172.md), which found that `tetmesh` puts no boundary vertex on a concave crease, so boundary facets bridge the crease and `face_of_triangle` names them after the nearer flat face. The owner chose a mesher fix ("crease recovery phases") over scaling tractions. This phase adds a crease stage to `fea-core::meshing::boundary_repair`. No gate, tolerance, budget, cap or timeout was changed. `occt_retirement_approved` stays false.

## Change

`fea-core/src/meshing/boundary_repair.rs` (`Work::{on_faces, crease_candidates, split_crease, crease_star, recover_creases, recover_creases_kept}`, `RepairReport::creases`):

- **Candidates.** Boundary edges whose two ends lie on model faces (within `tetmesh::FACE_TOL` of the size, the namer's tolerance) but on no face in common: the edges of a facet that bridges a model edge.
- **Crease point.** K is the point on a face of each end (`surface_point` on both faces, with its outside-depth ceiling and other-face clearance), within one edge length of the edge's midpoint, at a crease proven concave by `tetmesh::reentrant_edge`. It is seeded from the midpoint, then from either end.
- **Operation.** A plain edge split, as the design proposed, failed: all 65 tries on the plate-boss fixture gave inverted cells, because K lies about half a size inside the mesh, beyond the edge's own cells. `crease_star` instead replaces a cavity round the edge by the star of K, with `rebuild`'s ball and fold checks:
  - The cavity starts with the cells round the edge. It grows across faces K does not see, and across faces whose cell with K would be under the gate, within 1.5 sizes of K (at most 160 cells).
  - The replaced surface is a disk. It holds the two bridges, surface faces K does not see, and faces beyond an outline edge whose fan face would turn against the patch the namer gives it.
  - K must be behind every replaced face, or in front of it by no more than 0.02 of the size (the outside depth `surface_point` allows). The outer faces must bound a ball. Star plus wedge must equal the cavity's volume. Every new cell must be positive.
  - The new surface must pass `surface_refusal_on`. That is `surface_refusal` with one extra case: a new face whose owner is one of the crossed faces is admitted when every corner lies on a crossed patch.
  - A nested `cleanup_tet` pass runs round K. The split is kept only when the region's worst cell is no worse than min(old worst, gate 0.1) and no more of its cells are under the gate. K is a feature vertex.
- **Where it runs.** Last, after the quality rounds and the stuck-region rebuilds, on any mesh with model faces (`face_dists` and `triangle_face`). On a mesh with nothing under the target it runs before the untouched early return. It is undone whole if the mesh ends with more cells under the gate than it started with. A first version ran it before the rounds, as the design proposed: it changed where the rounds ended, and `screw.step` native went from 46 cells under the gate to 47 (below). Each split is judged over its own region only, so this whole-mesh guard is what bounds the count.

`fea-core/src/tetmesh.rs`: `FACE_TOL` and `reentrant_edge` are `pub(crate)`.

## Tests

- `fea-problem/tests/facet_naming.rs`: new asserted test `no_facet_named_after_the_top_face_bridges_the_boss_rim` (native pipeline, about 7 s in debug). On the box + boss, the top face has no facet more than 45 degrees off its plane, and its named area is within 0.5 % of its surface area. The table is kept as the ignored diagnostic.
- `fea-core/tests/tetmesh_repair.rs`: new test `no_boundary_facet_bridges_the_concave_plate_boss_rim`. It is ignored because it meshes the 170,000-cell plate-boss fixture twice (about a minute in release). It asserts a bridge area after repair of at most 15 % of the area before.
- `fea-problem/tests/occt_vs_native.rs`: the comparison table prints `open edges`, the boundary edges not shared by exactly two boundary facets (0 on a manifold boundary).

## Results (macOS arm64, release; off = stage disabled on the same build, on = this change)

**Fixtures.**
- Box + boss (`facet_naming`, native): the top face's facets more than 45 degrees off its plane went from 24 (2.34804 mm², all with corners on faces [6] [6] [7]) to 0. Its named area went from -0.007 % to +0.085 % of its surface area. The boss top (face8) is -2.526 % in both runs: its edge with the boss wall is convex, so the stage does not touch it.
- Plate-boss fixture (`tetmesh_repair`): bridges went from 68 (area 0.51544) without repair to 5 (0.02521, 4.9 %) with it. 38 crease splits; 0 cells under the gate after repair; minimum scaled Jacobian 0.1201. The ignored `connected_plate_boss_rim_passes_solver_gate` diagnostic also passes, including its boundary edge-incidence and vertex-link checks.

**Built-in models** (`table`, `NO_SOLVE=1`, 19 models, both paths). Cells under the gate are unchanged on every model (NEMA17 4 on each path, 0 elsewhere), and every boundary has 0 open edges. Only the models with concave edges change: plate with a boss, box + boss, box + boss + hole, t-junction and NEMA17 gain 33 to 124 cells. The t-junction's worst cell drops from 0.124 to 0.100 on native and from 0.123 to 0.103 on OCCT. That is still at or above the gate, as the split rule (no worse than min(old, 0.1)) allows. The asserted `the_native_path_matches_the_opencascade_path_on_quality_size_and_the_benchmark_solve` passes.

**Flat faces** (`flat_face_facet_areas`). Facets more than 45 degrees off a flat face remain only on NEMA17. There they total 18.67 -> 7.89 mm² on OCCT and 19.76 -> 10.44 mm² on native. On face36, the face with the most, they go from 9.54 to 0.51 mm² (OCCT) and from 10.88 to 1.98 mm² (native). The diagnostic now also prints faces named more than 5 % off their surface area. That shows NEMA17 faces 31, 32, 35 and 37 (0.054 mm² each) with no facet, and faces 18 and 30 named 11 to 32 % short, with the stage off as well as on: it is not this change.

**`screw.step`** (`table` with the solve; exact load 373.26):

| Measure | OCCT off | OCCT on | native off | native on |
|---|---|---|---|---|
| Cells under the gate | 43 | 43 | 46 | 46 |
| Tets | 167,236 | 167,324 | 166,669 | 166,797 |
| Reaction z | -383.309 | -385.506 | -371.734 | -373.459 |
| Loaded-face facet area (exact 37.326) | 39.047 (Phase 171) | 38.812 | 37.777 (Phase 171) | 37.428 |

On the slot bottom (`loaded_face_tilted_facets`), the steps across the concave edges (tilt 40 to 90 degrees) went from 3.13 mm² to none on native (no facet tilted more than 10 degrees), and from 3.29 to about 1.07 mm² on OCCT. The OCCT reaction moved away from the exact load (+2.7 % to +3.3 %) while its loaded-face facet area went down. The traction is integrated over the quadratic facets, so the reaction is not traction x linear facet area; this was not traced further.

A first version ran the stage before the quality rounds. On it, `screw.step` native ended with 47 cells under the gate against 46. Running the stage last, with the whole-mesh guard, fixed that.

**Corpus** (35 files, `NO_SOLVE=1`): unchanged. Both paths mesh 3 and pass the gate on 1. Cells under the gate: nist_ftc_11 43 / 32 (OCCT / native), screw.step 43 / 46, linkrods 0 / 0, with the stage off and on. 0 open edges.

## Limits

- macOS arm64 only; default element size only.
- Bridges remain where the replaced disk holds a boundary vertex sunk inside the solid next to the crease: K is then 0.04 to 0.36 of the size in front of that facet, beyond the 0.02 allowed. These are the unsnapped corners Phase 172 counted on `screw.step` and did not trace. The plate-boss fixture keeps 5 such bridges, and NEMA17 and the `screw.step` OCCT path keep some steep facets.
- The open-edge count checks edge incidence only, not vertex links; the plate-boss diagnostic checks both.
- No Windows or hosted CI evidence for this change yet.
