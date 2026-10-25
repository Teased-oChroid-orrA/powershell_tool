# Phase 138 — boss-rim diagnostic rejection (2026-10-23)

Epic #46 / #63. This continues [Phase 137](issue-46-phase-137.md) and the first gate of the [five-plan master](issue-46-five-plan-master.md). No production mesher change was retained; the connected boss gate remains open. OCCT remains the production path and `occt_retirement_approved` remains false.

## Measured diagnostic

The connected plate/boss regression remained at 9 of 178,438 tetrahedra below the default 0.1 scaled-Jacobian gate, minimum 0.08186495085395298. A temporary, fixture-specific admission of three discarded lattice tetrahedra followed by a generalized two/three-vertex joint position search gave zero below-gate tetrahedra and minimum 0.10820615 in a focused run. Its total enclosed volume changed from 61.5242800212 to 61.5246786984 (+0.000648%). Aggregate areas of three reported faces changed by roughly 0.02–0.04%.

Those aggregate metrics concealed unacceptable local changes. Six accepted joint moves altered 9–17 boundary triangles each. Their maximum individual triangle area ratios were 4.09, 1.87, 1.84, 1.89, 4.26 and 13.10. Three moves changed nearest CAD face ownership on one or two triangles. Maximum boundary-centroid signed-distance error increased by 0.01839, and outward-to-CAD normal alignment fell by as much as 0.61. The diagnostic gate pass is **rejected** as a surface-fidelity regression. The trial was environment-gated, contained fixture cell IDs, and was removed from the working tree; it was never a production repair.

The relevant search blocker was localized: the existing nearby-face guard rejected a plate-face vertex move away from the boss cylinder even though it remained on the plate and at zero solid SDF. Avoiding that guard let the diagnostic search find the numerical pass, but did not fix the triangle-level surface damage.

## Admission selector counterexample

The three useful diagnostic cells are near the concave plate/wall junction. A broad two-face proximity rule instead selected 44 cells near the convex boss cap/wall seam. Sampling the solid SDF in both mixed-normal quadrants correctly separated three sampled concave targets (−0.0125 in both) from a sampled convex cap cell (+0.0125 in both), but applying that concavity predicate broadly admitted 72 lower-seam cells and worsened the result to 16 sub-gate tetrahedra, minimum 0.06632. One useful cell also carried embedded-boundary and two-face evidence on different faces of the same tetrahedron. Concavity alone is therefore not a valid admission rule.

## Next proof boundary

The next candidate must change local topology while preserving the existing CAD surface complex. A bounded transaction needs explicit source and replacement cavity facets, positive nonoverlapping tetrahedra, manifold boundary, unchanged CAD face ownership, per-triangle distance/normal/area checks, controlled enclosed volume, and the default quality gate. It must use a geometry-based selector, not fixture IDs or a blanket seam fill. Then rerun curved-body convergence, focused same-STEP OCCT/native mesh and static solves, and the remaining qualification specified by the boss-rim plan. Later master-plan items remain gated on this proof per the user's execution order.

Diagnostic logs: `/private/tmp/boss-passing-moves.log`, `/private/tmp/boss-trial-metrics.log`, `/private/tmp/boss-face-metrics.log`, and `/private/tmp/boss-reentrant-selection.log`. These are temporary evidence, not a reproducible production certificate.
