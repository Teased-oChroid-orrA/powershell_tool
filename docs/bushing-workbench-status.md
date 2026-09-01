# Bushing Workbench: straight-bushing interference-fit calculator

A new Toolbench tool (`app/src/bushing_workbench.rs`, backed by the new
`bushing-solver` crate) ported from
`~/Claude/Projects/engineering.toolbox`'s much larger aerospace-grade
bushing workbench. Scoped deliberately - see "Scope decision" below - not
a 1:1 port of the source project's entire tool.

## Source project and why this is a scoped subset, not a full port

`engineering.toolbox`'s bushing workbench
(`src/lib/core/bushing/{solveEngine,solveMath,materials,reamerCatalog}.ts`,
~4,100 lines, 40+ Svelte UI components) analyzes press-fit/interference-fit
bushings for straight *and* countersunk/flanged geometry, with tolerance-
stack enforcement policy, service-duty/wear (PV) screening, process-route
review, and standards/approval review (FAA AC 43-13, NAS/MS, SAE AMS, OEM
SRM). Porting all of that was explicitly out of scope for this pass - the
user chose "core engine + essential UI" (Lamé interference-fit stress,
contact pressure, margins of safety for housing/bushing/ligament/edge-
distance, tolerance stack), plus two additions on top: the LaTeX-rendered
derivation view, and the reamer picker.

**Straight-bushing-only in practice means more than "skip the countersink
inputs."** Reading `solveEngine.ts` confirmed that excluding countersink/
flange geometry eliminates the "corner enumeration" worst-case tolerance-
stacking complexity entirely (`enumerateCountersinkCorners`,
`solveCountersink`, `csDiaToleranceFromBase`/`csDepthToleranceFromBase`),
which in turn collapses `calculateUniversalBearing`'s `t_eff_sequence` to
just `housing_len` (a single cylindrical segment, `eta = 1.0` - confirmed
by reading `src/lib/core/shared/bearing.ts`), so that whole shared module
didn't need porting either. This is a real scope boundary, not a
placeholder - a future flanged-bushing pass would need to port both of
those pieces, not just add UI fields.

## Crate: `bushing-solver`

Plain Rust library, zero GUI dependency (mirrors `search-core`'s own
"testable on any toolchain" property):

- `materials.rs` - the 17-entry material table (`Al7075`, `Ti-6Al-4V`,
  `Inconel 718`, bronze/steel washer stand-ins, etc.), ported verbatim.
- `tolerance.rs` - `resolveTolerance`/`makeRange`/`buildOdTolerance`/
  `containmentViolations` from `solveMath.ts`, the straight-bushing-
  relevant subset (the TS source's bore-capability/interference-policy
  auto-adjustment machinery that tries to auto-tighten an infeasible bore
  band is not ported - v1 always resolves tolerance bands as entered, and
  honestly reports `Infeasible` status when they don't overlap, rather
  than silently adjusting them).
- `lame.rs` - general, bushing-agnostic thick-wall (Lamé) pressure-vessel
  primitives: `lame_stress_at_radius` (the full two-boundary-pressure
  closed-form stress state at any radius, not a thin-wall or boundary-
  only reduction), `radial_displacement`, `diametral_interference_compliance`
  (the shrink-fit compliance term), and `sample_lame_field` (the per-radius
  stress distribution used for the cross-section stress plot). Self-
  contained - no bushing-specific types or fields - so a future non-
  bushing pressure-vessel calculation can depend on it directly.
- `solve.rs` - the bushing-specific interference-fit physics built on top
  of `lame.rs`: contact pressure, hoop stress in housing/bushing, installed
  OD, thermal interference correction, install force, and the four
  margin-of-safety candidates (housing hoop stress, bushing hoop stress,
  edge-distance sequencing, edge-distance strength) with the governing
  (lowest-margin) check selected automatically. `term_b`/`term_h` and the
  boundary hoop-stress values are derived from `lame.rs`'s general
  functions, not re-derived inline - see `lame.rs`'s own module doc for
  why that distinction matters.
- `reamers.rs` - the aircraft reamer catalog (`data/aircraft_reamer_catalog.csv`,
  48 real, sourced entries extracted verbatim from the TS source's own
  `aircraftReamerCatalogData.ts`) and `nearest(target_in, count)` for the
  reamer picker.

15 unit tests plus one differential test, all passing
(`cargo test -p bushing-solver`).

## Correctness: differential testing against the real TS engine

