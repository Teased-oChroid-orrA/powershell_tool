# Issue #11 Phase 13: two authored CSS bugs, not renderer gaps

Real screenshot feedback: "the right dock checks still does not scroll,
all you did was make the dock the length of the entire page... the dock
shall only enclose its content. the left menu dock does not hide, it
just minimizes an insignificant amount."

Unlike Phase 11/12, the status-rail issue this round was a real bug in my
own authored CSS, not a renderer gap - found by re-reading the Phase 12
diff line by line instead of assuming the mechanism worked.

## Root causes

1. **`.bushing-page { flex: 1; min-height: 0 }` was a silent no-op.**
   `flex` properties only mean anything on a flex *item*. `.stage`
   (`.bushing-page`'s parent) is not `display:flex` - just
   `overflow:auto` + padding. So `.bushing-page` never actually got a
   bounded height, `.bushing-workspace`'s `overflow-y:auto` never had
   anything to overflow within, and `.stage` stayed the real (only)
   scroller exactly as before Phase 12 - explaining "still does not
   scroll." Fixed: `height: 100%` instead of `flex: 1`. `.stage` does
   have a real, definite height of its own (from its own flex sizing
   inside `.main`), so a percentage height resolves against it fine
   regardless of `.stage`'s own display type.
2. **`align-items: stretch` was applied to both split children.**
   Because `.bushing-page` never actually had a bounded height (bug 1),
   `.bushing-workspace-split`'s own height was whatever the *unbounded*
   content grew to - and `align-items:stretch` made
   `.bushing-status-rail` match that, i.e. the entire page. Even after
   fixing bug 1, stretch was still wrong for the rail specifically - it
   should hug its own short content, not fill the workspace's full
   scrollable height. Fixed: `.bushing-status-rail` gets `align-self:
   flex-start`, overriding the split's own (still-needed, for
   `.bushing-workspace`) `align-items: stretch`.

## Left rail: stripped the animation to isolate the variable

"Minimizes an insignificant amount" - the Phase 12 `width` toggle
(12px collapsed / 232px on `:hover`) is built from primitives already
proven elsewhere in this file (`:hover`, explicit `width`,
`position:absolute`), but the `transition: width` on top of it was the
one genuinely untested part (every other transition in this file
animates a paint-only property, never a layout-affecting one). Removed
the transition - now an instant, unanimated toggle - to find out whether
the animation itself is what's misbehaving before layering anything else
on top of an already-uncertain mechanism. If it still doesn't collapse
correctly, the bug is in `:hover` matching or explicit-width layout for
a `position:absolute` element, not the transition - documented as the
next thing to isolate in the CSS comment.

## Verification

- `cargo build -p app`: clean, zero new warnings.
- `cargo test -p app`: 25/25 unchanged.
- Full diff reviewed before committing.
- Still needs a real screenshot - no local GUI capability in this
  environment. The rail fix in particular is a narrowing, not a
  confirmed fix; report back what it actually does at rest and while
  hovering.
