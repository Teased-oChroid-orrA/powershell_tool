# `fea-core`: general finite-element kernel

Decision record: `docs/adr/ADR-012-general-fea-kernel.md`. Crate contracts and pitfalls: `fea-core/AGENTS.md`.
Plan origin: turn the special-purpose lug solver into a general 2D/3D FEA with accuracy and speed first.

## Phase status

| Phase | Scope | Status |
|---|---|---|
| 0 | Speed baseline of the existing lug solver (`lug-solver/tests/bench.rs`) | **Done** |
| 1 | Elements (10 types), kernels, symmetric assembly, faer solve, loads, BCs by elimination, nodal stress, structured generators, boundary-face extraction, `.vtu` writer, position-dependent face loads, SPR + ZZ error estimate, MPC / reference-point rigid coupling, PCG + smoothed-aggregation AMG, anisotropic elasticity | **Done**. Deferred with reason: GEMM-tiled stiffness kernel and `pulp` SIMD (assembly is < 1 % of a 3D linear solve; revisit with the nonlinear Newton loop in Phase 3, where assembly repeats every iteration) |
| 2 | General geometry + 2D mesher, adaptive remeshing, extrude/revolve, Gmsh/Abaqus import | **Done**. (The lug star O-grid stays in `lug-solver` until Phase 5 moves it behind a generic `Mesh`.) |
| 3 | Material nonlinearity (small- and finite-strain J2, analytic consistent tangent, Newton driver) | **Done**. The `Hardening` law moved to `mechanics-core` (single copy, `lug-solver` re-exports it); GEMM/SIMD kernel measured and rejected (see Phase 3) |
| 4 | General contact (Gauss-point-to-surface, AL + Coulomb friction, BVH search) | **Done** (rigid analytic and deformable masters, 2D and 3D, normal + friction; see limits) |
| 5 | `lug-solver` bridge on `fea-core` (`lug-solver/src/fea.rs`), force-controlled pin, plastic / finite-strain collapse, meshed elastic pin, 3D double-shear lug, retire duplicated solvers | **Done**: the kernel reproduces the condensed solver (see Phase 5). Retiring `lug-solver`'s own sparse/finite solvers is **declined by the speed gate** (kernel 40-300x slower on the lug); single shear is out of scope |
| 6 | Docs/ADR upkeep, `app-tui` FEA toolbox (separate decision) | Docs done; the toolbox was built in Phase 8 |
| 7 | The kernel becomes the main lug solver: bushing, thermal fit, second order, oblique / hardening / finite-strain collapse, sweeps, elastic pin on the full model, speed, mesh settings and mesh-size test in the toolbox | **Done** (see Phase 7) |
| 8 | The FEA Workbench (`fea-problem` + `app-tui`), toolboxes moved onto the kernel (Bushing's plane-stress edge FE, Preload's member compliance), kernel additions (point location, multi-case solve, rigid-body check) | **Done** (see Phase 8) |
| 9, 10 | Eccentric-bushing upgrades; random corpora, shared adaptive loop | **Done** (see Phase 9, 10) |
| 11 | Adaptive solver programme: baseline, capability matrix, diagnostics, validation, verified strategy selection, modal / buckling / transient, 3D partial slip, parallel factorisation | **Done** (see Phase 11; each increment below with its proof) |

## Phase 0 baseline (existing lug solver, release build, Apple M1, 8 threads)

| Case | Time |
|---|---|
| build half model (condense) | 39 ms |
| solve axial case (Newton 9 iterations) | 12 ms |
| build full model | 322 ms |
| solve oblique 45 deg (Newton 17) | 56 ms |
| small-strain limit load (9485 lbf) | 125 ms |
| finite-strain collapse, EPP, 32 around (9842 lbf, 46 factorisations) | 506 ms |
| Q9 `element_stiffness` (`edge-check`, dense `B^T D B`) | 1.85 us / element |

Reproduce: `cargo test -p lug-solver --release --test bench -- --ignored --nocapture` (use `rtk proxy` to see stderr).
Every later optimisation is measured against these.

## Phase 1: method

* **Elements**: Tri3, Tri6, Quad4, Quad8, Quad9, Tet4, Tet10, Hex8, Hex20, Hex27 in VTK node order. Shape functions and
  derivatives are generated from the node coordinates (tensor Lagrange, serendipity, barycentric) and **tabulated once per
  type at the Gauss points** (`element.rs`); the kernels never evaluate a polynomial.
* **Stiffness**: node-pair blocks `K_ab = sum_g w [lambda g_a (x) g_b + mu (g_b (x) g_a + (g_a . g_b) I)]` (no `B`, no `B^T D B`,
  half the pairs by symmetry), identical code for 2D and 3D. Axisymmetric uses the 4x4 form with the hoop term. A second,
  component-major layout (`stiffness_fast`) turns every update into a rank-1 outer product over contiguous rows; it equals the
  node-pair kernel to 1e-12 (unit test) and is 0-15 % faster, which does not matter (below).
* **Assembly**: lower block triangle in compressed-column form; pattern and element-to-block scatter map built once; elements
  greedily coloured so a colour class writes disjoint blocks, run with rayon, bit-reproducible.
* **Constraints**: eliminated (reduced matrix gathered through a precomputed source-index map; prescribed non-zero values by one
  symmetric product on the right-hand side).
* **Solve**: faer supernodal Cholesky. Symbolic analysis once per constraint set; numeric refactor only refills values. Ordering:
  AMD, or geometric nested dissection on the node graph (`ordering.rs`); `Ordering::Auto` analyses both in 3D and keeps the
  sparser factor.
* **Loads**: point, edge/face traction and pressure (Gauss integration on Line2/3 and Tri/Quad faces), body force, uniform
  temperature change (consistent thermal load); thickness and `2 pi r` weights folded in.
* **Recovery**: stress at Gauss points, and at nodes (element stress evaluated at its nodes, averaged over sharing elements).

## Phase 1: validation (all in `cargo test -p fea-core`)

| Check | Result |
|---|---|
| Shape functions: Kronecker at nodes, partition of unity, linear completeness, derivatives vs finite differences, quadrature exactness, weights = reference measure | all 10 element types |
| Element matrix: symmetry, rigid translations/rotations carry no force, constant-strain field gives exact strain and energy `1/2 u^T K u = int 1/2 sigma:eps`, inverted element rejected | all 10 types, plane stress/strain, skewed curved geometry |
| Fast kernel equals node-pair kernel | all 10 types, 1e-12 |
| **Patch test**: boundary displaced to a linear field, interior exact | all 10 types, 1e-9 of the field scale |
| **Lame thick cylinder** (internal pressure, `eps_zz = 0`), quarter model: plane strain | Quad9 3.4e-5, Quad8 3.9e-5, Tri6 6.9e-4 max `u_r` error at 6x6; hoop stress within 0.7 % |
| same, **axisymmetric** (exercises the hoop-strain kernel) | Quad9/Quad8 6.5e-6, Tri6 2.5e-4 at 8 divisions |
| same, **3D extruded** Hex27/Hex20 5x5x1 | 7.0e-5 / 7.9e-5; Hex8 3.8e-3 at 12x12x1 |
| same with **tetrahedra**, exact boundary displacement | Tet10 1.9e-3 (4x4x1), Tet4 6.2e-3 (10x10x1) |
| **Convergence rate** (nodal `u_r`, 2/4/8 per side) | Quad9 2.7e-7 / 2.0e-8 / 1.3e-9 (x13, x15), Quad8 x13/x15, Tri6 x7/x8, Quad4 x3.8/x4.0, Tri3 x2.4/x2.9 (pre-asymptotic) |
| Thermal: fully constrained cube gives `-E alpha dT / (1 - 2 nu)` exactly (Hex8/20/27, Tet4/10); free expansion stress-free, `u = alpha dT x` (plane strain `(1 + nu) alpha dT x`) | 1e-9 |
| Reactions: bar under end traction plus self weight, reaction sum and tip deflection | 1e-5 (Hex8, `x^2` terms missing) to roundoff |

## Phase 1: remaining capabilities and their validation