Rather than hand-deriving expected values, `tests/differential.rs` runs
the project's own base fixture (`engineering.toolbox/tests/bushing-
fixture.ts`'s `baseBushingInput`) through the Rust port and asserts
against golden values captured by actually executing the production
TypeScript `computeBushing` function via `npx tsx` - the same "prove it
against the real thing" discipline this repo already uses for its DOCX/
PPTX/XLSX/PDF fixtures. This caught one real porting bug before the test
was even run: a naive `.max()` chain for `Fbru_ksi || Sy_ksi || 0` (JS
falsy-fallback semantics, not a numeric max) was replaced with an explicit
`if fbru_ksi != 0.0 { .. } else { .. }` during code review.

## UI: `app/src/bushing_workbench.rs`

Four collapsible input sections (Geometry incl. reamer picker,
Interference & tolerance, Materials, Environment & install), a live
results column (summary cards, governing-check badge, conditional
fail/clearance-fit/infeasible-tolerance alert banners, four margin rows,
a detail grid, and a toggleable derivation view) - `compute()` runs on
every render since the math is cheap pure arithmetic, no debouncing
needed.

### LaTeX derivation view

Blitz/dioxus-native has no live MathML/LaTeX layout engine. Two Rust-
native candidates were evaluated and rejected: `RaTeX` (v0.0.1, 77
downloads, `NOASSERTION` license - too immature and legally unclear) and
`pulldown-latex` (MathML output only - no visual rendering without
building a full math-layout engine from scratch). Instead, the 7 core
formulas are pre-rendered to static PNGs at authoring time using
`engineering.toolbox`'s own already-installed, mature KaTeX (v0.16.44) +
Playwright, from the exact same LaTeX source strings as its
`BushingInformationPage.svelte` (`../engineering.toolbox/
render_bushing_formulas_TEMP.mjs`, a one-shot script, not part of either
repo's normal build). Two color variants per formula (light-on-dark,
dark-on-light) since a raster PNG can't respond to `currentColor` the way
the original MathML did. Output lands in `app/assets/bushing_formulas/`
and is embedded via `include_bytes!` - fully offline, no runtime
dependency on KaTeX/Playwright/Node in the shipped app.

### The `<img src="data:...">` net-provider gap this surfaced

Rendering the formula PNGs as `<img src="data:image/png;base64,...">`
exposed a real gap: this app's hand-rolled `launch::run` (see `main.rs`)
passed `net_provider: None`, which Blitz resolves to
`blitz_traits::net::DummyNetProvider` - a true no-op `fetch` (verified by
reading `blitz-traits-0.2.0/src/net.rs` directly) that silently drops
every image load, including `data:` URIs. This had never mattered before
because the app had no `<img src>` usage at all until this feature.

Fixed with `app/src/net_provider.rs`, a small standalone module wiring up
`blitz-shell`'s own `DataUriNetProvider` (behind its `data-uri` Cargo
feature) - resolves `data:` URIs locally (base64 decode, no I/O, no
network) and returns an explicit `UnsupportedScheme` error for every other
scheme. Deliberately *not* `blitz-net::Provider` (`dioxus-native`'s normal
default), which would also pull in a real HTTP client (`reqwest`) this app
has no legitimate use for. Kept in its own file specifically so it's easy
to remove/replace: delete `net_provider.rs`, drop the `mod net_provider;`
and `net_provider::data_uri_only(...)` call in `main.rs`'s `launch::run`,
and drop `blitz-shell`'s `data-uri` feature from `app/Cargo.toml`.

### Reamer picker

A dropdown over `reamers::nearest(target_in, 8)`, opened from the bore-
diameter field. Picking a row sets `bore_dia`/`bore_tol_plus`/
`bore_tol_minus` directly from the catalog entry's nominal size and tool
tolerance, then closes the picker.

### Cross-section visualizer (`app/src/bushing_visualizer.rs`)

An axial cross-section of the housing (parent material) + installed
bushing, drawn as an engineering-style section: 45-degree cross-hatching
for cut material (housing and bushing hatched at *opposite* diagonals -
the real ANSI convention for distinguishing adjacent parts in an
assembly section), a chain-dash-dot centerline, and an interference
dimension callout as text rather than an exaggerated/to-scale gap (a real
interference is thousandths of an inch - invisible at any sane drawing
scale, and there is no dedicated graphical symbol for it beyond a
tolerance/dimension callout - researched, not guessed). Oriented with the
axial direction vertical and the head end (flange/countersink face) at
the top, radius mirrored left/right around the centerline.

Ported the same "geometry struct, then SVG string-building" split
`~/Claude/Projects/profile_capabilities`'s `joint_section_view.rs` uses -
but that project renders inside a real WebView (`dioxus::desktop`/wry);
this app deliberately has none (see CLAUDE.md's "Why dioxus-native"),
so the SVG is embedded as a `data:image/svg+xml;base64,...` `<img>` src
instead of inline `<svg>` markup, the same mechanism already used for the
derivation-view formula PNGs. Confirmed as a real working path by reading
`blitz-dom-0.2.4/src/net.rs`'s image decode: it tries a raster decode
first, falls back to `usvg::parse_svg` on the same bytes - `blitz-paint`'s
SVG support (`usvg`+`anyrender_svg`) is default-on.

No new geometry math - every coordinate comes from
`bushing-solver::geometry`'s already-tested section params. The one real
bug from the first version: a flange's sharp radius step got linearly
interpolated into a taper, because the original approach sampled a few
(z, radius) breakpoints from the continuous `evaluate_bushing_outer_radius`
(correct for finding a wall-thickness *minimum*, wrong for drawing a
profile outline) and connected all of them with straight lines. Fixed by
building each profile as an explicit, per-geometry-type point list
(`outer_profile_points`/`inner_profile_points`) that knows which
transitions are real tapers (a countersink chamfer) versus steps (a
flange shoulder - drawn as two perpendicular segments, confirmed against
real drafting convention, not assumed). Also fixed: the SVG's own
`viewBox`/intrinsic size is derived directly from the real geometry at a
fixed px-per-inch scale, not fitted into an arbitrary fixed-aspect box -
the first version forced every bushing into the same landscape frame
regardless of its actual proportions, which for a short/wide bushing drew
a sliver of content in a mostly-empty canvas.

Rendered output isn't visible to this agent (no GUI screenshot capability
in this environment) - verified instead by decoding the actual generated
SVG and rasterizing it with `inkscape` for direct visual inspection (see
`bushing_visualizer.rs`'s `dump_for_visual_inspection` test), for
straight/flanged/countersink cases, before treating this as correct.

## What's deliberately not in this pass

Service-duty/wear (PV) screening, process-route review, and standards/
approval review - all explicitly excluded per the scope decision above.

**Countersink/flange geometry and tolerance auto-adjustment (added in a
later pass, ported the same way as everything above - line-for-line, with
a real countersink/flanged differential fixture proving it against the
actual TS engine, not just internal self-consistency):**

- `bushing-solver/src/countersink.rs` - `solveCountersink`,
  `enumerateCountersinkCorners`, `csDiaToleranceFromBase`/
  `csDepthToleranceFromBase` (`solveMath.ts`).
- `bushing-solver/src/bearing.rs` - `calculateUniversalBearing`
  (`shared/bearing.ts`), just the `t_eff_sequence` field this port
  actually consumes.
- `bushing-solver/src/geometry.rs` - `resolveBushingSectionParams`/
  `evaluateBushingOuterRadius`/`evaluateBushingInnerRadius`/
  `computeMinimumBushingWall` (`shared/bushingProfileGeometry.ts`,
  solver mode only - no 3D-viewer render-mode branches).
- `bushing-solver/src/tolerance.rs` - `enforceBoreBandForTarget` plus the
  `ToleranceStatus::Clamped` variant it makes reachable.
- `solve.rs`'s `BushingInputs`/`compute` wire all of the above in behind
  `BushingType`/`IdType` (default `Straight`) and
  `EnforcementPolicy::enabled` (default `false`) - every new field
  defaults through `..Default::default()` to exactly reproduce the
  original straight-bushing-only behavior, so no existing caller/test
  needed to change.
- UI: `app/src/bushing_workbench.rs` gained OD/ID geometry type chip-rows,
  conditional internal/external countersink sections (a CS-mode chip-row
  gates which of dia/depth/angle is editable vs. derived, matching
  `normalize.ts`), flange fields, a neck-wall margin row/fail banner, and
  a tolerance auto-adjust checkbox. `MaterialField` was also fixed to use
  `components.rs`'s `Dropdown` instead of a raw `<select>` (`blitz-dom`
  renders `<option>` children as flattened text, no real popup - a real
  bug, not new scope, found during this pass).
