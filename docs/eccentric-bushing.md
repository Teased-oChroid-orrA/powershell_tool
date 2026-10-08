# Eccentric bushing toolbox and FEA core upgrade: plan

Status: BUILT and verified (2026-10-06 and 2026-10-07). Section 8 is the first session's record (its half-cosine pin load is SUPERSEDED); section 9 is the second session: the realistic pin, the kernel upgrades (follower loads, arc-length contact, anisotropy, rotation, a consistent frictional tangent), every item of the improvement list below with its outcome, and the bugs found on the way. Written after a read of `bushing-solver`, `edge-check`,
`lug-solver/src/fea*.rs`, `fea-core` (AGENTS + `docs/fea-core.md` limits) and a literature/web check. Evidence labels:
PROVEN (read in code / tested), SOURCED (external), HYPOTHESIS (must be disproved by a test before it is relied on).

## 1. What "eccentric bushing" means here

An eccentric bushing has its bore offset by `e` from its outer-diameter centre. Rotating it moves the pin position by up
to `2e` (used to take up hole misalignment). SOURCED (aircraft patents: landing-gear stay, Boeing anti-rotation filing):
the shear load on the pin makes a moment `F e sin(phi)` about the OD centre, which tends to rotate the bushing in its
housing; the installation must stop that, by interference + friction, or by lock plates/teeth/coated washers. No MMPDS or
NASM standard for eccentric bushing analysis was found: this is first-principles engineering, to be labelled as such in
the UI and the report ("design guide, not a certification value", same wording as Edge check).

## 2. Physics to implement (derivation, to be verified by tests before use)

Bushing equilibrium about the OD centre `O`, plane problem per unit thickness (thickness `L` multiplies):

* Pin load `F` acts on the bore, through the bore centre `B = O + e u`, direction `d` (angle `phi` between `u` and `d`).
  Moment about `O`: `T_req = F e sin(phi)`. (Normal contact forces from the housing are radial through `O`: no moment.)
* The only torque the housing can return is interface friction: `T_cap = integral mu(theta) p(theta) R^2 L d theta`
  (all points at the Coulomb limit), where `p` is the interface pressure AFTER fit and load. Uniform p gives the
  classical `T = 2 pi mu p R^2 L` (SOURCED: Calistrat, hydraulically fitted hubs; shaft-hub literature). `p` is NOT
  uniform for an eccentric fit: the wall `R - r(theta)` varies from `t - e` to `t + e` (`t` = mean wall), so the fit
  pressure is higher where the wall is thick, and the load adds a cosine-like redistribution with contact loss on the
  unloaded side. So `T_cap` needs the FE pressure map; Lame is only the `e = 0` limit and the first guess.
* No spin iff `F e sin(phi) <= T_cap(e, F, phi)`; the worst case is `phi = 90 deg` (load perpendicular to the offset
  line), which is the default design case; any `phi` is an input/sweep.
* **Maximum offset** `e_max`: root of `F e - T_cap(e, F) = 0` (monotone in practice: HYPOTHESIS, test by sweep), bounded
  above by `e < t - t_min` (wall left, from the existing `min_wall_straight`). Also report the **maximum load at a given
  offset** (`F_spin = T_cap / (e sin phi)`, the same solve) and the margin `T_cap / T_req - 1`.
* Dry static friction is the capacity (Coulomb). Interface friction, interference band and temperature each have a
  band: the design value is the lower bound over the toleranced fit (min interference, max bore), the way
  `bushing-solver` already ranges pressure (`pressure_range`).
* HYPOTHESIS to disprove early: a pump-bearing patent claims an eccentric wall is "self-locking" (thick part rotating into
  the thin side). In an interference fit with a rigid-body rotation the wall geometry is unchanged by rotating, so the
  pressure map just co-rotates; there is no geometric locking. A test that rotates the fit by 90 deg and re-solves must
  give the same `T_cap` (rotation invariance) and the torque-vs-angle curve of `p` must be the only effect.
