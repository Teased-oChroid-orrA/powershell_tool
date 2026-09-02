# Issue #11 Phase 4: failure-mode + margin-of-safety engine

New `pressure-vessel-solver/src/failure.rs`: the four v1 failure modes
confirmed data-available in `docs/issue-11-phase-3.md` - yield (maximum-
stress theory), Von Mises, Tresca (maximum shear), ultimate - each
evaluated at both the inner and outer surface with the worse (critical)
location reported explicitly, never assumed.

## Principal stresses, for real

For this crate's axisymmetric Lamé stress state, radial/hoop/axial stress
already ARE the three principal stresses - no shear exists on the
r-theta/theta-z/r-z planes by symmetry (Timoshenko & Goodier, *Theory of
Elasticity*). No Mohr's-circle transformation is performed or needed;
`principal_stresses()` is a direct, documented statement of this fact, not
a placeholder.

## Margin convention

`allowable / applied - 1`, matching `bushing-solver`'s own existing
convention (checked before choosing this, per issue #11's own "inspect
existing MS conventions... must be consistent across the tool"). Zero
applied stress reports `f64::INFINITY`, not a panic or a fabricated
number.

## A real finding caught by running tests, not by inspection

An early draft of the test suite asserted that external-pressure-only
loading would make the *outer* surface govern at least one mode - a
plausible-sounding guess that turned out to be **false** when actually
run: for a thick cylinder, hoop stress magnitude is largest at the
**inner** surface even under external-pressure-only loading (a real,
citable elasticity result, not a bug). Hand-checking a combined-pressure
case to find a real "outer governs" example also failed - for this vessel
shape, the inner surface governs all four v1 modes under every
non-negative pressure combination tried. The wrong test was replaced with
one asserting the actual (verified) behavior, plus a note in `failure.rs`'s
own module doc explaining why, and `worst_of`'s location-selection logic
is separately verified against synthetic, hand-controlled stress data
where the outer surface is deliberately made to have the larger demand -
so this is "the physics for this vessel always picks inner," not "the
code can only ever pick inner."

## Verification

- `cargo test -p pressure-vessel-solver`: **23/23** (9 new), including:
  - Von Mises and Tresca both reducing to the exact uniaxial stress value
    for a single nonzero principal stress (the standard calibration check
    for either criterion),
  - Von Mises of an equal triaxial (hydrostatic) stress state being
    exactly zero (a real physical property: pure hydrostatic stress
    causes no shape distortion),
  - zero-applied-stress margin being `f64::INFINITY`, not a panic,
  - yield vs. ultimate genuinely using different `Material` fields
    (`sy_ksi` vs. `ftu_ksi`) for the same demand function,
  - `governing()` picking the true minimum among a mixed positive/
    negative margin set,
  - the corrected external-pressure/inner-surface finding above, and
    `worst_of`'s location logic verified independently with synthetic
    data in both directions.
- `cargo test --workspace`: unaffected everywhere else.

## Next

Phase 5 (minimum-thickness solver) - iterates candidate thickness,
re-evaluates all four modes via `evaluate_failure_modes` at each
candidate, tracks the controlling margin, converges to the user's
required minimum MS, and must NOT assume which mode controls stays fixed
as thickness changes.
