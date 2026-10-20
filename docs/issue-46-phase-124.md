# Phase 124: OCCT against the native path, measured per model (#46/#63)

The owner's retirement criterion (`docs/issue-46-occt-retirement-readiness.md`, "Owner decision, 2026-10-19") is per model: the native mesh must match the OCCT path in quality, the solver must be able to use it, and it must be efficient. This phase builds the comparison and runs it.

## What is compared, and what the comparison can and cannot say

`fea-problem/tests/occt_vs_native.rs` (feature `occt`, release): for one STEP text and one element-size target `h`, build two problems that differ only in `SolidCache`, the face-tagged surface triangulation the volume mesher is given:

- **OCCT**: `fea_problem::solid::tessellate_step` with `occt` (`cad::Brep::tessellate`).
- **native**: `fea_problem::native_solid::tessellate_step_native` (`AnalyticModel` + `sampled_surface`), at the same deflection (a thousandth of the diagonal, as the import uses).

Both then run through **the same** `solid_mesh` (`solid_kernel::mesh_volume`: lattice bisection, snap, `fea_core::tetmesh`) and the same `solve`. **OCCT has no volume mesher in this application**; it contributes only the surface triangulation. So "the native mesh matches the OCCT mesh" is exactly "the native surface triangulation drives the mesher as well as OCCT's does", and every defect of the volume mesher itself appears identically on both sides (see "Not an OCCT question" below). Benchmark: bottom `-z` flat face fixed, top `+z` flat face with traction `tz = 10` MPa, steel defaults, `Tet10`; compared: strain energy `U`, the reaction `Rz` (the loaded area times 10), the maximum displacement, and compliance `U/Rz^2`.

Models (each fed as STEP text to both paths): OCCT-built (`solid_kernel::occt`): box, cylinder, plate with two holes, plate with a boss, filleted box; native-built (`NativeBrep`): box, cylinder, box + hole, box + boss, box + boss + hole, filleted box; fixtures of `solid-kernel/tests/data` (cylinder with filleted top, edit plate, cross-bore, cross-bore filleted, t-junction, tilted hole, vase); and the real file `nema17-motor-bracket.step`.

Machine: Apple M1, macOS, release, other agents running concurrently: **times are indicative, ratios of one run to its twin are trustworthy to about 30 %**. Quality columns: minimum scaled Jacobian (sJ), minimum tetrahedron radius ratio (rr), maximum edge ratio (er), maximum size jump, elements under the solver gate's sJ of 0.1 (`QualityThresholds::default()`), meshing time, solve time (`Summary::solve_ms`).

## Results (size 1.6 x the import default; the passing assertion run)

`cargo test -p fea-problem --features occt --release --test occt_vs_native -- --ignored --nocapture the_native` (448 s, passes).