* Axial/3D effects (end effects, housing length) are out of the first version: a plane model with the bore length as
  thickness, stated in the report. CORRECTION (2026-10-06, during the build): the default is **plane stress**, not
  plane strain as first written. A bushing with free ends has `sigma_z ~ 0`, which is exactly the open-end Lame
  assumption `bushing-solver` makes, so plane stress keeps the FE fit pressure equal to the Bushing toolbox's (verified:
  0.004 % from the two-cylinder Lame). Plane strain (axially constrained bushing) is an explicit input.

## 3. Architecture (reuse, single source of truth)

| Piece | Owner | Reuse |
|---|---|---|
| Concentric Lame pressure, tolerance band, materials | `bushing-solver` / `mechanics-core` | unchanged; used as the `e = 0` reference and the starting guess |
| Bushing + housing plane-strain contact with interference + Coulomb friction + free rigid pin | `fea-core` (`solve_nonlinear_contact`, `ContactSpec::two_pass`, `with_overlap`, `with_friction`, `RigidMaster::with_free_translation`, `Start`) | exactly the Lug `FeaLug` machinery; PROVEN on a round disc to Lame +0.00 % |
| Geometry/meshing with an offset circle | `fea-core::mesh2d` (`geometry.rs` lines/arcs/holes, named segments) | housing disc with bore, bushing annulus with eccentric inner circle; both unstructured Quad9 |
| Solver: spin capacity and `e_max` | NEW crate `eccentric-bushing/` (depends on `fea-core`, `mechanics-core`, `bushing-solver`) | NOT in `bushing-solver` (its contract is Lame-only, differential-tested to the TS engine) and NOT in `edge-check` (its contact FE is a banded O-grid solver built for a free edge) |
| UI | `app-tui/src/toolboxes/eccentric_bushing/` | clone the Bushing toolbox field/picker patterns; take geometry/material/friction defaults from the Bushing toolbox's model where possible (shared state decision below) |

Why a new crate and not `lug-solver`'s `FeaLug`: `FeaLug` is built on the lug's own O-grid mesh (concentric, a free-edge
lug). The only reusable parts (`Tuning`, the contact set-up in `contacts()`, `fit`) are small; they should be LIFTED into
`fea-core` (or `fea-problem`) as a bushing-in-housing contact builder that both `FeaLug` and the new crate call, rather
than copied (Fix-Everything: no second copy). Scope of that refactor is item 3.1 below and is the first code change.

Housing model: a round boss of outer radius `R_o` (free outer surface) by default (matches `bushing-solver`'s housing
width). A lug/edge housing (the offset next to a free edge) is a second phase: it reuses `edge-check`'s plate geometry.

Solution procedure per `(e, phi, F)`:
1. Mesh (ring refinement toward the interface), assign materials, install fit (stage 1: pin absent, interference only).
2. Stage 2: pin as rigid master with free translation, force-controlled `F` along `d`, 2-3 load steps (friction needs
   at least 2-3 steps: measured +8 % error with 1, `docs/fea-core.md`).
3. Read the interface pressure `p(theta)` and the friction state; integrate `T_cap`; `T_req = F e sin(phi)`;
   report slip fraction (share of interface at the Coulomb limit) and the margin.
4. `e_max`: bracket on `[0, e_hi]`, secant/Brent on `g(e) = T_cap(e) - F e sin(phi)`; each evaluation is a full contact
   solve, so the cost is the budget item (see 3.2: reuse the mesh topology, warm-start `Start`).
5. Tolerance band: run at the band corners (min/max interference, max/min bore, friction low) and report the worst.

