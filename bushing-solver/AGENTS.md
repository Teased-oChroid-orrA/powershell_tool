# bushing-solver

## Purpose
Owns: straight/flanged/countersunk interference-fit (press-fit) bushing calculation - Lamé
thick-wall contact pressure/hoop stress, tolerance-stack resolution, margins of safety
(housing/bushing hoop stress, edge-distance sequencing/strength, wall thickness), install
force, and the aircraft reamer catalog. A scoped Rust port of `~/Claude/Projects/
engineering.toolbox`'s much larger TS bushing workbench (`solveEngine.ts`/`solveMath.ts`/
`bushingProfileGeometry.ts`/`bearing.ts`) - see root `docs/bushing-workbench-status.md` for the
original scope decision (now partly stale on module layout, see Pitfalls).
Does not own: service-duty/wear (PV) screening, process-route review, standards/approval
review (FAA/NAS/SAE/OEM SRM) - explicitly out of scope, never ported. Also does not own general
(non-bushing) Lamé/material math - see `mechanics-core` below.

## Code Map

### Find It Fast
| Looking for... | Go to |
|---|---|
| End-to-end solve (the one real entry point) | `src/solve.rs::compute()` |
| Material properties (Al7075, Ti-6Al-4V, bronze, ...) | `mechanics-core/src/materials.rs` (not here) |
| Lamé thick-wall stress/compliance math | `mechanics-core/src/lame.rs` (not here) |
| Tolerance-band resolution, bore-capability auto-adjust | `src/tolerance.rs` |
| Countersink corner solve + worst-case tolerance search | `src/countersink.rs` |
| Axial cross-section geometry / min wall-thickness scan | `src/geometry.rs` |
| Edge-distance bearing-profile effective thickness | `src/bearing.rs` |
| Aircraft reamer catalog + nearest-size picker | `src/reamers.rs` + `data/aircraft_reamer_catalog.csv` |
| Proof this matches the real production engine | `tests/differential.rs`, `tests/differential_countersink.rs` |

### Key Relationships
`tolerance.rs`/`countersink.rs`/`geometry.rs`/`bearing.rs`/`reamers.rs` → `solve.rs::compute()`
(the only consumer wiring them together) → `mechanics_core::lame`/`mechanics_core::materials`
for the physics/material data. The dependency only goes one direction.

## Design Rationale
- **Problem solved**: interference-fit bushing analysis for straight/flanged/countersunk
  geometry, without the source project's duty/process/approval-review layers.
- **Core insight**: ported line-for-line against real captured TS golden output
  (`tests/differential*.rs`, from actually executing `computeBushing` via `npx tsx`), not
  re-derived from formulas - catches silent porting bugs a "looks right" review would miss.
- **Constraints**: zero GUI dependency (pure library; `app-tui` is the only consumer - `app`/`app-egui` were deleted); imperial units
  only for v1 (in, psi/ksi, lbf, °F) - no metric input path exists.

## Public API

### Key Exports
| Export | Used By | Change Impact |
|---|---|---|
| `solve::compute(&BushingInputs) -> BushingOutput` | `bushing_workbench.rs` (app), `bushing.rs` (app-egui) | Breaking - both UIs read dozens of `BushingOutput` fields directly |
| `solve::{BushingInputs, BushingOutput, RangedValue, MarginCandidate, EndConstraint}` | Both UIs | New field must default via `..Default::default()` (see Contracts) |
| `geometry::{BushingType, IdType, resolve_bushing_section_params, compute_minimum_bushing_wall}` | `bushing_visualizer.rs`, both UIs | Consumed directly for cross-section drawing, not just solving |
| `countersink::{CsMode, CsCorner, solve_countersink}` | Both UIs | Countersink field derivation display |
| `tolerance::{ToleranceRange, ToleranceStatus, EnforcementPolicy}` | Both UIs | Spec-row display, enforcement toggle |
| `reamers::{ReamerEntry, nearest}` | Both UIs | Bore-diameter reamer picker dropdown |
| `mechanics_core::lame::LameSample` (via `BushingOutput` stress-field fields) | Both UIs | Lives in `mechanics-core`, re-exposed by this crate's output type |

### Core Types
`BushingInputs` (all solve inputs, `Default`-derived) → `compute()` → `BushingOutput`.
Note: `RangedValue.min`/`.max` aren't tied to the same achieved-interference extreme (source:
`solve.rs::ranged()`) - it sorts by actual value, not by source: housing hoop stress' max comes
from max interference, but housing margin-of-safety's max (best case) comes from that *same*
max-interference evaluation producing *lower* stress.

## External Dependencies
| Crate | Used For | Failure Mode |
|---|---|---|
| `mechanics-core` (path dep) | Lamé stress/compliance (`lame.rs`) + material table (`materials.rs`) | Compile-time only, zero I/O either crate |

## Entry Points
| Task | Start Here |
|---|---|
| Change or add a solve formula | `src/solve.rs::compute()` (cite the `solveEngine.ts` line it mirrors) |
| Add a material | `mechanics-core/src/materials.rs`'s `MATERIALS` array (not this crate) |
| Verify against real TS engine output | `tests/differential.rs` / `tests/differential_countersink.rs` - each header has the `npx tsx` reproduction command |
| Add a new OD/ID geometry variant | `geometry.rs` + `countersink.rs` + `bearing.rs` together (see Patterns) |
| Add/update a reamer catalog entry | `data/aircraft_reamer_catalog.csv` (sourced data - cite a real catalog) |

## Downlinks / Consumers
No child AGENTS.md, but two active consumers:
- `app` (dioxus-native, being phased out per root `docs/app-egui-parity-checklist.md`):
  `bushing_workbench.rs`, `bushing_visualizer.rs`.
