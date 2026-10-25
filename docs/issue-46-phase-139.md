# Phase 139: boss geometry and repair contracts

2026-10-24. This continues [Phase 138](issue-46-phase-138.md) on `issue-46-boss-rim-repair`, based on `b24e27d40f85ca0813bc92ab31b86274269bf8ef` with uncommitted changes. The boss gate remains unqualified. No later item of the five-plan handoff has advanced. OCCT remains production and retirement approval remains false.

## The inherited guard does not clear the boss

The inherited changes passed 217 library tests and three routine repair tests. The explicit old connected-boss fixture instead left 35 of 177,852 tetrahedra below the default quality gate, minimum scaled Jacobian 0.06069878172762966. The recorded committed result was nine of 178,438, minimum 0.08186495085395298. Routine passes therefore did not qualify this change.

The smoother selected its best numerical position before checking surface fidelity. If that position failed, it discarded smaller valid improvements. A focused pyramid test failed on that path and passed after fidelity entered candidate scoring. The final surface guard remains in place. On the old boss fixture this separate fix still left 35 bad elements, so it is not a boss solution.

## The original synthetic geometry was defective

The plate and boss met at `z=1`, and their minimum SDF returned zero throughout their shared interior disk. At `[6,2,1]` the old result was zero. The distance to the nearest external boundary is one, so the correct signed distance is minus one. The face callback also omitted the plate sides and discarded nearby trimmed-face hits on the material side of the concave rim.

`tetmesh_repair.rs` now defines eight exact exterior patches. Their nearest points own the SDF and the face projection data. A patch hit uses the same signed projection convention as `SurfaceSolid::faces_near`. Mesher hits are limited to `1.2 * boundary_size`, while SDF evaluation uses every patch. This proximity limit matters because distant face hits can incorrectly forbid a valid wall slide.

The new geometry test checks the internal disk, both sides of the rim, all eight patch projections, side faces, the proximity filter, and a wall slide with no distant plate hit. The corrected geometry with the proximity filter left 26 of 173,973 tetrahedra below gate, minimum 0.056727433823971614, before the normal and classifier fixes below. This is a new target, not a quality improvement relative to the broken fixture.

Phase 137 and Phase 138 measurements made through the old SDF or old face callback are not CAD-fidelity certificates. Their raw geometry measurements, including triangle area ratios, remain observations. Historical notes remain intact.

## A fixed-boundary refill cannot repair the sampled cavities

The trace captures complete endpoint-star cavities rather than favorable subsets. The first three contained 27 points and 46 cells, 23 points and 31 cells, and 26 points and 42 cells. Each had a boundary triangle with an exact face-angle bound below the 0.1 quality gate. Their upper bounds were approximately 0.0895224, 0.0815227, and 0.0815227.

For the implemented metric, every incident tetrahedron satisfies `sJ <= sqrt(2) * sin(theta)` for each angle of each face. The screen compares the squared bound using exact rational representations of the captured binary coordinates. No choice of apex can lift these locked triangles through the gate. Some directed contractions remove the obstructing triangle, but direct replacement then inverts cells or leaves other sub-gate elements.

The local tool `.audit/issue46-cavity-screen.py` has an assertion-based self-check and reads the trace without adding dependencies. It reports necessary conditions only. It does not prove embedding, CAD fidelity, or a replacement partition. The independent design comparison selected the geometry certificate and this screen before either proposed search driver. No search driver has been integrated.

## Projection directions and face normals are different facts

`SurfaceSolid::faces_near` returns a projection direction near trims. Feature detection treated that direction as the face's geometric normal. At `[7.1,2,0.9]` both boss hits project to `[7.2,2,1]` with parallel directions, although the top and wall normals are perpendicular.

The repair now queries each face at its own projected point to recover its on-face normal. Missing hits, off-face results, and invalid normals pin the vertex and refuse feature motion. The trimmed projection arithmetic remains unchanged. This avoids adding another vector to every public `FaceHit` constructor.

`RepairContext::triangle_face` now receives the existing final `face_of_triangle` classifier, captured with the actual boundary size. The repair's separate centroid ownership heuristic is removed. A coplanar trimmed-patch test covers the disagreement between centroid preference and common-vertex tagging, including vertex permutations and the fallback path. A guard test checks that local cavity size does not change the chosen owner.

