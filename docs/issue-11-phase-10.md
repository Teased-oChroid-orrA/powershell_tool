# Issue #11 Phase 10: Bushing Workbench visual parity + full derivation view

Full UI redesign in response to direct feedback ("i dont like how it
looks... make the UI/UX similar to the bushing"), approved from a mockup
("i love the mockup to the letter") before any code was written -
matching the established mockup-then-implement discipline this whole
session has used for every visual round.

## What changed

Replaced the Phase 9 input-rail single-page layout entirely with the
Bushing Workbench's own shape:

- **`PvStep`/`PvStepperNav`** - a 5-step guided sequence (Geometry,
  Pressure, Material, Buckling, Results), mirroring `bushing_workbench.
  rs`'s own `Step`/`StepperNav` pattern exactly but kept as this file's
  own small, separate types rather than genericized versions of the
  Bushing Workbench's - those are tied to bushing-specific types
  (`Step`'s own variants, `ToleranceStatus`), and genericizing them risked
  the Bushing Workbench's own already-verified behavior for a savings of
  a few dozen lines. Same reasoning as Phase 6's original scope decision,
  just landing differently now that the user wants the fuller pattern.
- **`PvStatusRail`** - persistent design-status rail beside the workspace,
  same role as `DesignStatusRail`, simplified (no tolerance-band concept
  in this tool).
- **Vessel specification summary card** - a fabrication-style card
  (`fab-card`/`fab-badge`/`fab-grid`, the exact classes the Bushing
  Workbench's own equivalent card uses) listing outer/inner diameter,
  wall thickness, design pressure, end condition, material, and the
  governing failure mode, plus a note comparing the current wall
  thickness to the solved minimum.
- **8-step derivation view**, all real, cited/derived exactly as flagged:
  1-4 reuse the Bushing Workbench's own existing rendered PNG assets
  directly (`radial_equilibrium_ode`, `lame_trial_form`,
  `lame_constants_solved` - the *exact same* general Lame physics, not
  re-rendered) plus one new asset (`pv_hoop_at_inner_surface`, evaluating
  the trial form at the bore); 5-8 are new (`pv_closed_end_axial_stress`,
  `pv_von_mises_stress`, `pv_tresca_stress`, `pv_windenburg_trilling` -
  the one step tagged `Cited` per the actual code in `buckling.rs`, not
  presented as derived here either). Rendered via the same KaTeX +
  Playwright pipeline, same exact text colors (`#eef0f4`/`#171a1f`,
  verified against the existing PNGs' own pixels back in the bushing
  derivation round), same `include_bytes!`-embedded, offline, no runtime
  dependency. Every value line is computed from the vessel's real,
  current inputs (not a worst-case-across-tolerance derivation like the
  Bushing Workbench's own - this tool has no tolerance-band concept).

## A real bug caught and fixed while reviewing the diff

Phase 9's CSS added `.results-column { display: flex; ... }` - which
collided with an *already-existing*, unrelated `.results-column` class
the Search tool's own `ResultsPanel` uses (`main.rs` line ~895, pre-
dating this epic entirely). Because this app has no CSS scoping, both
rules would have applied to the Search tool's results column, silently
adding `display:flex; flex-direction:column; gap` to an element that
never asked for it. Caught by reviewing the diff before this commit (per
standing "work with diff" instruction), not by inspection alone - the
whole `.input-rail-layout`/`.input-rail`/`.results-column`(duplicate)
block Phase 9 added is now removed entirely, since Phase 10's stepper
layout replaces that approach anyway.

## Verification

- `cargo build -p app` / `--workspace`: clean, zero warnings (two
  `unused_mut` warnings from the removed "apply solved thickness" button
  - not present in the approved mockup, so not re-added - were fixed
  before this commit).
- `cargo test --workspace`: unchanged everywhere - this phase is UI
  composition plus new formula assets, no engine logic touched.
- Full diff reviewed before committing, including the Phase 9 CSS-
  collision fix above.
- New formula PNGs spot-checked visually (`pv_von_mises_stress_light`,
  `pv_windenburg_trilling_dark`) - correct LaTeX, correct color per theme
  variant.
- UI rendering still not independently verified end-to-end - no local GUI
  capability in this environment, same standing limitation as every UI
  phase this session.
