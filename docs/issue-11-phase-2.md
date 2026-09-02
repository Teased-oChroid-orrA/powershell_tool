# Issue #11 Phase 2: geometry model + closed/open-end axial stress

New crate `pressure-vessel-solver`: cylindrical vessel geometry,
validation, thin/thick-wall classification (reporting-only, per issue
#11's own core mandate), pressure loading, and the full applicable
stress-state evaluation (radial, hoop, axial) at any radius. Depends on
`mechanics-core` (issue #11 Phase 1) for the actual Lamé physics and
`engineering-math` (issue #10 Phase 1) as a declared dependency for
later phases (not yet consumed - units/precision/trace wiring is a later
phase, kept out of this one to avoid mixing concerns).

## New physics: closed-end axial stress

`lame.rs`'s only existing axial-stress formula
(`axial_scale * nu * (sigma_r + sigma_theta)`) models a mechanically
end-*constrained* part (a press-fit bushing pinned by its housing) - a
different physical mechanism from a real pressure vessel's closed-end cap
force. Added `mechanics_core::lame::closed_end_axial_stress`: derived from
axial force equilibrium on the end cap (internal pressure pushing outward
on the bore area, external pressure pushing inward on the full outer
area, reacted uniformly by the wall's annular cross-section) -
`sigma_z = (p_inner*a^2 - p_outer*b^2) / (b^2 - a^2)`, which is exactly
[`lame_constants`]'s `C_1` (not a coincidence - documented in the function's
own doc comment why the equilibrium algebra reduces to it). An open-end
condition needs no function at all: zero pressure-induced axial stress.

Verified against a standard textbook reference case (Shigley's
*Mechanical Engineering Design*, thick-wall cylinder chapter): ID 4 in
(a=2), OD 6 in (b=3), 5000 psi internal pressure, closed ends -> 4000 psi
axial stress, 13000 psi hoop stress at the inner surface, -5000 psi radial
stress at the inner surface (exactly `-p_internal`, the boundary
condition). All three numbers match the cited textbook values exactly,
not approximately.

## Geometry classification

`classify()` uses Shigley's own stated criterion (thin-wall when wall
thickness <= 10% of inner radius) rather than inventing a three-zone
"transition" band with no citation - issue #11 itself warns thresholds
"shall not be treated as universal physical boundaries," and a fabricated
transition zone would be exactly that. Classification never appears in
`stress.rs`'s control flow - both classifications call the identical
`mechanics_core::lame` functions, satisfying issue #11's core mandate
directly rather than by convention.

## Verification

- `cargo build -p pressure-vessel-solver` / `--workspace`: clean.
- `cargo test -p pressure-vessel-solver`: **14/14**, including:
  - the Shigley reference case (radial/hoop/axial all matching cited
    textbook values),
  - boundary-condition sanity (`sigma_r` at each surface equals exactly
    `-p_internal`/`-p_external`),
  - open-end zero-axial-stress,
  - qualitative physical sanity (hoop stress decreases from bore to OD
    under internal-only pressure; external-only pressure puts the wall in
    compression),
  - geometry validation (rejects non-positive inner radius, outer <=
    inner) and classification at and around the threshold.
- `cargo test --workspace`: unaffected everywhere else (real diff
  reviewed before committing - this phase only adds new, currently-
  standalone files; nothing existing was touched).

## Next

Phase 3 (materials audit - expected to be a short confirmation, not new
work: `Material`'s existing `sy_ksi`/`ftu_ksi`/`e_ksi`/`nu` fields already
cover what yield/Von Mises/Tresca/ultimate margins need) and Phase 4
(failure-mode + margin-of-safety engine).
