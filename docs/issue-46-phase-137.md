# Phase 137 — connected boss-rim cavity diagnosis (2026-10-23)

Epic #46 / #63. This is a bounded negative result for the first item of [the five-plan master](issue-46-five-plan-master.md). No production mesher change was retained. The user required each prior item to be fully operational before starting the next; tilted-hole, Windows runtime, ABC and native-window work were not started in this execution pass. OCCT remains production and `occt_retirement_approved` remains false.

## Reproduction

An ignored diagnostic `fea-core/tests/tetmesh_repair.rs::connected_plate_boss_rim_passes_solver_gate` now constructs a connected plate `[0,0,0]`–`[12,4,1]` with radius-1.2 boss centered at `[6,2]` up to `z=4` (`base=0.6`, `boundary_size=0.25`). Its analytic SDF and face hits are a deliberately small synthetic approximation; it is not the exact Phase 135 diagnostic or a CAD B-rep. The unchanged repair leaves **9 of 178,438 tets below** the default 0.1 scaled-Jacobian gate, with minimum **0.08186495085395298** and no inverted elements. The test was observed failing before a proposed fix, remains ignored so routine qualification stays green, and can be run explicitly with `--ignored`. It is an open regression target, not a passing certificate.

With `REPAIR_TRACE=1`, the existing pass finished in about 4.2 s after meshing and reported 4,323 link-condition refusals (`collapse#4`), 13,719 orientation refusals (`collapse#5`), 6,898 quality refusals (`collapse#7`) and 1,670 surface-fidelity refusals (`collapse#8`) across candidates and rounds. A sampled rejected collapse had an extra common neighbour of the short-edge endpoints absent from the edge link. The counts are attempted operations, not unique defects.

## Rejected candidates

1. A rollback-protected `remove_edge` then `collapse` sequence using both operations' ordinary quality guards made no change: 9 sub-gate tets, minimum 0.08186495. Trace added 14 quality-rejected edge-removal attempts and accepted no combined transaction.
2. A trial deferred the intermediate gain checks inside one journal transaction and required final whole-cavity quality, volume and surface checks. It still made no change: 20 eligible trials, 16 failed edge removal, four removed an obstructing edge but then failed positive orientation during collapse; none reached the final acceptance gate. This code was reverted.
3. For a representative direct collapse, 23 surviving source-star tets and 11 survivor-star tets were inspected. Projecting the survivor from `z=1.0044642857` straight to the radius-1.2 rim at `z=1` made one replacement tet's signed volume more negative (about -1.22e-6 to -1.59e-6 mm³). A sampled tangential shift of about 0.06 mm along the rim made that representative 34-tet star positively oriented, but the existing surface-change bound is about 0.013 mm (`0.1` times local size about `0.13` mm). Sampled positions within that bound left at least one tet inverted. This is evidence against a small survivor move for this local star; it does not prove that every larger cavity is impossible.

## Blocking proof and next step

The remaining operation is a larger constrained cavity replacement, not a relaxation of the snap, orientation, quality or CAD-fidelity gates. The current local `remove_edge` handles a link of at most seven tets; a proposed single-seed fan over a 34-tet union cavity is not certified because that cavity has not been shown star-shaped. Limiting the search to a few interior seeds would not prove that the resulting tetrahedra cover the cavity exactly, avoid overlap, preserve its exterior facets and face identities, or satisfy manifoldness and positive orientation. A future implementation must first provide a small reproducible cavity fixture and a checker for those properties, then test a bounded retriangulator with rollback and unchanged surface/volume/quality gates. Until that proof exists, the boss-rim plan is **not operational** and the master plan's dependent items must not advance.

## Verification and limits

On macOS arm64, `cargo test -p fea-core --locked --offline -j1 --test tetmesh_repair` passed 3 tests and ignored 2 (including the open boss target). `git diff -- fea-core/src/meshing/boundary_repair.rs` is empty and `git diff --check` passed after restoration. No OCCT/native same-STEP parity or static solve was run for this rejected candidate, and no Windows claim follows. About 9.2 GiB was free after the diagnostics; subsequent Cargo runs should continue serially in this checkout's own target.
