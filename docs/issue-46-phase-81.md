# Issue 46 — Phase 81: an upper bound on what starting-mesh advice can gain (#62)

Phase 80 found no gain from the advice on a plate with a hole. This phase asks the bounding question: what would a *perfect* advice gain? `an_oracle_start_bounds_what_any_advice_could_gain` (`fea-core/tests/amr_advice_classes.rs`) starts AMR from the mesh the default run ended on (size field = mean edge length of the nearest final element) and reruns to the same tolerance.

Measured (macOS arm64, release; same plate, goal and reference as Phase 80):

| tolerance (relative) | default start | oracle start | time ratio |
|---|---|---|---|
| 5e-4 | 2 cycles, 1794 dofs, 0.18 s | 1 cycle, 2558 dofs, 0.14 s | 0.79 |
| 1e-4 | 4 cycles, 7588 dofs, 0.71 s | 1 cycle, 10988 dofs, 1.11 s | 1.55 |

Reading: the oracle needs one cycle, but re-meshing from a size field over-resolves (10988 dofs against 7588), and one solve plus estimate on the final-size mesh costs as much as the whole geometric sequence that led to it. The cycles an advice could skip are the cheap early ones, so the bound on the saving is small (best case 21 % here) and can be negative. A size-multiplier advice (at most the oracle) therefore cannot show a material time-to-accuracy gain on a class whose AMR converges in a few cycles from a coarse start.

Consequence for #62: the exit criterion "improved initial mesh recommendations reduce time-to-accuracy on representative benchmark classes" is not demonstrable on the classes measured (L-plate, plate with a hole), and by this bound cannot be on any class with a short cycle sequence. Candidates that could still matter: classes needing many cycles (strong singularities, many features), where the early cycles are a larger share; and using the feedback for what it predicts well (expected cycles, dof growth, seconds, memory: budget and cost planning, #60) rather than for the start size. #62 stays open; nothing in `run_amr` or `feedback` changes. The Mode I crack class is not run: it needs a crack-tip geometry the 2-D mesher does not provide, and the bound above applies to it as well unless its cycle count is large.

Limits: one class; the oracle's size field is a crude nearest-centroid edge length; no Windows or hosted-CI evidence.
