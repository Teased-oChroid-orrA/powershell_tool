# Minimum wall thickness at each failure criterion

`pressure-vessel-solver/src/criterion_thickness.rs` (owner request 2026-11-15). The workbench reports, separately for every
requirement the user enters, the thinnest wall at which that one criterion's margin reaches the required value, plus the governing
(thickest) one and which criterion it is. The owner compares the numbers against their own program requirements outside the tool,
so **no factor, required margin or allowable is defaulted**.

## Requirement row
`CriterionRequirement { label, criterion, factor, required_ms }`: `MS = allowable / (factor * applied) - 1 >= required_ms`
(crate convention; the factor multiplies the demand, which equals scaling the load because stresses and the buckling pressure demand
are linear in pressure). One criterion may appear in several rows (e.g. proof and burst). Criteria: yield by maximum principal,
von Mises, Tresca; ultimate (`Ftu`) by maximum principal and by von Mises; external-pressure buckling (`buckling.rs`).

Public context for what one *might* enter, for reference only and not used by any code: 14 CFR 25.1435 (proof 1.5 and ultimate
3.0 x design operating pressure for tubes and fittings), 25.303 (1.5 on limit load), 25.625 (1.15 fitting factor). The owner's
program (Boeing) requirements are not public and are entered by the user.

## Cylinder (`solve_per_criterion`)
Bore fixed, outer radius found by the same bisection as `thickness::solve_minimum_thickness` (shared helper `bisect_outer_radius`),
once per row on that row's margin only. Outcomes per row: `Converged` (wall, outer radius, margin reached), `Infeasible` (nothing up
to 1000x the bore meets it: reported with the best margin found), `NotEvaluable` (buckling without external pressure or without an
unsupported length). `governing()` is an infeasible row if any, else the converged row with the largest wall.
Buckling: beyond the thin-shell validity range (`D/t < 40`) the requirement counts as met and the row is flagged
`validity_limited`, i.e. the reported wall is that limit and no buckling margin is claimed there.
`solve_minimum_thickness` is unchanged in behaviour (now a thin wrapper over the shared bisection).

## Supported line run (`solve_line_run_walls`)
Outer diameter, supports and loads fixed, wall varied (the bore changes). The class's own factor of safety is already in
`LineMargin`; the row adds `required_ms`. Wall also changes the tube weight and the thermal restraint force, so monotonicity is NOT
assumed: 64 log-spaced walls from `1e-4` to `0.95` of the outer radius are scanned, the answer is bracketed at the last failing
sample and bisected. `BelowSearchFloor` means the requirement holds even at the thinnest wall searched; `Infeasible` that it fails at
the thickest; `NotEvaluable` that the material class does not evaluate that criterion. A non-monotone dip between two samples
narrower than the scan ratio (~1.2) could be missed: stated limit.

## Proof (`cargo test -p pressure-vessel-solver`)
- Closed forms: Tresca, open end, `t = a (sqrt(S/(S-2p)) - 1)` with `S = Sy / (f (1 + MS))`; maximum principal, both end conditions,
  `t = a (sqrt((S+p)/(S-p)) - 1)`; thin limit `t -> p a / S` (relative difference `p/2S`).
- Each row's wall reproduces `margin >= required` with margin within 1e-6 above it, and a wall 1e-6 thinner fails (minimality).
- Governing equals the maximum of the rows; with the four existing modes at one common requirement it equals
  `solve_minimum_thickness` exactly, and the per-criterion margins equal `failure::evaluate_failure_modes`.
- Monotonicity of every criterion margin in wall thickness (the property the bisection needs), sampled per criterion.
- Line run: reproduction and minimality per criterion through `analyze`, the thin-limit wall `t = FS p r_o / Ftu`, class refusal.
- Tolerances are never loosened; invalid rows (factor `<= 0`, non-finite) are rejected, not defaulted.

## Window (Pressure Vessel, `app-tui`)
The window shows this as "Minimum wall by requirement" (owner-approved mockup): Requirement, Criterion, Factor, Required MS, Minimum
wall (a number and a bar against the wall entered, tick at the wall entered) and Margin at wall (met / short). The governing row is
tinted; the footer states the governing wall (and, for the cylinder, the nominal equivalent with the allowances).
- **Seeding.** Rows start from the material's class (`requirements::MaterialKind`: catalog elongation under 5 % is brittle, id `cfrp*`
  composite; the person can flip ductile / brittle): ductile yield von Mises and Tresca plus ultimate, brittle maximum principal against
  ultimate. Allowables come from the catalog and can be overridden (yield, ultimate, ultimate compression; compression defaults to the
  ultimate tension for a brittle line run and the window says so). Factor 1.0 and required MS 0.0 are neutral and shown with a hint to
  enter the program's values; a factor is never invented. A new material re-seeds only unedited rows and clears the overrides.
- **Cylinder:** `solve_per_criterion` inline on every edit (milliseconds); Results step card. Coulomb-Mohr is not evaluable for a plain
  cylinder: the row says so instead of being dropped.
- **Line run** (supports, point loads such as a valve, load cases): analysed inline for the drawing, its walls solved by
  `solve_line_run_walls` on the window's worker (stale answers dropped by `walls_revision`). The class safety factors are fixed at 1; a
  row's factor `f` and required margin `r` are folded in as the equivalent `required_ms' = f (1 + r) - 1`. Margin at wall is the
  factored margin at the wall entered, lowest over all cases. The margin curve under the drawing is, per station of the selected case,
  the lowest over the rows of `factored margin - required margin`, so zero is "just met"; the marker is its minimum.
- The rows travel in the case file (format 4; older cases load seeded) and the table is in the exported report.
- Live in the page, not the title bar: the equilibrium chip (sum of reactions against load) and Export report are the page's first row.

## Not done
- The line run's Tube / Governing / Reactions cards show the selected case only for reactions; there is no per-case table.