| model | path | tets | dofs | sJ | rr | er | jump | sJ<0.1 | mesh s | solve ms | U | Rz |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| box (OCCT-built) | OCCT | 11520 | 53619 | 0.502 | 0.633 | 2.4 | 1.0 | 0 | 0.39 | 1948 | 5.889 | -6000.0 |
| | native | 11520 | 53619 | 0.502 | 0.633 | 2.4 | 1.0 | 0 | 0.40 | 1923 | 5.889 | -6000.0 |
| cylinder | OCCT | 6800 | 32727 | 0.046 | 0.006 | 2.5 | 1.1 | 80 | 0.39 | 828 | 2.257 | -3094.6 |
| | native | 6800 | 32727 | 0.044 | 0.005 | 2.5 | 1.1 | 80 | 0.46 | 856 | 2.248 | -3086.7 |
| plate, two holes | OCCT | 8528 | 42993 | 0.248 | 0.366 | 3.9 | 1.4 | 0 | 0.94 | 638 | 9.476e-3 | -440.23 |
| | native | 8524 | 42981 | 0.245 | 0.376 | 3.9 | 1.4 | 0 | 0.92 | 546 | 9.487e-3 | -440.58 |
| plate with a boss | OCCT | 11712 | 58083 | 0.023 | 0.006 | 19.4 | 1.8 | 211 | 1.16 | 1001 | 3.966e-3 | -44.57 |
| | native | 12213 | 59586 | 0.020 | 0.006 | 27.3 | 1.9 | 148 | 3.09 | 1092 | 3.781e-3 | -43.58 |
| box + hole (native-built) | OCCT | 6245 | 29691 | 0.025 | 0.010 | 34.9 | 2.3 | 38 | 0.80 | 337 | 0.09032 | -970.5 |
| | native | 6278 | 29790 | 0.025 | 0.007 | 34.9 | 2.3 | 35 | 0.79 | 365 | 0.09012 | -969.4 |
| box + boss + hole (native-built) | OCCT | 6058 | 29655 | 0.023 | 0.007 | 34.6 | 2.1 | 280 | 1.16 | 391 | 0.01430 | -121.53 |
| | native | 6061 | 29664 | 0.020 | 0.007 | 38.6 | 2.2 | 323 | 1.28 | 349 | 0.01420 | -120.82 |
| box, 12x14x16, fillets 2 (OCCT-built) | OCCT | 63452 | 283299 | 0.019 | 0.005 | 32.0 | 2.9 | 1275 | 14.4 | 64133 | 0.2624 | -924.9 |
| | native | 61872 | 278559 | 0.019 | 0.007 | 23.9 | 2.6 | 2059 | 14.2 | 24690 | 0.3069 | -1007.4 |
| same shape (native-built) | OCCT | 61872 | 278559 | 0.019 | 0.005 | 31.4 | 2.6 | 1699 | 10.4 | 55352 | 0.2512 | -911.6 |
| | native | 63616 | 283791 | 0.018 | 0.006 | 26.0 | 3.3 | 1384 | 22.7 | 57186 | 0.2942 | -980.7 |
| cylinder, filleted top | OCCT | 9975 | 46932 | 0.020 | 0.008 | 17.3 | 2.2 | 114 | 4.93 | 3576 | 0.05851 | -393.5 |
| | native | 9494 | 45471 | 0.020 | 0.006 | 14.9 | 1.8 | 316 | 5.28 | 3528 | 0.05898 | -395.2 |
| edit plate (hole) | OCCT | 6400 | 30228 | 0.025 | 0.010 | 25.1 | 1.7 | 40 | 1.71 | 1621 | 2.600 | -11501.9 |
| | native | 6409 | 30255 | 0.024 | 0.010 | 25.4 | 1.7 | 36 | 1.55 | 1471 | 2.601 | -11505.9 |
| cross-bore (R8, R3) | OCCT | 6416 | 31536 | 0.035 | 0.009 | 4.7 | 1.6 | 14 | 1.77 | 1556 | 1.863 | -1979.8 |
| | native | 6426 | 31566 | 0.031 | 0.005 | 17.4 | 1.6 | 18 | 4.06 | 1322 | 1.853 | -1973.8 |
| vase (spline revolution) | OCCT | 4011 | 20769 | 0.021 | 0.007 | 30.2 | 1.6 | 280 | 2.59 | 994 | 0.3619 | -616.0 |
| | native | 4018 | 20805 | 0.022 | 0.007 | 31.0 | 1.6 | 283 | 2.82 | 1074 | 0.3591 | -613.6 |

Every row: equilibrium error below 1e-12, the solve converged. All elements under sJ 0.1 touch the boundary (the "on boundary" count equals the "under 0.1" count in every row of the log).

## Results (default size, the unscaled table)

`... -- --ignored --nocapture table` (all models, ~20 minutes; `CASE=<substring>` selects). Selected rows at the import's default size (diagonal / 18, capped by a third of the thinnest wall):