| Capability | Method | Validation (all in `cargo test -p fea-core`) |
|---|---|---|
| Boundary faces (`topology.rs`) | faces owned by one element, corner-face tables per family, midside / face-centre nodes found by natural-coordinate geometry; any element mix, material interfaces are interior | outward area vectors point out and close to zero on every type; face node counts per order; pressure on a selected face integrates to `p A` on all 3D types |
| Position-dependent face load (`FaceField`) | force per area as `f(position, outward normal)` integrated at the face Gauss points | drives the Timoshenko and Kirsch benchmarks below |
| `.vtu` writer (`vtu.rs`) | pure `fn -> String`, VTK node order, point/cell fields, 2D padded to 3D | round-trips connectivity, offsets, VTK type ids, rejects wrong-length fields |
| **Timoshenko cantilever** (exact elasticity solution, parabolic end shear) | exact displacement on the clamped end, exact traction on the free end | Quad9 / Quad8 1.2e-4 / 9e-5 max displacement error (cubic `x^3` term not representable), Tri6 6.5e-4; Quad4 converges (1.1e-1 -> 7.7e-3 at 4x refinement) |
| **Kirsch plate with a hole** (exact Kirsch traction on the outer boundary) | graded quarter model, symmetry | hole hoop error and Gauss-point stress converge with refinement (Quad9 hole hoop error 2.6e-2 -> 8.0e-3 -> 4.5e-3 of 3 sigma at 24x32 elements = 6.4k dofs); pole stress 2.987 sigma (exact 3). The near-hole ring is pre-asymptotic (high gradients); away from it (r > 2.5 a) the Gauss error converges x4 per halving. A grading exponent of 1.6 toward the hole made the Jacobian vanish at the hole edge and stalled convergence (use <= 1.2) |
| **Pressurised hollow sphere** (axisymmetric) | Lame, `u_r` and stress | `u_r` O(h^4) (Quad9 x15/halving); node-sampled stress O(h^2); hoop/radial errors 4e-3 / 7e-3 at 16x32 |
| SPR recovery + ZZ estimate (`recover.rs`) | patch polynomial (degree 1 linear / 2 quadratic elements) fitted to Gauss stresses; boundary nodes take the nearest interior neighbour's polynomial; fit needs `nt + 2` samples; midside/face/interior nodes average the owning corners' polynomials; ZZ = energy norm of recovered - FE stress | **exact** for a linear stress field on all 10 element types (corners and boundaries included, 1e-8 relative); sphere: SPR nodal RMS error falls x5.7-7 per halving (superconvergent) and 3-6x below nodal averaging, ZZ effectivity 0.98-1.04 at the finest mesh (Quad9, Quad8, Tri6); Kirsch: SPR RMS 6x below averaging at 25k dofs, effectivity 1.13 |
| MPC and reference-point rigid coupling (`mpc.rs`) | `u = T u_r + u_g`, `K_r = T^T K T`, chains resolved by substitution, extra dofs for reference translation and rotation (`u_s = u_ref + theta x r`), one-shot faer Cholesky of the reduced system | pure bending through a rigid tip is **exact** (3D Hex27 and 2D Quad9, `nu = 0`: rotation and tip deflection to 1e-9, base moment balance), axial stretch exact, transverse tip force within 0.7 % of Timoshenko beam; general equations with chains and offsets satisfied to 1e-14 and virtual-work stationary; circular chains, prescribed slaves, double slaves and singular reference points rejected |
| Iterative solve (`amg.rs`, `sparse.rs`) | PCG with a smoothed-aggregation AMG V-cycle: node aggregation, per-aggregate orthonormalisation of the rigid-body modes (QR tolerates rank-deficient aggregates), damped-Jacobi-smoothed prolongator, 2 symmetric Gauss-Seidel sweeps, faer Cholesky on the coarsest level; `SolveMethod::Auto` switches at 150k free dofs in 3D | equals the direct solution to 1e-11 on Hex8/20/27, Tet4/10, plane and axisymmetric models; prescribed non-zero displacements and reactions survive; iteration counts mesh-independent (Hex8 12-16 from 3.6k to 45k dofs); non-convergence is an error |
| Anisotropic elasticity (`Elastic::anisotropic` / `orthotropic` / `rotated_z`) | `6 x 6` D (library stress order) with compliance; plane stress condensed on `sigma_zz = 0`; axisymmetric axes `(r, z, theta)`; B-matrix stiffness path; isotropic `alpha dT` thermal strain | isotropic law through the aniso path equals the isotropic kernel to 1e-12 (stiffness, thermal load, stress; all 10 types, all analysis types); rotation algebra (90 degrees swaps axes, round trip, isotropic invariance, directional modulus `1/E(phi)`); orthotropic patch tests exact; free thermal expansion stress-free and constrained stress `-D alpha dT`; pure bending with zero Poisson coupling exact; rotating mesh and material together leaves energy and displacements invariant (a sign flip in the rotation is caught) |

### Iterative solver: measured (release, Apple M1, 8 threads)

Hex8 block, clamped base, traction on the top (`cargo test -p fea-core --release --test bench -- --ignored --nocapture iterative_against_direct`):

| Mesh | free dofs | PCG + AMG | faer Cholesky (nested dissection) | speed-up |
|---|---|---|---|---|
| 24^3 | 45,000 | 0.74 s (22 its) | 1.6 s | 2.2x |
| 32^3 | 104,544 | 1.65 s | 6.7 s | 4.1x |
| 40^3 | 201,720 | 3.1 s (20 its) | 25.1 s | 8.1x |

Hex20 (quadratic) needs ~2x the iterations (33-35) and is break-even with the direct solver at 23k dofs (1.15 s vs 0.95 s); 76k-dof Hex20 solves in 4.4 s where a factorization needs tens of seconds and gigabytes. A strength-of-connection threshold lowers iterations further but raises setup (coarsening stalls into a large direct coarse solve), so wall-clock is the metric and the default is no filter. Axisymmetric / strongly anisotropic-aspect 2D models converge slowly (~100 its on a 4:1 aspect Quad9 mesh): 2D never uses the iterative path under `Auto`.

## Phase 1: measured speed (release, Apple M1 8 threads)

**Speed gate: faer against `lug-solver`'s own nested-dissection supernodal Cholesky**, same reduced matrix (Quad9 plane stress):

| Mesh | dofs | faer factor | lug-solver factor | lug-solver ordering + symbolic |
|---|---|---|---|---|
| 20x20 | 3,362 | 5.5 ms | 5.7 ms | 7.9 ms |
| 50x50 | 20,402 | 29.9 ms | 65.3 ms | 50.2 ms |
| 100x100 | 80,802 | 149.9 ms | 353.8 ms | 246.4 ms |

faer is equal at lug size and 2.2-2.4x faster beyond; its symbolic phase is also 4x cheaper. **Decision: faer** (ADR-012).

**Ordering** (factor time; `nnz(L)` in millions):

| Mesh | dofs | AMD | nested dissection |
|---|---|---|---|
| Quad9 50x50 | 20k | 30.6 ms (1.6) | 47.2 ms (2.2) |
| Quad9 100x100 | 81k | 151.5 ms (7.6) | 230.3 ms (10.7) |
| Hex8 20^3 | 28k | 894 ms (25.6) | 483 ms (18.6) |
| Hex8 28^3 | 73k | 7435 ms (127) | **2590 ms (72)** |
| Hex20 10^3 | 15k | 345 ms (13.7) | 315 ms (12.0) |
| Hex27 10^3 | 28k | 882 ms (25.8) | 840 ms (27.7) |
| Tet10 10^3 | 28k | 920 ms (27.4) | 963 ms (30.3) |

-> `Auto`: AMD in 2D; both analysed in 3D, sparser kept.

**Pipeline** (assemble / symbolic / factor / solve, ms): Quad9 100x100 (81k dofs) 6 / 59 / 150 / 8; Hex27 10^3 (28k dofs) 24 / 112 / 796 / 20;
Hex8 14^3 (73k dofs) 19 / 302 / 8511 (AMD) -> ~2600 (nested dissection) / 65. The factorization dominates every 3D case by 10-100x,
so **element-kernel speed is second order** for linear statics.

**Element stiffness** (us / element): Quad4 0.33, Quad9 2.4 (old dense Q9 in `edge-check`: 1.85), Tri6 0.48, Hex8 2.9, Hex20 45, Hex27 79,
Tet10 2.5. The node-pair kernel is slower than `edge-check`'s dense Q9 on this machine (its 18-wide inner loops vectorise; 2x2 blocks do not);
the component-major kernel is 0-15 % faster, not the 3x hoped for. Known next step if assembly starts to matter (a nonlinear Newton loop
re-assembles every iteration): register-tiled GEMM formulation `P^T Q` per block and `pulp` runtime SIMD dispatch (a Windows x86-64 build is
SSE2-only by default). Measured, not yet done, because assembly is < 1 % of a 3D solve.

Reproduce: `cargo test -p fea-core --release --test bench -- --ignored --nocapture --test-threads=1` (`rtk proxy` for stderr).

## Phase 2: geometry, meshing, sweeps, import

