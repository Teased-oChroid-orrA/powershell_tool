# toolboxes/bushing/ — Bushing Workbench toolbox

> TL;DR: Thin bridge over `bushing-solver`/`mechanics-core`; `model.rs` computes nothing itself. Adds material picker, reamer catalog picker (primary way to set Bore Diameter), friction picker, Bushing ID library, fit-type selector. Libraries use `src/library.rs`.

## Purpose
Owns: UI state/key routing/rendering, pickers, and library wiring (reamer, material, Bushing ID) for the bushing solver.
Does not own: any Lamé/press-fit/tolerance math (`bushing-solver/AGENTS.md`).

## Code Map
| Looking for... | Go to |
|---|---|
| pure UI-facing state bridging `bushing-solver` (geometry/tolerance/countersink/enforcement/assembly-thermal inputs, full `BushingOutput` including the per-radius stress field) | `src/toolboxes/bushing/model.rs` |
| `BushingState`, toolbox-local key routing (`d` numbers panel, `c` edge-distance cross-check, `e` export) | `src/toolboxes/bushing/mod.rs` |
| adapter from the bushing model to the `edge-check` crate + the Results "Edge-Distance Cross-Check" section (key `c`, stale-marked when inputs change; included in the export) | `src/toolboxes/bushing/edge_check.rs` |
| rendering (field list + results readout, wide/narrow layout, plain-text report builder) | `src/toolboxes/bushing/view.rs` |
| housing/bushing material picker | `src/toolboxes/bushing/material_picker.rs` |
| full filterable aircraft reamer catalog (primary way to set Bore Diameter - `m` for manual numeric entry) | `src/toolboxes/bushing/reamer_picker.rs` |

## Entry Points
| Task | Start Here |
|---|---|
| Change a Bushing Workbench formula (Lamé/press-fit, tolerance-stack, countersink, bearing/edge-distance, reamer catalog) | `bushing-solver/` (NOT this crate — `toolboxes/bushing/model.rs` only bridges that crate's existing public API into UI state; see `bushing-solver/AGENTS.md`'s own differential-test discipline before changing a formula there) |
| Add/change a Bushing Workbench editable field | `src/toolboxes/bushing/model.rs` (`NumberTarget`/`field_rows`) - a dynamic row list gated by `BushingType`/`IdType`/`CsMode`/enforcement/assembly-thermal state, same pattern as Fastener Holes' `field_rows` |

## Contracts
- Picker-backed `Number` rows (Bore Diameter, Friction, Bushing ID: `Enter` opens a catalog/library) must be excluded from type-to-start-editing in `bushing::handle_key`; see parent Contracts. Add any new picker-backed row to that same exclusion list.
- Material inputs reach the solver as resolved `Material` values (see `bushing-solver/AGENTS.md`); resolve built-in vs library/custom materials in `model.rs` before solving.