The added field changes Rust struct literals for `RepairContext`. All workspace constructors are migrated together. `FaceHit`, `Solid`, `TetOptions`, and persisted data retain their shapes. Generic repair remains off by default.

## Evidence at this checkpoint

On macOS arm64, these focused runs passed.

- `cargo test -p fea-core --lib --locked --offline -j1` passed 219 tests and ignored seven before the geometry-contract additions.
- `CARGO_INCREMENTAL=0 cargo test -p fea-core --lib --locked --offline -j1` passed 236 tests and ignored seven with normal recovery, canonical ownership, and the test-only contact predicate.
- The corrected fixture's routine repair suite passed four tests and ignored two before normal recovery.
- The local smoother regression failed before feasibility scoring and passed after it.
- The exact-contact predicate passed 13 focused tests, including coplanar overlap, shared-simplex contact, containment, permutations, and near-degeneracy. It is registered only for tests and has no production repair caller. Independent review is still required.

An interrupted build was followed by an arm64 link failure with missing LLVM-local symbols. `cargo clean -p fea-core` removed only generated artifacts and reclaimed 11.9 GiB. Subsequent qualification uses this checkout's own target with `CARGO_INCREMENTAL=0` and serial Cargo runs. Installed OCCT libraries were found under `target/occt-8_0_1_rev3-aarch64_apple_darwin`.

The normal-recovery boss rerun, curved-body and fillet preservation, actual same-STEP quality and static solves, and full boss-plan qualification remain open at this checkpoint. Passing local tests does not qualify the application or authorize retirement.


## Later checkpoint: owned-patch fidelity and exact patch diagonals

The boundary guard now samples both the global SDF and the authoritative owner's trimmed distance. It uses that owner's recovered normal when face geometry is available. The canonical classifier computes its own deterministic compensated centroid, so caller vertex order cannot change the fallback tag. Three older synthetic tests had artificial moved boundary faces that violated the stronger contract. Their stars were corrected rather than loosening the guard. The library suite then passed 241 tests with seven ignored in `/private/tmp/issue46-owned-fidelity-lib.log`.

All 21 corrected captured cavities failed the inward-apex support screen. Fifteen also had an exterior face-angle bound below the gate. Existing-point refill, projected midpoint contraction, added kernel seed, and optimized single-fan prototypes did not produce a passing certificate. No broad refill search was integrated.

The existing open-link edge-removal operation had a narrower verified obstruction. It rejected every edge with two fixed endpoints, including a diagonal inside one flat CAD patch. Its new exception requires exact coplanarity, strict convexity, boundary incidence, and one authoritative CAD owner for all four old and new triangles. The existing DP and transaction remain responsible for replacement connectivity and rollback. Replacement cells must be exactly positive, distinct, and pairwise compatible under the exact contact predicate. The predicate is now privately available in production for this admission.

The captured six-point star passes the focused `fixed_endpoints_allow_only_a_single_flat_patch_diagonal` regression. Independent read-only review found no correctness blocker. The review requested explicit replacement-only owner, tilted-plane, and crossing-cell coverage, which was added before the full library rerun. Full boss acceptance is still open.

A standalone Rust replay uses the actual `cavity_contact.rs` and the kernel's exact orientation predicate on the older 21 captures. Captures 0 through 18 have no forbidden pair. Capture 19 has pairs `(6,17)`, `(6,19)`, `(14,23)`, `(14,24)`. Capture 20 has pairs `(8,28)`, `(8,32)`, `(23,37)`, `(23,38)`. The replay source and output are `/private/tmp/issue46-contact-replay.rs` and `/private/tmp/issue46-source-contact-rust.log`. These are real Rust confirmations of inherited contacts, not proof of their origin or of the current mesh's conformity.


## Current diagonal result and whole-mesh contact audit

The same-patch admission and current owned-patch fidelity changes pass `CARGO_INCREMENTAL=0 cargo test -p fea-core --lib --locked --offline -j1`, with 243 passed and seven ignored. The four focused integration targets `amr_in_place`, `high_order_convergence`, `tetmesh`, and `tetmesh_repair` pass 16 tests in total, with five ignored. Logs are `/private/tmp/issue46-patch-lib.log` and `/private/tmp/issue46-patch-integrations.log`.

