# Phase 143: chained vertex moves, and the relaxation starts from the best round

2026-10-25/26. Continues [Phase 142](issue-46-phase-142.md) on `main`. OCCT remains production and `occt_retirement_approved` remains false.

## Problem

After Phase 142 the t-junction keeps 7 (OCCT) and 8 (native) elements under the 0.1 gate, at four mirror-symmetric spots (|x| about 5.5, |y| about 2.5 to 3, z about 17.4 to 18.4) where the branch meets the main pipe. Each of these elements has three or four pinned feature vertices, and the reference sizes of adjacent seam vertices differ by up to a factor of 2.7 (0.63 against 1.74).

The trace (`REPAIR_TRACE=1 NO_SOLVE=1 CASE='t-junction'`) now tallies why each candidate position of a vertex move is refused. At the stuck elements nearly every candidate is refused because it pushes another element of the star under the gate, and most of those elements were sound before the move (scaled Jacobian 0.15 or more). They have the slack to recover, but a move of one vertex cannot use it.

## Tried first and dropped: splitting edges along the model feature

`split_edge` refuses a boundary edge between two pinned vertices (`split_edge#2`). The first attempt allowed it when both vertices lie on the same model edge (the same two faces), placing the new vertex on that edge. A unit test showed the split worked on a wedge, but on the t-junction it made the result worse:

| t-junction | Phase 142 | feature split |
|---|---|---|
| OCCT | 7/8 under the gate (min 0.053) | 10 (min 0.047) |
| native | 8 (min 0.060) | 10 (min 0.060) |