| Capability | Method | Validation (`cargo test -p fea-core`) |
|---|---|---|
| Geometry (`geometry.rs`) | closed loops of lines and circular arcs (outer + holes, named segments), arcs evaluated exactly; orientation normalised (outer CCW, holes CW); overlap/containment by boundary crossing + parity | areas with arcs integrated exactly, rejects gaps / holes outside / overlapping holes (a collinear-but-disjoint chord pair and a vertex-gap parity bug were found and fixed here) |
| Triangulator (`delaunay.rs`) | incremental Delaunay with exact predicates (`robust`), boundary subsegments **conformed** by splitting encroached ones at curve midpoints (no flip-based constraint recovery), exterior/holes removed by subsegment-crossing parity, Ruppert refinement (circumcentre insertion; an encroaching circumcentre splits the segment instead) for radius-edge ratio > 1.2 or circumradius > `0.66 h(x)`; an arc piece is split until it sweeps <= 20 degrees | valid CCW triangulation with adjacency symmetry and local Delaunay everywhere; 150 random star polygons (5-16 vertices) all valid, area equal to the boundary polygon, min angle above `min(24 deg, 0.9 x smallest input angle)`; plate with a hole graded 8x with bore vertices on the circle to 1e-13; thin slot narrower than the size |
| FE meshes (`mesh2d.rs`) | Tri3 / Tri6, or three quads per triangle (Quad4/8/9); boundary midside and quarter-point nodes on the true curve; segment names become node sets and load-oriented surfaces | **Lame quarter annulus on unstructured meshes**: Tri6/Quad8/Quad9 `u_r` converge ~h^3 (Quad9 8.6e-4 -> 1.2e-4 -> 1.3e-5), Quad4/Tri3 converge; curved-boundary area error 2.9e-7 (Tri6) vs 5.7e-3 (chords, Tri3); pressure on `inner` integrates to `p a` to 1e-6 on quadratic types; **Kirsch** on an unstructured graded Tri6 mesh: pole 2.9985 sigma, hole hoop error 6e-3 of 3 sigma; 3 holes + a rounded slot + 0.15-wide web: area within 2e-3, no inverted element |
| Adaptive remeshing (`adapt.rs`) | element size `h_new = h_old (eta_target / eta_e)^(1/p)` from the ZZ estimate, geometric-mean nodal sizes, grading limit 0.4, interpolated on the previous mesh (grid-accelerated; clamped barycentrics just outside the straight-sided background) | **Kirsch: same true stress error with 7.5x fewer dofs than uniform refinement** (946 vs ~7,300 dofs at 6.3e-3), ZZ effectivity 1.2-1.45; L plate refines the re-entrant corner (corner element 0.42 -> 0.038 over 5 passes) |
| `extrude` / `revolve` (`sweep.rs`) | Quad4/8/9 -> Hex8/20/27, mid-plane nodes at the parametric midpoint (on the circle for revolutions), closed 360 degree revolutions share the seam, node sets carried over, `start`/`end` and lateral surfaces; revolution maps `(r, z) -> (r cos, z, r sin)` so the Jacobian is positive | every node of every hex equals the shape-function map of its corners (pins the full VTK order incl. face centres); **extruded plane-strain ring equals the 2D Quad9 solution to 1e-17**; revolved Lame sector converges to the axisymmetric solution (1.3e-4 -> 8.7e-6 halving the angle) using MPC symmetry constraints; ring volume vs Pappus converges x15.6 per layer doubling |
| Import (`import.rs`) | Gmsh MSH 4.1 / 2.2 ASCII (physical names -> blocks, node sets, load-oriented surfaces matched to the volume boundary faces); Abaqus `.inp` (CPS/CPE/CAX 3/4/6/8, C3D4/10/8/20; `*ELASTIC` isotropic or engineering constants, `*EXPANSION`, `*SOLID SECTION`, NSET/ELSET incl. GENERATE and set-in-set); quadratic orders permuted to VTK and **checked geometrically at load** | node orders built from each format's own edge/face definitions import straight-sided (Tet10, Hex20, Hex27, Quad9, Tri6; Abaqus C3D10/C3D20 with continuation lines); a wrong order is rejected; a Hex27 bar round-trips through a Gmsh file with reversed surface elements and solves to 1e-12 of the native mesh; mutating one permutation entry fails the suite. Permutation tables cross-checked against meshio's |

Measured (release, M1): triangulation 450-575 k triangles/s (7k in 14 ms, 117k in 0.2 s, 467k in 1.0 s); mesh + Tri6 + solve of a 117k-triangle plate with a hole (470k dofs) 0.26 s + 2.4 s (assemble 43 ms, symbolic 560 ms, factor 1.8 s, solve 76 ms).

## Phase 3: material nonlinearity

| Capability | Method | Validation (`cargo test -p fea-core`) |
|---|---|---|
| Small-strain J2 (`material.rs`) | radial return with the exact piecewise-linear hardening increment, closed-form consistent tangent `K 1(x)1 + 2 G theta (I - 1/3 1(x)1) - 6 G^2 (1/(3G+H') - d/q) n(x)n`; 3D, plane strain, axisymmetric, plane stress (scalar Newton on `eps_zz` from `sigma_zz = 0`, exact condensation of the tangent) | tangent equals central differences to **1e-11** (elastic, plastic, with history); simple shear of an EPP body gives `tau = sigma_y / sqrt 3` and the exact `ep`; uniaxial-stress path follows the yield curve; plane-stress elastic law equals `E/(1-nu^2)` for any yield stress |
| Finite-strain Hencky J2 | total-Lagrangian, multiplicative plasticity: state `Cp^-1` and `ep`; spectral decomposition of `Be = F Cp^-1 F^T`, return on principal log strains, `P = tau F^-T`; **analytic tangent** `dP/dF` through divided differences of the isotropic tensor function (no finite differences, exact at repeated eigenvalues such as `F = I`); plane stress by Newton on the thickness stretch | tangent vs central differences **3e-10** (elastic, F = I, rotated, plastic with history, large strain); objectivity `P(QF) = Q P(F)`; plastic flow isochoric (`det Cp^-1 = 1` to 1e-12); Kirchhoff von Mises on the yield curve; simple shear to gamma = 3; agrees with small strain at small deformation |
| Nonlinear assembly and driver (`nonlinear.rs`) | one displacement-gradient formulation `K = (dH/du)^T A (dH/du)` for every analysis type (hoop term for axisymmetry); colour-parallel scatter; elastic blocks reuse the linear kernels; Newton with the consistent tangent, symbolic factorization reuse, post-hoc line search, automatic step cutting; **tangent predictor for prescribed displacements** (without it a 3D case needed 872 iterations / 75 s, with it 26 / 2.7 s); Crisfield cylindrical **arc-length** with a **secant predictor** and an **LDL^T fallback** past limit points; failure-strain stop (`NlOptions::failure_for_material` reads `mechanics_core::fracture`); optional chord Newton | linear problems: nonlinear = linear to 1e-13 in one step (Quad9/8, Hex20, plane stress/strain); **uniaxial tension, small strain: stress and `ep` exact to 8 digits** on Hex8/27, plane stress, axisymmetric; **finite strain: exact to all printed digits from stretch 1.002 to 2.0** on Hex8/Hex20/plane stress/axisymmetric against `ln(lambda) = tau/E + ep`; **thick cylinder (plane strain, EPP)**: arc-length peak `p/p_L` = 1.0013 / 1.00006 / 1.00000 at 4 / 8 / 16 elements against the exact `(2/sqrt 3) sigma_y ln(b/a)`; plastic radius vs Hill 1.007/1.008, 1.156/1.169, 1.382/1.394, 1.632/1.640 at p = 0.55-0.95 p_L; **thick sphere** (axisymmetric) 1.0113 -> 1.0006 of `2 sigma_y ln(b/a)`; axisymmetric and plane-strain cylinders agree to 3e-4; finite-strain collapse peaks at 0.9969 p_L, then softens to 0.970 at ep = 0.227, step-size independent, tangent indefinite only past the peak; Newton local order 2.06 (small strain) and 5.3 (finite) in a plastic step; step cutting recovers a 5-iteration cap; mixed elastic/plastic blocks equal the all-plastic-path assembly to 1e-9 |

Measured: a Newton iteration in 2D/3D plastic models is dominated by the **factorization** (Hex27 7^3: tangent assembly 41 ms, factor 130 ms, solve 4 ms; Hex8 16^3: 20 / 172 / 6 ms; Quad9 120^2 / 116k dofs: 50 / 269 / 13 ms), so the planned register-tiled GEMM / `pulp` SIMD element kernel is **not worth building** (assembly is parallel and < 25 % of an iteration). Chord Newton (`chord_iters`) saves factorizations (25 -> 16 on a 3D finite-strain block, 30-35 -> 24 on a cylinder) but costs extra iterations: ~10 % faster on Hex8 14^3, no gain on Hex20 or small 2D problems, so it stays an option, off by default.

## Phase 4: general contact

