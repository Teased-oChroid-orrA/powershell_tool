# Edge-distance cross-check

Status: on `main` (the contact FE below is on branch `fea-contact-model`). `bushing_solver::solve::compute` and its
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
| **Contact FE** (`C` only; `contact.rs`, `models/contact_model.rs`) | Bushing **and** housing meshed (Q9, plane strain, one lattice row per angular station so the interface stays inside the band). The interference is an initial overlap resolved by penalty contact (`kn = 100 E/a`) with Coulomb friction (C1-smoothed, slip limit `mu p` frozen within a solve and refreshed per load step). The fit is installed first (4 steps only if it yields), the rigid pin is conformed to the post-fit bore and pushed toward the edge under displacement control; the far-face reaction is a follower load of the computed pin load (Sherman-Morrison in the iteration matrix). Housing J2 perfect plasticity, bushing elastic. First-yield load extrapolated from one elastic pin step; collapse = load plateau (a further 1.6x of displacement adds < 2 %, twice). Modified Newton with a contact-set-aware refactorisation, damped steps while the set settles, Anderson acceleration for the plasticity. Solved on an edge-distance grid {1, 1.5, 2, 3, 4} D + the actual edge at {0, 1, 1.5}x the interference (at most 8 parallel solves); everything else is interpolated. | ~1.5 s per solve, ~3 s wall for `C` |
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

## Monte Carlo

Latin-hypercube samples (fixed seed, reproducible) of the fit pressure over its tolerance band
(uniform) and, optionally, a strength coefficient of variation (`EdgeConfig::strength_cv`, default 0).
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
~0.75-0.85 of the parallel-plane kinematic bound `2 k e t` at e/D 1-1.5. The UI flags any non-stress
model that needs >10 % more edge distance than the stress models.

### Should the stress-based number replace the legacy governing check?

Not yet, and never the mean-shear one alone. Recommendation: keep legacy governing; surface the
plastic-FE and tabulated results as advisory (warn, not fail, when either needs more e/D than is
provided); promote a stress-based number to governing only after it is correlated with edge-distance test
data or the MMPDS `Fbru(e/D)` curve (the plastic FE gives `Pc(1.5)/Pc(2.0) = 0.78` - compare), the
plane-strain assumption is bracketed by a 3D check, and countersunk/flanged geometry and load angle
are covered. Until then legacy is the most conservative of the computed numbers by only ~15 % and is
differentially tested.

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
| **Contact FE** | **1.49** | **30,400** (bearing limit 30,250) |

* **The fit barely changes the collapse load** (within the 1-2 % plateau scatter, either sign), as limit-load theory predicts for a residual stress installed before the load. The old dead-load treatment cost 8 %. The fit still matters for first yield and for retention.
* **Back-side contact is retained to ~2.7x the load the cosine models predict**: they open it at `P = p pi a t` (3.9 kips here), the real bushing keeps it to ~10 kips (back pressure 9.7 ksi at 0.6 kips, 2.3 ksi at 9 kips, 0 at ~11 kips; the bushing wall carries the load into the front half). The superposition's `pin_case` switch is therefore conservative; `edge_check` does not use it for the contact FE.
* Friction (mu 0.15) adds ~6 % collapse capacity vs frictionless; mu is a bushing input, not a model constant.
* The plain Contact-FE result at the default e/D is a **marginal pass** (+0.4 %): treat e/D ~1.5 as the boundary, not a safe value.

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
