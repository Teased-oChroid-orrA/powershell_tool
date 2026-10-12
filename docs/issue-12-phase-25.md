# Issue 12, phase 25: sketch UX fixes, model lifecycle, manual mesh, AMR, refinement hints, learning corpus

Decisions: `docs/adr/ADR-016-manual-mesh-model-lifecycle-and-learning-prior.md`. Previous phase: `phase-24.md`.
Branch `codex/fea-core-roadmap`, uncommitted on top of `ae05501` (with phase 24).

## What the user reported and what was done

| # | Report | Finding (evidence) | Fix + regression test |
|---|--------|--------------------|-----------------------|
| 1 | "I can create the sketch, but once I exit I don't see it." | Reproduced headlessly: after Finish the scene was the 3-vertex "mesh pending" placeholder; the sketch was drawn only while `sketching` was Some (`build_scene_inner`). With manual meshing it would stay empty. | `outline_scene` draws the sketches / outline in their plane whenever no current mesh exists. `after_finishing_the_sketch_is_drawn_even_before_any_mesh_exists`; rendered snapshot `FEA_SNAPSHOT_FINISHED=1 FEA_SNAPSHOT_EDITOR_OUT=dir cargo test -p app-tui --lib sketch_snapshot -- --ignored`. |
| 2 | "I can't delete the loaded template." | Templates only cycled, `new_sketch` silently converted a legacy model, `Delete feature` of the last feature left the geometry. | File > New part / Templates (list) / Open / Clear model; `New sketch` on a history-less model is refused; Delete always works and the last delete clears the model (`models_tests.rs`, `fea-problem` `Problem::clear_geometry`, `templates::{blank, imported}`). |
| 3 | "I don't see a feature tree for the new sketch." | NOT reproduced as a tree defect: the converted template lists `History (n)` / `Sketch1` expanded (headless tree test and rendered snapshot). A new empty model had no path to test; New part now shows `History (1)` and `Sketch1 [editing]` at once. | `new_part_replaces_the_template_with_an_empty_sketch_and_shows_it_in_the_tree_at_once`. If the user still sees no tree, the cause is outside the tree content (panel hidden / collapsed): ask for a screenshot. |
| 5 | Disable auto-solve and automatic meshing | Solve was already manual in the native editor; the debounce preview mesher was automatic. The terminal Workbench has its own `auto_solve` (default on) and auto-preview. | `GenerateMesh` is the only mesh trigger; "not generated" / "out of date" states; window test proves nothing meshes until the Mesh tab's button is clicked. Terminal Workbench left as is (no menus there): see Limits. |
| 6 | Mesh must use AMR | AMR (`adapt_passes`, ZZ) existed but defaulted to 0 and was invisible. | Default 2; result panel reports passes, error and node counts, and says "fixed mesh" for 3D / imported. `adaptive_refinement_is_the_default_and_the_result_panel_reports_what_it_did`. |
| 7 | ML highlight of recommended refinement with level | New. | `advice::refinement_regions` (rules), `MeshSpec::refine` zones, rings in the viewport, panel block, level ranker (`solid/refining.rs`), `fea-problem/tests/refine.rs`, `solid/refining_tests.rs`. |
| ML | "Generate comprehensive training recordings so the ML is fully trained, but keep learning" | Only the user's own recordings existed; no way to start from anything. | Simulated corpus + weighted prior + honest evidence label + continual refits (ADR-016 section 5): `solid/corpus.rs`, `rank.rs`, Advisor menu "Build training corpus (simulated prior)" and "Learn as I work". |

## The learning loop as built

`Advisor > Build training corpus` writes 600 scripted sessions per ranker (about 2,400 sketch offers over 9 situations,
2,200 boundary offers over about 40, mesh-density offers over the region kinds) and fits all three rankers. Measured on
held-out simulated sessions: constraints top-1 rules 59% vs learned 75% (z 4.5), boundary 74% vs 89% (z 6.7). That is the
policy being recovered, labelled SIMULATED everywhere; nothing is switched on (`Use learned ranking` stays an explicit
opt-in). As the user records (`Record my ... choices`) the rankers refit automatically (default on) and, once there are
`MIN_SESSIONS` (20) of their own sessions with `MIN_HELD_OUT_EVENTS` (100) held-out offers, the verdict is measured on
them and can only be "eligible" if they beat the rules.

Next step for learning, when real data exists: compare the user's accept rates against the policies in `corpus.rs` and
edit the policies (and `CORPUS_WEIGHT`) where they disagree; the corpus is regenerated deterministically.

## Verification

`cargo test -p app-tui -p fea-problem --locked -j4 --no-fail-fast`: 1200 passed, 12 ignored (GPU snapshots), 0 failed.
`cargo clippy -p app-tui -p fea-problem --lib --tests`: no warnings in `gpu_viewer` / `fea-problem`. `cargo check -p app-tui
--no-default-features --lib --bins` clean. Rendered snapshots (headless wgpu): finished sketch with outline and no mesh,
mesh view with refinement rings and the Mesh density panel. NOT verified: real mouse/keyboard on the desktop (no
Accessibility), Windows, CI.

## Limits (recorded, not hidden)

- The terminal Workbench (`toolboxes/fea_workbench`) still auto-previews and has an `auto_solve` toggle defaulting on.
  Making it manual needs keys/menus there; not done.
- AMR is 2D only. Extrusions/solids use the fixed mesh (the panel says so); a 3D size field in `fea-core` is the fix.
- Refinement zones are circles in the sketch plane; no zones from ZZ hot spots of a previous solve yet (AMR covers that
  for 2D), no per-surface sizing.
- The corpus policies are my statement of drafting / analysis practice, not observed behaviour. Simulated evidence is
  circular by construction (ADR-016); only real sessions can show the learned order helps.
- Open uses a typed path (no file dialog in the native window yet).

## Item 4 ("What isn't there"): NOT started in this phase, planned

Each is a separate piece of work with its own design note before code; the product decisions are the user's.

| Feature | Proposed approach | Decision needed |
|---------|-------------------|-----------------|
| Boss / cut, several solid features | Extend `History` with `Feature::{Boss, Cut}` over a *region-algebra on 2D profiles*: extrusions of profiles on the same plane fuse as 2D region unions/differences before meshing (no 3D boolean kernel); different planes need a 3D boolean | Is same-plane boss/cut enough for the first cut, or is a real 3D boolean required? (a 3D boolean is a large new kernel) |
| Sweep | Expose a general `fea_core::sweep` mapping along a polyline path with a parallel-transport frame; path = second sketch chain of lines/arcs | none (path from a second sketch) |
| Islands / separate regions | `Profile` returns a list of regions; geometry becomes a multi-region union meshed together (needs shared-edge handling) | none |
| Stable topological naming | Entity-id based names with a persisted map in the history | affects every `Shape` match: schedule with native arc `Shape` |
| Native arc `Shape` | New `Shape::Arc`-bounded polygon variant; every `Shape` match in `fea-problem` and the Workbench `model.rs` | none, but touches many files: do as one `safe-refactor` |
| Constraint glyphs / dimension text in the viewport, per-entity DOF colour | Needs line / text primitives in the renderer (scene is triangles only, 8x8 ASCII font via the overlay) | none |
| Datum-plane / face picker, construction geometry, splines/ellipses/symmetric, h/v distance tool | UI + sketch-model additions | order of priority |
| Thermal problem type + advice | New `Analysis` variant on `fea-core` heat conduction | which thermal loads/BCs first |
