# gpu_viewer/editor/line_run/ - the Line Run workbench in the native window

> TL;DR: `LineRunSession` is an `Authoring` over `toolboxes::line_run`'s `LineRunModel`: inspector cards for tube / material / supports / point loads / load cases, result cards (verdict, margins with bars, moment and axial-force station plots, reactions), and the line drawn along the vertical axis coloured by bending moment. No action or stress is computed here.

## Purpose
Owns: the window front end (cards, tree, ribbon, menus, typed edits, undo, case file, Export) and `drawing.rs`.
Does not own: the solve or any formula (`pressure-vessel-solver::line_run`, reached through `LineRunModel::recompute`), inputs / units / report text (`toolboxes/line_run`), the window drawing (`gpu_viewer/gui`).

## Code Map
| Looking for... | Go to |
|---|---|
| Session, `LineRunAct`, `Authoring` impl, prompts, undo / save / export | `mod.rs` |
| Cards, tree, ribbon, menus, status verdict | `cards.rs` |
| The picture (profile along y, collars at supports, moment / axial fields) | `drawing.rs` |
| The saved case (JSON of `Inputs`) | `file.rs` |
| Session tests | `tests.rs` |

## Contracts
- **Window only; dashboard tile "Line Run"** (`--gpu-line-run [file]`, default `line-run-case.json`). It deliberately does not live in the Pressure Vessel window (owner decision 2026-11-15).
- The approved mockup's layout maps onto the shared inspector: left rail cards = Tube / Material / Supports / Point loads / Load cases (case selector rows, selected one marked), result cards = Result verdict + residual, Margins (a `Stat` per criterion and a `Bars` card, the governing one tagged), Stations (two `Plot`s: moment resultant, dashed axial force; support and governing marks), Reactions. Deviations from the mockup, by design of the shared chrome: moment and axial force are two plots (one axis cannot carry two units; `Plot` has no fill), case pills are rows.
- Export writes `<case>.report.txt` and, when there is a result, `<case>.stations.csv`, both in the shown units. Units are an input (saved, undoable); they change text only.
- Selecting a case redraws but is not an edit (no undo step, not dirty).
- The tube is drawn enlarged (`drawing::enlargement`, at least 1/25 of the line length as diameter) for the picture only; the Drawing card says so.

## Pitfalls
- Case-file tests use a temp dir per test; never `open` the default path in a test.
- `Field` indices go stale when a row is removed after a card was drawn: `LineRunModel::set` / toggles ignore a missing row (tested), keep it that way.
