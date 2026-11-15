# pressure-vessel-solver

## Purpose
Pure calculation crate (zero GUI dependency) for cylindrical pressure components: exact Lame stress, failure margins, thickness,
buckling, UG-27 heads, and the supported-tube line run check. Consumed by `app-tui`'s Pressure Vessel window.

## Code Map
| Module | Owns |
|---|---|
| `stress.rs`, `geometry.rs`, `pressure.rs`, `thermal.rs` | Lame stress state (`StressState`), geometry, pressure + end condition, thermal stress |
| `failure.rs` | von Mises, Tresca, max-|principal|, `margin_of_safety` (`MS = allowable/applied - 1`) |
| `heads.rs`, `thickness.rs`, `buckling.rs` | UG-27 rules, minimum thickness, external-pressure / span buckling |
| `line_run.rs` (+ `line_run/tests.rs`), `line_beam.rs` | Supported tube line: loads, member actions, stress tensor around the circumference, criteria by material class. Doc: `docs/pressure-vessel-line-run.md` |
| `tests/members_cross_check.rs` | `line_run` reactions vs `fea-problem` frame solver (dev-dependency only) |

## Contracts
- Allowables, factors of safety, `S`, `E`: inputs, never defaulted. Unsupported classes (composite, other) and unmodelled loads
  (torque about the line axis) return a reason, not a number.
- `line_run` evaluates the `failure.rs` criteria on the principal triple carried in `StressState` (radial / hoop / axial fields hold
  the principals); do not read those fields as physical directions there.
- Sign convention of `line_beam.rs`: per plane `M = EI w''`, `V = dM/dx`, loads and deflection positive along the plane axis; a moment
  load is positive when it increases the slope. `Station::bend_y` deflects the line in y (stress `-bend_y y/I`).
- Every `line_run` case self-checks equilibrium and solve residual (`BALANCE_LIMIT`); never loosen it to make a case pass.

## Pitfalls
- `pressure_end_load = true` (Closed) and an explicit `flow_thrust` at the same bends double count the pressure thrust.
- Thin-wall `tau_x,theta` is an approximation for thick tubes; `PD/2t`, `PD/4t` tests carry an `O(t/r_m)` tolerance on purpose.
- Roark / Shigley table and page numbers are not verified against the sources (see the doc's Sources section).