Verification plan (numerical changes need numerical proof):
* `e = 0`: `p` equals the two-cylinder Lame pressure (existing test pattern), `T_cap = 2 pi mu p R^2 L` exactly (rel 1e-3).
* Rotation invariance: rotate `u` by an arbitrary angle at zero load: same `T_cap` to mesh noise.
* Closed form for a thin eccentric ring limit if one exists (Kirsch/Jeffery-type solution for an eccentric annulus under
  uniform pressure; a literature check is the first task of the solver crate) - otherwise a mesh-convergence series.
* Mesh convergence (Mesh-Size-Test pattern from Lug Analysis) and `p` penalty independence (0.01-0.1 `eps_n`).
* Differential: concentric `FeaLug` bushed run vs the new builder (same fit pressure) while the builder is shared.
* Torque balance: with the pin force applied, the FE friction traction integral about `O` equals `F e sin(phi)` below
  capacity (global moment equilibrium; a check on the solver, independent of any formula).
* Monotonicity of `T_cap - T_req` in `e` over the range (disproves/proves the single-root assumption).

## 4. Improvement list: FEA core and toolboxes that use it

Ranked by (benefit to this feature) then (risk). Each item needs a measurement or a closed-form test before it is
called done; none is started.

### 4.1 Needed by the eccentric bushing toolbox (do first)
1. **Shared bushing-in-housing contact builder** in `fea-core`/`fea-problem` (interference, two-pass friction contact,
   rigid free pin, load-step schedule); `FeaLug::contacts`/`Tuning` call it. Removes a latent duplicate and gives the new
   crate a validated path. Test: `FeaLug` results unchanged to 1e-9 on `lug-solver/tests/kernel_*`.
2. **Rigid rotation of a rigid master is not supported** (kernel limit). Not needed here (the bushing is deformable and
   held by friction; the pin only translates) but a *torque-controlled* slip test would need it. Decision: capacity by
   integrating `mu p R^2`, verified against an explicit moment-equilibrium check, instead of simulating the slip. Revisit
   only if the comparison shows the all-slip assumption is wrong (slip is local/progressive).
3. **Warm-started parameter sweeps over geometry**: each `e` changes the mesh. Reuse the symbolic factorisation when the
   node/element topology is identical (mesh `e` by moving nodes of a fixed-topology mesh); `solve_static_many` already
   shares one factorisation across load cases. Measure first: `NL_PROFILE=1`.
4. **Interface pressure/traction output** from `ContactSet` as a first-class result (position, normal, tangential, state)
   instead of reading Gauss-point pressures: documented scatter of about +-25 % pointwise on non-matching deformable
   meshes, so the integral must come from nodal/consistent traction. Add and verify: `integral p dA` equals the
   transmitted force and equals the Lame resultant at `e = 0`.

### 4.2 Robustness (kernel)
5. Contact pattern is static (built once from the initial proximity, `margin`): a contact reaching outside is an `Err`.
   Add automatic pattern extension (rebuild + refactor on that error) so a user input cannot fail on a sliding interface.
6. Friction needs a moderate penalty: sticking multipliers converge slowly. Investigate a consistent friction tangent
   (`mu dp` coupling stalled Newton when added naively; try it inside the AL pass only) - measure iterations on the lug
   friction cases before and after.
7. `check_constrained` is only run on linear solves. Run the same rigid-mode check inside the nonlinear/contact driver
   (a free body with no contact is already an error for free translations; an under-supported body held only by contact is
   the Lug `Loads::ground` springs hack - measure how much leak those springs introduce, `k u`, and bound it in a test).
8. Tri3/Tet4 poor on pressure problems: guard rail so `fea-problem` meshes with them only when asked and warns (Workbench).

### 4.3 Accuracy
9. Edge-check contact FE (banded solver, own contact) and the new kernel bushing contact now both model "bushing in
   housing": run the concentric kernel model against `edge-check/src/contact.rs` at identical inputs and tabulate the
   difference once (a cross-check, not a port: the NACA-validated bands must not move). If they disagree beyond mesh error,
   the larger one is investigated, not hidden.
