# Lug Analysis: rigid-pin contact FE (`lug-solver`)

Status: shipped - elastic contact FE, plane-strain plastic collapse (ultimate capacity) and a bushing with a solved
interference fit, in `lug-solver` + the `app-tui` Lug Analysis toolbox. Everything below was measured on this repo's
code; numbers are from the tests named.

## Problem

A pin-loaded lug: plane-stress plate with a bore, loaded through a pin. The interesting physics is the pin/bore contact
(the contact arc changes with load, the lug "wraps" the pin, a clearance closes or an interference fit pre-stresses the
bore) and the stress concentration at the bore / net section. Requirements: fast enough to re-run on every edit, accurate
at the bore, non-linear in the contact.

## Options considered

| Option | Verdict |
|---|---|
| Mesh the pin too, solid-to-solid contact | Rejected: doubles the dofs and needs contact between two deformable meshes for no benefit - the pin is orders of magnitude stiffer. |
| Fixed cosine (or other) pin pressure on the bore | Rejected: it cannot wrap, ignores clearance/interference/friction, and mis-states the peak at the edge ligament. (The older `edge-check` FE uses it for the cheap models only.) |
| Rigid analytic pin + **node-to-surface** penalty contact (the proposed design) | Right idea, but on Q9 elements node-only contact oscillates (uniform pressure loads corner / mid-side nodes 1:4:1). Improved below. |
| Rigid analytic pin + **Gauss-point** contact on each bore edge | **Chosen.** Same economics (one reference point, exact gap `|x - c| - r_pin`), smooth pressure, correct peak at the arc ends. |
| Global-local submodel (coarse model drives a fine model) | Rejected: a graded mapped mesh in 2D reaches the resolution with a few thousand nodes; two coupled models add bookkeeping without need. |
| Plastic (J2) in the same solve | **Built** as an initial-strain iteration around the same condensed contact solve (see "Plasticity"). |

## Method

* **Mesh** (`mesh.rs`): a star-shaped mapped O-grid. Every node is on a ray from the hole centre, so the bore ring, the
  graded transition and the outline layer are one construction. Outline = rounded rectangle (round or square head, any
  corner radii); stations sit exactly on the outline's corner/tangent angles so edges conform. First-layer thickness is set
  per ray to the local tangential element width (aspect ~1:1 at the bore); layers grow geometrically per ray with the ratio
  solved so the stack reaches the outline in a common layer count with `q <= max_growth` (1.25). Q9 (`edge-check`'s
  element kernels, banded Cholesky). Default lug: 2,555 nodes, bore aspect 1.18, worst aspect anywhere 2.1.
* **Angular refinement** (`Refinement`, `auto_refinement`): a loose pin touches a narrow patch (2D Hertz half-width
  `b = sqrt(4 P' R* / (pi E))`). Fine stations only in the loaded sector, growing 25 % per element outwards, quantised so
  small load edits keep the same mesh.
* **Condensation** (`fe.rs`): the lug is factorised once; the bore Green's matrix `G = (K^-1)_bb` and `S = G^-1` are
  computed once (parallel back-solves). A half model (`y >= 0`) is used for axial loads, the full lug otherwise (folded
  node ordering keeps the band narrow on the closed ring).
* **Contact** (`contact.rs`): rigid circle, 3 Gauss points per bore edge, penalty `kn = 100 E/a` made exact by an
  **augmented Lagrangian** (3 multiplier passes), Coulomb friction by penalty-regularised stick with a slip cap
  `mu p`, history carried between load steps. Newton on `S u_b = f_c(u_b, c)` with an Armijo 2-norm line search;
  the tangent `S + K_c` is solved by **Woodbury** through `G` (rows = active Gauss points, ~100) instead of refactoring a
  288x288 dense matrix. Free perpendicular pin coordinate (oblique loads) is a bordered scalar.
* **Load** (`solve.rs`): displacement control on the pin plus a bracketing root solve on the travel to reach the target
  load; first the pin is settled to its force-free equilibrium (an interference fit leaves a net force at the centred
  position), then loaded. Steps are capped at 0.1 a; a failed Newton step is sub-stepped (friction is path dependent).
