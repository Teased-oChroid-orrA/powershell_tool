# Issue 46 — Phase 86: flat-boundary flips remove the boundary slivers of anisotropic refinement (#58)

Phase 67 left "good quality at strong anisotropy" open with a worst cell of 0.0085 at 10:1. This phase measured where the bad cells are before changing anything (`metric_refine` ignored studies, 10:1 field across a layer in the unit cube):

- Distribution (default options, 40 passes): 1738 cells, worst **1e-15**, median 0.27, 569 cells below 0.1; the quality *falls* as passes increase (10 passes: 0.0085 / 32 cells below 0.1).
- **Every** bad cell touches the boundary (0 of 569 are interior-only; 480 have 3 or 4 boundary vertices) and they sit along the cube edges at the layer.
- Ablation: smoothing off, coarsening off, flips off: bisection alone (splits only) already produces 3278 cells below 0.1 and a worst cell of 4e-21; smoothing and coarsening help, but nothing repairs it because `tet_flip` never touches an edge lying on the boundary. The worst cells were flat tetrahedra with all four vertices in one cube face (volume 1.8e-11) next to a long crease edge.

Change: `tet_flip::FlipOptions::planar_boundary` (default off). An edge in a flat boundary patch (its two boundary faces exactly coplanar, forming a strictly convex quadrilateral, neither face protected) is replaced by the quadrilateral's other diagonal and the tetrahedra behind it are reconnected with it: a 2-2 flip for two tetrahedra, the boundary form of edge removal beyond (the link polygon is closed by the new diagonal and triangulated by the existing dynamic programme, now `triangulate_link`, shared with interior edge removal, whose behaviour is unchanged). All side tests are exact (`orientation`). The reconnection driver then checks the volume, that the boundary vertex set and total boundary area are unchanged, and that no protected face changed. It is off by default because the boundary triangulation changes: face names are not tracked here, so enable it only when coplanar boundary faces belong to one patch (the cube is; a plate whose top is split into two named faces is not).

Measured (macOS arm64, release, 40 passes):

| case | worst cell | median | cells below 0.1 |
|---|---|---|---|
| 10:1 default | 1.1e-15 | 0.27 | 569 of 1738 |
| 10:1 + flat-boundary flips | **2.8e-4** | **0.55** | **111 of 1139** |
| 4:1 default (converged) | 0.205 | 0.675 | 0 |
| 4:1 + flat-boundary flips (converged) | **0.352** | 0.707 | 0 |

Tests: a unit test of the 2-2 case (long diagonal replaced by the short one, quality up more than 5x, volume and area unchanged, locked by default, protected face keeps the edge, concave quadrilateral refused); a randomized test on ten point clouds (8 of 10 flipped cube-face diagonals; volume 1, positive orientation and the cube surface preserved); a fast 4:1 regression (worst cell 0.205 to 0.352) and an `--ignored` release 10:1 regression. `cargo test -p fea-core`: 524 passed, 0 failed; `cargo test -p solid-kernel --no-default-features`: 209 passed, 0 failed.

**Not delivered, honestly:** at 10:1 the refinement still does not converge (`converged` false at 40 passes; 120 passes gives 2394 cells, 302 below 0.1 and the in-band edge fraction falls to 0.23): the remaining bad cells (111, none interior-only) sit at the cube creases, where a face vertex close to a crease line forms a sliver with a long crease edge; creases are features and are not flipped. A crease-aware split (insert on the crease first), metric-Delaunay insertion or boundary-layer meshing is the next step; the planar-flip default stays off until it is wired to patch identity. #58 stays open.

Limits: one geometry (unit cube) and one family of fields; macOS arm64 only; no Windows or hosted-CI evidence.
