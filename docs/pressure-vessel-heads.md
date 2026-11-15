# Pressure vessel: UG-27 / UG-32 code-rule check (#234)

`pressure-vessel-solver/src/heads.rs`: required thickness and maximum allowable working pressure of a cylindrical shell
(circumferential and longitudinal), a spherical shell / hemispherical head, an ellipsoidal head (general `K`) and a
torispherical head (general `M`), under internal pressure, with the allowable stress `S` and joint efficiency `E` as inputs.

- Rules: UG-27(c)(1)-(3), UG-32(d)-(e) with `K = (2 + (D/2h)^2)/6` and `M = (3 + sqrt(L/r))/4`. Each rule's limit of applicability
  (t <= R/2 and P <= 0.385 SE, P <= 1.25 SE, t <= 0.356 R and P <= 0.665 SE, L/r <= 16.67) returns `HeadError::OutsideRules` with the
  reason instead of a number.
- Dimensions are in the corroded condition (use `WallAllowance::remaining_geometry`).
- Not defaulted: `S` and `E`. No allowable data is stored here; the coefficients are the published rules and should be checked against
  the owner's code edition before a stamped-vessel result relies on them.
- Proof (`cargo test -p pressure-vessel-solver`): hand values, `P -> t -> P` round trip for every component, the 2:1 head (`K = 1`),
  hemisphere = sphere rule, torispherical at L/r = 16.6 within 0.2 % of the `0.885 PL/(SE - 0.1P)` form, and the code thickness against the
  exact Lame hoop stress of this crate (within 2 %).
