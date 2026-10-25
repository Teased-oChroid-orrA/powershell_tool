# Phase 140: connected boss passes its solver and manifold gates

2026-10-24. Continues [Phase 139](issue-46-phase-139.md) on `issue-46-boss-rim-repair`. OCCT remains production and `occt_retirement_approved` remains false.

## Changes

- Boundary-repair collapse now evaluates the surface transaction over the whole affected endpoint region, not only newly added cells. A deletion-only collapse that exposes a surviving cell's base face is judged on that face. The regression `collapse_checks_the_boundary_exposed_on_surviving_cells` failed before this change (`/private/tmp/issue46-exposed-boundary-red.log`) and passes after it.
- `tetmesh::drop_pinched_sectors` runs on the kept lattice cut, before snapping. Around every vertex and edge the kept tetrahedra must form one face-connected fan. Otherwise the smaller-volume sectors are removed, repeating until no pinch is left. Phase 139 showed that all 62 nonmanifold boundary edges of the connected boss already exist with `snap:false, repair:false`, so no later stage could remove them. This generalizes the external edge-only prototype of Phase 139 to vertex pinches and to pinches exposed by an earlier removal. The unit test `pinched_sectors_keep_the_largest_fan` covers an edge pinch, a vertex pinch and a face-connected pair that must stay.

## Evidence (macOS arm64, `CARGO_INCREMENTAL=0`, serial Cargo)

- `connected_plate_boss_rim_passes_solver_gate` (ignored diagnostic, release) **passes**. It requires the default quality report, no inverted cells, no cells below the 0.1 scaled-Jacobian gate, every boundary edge shared by exactly two triangles, and a connected cyclic link at every boundary vertex. Before the prune the same run failed with `nonmanifold boundary edge` (`/private/tmp/issue46-pickup2-boss.log`). The passing log is `/private/tmp/issue46-pickup3-boss.log`.
- `cargo test -p fea-core -p solid-kernel -p fea-problem --release --locked --offline -j1 --no-fail-fast`: 1107 passed, 0 failed, 47 ignored (`/private/tmp/issue46-pickup3-suite.log`).
- Before the prune: fea-core library 251 passed, 7 ignored; `amr_in_place`, `high_order_convergence`, `tetmesh`, `tetmesh_repair` 16 passed, 5 ignored.

## Limits and what is still open

- The exact whole-mesh contact audit and the negative owner-normal alignment check of Phase 139 were not rerun on this build. Phase 139 measured the edge-only prune externally with zero forbidden contacts and one back-facing cylinder triangle. The surface-queue selector that removed that triangle is still external and not integrated.
- The removed sectors change enclosed volume. Phase 139 measured a slightly larger analytic-volume error for the edge-only prototype. This phase did not measure the new volume.
- Same-STEP OCCT/native mesh and static solves, Windows evidence and the remaining boss-plan qualification have not run. The tilted-hole gate and later five-plan items have not advanced.
