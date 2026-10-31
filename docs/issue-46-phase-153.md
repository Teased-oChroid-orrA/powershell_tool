# Phase 153: surface-constrained fillet prototype

Issue #63, with NEMA17 deferred by owner request. This evaluates the existing `plc_conform::conform` route as an alternative to the lattice mesher's boundary snapping. It changes no production code or acceptance limit. OCCT retirement is not approved.

The input was the 12 × 14 × 16 mm box with every convex edge filleted by 2 mm, written by `NativeBrep` to STEP and tessellated by OCCT at 0.05 mm deflection. The welded shell had 392 vertices, 780 triangles, exact box bounds, and a minimum input angle of 2.13737°. The throwaway example passed its vertices and triangles to `plc_conform::conform` with a 20,000-Steiner-point limit, 300,000-cell limit, unprotected recovery, coplanar facet merging, 0.1 scaled-Jacobian refinement target, a 30-second quality stage, and a 90-second total callback limit. The example was removed after the run.

| Configuration | Recovery | Cells | Volume error | Remaining quality result |
|---|---:|---:|---:|---|
| No interior reconnect | 1.51 s | 9,350 | 1.8 × 10⁻¹⁵ | Five unresolved bad cells; `to_solver_mesh` refuses minimum scaled Jacobian 0.000 against 0.100. |
| Interior reconnect and three smoothing rounds | 3.30 s | 8,964 | 1.2 × 10⁻¹⁵ | Five unresolved bad cells; `to_solver_mesh` refuses minimum scaled Jacobian 0.066 against 0.100. |

Both runs returned 4,100 recovered subfacets and a maximum Steiner offset relative to the input shell of about 1.0 × 10⁻¹⁴. These are promising geometry and runtime results for this one shell. They do not establish CAD-face ownership after coplanar merging, loaded-face area, Tet10 accuracy, a static solve, general termination, or parity on other models. The unchanged solver-quality gate rejects both meshes. The native source generated the shape, but OCCT supplied this prototype's surface tessellation; a native-surface run is still needed.

The constrained route can avoid creating new boundary triangles that cross the input shell's seams, but it cannot replace the production route until boundary-adjacent slivers pass the default solver gate and parent facets map reliably to source faces. Next, identify the five remaining cells and test a local, topology-preserving boundary repair on a redistributable synthetic fillet shell. Require exact facet recovery, positive cells, unchanged shell volume, loaded-face area within the existing 0.5% bound, and the same OCCT/native solve and quality checks as the 19-model table. A failed or timed-out case must remain an explicit refusal.