- Not built: the window layout for this check (waiting on the owner's mockup approval); external pressure (UG-28) is a separate #234 item.

## Shell-to-head junction FE check (`junction.rs`, #234)

`solve_junction(JunctionInput)` meshes the wall of a cylinder + hemispherical / ellipsoidal head (same wall, internal pressure) as an
axisymmetric solid (`fea-core` Quad9, `2 * refinement` elements through the wall, meridional size graded from `t/(2 refinement)` at the junction),
solves it and linearises the stress through the wall at each meridian station into membrane + bending `[meridional, hoop]`
(`JunctionSolution::{stations, junction(), far_cylinder(), crown(), peak_bending_meridional(), peak_meridional_surface()}`). No fea-core change.
API only: no window (waits on the owner's mockup).

- Geometry: mid-surface cylinder `R_i + t/2` tangent to a quarter ellipse `(R_i + t/2, h + t/2)`; wall = constant-thickness offset (inside of an
  ellipsoid differs from the exact inside ellipse by `O(t^2)`). Closed-end axial traction at the far end (exact Lame state, no end disturbance);
  cylinder length 6 decay lengths.
- Reference (`cylinder_hemisphere_reference`): Roark's Formulas for Stress and Strain, 7th ed. (Young & Budynas; table numbers are that edition's): cylinder
  edge loads Table 13.2 cases 8 / 10, partial-sphere edge loads Table 13.3 case 1 (Hetenyi) at `phi = 90 deg`, where it reduces to the cylinder form (checked:
  both give `dR = 2 Q0 R beta/(E t)`), membrane growth Table 13.1. The joint is symmetric, so `M0 = 0`, `Q0 = beta^3 D delta`, `delta = p R^2/(2 E t)`,
  `M_max = 0.3224 Q0/beta` at `beta x = pi/4`, surface stress `6 M/t^2` (about `0.29 p R/(2t)/sqrt(1-nu^2)`). The same Roark 7th ed. worked example
  (Sec. 13.3, cylinder with rigid end caps) uses the same cylinder edge formulas. The solver uses mid-surface radius `R`.
- **Ellipsoid: no Roark expression.** Roark 7th ed. has no cylinder-to-ellipsoid junction table; for non-spherical shells it only advises an
  "equivalent spherical shell" approximation (Sec. 13.3, Ref. 6) with no rule for the equivalent radius. The ellipsoid is therefore validated by membrane limits,
  convergence and load balance, not against Roark. Linked sources not used: the NASA SP-5039 scan is a general design survey (no OCR checked for this formula),
  the Wiley excerpt is not identifiable, and the ScienceDirect link is a time-limited signed URL that had expired.
- Proof (`pressure-vessel-solver/tests/junction.rs`, release): far cylinder axial = Lame `p Ri^2/(Ro^2-Ri^2)` (0.1 %), hoop bending = exact Lame linear part,
  crown = far field (0.5 %), inside normal stress = `-p` (0.1 %); peak discontinuity bending vs reference (level-4 mesh) 2.4 / 1.5 / 0.8 / 0.1 % at `R_i/t` =
  50 / 100 / 200 / 400 (tolerances 3 / 2 / 1 / 1 %; the residual is the reference's own thin-shell error, it falls with `R/t`), junction displacement 0.7 / 0.3 / 0.2 /
  0.1 % (tolerances 1 / 0.5 / 0.25 / 0.15 %), peak position within 0.15 decay lengths, zero moment at the junction, symmetric peaks; mesh levels 1/2/4 agree
  to 1 % (bending) and 0.05 % (displacement) for hemisphere and 2:1 ellipsoid; ellipsoid crown vs `p a^2/(2 b t)` within 1.5 %; `h = R_i` ellipsoid is bitwise the hemisphere; linear in `p`.
- Observed (R/t 100, `p` = 100): 2:1 ellipsoid peak meridional surface stress 12.3e3 against 6.5e3 for the hemisphere (membrane 5.0e3): the ellipsoid's
  discontinuity is large (hoop compression in the head), which is why a code-rule thickness alone is not a junction check. Values are linear-elastic stresses to
  compare as secondary stress (3S limits), not against the primary allowable.

### Ellipsoid: the closed-form Geckeler estimate underestimates the head side (checked against two papers)

The equal-wall closed form for a cylinder joined to an ellipsoidal head (Balicevic, Kozak, Mrcela, "Strength of Pressure Vessels with Ellipsoidal
Heads", Strojniski vestnik 54(2008)10, 685-692) was checked formula by formula: `M0 = 0`, `Q0 = p (1+gamma)/(8 beta) = p a^2/(8 beta b^2)`
(eq. 14), `beta = (3(1-nu^2))^(1/4)/sqrt(R t)` (eq. 13), seam meridional stress `pR/2t` (Table 1), seam hoop `(pR/t)(1 - R^2/(4b^2))` (eqs. 2, 19, 20; zero for
2:1), moment decay `M = -(Q0/beta) e^(-x) sin x` (eq. 17; eq. 18 flips the sign on the cylinder side), peak at `x = pi/4` giving `6 (0.3224) Q0/(beta t^2)` (derived;
it matches Table 1 within the grid). These hold. Corrections: the hoop stress away from the seam is the membrane value plus
`(pR/4t)(1+gamma) e^(-x) cos x` (head `+`, cylinder `-`), not a constant plus the bending term; the paper's `x = int beta ds` uses the local `R2`, so a constant-`beta`
`e^(-beta x)` is exact only on the cylinder. Unequal thicknesses (`M0 != 0`, the 2x2 system of eqs. 11-12) are not solved in either paper (NASA SP-8083 only lists references).

**Measured here (not in the papers):** the formula is an asymptotic result and is not conservative at ordinary `R/t`. For a 2:1 head the FE head-side peak bending
is 1.35 / 1.25 / 1.23 / 1.14 / 1.07 times the formula at `R/t` = 50 / 87 / 100 / 200 / 400 (cylinder side about 7 % below it at `R/t` = 100), against the paper's claimed
`O(h/R)` (< 5 %) error: about 25 % at the paper's own `R/h` = 87. The head's meridian curvature (`R1 = b^2/a` at the equator) changes its decay, which the symmetric
cylinder-like model omits. Do not use the closed form as a design value for the head side; use `solve_junction`. Locked by
`the_ellipsoid_geckeler_formula_underestimates_the_head_side_peak` in `tests/junction.rs`.
