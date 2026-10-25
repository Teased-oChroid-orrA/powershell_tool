# Phase 141: repair contracts qualified on all 19 parity models, fea-core audit fixes

2026-10-25. Continues [Phase 140](issue-46-phase-140.md) on `issue-46-boss-rim-repair`. OCCT remains production and `occt_retirement_approved` remains false.

## Mesher changes (`fea-core/src/meshing/boundary_repair.rs`, commit 6725bc8)

- **Folded-face selector.** In the repair round, a cell with a boundary face whose alignment to its CAD normal is below 0.5 is worked like a low-quality cell. This integrates the external Phase 139 selector.
- **Owner guard with tangent seams.** The CAD owners of the new faces of an operation must be among the owners of the removed faces. Strict equality dropped table passes from 26/38 to 16/38 (it broke `cylinder (native)`: 8 sub-gate elements with it, 0 without), so the rule is a subset. A face classified to another patch is still admitted when `face_touches_patch` holds for a removed patch: centroid within 0.1h of it, outward normal within 60 degrees of its normal. Across a tangent fillet seam the classification of a straddling face is arbitrary; without this exception the guard refused 110 repairs on `box, all edges filleted` (1/3 sub-gate elements, 0/0 with it).
- **Region surface-fidelity bound.** Phase 139 held every new face to an absolute 0.1h sampled chord error and each existing face to its own old error. On a fillet, correctly snapped faces already carry 0.2–0.4h, so almost every slide and re-tiling there was refused (84% of the refused new faces were no worse than the faces they replaced), leaving flat ear cells on `cross-bore, filleted`. Now no face after an operation may deviate more than the worst face of the region it replaces, or 0.1h if that is larger. This is the one relaxed check; it cannot move a region further from the CAD surface than it already was.
- **Stale budget.** Enabling the relaxation stage from the no-progress path now resets the stale counter. Without it the folded cells, which keep the count above target, ended the loop one round early: +12 sub-gate cells on the OCCT row of `cross-bore, filleted`.
- **No new sub-gate cells.** `better()`, `promote` and `optimise_vertex` refuse an operation that leaves more cells below the 0.1 gate than before (only the cavity's worst cell was required to improve, so t-junction rose from 14 to 18 sub-gate cells with the fidelity fix alone). This is stricter.
- Unit tests: `new_surface_faces_may_not_introduce_a_cad_patch`, `a_face_touches_a_tangent_patch_but_not_one_across_a_sharp_edge`, `surface_fidelity_is_bounded_by_the_worst_face_of_the_replaced_region`, `an_operation_may_not_add_elements_under_the_gate`.
- Cause localization used one-change-at-a-time toggles (all removed): for the filleted box only the owner guard changed the count; for the cross-bore, restoring dfdb309's `boundary_repair.rs` into c14da96 gave 0/0, so the tetmesh changes (prune, reentrant snap) played no part.

## fea-core audit fixes

Each defect test was seen failing on the old code before its fix.

- 55e3fd1 `ordering.rs`: nested dissection stopped on an empty separator (disconnected halves) and left the subset in natural order. Two disjoint Hex8 blocks gave 2.69x the fill of one block (3431898 vs 1273527); now within 1.8–2.3x (`tests/ordering_disconnected.rs`).
- 3f1679c `optimize.rs`: sizing optimisation set `material.e`, ignored by anisotropic materials (bar 0: 2.717 vs closed form 1.846). New `Elastic::scaled`.
- b3b6ab1 `sensitivity.rs`, `dynamics.rs`: the modulus sensitivity omitted the thermal load term; a lumped-mass mode's eigen sensitivity differentiated the consistent mass. `Mode::mass` records the mass kind; HRZ lumping is one helper `dynamics::lump_hrz`. Both tests failed on the old source (`tests/sensitivity.rs:177`, `:200`).
- c039852 `adapt.rs`: the size-field grading stopped after `min(n, 200)` relaxation sweeps silently, breaking `|h(x)-h(y)| <= grading |x-y|` on long meshes (600-element strip: h(0) = 1000 against a limit of 240.6). Now exact multi-source Dijkstra over mesh edges.
- e442dea `sweep.rs`: a revolution with profile nodes on the axis duplicated each axis node per angle level (1360 nodes vs 1105; zero-area boundary faces on the axis). Axis nodes are now one node shared by all layers.
- 8ac288d `adaptivity/marking.rs`: `trim_to_budget` recounted marks per drop (quadratic): 60k elements 0.256 s to 0.003 s, same result.
- 8867b5d, c456a41: audit tests with no defect found: every element kind against closed forms and nonlinear tangents against finite differences (`tests/audit_elements.rs`); assembled contact tangent, friction and rigid-master couplings against finite differences, worst relative error about 2e-9 (`tests/audit_contact.rs`).

## GUI (d3c3105)

The solid editor ribbon follows NX / SolidWorks: Model (Sketch, Features, Part), Mesh, Simulation (Study, Fixtures & loads, Run). One smart Fixed support / Load acts on the selection when there is one; file and edit commands live only in menus and shortcuts; a test fails on any duplicated ribbon command. Checked with the solid and sketch snapshots in light and dark.

## Evidence (macOS arm64, release, `CARGO_INCREMENTAL=0`, serial Cargo, at 8ac288d)

- 19-model same-STEP OCCT/native table with static solves (`docs/issue-46-phase-141-parity.txt`, 2039 s): **34/38 rows pass the gates** (Phase 128: 26/38). No row is worse than Phase 128. Elements below 0.1, OCCT/native, against Phase 128:

  | Model | Phase 128 | Phase 141 |
  |---|---|---|
  | plate with a boss (OCCT) | 8/26 | 0/0 |
  | box + boss (native) | 3/5 | 0/0 |
  | box + boss + hole (native) | 77/95 | 0/0 |
  | cross-bore (R8, R3) | 4/0 | 0/0 |
  | tilted hole | 0/1 | 0/0 |
  | t-junction | 40/53 | 8/8 |
  | NEMA17 motor bracket | 70/87 | 24/33 |
  | all other rows | 0/0 | 0/0 |

- Solve parity on the same STEP: max displacement differs by at most 1.6% between OCCT and native meshes (`plate, two holes`), at most 0.7% on every other row.
- `cargo test -p fea-core -p solid-kernel -p fea-problem --release --locked --offline -j1 --no-fail-fast`: 1121 passed, 0 failed, 47 ignored.
- `connected_plate_boss_rim_passes_solver_gate` (ignored diagnostic) passes.
- Boss audits on the Phase 140 build plus the selector (before the owner guard and the later fixes): 0 forbidden contacts, min owner-normal alignment 0.5728, 0 back-facing faces, volume 61.54609673 against exact 61.57168026 (0.042%), all cells distinct and exactly positive. These were not rerun on 8ac288d.
- `cargo test -p app-tui --release --locked --offline -j1`: 1226 passed, 0 failed, 22 ignored. (Two solve tests timed out once while the parity table and two agents loaded the machine; they pass alone and in this run.)

## Limits and what is still open

- t-junction (8/8) and NEMA17 (24/33) still fail the gate; every remaining sub-gate cell is on the boundary. Keeping the best round's mesh brought t-junction to 7/8 in a trial and was not kept.
- The region fidelity bound is weaker than the Phase 139 absolute bound on surfaces that were already coarse; a fidelity measurement on the fillet cases was not made.
- `face_touches_patch` also admits faces across a sharp edge whose dihedral normals differ by less than 60 degrees.
- Not audited in fea-core: derivative `path`/`complementarity`/`policy`/`tape`/`second_order`, adaptivity `controller`/`dwr`/`equilibrated`/`fusion`/`zones`/`moving`, `tet_delaunay`, `transition`, `hybrid`, `partition`, `metric_refine`, `strategy.rs`. Low-impact leads: plane-stress solvers return the last iterate without a convergence flag (no unconverged case in 20,000 random states); `Scalar::atan2` is NaN at (0,0), uncalled.
- Windows evidence and the remaining five-plan items have not advanced.