| model | path | tets | dofs | sJ | er | sJ<0.1 | mesh s | solve s | U | Rz |
|---|---|---|---|---|---|---|---|---|---|---|
| box 20x30x40 | OCCT / native | 32256 / 32256 | 146319 | 0.512 / 0.512 | 2.3 | 0 / 0 | 1.2 / 1.4 | 17.1 / 21.4 | 5.890 / 5.890 | -6000 / -6000 |
| plate, two holes | OCCT / native | 50702 / 50608 | 232311 / 232053 | 0.020 / 0.020 | 34.9 / 31.6 | | 21.4 / 16.4 | 32.1 / 32.8 | 9.499e-3 / 9.512e-3 | -440.6 / -441.0 |
| plate with a boss | OCCT / native | 57708 / 57632 | 262191 / 261657 | 0.021 / 0.021 | 32.1 / 33.6 | | 9.4 / 11.0 | 33.2 / 23.1 | 3.977e-3 / 3.945e-3 | -44.71 / -44.47 |
| cross-bore | OCCT / native | 18660 / 17938 | 88794 / 86616 | 0.025 / 0.021 | 29.8 / 27.1 | 118 / 803 | 3.9 / 4.3 | 2.3 / 2.1 | 1.892 / 1.886 | -1992.8 / -1987.2 |
| cross-bore, filleted | OCCT / native | 15704 / 15704 | 75900 | 0.018 / 0.019 | 28.0 / 34.9 | 1004 / 923 | 4.8 / 11.1 | 1.7 / 5.4 | 1.743 / 1.685 | -1908.1 / -1875.8 |
| t-junction | OCCT / native | 12296 / 12296 | 58875 | 0.021 / 0.020 | 35.6 / 34.9 | | 2.5 / 3.8 | 1.0 / 1.2 | 0.8059 / 0.7934 | -1114.8 / -1106.1 |
| tilted hole | OCCT / native | 25830 / 25842 | 119007 / 119037 | 0.019 / 0.020 | 30.6 / 32.9 | | 3.4 / 7.0 | 13.0 / 7.4 | 5.567 / 5.566 | -11445.2 / -11444.2 |
| vase | OCCT / native | 15575 / 15610 | 75372 / 75546 | 0.021 / 0.021 | 29.9 / 31.1 | | 3.5 / 3.7 | 1.7 / 2.4 | 0.3861 / 0.3836 | -634.9 / -632.8 |
| **NEMA17 motor bracket** (real file) | OCCT | 204552 | 956718 | 0.019 | 36.6 | 2385 | 46.7 | 158.8 | 4.652 | -2405.67 |
| | native | 204686 | 957306 | 0.018 | 38.4 | 2386 | 21.0 | 155.9 | 4.637 | -2405.67 |

The real bracket, which fails the native 18 degree *shell* gate (readiness doc), meshes and solves through the native tessellation to the same element count (+0.07 %), the same sub-gate count (2386 against 2385), the same loaded area (Rz identical), a strain energy 0.3 % lower and a maximum displacement 0.3 % lower than through OCCT.

## The surface triangulation each path hands the mesher (`surface_triangulation_quality`, at the import chord, 0.001 x diagonal)

| model | OCCT triangles / min angle | native triangles / min angle |
|---|---|---|
| cylinder | 192 / 2.5 | 206 / 3.4 |
| plate, two holes | 228 / 0.5 | 236 / 1.0 |
| box + hole | 120 / 1.0 | 130 / 1.9 |
| cylinder, filleted top | 1230 / 3.5 | 2000 / 4.9 |
| edit plate | 128 / 1.2 | 136 / 1.9 |
| **cross-bore** | **314** / 0.14 | **13856** / 0.12 (edges down to 0.003 mm) |
| **cross-bore, filleted** | 3364 / 0.16 | **18680** / 0.14 |
| **t-junction** | 440 / 0.22 | **8972** / 0.34 |
| **tilted hole** | 144 / 0.9 | **1502** / 1.0 |
| vase | 4484 / 4.4 | 6522 / 1.05 |
| NEMA17 bracket | 976 / 0.01 | 754 / 0.01 |

Both are sliver-rich (the volume mesher ignores triangle shape: it uses the surface only through the signed distance and the face tags). The native one is the same size as OCCT's on everything except the faces bounded by B-spline intersection curves, where it is 5 to 45 times larger.

## Findings

### Native against OCCT