* **Recovery** (`stress.rs`): superconvergent patch recovery (Zienkiewicz-Zhu): per node a quadratic fitted to the 3x3 Gauss
  stresses of the elements sharing it (same material only); bore hoop/radial/shear by rotation; plane-stress von Mises.
  The same recovered field gives the ZZ energy-norm error estimate. The clamped far end (corner singularities that are
  not the lug) is excluded from the peak von Mises and the error estimate.

## Validation (all in the default test suite)

| Check | Result |
|---|---|
| Lame: rigid pin with interference in a free disc (exact) | mean pressure error 1.6 % / 0.13 % / -0.01 % / -0.06 % at 24 / 48 / 72 / 144 elements around; non-uniformity 13 / 1.6 / 0.47 / 0.06 % |
| 2D Hertz: loose pin (R_pin = 0.8 a), exact `b`, `p0` | uniform mesh peak error -25 % (72) -3.6 % (144) -2.1 % (288) -1.7 % (576 elements); **auto-refined 4,141 nodes: peak -2.0 %, patch width -1.3 %** (uniform 288 needs 15,317 nodes for the same error) |
| Equilibrium | sum of contact force = applied load (1e-4) |
| Half vs full model, axial load | peak hoop, peak pressure, pin travel agree within 0.2 % |
| Linearity with the contact arc fixed (interference) | pin travel and peak hoop increments double exactly |
| Penalty independence (augmented Lagrangian) | oblique peak hoop varies 0.015 % over a 300x penalty range (pure penalty: 1.6 %) |
| Fbru(e/D) rule | shared with `edge-check` (`mechanics_core::materials::fbru_at_edge_ratio`), unit-tested |

## Performance (this machine, `lug-solver` compiled at opt-level 3 even in dev)

Default lug (D 0.5, W 1.5, t 0.25): half model 5,110 dofs, bandwidth 145: assemble 4 ms, factor ~40 ms, condensation
~28 ms (threaded); one load/pin case ~30 ms (Newton ~25 iterations incl. friction root solve). Full-lug oblique load:
build ~0.6 s, case ~0.4 s (was 1.7 s before the Woodbury solve). The UI caches the condensed model, so editing only the
pin or the load re-solves in tens of ms. For comparison the existing bushing contact FE (`edge-check`, bushing + housing
meshed, fit then pin, plastic collapse) takes ~1.7 s per run - a different, larger problem, not a like-for-like.

## Plasticity (ultimate capacity)

`plastic.rs`. Elastic-perfectly-plastic von Mises by the initial-strain method: the elastic factorisation stays the
iteration matrix, the plastic strain field is an equivalent nodal load `F_p`, its displacement `K^-1 F_p` enters the
condensed contact equations as an offset of the bore (`S (u_b - v_b) = f_c`), Anderson mixing (depth 6) accelerates the
fixed point. The pin is settled at its force-free position (the fit may already yield the bore), then driven along the
load direction, converged at every step, until the load peaks (`LugModel::limit_load`); the collapse is the peak. It does
not depend on the applied load (the toolbox caches it while only the load changes).

**Plane stress cannot be used for ultimate capacity.** A yielding plane-stress bore cannot carry a radial stress beyond
the yield ellipse's extent, (2/sqrt 3) sigma_f, so the collapse is bore crush at about 1.15 sigma_f D t whatever the head
length (`lug_limit_load_plane_stress_is_bearing_crush_limited`): it masks net-section and shear-out. The ultimate analysis
therefore runs plane strain (`PlaneMode::Strain`, 3D J2 radial return of the deviator, stiffness assembled with the
effective constants `E/(1-nu^2)`, `nu/(1-nu)`), at the flow stress rule `edge-check` validated (`(Ftu + Fty)/2`, capped by
`sqrt 3 Fsu`, never below yield), exactly as that model does. The elastic stresses stay plane stress.

| Check | Result |
|---|---|
| Exact: rigid pin in a thin plane-stress ring (full plastic ODE) | within 4 % |
| Exact: rigid pin in a thick ring (saturates at the yield-ellipse cap 2 sigma_f/sqrt 3) | +1.9 % |
| No plasticity (huge flow stress) | identical to the elastic contact solution |
| Collapse proportional to the flow stress | ratio 1.97 for a doubled flow stress |
| NACA TN 1503 pin-bearing tests (12 published points, 6 alloys x e/D 1.5, 2.0) | **all 12 predicted low, 9 to 28 %, mean about -18 %**; never over-predicts; e/D 1.5 -> 2.0 strengthening within 12 % |

