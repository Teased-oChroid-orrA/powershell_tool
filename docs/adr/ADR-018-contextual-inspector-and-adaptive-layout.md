# ADR-018: A contextual inspector instead of a permanent dock, and a layout that adapts to its user

Status: Accepted (user direction 2026-10-11 after reviewing the egui chrome: no permanent "Properties" dock, nothing
repeated that the tree or a right click already offers, let ML arrange and adapt the interface as the user works)

## Decisions

1. **No permanent properties dock.** The inspector is a stack of floating **cards** (`authoring::Card`, `Relevance::{Primary,
   Context, Info}`, optional `steps`) at the right edge of the model. The editor says what is relevant
   (`Authoring::inspector`); the window shows only what applies to the current selection and step, and the model's viewport
   shrinks by the cards' width while they show (the model is never drawn under them). Nothing to show, or the person closed
   it for this selection: no inspector at all. Pin keeps it open; Window > Inspector turns the feature off.
2. **Show what the selection owns; do not repeat what other places already say or do.** The solid inspector lists the value
   fields (number, component, text rows) of the focused feature only. Every choice and action row (template, analysis, a
   support's kind and edge, add, remove, element type, material search) is the right-click menu of that feature's **tree row**
   (`Authoring::tree_menu`; history rows offer edit / suppress / roll back / delete). A right click selects the row and opens
   the menu next frame, from the selected state. Removed as duplicates: the mesh and file lines (tree, title), the colour key,
   the selected-surface list (the tree highlights them), the solve timer (status bar), the result and field label (tree), the
   member editor's file, counts and colour key. Cards that belong to a step carry `steps` (mesh density with Mesh, results
   with Simulation) and selecting a tree row brings its step's tools (`Authoring::focus_step`).
3. **A learned layer arranges the chrome** (`editor/adapt.rs`). Smoothed acceptance rate `(used + 1) / (offered + 2)` per
   (context, item), decayed 0.85 per session, kept in `<app data>/fea-editor/layout/layout.json`. It decides: the order of
   tools inside each tool-pill group (a long group keeps its most-used in view, the rest under "More"), the order of cards
   inside a relevance level, which cards start folded (offered 8 times, never used), whether the tree starts open or as a chip
   (rail) after 3 sessions of not using it, and whether a finished step moves on to the step this person usually takes next
   (share >= 0.6 of >= 4 transitions) or only rings it. Until an item has 5 observations the fixed order stands.
4. **This tier is deliberately not gated like the engineering rankers.** `rank.rs` / `advisory.rs` propose engineering choices,
   so a learned order needs a measured held-out report first. The layout only arranges the chrome: it changes no model and no
   result, nothing it demotes is unreachable (menus and the palette list everything), every decision starts from the fixed
   rules, it stores only interface labels and counts (no values, names or paths), and Window > Adaptive layout turns it off
   (`FEA_UI_ADAPT=off` too), "What has the layout learned?" says what it knows, "Reset learned layout" forgets it.
5. Rejected: a neural or opaque model for layout (the data is a handful of counts per context; a smoothed rate is explainable
   and tests exactly), gating layout behind the 20-session report (it would never adapt in practice), hiding capabilities
   instead of reordering them.

## Consequences

- `Authoring` gains `inspector`, `tree_menu`, `focus_step`, `solving_for`; `properties()` stays as the flat list of the cards.
- Tests that asserted "every row is in the dock" now assert scoping and the tree menu (including that every tree row maps to the
  feature it focuses).
- The adaptive layout is verified deterministically; whether it feels right over weeks of real use is not something a test
  can show, which is why it can be switched off, inspected and reset.
