# Edge-distance cross-check

Status: on `main` (the contact FE below is merged). `bushing_solver::solve::compute` and its
differential tests are unchanged (one additive output field, `t_eff_seq`). The feature is the
`edge-check/` crate plus one Results-pane hook in `app-tui/src/toolboxes/bushing/` (key `c`).

## Why

`solve.rs` checks edge distance with `Fbru_eff = Fbru + 0.8 p` (sequencing) and
`Load / (2 t tau sin(theta))` (strength). The `0.8` has no derivation in this repo (it is inherited
from the TS engine), and it credits the interference fit - while elastic stress superposition says
the fit loads the ligament in hoop tension and shear. This crate computes the requirement from the
stress field instead, with two independent cross-checks, so the legacy numbers can be judged.

## Models (all behind `edge_check::model::EdgeModel`; registry = `models::default_models`)

| Model | Method | Cost (release) |
|---|---|---|
| **Stress superposition** (primary) | Muskhelishvili complex potentials for the half plane `x > 0` minus the bore disc. Fundamental solutions (poles of every order at the bore centre and its mirror image, Kelvin logs tied by `psi ~ -kappa conj(alpha) ln`) fitted by least squares to "bore carries the contact tractions, edge `x = 0` traction free". Smooth closed-form field, no mesh. | 3-5 ms per geometry |
| **Plane-stress FE** | Q9 elements, O-grid conforming to bore / free edge / far face / top face, half model about the load line, banded Cholesky, three unit load cases from one factorisation. | ~50 ms (fine mesh), ~15 ms (search mesh) |
| **Elastic-plastic FE limit load** (fallback when no bushing is given; superseded by the contact FE) | Same Q9 mesh, J2 perfect plasticity, **plane strain**, flow stress `min(Ftu, sqrt(3) Fsu)`, fit pressure as a dead load, pin load on the loaded half-arc raised under *displacement control* (load control has no equilibrium past collapse, so "stopped converging" is ambiguous). Initial-stress iteration on the elastic factorisation with Anderson acceleration; closed-form return map; jump straight to 90 % of first yield. The collapse load is solved on an edge-distance grid in parallel threads (+ the actual edge, + the no-fit case) and every other edge distance is read off that profile. | 1-2 s (6-9 parallel solves of ~0.5 s) |
| **Contact FE** (`C` only; `contact.rs`, `models/contact_model.rs`) | Bushing **and** housing meshed (Q9, plane strain, one lattice row per angular station so the interface stays inside the band). The interference is an initial overlap resolved by penalty contact (`kn = 100 E/a`) with Coulomb friction (C1-smoothed, slip limit `mu p` frozen within a solve and refreshed per load step). The fit is installed first (4 steps only if it yields), the rigid pin is conformed to the post-fit bore and pushed toward the edge under displacement control; the far-face reaction is a follower load of the computed pin load (Sherman-Morrison in the iteration matrix). Housing J2 perfect plasticity with flow stress `(Ftu + Fty)/2` (capped at `sqrt(3) Fsu`; see the test-data validation below), bushing elastic. Modified Newton with a contact-set-aware refactorisation, damped steps while the set settles, Anderson acceleration for the plasticity. Solved on an edge-distance grid {1, 1.5, 2, 2.5, 3, 4} D + the interference levels {0, 1.5}x at a fixed reference edge of 2 D (8 parallel solves, none depending on the edge being checked, so applying a recommendation cannot move it); everything else is read from a monotone cubic (Fritsch-Carlson) through those points and the origin (linear interpolation of this concave curve was low by up to 4.5 %). First-yield load extrapolated from one elastic pin step; collapse = load at a pin displacement of 10 % of D. | ~1.5 s per solve, ~3 s wall for `C` |
| **Tabulated allowables** | `P/(D t) <= Fbru(e/D)` (Fbru taken as the e/D = 2.0 value; interpolated to 1.5 only if a 1.5 value is supplied) and the classical 40-degree shear-out rule `P + p D t <= 2 Fsu t (e - (D/2) cos theta)` (the fit's `p D t` is charged at 100 %, same strip equilibrium as the stress models; bearing is not reduced by the fit). No stress field. | microseconds |

## Load model

* Pin load `P` acts toward the nearest free edge (worst case), transmitted through the bushing as a
  cosine radial pressure on the housing bore.
* If the fit retains contact (`p_fit >= P / (pi a t)`) the cosine acts on the **whole** bore (the back
  side is relieved, not separated) and the loaded half-arc carries `P/2`; otherwise it acts on the
  loaded half only and carries `P`. This assumes a rigid, frictionless-transfer bushing.
* The fit pressure comes from `solve::compute` (`output.pressure`, band `pressure_range`).
* Superposition is linear: three unit fields (fit, half-pin, full-pin) are solved once per geometry,
  then any `(p_fit, P)` is a linear combination - this is what makes Monte Carlo and the edge-distance
  search cheap.

## Failure modes

* **Shear-out**: mean shear on the two planes tangent to the bore, parallel to the load, `x in [0, e]`,
  against `Fsu`. By x-equilibrium of the strip between them this equals
  `(force on the loaded half-arc) / (2 t e)` exactly - the field integral is a built-in check
  (`shear_out_equals_the_arc_force_over_twice_t_e...`). The fit pressure on that half-arc adds
  `p D t` to the force. **Assumes full shear redistribution (ductile)**; mean shear is the limit load,
  not a first-yield bound.
* **Splitting**: mean hoop tension across the ligament (`y = 0`, bore to edge) against `Ftu`.
* **First yield**: peak elastic von Mises on the bore and free edge against `Sy` (applied load only).
* **Bearing** (allowables only): `P/(D t)` against `Fbru(e/D)`.

Three targets per model (`runner::TARGETS`): **Strength** (applied load; shear-out, splitting, bearing,
collapse), **Bearing** (the edge must not fail at the **bearing-limit load** `Fbru D t`, i.e. bearing
governs first - the legacy solver calls this "sequencing"; independent of the applied load) and
**First yield** (applied load, informational). Each target also reports `capacity_lbf`: the pin load at
which its lowest margin reaches zero at the actual edge distance and nominal fit (found by bracketing +
bisection on the load, so it is model-agnostic), and `capacity_no_fit_lbf` (fit removed). The UI shows
both as a second table; "fit uses" = `1 - capacity/capacity_no_fit` for Strength. `P(fail)` in the table
is the Bearing target's. The report also
shows each margin with the fit removed, to isolate what the interference does.

## Recommended edge distance (P90 / P95 / P99)

Because the nominal margin ignores variability, every run also finds, per model, the e/D at which the failure probability falls to 10 / 5 / 1 %, and recommends the largest of them over the models at P99 (`EdgeReport::recommended`, `TargetResult::e_levels`, `runner::search_levels`). Variability sampled (Latin hypercube, 800 samples, fixed seed): the fit pressure uniform over its tolerance band; the edge strength (`EdgeConfig::strength_cv`, **5 %, an assumption for typical handbook values, not user data**); the model's own error (`EdgeModel::model_cv`: contact FE 7 % = sd of its 12 NACA TN 1503 validation errors, superposition 15 % = engineering judgement since it is unvalidated, allowables 0 % since they are already statistical); and, for the bearing-limit load, the bearing strength as an independent factor (independent is the more conservative of independent and correlated). All normal, clipped at 3 sigma. The strength and bearing targets are searched (first yield is informational). With all scatter off the three levels collapse to the nominal minimum (tested).

The levels are reported separately for the **applied load** and for the **bearing-limit load**, because they behave differently with the housing length `t`: the applied-load requirement falls as `t` grows (capacity `~ t`, load fixed), while the bearing-limit one does not (the load it must carry, `Fbru D t`, also `~ t`, so `t` cancels; tested: `a_longer_housing_lowers_the_applied_load_recommendation_but_not_the_bearing_limit_one`). The recommendation is the larger of the two over the models at P99, and names which one governs. The tabulated allowables' `e/D >= 2.0` floor (no `Fbru` allowable below it without an `e/D = 1.5` value) is load- and length-independent and shows in the applied-load column.

Default bushing (e/D 1.5, 1000 lbf): nominal minimum 1.37 / 1.74 / 1.68 (superposition / allowables / contact FE); bearing-limit P90 1.73 / 1.86 / 2.02, P95 1.84 / 1.90 / 2.13, P99 2.11 / 1.97 / 2.38; applied-load P99 under 0.75 / 2.00 / under 0.75 (the allowables floor); recommended e/D 2.38 (1.19 in), bearing limit governing, set by the contact FE. Applying 1.19 in and re-running leaves the recommendation unchanged (tested: `the_recommended_edge_distance_is_a_fixed_point_of_applying_it`).

## Monte Carlo

Latin-hypercube samples (fixed seed, reproducible) of the fit pressure over its tolerance band
(uniform), the strength (`EdgeConfig::strength_cv`, default 5 %) and each model's own error (`model_cv`, switch `EdgeConfig::model_error`).
Reported: `P(margin < 0)` per target. Cheap because it only recombines stored unit fields.

## Validation (all in `edge-check/tests/verification.rs` and unit tests)

* Free-edge stress of a pressurised bore equals `4 p a^2 / (e^2 - a^2)` to 1e-6 for e/a = 1.5..8
  (closed form; found numerically, matches the known half-plane cavity result), and the edge is traction free.
* Bore tractions reproduced to 1e-8.
* Analytic vs FE on a half-plane-sized plate: shear-out <= 1 %, first yield <= 2 %, splitting <= 5 %
  (test tolerances; typically far tighter) - two independent methods.
* FE discretisation error vs a 5x finer mesh: < 0.4 % (default), < 0.9 % (search mesh), production plate.
* Finite plate (3 e or 10 a) vs half plane: <= 5 % on shear-out / first yield.
* Minimum edge distance is a true root; margins non-decreasing in e; MC `P(fail)` equals the exact
  fraction of the fit band on the failing side; reproducible per seed.

## Why plane strain for the plastic model

In plane stress the bore crushes at ~0.92 `sigma0 D t` regardless of edge distance (below the tabulated
`Fbru` of ~1.6 `Ftu`), so bearing crush masks the edge failure; plane strain (a thick bore, `eps_zz = 0`)
gives ~2.8 `sigma0 D t` at large edge distance, above `Fbru`, so the edge is what limits at the
bearing-limit load. Real bearing is triaxial but not fully plane strain, so plane strain is the
better of two imperfect 2D choices; a 3D model would remove the choice. A strengthened-bearing-zone
trick was tried and rejected: the result depended on the zone size and just moved the crush to the zone edge.

## Default bushing (bore 0.5, e = 0.75 = 1.5 D, t = 0.5, 1000 lbf, Al 7075, p = 8.8 ksi)

| Check | Minimum e/D |
|---|---|
| Legacy sequencing (`Fbru + 0.8 p`, `sin 40`) | 2.08 |
| Elastic-plastic FE limit load (plane strain) | 1.81 |
| Tabulated allowables (Bruhn 40-degree planes) | 1.64 |
| Stress superposition (mean shear) | 1.37 |
| Plane-stress FE (mean shear) | 1.35 |

The two elastic stress models agree with each other; both assume the shear on the planes reaches
`Fsu` everywhere, which is an upper bound on capacity. The plastic FE computes the failure instead
and lands 30 % higher in e/D, between the tabulated rule and the legacy number: collapse reaches only
~0.75-0.85 of the parallel-plane kinematic bound `2 k e t` at e/D 1-1.5. The UI advisory (below) warns whenever a
non-stress model's margin is negative at the actual edge distance; there is no 10 % threshold.

### Should the stress-based number replace the legacy governing check?

Not yet, and never the mean-shear one alone. Recommendation: keep legacy governing; surface the
plastic-FE and tabulated results as advisory (warn, not fail, when either needs more e/D than is
provided); promote a stress-based number to governing only after it is correlated with edge-distance test
data or the MMPDS `Fbru(e/D)` curve (the plastic FE gives `Pc(1.5)/Pc(2.0) = 0.78` - compare), the
plane-strain assumption is bracketed by a 3D check, and countersunk/flanged geometry and load angle
are covered. Until then legacy is the most conservative of the computed numbers by only ~15 % and is
differentially tested.

## Roles and the model-by-target matrix (owner decision, 2026-11-22)

* **Governing**: the legacy solver check (`Fbru + 0.8 p`, `bushing-solver/src/solve.rs`). It decides PASS / REVIEW. Nothing in
  `edge-check` or the matrix changes it, and no stress-based number is promoted to governing until it is correlated with
  edge-distance test data **and** the plane-strain assumption is bracketed by a 3D check (that evidence work is separate).
* **Best estimate**: the contact FE. It is shown as the owner's best estimate of the edge distance, not as a verdict.
* **Comparison**: the stress superposition, the tabulated allowables, the plane-stress FE and the plastic FE.

`edge_check::matrix::EdgeMatrix::build(&report, legacy)` turns a finished run into rows (models, best estimate first, then
the governing legacy row) by columns (the applied-load and bearing-limit targets, each at Nominal, P90, P95, P99), every
cell the minimum e/D with a flag for "the housing's actual e/D is short of it". The legacy row has Nominal cells only
(it is a closed form: no probability levels). First yield (informational) and P(fail) at the actual edge distance are kept
beside the table. The matrix only reads the report; a test (`bushing/tests.rs`) checks that the verdict, the legacy edge
numbers and the margins are identical before and after a run. The native Bushing window draws it (`editor/bushing/edge_matrix.rs`);
the window text quotes the validation numbers of the section below (a unit test keeps them equal to this doc).

## The shared estimate (`edge_check::estimate`) and a bare pin in a hole

One input, one output for every workbench with a hole near a free edge: `EstimateInput` (bore radius, edge distance, thickness, material strengths, the fit, the load case, the fit-pressure band, and the workbench's own legacy check if it has one) in, `EdgeEstimate { report, matrix }` out (`estimate()`; `prepare` / `execute` are the split the windows use to run on a worker thread). The Bushing window builds its input through it (`toolboxes/bushing/edge_check.rs::estimate_input`).

* **`BoreFit::Pin`**: a pin or fastener in a bare hole, no bushing and no interference. The contact FE meshes the pin as the NACA TN 1503 validation does (`estimate::pin_spec`: a steel cylinder E 29e6, nu 0.30, loaded through a core of 0.4 a, no interference, friction 0 by default because the tests agree best with a frictionless pin). A test (`tests/estimate.rs`) checks the pin estimate at e/D 2.0 equals that validated solve to 1 %. A bare hole has no fit pressure: the stress models run with `p = 0`.
* **`BoreFit::Bushing`** (the contact FE meshes the bushing; the fit is validated analytically only) and **`PressureOnly`** (the bushing is not described: no contact FE, the dead-load plastic plate model stands in on a deep run).
* **Governing or advisory.** A workbench with its own legacy check passes it in and that row is governing. With none (`legacy: None`) every row is advisory (`EdgeMatrix::is_advisory()`), and a window must say so.
* **Load case.** `LoadCase::PinTowardEdge`: a pin bearing on the bore, pulling toward the nearest free edge (worst case). A plate with a hole under remote tension is not supported yet (see the Fastener Hole work).

## Fastener Hole and plate tension (plan and open question, 2026-11-22)

**Case (a), a pin or fastener bearing toward the edge**, is what the shared estimate does today (`BoreFit::Pin`, `LoadCase::PinTowardEdge`). The Fastener Hole workbench has no load or material inputs and no plate thickness for a regular hole, so wiring it needs: an edge-strength section (housing material, plate thickness, pin load, pin-hole friction), the estimate run on a worker with the same stale / cancel behaviour as the Bushing window, and the shared matrix tables (`gpu_viewer/editor/bushing/edge_matrix.rs` to move to a shared place). Fastener Hole has no legacy edge check, so its matrix is **advisory** (`EdgeMatrix::is_advisory`): no row is governing, and the window says so.

**Case (b), a plate with a hole near an edge under an axial plate load** (owner decisions, 2026-11-22):
* **Geometry and load.** A finite plate or strip: inputs are plate width, edge distance, hole size, thickness, material and the **load angle (any angle)**.
* **Two failure criteria, each its own labelled row of the matrix:** net-section strength across the ligament against `Ftu`, and first yield at the hole edge. A model that cannot compute one criterion shows that cell as *unavailable*, never a number.
* **Which models can carry it.** The half-plane stress superposition (`analytic.rs`: one free edge, no far field) and the tabulated allowables (pin bearing and shear-out only) **cannot represent a finite strip at an angle**; they are shown as unavailable. The candidates are the plane-stress FE (`kfem.rs`, the same O-grid on `fea-core` with far and top faces, so a remote traction is possible) and the contact FE (an elastic-plastic plate with a hole when there is no pin: a different use of it, unvalidated). What each is checked against is stated when it is built; the planned limiting check is the open-hole finite-width stress-concentration factor `Kt(d/W)` as the width grows and the edge distance goes to infinity (`Kt` to 3 for a hole in a wide plate, tension across it).
* **Labels.** Fastener Hole has no legacy edge check: its matrix is advisory, no row governing.

## UI: advisory, tooltips, material data

* **Advisory, not governing.** After a run, any non-stress model (tabulated allowables, plastic FE) whose
  margin is negative at the actual edge distance adds a warning line to the Results check list
  ("Edge advisory - ..."). It never changes PASS/REVIEW or the legacy governing check, and it disappears
  while the run is stale.
* **Tooltips.** Hover a model name (or the "Edge Distance - legacy check" heading, or an advisory) to see
  what the check does, its strengths, weaknesses and restrictions; click pins it, Esc/click closes it
  (terminals without mouse-motion reporting). Text lives in `bushing/edge_check.rs::tip`.
* **`Fbru` at e/D = 1.5.** `Material::fbru_e15_ksi` (0 = not tabulated), an extra field in the Bushing
  add-material form, persisted with `#[serde(default)]` (old library files still load). The tabulated
  allowables interpolate between it and `Fbru` (taken as the e/D = 2.0 value) per the MMPDS rules
  (section 1.4.7.1: interpolate 1.5-2.0, use the 2.0 value above, tests below 1.5, t/D 0.25-0.50).
  **No built-in material carries a value**: MMPDS is proprietary, the e/D = 1.5 entries could not be
  sourced, and inventing them would be worse than a blank. Enter them from your licensed MMPDS. Note the
  built-in `Fbru` values are the TS engine's "typical" figures, not MMPDS A-basis (e.g. 7075-T6 sheet
  Fbru(e/D 2.0) is 146 ksi A-basis in MMPDS-05+ vs 121 here).

## Contact FE: validation and findings (2026-10)

Validation (`edge-check/src/contact.rs` tests): interference pressure vs plane-strain Lame 0.2 % (10.31 vs 10.33 ksi); iteration matrix = finite-difference derivative of the residual (1e-3); frictionless bore resultant = pin load (2 %); mesh convergence of the collapse load (production vs 1.4x finer mesh < 3 %, measured ~1 %); an 81-case sweep over e/D, interference, friction and bushing wall converges (`--ignored`).

Default bushing (7075 housing, e/D 1.5, p = 8.8 ksi):

| Check | e/D needed (bearing-limit load) | Capacity (lbf) |
|---|---|---|
| Legacy (`Fbru + 0.8 p`) | 2.07 | |
| Plastic FE, cosine half load, fit dead load (old) | 1.81 | 25,400 |
| Stress superposition | 1.37 | 33,800 |
| **Contact FE** (flow stress (Ftu+Fty)/2, collapse at 0.1 D displacement) | **1.68** | **27,600** (bearing limit 30,250: short by 9 %) |

* **The fit barely changes the collapse load** (within the ~1-2 % solver scatter, either sign), as limit-load theory predicts for a residual stress installed before the load. The old dead-load treatment cost 8 %. The fit still matters for first yield and for retention.
* **Back-side contact is retained to ~2.7x the load the cosine models predict**: they open it at `P = p pi a t` (3.9 kips here), the real bushing keeps it to ~10 kips (back pressure 9.7 ksi at 0.6 kips, 2.3 ksi at 9 kips, 0 at ~11 kips; the bushing wall carries the load into the front half). The superposition's `pin_case` switch is therefore conservative; `edge_check` does not use it for the contact FE.
* Friction (mu 0.15) adds ~6 % collapse capacity vs frictionless; mu is a bushing input, not a model constant.
* With the validated flow stress the Contact FE says the default e/D 1.5 is **9 % short** of the bearing-limit load (needs e/D 1.68). Earlier versions read +0.4 % / -4 % here (flow stress `Ftu`, then the plateau rule); the displacement rule and the standard flow stress moved it, and the test data say which side is safe. Allowables (1.74) and the legacy check (2.07) are stricter, superposition (1.37) looser.

## Test-data validation (NACA TN 1503)

Search result: no public source tabulates raw bushed-hole or interference-fit edge-distance tests. The Air Force lug method (Melcon & Hoblit, 1953) and MMPDS publish only curves and statistical allowables. The one openly available set of raw pin-bearing tests with edge distance is **NACA TN 1503** (R. L. Moore, Alcoa, 1948; NTRS 19930082186; page images, transcribed by hand and checked against its own Table III ratios): 0.500 in steel pin, t = 0.250 in (t/D 0.5), 2 in wide specimens, e/D 1.5 and 2.0, rolled bar of 75S-T (= 7075-T6), 24S-T (= 2024-T4) and 14S-T (= 2014-T6), two sizes each. TN 920/974/981 (sheet and plate) were not retrievable in readable form.

`edge-check/tests/validation_naca_tn1503.rs` runs the contact FE on the actual specimens (width 2 in, steel pin as a thick steel cylinder with a rigid core, no interference) and compares the predicted ultimate bearing stress `P/(D t)` with the measured one:

| Alloy (bar) | e/D | Test (psi) | Contact FE, flow stress (Ftu+Fty)/2 | with flow stress Ftu |
|---|---|---|---|---|
| 75S-T 1x2 | 1.5 / 2.0 | 115,500 / 151,900 | -0.6 % / -3.4 % | +3.9 % / +1.0 % |
| 75S-T 2x2 | 1.5 / 2.0 | 109,500 / 140,400 | -2.4 % / -2.9 % | +7.7 % / +7.4 % |
| 24S-T 1x2 | 1.5 / 2.0 | 98,500 / 123,000 | -17.4 % / -16.1 % | -4.5 % / -2.7 % |
| 24S-T 2x2 | 1.5 / 2.0 | 98,400 / 123,400 | -20.2 % / -19.4 % | -7.6 % / -6.3 % |
| 14S-T 1x2 | 1.5 / 2.0 | 102,800 / 129,500 | -10.5 % / -9.6 % | -6.6 % / -5.6 % |
| 14S-T 2x2 | 1.5 / 2.0 | 99,400 / 124,200 | -9.2 % / -7.6 % | -4.2 % / -2.4 % |

* The collapse load is read at a pin displacement of 10 % of the bore diameter (`COLLAPSE_D_OVER_D`): in a perfectly plastic pin-in-hole model the load never quite plateaus (the contact arc keeps growing), and the earlier "gain under 2 % per step" plateau rule scattered the result by +-3 % from one edge distance to the next (the recommended e/D wandered when applied). The fixed displacement is deterministic. The flow stress is the standard limit-analysis `(Ftu + Fty)/2`, not fitted. Result: all 12 points predicted 0.6-20 % **low** (mean -9.9 %, sd 6.9 %; none high), and `Fbru(2.0)/Fbru(1.5)` within 10 %. With `Ftu` as flow stress the errors are -7.6 % to +7.7 % (mean -2.7 %, best centred, but 7075 slightly high), hence the conservative choice for this tool's default material.
* The plain-pin friction (0.2 vs 0) raises the prediction 5-8 %; the tests agree best with a frictionless pin. The bushing/housing friction of the real model is a separate input.
* The plastic ductile alloy 24S-T is under-predicted (conservative) because perfect plasticity ignores its hardening.
* **Not validated**: the interference fit and the bushing (no public data found; the fit is validated analytically against Lame only), `t/D` other than 0.5, and e/D outside 1.5-2.0 (the tests are all in that range).

## Known limits

* Half plane: only the nearest free edge is modelled (the back edge of a narrow housing is ignored).
  The FE plate is 3 e / 10 a, so it includes some finite-plate effect the analytic model cannot.
* The elastic models assume full shear redistribution; the plastic and contact models compute it but are 2D
  (plane strain), perfectly plastic (no hardening, no fracture), small strain, and its collapse load
  carries ~1-2 % solver tolerance and ~4 % mesh error (coarse mesh; the profile is read off a 7-point
  grid by linear interpolation). Fit pressure dependence is linear between the no-fit and nominal-fit solves.
* Peak elastic stress at the bearing-limit load exceeds yield by construction, so no elastic-peak
  bound is offered for that load case (tried; it fails at every edge distance).
* Bearing allowable below e/D 2.0 needs a user-supplied `Fbru(e/D = 1.5)` (not in the material table).

## Removing it

Delete `edge-check/` (the plastic models are `plastic.rs` + `models/plastic_model.rs`; the contact model `contact.rs` + `models/contact_model.rs`), `app-tui/src/toolboxes/bushing/edge_check.rs`, the `edge_check` field /
`EdgeCheck` action / `c` key in `bushing/mod.rs`, the `edge_section` hook in `bushing/view.rs`, the
workspace member and `[profile.dev.package.edge-check]` in the root `Cargo.toml`, and the dependency
in `app-tui/Cargo.toml`. To drop one model, delete its line in `models::default_models`.


## How the fit pressure is charged (audit, 2026-10)

Question: does the interference-fit stress consume capacity at 100 %? Per model, verified in code:

| Model | Fit treatment | Verdict |
|---|---|---|
| Superposition / elastic FE | Fit and pin fields add linearly (`field.rs::FieldResponse::combine`); shear-out strip force includes `p D t`, splitting hoop includes the fit hoop, first-yield von Mises is of the sum. | 100 % charged. |
| Plastic FE | Fit applied as a dead pressure, then collapse; `fit_factor` interpolates linearly between the solved no-fit and nominal-fit cases. | Charged in full as a dead load. Limit-load theory says a purely self-equilibrated residual stress would not lower collapse, but the fit is not self-equilibrated against the pin load here, so full charging is the conservative reading. |
| Tabulated allowables | **Was ignored** (shear-out `P <= cap`). Now shear-out is `P + p D t <= cap`, derived from the same strip equilibrium. Bearing is not reduced: radial pressure confines the bore. | Fixed (this change). |
| Legacy solver (`solve.rs`) | `Fbru_eff = Fbru + 0.8 p` **credits** the fit (opposite sign to the stress models); `0.8` underived. | Unchanged on purpose (needs a product decision); flagged in the Legacy tooltip. |

Contact-retention load sharing (`pin_case`): when `p >= P/(pi a t)` the cosine pin pressure acts on the whole
bore, so the loaded half-arc carries `P/2 + p D t` instead of `P + p D t`. This is a real consequence of
the same linear superposition (the back side is unloaded, not lost), not a credit for the fit, but it
assumes a rigid, frictionless bushing and uses the *nominal* pressure for the capacity numbers; the
Monte-Carlo pass covers the pressure band. Not modelled: the housing's finite width in the half-plane
model (the FE model includes it, to <= 5 %).


## 3D bracket, second test set and promotion criteria (2026-11-22)

Evidence only. Nothing here changes a governing check, the Bushing window or any product behaviour; the legacy
`Fbru + 0.8 p` check stays governing. The owner decides promotion. Code: `lug-solver/tests/edge_3d_bracket.rs` (one
driver for 2D plane strain, 2D plane stress and 3D) and `edge-check/tests/validation_naca_tn1502.rs`.

### Result in one paragraph

**The plane-strain contact FE is unconservative against a three-dimensional model of the same plate and the same
material law, at every thickness and edge distance tested.** Plane stress is below 3D and plane strain above it on all
12 NACA TN 1503 points (t/D 0.5): plane strain is **12 to 14 % above 3D at e/D 1.5 and 35 to 36 % above at e/D 2.0**;
plane stress is 11 to 13 % and 15 to 16 % below. So the two 2D idealisations do bracket 3D, but the bracket is wide
and the contact FE sits on its upper side. The contact FE still reads below the test values (TN 1503 and TN 1502) because
three errors have the opposite sign: perfect plasticity (no hardening), the flow-stress rule, and a rigid frictionless
pin. Its agreement with the tests is therefore **partly compensation**, not proof that plane strain is the right
physics: a 3D model with the same material law is 15 to 45 % below the tests.

### How the bracket was made (and proved)

One driver (`run`) serves all three idealisations so that the only difference is dimensionality: `FeaLug`'s graded Q9 mesh
(36 elements around the bore), the housing elastic-perfectly-plastic J2 at the model's flow stress
(`contact_model::flow_stress`), an analytic rigid frictionless pin driven toward the edge by a prescribed travel
(`Tuning::collapse`), the load read at a pin travel of 0.1 D (`COLLAPSE_D_OVER_D`, the contact FE's rule). 3D extrudes the
same mesh into Hex27 layers over half the thickness (quarter model: load-line and mid-plane symmetry, far end clamped in
x and y) with a rigid cylinder pin. A bushing is installed first (two-pass frictional interference contact), then the pin
is made to fit the post-fit bore, as `edge-check` does. Proofs (default test suite):

| Check | Result |
|---|---|
| 3D with every `uz` locked equals 2D plane strain (same mesh, steps, pin) | 13728.6 vs 13728.3 lbf (+0.003 %) |
| the 2D driver equals `FeaLug::limit_load_axial` | identical to 4 digits (13728.3 lbf) |
| a very thin 3D plate (t/D 0.05) approaches plane stress | +2.26 % |
| kernel fit pressure of the default bushing equals `edge-check`'s contact FE | 9927 vs 9897 psi (+0.30 %; the two contact passes each carry about half the pressure, so the physical pressure is their sum) |
| 2D mesh, 36 to 72 elements around | plane strain -0.8 %, plane stress -1.9 % |
| 3D mesh, layers 2 / 3 / 4 over half the thickness | 12234 / 12227 / 12224 lbf (-0.08 %); 54 elements around, 2 layers: 12203 (-0.25 %) |

### NACA TN 1503, all 12 points (t/D 0.5, 2 in wide, 3D 2 layers; bearing stress P/(D t), psi)

| Bar | e/D | Test | Plane strain | Plane stress | 3D | strain / 3D | stress / 3D |
|---|---|---|---|---|---|---|---|
| 75S-T 1x2 | 1.5 | 115500 | 109826 (-4.9 %) | 87259 (-24.5 %) | 97873 (-15.3 %) | +12.2 % | -10.8 % |
| 75S-T 1x2 | 2.0 | 151900 | 138450 (-8.9 %) | 86729 (-42.9 %) | 102338 (-32.6 %) | +35.3 % | -15.3 % |
| 75S-T 2x2 | 1.5 | 109500 | 102034 (-6.8 %) | 80437 (-26.5 %) | 90683 (-17.2 %) | +12.5 % | -11.3 % |
| 75S-T 2x2 | 2.0 | 140400 | 128352 (-8.6 %) | 79983 (-43.0 %) | 94712 (-32.5 %) | +35.5 % | -15.6 % |
| 24S-T 1x2 | 1.5 | 98500 | 77327 (-21.5 %) | 59307 (-39.8 %) | 67997 (-31.0 %) | +13.7 % | -12.8 % |
| 24S-T 1x2 | 2.0 | 123000 | 96380 (-21.6 %) | 59078 (-52.0 %) | 70653 (-42.6 %) | +36.4 % | -16.4 % |
| 24S-T 2x2 | 1.5 | 98400 | 74592 (-24.2 %) | 57020 (-42.1 %) | 65502 (-33.4 %) | +13.9 % | -12.9 % |
| 24S-T 2x2 | 2.0 | 123400 | 92857 (-24.8 %) | 56812 (-54.0 %) | 68011 (-44.9 %) | +36.5 % | -16.5 % |
| 14S-T 1x2 | 1.5 | 102800 | 87657 (-14.7 %) | 68041 (-33.8 %) | 77456 (-24.7 %) | +13.2 % | -12.2 % |
| 14S-T 1x2 | 2.0 | 129500 | 109723 (-15.3 %) | 67725 (-47.7 %) | 80679 (-37.7 %) | +36.0 % | -16.1 % |
| 14S-T 2x2 | 1.5 | 99400 | 85950 (-13.5 %) | 66587 (-33.0 %) | 75889 (-23.7 %) | +13.3 % | -12.3 % |
| 14S-T 2x2 | 2.0 | 124200 | 107512 (-13.4 %) | 66286 (-46.6 %) | 79017 (-36.4 %) | +36.1 % | -16.1 % |
| **Mean error vs test** | | | **-14.9 %** | **-40.5 %** | **-31.0 %** | | |

The kernel's rigid-pin plane strain reads -14.9 % (the same as `lug-solver/tests/kernel_naca.rs`, -15.1 %); `edge-check`'s own
contact FE, whose pin is a steel cylinder with a rigid core, reads -9.9 %. In 3D the e/D 1.5 to 2.0 strengthening is only 4.6 %
(75S-T 1x2) against +31 % in the tests and +26 % in plane strain: at the larger edge distance the bearing crush at the
thin free surfaces, not the edge, limits the 3D plate, and 2D plane strain cannot see that.

### Thickness and edge-distance sweep (75S-T 1x2; model against model, no test data at these points)

Plane strain does not depend on thickness (P/(D t)); 3D does. 3D with 2 layers (t/D <= 0.5) or 3 (above).

| e/D | t/D | plane strain | plane stress | 3D | strain / 3D | stress / 3D |
|---|---|---|---|---|---|---|
| 1.5 | 0.1 | 109826 | 87259 | 90862 | +20.9 % | -4.0 % |
| 1.5 | 0.25 | 109826 | 87259 | 94113 | +16.7 % | -7.3 % |
| 1.5 | 0.5 | 109826 | 87259 | 97873 | +12.2 % | -10.8 % |
| 1.5 | 1 | 109826 | 87259 | 102167 | +7.5 % | -14.6 % |
| 1.5 | 2 | 109826 | 87259 | 106137 | +3.5 % | -17.8 % |
| 2.0 | 0.1 | 138451 | 86729 | 90543 | +52.9 % | -4.2 % |
| 2.0 | 0.25 | 138450 | 86729 | 95188 | +45.4 % | -8.9 % |
| 2.0 | 0.5 | 138450 | 86729 | 102338 | +35.3 % | -15.3 % |
| 2.0 | 1 | 138450 | 86729 | 114330 | +21.1 % | -24.1 % |
| 2.0 | 2 | 138450 | 86729 | 126885 | +9.1 % | -31.6 % |

| e/D (t/D 0.5) | plane strain | plane stress | 3D | strain / 3D | stress / 3D |
|---|---|---|---|---|---|
| 1.0 | 73627 | 70360 | 71453 | +3.0 % | -1.5 % |
| 1.25 | 92808 | 85130 | 87560 | +6.0 % | -2.8 % |
| 1.5 | 109826 | 87259 | 97873 | +12.2 % | -10.8 % |
| 2.0 | 138450 | 86729 | 102338 | +35.3 % | -15.3 % |
| 3.0 | 161548 | 86939 | 102559 | +57.5 % | -15.2 % |

Reading: plane stress < 3D < plane strain in all 15 cells. The plane-strain excess **narrows with thickness** (but is still
+3.5 % at t/D 2 for e/D 1.5 and +9.1 % for e/D 2.0, so plane strain is the thick-plate limit only well beyond t/D 2) and
**widens with edge distance** (+3 % at e/D 1.0, +57 % at e/D 3.0, where 3D has saturated at the bearing-crush level and plane strain keeps
rising). The contact FE's agreement with the tests is closest at small e/D and thick plates, where the excess is small.

### Does hardening explain the gap to the tests?

Ramberg-Osgood hardening through `Fty` and `Ftu` at the bar's elongation (engineering stress treated as true, flat at `Ftu`
beyond it: a strength-level idealisation, not a necking model), bearing stress psi, two bars of TN 1503:

| Bar | e/D | Test | plane strain EPP | 3D EPP | plane strain RO | 3D RO |
|---|---|---|---|---|---|---|
| 75S-T 1x2 | 1.5 | 115500 | 109826 | 97873 | 112464 (-2.6 %) | 100881 (-12.7 %) |
| 75S-T 1x2 | 2.0 | 151900 | 138450 | 102338 | 140502 (-7.5 %) | 106710 (-29.7 %) |
| 24S-T 1x2 | 1.5 | 98500 | 77327 | 67997 | 81162 (-17.6 %) | 73303 (-25.6 %) |
| 24S-T 1x2 | 2.0 | 123000 | 96380 | 70653 | 99379 (-19.2 %) | 79583 (-35.3 %) |

Hardening to `Ftu` adds only 3 to 11 % in either idealisation, so it does not close the gap between 3D and the tests: the 3D
model with a rigid, frictionless, uniformly loaded pin and small strain remains 13 to 35 % low. What is missing from 3D is the
subject of the next evaluation (deformable pin, friction, finite strain, true stress-strain beyond necking); until then the
sign of the 3D-versus-test error says only that a 3D model with this material law would be conservative, not that it is
accurate. Plane strain's near-agreement with the 75S-T tests (-2.6 %, -7.5 % with hardening) is the over-confinement
of plane strain compensating for the missing physics.

### Default Bushing case (bore 0.5, bronze bushing ID 0.375, 0.0015 diametral interference, 7075, friction 0.15, t/D 1.0, e/D 1.5)

| Quantity | Result |
|---|---|
| `edge-check` contact FE (its own plane-strain solver), collapse at 0.1 D | 27647 lbf (doc above: 27,600), 1.2 s |
| kernel 2D plane strain, same case | 26930 lbf (-2.6 % against `edge-check`: two independent 2D solvers agree), 2.7 s |
| mean fit pressure: `edge-check` / kernel 2D / kernel 3D (2 layers, 18870 dof) | 9897 / 9927 / 9200 psi: 3D end relief 7 % below plane strain at t/D 1.0 |
| kernel 3D pin stage | **not finished** (see "Known non-convergence and cost"); no 3D bushed collapse number |
| kernel 2D plane stress | **did not converge** (see below) |

### Second test set: NACA TN 1502 (24 points)

`edge-check/tests/validation_naca_tn1502.rs`: R. L. Moore, Alcoa, 1948, "Bearing Tests of 14S Sheet and Plate" (NTRS
19930082185, fetched with the owner's approval, deleted after transcription). 2 in wide specimens, e/D 1.5 and 2.0, 0.064 in
sheet on a 1/4 in pin (**t/D 0.256**) and 0.250 in plate on a 1/2 in pin (t/D 0.5); Alclad 14S-W and 14S-T sheet, 14S-W and 14S-T plate;
with-grain and across-grain tests each predicted from their own direction's tensile data (Table I). Bearing ultimate values are the
Table III averages; the transcription reproduces the report's own Table IV ratios to within 0.6 % (`table_iv_ratios_match`).
Result of the contact FE (frictionless steel pin, flow stress (Ftu+Fty)/2): **all 24 points under-predicted, mean -18.9 %, sd
6.5 %, range -28.6 % to -5.0 %**. That is looser than TN 1503 (mean -9.9 %): the soft W tempers (14S-T3/T4) are the worst
(-17 % to -29 %) because perfect plasticity ignores their strong hardening; the T6 tempers sit at -5 % to -23 %. There is no systematic
difference between t/D 0.256 and 0.5 for the same alloy. The e/D 1.5 to 2.0 strengthening is within 10 % of the tests on every row (asserted).
The report's nominal ratios (bearing / tensile ultimate 1.5 at e/D 1.5 and 1.9 at 2.0, i.e. Fbru(2.0)/Fbru(1.5) = 1.27) are the
published shape of the curve: the contact FE gives 1.22 to 1.28.

Pooled over both sets (36 points): mean -15.9 %, sd 7.8 %, none over-predicted. Note that the contact model's `model_cv` of 7 % (the sd of the 12
TN 1503 errors) is below the pooled 7.8 %, and its *bias* (-16 %) is not in the model error at all.

### Correlation survey

* **Public raw data with edge distance: NACA TN 1503 and TN 1502 only (both Alcoa, 1948).** The earlier NACA notes (TN 897, 901, 920,
  974, 981 for sheet, plate, extrusions; TN 2312 for castings; TN 2883 for forgings) tabulate the same e/D 1.5 and 2.0 tests; they were
  not retrievable as readable tables (page-image PDFs of 8 to 16 MB) and were not fetched. They would add alloys, not new e/D or t/D.
* MMPDS (proprietary) and the Air Force lug method (Melcon and Hoblit, 1953; reproduced in the Air Force Stress Analysis Manual) publish
  curves and statistical allowables, not raw tests; the bearing-efficiency curves start at e/D 1.5. The repo's `fbru_at_edge_ratio`
  encodes the MMPDS rule (interpolate 1.5 to 2.0, none below 1.5).
* No public bushed-hole or interference-fit edge-distance test data was found, and no test data below e/D 1.5 or above 2.0.
* **Not covered by any test value in the repo:** interference fit and bushing; t/D outside 0.256 to 0.5; e/D outside 1.5 to 2.0;
  materials other than aluminium; load angle; countersunk or flanged bushings; 3D effects measured (the tests give only the total load).

### Promotion criteria (proposed; the owner decides, nothing is promoted)

A stress-based or contact-FE number could become the governing edge-distance check only if each row below is met. Thresholds
are proposals for the owner to adopt or change. "Today" is the contact FE (the best of the three computed numbers); the stress-superposition
and plastic-FE numbers are no better on any row (the first is unvalidated against test data, the second is superseded).

| # | Criterion | Proposed threshold | Today (contact FE) | Met? |
|---|---|---|---|---|
| 1 | Bias against test data, conservative sign | no test point over-predicted; mean error between -25 % and 0 %, shown on at least 30 independent tests over at least 3 alloy families | 36 points (2 reports, 6 alloy / temper groups, all aluminium): none over-predicted, mean -15.9 % | **Met** for aluminium bearing at e/D 1.5 to 2.0 |
| 2 | Scatter | sd of the error at most the model error used for P90/P95/P99 (`model_cv`), and that cv includes the bias | pooled sd 7.8 % against `model_cv` 7 %; bias not included | **Not met** (small; fixable by re-deriving `model_cv` from 36 points) |
| 3 | t/D coverage of the range of use | tests at least at 0.25, 0.5 and the largest t/D the window accepts | 0.256 and 0.5 only | **Not met** (partly: MMPDS range 0.25 to 0.50 is covered; the Bushing window accepts any t) |
| 4 | e/D coverage of the range of use | tests below 1.5 and above 2.0, or the check refuses outside 1.5 to 2.0 | 1.5 and 2.0 only | **Not met** (and 3D says the model is least safe at large e/D: see 5) |
| 5 | 3D bracket of the plane-strain assumption | plane-strain number within +5 % of a 3D model (or below it) over the range of use, same material law | +3.5 % to +53 % above 3D for t/D 0.1 to 2 at e/D 2.0; +3.5 % to +21 % at e/D 1.5; +57 % at e/D 3.0 | **Not met, and unconservative** |
| 6 | Plane stress as the other side of the bracket | plane stress at or below 3D | 1.5 % to 32 % below 3D everywhere | Met (but 4 to 32 % too low to use) |
| 7 | 3D itself against tests | a 3D model that reproduces the tests (to about -10 % with sd 7 %) before it can be a reference | 3D with this material law and a rigid frictionless pin is 15 to 45 % below the tests | **Not met**: the reference to bracket against is not yet validated |
| 8 | Fit and bushing | tests with interference-fit bushings, or an accepted analytic-only argument | none; fit pressure equals Lame to 0.2 % (2D), equals the kernel's to 0.3 % (2D) and is 7 % lower in 3D at t/D 1.0 (end relief) | **Not met** |
| 9 | Material law beyond aluminium bearing | validated on each material class the library offers (steel, titanium, other) | aluminium only; hardening ignored (-17 to -29 % on soft tempers) | **Not met** |
| 10 | Numerical credibility | mesh convergence under 3 %; independent solver agrees; converges over the range of use | 2D mesh <1 %, 3D <0.3 %; kernel vs `edge-check` 2.6 %; contact FE swept over 81 cases; **plane-stress bushed run does not converge** | Met for plane strain; non-convergence noted |
| 11 | Geometry and load coverage | load angle, countersunk / flanged bushings, bushing length | none | **Not met** |

Recommendation (evidence, not a decision): keep the legacy check governing. Criterion 5 is the blocker and it points the
wrong way: the plane-strain contact FE, taken alone, over-predicts 3D by amounts that grow with e/D, so promoting it would credit
the edge distance where the third dimension takes strength away, relying on conservative material assumptions
(no hardening, flow stress at (Ftu+Fty)/2) that happen to be 15 % low on average to cover it. If a computed number is ever to
govern, the two defensible routes are a 3D-validated number (criterion 7 first), or the plane-strain number scaled by a measured 3D
knock-down that depends on t/D and e/D (the table above is a first map of it for one alloy).

### Known non-convergence and cost

* **Kernel 2D plane stress with a bushing does not converge.** The fit stage converges (4 factorisations, 9011 psi), then the plastic pin
  stage ran more than an hour with no result before being killed (CPU busy, no step completed). Hypothesis (not verified): plane-stress J2 crushes the
  bore at a limit load with no edge mechanism, and the plastic Newton iteration reads the missing equilibrium as slow convergence,
  the ambiguity `edge-check/AGENTS.md` records for load control. Reproduce: `BRACKET_TRACE=1 cargo test --release -p lug-solver
  --test edge_3d_bracket explore_bushed -- --ignored --nocapture` (the "2D stress" row). Unbushed plane stress converges (0.7 s). A cheap fix is not known
  (a displacement-controlled plateau stop as in `limit_load`, or a travel cap tied to the first flat step, would end the run but changes what is read); not attempted.
* **Kernel 3D bushed pin stage**: the fit stage takes 15 s (6290 nodes, 18870 dof, 12 factorisations); the plastic frictional pin
  stage had not finished after 45 minutes at 2 to 3 cores (RSS about 0.2 GB) when this section was written. Treat bushed 3D collapse as a manual study
  of hours, not a check (`fea-problem/AGENTS.md` records the same for the 3D bushing fit).
* Cost of the unbushed bracket (release, 8 cores shared with other work): 2D 0.5 s; 3D with uz locked 14 s; 3D 2 layers (17k dof) 44 s alone, 90 to 410 s
  under load for 17k to 31k dof; the 15-cell sweep took 28 minutes. Memory was not measured separately (the 31k-dof runs fit comfortably in the
  8 GB machine).

### Limits of this evidence

Rigid, frictionless, uniformly loaded pin (the tests' steel pin bends and has friction; `edge-check`'s plain-pin friction 0.2 raised its prediction 5 to 8 %);
small strain; hardening only as a strength-level Ramberg-Osgood cap; collapse read at a fixed 0.1 D pin travel; one alloy for the sweeps; plate
width and clamped far end as in the NACA specimens; 3D resolved with 2 to 4 Hex27 layers over half the thickness (converged to 0.3 % at t/D 0.5, not
re-checked at t/D 2 or e/D 3).
