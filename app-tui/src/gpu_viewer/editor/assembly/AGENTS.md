# gpu_viewer/editor/assembly/ - the Assembly Joints window

> TL;DR: `AssemblySession` is a read-only `Authoring` over a STEP assembly: the multi-body viewer mesh, the detected holes / fastener sites / interfaces and the advice over them, with the faces of the selected item or body highlighted. It decides nothing; every number comes from `solid_kernel::features` and `fea_problem::assembly_advice`.

## Purpose
Owns: the window front end (tree, inspector cards, ribbon, menus, selection, Exploded toggle, File > Open) and `scene.rs`, the picture of the bodies.
Does not own: detection (`solid-kernel/src/features.rs`), the advice items and their reasons / details (`fea-problem/src/assembly_advice.rs`), the mesh (`fea-problem/src/assembly_mesh.rs`), the window and card drawing (`gpu_viewer/gui`). Epic: `docs/epic-assembly-feature-advice.md` (Phase B step 3).

## Code Map
| Looking for... | Go to |
|---|---|
| Session, `AssemblyAct`, `Sel`, `load`, `Authoring` impl, keys, File > Open | `mod.rs` |
| Tree, "Assembly advice" / "Why" / Summary cards, ribbon, menus, status summary | `cards.rs` |
| Scene: shared vertices per face, body parity colour codes, selected code 3, `explode_offsets` | `scene.rs` |
| Unit tests (fixture `solid-kernel/tests/data/assembly-stacked-plates.step`) | `tests.rs` |
| GPU snapshots (none / fastener / interface exploded / joined / light) | `gpu_viewer/render.rs` `assembly_snapshot` (`FEA_SNAPSHOT_ASSEMBLY_OUT=dir`, ignored) |

## Contracts
- **Read-only.** No `Problem`, no file written, `dirty()` is always false. A `Problem` holds one body; applying a tie or fastener waits for the multi-body representation (epic Phase C).
- Advice rows are `label()` plus a dim `detail()`; the Why card is `reason()` split at sentence ends. Do not re-derive or reformat the numbers here.
- `FaceKey { body, face }` is valid for the file as read; the selection is cleared when another file is opened. Body numbers shown to the person are 1-based, `FaceKey::body` 0-based.
- Vertices are shared inside one face and never across faces, so the window's feature edges are face outlines and creases (not the tessellation) and a highlighted face colours only itself.
- Colour codes: body parity 0 / 1, selected 3; `key_labels` explains them. Explode shifts a body by `EXPLODE` (0.8) times the vector from the middle of all body centres to its own; a single body does not move.
- Letter keys accept either case (`E`; Windows reports Caps Lock / Shift letters in the other case).
- A file that cannot be read or is not STEP never errors out of `open`: the window starts empty and says why in the status bar.

## Pitfalls
- A Tie between faces that touch is internal when the bodies are joined: it shows nothing in a shaded view. Explode (default) or Wireframe.
- The inspector's cards are rebuilt every frame: keep `cards.rs` free of detection or meshing; everything it shows is in `Loaded`.
