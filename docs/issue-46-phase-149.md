# Phase 149: moving a boundary vertex away from a nearby CAD edge

2026-10-28. Continues the [Phase 148 screen](issue-46-phase-148.md) on epic #46. OCCT remains production, and `occt_retirement_approved` remains false.

## Problem

NEMA17 still has 12 OCCT and 11 native tetrahedra below the 0.1 scaled-Jacobian gate. At its lower gusset corners, the repair treats a vertex on the slope but about 0.15 from the gusset-side face as pinned to their shared edge. It can then move only along that edge, although the vertex lies on one face and needs room to slide away from the edge. The existing `fixed` classification also protects boundary topology in edge removal, peeling, collapse, merging, and splitting, so changing that classification would remove those protections.

## Change

`optimise_vertex_with` keeps `fixed` unchanged. For a pinned vertex near exactly two CAD faces, it checks their trimmed-face distances. If one distance is below 0.02 of the local size and the other exceeds it by more than 0.02 of the size, the optimiser tries its normal single-face surface slide. Other pinned vertices still move along their model edge or onto a model corner. The single-face projection refuses a move toward the other face; the existing surface check verifies face ownership, deviation, and orientation.

The new slide also keeps the total volume of its incident tetrahedra to within `1e-10` of their starting volume. This guard applies during the candidate search, before a move can be committed. The quality gate, target, surface limit, and topological `fixed` checks are unchanged.

## Result

The release NEMA17 mesh-only comparison improves from 12 to 10 sub-gate elements on OCCT and from 11 to 9 on native. Both paths still fail the 0.1 gate, and all remaining sub-gate elements lie on the boundary. The 19-model mesh-only table remains at 36 of 38 passing rows ([parity record](issue-46-phase-149-parity.txt)).

Four passing rows change while keeping their gates: both plate-with-two-holes rows, the native box-with-boss-and-hole row, and the OCCT edited-plate row. The native boss row's maximum edge ratio increases from 5.9 to 7.8, below the gate of 50. All other rows are unchanged apart from mesh time. The table does not include static solves.

## Rejected trials

The [Phase 148 screen](issue-46-phase-148.md) suggested a feature-preserving local cavity repair. A two-apex fill over one boundary face or a small connected patch preserved the sampled cavity boundary and volume, but both sampled lower-corner and slot-end cavities gained sub-gate elements. Moving only the lower-corner interior apex reduced its local bad-element count while worsening the worst element, so it failed the existing quality contract.

A coupled collapse of the short pinned edge and interior-apex move reached zero local sub-gate elements in a numerical screen. An in-process rollback probe rejected it at `collapse#8`: the removed vertex deviated 0.199 from the new boundary against a 0.089 limit, and a new face turned against its CAD owner. A nearby boundary slide could balance volume in the sampled region, but it did not repair these surface failures. The model-specific probe and trace code were removed.

## Evidence and limits

- `CARGO_INCREMENTAL=0 CASE=NEMA17 NO_SOLVE=1 cargo test -p fea-problem --features occt --release --locked --offline -j1 --test occt_vs_native table -- --ignored --nocapture`: passed, 10 OCCT and 9 native sub-gate elements.
- `CARGO_INCREMENTAL=0 NO_SOLVE=1 cargo test -p fea-problem --features occt --release --locked --offline -j1 --test occt_vs_native table -- --ignored --nocapture`: passed, 36 of 38 rows meet the quality gate.
- `cargo test -p fea-core --release --locked --offline -j1 boundary_repair`: 46 passed, 0 failed.
- `cargo test -p fea-core -p solid-kernel -p fea-problem --release --locked --offline -j1 --no-fail-fast`: 1,134 passed, 0 failed, 47 ignored.
- `cargo test -p fea-problem --features occt --release --locked --offline -j1 --test occt_vs_native -- --ignored the_t_junction_repair_keeps_its_best_round the_nema17_native_relaxation_starts_from_the_best_round`: 2 passed, 0 failed.
- `cargo test -p fea-core --release --locked --offline -j1 --test tetmesh_repair -- --ignored connected_plate_boss_rim_passes_solver_gate`: 1 passed, 0 failed.

The NEMA17 gate remains open. Solves on the Phase 145–149 meshes and Windows mesh or solve evidence remain open. The pre-existing OCCT `fillet_rim_tagging` failure remains a separate CI blocker.