The explicit connected-boss assertion still fails. It reports seven of 173,939 cells below 0.1, with minimum scaled Jacobian 0.03843258253158435. The log is `/private/tmp/issue46-patch-boss.log`. This run includes both the new diagonal exception and owned-patch fidelity changes newer than the previous full run. It does not isolate either change's causal contribution. The inward-apex screen of these seven captures remains negative for each selected fixed point set. Three captures contain exterior face-angle obstructions, including two captures of the same short-edge region.

A temporary integration-only capture exported the complete boss mesh with repair disabled and enabled. That capture test was removed after execution and saved separately as `/private/tmp/issue46-contact-capture-test.rs`. The mesh files are `/private/tmp/issue46-boss-repair-false.mesh` and `/private/tmp/issue46-boss-repair-true.mesh`.

A standalone Rust audit checks every AABB-overlapping pair using the actual exact contact module. Assertion checks distinguish a valid shared face from an invalid volume overlap before the full run. The unrepaired mesh has 116 forbidden pairs among 10,313,090 broad-phase candidates. The repaired mesh has 48 among 10,297,750 candidates. Thus the defect already exists before boundary repair, and repair does not make the final boss conforming. These counts concern forbidden intersections only. The first audit version does not separately certify duplicate cells or exact positive orientation. Audit source and complete outputs are `/private/tmp/issue46-global-contact-audit.rs`, `/private/tmp/issue46-global-contact-before.log`, and `/private/tmp/issue46-global-contact-after.log`.

No later five-plan gate advances. A temporary, bounded-region snap/reconnect trace is being used to locate the first invalid source state. Its geometry selectors are diagnostic only and must be removed before retaining source changes.


## Contact origin in the first snap pass

Separate validation of both exported meshes confirms that all 173,840 unrepaired and 173,939 repaired cells are distinct and exactly positive. Their forbidden intersections therefore survive checks stronger than the existing floating-point inversion count.

A temporary bounded-region trace around one affected rim segment records the lattice-cut source and every snap pass. The exact audit finds zero forbidden contacts in the initial 588-cell region and eight after the first 592-cell snap pass. Those eight persist through all later captured passes. Every snapshot contains distinct exactly positive cells. This locates a real embedding failure before reconnect or boundary repair. The temporary region selector and trace helper were removed from `tetmesh.rs` after the run. Raw captures and results are `/private/tmp/issue46-contact-stages.log` and `/private/tmp/issue46-contact-stage-results.log`.

Independent review of the proposed concave nearest-patch correction confirms a separate point-projection bug, but gives a counterexample to treating surface vertices as a conforming boundary. Three vertices on the two faces of an L can bridge the air notch while passing the current sampled error and quality limits. Accordingly, any snap correction remains unqualified until its full contact, rim-chain, fidelity, curved-body, and boss-quality gates pass.


## Reentrant projection regression and candidate correction

The focused test `reentrant_snap_preserves_a_point_already_on_the_wall` fails before the correction with actual `[0,0,0]` versus expected `[0,0,0.05]`. This is an exact wall point of the trimmed L-shaped body, moved unnecessarily onto its concave rim. The log is `/private/tmp/issue46-concave-red.log`.

The candidate retains the active-face selection and intersection loop. For exactly two converged, non-tangent faces, it recovers intrinsic normals and samples four signed sectors in their dual basis. Only an unambiguous reentrant sign pattern chooses the nearest original trimmed-patch foot. All other cases retain the previous projection. The foot must also lie on the actual body. `FaceHit::surface_normal` now owns the existing fail-closed normal re-query and is reused by boundary repair without changing public fields.

The first candidate library run passes 246 tests with seven ignored in `/private/tmp/issue46-concave-lib.log`. It includes wall preservation, nearest-patch choice, hit-order reversal, idempotence, an oblique dihedral, ambiguity, nonfinite SDF and unconverged-intersection cases. Independent read-only review finds no production correctness blocker and requests unaffected-branch comparisons and one misplaced comment fix. Those changes are included in the next library run, still pending here.

The same supporting-side mistake also rejected an exact repair wall slide away from a concave trim. The candidate now trusts certified actual-body membership rather than the neighbour's supporting-side sign at such a point. It retains the outside-depth ceiling, other-face clearance, face projection checks and final surface transaction guard. A focused wall-slide test also checks convex outside crossing and nonfinite SDF refusal. Its verification and independent review are pending at this checkpoint.

