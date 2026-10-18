# Issue 46 — Phase 108: NAFEMS LE1 / LE10 accuracy references and cost-model calibration data (#60)

`issue-46-benchmarks.md` listed the independent accuracy references and the calibration problems as "research only, nothing run". This phase runs two of them on the existing kernel (no kernel change).

## LE1 elliptic membrane (`fea-core/tests/nafems.rs`)

Plane stress, thickness 0.1 m, E = 210 GPa, nu = 0.3, quarter model between the ellipses (2, 1) and (3.25, 2.75), outward pressure 10 MPa on the outer ellipse, symmetry on both axes; published target `sigma_yy` at D = (2, 0): **92.7 MPa**.

| mesh (radial x angular) | sigma_yy(D), nodal stress | error |
|---|---|---|
| Quad9 2 x 6 | 92.453 | -0.27 % |
| Quad9 4 x 12 | 94.049 | +1.45 % |
| Quad9 8 x 24 | 93.537 | +0.90 % |
| Quad9 16 x 48 | 93.003 | +0.33 % |
| Tri6 12 x 36 | 90.851 | -2.0 % |

The nodal (averaged) stress at a boundary corner converges slowly and not monotonically; the finest mesh is within 0.33 percent and the test asserts 1 percent, improvement over the 4 x 12 mesh, and 3 percent for Tri6.

## LE10 thick elliptic plate

Solid, thickness 0.6 m, uniform normal pressure 1 MPa on the upper surface, symmetry on both axes, the outer face held in x and y, `u_z = 0` along the mid-height line of the outer face; published target `sigma_yy` at D = (2, 0, 0.6) (upper surface, inner edge): **-5.38 MPa**. Hex27, radial x angular x layers:

| mesh | sigma_yy(D) | error |
|---|---|---|
| 2 x 6 x 2 | -5.493 | +2.1 % |
| 4 x 12 x 2 | -5.753 | +6.9 % |
| 6 x 18 x 4 | -5.560 | +3.3 % |
| 10 x 30 x 4 | -5.522 | +2.6 % |
| 14 x 42 x 6 | -5.452 | +1.3 % |

The test asserts 2 percent on the finest mesh and improvement over the coarse one.

**A wrong first reading is recorded because it is instructive:** my first LE10 model (outer face supported vertically only, D on the mid-plane) gave +-57 MPa at the surfaces and -0.49 MPa at D, ten times off and stable under refinement. The published specification (checked against the Seamplex Fino and FeenoX reproductions of the benchmark) clamps the outer face in x and y, fixes `u_z` only on its mid-height line, and puts D on the upper surface; with those conditions the kernel reproduces the published value. The kernel was right, my recollection of the benchmark was not.

## Cost-model calibration (#60, `fea-core/tests/cost_validation.rs`)

LE1 (Quad9) and LE10 (Hex27) were added as families. Deterministic gate: the predicted factor size against the solver's on the new meshes is within 0.6 to 5.6 percent (ratios 1.006 to 1.056; the test's band is 0.9 to 1.25). Timing (`--release --ignored`, one machine, macOS arm64, comparison not a gate): held-out same-family predicted/measured ratios 0.36 to 3.08 for LE1 and 0.45 to 1.95 for LE10 (small meshes are under-predicted, large ones over-predicted: the rates calibrated on the first mesh do not carry a constant factor to 2.5x the size); cross-family 0.52 to 12.9 (the worst, 12.9, is rates calibrated on the 2-D triangle hole mesh applied to a 3-D hexahedral one). The honest statement for #60 is therefore an **error band of roughly a factor of 3 within a family and a factor of 13 across dimensions**, not a point accuracy; the cost model remains a planning aid, and choosing between meshes by predicted time should only be trusted when the candidates are the same family and of comparable size.

## Open (unchanged)

- Mode I crack (`K_I = Y sigma sqrt(pi a)`): the kernel has no crack-tip element or J-integral, so it cannot be run; it needs one of them first.
- AMR time-to-accuracy gate (#62) and learned prediction (#61): not run here.
- The values for LE1 / LE10 are the published NAFEMS targets as quoted by the Seamplex, Abaqus and SimScale reproductions; the NAFEMS book itself was not read.
