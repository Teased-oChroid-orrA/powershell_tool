# Issue 46 — Phase 113: #61 corpus widened to unstructured, graded and NAFEMS meshes; `CostModel::calibrate` on meshes without a flat face

Phase 88 qualified the learned stage-time model on generated structured grids only. This phase generates the missing data by running the solver (offline, deterministic, no external corpus): five new families, then re-runs the same held-out gate.

## Data generated

`fea-core/tests/cost_validation.rs::learned_cost_against_the_deterministic_model_on_held_out_families` (`--ignored`, release) now measures 16 families, 7 sizes each (112 meshes; best of three, assemble / factor / back-solve seconds):

- the Phase 88 families (Quad4/8/9, Tri3, Tri6 grid and plate with a hole, Hex8/20/27, Wedge6/15), and new:
- `tet4 ball`, `tet10 ball`: unstructured tetrahedra from `tetmesh::mesh_solid` (sphere, boundary size 0.5 to 0.15 / 0.7 to 0.22; up to 16.5 k and 54 k dofs);
- `tet4 graded`: the same ball with an interior size field refined towards (0.7, 0, 0);
- `LE1 quad8`, `LE10 hex20`: the NAFEMS LE1 / LE10 elliptic geometries of `tests/nafems.rs` (up to 75 k and 43 k dofs).

Machine: Apple M1, macOS arm64. Timings are machine-specific: retrain per machine.

## Result

| leave-one-family-out (105 learned answers of 111) | median error | 90th percentile | worst |
|---|---|---|---|
| learned | **x1.12** | x1.39 | x1.89 |
| deterministic, cross-family calibration | x1.88 | x4.86 | x9.04 |
| deterministic, own-family calibration | x1.89 | | |

The 90 % interval covered 95 of 105 learned answers (90 %); the model fell back to the deterministic one on 6 of 111 meshes, each time with its reason.

A disjoint split, not only leave-one-family-out: fit on the structured grids alone and predict the unstructured / graded tetrahedra, the NAFEMS geometries and the plate with a hole (41 meshes): learned median **x1.14** (answered 24 of 41; the rest fell back), deterministic **x1.92**. Asserted in the test (median under half the deterministic one).

Reading: the Phase 88 conclusion holds on data that is not a structured grid, including unstructured and graded tetrahedra. The entry condition of benchmarks D.3 (a held-out gain over the deterministic model) is met for direct linear static solves on this machine. Learned predictions stay **off** in production (nothing calls `LearnedCost` outside tests); turning them on needs per-machine training data from the verified telemetry store (`fea-problem::mesh_telemetry`) and a product decision. Not covered: other machines, contact / plasticity / iterative solves, adaptive loops, mesh generation time, the licensed ABC / Thingi10K corpora (not downloaded: licences, offline default; `benchmarks.md` section C).

## Defects found and fixed

1. **`CostModel::calibrate` failed on any mesh without a flat x-minimum face.** It fixed only the nodes at the x minimum, which for a ball is one node: "the model is not fully constrained (3 of 6 rigid-body motions are held)". Now it widens the supported patch (5, 10, 20 % of the x extent) until the system is constrained. Regression test `calibration_supports_a_mesh_without_a_flat_face`.
2. **Corpus label error**: LE10 Hex20 with 10 layers (42 603 dofs) was recorded as a direct solve with 0.031 s, 47x under the learned prediction. `MeshCost::of` estimates the factor with one node fixed (59.0 M entries, below the 60 M iterative threshold); the solver decides on the supported system (over 60 M) and goes iterative. The estimate differs from the solver's factor by about -10 % to +3 % on these meshes (Hex20 8^3: 22.0 M estimated vs 24.4 M solver; 9^3: 42.4 M vs 41.2 M; 10^3: 59.0 M vs over 60 M), so `MeshCost::iterative` can be wrong within about 15 % of the threshold. The harness now excludes a sample whose solver factor is zero and prints it. `fea-core/AGENTS.md` records the limit; making the flag support-aware needs the supports at cost time (an API change, not done).

## Evidence

`cargo test --release -p fea-core --locked --offline --no-fail-fast`: 549 passed, 0 failed (548 + the new regression test). The `--ignored` timing test above: passed.