## Edge-distance cross-check (`edge_check.rs`, `c` / `C`, `Edge check` / `+Plastic` buttons)
- Runs the independent `edge-check` models (on a worker thread via `Effect::RunEdgeCheck`; `BushingState::auto_edge_check`, called each `Tick`, re-runs the last-run set 400 ms after any input the check depends on - the bushing/friction included - stops changing, silently for a quick run, and cancels a run for older inputs; `edge_check::is_stale` compares the full signature) for the live inputs (~0.2 s with `c`; `C` also runs the bushing + housing contact FE, ~3 s wall on 8 cores; the UI stays responsive with a persistent running tooltip, then a completion tooltip; the crate is built optimised even in dev) and shows margins, the smallest edge distance that satisfies each check, the effect of the fit, and Monte-Carlo `P(fail)`. It never feeds back into `bushing_solver::compute`: the existing margins/governing check are unchanged. The result is stored with the exact `EdgeInput` it ran for; the section is marked stale as soon as `build_input` differs. The FE plate is sized `max(3 e, 10 a)`; applied load is `model.load`, thickness is `output.t_eff_seq`.
- Hover (`MouseEventKind::Moved` -> `BushingState::edge_hover`) or click-pin (`BushingAction::EdgeInfo`, Esc closes) a model name / the legacy heading / an advisory to see its `edge_check::tip` (what it does, strengths, weaknesses, restrictions). `edge_check::advisories` adds warn-only lines to the check list from non-stress models; they never change PASS/REVIEW or the governing check and vanish when the run is stale.
- Results show a compact table (one row per check: Superposition, Allowables, Contact FE (the plane-stress elastic FE and the dead-load plastic model are no longer rows; the plastic one is the fallback when the bushing is invalid), and the solver's Legacy row; columns = minimum e/D for Strength / Bearing / 1st yield, plus P(fail) of the Bearing target); a "recommended" block (P90/P95/P99 e/D per model, in an applied-load group and a bearing-limit group, plus the single recommendation: the larger of the two over the models at P99, naming the governing criterion, red when the actual e/D is short) and a second table ("capacity lbf") gives the pin load each check can carry at the actual e/D against the load it must carry, plus the share of Strength capacity the fit uses; a cell is red only when that check needs more edge distance than provided. Margins, governing mode, fit effect and notes live in the row's tooltip ("This run" block, `edge_check::detail_lines`). `edge_check::advisories` yields at most ONE warn line for the check list.
- Material pickers (Bushing and Pressure Vessel) are type-to-search: printable keys go to the always-visible search box (multi-word AND over name, family, spec, table - `widgets/material_detail.rs`); actions are Ctrl+N add, Ctrl+L import, Ctrl+E export (Bushing); PgUp/PgDn/Home/End move through the ~1,400 entries.
- Long field values (material names) wrap onto up to three indented rows under their label instead of widening the Inputs pane (`widgets/scroll_list::field_item`, pane width capped via `VALUE_CAP`); click regions come from `mouse::list_row_regions_var` because those rows are taller than one line.
- Remove the feature: see `docs/edge-distance-crosscheck.md` ("Removing it").

## Pass/fail checks and recommendations (`advice.rs`)
- `BushingModel::recompute` also fills `checks` (Pass/Warn/Fail per `CheckKind`) and `recommendations`; the Results pane highlights a failing check's name+value (red, `✗`) and the related input rows. Recommendations are **not** listed inline: Results has a fixed action bar (`Fixes (N)`, `Why OD clamped?`, `Numbers`, `Export` - all mouse buttons) and flagged lines end in `▸` (click to open that check's fixes). `f`/`a` opens the modal Fixes window (`BushingState::advice`; tabs Fixes / Explain, `w` opens Explain); inside it Up/Down select, Enter/`a`/Apply applies the selected fix to the real input fields and jumps the cursor to the first changed one, Esc/Close dismisses. Mouse and keys share `BushingState::perform(BushingAction)`; click regions are published as `MouseRegions::bushing_actions` (window-only while it is open; `scroll_paragraph::render_interactive` maps wrapped/scrolled lines to rects). `advice::explain_tolerance` builds the OD-clamped/infeasible explanation from the live numbers (heading/body pairs).
- Every recommendation is **verified by trial-solving a clone** (`BushingModel::trial`): bisect from the current value toward a limit, then require the failing check to clear (preferring no warning) and *no other check to newly fail*. The advice search calls `recompute_output`, never `recompute` (it would recurse). Do not hand-derive fixes here - the solver stays the authority.
- Changing Fit Type loads `fit_type_preset()` (interference ~0.003xD press / 0.005xD shrink / negative for clearance and slip; tolerance band never narrower than the bore band). Leaving Shrink only turns Install Thermal Assist off if Shrink turned it on.

