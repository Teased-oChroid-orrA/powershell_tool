# pressure-vessel-solver

> TL;DR: Pure calculation library (no GUI): thick-wall Lame stress, failure modes, minimum thickness, buckling, thermal, UG-27/UG-32 head rules, and the axisymmetric shell-to-head junction FE.

## Code Map
| Looking for... | Go to |
|---|---|
| Lame stresses, failure margins, thickness, buckling, thermal | `src/{stress,failure,thickness,buckling,thermal,geometry,pressure}.rs` |
| UG-27 / UG-32 code-rule thickness and MAWP | `src/heads.rs`, `docs/pressure-vessel-heads.md` |
| Junction FE (cylinder + hemisphere / ellipsoid, discontinuity stress), thin-shell reference | `src/junction.rs`, `tests/junction.rs`, `docs/pressure-vessel-heads.md` |

## Contracts
- `junction.rs` builds on `fea-core` (`grid` + axisymmetric `Model`), with no kernel changes. Stresses are linear elastic, linearised through the wall (membrane + bending), meridional / hoop components in the shell frame.
- Only the equal-wall cylinder-hemisphere case has a closed-form reference (`cylinder_hemisphere_reference`, Roark 7th ed. Tables 13.1-13.3 at `phi = 90 deg`). The ellipsoid has none sourced: do not describe it as Roark-validated.
- Never loosen the `tests/junction.rs` tolerances to pass; they are tied to measured errors that fall with `R/t`.

## Pitfalls
- The closed-form Geckeler estimate for a cylinder-ellipsoid junction underestimates the head-side peak bending (35 % at R/t 50, 7 % at 400; `docs/pressure-vessel-heads.md`). Do not quote it as conservative.
- The Lame hoop stress is not uniform through the wall, so a far-field "bending" hoop of about 1 % at `R/t` = 50 is real, not an error.
- The first release build of `fea-core` in a fresh worktree takes ~15 minutes.