10. Pointwise pressure scatter on non-matching deformable interfaces: mortar-like or segment-to-segment integration is a
    large item; first try `ContactRule` choices on the conforming bushing/housing interface (conforming meshes exist
    here) and report the pressure error vs Lame as a regression metric.
11. Plane stress vs plane strain choice (bushing long -> strain): make the mode explicit in every kernel bridge report.

### 4.4 Speed
12. faer is sequential below 40k dofs (measured: its threads cost more than they save); the 2D contact model here is
    ~5-15k dofs, so the budget is dominated by Newton/AL iterations (60-90 factorisations for a lug). Candidates, each to be
    measured with `NL_PROFILE=1` and the bench: reuse a factorisation across AL passes while the contact set is unchanged
    (the edge-check solver does this with `REUSE_DIFF`), chord/modified Newton inside a pass (tried and rejected for the
    lug once evaluation was cheap - re-check for friction), coarser mesh away from the interface (`mesh2d` size field).
13. Assembly SIMD / tiled stiffness: deferred in the kernel notes until a profile shows assembly matters. A profile of the
    eccentric sweep decides; do NOT do it speculatively.
14. Parallelise independent solves (tolerance-band corners, `phi` sweep) with rayon at the sweep level: the lug sweeps
    already do this (`load_sweep`, `collapse_sweep`).

### 4.5 Toolboxes using the kernel
15. FEA Workbench is linear-static only: expose contact (+ friction + interference) authoring, then the eccentric bushing
    problem is reproducible as a Workbench `Problem` file - a cheap independent check of the new crate (same JSON, solved
    through `fea-problem`). Scope-limited: only after 4.1.
16. Lug Analysis: reads its bushing fit from its own `BushingSpec`; the eccentric offset case shares geometry types with
    the new crate. Do not add eccentric support to Lug Analysis in this pass; note it as a follow-up.
17. Bushing toolbox: add an **Eccentric** results section/button only if the toolbox stays a bridge (no formula here):
    `bushing-solver` provides `T_cap` for the concentric limit, the new crate everything else.

## 5. Product decisions (defaults chosen; user may override)

* Load case: transverse pin shear `F` at the bore centre, default worst case `phi = 90 deg`; optional pure torque input.
  Default `F` = the Bushing toolbox's `model.load`.
* Housing for v1: round boss (outer radius), free outer surface. Edge-limited housing is v2.
* Output: `e_max` (spin-free), `F_spin` at the entered offset, margin, pressure map `p(theta)`, friction utilisation,
  contact-loss arc, wall at thin/thick sides, tolerance-band worst case.
* Spin is judged on the **friction capacity at the interface only**. Lock plates, teeth, washers, bonding are out of scope
  (flagged in the report as ways to exceed this limit).
* Imperial units only, as `bushing-solver` (a metric entry is a separate decision).

## 6. Work order

1. Literature check for a closed form for an eccentric annulus under interface pressure (verification reference).
2. Item 4.1.1 (shared builder) with the lug regression suite as the safety net.
3. `eccentric-bushing` crate: geometry -> fit -> load -> `T_cap`; tests of section 3; `e_max` solve; tolerance band.
4. Measurements (4.4) on that crate; apply only what the profile supports.
5. Toolbox (`app-tui`), real-terminal check in tmux (Caps-Lock rule for every single-letter binding), docs, AGENTS.md
   nodes (`eccentric-bushing/`, toolbox node), ADR only if the crate boundary changes.
6. Cross-checks 4.3.9 and 4.5.15; `code-review`, `simplify`, `intent-layer-compound`.

## 7. Open uncertainties

* UNKNOWN whether `T_cap` from the all-slip integral matches the true slip onset when pressure is strongly non-uniform
  (progressive slip can start before all points reach the limit; the capacity is still the integral, but the load at
  which a *finite rotation* begins may be lower with softening friction). Settled only by the moment-equilibrium test.