Nine feature splits were accepted in the early rounds. A split is accepted when the cavity's minimum is not worse, so these splits changed the path of the repair without lowering any count. OCCT's best round fell to 10 (round 5), native held at 10 to 11 for more than ten rounds, and 55 later split attempts were undone (`split_edge#3`). The split was removed. The refactor it introduced, `Work::feature_edge` (a feature vertex's two faces and edge tangent, factored out of `optimise_vertex` with the same refusal codes), stays.

## Change (`fea-core/src/meshing/boundary_repair.rs`, ebd19ea)

- **`Work::chained_move(v)`**, a new step of `cleanup_tet` after the penalty smoothing and before the split. It moves `v` to the best position for its star even when that puts other elements of the star under the gate (`optimise_vertex_with(.., push = true)`; the surface checks still apply), then repairs each element of the star under the gate by moving its other vertices (`optimise_vertex`, up to two passes). The chain is one transaction: it is kept only when the elements around every moved vertex hold **strictly fewer** elements under the gate than before and their worst scaled Jacobian is not lower. Otherwise every move is undone, and the report counters with them. Only vertices move, so the elements around the moved vertices are exactly the elements the chain changed, and the comparison is exact.
- The strict decrease is the lesson of the feature split: an operation accepted on "no worse" can spend the rounds without lowering any count.
- `RepairReport::chained` counts kept chains.
- **The relaxation stage starts from the best round.** The repair loop enables its last-resort relaxation (boundary vertices may sink a little under the surface) when the exact stage stalls or stops progressing. It used to relax from wherever the exact stage stalled; it now restores the best round's `Snapshot` first (the same snapshot and ranking key Phase 142 keeps for the end). The chained move alone made NEMA17 native worse: its exact stage went further (33 elements under the gate against 37 to 40 without it) but then stalled at 37, and the relaxation from there ended at 32 against Phase 142's 29 (relaxation from a stall at 39). Which state the relaxation starts from decides what it reaches; starting it from the best one removes that dependence on where the stall happened.
- Trace only (`REPAIR_TRACE`, no change in behaviour): refusal tallies per candidate position in `optimise_vertex` (including the quality band of elements pushed under the gate), the operation that succeeds in `cleanup_tet`, the mesh-wide count before and after each probe, each chain's counts, and up to 8 probed elements (was 3).

## Tests

- `fea-problem/tests/occt_vs_native.rs::the_t_junction_repair_keeps_its_best_round` (occt, ignored, about 23 s in release) is tightened from `<= 7` to `<= 5`. With the chained step disabled it fails ("7 elements under the gate"); with it, it passes.
- `fea-problem/tests/occt_vs_native.rs::the_nema17_native_relaxation_starts_from_the_best_round` (occt, ignored, about 2.5 minutes in release): NEMA17 through the native tessellation ends with at most 25 elements under the gate. With the chained move but without the restore it fails ("32 elements under the gate"); with both it passes.
- `boundary_repair::tests::a_move_that_pushes_neighbours_under_the_gate_is_kept_once_they_are_repaired`: a 4 x 4 x 4 lattice with perturbed interior vertices (found by a seeded search) has 5 elements under the gate; the plain move of vertex 21 is refused, the chain clears all 5, raises the minimum, and commits as one change.
- `boundary_repair::tests::a_chain_that_does_not_lower_the_count_is_undone_whole`: on the same lattice every refused chain leaves the points, the log and the report untouched.

## Evidence (macOS arm64, release, `CARGO_INCREMENTAL=0`, serial Cargo, at ebd19ea)

- The 19-model same-STEP table with solves (`docs/issue-46-phase-143-parity.txt`): **34/38 rows pass**, as in Phase 142. Three rows changed, none worse. All other rows are identical to Phase 142 in element count, minimum scaled Jacobian, gates and sub-gate counts:

  | Model | Phase 142 | Phase 143 |
  |---|---|---|
  | t-junction, OCCT | 7/8 under the gate | 5 (min sJ 0.053 to 0.050) |
  | NEMA17 motor bracket, OCCT | 24 | 24 (206043 to 205998 elements) |
  | NEMA17 motor bracket, native | 29 | 25, of which 24 on the boundary |

- Each change alone, measured mesh only on the two affected models (Phase 142: t-junction 7/8, NEMA17 24/29):

  | Configuration | t-junction OCCT | t-junction native | NEMA17 OCCT | NEMA17 native |
  |---|---|---|---|---|
  | feature split (dropped) | 10 | 10 | not run | not run |
  | chained move only | 5 | 8 | 24 | 32 |
  | relaxation from the best round only | 8 | 8 | 24 | 22 |
  | both (this phase) | 5 | 8 | 24 | 25 |

  Only the combination is no worse than Phase 142 on every row. The outcome still depends on the path the rounds take: the restore alone reaches 22 on NEMA17 native but loses a cell on the t-junction. Neither number is an optimum.
- Solves on the changed rows, against Phase 142 (Phase 141 for NEMA17 OCCT): max displacement moves by at most 1.5e-4 relative (t-junction OCCT 1.45612e-3 to 1.45634e-3; NEMA17 OCCT 6.73173e-3 to 6.73131e-3; NEMA17 native 6.71349e-3, unchanged). Reactions and equilibrium are unchanged. The OCCT/native max displacement gap is 0.66% (t-junction) and 0.27% (NEMA17).
- `cargo test -p fea-core -p solid-kernel -p fea-problem --release --locked --offline -j1 --no-fail-fast`: 1124 passed, 0 failed, 47 ignored (Phase 142: 1122, plus the two new unit tests).
- `connected_plate_boss_rim_passes_solver_gate` passes.
- `cargo test -p app-tui --release --locked --offline -j1`: 1226 passed, 0 failed, 22 ignored.
- Solve and mesh times were taken under outside load and are not compared.

## Limits and what is still open

- t-junction (OCCT 5, native 8) and NEMA17 (24/25) still fail the gate. On the native t-junction every chain tried at the stuck cells raised the count after the repair of the pushed neighbours (1 to 4, 1 to 2) and was undone.
- The repair remains sensitive to its path (table above). A strategy that does not depend on the order of rounds, for example a joint optimisation of the pinned vertices of a stuck region, or rebuilding the region locally, is the next lever; neither is started.
- NEMA17 native now has one element under the gate that is not on the boundary.
- The seam vertices at the t-junction stuck spots carry reference sizes that differ by up to 2.7 times between neighbours; the surface discretisation of that intersection curve was not examined.
- **Pre-existing CI failure, not caused by this phase:** `fea-problem/tests/fillet_rim_tagging.rs::the_flat_face_next_to_tangent_fillets_is_tagged_with_its_own_area_and_no_vertex_leaves_the_solid` fails whenever the OpenCascade feature is on (the macOS and Ubuntu `qualify` jobs, which pass `--features app-tui/occt`; every run since 2026-10-20 gives the same 80.42372 mm2). Its tolerances were measured on the native path only. Measured here at h = 1 (release, macOS): OCCT tags 80.424 mm2 as the top face (+0.53 %, gate 0.5 %), native 80.107 (+0.13 %). With OCCT, 7.42 mm2 of the tagged triangles have centroids under z = 16 and their lowest node is at z = 15.988: rim vertices sit about 0.22 mm past the tangent seam on the fillet. Tangent seams are deliberately not pinned (`Work::fixed` lets a vertex cross them), so where the tagged region ends depends on the tessellation. Fixing it means placing the vertices between two tagged regions on the B-rep edge (seam-conforming tagging), a meshing-core change that needs the full table requalified; it is left open and the gate is not loosened.
- Everything else in the Phase 142 Limits list is unchanged: the boss audits on the final build, a fidelity measurement under the region bound, `face_touches_patch` across shallow sharp edges, the unaudited fea-core modules, and Windows evidence.
