# Issue 12 - Phase 23: native CAD front end with benches (2026-10-10)

Decisions: `docs/adr/ADR-014-native-cad-process-model.md`. Status table is updated as items land; everything is
uncommitted on `codex/fea-core-roadmap` until the user commits.

## User reports that started this phase (verified causes)

| Report | Cause | Fix |
|---|---|---|
| Cannot create / edit models | `cargo run -p app-tui` starts the terminal UI; native editors open from Workbench keys; `w` (solid editor) was not in the footer | footer lists `w` and `f`; `g`/`t`/`w` open the one CAD window |
| z of new nodes fixed at 0 | no way to set the working-plane offset with nothing selected | property row `plane z =` (`PropertyKind::PlaneOffset`) |
| Truss click does not snap to a node | fixed 10 px pick radius = half the visual size on Retina | `camera::pick_radius(size)` (viewport/50, 12..40 px) |
| No advisor recorder | entries buried in Tools of the member editor only | own `Advisor` menu in every bench (disabled with reason where no rules exist) |
| No hole feature | only an "Add hole" row action at a default position | Hole tool: click the part to place (`solid/sketch.rs`) |
| Supports/loads applied by cycling | tool click mutated blindly | click a node -> list of applicable items at the cursor (`Authoring::take_popup`) |

## Plan and status

1. [done] bench registry, Window > Bench, per-bench Simulation menu/ribbon, ADR-014.
2. [done] studies (modal / buckling / transient) inside the solid editor; terminal `g`/`t` consolidated.
3. [done] member editor: selection-driven supports/loads, tree shows loads/supports with show/hide.
4. [done] command palette (Ctrl/Cmd+K) + Alt+letter mnemonics + letters in open menus.
5. [done] Shift-click / Shift-drag box select (solid surfaces, member nodes+members; group delete/move one undo step); front-most-per-pixel 3D face pick.
6. [partly] per-bench advisor: separate folders per bench (`Store::for_bench`); rules exist only for members, other benches show the menu disabled with the reason. Needs rules per bench and real recordings.
7. [todo] GUI polish list (see `docs/issue-12-claude-handoff.md`).
8. [todo, per-crate] lug / bushing / eccentric bushing / preload / thermal benches.

## Verification (2026-10-10)

`cargo test -p app-tui -p fea-problem --locked`: 996 lib + 38 rendering + fea-problem tests pass; `cargo clippy -p app-tui --lib
--tests` clean for `gpu_viewer`; `--no-default-features` builds; release build opens `--gpu-solid` and `--gpu-editor` for 6 s
without stderr. NOT verified: real mouse/keyboard on a desktop (macOS Accessibility missing), Windows, hosted CI.

## Known gaps (not done this phase)

* Dead terminal helpers: `start_animation`, `open_gpu`, `Effect::RunFeaAnimation/OpenGpuScene` paths are no longer reachable from keys.
* Benches for lug, bushing, eccentric bushing, preload, thermal, material lookup (each needs an `Authoring` impl over its crate).
* GUI polish list in `docs/issue-12-claude-handoff.md` (IME font, dimensions, ids/arrows, ghost, probes, view cube, export, copy/paste/mirror,
  section pickers, units, autosave, file dialogs, solid per-element contours, chrome dirty flags).
* Roller semantics: `Support::roller` holds ONE component (the surface slides in the others).
