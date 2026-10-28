# Phase 150: OCCT fillet seam tagging diagnosis

2026-10-28. This records a pre-existing OCCT mesh failure identified in [Phase 148](issue-46-phase-148.md). No production mesh change was made for it. OCCT remains production, and `occt_retirement_approved` remains false.

## Reproduction

The fully filleted 12 × 14 × 16 mm box has an exact flat top area of `(12 - 4) × (14 - 4) = 80 mm²`. At mesh size 1 mm, the volume mesh tags 80.42372283704826 mm² as that face. The existing [fillet regression](../fea-problem/tests/fillet_rim_tagging.rs) allows 0.5% (up to 80.4 mm²), so it fails. The same value was recorded on `main` at `a3c61bd` in Phase 148; Phase 149 did not introduce it.

```sh
CARGO_INCREMENTAL=0 cargo test -p fea-problem --features occt --release --locked --offline -j1 --test fillet_rim_tagging the_flat_face_next_to_tangent_fillets_is_tagged_with_its_own_area_and_no_vertex_leaves_the_solid -- --nocapture
```

This command failed on the Phase 149 worktree with `h 1: tagged area 80.42372283704826 against the exact 80`. The existing traction test separately checks that a load on this face produces the closed-form force.

## Cause and required repair

`solid-kernel::mesh_volume` passes the tagged surface to `fea-core::tetmesh::mesh_solid`, which builds an unconstrained tetrahedral boundary. Some resulting boundary triangles cross the tangent seam between the flat top and its fillet. `faces_of_named` calls `face_of_triangle`; when no CAD patch contains all three vertices, that function assigns the whole triangle by its centroid. One such top-tagged rim triangle includes a vertex near `z = 15.988226` and about 0.030004 mm outside the flat top's x/y bounds. Whole-triangle ownership cannot represent its two physical patches.

A repair must locate CAD seam intersections on crossing boundary edges, split the incident tetrahedra conformingly, and assign names only after every boundary triangle lies on one CAD face. It must preserve a closed manifold, positive elements, the volume and surface-deviation guards, scaled-Jacobian quality, and the traction area. The existing `boundary_repair::split_edge` is quality-driven, refuses some pinned feature edges, and does not locate seam intersections; applying it blindly is insufficient. Relabeling a straddling triangle or relaxing the area gate would conceal the force error.

The seam-conforming topology work is larger than this session's safe local change. The failing regression remains enabled as the gate for a future meshing increment.
