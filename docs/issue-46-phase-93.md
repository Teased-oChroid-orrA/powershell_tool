# Issue 46 — Phase 93: the exact upper bound on starting-mesh advice (#62)

Phase 81 bounded the gain of any advice with an oracle that re-meshed from the final mesh's size field. That oracle over-refined (10988 dofs against 7588; 47082 against 32812 at six cycles) and so was slower than the default, which made the bound look worse than it is. The sharp oracle is the finished mesh itself, handed to the controller as its starting problem (`starting_from_the_finished_mesh_itself_is_the_best_any_advice_could_do` in `tests/amr_advice_classes.rs`): one solve and estimate on it, then convergence.

Measured (plate with a hole, macOS arm64, release):

| tolerance (relative) | default | from the finished mesh | time ratio |
|---|---|---|---|
| 1e-4 | 4 cycles, 7588 dofs, 0.72 s | 1 cycle, 7588 dofs, 0.35 s | **0.49** |
| 5e-6 | 6 cycles, 32812 dofs, 3.60 s | 1 cycle, 32812 dofs, 2.20 s | **0.61** |

So a *perfect* advice would save 39-51 % of the time on these classes (more cycles, smaller share: the last solve dominates), not the 21 % of Phase 81 and not a loss. The advice actually produced by `adaptivity::feedback` (zone size multipliers of about 0.9, applied through a size function) realised about 3 %. The gap is the whole question: a useful advice would have to predict where the final mesh ends up closely enough to start within one refinement of it, which a multiplier per zone applied to a coarse uniform start cannot do; and when a size field is the vehicle, the re-meshing itself over-refines. Consequences for #62: the exit criterion is still not met and is not ruled out in principle (the exact oracle shows half the time is on the table); what would be needed is advice that carries the final element sizes (a stored size field per workload key) and a mesh-to-mesh start that does not re-mesh from it. Not built here: it would be a new feature whose gain is bounded by 0.49-0.61 and whose cost is a per-class store; that is a product decision.

The earlier "oracle start" test (size-field start) stays as the measurement of that route; its tolerances are 5e-4 and 1e-4 (at four and six cycles it is 1.5x and 4.3x slower than the default).