Same 12 points with the `edge-check` contact FE: -0.6 to -20 %, mean -10 %. The rigid pin lacks the load-spreading of a
deformable steel pin, so the lug solver is about 8 points more conservative. The bias is not corrected; the toolbox says
so next to the ultimate margin. Per case: 75S-T -9..-12 %, 24S-T -24..-28 %, 14S-T -15..-19 %.

## Bushing

`BushingSpec`: inner diameter, material, diametral interference, interface friction. The mesh adds uniform bushing
layers (2-6) inside the hole on their **own nodes**, so the bushing-to-hole interface is a conforming contact (three Gauss
points per edge pair, normal penalty + augmented Lagrangian, initial overlap = the interference, Coulomb friction with
a slip reference). The condensed contact set is the pin surface plus both interface surfaces; the bushing is a free body
held only by that contact, so its nodes get 1e-6 E grounding springs. The pin then bears on the bushing; the bushing stays
elastic in the collapse analysis.

| Check | Result |
|---|---|
| Exact two-cylinder Lame press fit (bushing in a disc) | fit pressure +0.02 % / +0.03 % (two meshes), spread 0.01 % |
| Zero interference / clearance | no fit pressure |
| Stiffer bushing | more fit pressure (monotone) |
| Half vs full model, bushed lug | lug hoop 0.003 %, fit pressure exact, bushing peak von Mises 0.4 % |
| Equilibrium of the pin contact | load balanced to 2e-4 |

Interface friction matters: on the default lug it moves the lug's peak hoop stress by 9 % (37.6 -> 40.9 ksi), so it is an
input, not a detail. A bushed lug costs more than a plain one (more contact dofs, friction iterations): about 0.45 s per
elastic case and 1.7 s for the collapse on the default lug (plain: 0.02 s and 0.24 s).

### Against the `edge-check` bushing contact FE

`tests/compare_edge_check.rs` (ignored, ~15 s): the same bushed housing near a free edge (bore 0.5, bushing bore 0.375,
interference 0.001 diametral, 7075 housing, bronze bushing, t 0.5, friction 0.2, plane strain, flow 73.5 ksi) in both:

| e/D | edge-check | lug-solver | ratio | time (ec / lug) |
|---|---|---|---|---|
| 1.5 | 27,889 lbf | 27,322 lbf | 0.98 | 3.5 s / 1.0 s |
| 2.0 | 34,633 lbf | 35,664 lbf | 1.03 | 3.3 s / 1.2 s |
| 3.0 | 44,218 lbf | 45,789 lbf | 1.04 | 3.1 s / 1.1 s |

Two independent discretisations (a rectangular plate pushed by a rigid cylinder versus the mapped lug with the condensed
rigid pin) agree within 4 %, which corroborates both. The lug solver is faster per solve at the existing
model's default mesh, **but the app's `edge-check` runs a coarser tuned mesh (~1.7 s for the whole fit/edge profile) and
owns profile tables, Monte Carlo, caching and the recommended-edge-distance fixed-point test, all validated together.**
Replacing it would mean re-validating that machinery for a speed gain that is not demonstrated end to end, so it was not
replaced. If the contact FE ever becomes a bottleneck, the lug solver is a validated drop-in candidate for the collapse
solves.

## Load angle

Every angle 0-180 deg (plain, 1 and 4 kip, 15 deg steps, mirror pairs +-theta equal within 3 %) and a bushed lug at 0/45/90 deg
solve; collapse at +-45 deg agrees within 5 %. 45 deg used to fail: secant extrapolation in the pin-travel root solve, then a
neutral sideways pin DOF at first touch, then (60/75/90 deg) Newton non-convergence cured by an `advance_nested` sub-step
fallback. The oblique load needs ~0.055 in of pin travel at 4 kip (above the 0.1 a note threshold), so the large-displacement
note appears there. Details and rules: `lug-solver/AGENTS.md` Pitfalls. Timings (bushed full ring, 6336 nodes): 1.6 s axial,
5-6 s at 30/45 deg, 3 s at 90 deg.