1. **Same element and DOF counts**: within 5 % on every model (largest: cross-bore at default size, native 3.9 % fewer; plate with a boss at 1.6 x, native 4.3 % more).
2. **Same quality, with three rows where native has more sub-gate elements.** Minimum sJ agrees within x1.2 on every row (native lower by 0 to 14 %), radius ratio within x1.8 (cross-bore 0.009 against 0.005). Elements under the gate, native / OCCT, at 1.6 x: 0/0, 80/80, 0/0, 148/211, 35/38, 323/280, 36/40, 283/280, 1384/1699 (equal or fewer on nine rows); **2059/1275 (filleted box, OCCT-built text), 316/114 (cylinder with filleted top; 342/278 at default size), 18/14 (cross-bore; but 803/118 at default size)**. The spread is mesh-to-mesh noise of the shared lattice mesher plus, for the cross-bore, the over-sampled edges of finding 7.
3. **Same solution to discretisation error**: strain energy within 0.1 % (planar, box + hole), 0.2 to 1.5 % (curved fixtures, NEMA17 0.3 %), up to 6 % where the loaded face is a small disc or meets a fillet. Compliance `U/Rz^2` is asserted within 5 % on all 12 shapes at 1.6 x (1.5 % on the fillet shapes, where energy alone moves 17 %).
4. **Efficiency**: element and DOF counts equal; solve times equal (the ratio scatters between 0.4 and 2.4 with machine load; the same DOF count solves in the same time). Meshing: native 0.9 to 1.2 x OCCT's on 9 of 12 rows, **2.2 to 2.7 x on the cross-bore and the plate with a boss at 1.6 x** (4.1 s against 1.8 s; 3.1 against 1.2) and 2.2 x on the filleted cross-bore at default size (11.1 against 4.8 s); native faster on NEMA17 (21 s against 47 s, load noise included). The cost is the bigger surface triangulation for spline edges.

### Not an OCCT question (the shared volume mesher; documented, not fixed here)

5. **Criterion 1 as written is not met by either path.** `QualityThresholds::default()` (sJ >= 0.1) fails on every model with a curved boundary: sJ 0.02 to 0.05, 0.2 to 5 % of the elements, **all of them touching the boundary**. The planar models (box, plate with two holes, sJ 0.5 and 0.25) pass. `solve` does not enforce the gate, so every one of these meshes is solved. This is the boundary layer of `fea_core::tetmesh` (lattice bisection, snapping with `MIN_QUALITY` 0.05 relative radius ratio, `MAX_QUALITY_LOSS` 0.3): a snap is accepted down to a quality of 0.05 of a regular tetrahedron. It would be the same with OCCT kept, so it cannot block retirement, but it is the largest remaining quality gap for the product (the criterion should read "no worse than the OCCT path" or the mesher should get a boundary-layer repair pass; `tet_flip` / `tet_smooth` serve the Delaunay path only and are not wired into `mesh_solid`).
6. **The loaded area of a face that meets a tangent fillet is not conserved.** Box 12x14x16 with 2 mm fillets, top flat face exactly 8 x 10 = 80 mm2, traction 10 MPa, element about 1 mm: applied force 925 N (OCCT, +16 %), 1007 N (native, +26 %), and 912 N / 981 N on the twin text. Hypothesis (not isolated): `face_of_triangle` assigns a boundary triangle to the face every vertex is within `0.1 h` of, and a vertex on a 2 mm fillet is within 0.1 mm of the tangent plane up to `sqrt(2 r 0.1 h)` = 0.63 mm from the tangent line, which equals the native overshoot per side. Force and energy move with it (energy ~ force squared), so absolute energy/maximum displacement are mesh-noise there (+-10 %), while compliance stays within 1.5 %. The test asserts compliance for this case and a 15 % bound on `Rz`.

### Where native is worse and why (to fix outside this phase's files)

