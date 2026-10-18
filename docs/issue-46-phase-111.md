# Issue 46 — Phase 111: a hexahedral core inside a native body, joined to tetrahedra (#56)

Phase 102 built the hexahedron | pyramid | tetrahedron join (`hybrid::hex_pyramid_tet`) on a hand-made cantilever; what it left open was **choosing the kept hexahedral region of a natively imported body**. `hybrid_region::hex_core_hybrid` does that for an interior core.

## What changed

- **Region chooser** (`hybrid_region::core_cells`). A regular grid of edge `cell` over the body's bounding box; a cell is kept when all eight corners lie at least `margin_cells` (default 2) cell edges inside (`SurfaceSolid::signed_distance`, a BVH with pseudo-normal signs): room for the pyramid layer (apex height 0.5 cell) and the tetrahedra between it and the shell. Only the largest face-connected group stays, and the weakest cell of any boundary **pinch** is dropped until none is left: an edge shared by more than two boundary quadrilaterals, or a vertex whose boundary quadrilaterals fall into more than one edge-connected fan. The interface is then a closed manifold surface.
- **The join** (`hex_core_hybrid`): the kept cells become one `Hex8` block, the boundary quadrilaterals of that block the interface, the welded native shell (`SurfaceSolid::vertices` / `triangles`, new accessors) the tetrahedral region's outer surface. `hex_pyramid_tet` already supported a closed interface (the interface nodes the shell lacks become interior nodes that only pyramid sides touch); its pyramid layer is a **cavity** of the tetrahedral region, and `conform` / `conform_preserving` extract the cells between the shell and the cavity (a cube with a cubic cavity: volume exactly 56 of 64 - 8, probed).
- **Concave steps** (`hybrid.rs`): at a concave edge of the interface the apexes of the two pyramids over it coincide exactly (the 90 degree valley of apex height 0.5), which the tetrahedral engine rejected as a duplicate point. The coincident apexes now share one vertex and the pair of coinciding opposite-orientation sides over the concave edge, an interior face between two pyramids, is dropped from the frozen facets.
- **`plc_conform` returned Steiner points that no cell uses** (a quality point inserted outside the body, found at (9.7, 40.3, 10) for a block with y <= 30): they became nodes of no element. `conform` now drops every unused Steiner point and renumbers; the input vertices keep their indices (the join relies on that).

## Evidence (`tests/hybrid_region.rs`)

| body | hexahedra | pyramids | tetrahedra | checks |
|---|---|---|---|---|
| block 40 x 30 x 20 with a tilted R4 hole (native import, graded shell), cell 2.5 | 178 | 276 | 4055 | conforming (0 non-manifold, 0 mismatched, 0 hanging, one component), 0 inverted, element volume 22891.045740 = body volume (1e-9), radius-edge max 1.998 |
| cylinder R5 x 10 with a rounded top, cell 1.0 | 96 | 128 | 2172 | same checks, volume to 1e-9 |

**Linear patch test** (tilted-hole body): the whole boundary prescribed from a linear field, 664 interior nodes (hexahedron corners, apexes, tetrahedron nodes) follow it to 4.9e-17.

## Limits (recorded)

- The core is **interior** (never touches the shell): a core that reaches the surface (a hexahedral end of a part) needs the shell and the hexahedra to share their boundary ring, which is `hex_pyramid_tet`'s own contract and is not built here.
- Axis-aligned cubic cells only; no grading, no body-fitted hexahedra; the core's size is whatever fits.
- The tetrahedral recovery's own limits apply: the **cylinder with a cross bore** (spline-edged saddle intersection) failed after 127 s with "sub-facet circumcentre leaves its facet without encroaching a segment" (fixed in Phase 112, which also records the remaining limit: the saddle's small input angles need a looser radius-edge target); the shell must be graded with chart quality first (plain sampling leaves needles).
- Pinch removal handles edge and vertex pinches of the boundary but does not look for thin cavities inside the core.
- No Windows or hosted-CI evidence of its own.
