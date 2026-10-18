# Issue 46 — Phase 109: Mode I edge crack (K_I accuracy and AMR time-to-accuracy) and a node-set goal defect (#60/#62)

`issue-46-benchmarks.md` section A lists the Mode I crack as the singular class where goal-oriented refinement should pay most, and Phase 108 recorded that the kernel could not run it ("no crack-tip element or J-integral"). It can: the upper half of an edge-cracked plate needs no crack-face node duplication (the crack face is a free boundary, the ligament carries the symmetry condition `u_y = 0`), and `K_I` can be read from the crack-face opening. `fea-core/tests/mode_i_crack.rs`.

## K_I against the handbook

Plate width W = 1, half height 2, uniform tension 1000, E = 1e7, nu = 0.3 (plane stress), Tri6 meshes graded toward the tip. `K_I` from `u_y(r, pi) = K_I (kappa + 1) / (2 mu) sqrt(r / 2 pi)`, extrapolated linearly to `r = 0` over `r` in `[0.02, 0.25] a`. Handbook: `K_I = Y sigma sqrt(pi a)`, `Y = 1.12 - 0.231 l + 10.55 l^2 - 21.72 l^3 + 30.39 l^4`, `l = a/W` (about 0.5 percent).

| a/W | tip element a/25 | a/100 | a/400 | handbook |
|---|---|---|---|---|
| 0.2 | -5.35 % | -1.99 % | -0.66 % | 1086.5 |
| 0.3 | -4.59 % | -1.66 % | -0.48 % | 1611.5 |
| 0.4 | -4.92 % | -1.20 % | -0.13 % | 2358.0 |

The error decreases monotonically with the tip size (asserted) and the finest mesh is within 1 percent (asserted; 0.13 to 0.66 percent measured), inside the handbook's own accuracy. This is the Mode I accuracy reference.

## Time to accuracy with and without advice (#62), singular class

Goal: the opening of the crack mouth (the displacement of one node). Reference: a tip element of 0.0008 on a 0.04 far field (CMOD 1.40638e-4; it varies by 0.05 percent over six tip/far-field combinations). The default start is a uniform 0.2 mesh on a crack of length 0.3.

| relative tolerance | default | advised start | finished-mesh oracle |
|---|---|---|---|
| 2e-3 | 6 cycles, 2840 dofs, 0.65 s, goal error 4.0e-7 | identical | 1 cycle, 2840 dofs, 0.12 s (ratio 0.19) |
| 5e-4 | 8 cycles, 5362 dofs, 1.09 s, goal error 4.8e-8 | identical | 1 cycle, 5362 dofs, 0.24 s (ratio 0.22) |

- The estimator's claim holds against the reference: the true goal error is 1.4 and 0.7 times the requested tolerance (asserted below 5).
- **The advice gives no gain here either.** The three-run history advises a refinement of only 1.2 to 1.6 percent (`GeometricDiscontinuity` x0.988, `Smooth` x0.984); applied near the tip it leaves the mesh and the run unchanged. The perfect-oracle start from the finished mesh would be about 5 times faster, so the headroom exists but the shipped advice does not capture it, as for the hole in Phase 80 and the L-plate in Phase 75. **The #62 time-to-accuracy gate is not met**; the advice depth (not the estimator) is the limiting factor.

## Defect found and fixed: node-set goals could not run on the enriched dual

`GoalSpec::mean_displacement` and `reaction` failed with `no node set 'crack'` as soon as the enriched DWR estimator ran: `adaptivity::enrich::refine` built the refined mesh without the node sets, and the goal is rebuilt on it. The coarse nodes keep their indices on the fine mesh, so the sets now carry over unchanged (`enrich.rs`) and the goal means the same nodes on both meshes. (Element-based goals, the only kind the tests used, were unaffected.)

## Lesson recorded: a mean over nodes is not a mesh-independent goal

The first goal here was the mean opening over the crack-face nodes. It does not converge (7.6e-5, 6.5e-5, 5.8e-5, 5.5e-5 as the tip element shrinks) because a finer graded tip adds nodes where the opening is small; the AMR run then "converged" 7.7 percent from the reference. A point value (the crack mouth) is mesh independent. `GoalSpec::mean_displacement` over a node set is only meaningful when the set is the same nodes on every mesh (a single node, or sets that do not grow under refinement).

## Open

- AMR time-to-accuracy gate (#62): not met (advice captures none of the oracle headroom); learned prediction (#61) not run.
- The crack is straight and the extraction is by crack-face opening; there is no J-integral, no mixed mode.