## Tolerances, catalog snapping, alternatives (added later)
- Toleranced dimensions are ONE compact row (`FieldRow::Tol(TolGroup)`) instead of separate `+tol`/`-tol` rows; the global `Tolerance Entry` row (`ToleranceMode`) switches between `+plus / -minus` and `min .. max` for all of them (like Fastener Holes). The model still stores nominal + plus + minus; `commit_tolerance_text` parses (one value = symmetric, signs pick the side, `..`/`/`/space separate; Min/Max keeps the nominal when it lies inside). The Bushing ID has **no** tolerance band by design.
- Recommendations that change a drilled/reamed size (`IdBushing`, and `BoreDia` only ever *smaller*) are snapped to the nearest real catalog size on the passing side (`BushingModel::catalog_sizes` = built-in aircraft reamers + the user's reamer library, synced via `sync_user_reamers`); a bore change also takes the reamer's tolerance and widens the interference band to cover it. If no catalog size passes, the fix says it needs custom tooling. Every recommendation carries `impact` (bore line first, then metrics that moved, then newly-warning checks), computed from the trial solve; several alternatives per failure are offered, including ones that leave the bore alone.
- Results show Δ interference from service temperature and from install thermal assist (solver `delta_thermal`, `assembly_thermal_delta`, `install_delta`).

## Drill bit catalog
- `bushing_solver::drills` embeds the AFT Fasteners drill chart (`bushing-solver/data/drill_bit_catalog.csv`: fractional, #1-#107, A-Z - inches only, metric rows removed; two source typos corrected, see its module doc) with RapidDirect's "most common" sizes flagged. The Bushing ID picker lists the user's saved sizes first, then every drill (`[common]` tag like the reamer list's `[preferred]`; filter by label or kind (`letter`/`common`...); **typing digits/`.` filters live by decimal inches** in both the Bushing ID and reamer pickers (`size_filter.rs`: `0.26` lists 0.2600, 0.2624, 0.2652...)). Fix recommendations may snap an ID to a drill; a **bore** never snaps to a drill (it is reamed) - `advice::bisect_fix` filters `source == "drill"` for `BoreDia`.

## Pitfalls
- `field_rows` shows the basic fit/housing/materials/load rows; tolerances, installation limits, OD/ID geometry, enforcement and thermal assist sit behind `FieldRow::AdvancedSection` (`BushingModel::advanced_open`, collapsed by default). Tests needing those rows set `advanced_open = true`.
- Reamer catalog, material library, and Bushing ID library are `src/library.rs` consumers; conflict/duplicate handling lives there, not here.
- The import-conflict prompt (`k`/`o`/`z`/`a` keys and its dialog) is shared in `conflict_prompt.rs`; a picker supplies only its title and two description lines. Popup geometry is `widgets/popup.rs::centered_rect`, shared app-wide - do not re-add a local copy.
- `m` inside the reamer picker reaches manual numeric entry; Bore Diameter's `Enter` opens the catalog (a real bore is reamed to a real size).

## Known gaps (deliberately not done)
- **Edge-check strength CV (5%) and model-error CVs are fixed assumptions**, shown in the tooltips/report but not editable. They are not solver inputs, so they must NOT become a `NumberTarget` (that enum is 1:1 with `BushingInputs`, and `advice.rs` trials rely on it); expose them as edge-check state with their own control in the Results pane.
- **`FitType` presets are generic ratios of bore diameter, not ISO/ANSI fit classes** (H7/p6, FN, ...). Mapping to a standard needs a product decision on which standard.

## Public API
Crate-internal. `mod.rs`: `BushingState`, `handle_key(&mut BushingState, KeyEvent) -> (bool, Vec<Effect>)`, `PANE_MAIN`/`PANE_COUNT`. `model.rs`: `FieldRow`, `NumberTarget`, `field_rows`, `row_label`, `field_hint`, and `cycle_*`/`label_*` pairs for each enum row (bushing type, ID type, end constraint, countersink mode, `FitType`). Pickers: `material_picker`, `reamer_picker`, `friction_picker`, `bushing_id_picker`, each with a `*_persistence` module.

## Design Rationale
- Bore Diameter opens the reamer catalog instead of a free-typed number: a real installed bore is reamed to a real tool size. This replaced a "type, then open Nearest Reamer" flow.
- Reamer, material, and Bushing ID catalogs share `src/library.rs` rather than three import/export implementations.
- `model.rs` stays a bridge: all Lamé, tolerance, and countersink math is in `bushing-solver`/`mechanics-core`.

## Patterns
### Adding an editable field
1. Add a `NumberTarget` variant (or a cycling/toggle `FieldRow`) in `model.rs`.
2. Add its `FieldRow` to `field_rows`, plus a `row_label` and `field_hint` arm (the hint feeds the bottom Hint panel).
3. Read/write it in the model's value getter/setter and feed it into the solver input.
4. Render nothing extra: `view.rs` draws from `field_rows`. Add a validation or readout line only if the result needs one.
5. Enum rows (`FitType`, bushing/ID type, ...) need a `cycle_*` and `label_*` pair; the row list is dynamic, gated by type, countersink mode, enforcement, and thermal state.
6. A picker-backed number row must be excluded from type-to-edit in `handle_key`.

## Boundaries
### Always
- Change formulas in `bushing-solver` (follow its differential-test discipline), never here.
- Route new catalogs through `src/library.rs`.
### Never
- Add a second material or Lamé path to this toolbox.

## Navigation
Parent: `app-tui/AGENTS.md` (crate-wide contracts: `Effect` reducer rule, `Number`-row editing, Caps-Lock key patterns, scroll widgets, disk-space `-p app-tui` scoping). Solver node: `bushing-solver/AGENTS.md`.
