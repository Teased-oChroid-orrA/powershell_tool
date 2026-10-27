# Phase 148: NEMA17 boundary repair feasibility screen

2026-10-27. Diagnostic continuation of [Phase 147](issue-46-phase-147.md). No production code changed. OCCT remains production, and `occt_retirement_approved` remains false.

## Baseline

I reran the release NEMA17 comparison with `REPAIR_TRACE=1`, `CASE=NEMA17`, and `NO_SOLVE=1` on `main` at `a3c61bd`. The test passed. The quality gate still fails on both paths: OCCT has 12 elements below scaled Jacobian 0.1, and native has 11. Every sub-gate element is on the boundary. The OCCT and native meshes have 205,952 and 205,995 tetrahedra, respectively. No static solve ran.

## Rejected local changes

The lower gusset corner still has an interior apex over a thin boundary triangle. The triangle crosses the convex slope and gusset-side feature. Moving its nearby corner vertex fails the neighbouring quality or surface checks (`corner#3` and `corner#4`), while collapsing its short pinned edge fails the quality check (`collapse#7`). These are the same refusals Phase 147 recorded.

`remove_edge` already offers a boundary edge swap. When both endpoints are pinned, it requires `same_owner_flat_quad` before replacing the edge. The lower-corner edge crosses two CAD faces, so lifting the interior apex alone cannot make that edge swap valid. I left this guard intact. A replacement must preserve the model feature and check the changed surface against its prior CAD face owners.

At the base slot ends, two flat cells each have two boundary faces that meet on a pinned feature edge. `peel` refuses the pinned pair (`peel#3`), and `remove_edge` refuses the same feature-crossing swap (`remove_edge#6`). Relaxing either guard would remove the CAD edge. The one-apex `rebuild` also leaves these cells below the gate. This screen does not prove that every larger cavity is impossible.

## Next work

Try a feature-preserving cavity change that keeps the existing CAD edge, places the interior vertices, and judges all affected cells together. Require positive elements, unchanged cavity volume, no overlapping neighbours, unchanged face ownership, and strictly fewer elements below the 0.1 gate. If a candidate fails those checks or leaves the NEMA17 counts at 12 and 11, revert it and record the result. The OCCT `fillet_rim_tagging` test is a separate confirmed CI failure: at size 1, it tags 80.42372283704826 mm² against the exact 80 mm² and the existing 0.5% limit. It needs seam-conforming face tagging, not a looser limit.

The baseline command was:

```sh
REPAIR_TRACE=1 CARGO_INCREMENTAL=0 CASE=NEMA17 NO_SOLVE=1 cargo test -p fea-problem --features occt --release --locked --offline -j1 --test occt_vs_native table -- --ignored --nocapture
```

The focused OCCT test command was:

```sh
cargo test -p fea-problem --features occt --release --locked --offline -j1 --test fillet_rim_tagging the_flat_face_next_to_tangent_fillets_is_tagged_with_its_own_area_and_no_vertex_leaves_the_solid -- --nocapture
```
