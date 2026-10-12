# ADR-016: Manual meshing, explicit model lifecycle, refinement zones, and a simulated prior for the rankers

Status: Accepted (user direction 2026-10-11 after hand-testing the native solid editor: nothing may mesh or convert on
its own; every loaded thing can be replaced; refinement must be recommended and levelled; the learned rankers need a
broad training corpus and must keep learning)

## Decisions

1. **Nothing meshes on its own.** `SolidSession::mesh_requested` is set only by `SolidAction::GenerateMesh` (Mesh tab,
   Tools menu); `poll` starts a mesh only while it is set (the 300 ms debounce is gone). A stale or missing mesh is
   stated ("not generated (Generate)", "out of date"), never silently rebuilt. While no current mesh exists the viewport
   draws the model's sketches (or its outline and holes) as tubes in their plane (`outline_scene`), so a finished
   sketch is always visible. Solve was already manual; the terminal Workbench keeps its own auto-preview and
   `auto_solve` toggle (a documented difference: it has no menus to ask from).
2. **A model is replaced explicitly, never converted silently.** File > New part (empty `Sketch1`, sketch opened),
   File > Templates (a list), File > Open (`.json` problem or Gmsh/Abaqus mesh by path), Clear model. Each is one undoable
   edit and forgets the old model's results, mesh and selection. `New sketch` on a model without a feature history is
   refused with the choices (Convert, or New part): it used to convert the loaded template behind the user's back.
   `Delete feature` always succeeds: when what is left has no valid body the model is "unbuilt" (`validate_geometry`
   says why; the viewport shows the sketches); deleting the last feature clears geometry and boundary conditions
   (`Problem::clear_geometry`, history = empty). Rejected: keeping the old geometry after deleting its history (the
   reported bug: "I can't delete the loaded template").
3. **AMR runs inside Solve and is on by default.** `MeshSpec::default().adapt_passes` is 2 (was 0). It applies to 2D
   sketch problems only; 3D solids and imported meshes use the fixed mesh and the result panel says so with the ZZ
   error estimate. The panel reports passes, error before -> after and node counts. Extending AMR to extrusions needs
   a 3D size field in `fea-core` (not done; see `docs/issue-12-phase-25.md`).
4. **Refinement zones are part of the problem** (`MeshSpec::refine: Vec<RefineRegion{centre, radius, level}>`, serde
   default, omitted when empty; levels 1-3 divide the element size by 2 / 4 / 8, blending back over one more radius;
   at most 16). They apply to the 2D profile mesh (so also through extrusion/revolve) and are kept by adaptive passes
   (the pass's size field is clamped by the zones). `fea_problem::advice::refinement_regions` recommends them by rule:
   round holes (level 3 when a pin load or bushing acts, else 2), re-entrant polygon corners (3), supports and load
   edges (1), with the mesh resolution class of the feature. Rings in the viewport (thicker = finer; codes between the
   state codes so a ring is never read as support/load/selection) and a panel block apply a level, remove it, or apply
   all. Rejected: auto-applying hints; ML choosing *where* (the rules decide what is allowed, the ranker only orders
   the three levels).
5. **The rankers learn from a simulated prior plus the user's own sessions, and the gate stays honest.**
   - `solid/corpus.rs` generates scripted sessions (deterministic in a seed) for the three rankers (sketch constraint
     suggestions, boundary-condition kinds, refinement levels) through the real code paths (`SketchEdit`,
     `advice::boundary_options`, `refinement_regions`); only the *choice* is a stated design-intent policy. Stored in
     `<ranker>-corpus.jsonl`, separate from the user's `<ranker>.jsonl`; session ids carry `CORPUS_BIT`, and
     `read_corpus` drops anything without it.
   - `rank::train(recorded, corpus)` fits both (corpus at `CORPUS_WEIGHT = 0.25` per offer) and measures on the
     held-out sessions of the *user's* recordings once there are `MIN_SESSIONS` and `MIN_HELD_OUT_EVENTS` of them
     (`Evidence::Real`); until then on held-out corpus sessions (`Evidence::Simulated`), and every report, panel line and
     status says "SIMULATED". Real evidence overrules a favourable simulated one (test:
     `enough_real_sessions_overrule_a_corpus_that_looks_better`). Nothing is ever on by default.
   - Learning continues: while recording is on and `auto_train` (default on) is set, a ranker refits after
     `max(50, a tenth of the recordings used last time)` new offers; a refit that no longer passes the gate switches
     the learned order off.
   Why not train on simulated data and call it done: held-out *simulated* sessions come from the same policy as the
   training ones, so a gain there proves only that the model recovers the policy, not that it helps a person.
   The corpus is a prior for situations nobody has recorded yet; only the user's sessions can switch it from
   "simulated" to "measured".

## Consequences / limits

- The policies encode conventional drafting and structural-analysis practice as written in `corpus.rs`; they are not
  observed behaviour. Change them there (and the corpus-coverage tests) when real recordings disagree.
- `RANK_VERSION` stays 1: stored settings and bundles keep loading (weights are f64 counts; new report fields default).
- Existing users' saved models retrain on the next fit; nothing is lost but the bundle.