Neither correction is a boss acceptance result. A point-level projection fix still requires whole-mesh contact and surface-complex proof.


The expanded candidate run `CARGO_INCREMENTAL=0 cargo test -p fea-core --lib --locked --offline -j1` passes 248 tests with seven ignored. Its log is `/private/tmp/issue46-concave-contracts-lib.log`. Added comparisons use the original snap implementation as a test reference and compare floating-point bits for convex trimmed corners, bore/end intersections, tangent planes, a sphere, three-face corners, and ambiguous sector fallback. The oblique reentrant case also reverses hit order. The wall-slide test passes, and a separate independent source review finds no correctness blocker. This remains local contract evidence while the full boss rerun is pending.

## Whole-boss embedding passes; boundary manifoldness remains open

With the reentrant correction, the explicit connected-boss quality assertion passes. A captured repaired mesh has 173,875 cells, zero cells below 0.1 and minimum scaled Jacobian 0.12013473401847495. The full exact pair audit reports zero forbidden contacts both before and after repair, over 10,435,658 and 10,413,169 broad-phase pairs respectively. Both exports have distinct, exactly positive cells. The raw logs are `/private/tmp/issue46-concave-contact-before.log` and `/private/tmp/issue46-concave-contact-after.log`.

The focused integration run passes 16 tests across `amr_in_place`, `high_order_convergence`, `tetmesh`, and `tetmesh_repair`, with five ignored. The cylinder convergence rows are Tet4 maximum/RMS radial errors 0.04627/0.03159 at h=0.40 and 0.01708/0.01076 at h=0.28; Tet10 rows are 0.009099/0.006463 and 0.001894/0.001097. These are measured current-tree results in `/private/tmp/issue46-concave-integrations.log`, not yet a paired baseline comparison.

A separate boundary edge/vertex-link audit prevents acceptance. Before repair there are 62 edges with four boundary triangles and 124 noncyclic vertex links. After repair there are two such edges and four bad vertex links. Both meshes have one boundary component and Euler characteristic two, demonstrating why those aggregate counts cannot certify manifoldness. The two remaining edges lie at x=6,z=1, spanning y=[0.7142857142857142,0.8] and [3.2,3.2857142857142856]. Details are in `/private/tmp/issue46-repaired-pinches.json`. The asserted boss regression now also checks boundary edge incidence and connected cyclic vertex links.

Separately, a reduced nine-cell source proves the old collapse's vertex-only link equality is insufficient. The actual Rust regression `collapse_rejects_a_shared_ghost_edge_and_preserves_the_mesh` fails its refusal assertion before the fix, confirming that the old implementation accepts a topology-damaging collapse. The source has exactly positive, embedded tetrahedra and a manifold boundary; the target remains embedded but acquires a boundary pinch. The missing link simplex is the edge `{0,GHOST}`. See `/private/tmp/issue46-full-link-red.log` and the independent review `/private/tmp/issue46-full-link-verdict.md`.

The candidate now compares full simplicial links, including all nonempty complement subsets of endpoint-star cells and physical boundary triangles coned to the outside ghost. The complete endpoint region is retained without truncation. Independent source review finds no correctness blocker. Library verification and the next whole-boss run are pending at this checkpoint. These corrections do not yet qualify the boss or advance a later plan gate.

The full-link boss rerun retains zero sub-gate cells and minimum scaled Jacobian 0.12013473401847495, now with 173,879 cells. Its asserted test fails the new manifold-edge condition, with the same two bad boundary edges and four bad vertex links. `/private/tmp/issue46-full-link-boss.log` records this failure. The temporary export helper was removed from the integration test after capture.

A paired standalone Rust harness compares the committed `HEAD` tetmesh implementation with the current library on the exact `high_order_convergence` problem at h=0.40 and h=0.28, for Tet4 and Tet10. It asserts bit-identical maximum and RMS radial-error values and equal element/DOF counts. The comparison passes in `/private/tmp/issue46-paired-high-order.log`. The baseline adapter only reuses the current unchanged-field `FaceHit` type and supplies the new optional repair callback as `None`; these cases leave repair disabled. Baseline module, test copies and harness are `/private/tmp/issue46-baseline-tetmesh.rs`, `/private/tmp/issue46-baseline-high-order.rs`, `/private/tmp/issue46-current-high-order.rs`, and `/private/tmp/issue46-baseline-harness.rs`. This is numerical preservation evidence; no speed comparison is claimed.

