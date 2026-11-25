# gpu_viewer/editor/assembly/ - the Assembly Joints window

> TL;DR: `AssemblySession` is a read-only `Authoring` over a STEP assembly: the multi-body viewer mesh, the detected holes / fastener sites / interfaces and the advice over them, with the faces of the selected item or body highlighted. It decides nothing; every number comes from `solid_kernel::features` and `fea_problem::assembly_advice`.

## Purpose
Owns: the window front end (tree, inspector cards, ribbon, menus, selection, Exploded toggle, File > Open) and `scene.rs`, the picture of the bodies.
Does not own: detection (`solid-kernel/src/features.rs`), the advice items and their reasons / details (`fea-problem/src/assembly_advice.rs`), the mesh (`fea-problem/src/assembly_mesh.rs`), the window and card drawing (`gpu_viewer/gui`). Epic: `docs/epic-assembly-feature-advice.md` (Phase B step 3).

## Code Map
| Looking for... | Go to |
|---|---|
| Session, `AssemblyAct`, `Sel`, `load` (one `fea_problem::assembly_mesh::read_assembly` for detection and mesh), `Authoring` impl, keys, File > Open | `mod.rs` |
| Tree, "Assembly advice" / "Why" / Summary cards, ribbon, menus, status summary | `cards.rs` |
| Scene: shared vertices per face, body parity colour codes, selected code 3, `explode_offsets` | `scene.rs` |
| Unit tests (fixture `solid-kernel/tests/data/assembly-stacked-plates.step`) | `tests.rs` |
| GPU snapshots (none / fastener / interface exploded / joined / light / slid plates with an unmated hole) | `gpu_viewer/render/snapshot_tests/fit_workbenches.rs` `assembly_snapshot` (`FEA_SNAPSHOT_ASSEMBLY_OUT=dir`, ignored) |

## Contracts
- **Read-only.** No `Problem`, no file written, `dirty()` is always false. A `Problem` holds one body; applying a tie or fastener waits for the multi-body representation (epic Phase C).
- The inspector groups are Fastener sites, Interfaces, Holes without a mate (`AssemblyAdvice::Unmated`: a bolt-sized hole with no mating bore, a place for a support or a load) and Check (a warning group: `AssemblyAdvice::NearClosed`, a bore that covers nearly a full turn, not a hole, and an unresolved pair). A near-closed bore is highlighted by its bore faces (the same path as every item); where the gap is, is in its Why text (`gap_point`, millimetres), not a marker on the model. `Loaded::near_closed` counts them for the status bar, the Summary card and the Detected tree node. `Loaded::unmated` counts the third group for the status bar, the Summary card and the Detected tree node.
- The Why text of a fastener site and of an unmated hole ends with the geometric nearest free edge of each bore (`solid_kernel::free_edge`, via `assembly_advice`): distance from the axis, e/D, edge kind, plate thickness, or why none was read. Geometry only: the window has no material or load, so no strength estimate (`edge_check::estimate`) is built here; that needs per-body material and a pin load first (`docs/edge-distance-crosscheck.md`, Assembly Joints).
- Advice rows are `label()` plus a dim `detail()`; the Why card is `reason()` split at sentence ends. Do not re-derive or reformat the numbers here.
- `FaceKey { body, face }` is valid for the file as read; the selection is cleared when another file is opened. Body numbers shown to the person are 1-based, `FaceKey::body` 0-based.
- Vertices are shared inside one face and never across faces, so the window's feature edges are face outlines and creases (not the tessellation) and a highlighted face colours only itself.
- Colour codes: body parity 0 / 1, selected 3; `key_labels` explains them. Explode shifts a body by `EXPLODE` (0.8) times the vector from the middle of all body centres to its own; a single body does not move.
- Letter keys accept either case (`E`; Windows reports Caps Lock / Shift letters in the other case).
- A file that cannot be read or is not STEP never errors out of `open`: the window starts empty and says why in the status bar.

## Pitfalls
- Tree node ids come in blocks (`cards.rs`: fixed nodes below 1000, bodies from 1000, advice from 1000000): the expand state is keyed by the id, so an assembly with 90 or more bodies used to collide with the fixed nodes.
- `selected_faces` for a body is a sorted set, not a `dedup`: nothing orders the mesh triangles by face.
- A Tie between faces that touch is internal when the bodies are joined: it shows nothing in a shaded view. Explode (default) or Wireframe.
- The inspector's cards are rebuilt every frame: keep `cards.rs` free of detection or meshing; everything it shows is in `Loaded`.