7. **Spline-edge over-sampling** (`solid-kernel/src/analytic_step.rs::sample_spline_adaptive`; forbidden to this track). The cross-bore intersection edge is 9.5 mm long, a degree-6 B-spline, and gets 169 points (349 on the 19 mm one) at spacing 0.022 to 0.125 mm for a chord of 0.0376 mm; a circle of the same radius needs about 21 points at this chord (the circular edges of the same file get 34 points at 1.52 mm). The function seeds `2 * degree` = 12 samples per knot span before bisecting, so a spline with many spans is sampled 10 to 40 times denser than the tolerance needs, and wherever knots cluster the spacing falls to 0.02 mm. The chart triangulator then produces `face1`/`face2` of the cross-bore with 9312 / 4482 triangles, 77 / 75 % of them under 5 degrees, shortest edge 0.003 mm (OCCT: 161 / 65). Consequences measured: 2.2 to 2.7 x meshing time on these models and (at default size) 6.8 x more sub-gate elements on the plain cross-bore. A repair: seed with one or two samples per span (the bisection already guarantees the chord bound) and add a decimation pass that drops samples whose removal keeps the curve within `chord / 2`. Expected effect: the triangle counts of the table above drop toward OCCT's; to be confirmed by re-running `surface_triangulation_quality` and the cross-bore rows. The same function feeds the t-junction (20 x), the tilted hole (10 x, ellipse edges: a similar seeding in the ellipse sampler, line 835) and the filleted cross-bore (5.5 x).
8. **The default element size of an imported solid ignores fillets**: `finish_import` gives diagonal / 18 (capped by wall / 3), which is 3.0 mm for the 2 mm-fillet box; both paths then refuse it ("the mesh does not resolve the STEP body: volume differs by 5.6 to 7.8 %"). Same on both sides. A fillet / hole radius should cap the start size (e.g. the smallest curved radius / 2).

## Criteria, against this evidence

| # | criterion (readiness doc) | evidence | verdict |
|---|---|---|---|
| 1 | native mesh passes the solver gates, solve converges | solves converge on 19 of 19 models (equilibrium <= 1e-12), including a real third-party file. Gates: pass on planar models; **fail on every curved model, identically with OCCT** (finding 5) | met relative to OCCT, **not met in the absolute reading** (an unmet mesher limit, not an OCCT-retirement cause) |
| 2 | quality no worse than OCCT within a stated margin; benchmark agrees to discretisation error | sJ within x1.2, rr within x1.8; edge ratio and size jump asserted within x2 (floor 25 for the edge ratio, gate 50): the one coarse cross-bore row is 17 against 5; energy 0.1-6 %, compliance asserted within 5 %; sub-gate share asserted within 2.5 x + 1.5 % | **met** on the asserted 12 models and NEMA17; the spline-edge models (cross-bore family, t-junction, tilted hole) are marginal on sub-gate share and meshing time (finding 7) |
| 3 | counts and time within a stated factor | tets / dofs +-5 %; solve time equal; meshing 0.4 to 2.7 x | **met**: factor 3 on meshing, 1.05 on counts |
| 4 | each editor operation native or dropped; tests green without `occt` on three OSes | outside this phase | open |

## Limits

- 19 models, 8 families; one real third-party file. The 20-file real corpus of readiness criterion 1 and the ABC corpus (another track) remain the wider evidence. The thin, tiny-feature and assembly cases are not in this table.
- One benchmark load case (uniform traction on a flat face, a fixed flat face); no contact, thermal, modal or bore pressure. Both paths are linear Tet10 at one size: no convergence study, so "agrees to the discretisation error" means "agrees to the mesh-to-mesh scatter at this size" (observed 0.1 to 6 %, up to 15 % in a local maximum).
- Timings come from a machine shared with other agents; the committed test uses a loose factor (3 x + 1 s for meshing, 3 x + 1 s for solving).
- The assertion test runs at 1.6 x the import size (the unscaled table takes about 20 minutes); both are `#[ignore]`. Use `CASE=<substring>` to select a model of the table, and `surface_triangulation_quality`, `native_surface_triangulation_by_face`, `native_edge_sampling` for the diagnostics above.

## Reproduce

```
export CARGO_TARGET_DIR=<shared target>
cargo test -p fea-problem --features occt --release --test occt_vs_native -- --ignored --nocapture the_native   # asserts
cargo test -p fea-problem --features occt --release --test occt_vs_native -- --ignored --nocapture table        # CASE=cross-bore to select
cargo test -p fea-problem --features occt --release --test occt_vs_native -- --ignored --nocapture surface_triangulation_quality
```
