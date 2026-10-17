# Issue 46 — Repository completion checklist

Checked lines are delivered scoped increments, with evidence committed in this repository. Broad work-package exit gates remain unchecked until their complete acceptance criteria pass. Phase numbers count implementation increments, not completed priority tiers. Use the [live qualified state](issue-46-current.json) for exact source/test/PR/integration state, the [support matrix](issue-46-native-support-matrix-2026-10-16.md) for detailed scope, and the [resume guide](issue-46-resume.md) for Claude Code continuation. GitHub epic #46 and child checklists carry the corresponding progress.

## P0 foundations

- [x] #47: Source capability/dependency/license audit and candidate investigations recorded ([Phase 0](issue-46-phase-0-audit.md), [Phase 11](issue-46-phase-11.md)). Truck evaluation failures/limitations preserved; no kernel selected on claims alone.
- [x] #48: Backend-neutral CAD boundary, retained OCCT adapter, capability/refusal reporting and regression foundations ([Phase 1](issue-46-phase-1.md)).
- [x] #49: Robust 3D predicates, transactional tetrahedral topology/rollback tests and bounded reference point-set tetrahedralization ([Phase 1](issue-46-phase-1.md)).
- [x] #50/#64: Initial opt-in independently verified telemetry/store and deterministic compatibility registry pilot ([Phase 1](issue-46-phase-1.md)).
- [ ] Full #47–50/#64 exit acceptance: representative corpus/quality/memory/runtime baselines, full capability/formulation/budget/version matrix and complete isolation/persistence/target-platform qualification.

## P1 native CAD, constrained meshing and quality

- [x] #51: Bounded STEP syntax/references/units, source body/analytic geometry and sampled trim/surface diagnostics; explicit reported conservative repair subsets ([Phases 2–10](issue-46-native-support-matrix-2026-10-16.md)).
- [x] #51: Exact-source identities and sampled CAD-to-volume boundary/selection metadata bridges, including scoped Tet4/Tet10 reference mechanics ([Phases 11–16](issue-46-native-support-matrix-2026-10-16.md)).
- [x] #51: Positive-weight rational spline curve/surface evaluation, differentials, source bindings, whole-domain enclosures and bounded distance foundations ([Phases 19–40](issue-46-native-support-matrix-2026-10-16.md)).
- [x] #51: Bounded source p-curve declarations/maps and scoped whole-locus agreement for all 228 bracket associations, including closed quadratic-circle single-winding checks ([Phases 41–47](issue-46-native-support-matrix-2026-10-16.md)).
- [x] #51: Source spline-surface composition, world-distance brackets and reproduced exact-domain-boundary arithmetic correction ([Phases 48–49](issue-46-native-support-matrix-2026-10-16.md), PRs #76–77).
- [x] #51: Bounded quadratic sampler work reduction at unchanged tolerance: 4094 → 32 quarter-circle segments ([Phase 50](issue-46-phase-50.md), PR #78).
- [x] #51: Explicit finite spline interval sampling and checked source vertex endpoints/sense, with raw parameter/unit preservation ([Phase 51](issue-46-phase-51.md), PR #79).
- [x] #51: Shared plane/cylinder/spline world-query dispatch; all 228 bracket p-curves and independent physical-distance tests ([Phase 52](issue-46-phase-52.md), PR #80).
- [x] #51: Discovery of finite p-curve endpoint parameters, oriented arc selection (interior/wrapping) and finite segments/arcs for all 228 bracket edge associations ([Phases 54–57](issue-46-phase-57.md), PRs #82–#85).
- [x] #51: Explicit finite p-curve endpoint correspondence with outward source conversion and hard work/cancellation gates ([Phase 53](issue-46-phase-53.md)); source `de5ceed` qualified at 185 CAD-enabled / 168 feature-off tests and app compile. Integration status is in the live pointer.
- [x] #52: Immutable validated single-shell sampled PLC input, preserved constraints/source metadata and limited strictly star-shaped conforming seed ([Phases 13–14](issue-46-native-support-matrix-2026-10-16.md)).
- [x] #53: Existing solver quality/conformity gates reused; thin/near-boundary refusal regressions and explicit quality-safe star-kernel trial ([Phases 15–18](issue-46-native-support-matrix-2026-10-16.md)).
- [ ] #51: General spline B-Rep, finite/master/oriented trim and exact body agreement, production healing/assemblies/metadata and persistent associations.
- [x] #52: Incremental exact-predicate Delaunay engine and conforming Steiner recovery for well-conditioned closed PLCs; the NEMA17 sampled shell is a documented non-convergent case ([Phase 58](issue-46-phase-58.md)).
- [ ] #52: General constrained segment/facet recovery, cavities/regions/components and constrained refinement with boundary conformity.
- [x] #53: Radius-edge Delaunay refinement of recovered cells for well-conditioned PLCs, with exact volume/boundary checks ([Phase 59](issue-46-phase-59.md)); slivers and sampled surface shells remain open.
- [x] #53: Interior vertex smoothing alternated with flips raises the worst radius ratio of random clouds ~20-30x over flips alone ([Phase 65](issue-46-phase-65.md)); boundary slivers, Steiner exudation and sampled shells remain open.
- [x] #53: Planar boundary-vertex sliding extends the smoothing to boundary-adjacent slivers on flat faces ([Phase 66](issue-46-phase-66.md)); curved shells, edge sliding and Steiner exudation remain open.
- [ ] #53: General sliver handling/flips/smoothing/remeshing and representative solver-conditioning/accuracy/cost acceptance.
- [x] #54/#64: Registry v2 covers the native constrained strategy with an input-angle gate and stable rejection codes ([Phase 61](issue-46-phase-61.md)).
- [ ] #54: Full deterministic strategy generation/selection and end-to-end qualification beyond the early registry pilot.