| Capability | Method | Validation (`cargo test -p fea-core --test contact`) |
|---|---|---|
| Masters (`contact.rs`) | **rigid analytic** (circle, sphere, plane / line, cylinder; outside or inside; translation `shift + lambda * travel`) or **deformable faces** (closest-point projection by Newton on the quadratic face shape functions, exact normal and gap) | edge projection orthogonal and equal to the signed distance on a curved quadratic edge; rigid-circle tangent equals the analytic `eps n n^T - p (I - n n^T) / rho` |
| Slave side | Gauss-point-to-surface on listed faces, integrated over the *reference* surface (thickness / `2 pi r`), position from current displacements; collocation rule `Auto`: **nodal (Simpson) against rigid masters, 2-point reduced Gauss against deformable ones**; `Gauss` / `Reduced` / `Nodal` selectable | rigid master, quadratic edges: full Gauss pressure oscillates (10.6 % of p0), reduced 1.1 %, nodal 0.9 %; 3D sphere: reduced rule shows a checkerboard (peak +30 %), nodal 3.0 % within 0.4 a |
| Normal | augmented Lagrangian `p = max(0, lam_n - eps_n g)`, multiplier passes between Newton solves (`max_outer`, `outer_tol`) | penetration removed to < 0.1 % of the indentation; **transmitted force independent of the penalty over a decade** (5824.3 / 5824.1 / 5824.1 for 3e8 / 1e9 / 3e9) |
| Friction | AL Coulomb: tangential multiplier, incremental slip over the master material point of the last committed state (body frame for rigid masters, parametric coordinates for deformable), capped at `mu p`; the slip history is seeded for initially touching points | sliding transmits **5997.5 against mu N = 6000** (0.04 %), the reverse drag gives the opposite force; sticking gives 104.46 against the elastic-shear reference 104.85 (0.4 %) |
| Tangent | each point's residual (force pair `-+(p n + Lambda)` on slave / master by shape functions) in closed form; **tangent by central differences of that smooth local residual** w.r.t. its <= 30 variables, symmetrised; sliding friction frozen in the tangent (its derivative is unsymmetric and, symmetrised, indefinite); LDL^T fallback in the Newton factorization when contact is present; points merely close to the master add their normal stiffness only while nothing is in contact (initially touching bodies) | force / moment equilibrium of a master-slave pair to 1e-12; FD tangent symmetric |
| Search | per-evaluation face positions + **BVH** (median split, rebuilt each evaluation) for projection candidates and for the matrix-pattern coupling pairs | BVH point and box queries equal a brute-force scan on 400 random boxes |
| Benchmarks | 2D **Hertz**, rigid cylinder on a half-space (unstructured graded Tri6 mesh) | contact half-width +1.5 %, peak pressure -0.5 % (nodal), profile within 1 % of p0 |
| | **3D Hertz** rigid sphere, Hex27 (`--ignored`, ~30 s) | peak pressure **+1.7 %**, inner-profile deviation 3.0 % of p0 |
| | **Both bodies meshed** (two elastic cylinders: `1/E* = (1-nu1^2)/E1 + (1-nu2^2)/E2`) | contact half-width +4 % (resolution), inner mean pressure +0.4 ... +1.0 % |
| | **Lame shrink fit** between two unstructured, non-matching meshes | mean contact pressure 8227.9 against 8241.8 (**-0.17 %**); the two-pass sum 8235.5 |
| | Contact **patch test** across 6 / 4 element non-matching meshes | mean pressure and transmitted force exact; pointwise 9126-10685 / 9823-10182 of 10000 (classical Gauss-point-to-segment limitation) |
| | uniform pressure on a rigid plane (2D Quad9, 3D Hex27) | exact (7e-15 / 5e-14) |
| | contact **with plasticity** | first yield below the surface at **0.77 a** (Johnson 0.78 a), none at the surface |

Operating limits found while validating (all in `fea-core/AGENTS.md` Pitfalls):

* **Friction needs penalties comparable to the structural stiffness** (`eps_n ~ E / h`, not 1e9 on a 1e7 psi block): the friction force reacts to gap changes with stiffness `mu eps_n w` and the symmetric quasi-Newton tangent does not capture that coupling, so a large penalty makes Newton oscillate. The multiplier passes supply the accuracy.
* Pointwise pressures at Gauss points on *non-matching deformable* interfaces scatter (about +-25 % on coarse meshes); the transmitted force, the mean pressure and the contact extent are the reliable outputs.
* Contact uses **load control** (no arc-length); rigid masters move by a prescribed `shift + lambda travel` and/or by **free translations** (extra unknowns, force-controlled pin, Phase 5); there is no rigid rotation. The matrix pattern is built once from the initial proximity (`margin`), and a contact outside it is an error.

## Phase 5: the lug on the kernel (`lug-solver/src/fea.rs`, `lug-solver/tests/fea_bridge.rs`)

`FeaLug` hands the lug's own Q9 mesh (node order permuted to VTK, bore edges reordered to the kernel's `[end, end, midside]`) to `fea-core`, makes the rigid analytic pin a **force-controlled master** (free translation unknowns solved by a Schur complement on the bordered Newton system: `ContactSet::eval` returns the residuals and couplings of the extra unknowns, `Ctx::newton_step` condenses them) and runs the kernel's AL contact. Nothing in `lug-solver`'s validated path changed: `lug-solver` depends on `fea-core`, not the other way round (the old dev-dependency was removed; its speed-gate bench moved to `lug-solver/tests/sparse_speed_gate.rs`).

| Case (aluminium lug `D = 0.5`, `W = 1.5`, `t = 0.25`) | Kernel | Condensed / reference | Agreement |
|---|---|---|---|
| Axial 2000 lbf, frictionless, half model (Mesh 48) | pin travel 3.2737e-3, peak pressure 18355, hoop 30661, arc 153.2 deg | 3.2736e-3, 17944, 29967, 151.7 | travel **3e-5**, hoop 2.3 %, pressure 2.3 %, arc 1.5 deg |
| 45 deg 1500 lbf, frictionless, full model | travel 2.0491e-2, hoop 29539, arc 158.9 | 2.0491e-2, 29195, 158.9 | travel **<1e-5**, hoop 1.2 %, pressure 1.0 % |
| 45 deg, `mu = 0.15` | travel 1.7746e-2, hoop 32706 | 1.8048e-2, 30357 | 1.7 % / 7.7 %: see "friction" below |
| Plastic collapse, axial, plane strain, perfectly plastic `sigma_f = 55 ksi`, small strain | 8964 lbf (plateau) | 8673 lbf (`limit_load`) | 3.4 % (the condensed run stops at its first detected plateau) |
| **Finite-strain** collapse, same lug, `sigma_y = 60 ksi` | **9846 lbf** | `FiniteLug` **9842 lbf** | **0.04 %** (two independent total-Lagrangian log-strain J2 implementations) |
| Meshed elastic steel pin (ring with core hole, deformable contact, body-force load through the length) | travel 3.3698e-3, hoop 30656, arc 152.7 | `PinBody::Elastic` 3.3702e-3, 30348, 155.0 | travel **1e-4**, hoop 1.0 %, pressure 5 % |
| 3D double shear (quarter model, 17952 dofs, Hex27 lug + pin, ear load as body force) | equilibrium 2003 / 2000 lbf; face layer 1.12 x mean, mid 0.94 x | beam model (`through_thickness`): 1.006 / 0.995 | **disagreement, see below** |

Speed (release, M1; mesh 32-48): the condensed lug solves in 5-40 ms; the kernel in 0.3-5 s elastic (60-90 factorizations; the AL + Newton loop on a general 2D mesh), 20-40 s for the plastic/finite-strain collapse curves (1800-4000 factorizations), against `FiniteLug` 0.5-1.3 s. The speed gate for retiring `lug-solver`'s own solvers is **not met**, so the condensed solvers stay the interactive default and `FeaLug` is the independent cross-check. (Linear algebra alone: faer is 1.0-2.0x faster than `lug_solver::sparse` on the same Quad9 matrix: 6.0 / 6.1, 37.6 / 74.1, 179 / 355 ms at 20 / 50 / 100 elements per side; `cargo test -p lug-solver --release --test sparse_speed_gate -- --ignored --nocapture`.)

Findings from the cross-check:

* **Kernel bug fixed**: the contact tangent was a central difference of the local residual with a step `1e-6 (1 + |v|)` straddling the `max(0, .)` pressure kink. Any point with `p < eps_n * h` (a lightly loaded bore at `eps_n = 100 E / a`) got **half** its normal stiffness, Newton crawled at ~3 % per iteration and never settled. The tangent now differentiates the smooth active branch (`PointCtx::smooth`); the FD check agrees to 1e-11 at every iteration, and `a_lightly_loaded_stiff_penalty_contact_converges_in_few_factorisations` fails without the fix. `NL_FDCHECK=1` runs that check on any analysis (`NL_FDH` sets the step, `NL_TRACE=1` prints per-iteration active points, `NL_DEBUG=1` prints failed steps and AL passes).
* **Driver fixes**: step growth now follows the Newton effort of the step itself, not of its AL passes (frictional steps never grew back after a cut); the line search backtracks up to 24 halvings with contact (a free pin meeting a penalty stiffness overshoots by orders of magnitude); the free rigid translation of `RigidMaster::with_free_translation` was not passed to the point residual (the first force-controlled run silently ignored it).
* **Friction (finding, not fixed)**: the condensed solver's frictional answer moves with its normal penalty although the AL passes should remove that dependence (`LugModel::solve`, 45 deg, `mu = 0.15`, mesh 32, penalty factor 10 / 30 / 100 / 300 / 1000: hoop 31284 / 30967 / 30357 / 29971 / 29849, peak pressure 16730 ... 28906; diagnostic `condensed_friction_sensitivity_to_its_normal_penalty`, ignored). The kernel's answer is independent of its friction penalty (0.01-0.1 `eps_n`, hoop 32706 throughout) and of the AL tolerance. Frictionless cases agree to <1e-5 in travel, so the discrepancy is in the condensed friction scheme; path dependence of Coulomb friction (how the pin seats) may account for part of it. Not resolved here (needs a product decision on which friction answer is the reference); the bridge test uses a coarse tolerance for the frictional case.
* **Through-thickness (finding)**: the 3D double-shear lug shows the face layer nearest the ears carrying **12-17 % more** than the mid-plane (mesh dependent: 1.05 / 1.12 / 1.17 x mean at 2 / 4 / 6 layers per half thickness; a uniform-pin baseline gives 0.96 at the face, 1.03 mid, so ~20 % of the face peaking is load feeding in through the pin), while `LugModel::through_thickness` (stubby steel pin as a Timoshenko beam on the lug's springs) predicts 1.006: the pin is almost rigid on the bearing springs (`(4 E I / k)^(1/4) ~ 3.5 in >> t`), so the beam sees no peaking. The 3D face value keeps growing with refinement (a load-transfer singularity at the ear / lug interface), so it is an upper-trend indication, not a validated peaking factor; the beam model is therefore not a conservative predictor of face bearing stress for stubby pins. Single shear is not built (it needs the tang / clevis members' own bending, a product decision as noted in `thickness.rs`).

