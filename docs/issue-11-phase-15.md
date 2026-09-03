# Issue #11 Phase 15: mirror the approved mockup exactly

Direct instruction: match the approved mockup artifact's UI/UX exactly,
no deviations except where genuinely impossible, applied consistently
across every tool - not just Pressure Vessel.

## Gap analysis against the mockup (what Phase 14 actually shipped vs.
what was approved)

- Rail nav items were plain text `selectable_label` rows - no icon, no
  two-line title+desc, no "Soon" pill, no hover/active treatment.
- Step tabs were plain `selectable_label` buttons - not the mockup's
  pill shape with a step-number circle and em-dash separators.
- The Results headline was an ad-hoc horizontal row, not the mockup's
  dot+status+mini-stat-columns layout.
- Cards used inline `egui::Frame` calls repeated per call site instead
  of one consistent styled primitive.
- **The labeled engineering cross-section sketches - the single most
  worked-on piece of the whole mockup review (multiple rounds: overlap
  fixes, hatch-intensity tuning, off-canvas label fixes) - were not
  implemented at all.** Flagged as a known gap in Phase 14's own doc,
  now closed.
- Bushing Workbench's steps didn't match the mockup's actual step names/
  grouping: shipped as Geometry/Fit/Material/Loads, when the approved
  mockup (and the original `bushing_workbench.rs` it was built from) use
  Repair/Geometry/Material/Fit/Analysis. Housing/bore fields were living
  under a step called "Geometry" when the mockup calls that step
  "Repair" - a real, if unintentional, deviation.

## What this phase fixes

- **`widgets.rs`** (new): `card`, `stepper`, `nav_item`, `headline` -
  one shared implementation of each mockup chrome piece, used by every
  tool identically, so fixing one fixes all three consumers at once
  rather than three near-duplicate implementations drifting apart.
- **`sketches.rs`** (new): the labeled cross-section sketches, ported
  directly from the mockup's hand-authored SVG coordinate math into
  `egui::Painter` calls - not rasterized through resvg (simpler, crisper,
  and this crate already has direct `Painter` access). Same drafting
  conventions the mockup settled on after real feedback: ANSI-style
  hatch for cut material, dash-dot centerlines, a center-mark cross,
  dimension lines kept outside the part, and the "hidden not translucent"
  rule - only the active step's dimension group draws at all, everything
  else is fully absent rather than faded. One disclosed approximation:
  hatch fill clips to the shape's bounding rect rather than its exact
  outline (`egui::Painter` has no arbitrary clip-path primitive) - reads
  as "hatched material" correctly, just not pixel-exact at a ring's
  curved edge.
- **`pressure_vessel.rs`/`bushing.rs`**: rebuilt to use the new widgets
  throughout, and each non-Results step now shows its labeled sketch
  pair (head-on + side view) beside the form, exactly the mockup's
  `.step-wrap` shape.
- **Bushing step realignment**: renamed/regrouped to
  Repair/Geometry/Material/Fit/Analysis/Results, matching the mockup and
  the original app exactly. "Geometry" is now genuinely about flanged
  OD geometry (a real `flanged`/`flange_od`/`flange_thk` toggle wired to
  `BushingInputs::bushing_type`/`flange_od`/`flange_thk` - closing a
  Phase 14 gap in the process, not just relabeling).

## One disclosed, deliberate non-mirror

The mockup's Fit step showed a "Fit class" chip row (Class 1/2/3) as
descriptive text - `BushingInputs` has no corresponding field, so it
never drove real computation in the mockup either (it was illustrative).
Shipping a selector that doesn't affect the result would be a fake
control, which is worse than the honest gap of omitting it. Only the
real `interference` input is here. Everything else in this phase has a
real, working control behind it.

## Verification

- `cargo build --workspace`, `cargo test --workspace`: clean, all
  existing suites green.
- CI (`rust-build.yml`, `app-egui-build-windows` job): win-x64 release
  build, run per this phase's standing "commit, push, CI, fix, repeat"
  instruction.
- **Still not verified**: actual on-screen appearance/spacing/proportion
  match against the mockup. No local GUI capability in this environment
  - CI proves this compiles and links for the target platform, not that
  it looks right. A real screenshot is still the only way to confirm
  pixel/spacing fidelity - flagging explicitly rather than claiming a
  visual match from source review alone.