## P2, P3 and final gates

- [x] #55: Rigorous Bernstein-bound validity of Tet10/Tri6 maps, mesh-level check, and the mesher's sampled guard hardened ([Phase 64](issue-46-phase-64.md)); exact CAD projection and convergence evidence remain open.
- [ ] #55: High-order curved CAD-conforming elements (exact projection, convergence, cost).
- [ ] #56: Verified structured–unstructured integration and transitions.
- [x] #57: Validated SPD metric tensors, metric-space lengths, log-Euclidean interpolation, intersection, gradation, curvature- and error-field-driven sizes and complexity/budget control, all with analytic tests ([Phase 62](issue-46-phase-62.md)). No mesher consumes the field yet, so the gate stays open until #58 integrates it.
- [ ] #57: Verified spatial metric fields consumed by a mesher.
- [x] #58: Metric-driven refinement by terminal-edge splitting and metric-quality flips, conforming and geometry-preserving, converging for isotropic and mild anisotropic fields ([Phase 63](issue-46-phase-63.md)); strong anisotropy, coarsening and curved-boundary projection remain open.
- [ ] #58: Metric-driven anisotropic adaptation (unit-metric meshes at strong anisotropy).
- [ ] #59: Mesh motion/ALE and consistent state/history transfer.
- [ ] #60: Solver-aware cost models and advanced optimization.
- [ ] #61: Optional learning with representative verified training data, held-out/OOD checks and deterministic fallback.
- [ ] #62: AMR feedback and production autotuner/time-to-accuracy qualification.
- [ ] #63: Verified native production parity and separately approved OCCT retirement. OCCT remains the production backend.

Detailed phase reports/manifests preserve exact commands, commits, failures and limitations. Tracking-only updates reuse valid source qualification and trigger no hosted CI. The root NEMA17 fixture has unknown provenance and declares millimetres. Sampled geometry, parsing, endpoint checks and distance bounds are never promoted to unproved production CAD or mesh capabilities.