* UNKNOWN real friction values for installed aircraft bushings (the existing friction picker applies).
* The previous session's "30571 psi on a 10000 psi plate" question is closed: the toolbox reports the maximum stress, not
  the far-field stress, so the value is the Kt peak at the hole (user confirmed the misunderstanding).

## 8. Built: evidence and departures from the plan (2026-10-06)

**Built**: `fea-core/src/fit.rs` (shared `Tuning`, `interference_contacts`, `start_after_fit`; `FeaLug` now calls them, the 15 lug kernel tests
incl. the Lame press fit are unchanged), crate `eccentric-bushing/` (9 verification tests), toolbox `app-tui/src/toolboxes/eccentric_bushing/`
(inputs shared with the Bushing Workbench; keys `r` `m` `l` `d` `e`; checked in a real terminal at 170 and 80 columns).

**Verified** (`eccentric-bushing/tests/verification.rs`):
* Concentric limit: FE fit pressure 14135.8 psi vs two-cylinder Lame 14136.4 (0.004 %); `T_cap` = `2 pi mu p R^2 L` to 3e-6.
* Global equilibrium of the eccentric solution, independent of any formula: interface force returns the pin load (1501.5 for 1500 lbf) and the FE friction moment returns `F e sin(phi)` (59.79 vs 60.0, 0.35 %).
* Mesh independence: capacity identical to 4 digits from element size 0.05 to 0.022 (10k to 31k dofs); graded automatic mesh vs uniform 0.02 on a thin wall: 281.855 vs 281.858.
* The pressure is higher on the thick side (15820 vs 9975 psi at e = 0.04); the capacity falls with the offset (414 -> 392 lbf in at e = 0.04); mirror loads agree.

**Departures and findings**:
* **Pin load is a half-cosine bearing pressure, not a rigid pin contact** (section 3 said rigid pin). A zero-gap rigid pin on the as-fitted bore is a degenerate free-body contact (singular / no convergence at some mesh sizes for either pin radius choice), and a pin of the nominal ID size starts with interference (the fit closes the bore) and silently adds pressure that does not depend on the load (`T_cap` jumped 414 -> 684 and ignored `F`). The cosine load is `edge-check`'s idealisation of the same pin and was robust at every size tried.
* **Plane stress is the default** (see the correction in section 2).
* **Design capacity = min(fit alone, with the pin load)**: the pin's bearing pressure is one-sided, so its mean over the bore is not zero (`2F/(pi^2 r_i t)`, about 2100 psi at 1000 lbf) and squeezes the bushing; the loaded capacity is +2 % to +55 % above the fit's. My first reading of this as an artifact (a symmetry argument for `F -> -F`) was wrong: the load for `-F` is not the negative of the load for `+F`. The credit rests on the assumed pin pressure distribution, so none is taken; contact loss on the unloaded side is still caught by the loaded run.
* **Hard wall floor**: the contact solve does not converge reliably below a thin wall of ~2 % of the bore (verified down to 2.5 %), so the model needs >= 3 % (`MIN_WALL_FRACTION`); the Bushing Workbench's `min_wall_straight` is carried as `min_wall` and reported separately (`wall_ok`, `OffsetLimit::wall_limit`): with the default inputs the minimum wall, not spin, limits the offset.
* **Friction must be >= 0.01**: at 0.001 the friction contact does not converge (and carries nothing).
* **Speed** (profile, `NL_PROFILE=1`): one fit solve at e = 0.04 is 3.4 s, 80 factorisations = 2.0 s of it (60 %), evaluation 1.1 s, solves 0.13 s; so the answer is inherent per solve and the lever is the number of solves: `max_offset` is an 8-section search (parallel solves, 2 rounds for 2 %) on the fit alone, confirmed by one loaded run: about 20 s on 8 cores for the default inputs (it was ~80 s sequential bisection in the test).
* Not done from the list: 4.1.2 (rigid rotation not needed: capacity by integration, verified by the moment check), 4.1.3 (topology-reusing sweeps: the search is parallel instead), 4.1.4 (traction output: the integrals agree to 0.35 % with equilibrium), 4.2-4.5.
* Open: the ~2-10 s solve of a thin-wall case is the cost of friction AL passes; a friction-free fit stage (pressure only, friction only in the load stage) was not tried. The housing is a round boss on two point supports; an edge-limited housing is v2.