## Flow-stress rule (the collapse-load accuracy lever)

The collapse of a perfectly plastic model is proportional to its flow stress, so the NACA under-prediction is the rule, not
the solver (the elastic pin changes nothing; hardening capped at Ftu cannot reach the tests). Same solver, 12 points:

| Rule | Mean error | Range |
|---|---|---|
| (Ftu + Fty)/2 | 18 % | -9 to -28 % |
| Ftu (default) | 10 % | -1 to -17 %, never high |
| Ftu (1 + elongation), Table I elongations | 4.3 % | -5 to +10 %, 75S high |

The error follows strain hardening (worst for 24S-T, Fty/Ftu 0.71): the tests need the true stress at large local strain.
Closing the rest needs finite-strain kinematics and a ductile failure criterion: `docs/finite-strain-collapse.md`.

## Upgrade pass: accuracy and robustness (`docs/lug-solver-upgrade-plan.md`)

Fifteen items were planned; this is what each did, measured. Tests named in `lug-solver/tests/`.

| # | Item | Outcome |
|---|---|---|
| 13 | Solution self-checks (`Verification`) | Force balance (1e-5), the discrete energy identity (`U + ground = 1/2 f.u`, 1e-12), penetration, friction cone, grounding leak, ZZ error. **Found a real defect**: the 1e-6 E grounding springs of the bushing leaked 1.4 % of the load; now 1e-9 E (leak 0.016 %, reported). |
| 5 | Patch recovery + ZZ estimate | Smooth nodal stresses; the estimate falls with refinement (2.9 % -> 2.3 %, oblique 17 % -> 4 % at 144 elements). It showed the default mesh under-resolves the von Mises peak of oblique loads (peak hoop converges within 3 %, the von Mises peak moves 10 %). |
| 14 | Sweeps | `solve_sweep` / `collapse_sweep`: one condensed model, parallel angles (capacity envelope). |
| 15 | Speed | 4-accumulator dot in the banded Cholesky (9 %), parallel sweeps. Profiling showed the cost is the *number* of tangent factorisations (~600 x 6 ms on the bushed model), not the product kernels, so no parallel Woodbury. |
| 6 | Locking | Plane-strain ring collapse is exact `(2/sqrt 3) sf ln(R/a)`: FE -0.3 %. The Q9 mesh does not lock; no change. |
| 2 | Hardening | Multilinear / Ramberg-Osgood law, plane strain, deformation-theory return map (exact in simple shear), collapse at a failure strain. Validated by the return map in shear, flat-law == EPP, and NACA TN 1503: 10 % strain gives 7-24 % low (mean 16 %) against 9-28 % (18 %) perfectly plastic. |
| 3 | Consistent-tangent plasticity | **Not built, measured instead**: the whole default collapse is 42 iterations / 0.16 s with Anderson; a Newton with the consistent tangent needs a ~1000-row Woodbury (~0.15 s) per iteration, so it is slower. |
| 11 | Consistent Coulomb friction | The tangential Alart-Curnier multiplier replaces the pure-penalty stick: a sticking point no longer creeps by `t/kt` (test: 100 % creep -> < 10 %). The old results were **4-8 % low** in peak hoop at 45-90 deg (28.4 -> 29.7 ksi, 18.7 -> 20.2 ksi). `STICK_RATIO` is now only a convergence rate (0.005; 0.005-0.03 agree within 0.3 %). |
| 12 | Bushing rigid modes | The leak is now measured and 1e-9 E costs no speed once friction is consistent; a constraint formulation would add nothing. |
| 4 | Interface pressure | Gauss-point constraints on quadratic edges gave a 3-point sawtooth (+-60 %, roughness 0.37) and false separation; nodal collocation with lumped edge weights (1/6, 4/6, 1/6) gives 0.004 at the same mean and peak. Pin contact (analytic circle) only has edge-point noise (~1 %), unchanged. |
| 10 | Load controller | Not replaced by a load-controlled bordered Newton. Improved instead: one multiplier pass for trials far from the target, halve-the-step before the nested sideways-coordinate search, no forced growth near the target: bushed 45 deg 8.5 s -> 1.9 s, plain 0.12 s -> ~0.05 s. |
| 1 | Elastic pin | The pin disc's surface compliance `A` is added to the bore Green's matrix (`G~ = G + A`) and the contact is unchanged (rigid circle against `u_lug + A f`). Exact two-cylinder Lame press fit: 0.2 %. Steel pin lowers the edge pressure peak 3-9 %. **It does not change a limit load** (as theory says; NACA 18.0 -> 18.2 %): the NACA gap is not a rigid-pin artefact. |
| 9 | Thermal | `Thermal`: free expansion changes the bushing interference `dT D (a_b - a_l)` and the pin size against its seat; fit pressure is linear in the net interference (2 %). Structural thermal stress is not modelled. |
| 8 | Through-thickness | `through_thickness`: pin as a Timoshenko beam on the lug's own `P'(s)` curve, double shear. Stiff pin: uniform and the statics moment `P(t/8 + c/2)` (2 %); t/D 1.8 on a steel pin peaks 1.46 x the mean. Single shear is deliberately absent (net couple only the members' bending can balance). |
| 7 | Second order | `solve_second_order`: geometric stiffness of the converged stress added to the tangent, to a fixed point. Beam-column check, u = 0.8: tension 0.786 (beam 0.797), compression 1.282 (beam 1.346, shear flexibility). The mesh is not moved. |

## What the toolbox reports

Contact patch, peak pressure, pin travel, peak hoop (and where), peak von Mises, net-section stress, Kt(net); with a
bushing also the unloaded fit pressure, the interface pressure and the share that has lost contact, the bushing's own peak
stresses; and checks against the material: bearing `P/(D t)` vs `Fbru(e/D)` (only where tabulated, never guessed),
nominal net section vs `Ftu` (axial component), first yield (peak von Mises vs `Fty`), peak elastic hoop vs `Ftu`,
bushing stress vs the bushing yield, the fit, and the **ultimate margin** `P_collapse / P - 1`. The elastic peaks are
indicators: above yield they mean local plasticity, not failure (the readout says so).

## Not done (deliberately)

* **Updated-Lagrangian large displacements.** Contact normals follow the pin and the optional second-order analysis adds the
  geometric stiffness, but the mesh is not moved; a note is shown when pin travel exceeds 0.1 a. A *free* body that can
  spin freely (a loose disc) is neutral to rotation; real lugs are clamped and bushings are held by the interface.
* **Bushing plasticity, rate effects, hardening in plane stress.** Hardening is plane strain (the collapse model) only.
* **Single-shear pin bending, structural thermal stress, a consistent-tangent plastic Newton, a load-controlled
  bordered Newton** (see the upgrade table for why).
* **Validation against published lug tests.** The NACA TN 1503 pin-bearing data (plate with edge distance, no bushing) is
  the published test set used; lug-specific test data is not in the repo.
* **Replacing the `edge-check` bushing contact FE** (see the comparison above).


## The general kernel is the main solver (Phase 7 of `docs/fea-core.md`)

The toolbox runs every analysis on `fea-core` (`FeaLug`) by default: elastic contact with friction, bushing with its
interference fit, thermal change of the fit, second order, meshed elastic pin, oblique / hardening / finite-strain collapse,
sweeps. The condensed solver and `FiniteLug` described above remain as `Solver: Legacy` and `Solver: Compare` and as the fallback.
What changed for the reader of a result: friction is path-resolved on the kernel (the legacy hoop stress is ~8 % lower at 45 deg and
friction 0.15); the kernel's collapse loads are ~3 % above the legacy ones (NACA TN 1503: `(Ftu + Fty)/2` 6-25 % low, `Ftu` from 13 % low
to 2 % high, `Ftu (1 + elongation)` -2 to +14 %); oblique collapse loads are mesh dependent (use the `Mesh Size Test` row, which
includes the collapse). Mesh settings (elements around the bore, growth ratio, first-layer aspect, contact refinement) live in the
collapsible `Mesh` section. Single shear and a converged face bearing for pin bending are still not offered (see `docs/fea-core.md`
Phase 7 for why).