Operating notes for `FeaLug`: the pin starts touching the loaded side (`shift = clearance * direction`); `Reduced` collocation (nodal oscillates on a quadratic bore); a half model carries half the load; the friction penalty is `0.03 eps_n` (soft keeps the AL passes few); `limit_load_axial` drives the pin by prescribed travel (axial only; an oblique drive needs a travel along a non-axis direction) and reads the load per step (`StepInfo::master_force`).

## Not done / limits

* No shells or beams. Anisotropy is elastic only with an isotropic thermal strain (no thermal-expansion tensor).
* MPCs use a one-shot Cholesky of the reduced system (no iterative path, no symbolic reuse across load steps).
* The iterative path is for 3D; 2D under `Auto` always factors.
* `Tri3`/`Tet4` accuracy is poor on pressure problems (library elements for meshers, not for results).
* Meshing: no sharp-input-angle protection beyond `min_edge` (corners below ~30 degrees keep a smaller angle); no islands inside holes; no 3D unstructured mesher (extrude/revolve or import); Tri meshes cannot be swept (no prism element).
* Import: ASCII only; Gmsh 4.0 and binary not supported; Abaqus surfaces / contact definitions are not read (use `Mesh::select_faces_by_nodes` on an imported node set).
* Nonlinear: dead (conservative) loads only (no follower pressure, no contact until Phase 4); isotropic J2 with isotropic hardening only (no kinematic hardening, no rate dependence, no thermal strain with plasticity); arc-length control needs zero prescribed displacements; the iterative (AMG) solver is not used by the nonlinear driver; the indefinite factorization is LDL^T without pivoting (a zero pivot fails the step, which is then cut).
* Uniform tension past the Considere point is an unstable equilibrium: Newton may leave the uniform state through rounding noise (observed on Hex20 with a Swift-type curve); tests use stable hardening laws.
* Contact: see the Phase 4 limits (load control only, rigid masters translate only, static pattern, no mortar, friction needs moderate penalties); arc-length cannot be combined with contact or free rigid translations.
* (Superseded by Phase 7: the lug on the kernel is now the toolbox's main solver.)

## Phase 7: the kernel is the main lug solver (`lug-solver/src/fea*.rs`, `app-tui` Lug Analysis)

Supersedes "the lug on the kernel is a cross-check, not the interactive path" above: the Lug Analysis toolbox now runs
every analysis on `fea-core` by default (`Solver: General kernel`); the condensed / finite-strain solvers stay as
`Solver: Legacy` and `Solver: Compare` (both side by side). A case the kernel cannot run falls back to the legacy
solver with a note (only one exists: a meshed elastic pin with friction on the full oblique model needs the clevis
torque reaction).

**What runs on the kernel** (`FeaLug`): the elastic analysis with the condensed solver's own result type
(`analyze` -> `LugSolution`: contact points, bore stresses, peaks, bushing fit, verification incl. ZZ error), bushing
with an interference fit and friction (a conforming interface, two-pass contact with `ContactSpec::with_overlap`,
solved as its own stage and continued with `Start`), thermal change of the fit (the pin and interference sizes),
second order (every block a finite-strain elastic body), meshed elastic pin (axial half model, or full model without
friction), oblique plastic collapse (the lug is rotated so the load is `-x`; the pin's sideways translation is free and
force-free), hardening collapse with a strain limit, finite-strain collapse (`finite_collapse`, any direction, with a
bushing), load and angle sweeps (`load_sweep`, `analyze_sweep`, `collapse_sweep`, parallel).

Validation against the condensed solver on the same mesh (`lug-solver/tests/kernel_analysis.rs`, `kernel_features.rs`,
`kernel_naca.rs`):

| Case | Kernel | Condensed / exact |
|---|---|---|
| Press fit, round disc | fit pressure 9557.4 psi | two-cylinder Lame 9557.3 psi (+0.00 %, with and without friction) |
| Press fit in the lug, steel bushing | 7593 psi | 7593 psi |
| Heated steel bushing in aluminium | linear in the net interference within 2 % | `Thermal` helpers |
| Beam-column amplification, tension / compression (`u = 0.8`) | 0.806 / 1.231 | beam 0.797 / 1.346 (second order from finite-strain elastic blocks) |
| Axial rigid pin | hoop +2.5 %, travel 1e-4 | condensed |
| Bushed axial | hoop +0.8 % | condensed |
| Elastic pin, axial / oblique 45 deg | hoop +1.2 % / +1.1 % | condensed |
| Collapse, axial / bushed / 30 / 60 deg | +3.3 % / +2.3 % / -3.1 % / -2.4 % | condensed (which only reports a lower bound for oblique) |
| Finite-strain collapse, axial / mu 0.15 / 30 deg | +0.05 % / -0.4 % / 0.00 % | `FiniteLug` |

**Speed** (release, M1, mesh 32 to 72 around the bore; the toolbox default is 72): frictionless elastic 1.1 s -> 0.07-0.3 s;
oblique with friction 4000 lbf (10k dofs) failed (NaN tangent) -> 2 s; axial collapse 8.5 s -> 0.4 s; oblique collapse with
friction at 54 elements 15 s -> 5.8 s; default toolbox run (axial, friction 0.15, collapse) 3.3 s (legacy 0.55 s); default
oblique 45 deg 30 s (legacy 3.3 s; elastic and collapse run side by side). Where it came from, in order of effect:

* **Bug: negative friction capacity.** On the smooth branch the tangent can see a tiny negative pressure at a point on the
  active-set boundary; `cap = mu p` was then negative and, with zero slip, `cap / |trial|` was 0/0. The NaN tangent made every
  frictional lug near full load fail ("ZeroPivot") or crawl through step cuts. Fixed (`cap.max(0)`), regression test
  `a_slightly_negative_smooth_pressure_gives_finite_friction_with_no_capacity`; friction runs got 3-10x faster.
* **Soft penalty.** `10 E/a` against `100 E/a` is 5-13x faster for the same answer (the multiplier passes remove the penalty
  error); friction needs `30 E/a` (a soft one leaves the pressure peak ~10 % low). `Tuning` presets: `frictionless`, `friction`,
  `collapse`, `reference`.
* **faer sequential below 40k dofs.** On a 10k-dof lug its threads cost more than they save (3.4 s sequential against 6.0 s on 8
  threads for the same 165 factorisations); the assembly keeps rayon (`linear.rs::parallelism_for`).
* **2D plastic tangent.** The generic 9 x 9 `dS/dH` contraction replaced by the 2D form: 5x fewer operations (`tangent_tests`).
* **Looser Newton tolerance** (1e-4 against 1e-6/1e-9) with the multiplier passes unchanged: identical hoop stress, pressure and
  travel in 60 % of the time; one multiplier pass per collapse step (collapse load moved 0.01 %).
* Dependencies are optimised in dev builds (`[profile.dev.package."*"]`): faer in an unoptimised build made a lug analysis take minutes.

**Findings** (all with tests):

* **Friction is path dependent and the kernel is the reference.** One load step skips the history and lands ~8 % low on the hoop
  stress; two or more steps agree to 4 digits (2, 4, 16, 32 steps at 45 deg, mu 0.15, and pf 30 or 100). The kernel's answer is
  independent of the penalty; the condensed solver's is 5 % penalty-sensitive and sits ~8 % below the kernel's (`Legacy` and `Compare`
  say so in the notes). This is item 1 of the handoff, resolved by making the kernel the reference rather than by changing the legacy path.
* **The condensed small-strain collapse is not stopped early by its plateau detector** (item 4): with the detector off its curve still
  peaks at 8673 lbf and falls to 8085; the kernel's fully converged Newton is ~3 % above it at every mesh. NACA TN 1503
  (`kernel_naca.rs`): `(Ftu + Fty)/2` 6-25 % low (mean 15 %), `Ftu` from 13 % low to **2 % high** (mean 7 % low: no longer "never
  high"), `Ftu (1 + elongation)` -2 to +14 % (mean +5 %). The toolbox quotes the figures of the solver in use (`naca_statistics`).
* **Oblique collapse is mesh dependent** (both solvers): 45 deg, 24 / 32 / 48 / 72 / 120 around the bore gives 5322 / 5152 / 4658 /
  4583 / 4380 lbf (still falling at 120); axial is converged (9772 -> 9733). The mesh-size test therefore includes the collapse load.
* **The plateau detector must read travel, not steps**: after step cuts three equal loads over a sliver of travel looked flat (a bogus
  351 lbf collapse). The rule is now "gained under 1.2 % of the top over the last 5 % of the travel" (`load_flat`), and a pin that
  cannot be pushed further once its load is within 4 % of flat has reached its limit state (Newton fails on the singular plateau).
* **Through-thickness (item 2)**: the 3D double-shear lug's face bearing (1.12-1.17 x mean) grows with refinement: it is a load-transfer
  singularity at the lug face, no converged elastic value exists. The beam model is the mean-field estimate (peaking 1.006), so the
  toolbox says to read it as a lower bound for a stubby pin. Not changed; a converged face value would need plasticity or a chamfer model.
* **Single shear (item 3)** stays unoffered: it needs the tang / clevis members' own bending, a product decision (a 3D model with the
  members would do it).
* A frictionless bushing interface now solves (weak grounding springs, `Loads::ground`); the condensed solver cannot.
* A nodal Von Mises peak is noisy at the contact edge: both solvers report the averaged nodal peak; kernel and condensed agree within 1-3 %.

Limits: elastic pin with friction on the full model falls back to the legacy solver; the kernel is 3-10x slower than the condensed solver
for the elastic case and ~10x for the collapse (a collapse at 72 elements around is 4-10 s), which the toolbox hides behind its worker thread.

## Phase 8: a workbench, and the toolboxes that can use the kernel use it

**Decision.** Which toolboxes can use a finite-element kernel? Lug Analysis already does (Phase 7). Pressure Vessel (exact Lame, thermal gradient the kernel cannot load: it has uniform `delta_t` only), Fastener Holes (tolerances) and Search have nothing for it to add; an FE re-derivation of a closed form is a test, not a feature. That leaves three real uses, and a place to author problems:

1. **FEA Workbench** (`app-tui` toolbox over the new `fea-problem` crate): a sketched outline with holes (rectangle / circle / slot / polygon), plane stress / plane strain / axisymmetric / 3D extrusion, or an imported Gmsh / Abaqus mesh; material from the handbook browser; edge and point supports with prescribed components; pressure, traction, edge force, bearing (pin) load, point force, body force, temperature change; mesh size, element type, hole refinement, adaptive passes on the ZZ error estimate; mesh preview then colour contour of von Mises, principals, components and displacements, with the deformed shape; JSON problems, text report, `.vtu`. Validated in `fea-problem/tests/problems.rs` against Heywood's Kt (4 %), Timoshenko's cantilever (2 %), Lame in axisymmetry (0.2 % displacements), an exact 3D bar, bearing-load resultants, equilibrium of every template.
2. **Bushing Workbench edge-distance check**: the plane-stress FE model (`edge-check/src/kfem.rs`) now runs on the kernel with the *same* O-grid, loads and constraints as the banded solver it replaces for this model: displacements agree to 1e-12 (pin cases) and 3e-6 -> 0 (the fit case, once its load is the radial field on the true circle instead of the element edge's own normal), stress at probes to 1e-6. `edge-check`'s contact and plastic-collapse solvers keep their own banded factorisation: they reuse it as an iteration matrix and are validated against NACA TN 1503 with a fixed-point invariant on recommended edge distances; porting them means re-validating those bands and is not done (the lug's own contact collapse already runs on the kernel in Lug Analysis).
3. **Preload Analysis**: an FE cross-check of the clamped-member compliance (`fea-problem::joint`, axisymmetric rings sharing nodes, equal and opposite bearing pressures, `C = 2U/F^2`) beside the pressure-cone `C_m`, with the joint load fraction each gives and the cone half angle that reproduces the FE compliance. A typical two-plate aluminium joint under a 0.75 in head gives an FE compliance 17 % above the 30 degree cone's, an equivalent half angle near 22 degrees. Read-only: the solver's numbers are unchanged. Members are assumed Poisson's ratio 0.3.

**Kernel additions** (each with tests in `fea-core/tests`): `Model::locator` / `Locator::stress_at` (stress at any physical point by Newton inverse of the isoparametric map, averaged over the elements containing it, nearest element within 2 % of its natural extent for a point a hair off a curved edge), `Model::solve_static_many` (several load cases on one factorisation), and `Model::check_constrained` (called by `solve_static_with`).

**Bug found and fixed.** An under-supported model (a roller on one edge with nothing holding the other direction, or one pinned node) used to factorise anyway - rounding leaves tiny positive pivots - and return the true answer plus an arbitrary rigid motion, silently. `check_constrained` tests the rigid-body modes of every connected piece against the constrained dofs and refuses with "not fully constrained"; regression test `unconstrained_rigid_body_motion_is_an_error`.

Limits: the workbench is linear static (no plasticity or contact authoring yet; the kernel has both), one material per problem, supports and loads reference edge names (no picking in the terminal), the polygon editor lists outer points only, the contour of a 3D model is its front face (`z` maximum), the axisymmetric extrusion / revolution is not offered, and an imported mesh needs its names in the file.

## Phase 9: shared bushing contact set-up and the eccentric-bushing upgrades (`fea-core/src/fit.rs` and the kernel)

`Tuning`, `interference_contacts` and `start_after_fit` / `start_from` moved here from `lug-solver/src/fea.rs` so that the new `eccentric-bushing` crate, the FEA Workbench's bushings and `FeaLug` share one validated path (`lug_solver::fea::Tuning` is a re-export). Guard: `lug-solver/tests/kernel_analysis.rs` (press fit vs Lame to 0.00 %) and `kernel_features.rs`; the whole `lug-solver` suite is green after every change below.

Kernel limits this effort removed (each with its test; the evidence and the measurements are in `docs/eccentric-bushing.md` section 9):

| Limit listed in "Not done / limits" | Now |
|---|---|
| Dead (conservative) loads only | `Loads::followers`: follower pressure with its load stiffness; exact Hencky hydrostatic compression to 1e-9 (2D plane strain Quad9, 3D Hex20), load stiffness = finite differences to 1e-6; a dead pressure demonstrably gives a different state (`tests/follower.rs`) |
| Contact needs load control (no arc-length) | Arc-length with deformable contact (free rigid masters still need load control), last step lands exactly on `lambda_max`, `Start::lambda`, `NlOptions::stop_at_force` (`tests/contact.rs`: a free block crosses a gap into contact) |
| Anisotropy elastic only | Anisotropic elasticity with small-strain J2 plasticity (iso-equivalence 1e-9, yield surface, consistent tangent, plane stress, an orthotropic bar yields at exactly the uniaxial yield stress) and an anisotropic thermal expansion tensor that rotates with the material (`tests/anisotropic.rs`) |
| Rigid masters translate only | A 2D circular master can rotate under a moment (`RigidMaster::with_free_rotation`); torque equilibrium of a friction-transmitted moment to 3 % (`tests/contact.rs`); moment-controlled runs slow near the friction limit (symmetric Schur complement) |
| Static contact pattern | A node pair outside it retries with doubled margins |
| Friction tangent frozen when sliding (Newton stalls on stick/slip chatter) | Defect-correction refinement makes the step consistent; the failure that motivated it (pin loaded on an eccentric bushing) converges |
| (not a listed limit) a body held by nothing | `Model::solve_nonlinear*` refuses it; `NlSolution::ground_leak` reports the springs' share |
| (not a listed limit) Tri3 / Tet4 | The Workbench's results carry the documented caution |

Bugs found and fixed on the way (all would have bitten other models): the closest-point projection's absolute `1e-14` tolerance (contact away from the origin), the relative convergence tolerance that cancelled at equilibrium (starting a run from a converged fit), the tangent predictor dragging approaching bodies, and a silent wrong answer (a converged run that carried none of the pin load; now an equilibrium guard in the consumer).

Measured and declined: element-stiffness caching / SIMD assembly (assembly is 9 % of a contact solve, contact evaluation 35 %, factorisation 33 % of 80-160 factorisations); reusing a factorisation across AL passes or chord Newton in the arc corrector (153 vs 145 factorisations: no gain); looser AL tolerances or stiffer penalties (they fail, or converge to a wrong state); topology-reusing sweeps (mesh and model set-up are ~3 % of a solve: the offset search is parallel instead); the contact collocation rule (`Reduced` 0.02 % pointwise scatter on a concentric fit, full Gauss 0.03 %, nodal does not solve).

Not done (too large for the session, no product decision needed): an unstructured 3D mesher, a prism element, shells and beams, dynamics.

Answer to "can the kernel solve any problem type?": no. Statics of 2D / 3D solids (isotropic and anisotropic elastic, J2 plasticity small / finite strain, anisotropic small-strain plasticity), contact with friction, follower pressure, thermal loads and point-in-mesh stress queries yes; shells, beams, dynamics / modal / buckling, rate-dependent or kinematic-hardening plasticity, plasticity with a temperature change and an unstructured 3D mesher are not there.

## Phase 10: random corpora, a shared adaptive loop, and what they found (2026-10-08)

Method: instead of waiting for a user to report a failing input, seeded random valid inputs for every consumer of the contact kernel, each failure traced before it was fixed and each fix tried on the other corpora. Corpora: `fea-problem/tests/sweep.rs` (bushed plates, linear templates, bolted-joint stacks, `tests/data` regression problems), `lug-solver/tests/kernel_sweep.rs` (random lugs on `FeaLug`), `fea-core/tests/contact_sweep.rs` (stacked blocks, 2D and 3D, matching and non-matching meshes, friction below the limit), `eccentric-bushing/tests/sweep.rs` (existing, plus the safety-net usage measurement).

| Finding | Evidence | Fix |
|---|---|---|
| A slave point on a master element edge picks either face on round-off; the faces' normals differ (C0 surface) | bushed plate seed 11: Newton stagnated at every step size; residual ±5.5 lbf at 5 points, indifferent to `dx` | committed face kept while the point is over it; the active-set hold also freezes faces (`contact.rs`) |
| The state returned by a load step paired the displacements with multipliers updated after the last solve | 8 of 24 random lugs failed `Verification::force_balance` (up to 4.5 % of a small pin load) | `refine_contact` returns an equilibrated pair (`nonlinear.rs`) |
| Hand-written tangential penalty ratios | 3D stacked blocks stagnate at 0.1, converge at 0.03 | `fit::EPS_T_RATIO`, `Tuning::eps_t` |
| Holding the active set for every contact solve, a stricter free-body force tolerance, friction history across master faces | each broke an existing test or a corpus case | tried and rejected (see `fea-core/AGENTS.md`) |
| 3D partial slip (shear above ~0.4 of the friction limit) | 8 of 24 random cases do not converge | **fixed in Phase 11**: the consistent frictional step is solved by preconditioned GMRES instead of plain refinement |
| Linear templates, bolted-joint compliance | 14 random templates and 12 stacks: no failure | none needed |
| Lug on the kernel | 24 random lugs: all pass after the pairing fix | none |

Adaptive meshing is now a kernel service: `adapt::refine` (+ the `Pass` trait) is the loop (refine above the target, coarsen below it, keep a pass only if it lowers the error or meets the target with < 90 % of the unknowns); `fea-problem` calls it for sketches **including bushed plates** (the fit and the pin load are re-solved on every new mesh; the bushing is sized from the same field as the plate so both sides of the interface ask for the same element size). Measured on the bushed template: 10281 -> 4769 nodes at zz 0.011, torque capacity identical, von Mises -0.2 %, peak pressure +4 %. The eccentric-bushing and lug toolboxes keep their own parametric meshes (their sizing follows the wall and the bore, and their mesh studies show the capacity is mesh independent from r/6; an error-driven loop would only add cost there).

Safety nets of the eccentric-bushing analysis, measured over 192 random cases (default and coarse meshes, three seeds) after the active-set hold: the second and third loaded attempt, the mesh routes and the fit-alone fallback were reached 0 times; stalled-residual acceptance 3 times; the hold itself 65 % of the time. The attempts, the mesh routes (and `Control`'s route memory, `Analysis::mesh_scale`) and `NlOptions::step_memory` were deleted; the verification suite and the soaks pass without them.

Debug aids added: `NL_TRACE` now prints the residual, `|dx|`, the minimum gap, per contact the points in contact / slipping / off their committed face, and the two largest residual dofs; `NL_TRACE=full` also lists the first contact's pressures. `NlSolution::held_solves` counts the solves finished with a held active set.

## Phase 11: the adaptive solver programme (started 2026-10-08)

**Governing principle** (user brief): start from the most complete applicable reference formulation; adapt the strategy toward the lowest cost that demonstrably meets the required accuracy; escalate or revert when evidence requires it. Accuracy is never traded for speed. Every automatic simplification is recorded, verified and reversible; an unsupported method is reported, never silently substituted. Order of operations: define, validate the model, establish the reference formulation, choose the initial strategy, baseline, solve and monitor, adapt, verify every reduction, escalate/revert, accept and report.

**Machine learning**: only if it costs no speed and never overrides a verification. Position: (1) first build the decision log (below); (2) learned or tabulated predictors may only choose *starting values* (step size, tuning, solver path) that the verifier then checks, so a wrong prediction costs one retry, never a wrong answer; (3) prediction must be cheaper than the saving (a lookup or a small regression on logged features, not a network inference per iteration). Nothing is built before the log exists to train and measure it on.

### Increments

| # | Increment | Proof |
|---|---|---|
| 1 | Baseline timings (below) and this matrix | measurements recorded here |
| 2 | Structured solve report: per-step/iteration convergence history, strategy decisions (solver path, ordering, tuning, step cuts, active-set holds), tolerances, conditioning estimate, verification outcome; machine readable, no `NL_*` env var needed | reproduces `NL_TRACE` data; deterministic |
| 3 | ADR-013: eigen / time integration / sensitivity (build vs crate) and the strategy-selection architecture (reference run, reduction, verify, revert) | decision record |
| 4 | Modal analysis (validated on cantilever / plate closed forms), then linear buckling (Euler column), then Newmark / HHT transient | closed forms, convergence rates |
| 5 | 3D frictional partial slip (root cause: the defect-correction refinement diverged; solved by GMRES, no semismooth rewrite needed) | `contact_sweep` shear to 0.8 + all four corpora |
| 6 | Strategy selector with verified reductions (e.g. element order / integration / tolerance / solver path), error estimate against the reference | reduced result within the required bound of the reference on a corpus |
| 7 | Heat conduction field, sensitivity / optimisation, UI | later |

### Baseline (release, Apple M1, 8 threads, 2026-10-08; `cargo test -p fea-core --release --test bench -- --ignored --nocapture`)

Linear static (ms; setup / assemble / symbolic / factor / solve): Quad9 100x100 (80.8k dofs) 13.4 / 6.6 / 59.4 / 140 / 8.5; Quad4 100x100 (20.4k) 3.5 / 2.4 / 10.3 / 22.7 / 1.7; Hex8 10^3 (27.8k) 7.5 / 8.1 / 74 / **1466** / 16; Hex20 10^3 (14.9k) 5.3 / 10 / 52 / 639 / 8.2; Hex27 10^3 (27.8k) 9.8 / 17.6 / 113 / 1480 / 16; Hex8 14^3 (73.2k) 20 / 19 / 301 / **7974** / 69; Hex20 14^3 (38.5k) 16 / 33 / 187 / 5048 / 38; Tet10 10^3 (27.8k) 11 / 8 / 82 / 1621 / 17. In 3D the numeric factorisation is 90-95 % of the cost and grows steeply (Hex8 6^3 -> 10^3: 4.2x the dofs, 20x the factor time); 2D is cheap. The PCG + AMG path (`iterative_against_direct_on_large_hex_models`) is the existing alternative above 150k free dofs: the threshold is a candidate for the strategy selector to revisit with these numbers.

Nonlinear plastic (one tangent: assembly / factor / solve ms): Hex8 16^3 (14.7k dofs) 19 / 225 / 5; Hex20 8^3 (8.0k) 36 / 145 / 3; Hex27 7^3 (10.1k) 43 / 194 / 4; Quad9 120^2 (116k) 23 / 238 / 13. Factorisation dominates every nonlinear iteration.

Contact (`contact_sweep.rs::contact_speed_baseline`, stacked blocks, friction, best of 3): 2D 260 dofs 36 factorisations 27.6 ms; 2D 240 dofs 23 / 16.8 ms; 3D Hex27 1050 dofs 55 factorisations **511 ms** (9 ms per factorisation); 3D 900 dofs 33 / 279 ms. 3D contact is the expensive case at small size.

### Capability matrix (state of the code, verified 2026-10-08, updated at the end of the session)

| Capability | State | Proof |
|---|---|---|
| 2D plane stress/strain/axisymmetric, 3D solids, 10 element types | implemented | `tests/linear_static.rs`, `tests/classic.rs` |
| Direct sparse solve (faer, AMD / nested dissection, parallel from 1M / 8M factor entries), PCG + AMG (`Auto` from 60M entries) | implemented | `tests/iterative.rs`, `tests/bench.rs`, `tests/strategy.rs` |
| MPC, rigid coupling, Dirichlet by elimination | implemented | `tests/mpc.rs` |
| Isotropic / anisotropic elasticity, thermal load (uniform dT **and a nodal temperature field**) | implemented | `tests/linear_static.rs`, `tests/anisotropic.rs`, `tests/thermal.rs` |
| J2 plasticity small / finite strain (isotropic hardening), anisotropic small strain | implemented | `tests/nonlinear.rs`, `tests/anisotropic.rs` |
| Hyperelasticity: Hencky (finite-strain elastic) and **compressible neo-Hookean** (solid, plane strain, axisymmetric) | implemented; Mooney-Rivlin / Ogden missing | `tests/hyperelastic.rs` (tangent vs finite differences, uniaxial closed form to 1e-6, small-strain limit, quadratic Newton) |
| Follower pressure, arc-length, Newton + line search + step cutting | implemented | `tests/follower.rs`, `tests/nonlinear.rs` |
| Contact: normal AL + Coulomb friction, deformable / rigid masters, 2D and 3D (shear to 0.8 of the limit) | implemented | `tests/contact.rs`, `tests/contact_sweep.rs` |
| Error estimate (SPR, ZZ), adaptive remeshing | implemented | `tests/recovery.rs`, `tests/adapt.rs` |
| Meshing: 2D unstructured, structured, sweep, Gmsh/Abaqus import | implemented; no 3D unstructured, no prisms | `tests/mesh2d.rs`, `tests/sweep.rs`, `tests/import.rs` |
| Convergence history / decision log as data, JSON report | implemented | `tests/report.rs` |
| Conditioning estimate | implemented | `tests/report.rs` (dense eigen oracle) |
| Model validation before a solve | implemented | `tests/strategy.rs` |
| Modal analysis (consistent / lumped mass), buckling (both-sign stress states), linear transient (Newmark / HHT) | implemented | `tests/dynamics.rs`, `fea-problem/tests/dynamics.rs` |
| Heat conduction: steady and transient, flux / convection / source, temperature-dependent conductivity | implemented | `tests/thermal.rs` |
| Sensitivities (adjoint: displacement, compliance, frequency; modulus, density, thickness, node position) and a block-sizing optimiser | implemented | `tests/sensitivity.rs` (vs global finite differences), `tests/optimize.rs` (closed form) |
| Automatic strategy selection with verified reductions | implemented for linear statics (`solve_adaptive`), the nonlinear / contact ladder (`solve_nonlinear_ladder`, caller supplies the check) and the eigen method (Lanczos, then subspace iteration if rejected) | `tests/strategy.rs`, `tests/dynamics.rs` |
| Kinematic / rate-dependent plasticity, plasticity with dT | missing | |
| Temperature-dependent elastic properties, thermal-structural coupling beyond one way | missing | |
| Shells, beams, prisms, 3D unstructured meshing | missing | |
| Machine-learned strategy prior | **evaluated, not built**: nothing to predict (see below) | `contact_sweep.rs::tuning_preset_measurement` |

### Done in Phase 11 (2026-10-08, evidence in the tests named)

| Increment | What | Proof |
|---|---|---|
| Solve report | `NlSolution::{events, profile, report_json}`: step cuts, held active sets, accepted stalls, indefinite tangents, widened margins, augmented-Lagrangian passes that ended without meeting `outer_tol`, strategy changes and warnings, plus per-step residual histories and the time split; `NL_EVENTS=1` echoes them. `Solution::{method, rel_residual, factor_nnz, fallback}` for linear solves. `Model::condition_estimate` | `tests/report.rs` (events = cuts, deterministic JSON, estimate vs dense eigen oracle, cond ~ h^-2) |
| Model validation | `Model::validate`: empty mesh, bad constraints, non-finite data, invalid material, inverted elements, aspect ratio, orphan nodes, rigid body, load assembly, low-order elements, axisymmetric radius | `tests/strategy.rs` |
| Verified adaptive solve | `Model::solve_adaptive(loads, bc, Requirements)`: validate, reference direct solve or the iterative path where the factor size says it pays, independent verification (a-posteriori residual, global force balance, optional ZZ requirement), reversion to direct on a failed check, JSON report; `Model::solve_nonlinear_ladder`: rungs cheapest to most robust, each verified, escalation recorded | `tests/strategy.rs` |
| Modal analysis | `Model::modal`: consistent or HRZ-lumped mass, shift-and-invert Lanczos (default) or subspace iteration, residual-verified, effective modal masses | `tests/dynamics.rs`: axial bar (closed form), Euler-Bernoulli cantilever 2D / 3D, free-free beam with rigid modes, mass = rho V, lumped below / consistent above, Lanczos vs subspace incl. multiple eigenvalues |
| Buckling | `Model::buckling`: geometric stiffness from the linear stress state, `K phi = lambda (-K_G) phi` | Euler cantilever column 2D (1.5 %) and 3D (2 %), tension has no positive factor |
| Transient | `Model::transient`: Newmark / HHT-alpha, Rayleigh damping | second order against exact modal superposition (error ratios 3.5-4.5), energy conserved to 1e-10, HHT / damping never gain energy |
| 3D partial slip (root cause found and fixed) | the frictional Newton step is now solved by GMRES preconditioned with the factorisation (`refine_consistent`); plain defect refinement diverged where most points are at their friction limit | `contact_sweep` shear to 0.8: 0 of 24 random 3D cases fail (8 of 24 before); the whole sweep takes 10 s instead of 58 s |
| Latent wrong answer found by that fix | holding the active set of a stagnating iterate that had lost the pin's contact froze a state carrying none of the pin load; a hold now needs at least half of the start's contact per interface | `eccentric-bushing` `a_stagnating_step_that_lost_the_pin_is_not_frozen_into_a_wrong_answer`; all corpora re-run clean |
| Parallel factorisation | the numeric factorisation runs on all threads when the factor is large enough for the dimension (3D: 1M entries, 2D: 8M) instead of only above 40k unknowns | 3D blocks below 40k dofs 1.8-2.1x faster (Hex8 10^3 1501 -> 787 ms, Hex20 14^3 5145 -> 2578 ms), 2D unchanged; bit-for-bit reproducible (`tests/strategy.rs`) |
| `Auto` solver choice | in 3D from 20k free dofs `Auto` looks at the exact factor size and takes PCG + AMG from 60M entries (was: from 150k dofs), verified, with the direct solve as the recorded fallback | `iterative_against_direct_on_large_hex_models`: equal at 12M (Hex8) / 49M (Hex20), 1.6x faster at 39M, 3.6x at 125M, 6x at 310M |

Method note: the verification is never the iteration's own convergence flag. The eigensolver returns a residual per mode and refuses a pair above the tolerance; the adaptive solve recomputes `|K u - f|` and the global force balance from the result; the ladder takes a caller-supplied physical check.

Eigensolver cost (`modal_analysis_cost`, release, 6 / 20 modes, 3k-11k dofs): Lanczos with the completeness check 100-1300 ms against 160-3400 ms for subspace iteration (1.2-2.6x).
| Heat conduction | `Model::solve_heat_steady` (Picard for a conductivity table, residual and energy balance reported), `solve_heat_transient` (theta method), `Loads::temperature` drives the structure (loads and stresses at the Gauss points) | `tests/thermal.rs`: linear and parabolic profiles exact, flux / convection, Kirchhoff transform, cooling slab at second order, hollow cylinder, 3D energy balance, a linear field expands a free body stress-free (exact quadratic displacement), a clamped bar carries -E alpha mean(T) |
| Sensitivities and sizing | `Model::{sensitivities, compliance_sensitivities, eigenvalue_sensitivities}`, `optimize_compliance` (optimality criteria at a material budget) | adjoint = global central finite difference for displacements, compliance (incl. a loaded node's pressure load vector) and frequencies; series bars reach the closed form `s ~ 1/sqrt(E)`, with a binding bound |
| Neo-Hookean | `J2::neo_hookean()` through the finite-strain path | `tests/hyperelastic.rs`; written in the displacement gradient so a tiny strain keeps its digits (the form in `F` stalled Newton at `G eps` absolute error) |
| Buckling with tension and compression | a Cholesky-proven shift below the smallest positive factor (`K - sigma B` SPD) puts the wanted factors nearest the shift; both signs are found and the positive ones kept | `fea-problem/tests/dynamics.rs::a_mixed_sign_stress_state_still_gives_buckling_factors`; the unshifted pencil did not converge (residual 1e-5) |
| Workbench | keys `n` (6 natural frequencies) and `b` (3 buckling factors), mode picker `[` `]`, deformed mode shape; `Mass Density` row | `app-tui` unit and render tests; checked in a real terminal (tmux): cantilever 319 Hz vs Euler-Bernoulli 321 Hz, axial mode 4978 Hz vs 4972 Hz |
| Bug fixed on the way | `stagnated_cuts` was cumulative over the whole analysis, not per load step as documented, so a long run exhausted the budget on early trouble: a random 2D friction case failed with the standard preset | `contact_sweep.rs::tuning_preset_measurement` 60 of 60 |

### Machine learning: evaluated before it was built

The brief: use ML if it costs no speed. Procedure: find a decision whose outcome varies and costs time, log it, see whether a cheap predictor could save that time. Decisions in the kernel: (1) direct vs iterative solve, (2) the contact `Tuning` preset, (3) eigensolver method, (4) factorisation parallelism.

* (1) and (4) are functions of the exact symbolic factor size, measured above: a threshold read off a table, no model needed and nothing a learner could add.
* (2) `tuning_preset_measurement` (60 random stacked-block cases, 2D and 3D, friction, shear to 0.8): the soft preset passed 60 of 60 in 3.6 s, the standard frictional preset passed 60 of 60 in 9.6 s once the per-step budget bug was fixed (59 of 60 before). A predictor would save the failing attempts, and there are none to save: the cheaper rung is accurate on the verified quantities. The consumers' accuracy needs (peak pressure, not only force balance) are physical requirements, not predictions.
* (3) Lanczos is rejected by its own residual check in a small class of problems (mixed-sign buckling, now handled by the shift); the fallback costs one extra attempt.

Conclusion: no learned component is justified, and none is shipped. The decision log is in place (`NlSolution::report_json`, `Adaptive::report_json`) so the question can be reopened on real usage data; a model would still only choose a starting rung behind the verifier.