## 9. Second session (2026-10-07): realistic pin, kernel upgrades, every list item closed

**The half-cosine bearing pressure is gone.** The pin is a meshed elastic disc in frictional deformable contact with the bushing bore (`pin`, `pin_friction`, `pin_clearance_dia` to the bore as fitted), loaded through its length by the body force of the lug's elastic-pin model, free to rotate; its pressure on the bore is whatever its stiffness, clearance and friction make it (steel pin, 0.001 in clearance, 1500 lbf on a 0.375 in bore: 95 degrees of contact, 12 ksi peak; 0.004 in clearance: 50 degrees, 23.5 ksi; no clearance: 180 degrees). Torque demand stays `F e |sin(angle)|` exactly: a free pin adds no net torque, so pin friction changes the pressure distribution, not the torque the interface must return (tested). The capacity basis is an input: with the pin loaded (default, the pressure the real pin produces) or fit alone (conservative); both are always reported.

**Why it took a two-stage scheme** (what failed, in the order tried): load control from a touching pin (a free body with nothing to push against: enormous Newton steps), a rigid pin (degenerate zero-gap contact), a seat overclosure (its own force dominates light loads, and a smaller seat drowns in mesh geometry error), arc-length steps (they can leap the bore and the corrector stalls on friction chatter). What works: stage A presses the pin in as a rigid body by prescribed displacement until it carries a quarter of the load, stage B releases it and ramps the body force from that level to the full load. That needed `NlOptions::stop_at_force` and `Start::lambda`.

**Bugs found on the way** (kernel, not the toolbox): (1) the fitted-bore centre was the mean of the displaced bore nodes, biased 1.5 mm by non-uniform spacing: every eccentric loaded run failed; now a least-squares circle; (2) the relative convergence tolerance used `max(|f|, lambda |f_ext|)`, but `f` cancels at equilibrium, so a run started from a converged fit could never meet it; now the elastic internal force is the force level; (3) the tangent predictor for prescribed displacements included the near-contact stabiliser and dragged approaching bodies together; (4) a converged run that carried none of the pin load (a silent wrong answer) is now an `Err`; (5) the closest-point projection's absolute `1e-14` tolerance: contact points silently left contact when coordinates were of order one (found through the Workbench's bushed lug: a hole at (1, 1) failed after 60 s, at the origin it solved in 2 s). Each has a regression test.

