# Issue #11 Phase 5: minimum wall thickness solver

New `pressure-vessel-solver/src/thickness.rs`: solves for the minimum
outer radius (wall thickness) satisfying a user-required minimum Margin
of Safety across all four v1 failure modes - issue #11's core Phase 8
ask, with its explicit warning honored directly in the code: every
candidate thickness re-runs `evaluate_failure_modes` in full and takes the
`governing` (minimum) margin, never assuming which mode controls or
solving only one.

## Method: bisection, justified by a verified (not assumed) property

Increasing wall thickness at fixed inner radius/pressure never makes any
of the four v1 margins worse - the expected physical behavior (more
material reacting the same load), but verified directly
(`governing_margin_is_monotonically_non_decreasing_with_wall_thickness`)
before relying on it to make bisection sound, not taken on faith. The
solver expands an upper search bound by doubling the wall thickness until
it satisfies the requirement (capped at 1000x the inner radius - a bound
no real design would need), then bisects to the requested radius
tolerance.

## A real infeasibility case, confirmed by the physics

Internal pressure alone imposes a genuine, finite upper bound on
achievable margin no matter how thick the wall gets: as outer radius ->
infinity, inner-surface hoop stress asymptotically approaches the applied
internal pressure itself (the classical "infinite plate with a hole"
limit), never below it. A required minimum MS higher than
`material_allowable / internal_pressure - 1` is therefore **truly**
infeasible, not just "outside the search's patience" - confirmed with a
test requesting an unreasonably high MS (1,000,000) against a case whose
real asymptotic ceiling is nowhere close, and checking the solver reports
`Infeasible` rather than looping or fabricating a number.

## Verification

- `cargo test -p pressure-vessel-solver`: **28/28** (5 new), including:
  - the monotonicity property itself,
  - a solved solution's own governing margin genuinely satisfying the
    requirement,
  - a wall 10% thinner than the solved solution genuinely failing the
    same requirement (proves the solver found the real minimum, not just
    *a* feasible thickness),
  - a `required_minimum_ms = 0.0` case converging tightly near the
    zero-margin boundary (matches the shape of issue #11's own worked
    example, "MS Required = 0.00"),
  - the confirmed-infeasible case above.
- `cargo test --workspace`: unaffected everywhere else.

## Next

Phase 6 (UI: `pressure_vessel_workbench.rs` + `ToolId::PressureVessel`,
reusing the Bushing Workbench's proven Step/`DesignStatusRail`/
`CheckGauge`/headline/derivation-view patterns).
