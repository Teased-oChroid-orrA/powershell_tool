# Issue 46 — Phase 97: the size-field start corrected, and why only a stored mesh beats the default (#62)

Phase 93 left the size-field oracle over-refining (10988 dofs against 7588). The cause was a unit error in the test, not in the advice: the meshers are asked for a *size*, and a simplex edge comes out at about 0.87 of it (the controller's own `element_sizes` convention), so asking for the measured mean edge length makes the mesh about 1.4 times denser. With the size divided by 0.87 (`SIMPLEX_EDGE_PER_SIZE` in `tests/amr_advice_classes.rs`):

| tolerance | default | size-field start (corrected) | time ratio |
|---|---|---|---|
| 5e-4 | 2 cycles, 1794 dofs, 0.17 s | 1 cycle, 1994 dofs, 0.12 s | **0.67** |
| 1e-4 | 4 cycles, 7588 dofs, 0.70 s | 1 cycle, 8314 dofs, 0.82 s | **1.17** |

Now the dofs are right (+10 %) and the remaining cost is visible: re-meshing from a size field at the final density costs about as much as the cycles it saves (the default run also pays a re-mesh in every cycle, and the last one dominates), so the size-field start is a loss at four cycles. Starting from the finished mesh itself (Phase 93: 0.49x at four cycles, 0.61x at six) is the only start that wins, because it skips both the re-meshing and the early solves. Consequence for #62: the usable form of "advice" is not a size multiplier or a size field but **reuse of the stored adapted mesh of the same workload key** (same geometry, material, boundary conditions, quantities of interest; loads may differ), verified by one adaptive cycle that either accepts it or continues. The key and the exact-equality rule already exist (`WorkloadKey`); a mesh store is a persistence feature in `fea-problem` (the verified telemetry store is the model for its integrity rules) and a product decision about what to keep on disk; it is not built here. The gate "advice reduces time-to-accuracy on representative classes" is therefore met only by that route, whose measured ceiling is 0.49-0.61x.