**Direct spin simulation** (`spin_onset_torque`, optional `direct_onset`): a pure torque on the bore as a uniform tangential traction, raised under arc-length control with contact. The load factor shows a sharp knee. Concentric: knee 418.5 vs the integral capacity 416.35 lbf in (+0.5 %). `e = 0.04` (wall 0.0225 / 0.1025): knee 377.8 vs 391.6 (-3.5 %): the all-slip integral over the fit pressure overstates the onset by 3-4 % when the wall varies 3:1, never understates it. This settles the open question of section 7 (UNKNOWN #1): the integral is exact for a uniform interface and slightly unconservative for a strongly eccentric one; `direct_onset` caps the capacity at the knee.

**Improvement list, one line each** (section 4 numbering):

| Item | Outcome |
|---|---|
| 4.1.1 shared builder | Done (session 1); now also used by the Workbench |
| 4.1.2 rigid rotation | Done: a 2D circular master rotates under a moment; torque equilibrium to 3 % at 50 % and 60 % of the capacity (slows near the limit, documented) |
| 4.1.3 topology-reusing sweeps | Declined with a measurement: set-up (mesh, pattern) is ~3 % of a solve; the offset search is a parallel 8-section search instead |
| 4.1.4 traction output | Done: `NlSolution::contact_tractions` (position, normal, pressure, friction, weight, slipping); the eccentric crate and the Workbench read it; integrals equal the transmitted force to 1e-3 |
| 4.2.5 contact pattern extension | Done: retry with doubled margins |
| 4.2.6 friction tangent | Done: defect-correction refinement; the case that stalled Newton (pin on an eccentric fit) converges |
| 4.2.7 constraint check / ground leak | Done: floating bodies refused; `ground_leak` reported (the eccentric tests assert it below 0.2 % of the pin load) |
| 4.2.8 Tri3 / Tet4 guard | Done: a caution in the Workbench summary and report |
| 4.3.9 cross-check against `edge-check` | Done: the same concentric plane-strain fit, kernel vs the banded NACA-validated contact FE vs Lame, agree within 1 % / 1.5 % (`tests/cross_check.rs`) |
| 4.3.10 pressure scatter | Measured, rule kept: reduced collocation 0.02 % pointwise scatter concentric, full Gauss 0.03 %, nodal does not solve |
| 4.3.11 plane mode in reports | Done for the toolbox and the Workbench report (`Analysis:` line, axial condition in the eccentric report) |
| 4.4.12 factorisation reuse | Declined with a measurement (153 vs 145 factorisations with chord Newton) |
| 4.4.13 SIMD / cached stiffness | Declined with a measurement (element assembly is 9 % of a contact solve) |
| 4.4.14 parallel sweeps | Done: the section search runs candidates in parallel |
| 4.5.15 Workbench contact authoring | Done: interference-fit bushings (eccentric offset, friction, contact solve, interface results, a template, a UI section); a concentric bushing in a large plate gives 8231 psi vs Lame 8267 (0.4 %) |
| 4.5.16 Lug Analysis eccentric | Not done on purpose: the Workbench's bushed plate and the eccentric toolbox cover offset bores; Lug Analysis has its own validated O-grid |
| 4.5.17 Bushing toolbox section | Not done on purpose: a separate toolbox sharing the Bushing Workbench's inputs is the better home |
| edge-limited housing (v2) | Done: `Inputs::edge_distance` (plate, free edge on the `-x` side); the fit pressure is lower on the edge side and the capacity below the infinite boss (tested) |
| anisotropic housing / bushing / pin | Done in the crate (`Elasticity::ortho`: in-plane orthotropy with axes at an angle; isotropic-equivalence 1e-4 and the pattern turns with the axes, tested); no UI rows (library materials are isotropic) |
| friction-free fit stage | Not tried: the fit stage is not the bottleneck any more (the pin stages are) |

**Late findings** (found by running the toolbox's default case, which the crate's own tests had not covered): the loaded stage stagnated with a fixed active set on the Bushing Workbench defaults (bronze bushing, aluminium boss, 0.0425 in thin wall, 1000 lbf): the pin's anchors were too soft to make its rotation/translation determinate under sliding friction (`1e-6 E t`; `3e-5` fixes it, `ground_leak` 0.1 %); the graded default mesh solved three times slower than a uniform one (41 s vs 13 s) with the same capacity; and the closest-point projection's `FaceGeom::second` now uses the exact constant second derivatives for edges (it differentiated numerically on every Newton iteration).

**Measured speed** (8 cores, default mesh, 0.5 in bore): one fit solve 2-4 s; `analyze` with the pin 8-20 s; `max_offset` with the pin credited is two rounds of eight parallel analyses (~1-2 minutes, on a worker); on the fit-alone basis ~20 s; the direct spin check adds 15-30 s.

**Not done, why**: an unstructured 3D mesher, a prism element, shells / beams / dynamics (each is a project of its own, no product decision missing); moment-controlled friction runs near the slip limit with a free rotating master (the Schur complement assumes symmetry).

## 10. Third session (2026-10-07): slow and failing searches, the reported case

Reported: `m` sometimes ran over 600 s; `l` failed with "the pin could not be pressed in to 2214 lbf (Completed)" (bore 0.5, ID 0.1875, length 0.125 in, interference 0.0025, friction 0.15, 1000 lbf, boss 1.25, offset 0.02; materials not given, aluminium housing / bushing of E 10.8 Msi reproduce its fit capacity 141.8 vs 141.7 lbf in).

**Evidence (all measured on the development Mac, release build, `ECCENTRIC_TRACE=1`):**

| Finding | Measurement |
|---|---|
| The reported error is stage A running out of prescribed travel | `max_load` probes 1.25 x the fit-alone limit = 8856 lbf; press-in `Completed` at travel 0.0099 in, `ForceReached` after extending to 0.0296 in |
| Stage A steps were wasteful | 200 steps: 107 s; 100: 26 s; 50: 19 s; 25: 19 s; margin 1.443 / 1.442 / 1.443 / 1.441 |
| Default solves were 10-30 x slower than the 5-15 s in section 9 | the old default mesh (0.031 in) plus the loaded stage's stick-slip failures: 150 s fail + 70-90 s retry on the Workbench defaults at 1500 lbf |
| The capacity does not need that mesh | margin 5.921 / 5.920 / 5.921 at 0.1 / 0.07 / 0.05 in elements, 5.921-5.924 at 0.031; thin wall (0.016 in) fit capacity 209.237 / 209.2185 / 209.2214 at 0.1 / 0.05 / 0.03 in. The displayed pressure range does need r/5 (bin scatter 10 % at r/4, <0.5 % at r/5) |
| Loaded stage stalls on friction stick-slip, path dependent | fails at release share 0.25 / 0.4 and anchor stiffness 1e-5 / 1e-4, converges at 0.12 and 3e-4 (all to margin 5.924); stalled residual 1e-3..1e-2 of the force scale |
| A random sweep finds more | 5 of 12 coarse-mesh cases failed before the changes; one case crawled 657 factorisations (103 s) with the strict tolerance and takes 7 s accepting a stall below 0.3 % of the force scale (margin 51.361 vs 51.358) |
| Step memory (damped Newton after a backtrack) is not a general cure | the reported case converges in 4 s without it and fails with it; the fine-mesh default case the reverse. It is now a second attempt |
| Stall acceptance needs a physical check | the soak sweep accepted a stalled residual that carried 84 of 207 lbf (the force scale is dominated by the fit): `transmitted` is checked before accepting |
| More parallel candidates do not help `max_load` much | reported case, 14-18 solves: k = 2 242 s, 4 363 s, 8 282 s: per-probe cost (failed attempts) varies far more than the thread count matters; kept k = cores clamped 2..8 |
| Two margins near the limit differ ~1 % between runs of different k | 12341 / 12407 / 12620 lbf (tolerance 2 % of the range): the loaded margin is slightly path dependent near the limit |

**Done:** see the crate AGENTS.md (`Control`, `halted` / `caveat`, `loaded_failure`, attempts, mesh, stage A extension), `fea-core/AGENTS.md` (`Interrupt`, `StepObserver`, `stick_slip_guard`, `step_memory`), the toolbox (progress, queue, cancel, sweep, history, `.vtu`, CSV), the Workbench (`.csv`, template benchmarks).

**Declined / not done, why:** (1) a kernel friction update that does not depend on the path (the real fix; the soft-parts and the thick concentric case still fail every attempt and now fall back to the fit alone, stated); (5) Brent / secant search: with 8 cores the 2-round parallel section search has the shorter wall time, so it was improved (interpolated second round, k + 3 solves) rather than replaced; (4) warm start across offsets: a different offset is a different mesh, nothing to map; the load-independent fit is cached across probes instead (`max_load`); (8) units selection: the whole toolbench is inch / lbf / psi, a unit system is a project-wide change, not a toolbox feature.
