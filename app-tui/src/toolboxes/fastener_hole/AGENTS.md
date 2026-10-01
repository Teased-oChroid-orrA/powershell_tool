# toolboxes/fastener_hole/ — Fastener Holes toolbox

> TL;DR: Regular/countersunk fastener-hole design tool, unique to this crate. Owns its own pure engineering `domain/` (zero ratatui/crossterm), deliberately not built on `bushing-solver::tolerance`.

## Purpose
Owns: `domain/` (tolerance normalization, regular-hole fit envelope, countersink three-of-four solver, lateral surface area, secondary derivations), `model.rs`, `mod.rs` state/key routing, `view.rs`.
Does not own: any shared solver crate - the engineering math here is wholly original to this crate.

## Code Map
| Looking for... | Go to |
|---|---|
| pure engineering domain (tolerance normalization, regular-hole fit envelope, countersink three-of-four solver, lateral surface area, secondary derivation methods) - zero `ratatui`/`crossterm` | `src/toolboxes/fastener_hole/domain/` |
| UI-facing state bridging domain results (`FastenerHoleModel`, `field_rows`, number editing) | `src/toolboxes/fastener_hole/model.rs` |
| `FastenerHoleState`, toolbox-local key routing | `src/toolboxes/fastener_hole/mod.rs` |
| rendering (field list + calculated/secondary readout, wide/narrow layout) | `src/toolboxes/fastener_hole/view.rs` |

## Entry Points
| Task | Start Here |
|---|---|
| Change a Fastener Holes formula (fit envelope, countersink solver, lateral area, secondary derivation) | `src/toolboxes/fastener_hole/domain/` - each concern has its own module (`regular_hole.rs`/`countersink.rs`/`surface_area.rs`/`tolerance.rs`); extend that module's own test suite (brute-force/round-trip proofs, not just a spot check) |
| Add/change a Fastener Holes editable field | `src/toolboxes/fastener_hole/model.rs` (`NumberTarget`/`field_rows`) - only INPUT dimensions belong in `field_rows`, never a CALCULATED/TRANSFERRED/PRESERVED one |

## Contracts
- `domain/` has zero `ratatui`/`crossterm` imports; only INPUT dimensions belong in `field_rows`, never CALCULATED/TRANSFERRED/PRESERVED values.
- Each engineering concern has its own `domain/` module with brute-force/round-trip tests; extend those, not spot checks.

## Pitfalls
- **Fastener Holes deliberately has no `d`-toggled Numbers panel**, unlike Bushing/Pressure Vessel Analyzer/Preload Analysis. Investigated and confirmed: this toolbox's readout panel (`view.rs::regular_readout_lines`/`countersink_readout_lines`) already shows full nominal+min/max detail for every calculated/derived value inline, unconditionally - there is no extra hidden detail comparable to Bushing's per-radius stress field to put behind a toggle. Adding one here would be a no-op UI element with nothing behind it, not real parity. If a future change adds genuinely new derived detail to this toolbox that's too verbose for the always-visible readout, that's when a Numbers-panel-style toggle would earn its keep - not before.
- `fastener_hole/view.rs::draw()` originally split the toolbox into a fixed `Constraint::Percentage(45)`/`Percentage(55)` two-pane layout once `inner.width >= 84`, while the fields pane's per-row `label_width` was computed from the *widest label across all rows* with no corresponding awareness of how wide the value text plus that label actually needed to be. Confirmed via a real `tmux` session at deliberately narrow widths (not just `TestBackend` unit tests, which never happened to render at 84-98 inner columns - the exact failure zone): at those widths the 45% column was reliably too narrow for rows like `"Secondary Hole Diameter Nominal  0.3125 in"`, and ratatui's `List` silently truncates rather than wrapping or erroring, so values were cut off right at the pane's own border with no visual indication anything was missing. Fixed by computing the fields pane's *required* content width directly (`fields_required_width`: marker + widest label + widest value + border) and only using a horizontal split when that plus a minimum readout width (`MIN_READOUT_WIDTH`) actually fits (`Constraint::Length`/`Constraint::Min` instead of two `Percentage`s) - otherwise falling back to the full-width vertical stack. If a future toolbox renders a variable-width label/value list next to another pane, size the list's column from its own worst-case content width rather than a fixed percentage/breakpoint, and add a regression test at the specific width range in between the old breakpoint and the point where content actually fits (a `TestBackend` sweep across candidate widths with the row *selected* - `scroll_list::render`'s `ListState::select` only scrolls the selected row into view, so a truncation test must select the row it's checking, not assume every row is simultaneously on-screen).

## Public API
Crate-internal. `mod.rs`: `FastenerHoleState`, `handle_key(&mut FastenerHoleState, KeyEvent) -> (bool, Vec<Effect>)`, `PANE_MAIN`/`PANE_COUNT`. `model.rs`: `FastenerHoleModel`, `FieldRow`, `NumberTarget`/`NumberPart`, `field_rows(&model)`, `field_hint`. `view.rs`: rendering. `domain/`: pure engineering functions, zero UI imports.

## Design Rationale
- Own tolerance types in `domain/` rather than `bushing-solver::tolerance`: see that module's doc comment for why.
- No `d` Numbers panel: the readout already shows full nominal and min/max detail inline (see Pitfalls).
- Fields pane is sized from its own required content width, not a fixed percentage split (see Pitfalls).

## Patterns
### Adding an editable field
1. Add a `NumberTarget` variant (or a cycling/toggle `FieldRow`) in `model.rs`.
2. Add its `FieldRow` to `field_rows`, plus a `row_label` and `field_hint` arm (the hint feeds the bottom Hint panel).
3. Read/write it in the model's value getter/setter and feed it into the solver input.
4. Render nothing extra: `view.rs` draws from `field_rows`. Add a validation or readout line only if the result needs one.
Fields here are toleranced: `NumberPart` selects nominal/plus/minus. Only input dimensions go in `field_rows`.

## Boundaries
### Always
- Extend the matching `domain/` module's brute-force/round-trip tests with any formula change.
- Render the readout through `widgets/scroll_paragraph.rs`.
### Never
- Add `ratatui`/`crossterm` imports under `domain/`.
- Put calculated values in `field_rows`.

## Navigation
Parent: `app-tui/AGENTS.md` (crate-wide contracts: `Effect` reducer rule, `Number`-row editing, Caps-Lock key patterns, scroll widgets, disk-space `-p app-tui` scoping). 
