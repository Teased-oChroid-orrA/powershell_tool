# gpu_viewer/editor/eccentric/ - the Eccentric Bushing workbench in the native window

> TL;DR: `EccentricSession` is an `Authoring` over the terminal toolbox's own `EccentricState` (inputs, runs, queue, history, report; `eccentric-bushing` solver). The bushing inputs are read from a Bushing case file, never entered twice. Before an analysis the picture is a derived FEA `Problem`; after it, the solver's own solved model (housing, bushing and pin) read back from its `.vtu` text.

## Purpose
Owns: the window front end (inspector cards for Inputs and Analysis, tree, ribbon, menus, undo, the case file, exports beside the case) and the two derived pictures: `fe.rs` (inputs -> `Problem`) and `vtu.rs` (the solver's `.vtu` -> `SceneSource`).
Does not own: any mechanics (`eccentric-bushing/AGENTS.md`; the crate is unchanged), the Bushing inputs (`gpu_viewer/editor/bushing`, `toolboxes/bushing`), the window or the cards' drawing.

## Code Map
| Looking for... | Go to |
|---|---|
| Session, `EccentricAct`, `Authoring` impl, runs on worker threads, exports | `mod.rs` |
| Cards, tree, ribbon, menus | `cards.rs` |
| Inputs -> picture before an analysis (boss or plate, offset bore, bushing, pin load arrow) | `fe.rs` |
| The solver's `.vtu` text -> mesh + von Mises, displacement magnitude, stress components | `vtu.rs` |
| The case (`EccentricFile`: own inputs + path of the Bushing case) | `file.rs` |

## Contracts
- **Analyse is the toolbox's `Task::Fields`**: one solve returns the `Analysis` and the fields (`analyze_fields` runs the same `analyze_core` as `analyze_with`), so the verdict and the picture are one run. The field file is written only when the export was asked for (`export_fields`).
- A result is fresh only for the exact `Inputs` it ran for (`EccentricState` stores them; `result_is_current` / `analysis_is_current`). A stale result is listed as stale and is not drawn; the picture of the inputs returns.
- The Bushing case file is read at open and on Reload; changing it elsewhere does not change an open session. A missing or unusable case leaves the Bushing workbench's defaults and says so in the card.
- The picture is read-only (no picking). Its mesh is its own (`fe.rs`), not the solver's: do not compare its stresses with an analysis. The solved scene opens at true size (`gain = 1`).
- Switches (axial condition, housing, capacity basis, direct spin check, Advanced) work whether or not the Advanced rows are open: the ribbon offers some of them.

## Pitfalls
- `vtu::scene_source` reads only what `fea_core::vtu::write` writes (`displacement`, `von_mises`, `stress_xx_yy_zz_xy_yz_zx`; plane cell types); a change to that writer or to the solver's field names breaks it, and `vtu::tests` writes then reads to catch it.
- An analysis is 5-15 s on a worker (`jobs`, `EccentricState::job`); `Esc` cancels. Runs started while one is going queue (the toolbox's rule).
