# Native Rust CAD, advanced meshing and autotuning — execution index

Parent epic: [#46](https://github.com/Teased-oChroid-orrA/powershell_tool/issues/46). Working branch: `feature/native-cad-advanced-meshing`.

## Source and precedence

- [Original epic, verbatim](epic-native-cad-advanced-meshing-source.md), sections 1–51.
- [Pillar 7 addendum, verbatim](epic-native-cad-autotuning-source.md), sections 52–68.
- The user's revised priority order below governs execution where the original phase order differs. Original section numbers remain stable for traceability.

Both source documents are proposals. Their algorithms, module paths, dependency candidates and illustrative interfaces require comparison against the actual repository; they are not claims of existing capability. Audit before adding dependencies or changing implementation. No new mesher or learning subsystem is implemented by this planning milestone.

## Work packages and dependencies

GitHub native sub-issues and blocked-by links provide live progress. This table is the initial dependency baseline; record later changes as dated addenda rather than rewriting historical milestones.

| Issue | Priority | Work package | Prerequisites |
|---|---|---|---|
| [#47](https://github.com/Teased-oChroid-orrA/powershell_tool/issues/47) | P0 | Repository capability, dependency and license audit | Merged reliability baseline |
| [#48](https://github.com/Teased-oChroid-orrA/powershell_tool/issues/48) | P0 | Isolate CAD backends behind capability-aware interfaces | #47 |
| [#49](https://github.com/Teased-oChroid-orrA/powershell_tool/issues/49) | P0 | Robust 3D predicates and transactional mesh topology | #47 |
| [#50](https://github.com/Teased-oChroid-orrA/powershell_tool/issues/50) | P0 | Start verified workload telemetry and bounded benchmark storage | #47 |
| [#64](https://github.com/Teased-oChroid-orrA/powershell_tool/issues/64) | P0 | Early deterministic compatibility registry | #47 |
| [#51](https://github.com/Teased-oChroid-orrA/powershell_tool/issues/51) | P1 | Native B-Rep exchange, validation, healing and persistent associations | #48, #49 |
| [#52](https://github.com/Teased-oChroid-orrA/powershell_tool/issues/52) | P1 | Constrained 3D tetrahedralization and boundary recovery | #49, #48 |
| [#53](https://github.com/Teased-oChroid-orrA/powershell_tool/issues/53) | P1 | Sliver elimination, mesh validity and solver conditioning | #52 |
| [#54](https://github.com/Teased-oChroid-orrA/powershell_tool/issues/54) | P1 | Deterministic compatibility registry and strategy selection | #50, #48, #64 |
| [#55](https://github.com/Teased-oChroid-orrA/powershell_tool/issues/55) | P2 | High-order curved CAD-conforming elements | #51, #53 |
| [#56](https://github.com/Teased-oChroid-orrA/powershell_tool/issues/56) | P2 | Integrate structured and unstructured meshes with verified transitions | #53, #54 |
| [#57](https://github.com/Teased-oChroid-orrA/powershell_tool/issues/57) | P2 | Validated metric tensors and size/complexity budgets | #53 |
| [#58](https://github.com/Teased-oChroid-orrA/powershell_tool/issues/58) | P2 | Metric-driven anisotropic adaptation | #57, #51 |
| [#59](https://github.com/Teased-oChroid-orrA/powershell_tool/issues/59) | P3 | Mesh motion, ALE and consistent state/history transfer | #58, #56 |
| [#60](https://github.com/Teased-oChroid-orrA/powershell_tool/issues/60) | P3 | Solver-aware cost models and advanced optimization | #53, #54, #50 |
| [#61](https://github.com/Teased-oChroid-orrA/powershell_tool/issues/61) | P3 | Optional confidence-aware performance learning | #60 |
| [#62](https://github.com/Teased-oChroid-orrA/powershell_tool/issues/62) | P3 | AMR-informed recommendations and production autotuner qualification | #54, #60, #58 |
| [#63](https://github.com/Teased-oChroid-orrA/powershell_tool/issues/63) | Final | Verified production parity and explicitly approved OCCT retirement | #51, #53, #55, #56, #58, #59, #60, #61, #62 |

The registry starts with the isolation work, using only audited existing capabilities; the completed selector grows as compatible mesh strategies mature. Telemetry starts alongside isolation and does not wait for a new tetrahedralizer. Robust predicates/topology precede constrained 3D operations. Native import/validation/healing and CAD-boundary recovery must converge before claiming reliable meshing of real solids. Sliver/validity checks gate production use of the new volume mesher. High-order, hybrid and anisotropic work follow the foundations; motion/ALE and advanced optimization follow mature infrastructure. OCCT retirement is last and requires separate explicit approval.

Learned predictions require representative verified mesh-generation and solver-performance data, held-out geometry families, leakage prevention, uncertainty/OOD handling and a deterministic baseline. Existing deterministic cost tools can be audited/reused early; implementing new learned models is deferred. AMR independently establishes resolution adequacy and cannot be suppressed for a predicted speed target. Production selection does not perform hidden exploratory solves.

## Phase 0 source inspection starting points

This is a source-inspected orientation, **not a completed capability/license/performance audit**. Issue #47 must produce and verify the complete matrix and dependency/risk report before implementation.

| Area | Observed entry points | Audit requirement |
|---|---|---|
| OCCT dependency | `solid-kernel/Cargo.toml`: optional `cadrum 0.9`; `occt` and `occt-source`; propagated by `fea-problem` and `app-tui` | Inventory all APIs/callers, license/build/platform constraints; retain reference adapter |
| CAD boundary | `solid-kernel/src/occt.rs`, `fea-problem/src/solid.rs` | Backend-neutral import/evaluation/modelling contracts, units and persistent identity |
| Surface/volume bridge | `solid-kernel/src/lib.rs`: `SurfaceSolid`, `mesh_volume` | Preserve closed-manifold checks, tags and geometry-to-FEA associations |
| Existing 3D mesher | `fea-core/src/tetmesh.rs`: signed-distance lattice, longest-edge refinement, centroid classification and guarded snapping | This is not constrained 3D Delaunay/PLC recovery; quantify feature and boundary limits |
| Existing predicates | `fea-core/src/delaunay.rs`: `robust::orient2d/incircle`; existing `robust` dependency | Audit 3D predicate reuse and degeneracies; no assumption 2D support proves 3D topology |
| Elements | `fea-core/src/element.rs`: Tri3/6, Quad4/8/9, Tet4/10, Hex8/20/27, Wedge6/15 | Cross-check physics/contact/order combinations; no pyramid family appears in this enum |
| Hybrid/quality | `fea-core/src/meshing/{hybrid,adjacency,quality,weld,smooth,multimaterial}.rs`, `sweep.rs` | Reuse verified infrastructure from #25; distinguish 2D/extruded support from arbitrary 3D transitions |
| Cost and size prediction | `fea-core/src/meshing/{cost,predictor}.rs`, `hybrid::mesh_best_2d` | Existing symbolic cost/rate calibration and deterministic 2D size prediction are reuse candidates, not new ML |
| AMR/transfer | `fea-core/src/adaptivity/{controller,report,transfer,transaction}.rs` | Audit cycle timing, QoI reliability, factor-byte estimates, state/history transfer coverage; factor bytes are not process peak memory |
| Derivatives | `fea-core/src/derivative/`, #39 and follow-up #41 | Preserve existing verified scopes; do not infer general differentiability across topology changes |

## Baseline and known limits

Reuse [the reliability handoff](workbench-reliability-2026-10-15.md) and [clean qualification report](workbench-qualification-linux-2026-10-15.json). At frozen application-source commit `99d33aff2b85a835618e6bbd03502fa8e03e4064`, the CAD-enabled workspace passed 2,569 tests across 137 targets (86 intentionally ignored); terminal-only compilation and 9 explicit software-Vulkan checks passed. Later reliability commits change CI/docs only. Record final hosted results and merge SHA in the transition handoff; do not invent a fresh baseline or call ignored soaks passed.

The root `nema17-motor-bracket.step` has unknown provenance/material and declares millimetres. Default Tet10 meshing yields 318,906 nodes / 204,552 elements; four tiny faces remain unresolved, reported and guarded against boundary assignments. Preserve this real CAD regression. Do not silently heal away small features or weaken geometric/numerical acceptance. Linux software graphics and hosted tests do not prove macOS/Windows hardware or packaged startup.

## Evidence, handoff and CI budget

For each milestone record issue, exact commit/PR, files/modules, supported scope, test commands/exits, numerical accuracy, mesh quality, timings/memory, failures, limits, dependencies and next tasks. Keep validation and failure records distinct from successful training examples. Use versioned local bounded storage, verification evidence from the numerical pipeline, optional collection and export/reset/deletion; omit proprietary geometry by default.

Use focused local checks first. Full regression runs are justified at integration/release gates or after changes with broad impact, not after issue edits. Do not dispatch hosted CI for this documentation-only planning milestone. Existing workflows run qualification on relevant PR changes and release builds on tags/manual dispatch; pushing a branch without a PR does not require a hosted run. Reuse successful checks; cancel only obsolete runs belonging to this work. Do not alter acceptance tolerances or hide failures to save minutes.

The current authorization merges only reliability PR #45 and creates this feature branch/tracking. Future implementation remains on the dedicated branch pending a separate merge decision. OCCT retirement additionally requires its explicit final acceptance review.

## Verified diagnostic progress snapshot — 2026-10-16

Phases 1–17 now deliver audited foundations, bounded native analytic STEP/sampled geometry, explicit reported repairs, exact-document references, regenerated source selections, immutable PLC admission and a strictly certified limited conforming seed/solver bridge with one guarded interior quality trial. [Machine-readable snapshot](issue-46-progress-2026-10-16.json), [support matrix](issue-46-native-support-matrix-2026-10-16.md) and [Claude handoff](issue-46-claude-handoff.md) distinguish verified scope from the still-open epic gates. Latest qualified source bfc91ef passes 70 CAD-enabled / 53 feature-off tests; source 93f0fb6 has the latest app compilation evidence. No new hosted CI or production switch. Main is unchanged at 8ba27a2; feature merge and OCCT retirement still require separate approval.

General CAD/assembly/exact trims, general constrained recovery/refinement/cavities/regions, general sliver elimination/conditioning, P2/P3/final parity remain open. The small generated pilot corpus is not representative learning evidence. No issue closure or production availability follows from these diagnostic increments.

Phase 21 snapshot: [current machine-readable progress](issue-46-progress-phase-21.json), [declaration mapping](issue-46-phase-21.md) and [unit-underflow reproduction/fix](issue-46-phase-20.md). Source 7e2d7a8 passes 83 CAD-enabled/66 feature-off checks and app compilation. Rational curve evaluation/declaration extraction does not expand full native body support; sampling/surface/trim and all broader CAD/PLC/quality/P2/P3/final gates remain open. No additional hosted CI, production switch or merge/retirement approval.

Phase 30 snapshot: [current progress](issue-46-progress-phase-30.json) and [handoff](issue-46-claude-handoff.md). Whole-interval curve sampling, global curve-distance brackets and tensor patch enclosures are verified diagnostic APIs; full native spline-body import remains refused. Latest source d3e81dc passes 121 CAD-enabled / 104 feature-off tests and app compilation. Broader gates/approval boundaries remain unchanged.

User-approved transition (2026-10-16): PR #65 merged at 37cf932, old branch deleted; continuation on `feature/native-cad-spline-import`. [Phase 31](issue-46-phase-31.md) / [snapshot](issue-46-progress-phase-31.json). Foundation merge is authorized/completed; OCCT retirement remains unapproved and unimplemented. Prior no-merge statements above are historical.

Phases 32–35: [snapshot](issue-46-progress-phase-35.json), source 2042c2d, 130 CAD-enabled / 113 feature-off tests plus app compilation. Only the verified full-domain linear spline class expands native body support; all general CAD/trim/PLC/quality/later gates stay open. User authorization includes merging completed increments and continuing; OCCT is retained.

Completed increment PR #66 merged at 3aebf99 under the user authorization; old branch deleted. Continue on `feature/native-cad-planar-spline-trims`. [Phase 36](issue-46-phase-36.md) / [current snapshot](issue-46-progress-phase-36.json): finite exact planar bilinear faces qualified, p-curve/master agreement and remaining epic gates open. OCCT remains production.

PR #67 merged at 75e4faa; old branch deleted. Current execution: `feature/native-cad-pcurve-validation`, [Phase 38](issue-46-phase-38.md), [snapshot](issue-46-progress-phase-38.json), 139 CAD-enabled / 122 feature-off tests and app compilation. P-curve declarations now decode; their mapping/master semantics and all broader roadmap gates remain open.

Current execution pointer: [issue-46-current.json](issue-46-current.json). Phases 39–40 qualify source-plane transforms and 148 finite plane/line p-curve loci at 1e-7 mm; 145 CAD-enabled / 128 feature-off tests plus app compilation pass. Other p-curve/master/general CAD/PLC/later gates remain open. PR #68 stays draft on `feature/native-cad-pcurve-validation`; completed PRs #65/#66/#67 are merged.

- [Phase 41: whole-interval planar spline p-curve bounds](issue-46-phase-41.md): source/budget guards and explicit unit qualification.

- [Phase 42: bounded source cylindrical parameters](issue-46-phase-42.md): explicit radian/length semantics and shared source placement; trim/master gates remain open.

- [Phase 43: unsigned planar circle carrier agreement](issue-46-phase-43.md): 13 whole-carrier checks; finite arc/master/body gates remain separate.
