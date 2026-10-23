# Phase 132 — shared boss-rim mesh quality diagnosis (2026-10-22)

Epic #46 / #63, continuing [Phase 131](issue-46-phase-131.md). This phase is diagnostic only. OCCT retirement remains unapproved; no solver quality threshold or geometry guard changed.

## Representative comparison

The existing release OCCT/native parity harness was rebuilt from `0db63d1` plus the concurrently tested, subsequently reverted Phase 131 edits to `solid-kernel/src/chart_mesh.rs` and `planar_step.rs` (their temporary diff SHA-256 was `979dfd74af462dbce67afaad19f0815462664f6367eeee4867b022dbc7bb108a`). The focused `CASE='plate with a boss' NO_SOLVE=1 REPAIR_TRACE=1` table test exited 0. That harness reports measurements but does not assert its quality gate. These numbers are diagnostic for that temporary source snapshot, not a fresh baseline measurement at the retained source.

| Surface path | Tets | Minimum scaled Jacobian | Tets below 0.1 | Mesh time |
|---|---:|---:|---:|---:|
| OCCT | 57,935 | 0.066 | 6 | 7.22 s |
| Native | 57,934 | 0.063 | 20 | 16.80 s |

Every remaining sub-gate tet touched a boundary node. The repair began with 639 sub-gate tets on the OCCT path and reduced them to 6; the native path likewise reduced a large boundary-layer population to 20. The first unresolved cells on both paths lie around the circular boss's base at `z≈1`, near `(4.9, 1.6)` and `(7.1, 2.4)` respectively (boss center `(6, 2)`, radius `1.2`). Native unresolved cells also contain edges about `0.013–0.021 mm` next to local length scales about `0.15–0.24 mm`.

Diagnostic retries of the first few cells did not clear them. The repair trace shows no feasible local flip or edge removal for many candidate cavities; proposed edge collapses can invert neighboring cells, and vertex moves often leave the minimum quality unchanged or below the 0.1 gate. Those refusals enforce the existing orientation, surface-fidelity and local-quality contracts. A single synthetic tet cannot reproduce the decision: repair evaluates the full vertex stars and neighboring cavity, with face-distance projections at the rim. No threshold, round count or rejection reason was changed on this evidence.

## Limit

This locates the shared failure at the boss/plate junction and rules out the simple 100-round stagnation explanation already tested in Phase 128. It does not establish which upstream sampling or snapping step creates each near-short edge, nor a topology-preserving repair. A useful next regression would retain a small connected boss-rim cavity with its model face-distance context, then compare an admissible topology change against the current refusal. The focused run skipped static solves, other models, and Windows runtime meshing.
