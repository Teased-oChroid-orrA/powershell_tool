# Issue #11 Phase 11: mockup-fidelity fixes + auto-hiding rail

Direct response to real screenshot feedback ("14 and 15 are the current
implementation... no where like the artifact... PERFORM AN EVALUATION WHY,
USE THE ORCHESTRATOR AND CLAUDE.MD"). Root-caused against the approved
mockup's actual CSS and this file's own real, current CSS - not guessed.

## Root causes found

1. **Gauge label overfill.** `.value-track .allow-tag`/`.end-label`
   (`components.rs`'s `CheckGauge`) are `position:absolute` spans with
   `white-space:nowrap` and no explicit width. Two independently-styled
   spans in the real screenshots ("limit"/"70000", "13000"/"psi") both
   wrapped onto two lines despite `nowrap` - evidence that auto-width
   sizing for absolutely-positioned inline elements isn't reliable on
   this renderer. New, previously-undocumented gap in the same bug class
   as the already-known `onchange`/`<details>` gaps.
2. **Status rail not scrolling with the page.** The approved mockup used
   CSS Grid + `position:sticky` for `.status-rail`; the shipped
   `.bushing-status-rail` never got either. Since sticky was *dropped*,
   not added, "doesn't scroll with the page" can't be sticky actually
   working - points instead to the fixed-sidebar-beside-variable-height-
   column shape itself, which this file already has one documented
   precedent for going wrong (the lightbox `overflow-y:auto` fix, this
   file's own comment at the lightbox backdrop rule).
3. **Results headline stacking like a table.** `.bushing-headline`/
   `.bushing-mini-stats` CSS matches the mockup's own almost verbatim
   (flex + `flex-wrap:wrap`) - not a missing-implementation bug. Real
   cause: the Governing mini-stat renders
   `"{governing_result.name} \u{b7} {fmt_margin(...)}"` inside an
   `.ms-pill` (a badge class with no width constraint, meant for short
   text) - real content the mockup's static placeholder never exercised
   at this length, wide enough to force the row past available width and
   wrap.
4. **New feature** (not a bug): auto-hiding left rail, hover-reveal at the
   left edge, to maximize screen real estate.

## Fixes

- `CheckGauge`'s `allow-tag`/`end-label`: added `width: max-content` -
  forces correct shrink-to-fit sizing regardless of the auto-width bug,
  instead of trusting `white-space:nowrap` alone.
- `.check-item` padding restored to the mockup's approved `14px 2px`
  (had drifted to `var(--space-3) var(--space-1)` = 12px/4px).
- `.bushing-status-rail`: removed an unnecessary `overflow:hidden` (no
  content needs clipping there) - the low-risk fix in the direction this
  file's own overflow-quirk precedent points, rather than reaching for
  `position:sticky`, which has zero prior verification on this renderer.
- `.bushing-headline`: `flex-wrap` changed from `wrap` to `nowrap` +
  `overflow:hidden`; `.bushing-mini-stat .ms-pill` gained
  `overflow:hidden; text-overflow:ellipsis; white-space:nowrap;
  max-width:100%` - guarantees a single row regardless of how long the
  governing-mode text is, truncating gracefully instead of wrapping.
- `.rail` (Toolbench nav sidebar, `main.rs`'s `App()` shell - applies
  app-wide, not just this tool): taken out of `.shell`'s flex flow via
  `position:absolute`, default `transform:translateX(-220px)` leaves a
  ~12px hover strip at the left edge, `.rail:hover { transform:
  translateX(0) }` reveals it as an overlay. `.main` no longer reserves
  232px, reclaiming that width when the rail is hidden.

  Verified `:hover` itself is safe to rely on before using it -
  `.nav-item`/`.add-tool-btn`/`.theme-toggle` already depend on it
  elsewhere in this exact file. Deliberately did NOT use
  `onmouseenter`/`onmouseleave` Dioxus event handlers for this - checked
  `blitz-traits-0.2.0`'s `DomEventData` enum directly
  (`~/.cargo/registry/src/.../blitz-traits-0.2.0/src/events.rs`): only
  `MouseMove`/`MouseDown`/`MouseUp`/`Click`/`Key*`/`Input`/`Ime` exist, no
  Enter/Leave/Over/Out variant at all. A JS-event-driven show/hide would
  have silently never fired, the same failure shape as the `onchange`
  bug. Pure CSS `:hover` sidesteps that gap entirely.

## Verification

- `cargo build -p app`: clean, zero warnings.
- `cargo test -p app`: 25/25 unchanged - CSS/layout only, no logic touched.
- Full diff reviewed before committing (per standing "work with diff"
  instruction).
- Still not independently verified end-to-end against a live render - no
  local GUI capability in this environment, same standing limitation as
  every UI phase this session. Next real screenshot round is the actual
  verification for this phase's hypotheses (particularly the
  `width:max-content` fix and the rail-scroll fix, both inferred from
  code-level evidence and this file's own documented precedents rather
  than a live repro).
