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
- Not built: the window layout for this check (waiting on the owner's mockup approval); external pressure (UG-28) and the shell-to-head
  junction FE check are separate #234 items.
