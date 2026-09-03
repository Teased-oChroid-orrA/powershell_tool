# Issue #11 Phase 12: bug hunt - pinned status rail, working auto-hide rail

Follow-up to Phase 11: two of that phase's fixes turned out to rest on
wrong assumptions, caught by re-testing against the user's exact wording
("the checks... shall stay in the same position as I scroll, always
visible" / "the left menu is not hiding") plus direct inspection of
`blitz-dom-0.2.4` source - not guessed a second time.

## Root causes (source-verified)

1. **`position: sticky` is parsed but never implemented.**
   `blitz-dom-0.2.4/src/layout/damage.rs:368` and `src/node/node.rs:190`
   both bucket `Position::Sticky` with `Static`/`Relative` for paint/
   z-ordering only - no offset-on-scroll logic exists anywhere in the
   crate. A sticky element behaves exactly like a static one.
2. **`transform` is invisible to hit-testing.** `Node::hit()`
   (`src/node/node.rs:716`) computes hit bounds purely from
   `final_layout.location`/`final_layout.size` - transform never enters
   that math. Phase 11's `.rail:hover { transform: translateX(0) }`
   never actually stayed collapsed: the hit-test box stayed permanently
   at its untransformed, full-size position, so hovering anywhere over
   where the rail *would* sit if open kept re-triggering `:hover`.

Both added to `CLAUDE.md`'s renderer-quirk list with source citations.

## Fixes

- **Status rail pinned while scrolling.** `.bushing-page` gains
  `flex:1; min-height:0; overflow:hidden` (was previously unbounded on
  purpose - see the updated comment). `.bushing-workspace-split` gains
  `align-items:stretch; flex:1; min-height:0`. `.bushing-workspace`
  gains `min-height:0; overflow-y:auto` and becomes the one scrolling
  pane. `.bushing-status-rail` gains `max-height:100%; overflow-y:auto`
  as a safety net - in the normal case its content just fits the full
  split height and never scrolls, staying visually fixed while the
  workspace scrolls beside it. Reuses the same shape already proven by
  the Search tool's `.main-grid{overflow:hidden}` +
  `.settings-column`/`.results-column{overflow-y:auto}`. Shared classes
  between `bushing_workbench.rs` and `pressure_vessel_workbench.rs`, so
  this fixes `DesignStatusRail` too, not just `PvStatusRail` - updated
  bushing_workbench.rs's own now-stale "single scrollbar" comment to
  match.
- **Auto-hide rail that actually works.** `.rail` now animates real
  `width` (12px collapsed -> 232px on `:hover`) instead of `transform`,
  so the hit-test box - driven by actual layout - shrinks and grows with
  what's visually shown. First animated layout-affecting property in
  this file (existing transitions are all paint-only); flagged in the
  CSS comment as reasoned from `stylo.rs`'s generic transition handling
  rather than a proven precedent, pending the next screenshot.
- **Optional improvement (requested, included): status-rail "jump to
  check" now highlights the specific row**, not just switches to the
  Results step. `PvStatusRail`'s `on_jump` now carries the clicked
  check's name; a `highlighted_check: Signal<Option<&'static str>>`
  drives a `.check-row-highlight` wrapper class around the matching
  `CheckGauge`. Deliberately does NOT scroll to it: `dioxus-native`/
  Blitz has no JS engine at all (unlike `dioxus-desktop`'s WebView), so
  `scrollIntoView` isn't available on this renderer - highlighting is
  the reachable equivalent, documented as such in the code.

## Bug hunt (no other issues found worth fixing)

Full read-through of `pressure_vessel_workbench.rs`. Grepped `main.rs`
for other `transform:` usage tied to interactivity - none (the only
other hit, `details[open] summary::before { transform: rotate(90deg) }`,
is a purely decorative chevron whose click target doesn't depend on the
rotation). `.detail-field-value` in the fabrication-summary grid has no
overflow handling for long text, but CSS Grid wraps it onto a second
line inside its own cell instead of overlapping neighbors - degrades
safely, left alone.

## Verification

- `cargo build -p app`: clean, zero new warnings.
- `cargo test -p app`: 25/25 unchanged.
- Full diff reviewed before committing.
- Still not independently verified end-to-end - no local GUI capability
  in this environment. The `width`-transition mechanism in particular is
  the one part of this round not backed by an existing proven precedent
  in this file; next screenshot round is the real check.
