# toolboxes/bushing/ — Bushing Workbench model

> TL;DR: This module bridges `bushing-solver` and `edge-check` into the native Bushing window. The terminal Bushing toolbox was retired. `model.rs` computes no engineering formulas.

## Purpose

`BushingState` owns the model, checks, recommendations, and edge-check state. `gpu_viewer/editor/bushing/` owns the UI. The model also supplies a default case to the terminal Eccentric Bushing toolbox; the native Eccentric window picks its own Bushing case file (#242).

## Code Map

| Looking for... | Go to |
|---|---|
| Solver input bridge, `FieldRow`, `NumberTarget`, `field_rows`, field labels | `model.rs` |
| Trial-solved recommendations and tolerance explanation | `advice.rs` |
| Independent edge-distance checks, uncertainty, worker input and report rows | `edge_check.rs`, `mod.rs` |
| Saved reamer, material, and Bushing ID libraries | `reamer_picker.rs`, `material_picker.rs`, `bushing_id_picker.rs`, `src/library.rs` |
| Plain-text engineering report | `report.rs` |
| Native input and result cards, pickers, Send dialog, FE check | `gpu_viewer/editor/bushing/` |

## Contracts

- Change Lamé, fit, tolerance, and stress formulas in `bushing-solver/`, never in this adapter. Change the independent edge models in `edge-check/`.
- `BushingModel::recompute` fills checks and recommendations. `advice.rs` verifies each recommendation by trial-solving a clone and rejects one that creates a new failed check. The advice search calls `recompute_output`, not `recompute`, to avoid recursion.
- Bore recommendations use real reamer sizes. Bushing ID recommendations may use drill sizes. A bore must never snap to a drill.
- `BushingState::edge_settings` holds session-only uncertainty values. Values are finite percentages from 0 through 100. Changing one invalidates the report and in-flight edge result. The saved engineering case and undo state exclude these controls.
- Edge checks store the exact `EdgeInput` and configuration used. A changed input marks the result stale; the independent check never changes the solver's governing result. The contact FE check is optional and slower than the quick models.
- Built-in and user materials resolve to `Material` values before reaching the solver. Keep saved library formats and conflict rules in `src/library.rs`.

## Pitfalls

- The default case fails the edge-distance sequencing check. This is the solver's result for the fixture.
- The native window manages the saved reamer, material, and Bushing ID libraries (`gpu_viewer/editor/bushing/library.rs`). The model loads `custom_materials` / `user_reamers` once in `Default`; library edits must go through `set_custom_materials` / `set_user_reamers`.
- Pickers load the user's library when opened. Construct each picker once per opening, never on each frame.

## Navigation

Parent: `app-tui/AGENTS.md`. Native window: `gpu_viewer/editor/bushing/AGENTS.md`. Solver: `bushing-solver/AGENTS.md`.
