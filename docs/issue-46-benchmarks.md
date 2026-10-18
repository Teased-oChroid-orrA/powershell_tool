# Issue 46: benchmark candidates for #60 / #61 / #62 (research record, 2026-10-17)

Purpose: the cost model (#60), the learned predictions (#61, which need held-out and out-of-distribution data) and the AMR-feedback time-to-accuracy gate (#62) can only be qualified against problems whose answer is known independently of this code. Phase 75 measured no gain on the only benchmark so far (the L-plate of `fea-core/tests/amr.rs`), so at least one class is needed where the starting-mesh advice demonstrably matters, and at least one control where it must not.

Status: **The Mode I edge crack has been run too (Phase 109: K_I within 0.13 to 0.66 % of the handbook, time-to-accuracy measured: no gain from the advice, gate not met). LE1 and LE10 have been run (Phase 108, `fea-core/tests/nafems.rs`: 92.7 MPa reached within 0.33 %, -5.38 MPa within 1.3 %) and added to the cost-model calibration; everything else below is research only and has not been run as a qualification.** Values marked *secondary* come from published verification reports that quote the NAFEMS originals; confirm them against the original before they become an assertion in a test. The built app stays offline: these are build/research-time inputs, never runtime dependencies.

## A. Closed-form classes (reference value exact; no licence issue; commit as tests)

| Class | Reference | Why it discriminates | Already in repo |
|---|---|---|---|
| Plate with a circular hole, uniaxial tension (Kirsch) | stress concentration factor 3 at the hole edge; field `kirsch` in `tests/classic.rs` | Demand is localised at the hole edge, far from the default uniform start: the case where regional advice should pay | `tests/classic.rs` (Kt only; no AMR run) |
| Thick-walled cylinder / pressurised sphere (Lamé) | exact radial/hoop stress | Smooth field: advice must give **no** gain (negative control); also the cost-model baseline | `tests/classic.rs` |
| Mode I crack (centre or edge crack in a plate) | `K_I = Y sigma sqrt(pi a)` with a tabulated finite-width correction `Y` (e.g. edge crack, `Y` ~ 1.12 for a short crack) | `r^-1/2` stress singularity: refinement demand concentrates at the tip, the strongest test of goal-oriented advice | no |
| L-shaped domain (re-entrant corner) | Laplace: `u = r^(2/3) sin(2/3 (theta + pi/2))`; elasticity: Williams exponent | Singular corner; this is the benchmark that showed **no** gain (Phase 75), keep as the documented negative | `tests/amr.rs` |
| 3-D Fichera corner | `u = (x1^2 + x2^2 + x3^2)^(1/4)` on `(-1,1)^3 \ [0,1]^3` (scalar problem, manufactured right-hand side `f = -3/4 r^(-3/2)`... check against the source before use) | 3-D singular point; exercises the tetrahedral path and the cost model's DOF growth | no |
| Hertz line/sphere contact | closed-form contact half-width and pressure | Localised contact; relevant to the contact AMR that is deliberately not exposed | `tests/classic.rs` |

## B. Published numerical benchmarks (NAFEMS, linear static; *secondary* values)

Quoted by the ESRD benchmarks guide, the DIANA verification manual and the HOOPS Solve benchmark report; the original is the NAFEMS "Standard NAFEMS Benchmarks" (Linear Static Benchmarks Volume 1), which is copyrighted: quote the numbers and describe the geometry, do not copy the document.

- **LE1 elliptic membrane** (plane stress, thickness 0.1 m, 10 MPa outward pressure on the outer ellipse): target `sigma_yy` at point D = **92.7 MPa**.
- **LE10 thick elliptic plate** (3-D, 0.6 m thick, 1 MPa pressure, E = 210 GPa, nu = 0.3, quarter model): target `sigma_yy` at D = **-5.38 MPa**.
- **LE6 skew plate** (shell): maximum first principal stress on the lower surface at mid-plate = **0.802 MPa**.
- **LE5 Z-section cantilever** (torque 1.2 MN m): target is the axial stress 2.5 m from the fixed end; the value was not found in the sources read, so it is *unknown* here. Shell-based; not applicable until the kernel has shell elements.

Fit: LE1 and LE10 are smooth-ish with a stress-recovery target, good cost-model calibration points (known answer, moderate size, 2-D and 3-D). They are not expected to show an AMR-advice gain.

## C. CAD / mesh corpora (no reference answer; for robustness, cost data and held-out splits)

| Corpus | Licence (as reported) | Use | Do not |
|---|---|---|---|
| ABC (1 M Onshape models, STEP/Parasolid) | dataset MIT; model copyright stays with the creators (Onshape terms); processing pipeline GPL | Native-import and shell-quality statistics, the registry's angle-gate calibration (item below), held-out split source | commit models; assume the MIT grant covers every model |
| Thingi10K (10 k 3-D-printing meshes) | per file: roughly 3.1 k CC0/CC-BY/PD/BSD, 4 k share-alike/GPL/ND, 2.9 k non-commercial (one third-party count) | Second, independent source for out-of-distribution tests of learned predictors | redistribute; use the NC subset in anything shipped |
| Tetrahedral Meshing in the Wild dataset (10 k meshes from Thingi10K) | research use, cite the paper; fTetWild code MPL-2.0; per-file source licences still apply | Reference tet meshes (element counts, quality) to compare cost predictions | treat as ground truth for accuracy: it has no solution field |
| NIST PMI CAD test models | to verify (US-government, expected public domain) | STEP with product-manufacturing information; healing and face-tag tests | not verified |
| Fusion 360 Gallery | to verify | B-rep reconstruction set | not verified |
| NEMA17 bracket (`nema17-motor-bracket.step`) | unknown provenance | the existing worked example only | use as evidence of generality |

None of these has a solution field, so none can qualify accuracy; they qualify robustness (does the pipeline finish, at what cost) and provide the held-out distribution that #61 requires. Accuracy comes from sections A and B.

## D. How these map to the open gates

1. **#62 time-to-accuracy gate.** Run goal-oriented AMR with and without `adaptivity::feedback` advice on: Kirsch hole (expected gain), Mode I crack (expected larger gain), Lamé cylinder (expected none, control), L-plate (recorded none). Gate = unknowns or wall time to reach a fixed relative error in the closed-form functional, over a sweep of tolerances, identical solver settings. A gain on the hole/crack classes and none on the controls is the qualifying outcome; no gain on any class means the advice stays unqualified (do not change `run_amr`).
2. **#60 cost model.** Fit/validate predicted vs measured assemble+solve time and memory on LE1, LE10, Lamé, Kirsch at three refinement levels each; report the prediction error band, not a single number.
3. **#61 learning.** Train on one family (e.g. closed-form classes and ABC), test on a disjoint one (Thingi10K, NEMA17); a held-out gain over the deterministic cost model is the entry condition. Until then learned predictions stay off. Status 2026-10-18 (Phase 113): met for direct linear static solves on one machine with solver-generated corpora (no ABC / Thingi10K); production stays off.
4. **Angle gate (`NATIVE_MIN_SHELL_ANGLE_DEG` = 18).** Do not relax it on the bracket alone. To judge whether it is too conservative, sample shells from ABC and Thingi10K (permissive-licence subset), compute each shell's smallest angle, and record how many meet and miss 18 degrees and whether `plc_conform` terminates on those between, say, 5 and 18 degrees. Only a measured termination boundary justifies a different constant.

## Provenance

Search date 2026-10-17. Sources: [ESRD NAFEMS linear elastic benchmarks guide](https://www.esrd.com/wp-content/uploads/dlm_uploads/Benchmarks-Guide-Standard-NAFEMS-Benchmarks-Linear-Elastic-Tests.pdf), [DIANA NAFEMS verification](https://manuals.dianafea.com/d105/Verify/Verifyse5.html), [HOOPS Solve NAFEMS report](https://docs.techsoft3d.com/hoops/mesh/_static/benchmark_reports/benchmark_results_2.13.0.pdf), [NAFEMS Linear Static Benchmarks Vol. 1](https://www.nafems.org/publications/resource_center/p07/), [FeenoX LE10 example](https://www.seamplex.com/feenox/examples/mechanical.html), [Learning robust marking policies for AMR](https://arxiv.org/html/2207.06339v2) (L-shape and Fichera exact solutions), [ABC dataset](https://ar5iv.labs.arxiv.org/html/1812.06216), [fTetWild / Tetrahedral Meshing in the Wild](https://github.com/wildmeshing/fTetWild), [Courant geometric datasets](https://cims.nyu.edu/gcl/datasets.html). Licence statements for ABC and Thingi10K are as reported by those pages and by one third-party review; read the primary terms before any redistribution.
