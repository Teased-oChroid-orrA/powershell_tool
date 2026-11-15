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
- Reference (`cylinder_hemisphere_reference`): Geckeler / beam-on-elastic-foundation closed form for the equal-wall cylinder-hemisphere joint, derived
  in its doc comment from the standard semi-infinite-cylinder edge-load response: `M0 = 0`, `Q0 = beta^3 D delta`, `delta = p R^2/(2 E t)`,
  `M_max = 0.3224 Q0/beta` at `beta x = pi/4`, surface stress `6 M/t^2` (about `0.29 p R/(2t)/sqrt(1-nu^2)`).
  **Roark's own table coefficients were not available offline and are not quoted**; the reference is the textbook derivation the Roark shell-edge cases
  also rest on. **No closed form was sourced for the cylinder-to-ellipsoid junction**, so the ellipsoid is validated by membrane limits, convergence and
  load equilibrium, not against Roark: if a sourced expression is wanted, it needs the owner's copy of Roark Ch. 13 (or Timoshenko) to add as a second reference.
- Proof (`pressure-vessel-solver/tests/junction.rs`, release): far cylinder axial = Lame `p Ri^2/(Ro^2-Ri^2)` (0.1 %), hoop bending = exact Lame linear part,
  crown = far field (0.5 %), inside normal stress = `-p` (0.1 %); peak discontinuity bending vs reference (level-4 mesh) 2.4 / 1.5 / 0.8 / 0.1 % at `R_i/t` =
  50 / 100 / 200 / 400 (tolerances 3 / 2 / 1 / 1 %; the residual is the reference's own thin-shell error, it falls with `R/t`), junction displacement 0.7 / 0.3 / 0.2 /
  0.1 % (tolerances 1 / 0.5 / 0.25 / 0.15 %), peak position within 0.15 decay lengths, zero moment at the junction, symmetric peaks; mesh levels 1/2/4 agree
  to 1 % (bending) and 0.05 % (displacement) for hemisphere and 2:1 ellipsoid; ellipsoid crown vs `p a^2/(2 b t)` within 1.5 %; `h = R_i` ellipsoid is bitwise the hemisphere; linear in `p`.
- Observed (R/t 100, `p` = 100): 2:1 ellipsoid peak meridional surface stress 12.3e3 against 6.5e3 for the hemisphere (membrane 5.0e3): the ellipsoid's
  discontinuity is large (hoop compression in the head), which is why a code-rule thickness alone is not a junction check. Values are linear-elastic stresses to
  compare as secondary stress (3S limits), not against the primary allowable.