The source investigation separately confirms that `snap:false, repair:false` already yields all 62 nonmanifold edges. Initial-cut and corrected snapped-unrepaired cell connectivity are identical. The regularization prototype remains outside the repository until it demonstrates end-to-end fidelity, quality and manifoldness. No later qualification gate advances.

The final full-link library run passes 250 tests with seven ignored, including both the refusal regression and a safe interior subdivision collapse that preserves the original tetrahedron's boundary. `/private/tmp/issue46-full-link-final-lib.log` records the run.

An external-only initial-cut prototype removes the smaller disconnected tetrahedral sectors at pinched edges, deleting 124 cells before snapping. The prototype reaches 174,086 repaired cells with minimum scaled Jacobian 0.12008401907826168 and zero sub-gate cells, bad boundary edges, or bad boundary vertex links. It has one boundary component and Euler characteristic two. This candidate is not integrated. Current measured limits include a slightly larger analytic-volume error and some negative boundary-normal/owner alignment. The whole-mesh exact-contact audit and a canonical-owner review are pending. Raw candidate artifacts are `/private/tmp/issue46-pruned-tetmesh.rs`, `/private/tmp/issue46-prune-full-prototype.log`, `/private/tmp/issue46-pruned-fidelity.log`, and `/private/tmp/issue46-pruned-contact.log`.

The sector-removal prototype passes the full default quality report, including largest edge ratio 6.054394107672636 and neighbouring size jump 1.7435940022147527. Its exact whole-mesh audit confirms zero forbidden contacts. The independent source review is `/private/tmp/issue46-source-manifold-review.md`.

The remaining back-facing cylinder triangle exists before snapping, with owner-normal alignment -0.23938616921131137. Snapping worsens it to -0.9703178147284399, and repair improves it to -0.45163571901472410. Its complete 54-cell source star, both initial and snapped coordinates, is `/private/tmp/issue46-fold-complete-source-star.json`.

Two external-only queue experiments revisit cells incident to negative-alignment surface faces while retaining the current move acceptance checks. The first adds one slide but leaves one negative face. A second also allows the existing bounded vertex pattern search to run past its usual quality stopping value on such stars. It still leaves one negative face. Neither experiment is integrated. Their logs are `/private/tmp/issue46-surface-selector-first.log` and `/private/tmp/issue46-surface-selector-unbounded-search.log`; the latter name refers only to removing the quality early-exit, since the existing iteration and step bounds remain in place.

A further surface-cavity audit is open. Collapse evaluates fidelity and removed-point deviation only on newly added cells. A deletion-only collapse has no new cells and is refused by an infinite deviation. It may also expose physical boundary faces on surviving cells, which require inclusion in the surface transaction certificate. An external candidate evaluates the complete old endpoint region plus replacement cells. Its runtime result and independent review are pending.

The complete affected-boundary collapse experiment clears the inward face using the existing operations. With the surface work queue selecting alignment below the guard's existing 0.5 floor, the external candidate ends with minimum alignment 0.57281360747391752, no bad surface candidates, no sub-gate cells, and no splits or off-surface relaxation. It has 174,043 cells, passes the full default quality report, and has zero bad boundary edges or vertex links. Maximum sampled global and owner-trim distance falls to 0.03492922925695235. Its volume is 61.54609673038943640. This is still an external candidate pending its exact full contact audit and independent review. Logs are `/private/tmp/issue46-surface-selector-half.log`, `/private/tmp/issue46-surface-selector-half-fidelity.log`, and `/private/tmp/issue46-surface-selector-half-contact.log`.

The focused actual-body regression `collapse_checks_the_boundary_exposed_on_surviving_cells` is now in the test module. It creates a thin ear on a surviving tetrahedron and requires a valid collapse to expose the surviving base face. The pre-fix run is compiling into `/private/tmp/issue46-exposed-boundary-red.log`. The production collapse has not yet received the complete-region surface change. A separate external guard experiment also requires the set of CAD owners on removed boundary keys to equal the set on added keys, to avoid admitting a deleted CAD patch through the new-face branch.
