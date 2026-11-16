# pressure-vessel-solver

## Purpose
Pure calculation crate (zero GUI dependency) for cylindrical pressure components: exact Lame stress, failure margins, thickness,
buckling, UG-27 heads, the axisymmetric shell-to-head junction FE (on `fea-core`), and the supported-tube line run check. Consumed by `app-tui`'s Pressure Vessel window.

## Code Map
| Module | Owns |
|---|---|
| `stress.rs`, `geometry.rs`, `pressure.rs`, `thermal.rs` | Lame stress state (`StressState`), geometry, pressure + end condition, thermal stress |
| `failure.rs` | von Mises, Tresca, max-|principal|, `margin_of_safety` (`MS = allowable/applied - 1`) |
| `heads.rs`, `thickness.rs`, `buckling.rs` | UG-27 rules, minimum thickness (governing, shared `bisect_outer_radius`), external-pressure / span buckling |
| `criterion_thickness.rs` | Minimum wall at EACH criterion separately (user factor + required margin per row), cylinder and line run. Doc: `docs/pressure-vessel-thickness.md` |
| `line_run/` (`mod.rs`, `segment.rs`, `tests.rs`), `line_beam.rs` | Supported tube line (+ segment mode: end fixity, rotational-spring clips, pinned/clamped bounds): loads, member actions, stress tensor around the circumference, criteria by material class. Doc: `docs/pressure-vessel-line-run.md` |
| `junction.rs`, `tests/junction.rs` | Junction FE (cylinder + hemisphere / ellipsoid, discontinuity stress, membrane + bending per station), thin-shell reference. Doc: `docs/pressure-vessel-heads.md` |
| `tests/members_cross_check.rs` | `line_run` reactions vs `fea-problem` frame solver (dev-dependency only) |

## Contracts
- Allowables, factors of safety, `S`, `E`: inputs, never defaulted. Unsupported classes (composite, other) and unmodelled loads
  (torque about the line axis) return a reason, not a number.
- `line_run` evaluates the `failure.rs` criteria on the principal triple carried in `StressState` (radial / hoop / axial fields hold
  the principals); do not read those fields as physical directions there.
- Sign convention of `line_beam.rs`: per plane `M = EI w''`, `V = dM/dx`, loads and deflection positive along the plane axis; a moment
  load is positive when it increases the slope. `Station::bend_y` deflects the line in y (stress `-bend_y y/I`).
- `criterion_thickness.rs`: factors and required margins are per-row inputs, never defaulted (public FAR numbers are doc examples only). Line-run walls are scanned, not bisected blind: wall changes weight and thermal force, so margin is not assumed monotone. Cylinder bisection relies on the per-criterion monotonicity test.
- `Support.rotation_stiffness` (spring, moment/rad) conflicts with `rotation_fixed` and is refused if negative / non-finite. `analyze_segment_bounds` (pinned-pinned vs clamped-clamped) brackets the stress margin of uniformly loaded spans only: do not present its end actions or reactions as bounds (clamped `wL^2/12` < continuous `wL^2/10`; tested). Free ends are refused there. Cut-end loads from a full run go in as `PointLoad::moment` / `force` (no separate API). Adding a `Support` field breaks struct literals outside this crate.
- Every `line_run` case self-checks equilibrium and solve residual (`BALANCE_LIMIT`); never loosen it to make a case pass.
- `junction.rs` builds on `fea-core` (`grid` + axisymmetric `Model`), no kernel changes. Linear elastic stresses linearised through the wall, meridional / hoop in the shell frame.
- Only the equal-wall cylinder-hemisphere case has a closed-form reference (`cylinder_hemisphere_reference`, Roark 7th ed. Tables 13.1-13.3 at `phi = 90 deg`). The ellipsoid has none sourced: do not describe it as Roark-validated.
- Never loosen the `tests/junction.rs` tolerances to pass; they are tied to measured errors that fall with `R/t`.

## Pitfalls
- The closed-form Geckeler estimate for a cylinder-ellipsoid junction underestimates the head-side peak bending (35 % at R/t 50, 7 % at 400; `docs/pressure-vessel-heads.md`). Do not quote it as conservative.
- The Lame hoop stress is not uniform through the wall, so a far-field "bending" hoop of about 1 % at `R/t` = 50 is real, not an error.
- The first release build of `fea-core` in a fresh worktree takes ~15 minutes.
- `pressure_end_load = true` (Closed) plus an explicit `flow_thrust` double counts the pressure thrust: mark thrust loads `pressure_thrust: true`; `analyze` refuses the combination.
- Thin-wall `tau_x,theta` is an approximation for thick tubes; `PD/2t`, `PD/4t` tests carry an `O(t/r_m)` tolerance on purpose.
- Roark / Shigley table and page numbers are not verified against the sources (see the doc's Sources section).