- `app-egui` (active stack): `bushing.rs`, `sketches.rs`/`components.rs`.
Both declare `bushing-solver` **and** `mechanics-core` as direct path deps - no re-export shim,
so a UI needing `lame`/`materials` types imports `mechanics_core::...` directly.

## Contracts
- **Imperial units only, unenforced by types**: `in`, `psi`/`ksi`, `lbf`, `°F` throughout
  (source: `Cargo.toml` module doc, "imperial units only for v1"). A metric value silently
  produces a wrong-but-plausible number, not an error.
- **Every new `BushingInputs` field must default (`..Default::default()`) to reproduce the
  original straight-bushing-only behavior** (`BushingType::Straight`, `IdType::Straight`,
  `EnforcementPolicy::enabled = false`) - stated in `solve.rs`'s own doc comment, load-bearing
  for every existing caller/test that doesn't set the new field.
- **`EnforcementPolicy::enabled = false` (default, source: `tolerance.rs::EnforcementPolicy::
  default()`) reports an infeasible tolerance band as `ToleranceStatus::Infeasible` honestly,
  never silently auto-tightened** - a deliberate v1 cut from the TS source's own bore-capability
  auto-adjustment machinery.
- **A countersink's derived dimension is always re-solved via `solve_countersink` from the other
  two + base diameter - never perturbed independently.** `enumerate_countersink_corners`'s
  worst-case search depends on this; proven against a full brute-force cartesian search in
  `countersink.rs`'s own tests.
- **Hoop stress boundary values and the stress-field plot must both come from `mechanics_core`'s
  `lame` module** (`lame_stress_at_radius`/`sample_lame_field`), never a hand-rolled duplicate -
  `solve.rs` calls this out directly since a second copy could drift.

## Patterns

### Changing or adding a solve formula
1. Find the corresponding line(s) in `engineering.toolbox`'s TS source (cited in doc comments).
2. Change `solve.rs` (or the relevant module) to match, keeping the TS line-number comment.
3. Extend `tests/differential.rs`/`differential_countersink.rs` with a golden value captured by
   actually running the TS engine (`npx tsx`, command documented at the top of each file) -
   internal self-consistency alone isn't accepted as proof here.

### Adding a new bushing OD/ID geometry variant
"Straight-bushing-only" eliminates whole subsystems, not just input fields: excluding
countersink/flange geometry collapses `bearing.rs`'s `t_eff_sequence` to a single cylindrical
segment (`eta = 1.0`) and removes the corner-enumeration worst-case search entirely. Expect a
new variant to touch `countersink.rs`, `geometry.rs`, and `bearing.rs` together.

### Adding a new countersink-derived-dimension tolerance formula
Mirror `cs_dia_tolerance_from_base`/`cs_depth_tolerance_from_base`/
`cs_angle_tolerance_from_base`: derive the monotonicity direction yourself (don't assume it
matches a sibling formula's sign - the depth-tolerance formula flips it), then prove it against
`all_three_derived_tolerance_formulas_match_a_full_brute_force_cartesian_search`, not just the
closed-form derivation.

## Boundaries

### Always
- Verify a new/changed formula against `tests/differential*.rs` real captured TS golden values.

### Never
- Reference `bushing_solver::lame` or `bushing_solver::materials` - moved to `mechanics-core`
  (issue #11 Phase 1); no re-export shim exists.
- Hand-roll a second Lamé stress/compliance formula - always go through `mechanics_core::lame`.

### Ask First
- Changing `BushingInputs.mat_housing`/`mat_bushing` back to string ids: they are resolved `mechanics_core::materials::Material` values (same pattern as `pressure-vessel-solver`), which is what makes custom/library materials work in `app-tui`. Callers resolve the material first; this crate does no id lookup.
- Changing what an existing `BushingInputs` field defaults to, or its type - its UI consumer (`app-tui`)
  reads many fields by name and existing tests assume current defaults.

## Pitfalls
- **Root `docs/bushing-workbench-status.md` still describes `materials.rs`/`lame.rs` as living
  in this crate** - extracted to `mechanics-core` in commit `0a9071d` (issue #11 Phase 1), doc
  never updated. Trust `src/lib.rs` (documents the move) and this file instead.
- **A `.max()`-style chain for `Fbru_ksi || Sy_ksi || 0` would silently reproduce JS
  falsy-fallback semantics instead of a numeric fallback** - caught before the differential test
  even ran, fixed with an explicit `if fbru_ksi != 0.0 { .. } else { .. }`. Port any `||`-style
  TS fallback as an explicit zero/non-finite check, never `.max()`.
- **`EndConstraint` (Free/OneEnd/BothEnds) silently did nothing for a while**:
  `axial_constraint_factor`/`axial_length_factor` were computed but never multiplied into an
  actual axial stress. Fixed; `end_constraint_both_ends_matches_real_ts_axial_stress` proves
  `BothEnds` is exactly 2x `OneEnd`, not just a hardcoded golden pair.
- **Install-time shrink-fit thermal assist used to be silently discarded**: `install_force`/
  `install_pressure` just reused in-service `pressure`/`retained_install_force` unconditionally,
  ignoring `assembly_*_temperature` entirely. Fixed via `install_delta`; if you touch this path,
  `no_assembly_temperature_makes_install_force_equal_retained_install_force` must still pass
  (the two must agree exactly when neither assembly temperature is set).
- **`cs_*_angle_tol_out` used to unconditionally call the derive-formula, discarding the user's
  own entered angle tolerance whenever angle was a *direct* input** (`DepthAngle`/`DiaAngle`
  mode - only `DiaDepth` should derive angle). Fixed; regression test
  `angle_tolerance_survives_to_the_output_when_angle_is_a_direct_input` guards this.
