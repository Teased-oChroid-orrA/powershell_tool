# Issue #11 Phase 6: UI

New `app/src/pressure_vessel_workbench.rs` + `ToolId::PressureVessel`,
wired into the Toolbench rail/topbar/stage the same way `ToolId::Bushing`
already is.

## Scope decision: single-page, not a multi-step wizard

The Bushing Workbench's Step-based wizard (`StepperNav`/`DesignStatusRail`)
exists because that tool has 30+ inputs across genuinely distinct concerns
(geometry, countersink, material, fit, environment). v1's whole input set
here - two radii, two pressures, an end condition, a material, and a
required margin, seven fields - is small enough that a stepper would be
ceremony without benefit. This *is* still "reusing the Bushing Workbench's
proven patterns" as planned: `NumberField`, `MaterialField`, `CheckGauge`/
`CheckRowData`, and `margin_class`/`fmt_margin` (Phase 6 prep commits)
were extracted into `components.rs` specifically so this tool could depend
on the real, already-proven components rather than reimplementing them -
the wizard shell itself just isn't warranted by this tool's input count.

## What's built

- Geometry card (inner/outer radius), Pressure card (internal/external
  pressure + closed/open end condition chips), Material/requirement card
  (material picker + required minimum MS) - all real `NumberField`/
  `MaterialField` components, not placeholders.
- A headline banner (PASS/REVIEW, governing failure mode, geometry
  classification, wall thickness) reusing the exact `bushing-headline`
  visual pattern.
- A geometry-classification note stating explicitly that thin/thick-wall
  classification is for interpretation only - both classifications run
  the identical full Lamé solution, matching issue #11's core mandate in
  the UI copy itself, not just in the underlying code (already true since
  Phase 2).
- A Checks card: all four v1 failure-mode margins as `CheckGauge` rows
  (via a new `margin_result_to_row` adapter from `pressure_vessel_solver::
  failure::MarginResult` to the shared `CheckRowData`).
- A minimum-wall-thickness card: solved outer radius/wall thickness/
  governing mode on success, an explanatory message (citing the real
  asymptotic-limit reason from Phase 5) on `Infeasible`, and an "Apply
  solved outer radius" button that sets the outer-radius input directly -
  the same one-click-apply pattern the Bushing Workbench's reamer picker
  already uses.
- Invalid geometry/pressure inputs (e.g. outer <= inner, negative
  pressure) show a real validation message instead of silently computing
  garbage or panicking - the whole results section is gated on both
  `CylinderGeometry::new`/`PressureLoading::new` succeeding.

## CSS: reused verbatim, not duplicated

Every class this page uses (`bushing-page`, `bushing-headline`,
`bushing-card`, `field-row`, `chip`/`chip-row`, `check-item`/`value-track`
via `CheckGauge`, `bushing-detail-grid`, `bushing-alert`, `link-button`) is
the Bushing Workbench's own existing CSS, unchanged - a deliberate
decision to keep one visual language across tools rather than fork a
parallel `pv-*` class set for no functional difference. The `bushing-`
prefix on some class names is a naming artifact of which tool defined them
first, not a semantic restriction - functionally these are already
generic layout/color rules.

## Verification

- `cargo build -p app` / `--workspace`: clean, zero new warnings.
- `cargo test -p app` / `--workspace`: unchanged (25/149/etc. - this phase
  adds a UI file with no `#[cfg(test)]` module of its own, matching
  `bushing_workbench.rs`'s own convention: UI composition isn't unit-
  tested, the pure logic underneath it already is, exhaustively, in
  `pressure-vessel-solver`'s own test suite from Phases 2, 4, and 5).
- **Not independently verified**: actual rendering in the real
  `dioxus-native`/Blitz window. Same standing limitation this whole
  session's Bushing Workbench UI work has had - no local GUI capability
  in this environment. A screenshot round-trip with the user is the real
  verification step for this phase, same as every prior UI phase.

## Next

Phase 7 (full-suite validation - already continuously green through
Phases 1-6, so this is mostly a final confirmation pass) and Phase 8
(docs rollup / final completion report).
