# Issue 46 — Claude Code handoff

## Transition, 2026-10-16 UTC

Reliability PR [#45](https://github.com/Teased-oChroid-orrA/powershell_tool/pull/45) merged with explicit user authorization at **`8ba27a2fd21b88f7790d3ac2f530b498e3b031c2`**. Its head was `6f76dbb9eb2cd224a3a611b7e8bbeb69c5c5cab1`. The merged `codex/workbench-reliability` branch was removed locally and remotely. The new **`feature/native-cad-advanced-meshing`** branch starts at that exact merge commit. Do not merge this new epic without a subsequent decision.

Parent [#46](https://github.com/Teased-oChroid-orrA/powershell_tool/issues/46) has **18 native GitHub sub-issues (#47–64)** with blocked-by dependencies and acceptance checklists. Read [the execution index](issue-46-execution-plan.md) and [machine-readable initial tracking](issue-46-tracking.json). The original [epic](epic-native-cad-advanced-meshing-source.md) and [Pillar 7 addendum](epic-native-cad-autotuning-source.md) are preserved byte-for-byte. Their SHA-256 hashes respectively are `b28c41af3e0be55110e81b1564af5d92246c7c304b79d2d65e0b40ea9259d88c` and `8fac71c8ddbb512cd0ab2ee62b307aa152c4bbdc0e70b6930248e87730ba48de`.

## Current evidence and outstanding validation

The complete clean local application-source qualification passed: **2,569 tests, zero failures, 86 intentionally ignored**, terminal-only compilation, and **9 explicit software-Vulkan checks**. See [the immutable report](workbench-qualification-linux-2026-10-15.json) and [reliability handoff](workbench-reliability-2026-10-15.md) for commands, exact source revision, earlier failures and corrections. The only subsequent reliability changes were CI/docs. No application, dependency or test source changes occur in this new planning commit, so no redundant test/build run is needed.

[Hosted final-revision qualification](https://github.com/Teased-oChroid-orrA/powershell_tool/actions/runs/38850003423) was still running at merge; **it is not claimed as passed**. GitHub accepted the normal merge without a gate override. Read live checks before making cross-platform claims and investigate any actual failure. Earlier Linux/macOS default-feature checks passed; the obsolete earlier run was cancelled to avoid duplicate Windows usage. Do not rerun successful local qualification or dispatch new CI simply to create tracking. Hosted Windows CAD, macOS/Windows hardware graphics and clean-machine packaged startup remain unverified. Historical PR/issue comments may describe the prior no-merge authorization; this dated transition records the user's later explicit merge approval.

## Start here

1. Read root CLAUDE.md, relevant AGENTS.md, the execution index and issue #47. Preserve existing work; no new worktree is needed.
2. Finish the actual Phase 0 capability/license/dependency/risk audit. The source-linked orientation table is **not** a completed audit, fresh performance baseline or kernel selection.
3. Lead with CAD backend isolation (#48) and robust predicates/topology (#49). Start verified workload telemetry (#50) and the early compatibility registry (#64) alongside isolation, using only capabilities confirmed by the audit.
4. Native B-Rep import/validation/healing (#51), constrained 3D boundary recovery (#52) and sliver/mesh validation (#53) precede advanced high-order/hybrid/anisotropic work. Preserve the OCCT reference adapter.
5. Audit/reuse existing cost, hybrid, AMR timing/transfer and quality infrastructure. Defer learned predictions until representative verified benchmarks exist. AMR remains independent and authoritative; no hidden multi-solve exploration.
6. Record each milestone's commit/PR, exact tested revision, files/modules, applicable commands/exits, accuracy, quality, time/memory, failures, limits and next tasks on its issue and in a new append-only phase log.
7. OpenCascade retirement (#63) is final, with full committed-workflow parity and separate explicit approval. The reliability merge authorization does not authorize retirement or future feature merges.

## Local environment

Use the installed toolchain and caches, not a new install:

```sh
export CARGO_HOME=/workspace/toolchains/cargo
export RUSTUP_HOME=/workspace/toolchains/rustup
export PATH="$CARGO_HOME/bin:/workspace/toolchains/link-tools:$PATH"
export OCCT_ROOT=/workspace/toolchains/occt/occt-8_0_1_rev3-x86_64_unknown_linux_gnu
export CARGO_INCREMENTAL=0
cd /workspace/powershell_tool
```

For a justified full integration check: `python3 scripts/qualify.py --cad --jobs 1 --output /workspace/qualification.json`. Check disk space first. For targeted numerical changes run the relevant existing tests and analytical/dimensional acceptance cases; never weaken tolerances. Actual UI changes need a real native window/screenshot, not compilation alone. Software-Vulkan and Xvfb setup is in the reliability handoff/saved environment instructions. Keep setup scripts separate from authorized application edits.

There is no new epic implementation yet. No new PR/hosted run is needed for this documentation-only branch initialization. The user explicitly requests GitHub usage restraint. Future CI must be justified by a required check, meaningful cross-platform risk or release gate; use local focused checks and existing evidence first.

## Phase 1 implemented — 2026-10-16

The initialization-only status above is superseded by [Phase 1](issue-46-phase-1.md) and [its verification record](issue-46-phase-1-verification.json). Clean application-source revision `2677514` passed 53 meshing checks, five derivative checks, the explicit scaling benchmark, 14 solid-kernel checks, 45 problem-layer checks, CAD-enabled consumer compilation and CAD-feature-off compilation. The one targeted Windows foundations run **38896885191 passed**. The earlier Windows reliability failure was a nonportable timing-order assertion, corrected here with numerical work/accuracy checks rather than relaxed tolerances. Main is unchanged by this phase; future merges remain separate.

Current code: neutral CAD boundary and explicit capabilities; exact-sign checked 3D topology; reference point-set Delaunay using real hull insertion (not finite supervertices); quality-gated Tet4 bridge; deterministic compatibility slice; opt-in independently verified, bounded telemetry/pilot collector. The new point-set core remains unavailable for application CAD meshing because PLC boundary recovery is not implemented. A larger benchmark found an incomplete-hull defect during development; it was fixed and promoted into an ordinary regression. Do not resurrect the superstructure implementation from intermediate commits.

Next: finish the native B-Rep/corpus/capability gates (#47/#51), general constrained recovery (#52), then sliver/production quality work (#53). Truck at `88ed005` parsed 38 bracket faces but failed three bounded tessellation probes: two exceeded 4 GiB RSS and one thread exceeded 300 seconds. Keep OCCT; these results do not qualify a native replacement. Do not infer that all Truck geometry fails. The three independently verified pilot templates are not a representative learning corpus; imported records are quarantined. Efficient point location, geometric intersection certification, assembly/entity/healing coverage and the later P2/P3 work remain open.

Use `python scripts/qualify.py --scope foundations --jobs 1 --output /workspace/foundations.json` for the targeted scope. The default remains full workspace qualification. The runner rejects source drift and records scope/start/end revision; do not edit source during a qualification run. Hosted `scope=foundations` dispatches one Windows job, while ordinary PR qualification retains its existing matrix. Do not dispatch CI again for docs or to repeat the successful targeted run. Windows artifact storage was blocked, but job success was verified; older failed-run archive logs were available through the GitHub run archive API, so no extra network domain is required.

## Phase 2 native import foundation — 2026-10-16

[Phase 2](issue-46-phase-2.md) and [verification](issue-46-phase-2-verification.json) add `solid-kernel::{step,step_units}` and a no-OCCT `step-inventory` example. Source `b78068c` passed 23 CAD-enabled solid-kernel checks and the actual native audit command. The bracket has 3,088 retained entities, 38 faces and an assigned 0.001 m/source-unit length context. Integer precision, references, bounded parsing and declared SI/conversion factors are checked; missing/ambiguous/cyclic/unsupported units are refused, not guessed. This code is not connected to production CAD import and does not certify geometry or normalize model coordinates.

Next #51 work: map a canonical geometric/topological B-Rep and representation/body contexts/placements, validate all entity/loop coverage and exact geometry, then qualify bounded native tessellation/healing on the corpus. The negative Truck probes and unrecovered PLC gate still stand. Do not treat syntax/reference success or a declared length factor as a valid solid. No new dependency or extra CI run was introduced in Phase 2.

## Phase 3 raw topology — 2026-10-16

[Phase 3](issue-46-phase-3.md) and [verification](issue-46-phase-3-verification.json) record source `28c630e`: 28 solid-kernel checks pass, including 14 native cases; the default audit example retains the bracket's 70 vertices, 114 edges, 38 faces, 52 loops and 228 uses. `step_topology` has cumulative expansion budgets and rejects broken/incomplete supported bodies. These are connectivity checks, not geometric solid certification. All actual fixture bounds lack explicit outer-loop roles; do not guess them from order.

The audit distinguishes referenced 3D curves (88 LINE / 26 CIRCLE) from auxiliary pcurve declarations. This narrows the next native analytic scope: planes/cylinders and 3D lines/circles, with actual representation/unit association and geometry validation. Assemblies, exact trims, orientability, vertex-manifoldness, healing and geometric intersections remain unverified. No production CAD import is switched; no new CI dispatch was made for this portable read-only increment.

## Phase 4 analytic geometry — 2026-10-16

[Phase 4](issue-46-phase-4.md) and [verification](issue-46-phase-4-verification.json) record source `6e354f8`: 33 CAD-enabled solid-kernel tests pass. Native `analytic_step` supports directly represented single bodies, explicit length context normalization, line/circle edges and plane/cylinder surfaces with bounded shared-edge sampling and analytic consistency checks. The actual bracket produces 470 samples across 114 edges and 38 untrimmed surfaces. Independent OCCT cube/cylinder exports verify normals/chord bounds. Run the no-OCCT diagnostic with `step-inventory FILE.step --analytic`.

Next: classify and validate sampled planar trim loops without guessing bound order, then address periodic cylinder charts, shared-boundary-preserving tessellation and solid validation. Polygonal approximation checks must not be advertised as exact trim-intersection proofs. Production still uses OCCT; no additional CI was dispatched. Unsupported geometry/assemblies fail explicitly. The full epic remains open.

## Phase 5 sampled planar trims — 2026-10-16

[Phase 5](issue-46-phase-5.md) / [verification](issue-46-phase-5-verification.json) record `7a40278`: all 36 CAD-enabled solid-kernel tests pass. `planar_step` validates sampled polygon crossings/touches/overlaps and classifies outer/hole roles by containment, preserving source order. All 28 bracket planar faces pass; an independent OCCT cube gives the exact 52 mm² total planar area within 1e-12. Projection expansion and pair/containment work are bounded. No production CAD switch, new dependencies or additional CI.

Next: cylinder periodic charts/seams, shared-boundary triangulation, global solid checks, then PLC recovery. Do not call polygonal approximations exact curved-trim certificates. Nonplanar faces explicitly fail this API. Keep OCCT and the other P1/P2/P3 gates open.

## Phase 6 cylindrical charts and draft PR — 2026-10-16

Draft [PR #65](https://github.com/Teased-oChroid-orrA/powershell_tool/pull/65) tracks the implemented foundations without closing the epic. [Phase 6](issue-46-phase-6.md) / [verification](issue-46-phase-6-verification.json) record source `05e9f0e`: 40 CAD-enabled and 31 feature-off solid-kernel checks pass. All ten bracket cylindrical faces unwrap as chart-closing single-bound polygons. Independent translated axis-Z cylinder area is 12π mm² within 1e-12. Integer-period reconstruction avoids accumulated unwrapping drift; expansion and classification remain bounded.

Important blocker/limit: a tilted cylinder export fails the strict single-turn chart certificate and is deliberately rejected by regression. Do not relax the bound or silently snap the seam. Explicit uncertainty-aware canonicalization/healing reports are needed to extend this subset. Cylindrical holes, unseamed winding bands, exact trim intersections, triangulation/global solid validity, general assemblies/B-splines/healing and PLC recovery remain open. Preserve shared source identities when implementing chart triangulation. `TrimLoop` is the common polygon type; `PlanarLoop` remains a compatibility alias.

No new CI dispatch, dependency/lockfile or production importer change. Head commits use `[skip ci]` during local diagnostic milestones, as requested; final cross-platform qualification is still a gate. Main remains at `8ba27a2`; do not merge this feature or retire OCCT without the separate approval. All later P1/P2/P3/Final work remains open; Phase 6 is not epic completion.

## Phase 7 native sampled surfaces — 2026-10-16

[Phase 7](issue-46-phase-7.md), [dependency audit](issue-46-phase-7-dependency-audit.md) and [verification](issue-46-phase-7-verification.json) record source `0932371`: 47 CAD-enabled and 36 feature-off solid-kernel tests pass. `chart_mesh` uses pinned MIT-elected spade 2.15.1 for fixed-boundary CDT; `native_surface` assembles all bracket faces with exact shared identities, cyclic vertex links, bounded work and cancellation. Native diagnostic: `cargo run -p solid-kernel --example step-inventory --locked -j1 -- nema17-motor-bracket.step --surface` (no OCCT feature). Default result: 469 vertices / 978 triangles / 38 faces, 474 edge samples, volume 34654.807 mm³. Fine 0.005 mm result converges to 34633.562 mm³ versus OCCT tessellation 34632.794 mm³, passing the unchanged 1e-3 relative criterion. Standalone native process measured ~0.015 s / 9344 KiB RSS, one cloud-host observation; not solver training evidence.

Do not resurrect centroid classification, one-chord short arcs or boundary-only curved error control: development exposed omitted thin regions, duplicate facets, four-way edges and persistent interior chord volume bias. Batched interior refinement now passes the original default work cap. No tolerance/default cap was raised; world boundary samples are never proximity-welded. New MIT notice is shipped; no existing locked versions change.

Next: global sampled triangle self-intersection certification before any production/backend/solver switch, then exact trim/corpus coverage and PLC recovery. A prototype intersection module lives outside the checkout at `/workspace/epic-evidence/triangle_intersection.rs`; it is not integrated or verified yet. Keep all Phase 6 limitations, production OCCT and later epic gates open. Draft PR #65 remains the review vehicle; no additional hosted CI. Current main remains untouched.

## Phase 8 sampled intersections / solver bridge — 2026-10-16

[Phase 8](issue-46-phase-8.md) / [verification](issue-46-phase-8-verification.json) record `f761f2c`: 52 CAD-enabled and 40 feature-off solid-kernel checks, 45 problem consumers and CAD-enabled app compilation passed. `triangle_intersection` consumes a single connected sampled shell into an immutable checked result, using bounded exact-sign facet tests. Bracket: 98,183 candidate pairs pass. The checked distance-solid bridge preserves indices/source faces without welding; both Tet4/Tet10 native STEP-bar tension patches pass the existing 1e-6 relative criterion through the **existing** volume mesher. Legacy normal/BVH construction is shared. No generic PLC claim or production importer switch.

Next: explicit, reported chart canonicalization within the existing geometric tolerance for the tilted-cylinder case; strict default refusal remains unchanged. Broader native exchange/trim/corpus/assembly/healing/persistent-name and later P1/P2/P3 gates remain open. The outside-checkout intersection prototype mentioned above is superseded by the committed module. No new hosted CI, main merge or OCCT retirement.

## Phase 9 explicit chart canonicalization — 2026-10-16

[Phase 9](issue-46-phase-9.md) / [verification](issue-46-phase-9-verification.json) record `8ab1b2b` (feature source `9f55d8e`): 54 CAD-enabled / 42 feature-off tests and CAD-enabled app compilation pass. `ChartPolicy::Strict` remains default. Explicit `CanonicalizeWithinTolerance` fixes the tilted chart using analytic line/circle invariants within the existing 1e-7 mm geometry tolerance, preserving original world samples. Reports include face/bound/oriented-use/edge/sample IDs and before/after parameters plus projection deviation; reports survive the checked distance-solid bridge. `NativeDistanceSolid::into_parts` now returns solid, source-face IDs and chart reports.

Committed generated fixture `solid-kernel/tests/data/tilted-cylinder.step` enables no-OCCT regression. CLI `--canonical-surface` explicitly opts in; 22 adjustments, max deviation 1.4071e-12 mm, 2,275 candidate pairs pass. Strict `--surface` still exits 1 with the single-turn refusal. No tolerance/default-budget relaxation or world/source mutation. No extra dependency, CI or production/main change.

Next P1 healing increment: explicit face-orientation repair (default strict), exact edge-incidence parity propagation, reject nonorientability/nonmanifold/mixed-face flips, choose outward global sign, record every source face-sense change, then retain connectivity/intersection gates. Prototype plan: `/workspace/epic-evidence/phase10-orientation-plan.md`. Do not silently delete/weld faces or claim general healing, assemblies, NURBS, PLC or epic completion. Keep OCCT and separate merge/retirement approval.

## Phase 10 explicit orientation repair — 2026-10-16

[Phase 10](issue-46-phase-10.md) / [verification](issue-46-phase-10-verification.json) record `3b55ed5`: 58 CAD-enabled / 46 feature-off tests and CAD-enabled app compilation pass. Strict remains default; explicit `RepairWithReport` only flips whole source faces after closed-edge parity and positive global volume checks. Refuse nonorientability, mixed-face flips, nonmanifold/disconnected shells; cancellation is atomic. Real corrupted bracket exactly restores original mesh with one reported change, source model unchanged. Normal bracket CLI `--repair-orientation`: zero changes, 98,183 candidate pairs pass. CLI supports combining explicit chart/orientation policies and reports phase work progress.

Draft API update: `NativeDistanceSolid::into_parts` now yields `(SurfaceSolid, NativeGeometryMetadata)` containing source face IDs, chart adjustments and face-sense adjustments. No existing consumer used the earlier tuple. Next: document-scoped source references to guard against applying numeric IDs to a different STEP document; broader native CAD coverage/PLC and later gates remain open. No hosted CI, production/main change or OCCT retirement. Initial missing-SDK-variable failure and interleaved local log are recorded; qualified rerun is clean.

## Phase 11 exact-document associations — 2026-10-16

[Phase 11](issue-46-phase-11.md) / [dependency audit](issue-46-phase-11-dependency-audit.md) record `d5c73a9`: exact accepted STEP source bytes receive a SHA-256 document fingerprint. `EntityReference` pairs it with numeric ID; `Inventory::resolve_reference` rejects mismatched documents and missing IDs. Fingerprint persists through analytic/sampled/checked native geometry to metadata. Real bracket regression resolves all source faces and rejects modified source even with reused IDs. Identical reload is supported, whitespace changes conservatively invalidate; not cross-edit naming or general BC/material/contact/AMR transfer. Reuse locked sha2 0.10.9 with MIT notice, no existing package versions changed.

Next: explicit face-reference remapping across native triangulation regeneration, then broader exchange/assembly/exact trims and PLC recovery; all later epic gates still open. Retain OCCT, strict defaults, no extra hosted CI or main merge. Verification results are appended after complete checks below.

Phase 11 final verification at `d5c73a95e2779fdd56289a050ddb3f0bc1b8814d`: **60 CAD-enabled / 48 feature-off tests**, CAD-enabled app lib/tests compile; all pass, no new failure. [Evidence](issue-46-phase-11-verification.json).

## Phase 12 regenerated face selections — 2026-10-16

[Phase 12](issue-46-phase-12.md) records `98cd85a`: identity-based face lookup remaps stored selections across coarse/fine native triangulations and refuses unknown/missing/mismatched/duplicate source IDs. `NativeVolumeMesh` retains mesh/report/metadata immutably; checked native geometry delegates to the existing volume mesher and resolves source references to nonempty generated boundary regions. Empty/absent regions are explicitly unresolved and enumerable, never rebound. Native bar Tet4/Tet10 tension patches now use source references for loads/supports. No full coverage, exact CAD, AMR/history/material/contact transfer or PLC claim. Source coordinate units remain mm, caller material units must be consistent.

Next P1: validated immutable PLC boundary admission from checked sampled geometry, then bounded constrained recovery; maintain clear distinction from existing convex-hull Delaunay. Broader native exchange/assembly/exact trims and later epic gates remain open. No new dependency, hosted CI, production/main change or OCCT retirement. Final check results are appended below.

Phase 12 final verification at `98cd85a8720c7a70f8eb3fce7030e55beb59ac10`: **62 CAD-enabled / 50 feature-off tests and CAD-enabled app lib/tests compilation pass**, no failures/ignored checks. [Evidence](issue-46-phase-12-verification.json).

## Phase 13 triangulated PLC admission — 2026-10-16

[Phase 13](issue-46-phase-13.md) records `2367cf5`: immutable `ValidatedPlc` consumes a globally checked sampled single shell and retains every original point/facet/source face/report; all undirected triangular edges become explicit constraints. Bracket regression: 469 vertices, 978 facets, 1,467 segments preserved; source references resolve, limits/cancellation refuse. No repeated geometric check, repair or changed production capability. This is input admission only: no constrained tetrahedral recovery, cavity/multi-region support or exact CAD boundary claim.

Next: a strictly certified star-kernel conforming seed could preserve every boundary constraint for a limited shape class before general PLC recovery. It must refuse nonvisible facets/zero-volume cells and keep the bracket/holes limitation explicit; do not advertise a general constrained Delaunay mesher. Broader CAD/exact trims/assemblies and later epic gates remain open; retain OCCT and separate merge/retirement approval. Final check results follow.

Phase 13 final checks at `2367cf573c438a018f25072c0095592c7426c2ed`: **63 CAD-enabled / 51 feature-off tests pass**, no failures/ignored checks. The application compilation evidence remains Phase 12; this additive diagnostic PLC API has no app caller. [Verification](issue-46-phase-13-verification.json).

## Phase 14 strict star-kernel seed — 2026-10-16

[Phase 14](issue-46-phase-14.md) records `f1937f1`: exact orientation certifies a supplied point strictly inside every outward PLC facet's halfspace before coning facets to it. Existing TetTopology validates cells; final boundary must exactly match all PLC facets. Bar regression: nine points / 12 cells / 12 preserved facets, volume 3 mm³ within 1e-12, existing solver-mesh quality/conformity conversion passes. Original checked PLC/source metadata stays with seed. Outside/on-boundary/nonfinite centres, budgets/cancellation refuse. Real bracket hole-bearing domain explicitly refuses; do not fill holes with a hull or advertise general constrained Delaunay.

This is a limited diagnostic conforming seed only. General constrained recovery/refinement, cavities/regions, automatic kernel search, source-boundary solver tagging and sliver optimization remain open, as do broader native CAD and all later epic gates. No new dependency, hosted CI, tolerance relaxation or production/main change. Final qualified checks follow below.

Phase 14 final checks at `f1937f1b6c03237a2317e9f0d0a8f8f2a7f9831c`: **65 CAD-enabled / 52 feature-off tests and CAD-enabled app lib/tests compilation pass**, no failures/ignored checks. Hole-bearing bracket refusal passes. [Verification](issue-46-phase-14-verification.json).

## Phase 15 hollow-domain and quality-refusal corpus — 2026-10-16

[Phase 15](issue-46-phase-15.md), source `60e9e08`, adds only CAD-enabled regressions. Hollow tube r=1..2 mm, L=3 mm: four native source faces, analytic 9π mm³ volume criterion and hole/wall/exterior signed-distance signs pass. Thin plate (0.01 mm) and near-boundary (1e-12 mm) bar kernel seeds preserve valid geometry/positive topology but must fail unchanged solver quality limits. These refusal cases are not optimized solver meshes or successful training samples. [Support snapshot](issue-46-native-support-matrix-2026-10-16.md) consolidates all diagnostic scope and remaining gates.

Next implementation priorities remain general native CAD/assembly/exact trim and conservative healing coverage, general PLC recovery/refinement and sliver/conditioning qualification before P2/P3/final gates. Source references currently support identical-source regeneration, not cross-edit naming or general AMR/material/contact/history transfer. Star seeds retain generic solver boundary tags; source wrapper migration remains open. Production stays OCCT-backed, no feature/main merge or retirement approval is implied. Final CAD check result follows; feature-off/app evidence remains Phase 14 for this test-only increment.

Phase 15 qualified source `60e9e086f4bf41d8a276e0ca0c9e92285088763a`: **67 CAD-enabled tests pass**, no failures/ignored tests. Feature-off (52 tests) and app compilation reuse Phase 14 evidence for unchanged implementation. [Verification](issue-46-phase-15-verification.json).

## Phase 16 source-tagged seed solve — 2026-10-16

[Phase 16](issue-46-phase-16.md), source `93f0fb6`: limited certified star seeds now convert through unchanged solver quality/conformity gates, restore every original PLC facet's source-face group and retain source/repair metadata. Shared `NativeBoundaryMesh` owns immutable selection lookup for both seed and existing lattice wrapper; lattice API remains compatible. CheckedSurface ownership extraction is shared with distance conversion, no duplicate BVH or guessed identity. Unknown source provenance refuses before costly conversion.

The shared analytic tension patch now covers native seed Tet4 and lattice Tet4/Tet10, using stored source-face load/support selections and unchanged 1e-6 relative displacement criterion. Thin/near-boundary seed quality refusal remains; unknown-source synthetic PLC cannot invent source selections. This does not add general PLC recovery, holes/cavities, high-order seed generation or production strategies. Broader CAD/assembly/exact trims, sliver/conditioning and later epic gates stay open. No new dependency/hosted CI/tolerance relaxation/main change/OCCT retirement. Qualified results follow.

Phase 16 final checks at `93f0fb60a867ee63a4533e8fbdf42d12e3b7b2a5`: **68 CAD-enabled / 53 feature-off tests and CAD-enabled app lib/tests compilation pass**, no failures/ignored checks. [Verification](issue-46-phase-16-verification.json).

## Phase 17 bounded interior quality trial — 2026-10-16

[Phase 17](issue-46-phase-17.md), source `bfc91ef`: one optional compensated boundary-vertex-mean kernel trial, reusing exact visibility/topology/boundary certificates and existing quality metrics. Accept only strictly improved minimum scaled Jacobian with non-worsening worst edge ratio; every trial records coordinates/metrics/refusal. Staged cancellation precedes commit. Original CAD boundary/source data never move. Near-boundary bar min-J improves 2.82842712474619e-13 → 0.13483997249264842, worst edge ratio stays 3.1622776601683795; unchanged solver gates and analytic reference-selection tension patch pass. Centered bar remains unchanged. Concave L-prism volume=5 mm³ within 1e-12; its vertex mean is outside the kernel and is refused with original points/cells unchanged.

This is a limited interior sliver-quality pilot only, not general elimination, smoothing, recovery/refinement, solver-conditioning qualification or training evidence. General CAD/assembly/exact trims, PLC cavities/regions and later epic gates remain open. No new dependency/hosted CI/tolerance relaxation/main or production strategy change. Phase 16 app compilation remains applicable to unchanged production callers; full solid checks follow below. Continue substantive P1 work, keep explicit support limits and OCCT, and do not close the epic or infer merge/retirement authorization.

Phase 17 final checks at `bfc91ef46eadc79b368e99499633a5a921c5c94b`: **70 CAD-enabled / 53 feature-off tests pass**, no failures/ignored checks. Phase 16 app compilation remains the latest production-consumer check. [Verification](issue-46-phase-17-verification.json).

## Phase 18 runnable diagnostics — 2026-10-16

[Phase 18](issue-46-phase-18.md) / [verification](issue-46-phase-18-verification.json): feature source `53a223f`, final CLI source `f349ed3`. `step-inventory` now supports `--plc`, `--star-seed X,Y,Z` in mm and optional `--trial-kernel`, combining with explicit chart/orientation policies. Staged progress is deduplicated; rejected candidate metrics are printed separately from retained metrics. Seven standalone cases pass (bracket constraints; canonical tilted seed; expected strict/hole/nonfinite/missing-coordinate refusals; help). Tilted seed: 60 points/114 cells, candidate correctly declined by quality guard. No material inference/solver analysis or production availability.

Two development example-build failures from progress-variable cleanup (E0425/E0282) were fixed; final build/cases pass. Library unchanged, so reuse Phase 17 core 70 CAD-enabled/53 feature-off tests and Phase 16 app compile. No extra hosted CI/dependency/tolerance/production/main change. Main last verified remotely at 8ba27a2; feature merge and OCCT retirement still require separate approval. Continue substantive native CAD/assembly/exact trim and general PLC recovery/refinement, quality/conditioning and later epic gates; the full epic remains incomplete. Do not treat diagnostic seed counts or rejected quality trials as verified training data.

## Phase 19 bounded rational curves — 2026-10-16

[Phase 19](issue-46-phase-19.md), source `6cf9dfe`: privately owned clamped continuous positive-weight `NurbsCurve`, admission budgets/arithmetic checks and homogeneous de Boor evaluation. Exact endpoint source controls retained; invalid/outside parameters refuse. Five analytic/invariance regression cases cover rational quarter-circle, Bezier/polyline spans, reparameterization/common-weight scaling, cubic degree elevation/repeated knots and invalid/budget cases. No STEP integration or production capability is enabled; analytic bodies still only accept LINE/CIRCLE and PLANE/CYLINDRICAL_SURFACE.

Next: bounded STEP simple/complex B-spline declarations with source refs, preserved declared metadata and explicit caller-bound mm scale, reusing existing point/scalar decoding. Do not infer schema/form/self-intersection validity, unit context, placements or full B-Rep success from curve extraction. Later projection/sampling/surface/trim/CAD and general PLC/quality/P2/P3/final gates remain open. No new dependency/hosted CI/tolerance relaxation/main change/retirement. Full solid checks follow below; no production caller uses the new curve API.

Phase 19 final checks at `6cf9dfeba1f05384357b37be0b995c7f235c4572`: **75 CAD-enabled / 58 feature-off tests pass**, no failures/ignored checks. App compilation remains Phase 16 for unchanged production callers. [Verification](issue-46-phase-19-verification.json).

## Phase 20 coordinate conversion underflow — 2026-10-16

[Phase 20](issue-46-phase-20.md), source `0f9301c`: reproduced `1e-310 * 1e-21 → 0` being silently accepted as a zero point by the old native decoder. New test failed before fix (accepted `[0,0,0]`); shared coordinate normalization now rejects any nonzero source component becoming zero, for both CARTESIAN_POINT and raw body vertex paths. Positive finite factor and unchanged normalized arithmetic checks remain; source point offsets preserved. Radius and unit-factor paths already rejected underflow and were not relaxed. Three regression cases cover direct/mixed underflow, ordinary inch/mm/zero/invalid factors and actual bracket source vertex under YOCTO units. No production OCCT/material behavior claim.

Next remains bounded STEP simple/complex spline declaration mapping, reusing corrected point/scalar decoding and explicit caller-bound mm scaling; metadata/form/closed/self flags are declarations only. No new dependency/hosted CI/tolerance relaxation/production/main change. Source grep confirms no production AnalyticModel caller; app evidence remains Phase 16. Full test results follow below; all broader CAD/PLC/quality/P2/P3/final gates remain open.

Phase 20 final checks at `0f9301c583612f1ac977bc919b499e9c8859f49e`: **78 CAD-enabled / 61 feature-off tests pass**, no final failures/ignored checks. The pre-fix underflow reproduction failed as intended and is recorded separately. [Verification](issue-46-phase-20-verification.json).

## Phase 21 STEP spline declarations — 2026-10-16

[Phase 21](issue-46-phase-21.md), source `7e2d7a8`: bounded simple/complex polynomial/rational curve declaration reader, required/unique known inheritance components, exact multiplicities and capped expansion. Retains document-scoped curve/Cartesian-control refs, name and declared form/closed/self/knot flags plus explicit caller-provided mm factor. Flags are not certificates; reader does not bind unit contexts, apply placements or import a B-Rep. Reuses common scalar/point conversion (crate-private visibility only), including Phase 20 underflow guard. Five feature-off regressions cover rational analytic geometry/scaling/metadata, polynomial defaults/no form inference, malformed/huge/budget refusals and source-point errors. Actual bracket endpoint-matching line→spline declaration extracts, but full native body import still explicitly refuses that curve.

Initial E0507 borrowed-weight move failure was fixed by borrowing (no array clone). Full checks/consumer compilation follow below. Next native CAD foundation: verified derivatives/projection/sampling and tensor-product rational surface evaluation, preserving strict importer refusal until complete surface/trim gates pass. General CAD/assembly/exact trims, PLC recovery/refinement/cavities/regions, general sliver/conditioning and P2/P3/final gates remain open; no new dependency/hosted CI/tolerance/main/production/retirement change.

Phase 21 final checks at `7e2d7a8a8c99d6d08dc4b7785f05a1e287ec05d8`: **83 CAD-enabled / 66 feature-off tests and CAD-enabled app lib/tests compilation pass**, no final failures/ignored checks. [Verification](issue-46-phase-21-verification.json).

## Phase 22 tensor rational surfaces — 2026-10-16

[Phase 22](issue-46-phase-22.md), source `96bb22a`: bounded positive-weight clamped tensor evaluator, shared validated Basis/homogeneous recurrence/control checks/project with curves. Both parameter directions retain homogeneous weights; precomputed spans and active supports only. Global net/degree/evaluation-blend budgets, authoritative corners, no outside-parameter clamping. Five new plane/cylinder/sphere (including pole point)/parameter-invariance/negative tests pass alongside all old curve tests. No normal/projection/trim/solid certificate or body support is enabled.

Important next-mapper invariant: evaluator storage is V-major index `v*u_count+u`; STEP B_SPLINE_SURFACE controls/weights are U rows containing V columns. Transpose deliberately and retain original Cartesian source refs in matching evaluator order. Reuse logical/integer/knot/component decoding from step_nurbs instead of duplicating it. Preserve declared flags as declarations only, explicit caller-bound mm scale, aggregate net budgets, strict body refusal. No dependency/hosted CI/tolerance/main/production/retirement change; app evidence remains Phase 21 for unchanged callers. Full checks follow below; general CAD/PLC/quality/P2/P3/final gates remain open.

Phase 22 final checks at `96bb22a2240db1c3a07cb90cacf8cc8b48dd74ea`: **88 CAD-enabled / 71 feature-off tests pass**, no failures/ignored checks. Phase 21 app compilation remains applicable to unchanged production callers. [Verification](issue-46-phase-22-verification.json).

## Phase 23 STEP surface nets — 2026-10-16

[Phase 23](issue-46-phase-23.md), source `9aa603f`: simple/complex rational/polynomial surface declarations preserve source/control refs, flags/form/name and explicit mm scale; STEP U-row/V-column net transposes to evaluator V-major order. Knots remain original parameters, only Cartesian points are length-scaled. Shared inheritance/logical/integer/knot expansion with curve reader. Six surface regressions cover quarter-cylinder analytic geometry/units/identity, polynomial/no tag inference, malformed aggregate caps, underflow and strict body refusal. An equivalent planar spline replacing a real bracket plane evaluates correctly but default body import still rejects that exact surface.

Verified additional bug: with a deliberately relaxed caller degree cap, invalid huge degrees reached Vec capacity and panicked in both declaration readers. Pre-fix tests captured capacity overflow (two expected failures); shared unconditional degree/control-count admission now returns Geometry before allocation. Curve and surface readers each have a passing no-panic regression. Resource defaults/tolerances unchanged. Root live code-map OCCT build note corrected to prebuilt default / CMake occt-source (Windows static CRT).

Next: local curve/surface differentials by reusing fea-core's existing Dual/Scalar arithmetic, not a second AD engine; refuse nonsmooth knot points and label one-sided endpoints. No new fea-core/global sensitivity capability should be advertised. Normalization can reuse/extract the existing stable native direction helper. Derivative/normal/projection/sampling/trim and full B-Rep/CAD/assembly, general PLC/quality and P2/P3/final gates remain open. Preserve strict importer refusal, source ownership, budgets and OCCT; no extra hosted CI/main merge/retirement. Full source-specific checks follow below.

Phase 23 final checks at `9aa603f0434f3b4faf237eed8b30c87ef57ac967`: **95 CAD-enabled / 78 feature-off tests and CAD-enabled app lib/tests compilation pass**, no final failures/ignored checks. Two pre-fix malformed-degree reproduction failures are recorded separately. [Verification](issue-46-phase-23-verification.json).

## Phase 24 local spline differentials — 2026-10-16

Source `9269808` reuses fea-core's existing Dual/Scalar arithmetic in the shared homogeneous de Boor evaluator. Curve tangents and tensor-surface partials are derivatives with respect to the original knot parameters. This adds no global CAD-edit, mesh or solver sensitivity capability and no second AD engine.

Endpoints report one-sided derivatives from inside the domain; exact internal C0 knots refuse differential evaluation. Authoritative endpoint/corner coordinates remain unchanged. Surface differential work charges three lanes against the existing evaluation budget. Computed normals use stable direction normalization; zero/parallel partials and sphere poles refuse normals, although pole point evaluation remains valid. Computed directions are not regularity certificates.

Eight differential regressions check independent analytic rational tangents, planes/cylinders/spheres, endpoint sides, knot/length/common-weight transformations, existing Taylor verification (order >1.9), C0 refusal, singular normals and unchanged work limits. Existing curve/surface value tests remain passing.

An additional verified normalization defect was reproduced: `[1e-300,1e300,0]` silently lost its nonzero first component. The expected-error regression failed before the fix. The shared native direction helper now refuses nonzero component underflow; ordinary vectors at extreme uniform scales remain supported. Native analytic frames reuse this helper without changing tolerances.

Final checks at source `92698088cd1300a17fafee37d62da885486ae5e0`: **104 CAD-enabled tests, 87 feature-off tests, and CAD-enabled application lib/tests compilation pass**, with zero final failures/ignored tests. The pre-fix reproduction is recorded separately. [Verification](issue-46-phase-24-verification.json).

No dependency, production backend, main, hosted CI or OCCT retirement change. Full spline-backed B-Rep import remains explicitly refused pending context/placement binding, bounded sampling/projection and trim/locus validation. General CAD/assemblies, PLC recovery, quality and P2/P3/final gates remain open. Next: share the strict single-body representation/unit-context binding before adding more native geometry integration.

## Phase 25 shared context — 2026-10-16

Source `c6bd290` extracts the existing strict single-body representation/context admission from AnalyticModel into `step_body::BodyContext`. Both paths now use one implementation. The immutable context exposes raw source-unit topology, a document-scoped length-context reference and its declared millimetres-per-source-unit factor. It does not normalize the raw coordinates or certify geometry.

Existing assembly/mapping, multiple-body, ambiguous representation, unsupported item and absent length-context refusals remain. No inferred units or fallback scale is introduced. Two new regressions verify millimetre/metre factors with unchanged raw point identities/coordinates, exact-document rejection after source edits, and duplicate/missing direct representation refusal. Existing analytic importer tests exercise the extracted path.

An initial compile error used Result-style `?` on the source-reference Option; corrected with an explicit missing-context error before qualification. No final code change to dependencies, tolerances, production backend or main. This is reusable context admission for subsequent spline declaration integration, not full spline-backed B-Rep support or assembly placement support.

Source-specific final results are recorded in [verification](issue-46-phase-25-verification.json). Native spline-body sampling/projection/trims and broader CAD/PLC/quality/P2/P3/final gates remain open. No hosted CI was dispatched.

Final checks at `c6bd290ca89f85320db2c7f6dcc57ce342c42967`: **106 CAD-enabled / 89 feature-off tests and CAD-enabled app lib/tests compilation pass**, zero final failures/ignored checks.

## Phase 26 bound surface declarations — 2026-10-16

Source `0b19cd0` adds `BodyContext::read_spline_surface`. The caller supplies a document-scoped body-face reference, not an unqualified surface number or guessed unit multiplier. Both context and face references resolve against the exact source document before numeric topology lookup. Non-face references refuse; the surface reader uses the bound representation length factor. The returned declaration retains its actual source surface identity.

The existing equivalent planar-spline bracket regression now exercises this route. It verifies declared mm and metre scaling, unchanged knot domains, coordinate scaling at three independent parameter locations, non-face refusal, stale context and stale face refusal. Full AnalyticModel import still rejects the exact unsupported spline surface; no partial success is advertised. All previous declaration parsing/limits tests remain.

This binds declarations only: trims, regularity, exact surface/edge agreement, sampled error bounds and full B-Rep validation remain required. No production backend switch, dependency/tolerance change, hosted CI, main modification or OCCT retirement. Source-specific tests and consumer compilation are recorded in [verification](issue-46-phase-26-verification.json).

Final checks at `0b19cd0ac3bd3c57ecf4f808d9daec280673a81d`: **106 CAD-enabled / 89 feature-off tests and CAD-enabled app lib/tests compilation pass**, zero failures/ignored checks.

Next: equivalent source-bound curve extraction with shared bounded wrapper resolution, then separately verified curve sampling/projection and trim/locus gates. Do not enable spline-body success from declaration evaluation. All broader epic gates remain open; preserve OCCT and branch isolation.

## Phase 27 bound curve declarations — 2026-10-16

Source `69ae967` adds `BodyContext::read_spline_curve`: exact-document edge references resolve before numeric body lookup; the terminal spline declaration uses the body's declared length factor and retains the actual curve source reference. Non-edge and stale context/edge references refuse.

The existing analytic importer and this reader now share one bounded 3D SURFACE_CURVE/SEAM_CURVE wrapper traversal, preserving the 32-entity/cycle refusal and analytic geometry errors. Complex terminal declarations remain available to the spline declaration reader while the strict analytic body reader still rejects unsupported spline geometry. Associated p-curves are not certified by this helper.

The existing endpoint-matching spline replacement on the actual bracket now tests source-bound extraction, retained curve identity, declared mm/metre factors, authoritative endpoint scaling and unchanged knot domains. Both stale-reference directions, non-edge references and a cyclic wrapper refuse. Full native analytic body import still refuses the same exact unsupported curve.

No dependency, tolerance, production backend, main or hosted CI change. Bounded spline sampling/projection, edge/surface locus and trim validation are still required before full spline-body support. General CAD/assemblies, PLC recovery/quality and all P2/P3/final gates remain open. Source-specific final evidence is recorded in [verification](issue-46-phase-27-verification.json).

Final checks at `69ae967c2a3b8fcb271f24c38167fed6d5fd5782`: **106 CAD-enabled / 89 feature-off tests and CAD-enabled app lib/tests compilation pass**, zero failures/ignored checks.

Next substantive CAD milestone: bounded rational-curve enclosures and error-controlled shared-edge sampling, followed by verified surface projection, p-curve/trim and edge/surface locus gates. Reuse existing basis arithmetic; a local Newton candidate or midpoint chord test alone is not a global projection/error certificate. Preserve budgets/cancellation and strict unsupported refusals. Body/curve/surface declaration binding is now implemented; do not repeat it. General PLC/quality and all P2/P3/final gates remain open; OCCT remains production.

## Phase 28 rational enclosure/sampling — 2026-10-16

Sources `92bd937` (enclosure) and `166279e` (sampling). This advances the native CAD geometry gate, not full spline-body import or production strategy availability.

## Enclosure contract

`NurbsCurve::enclose` bounds a closed parameter subdomain across its intersected knot spans. It uses the same homogeneous de Boor recurrence as point/Dual evaluation through a minimal arithmetic/blend trait. Existing Dual/Scalar evaluation remains on its original arithmetic path. A private binary64 interval implementation rounds primitive operations outward; denominator subtraction and homogeneous control products are included. Convex blending intersects with the input hull only where active-span coefficients are mathematically in [0,1]. Positive rational weights remain bounded away from zero. Nonfinite/domain/order/work-budget inputs refuse.

Bounds describe the exact real rational curve defined by accepted binary64 controls/weights/knots. They do not recover decimal source precision, prove regularity or injectivity, or validate CAD trims. There is no new dependency, second AD engine, tolerance relaxation or global derivative capability.

## Sampling contract

`nurbs_sample::sample` retains distinct knots and evaluator endpoints, recursively subdividing entire parameter intervals. Each output segment carries a symmetric Hausdorff upper bound in the control coordinate units. Source-bound STEP declarations already use mm; standalone evaluators may use other explicit units. A positive-weight degree-one span traces a straight segment and uses its endpoint-rounding bounds without unnecessary subdivision, including diagonal spans.

For a general interval, distance to the segment is convex; bounding every corner of the curve enclosure bounds every enclosed point. Each corner uses an outward-rounded distance to a computed real convex blend of segment endpoints (the blend need not be the exact nearest point). Endpoint enclosures additionally bound evaluation rounding. A continuous curve inside a segment's epsilon capsule with both endpoints within epsilon of their corresponding segment endpoints has symmetric Hausdorff distance at most epsilon: use the intermediate value of its axial projection for interior segment points, and an endpoint for any uncovered end interval. This requires the existing clamped/continuous/positive-weight curve admission; it is not a simple-curve or edge/surface agreement certificate.

Progress reports cumulative enclosure calls, accepted segments and pending intervals rather than a fabricated percentage. Cancellation is checked before work, at every adaptive interval and before returning the immutable result. Segment, recursion and enclosure budgets, and parameter precision exhaustion refuse atomically; no partial result or implicit tolerance increase is returned.

## Evidence and limitations

Eight new regressions cover outward rounding/subnormal arithmetic/zero denominators; rational-circle analytic enclosure and shrinking subdomains; hidden S-curve excursions despite a chord-aligned midpoint; span/domain/work refusals; analytic circle curve-to-chord and chord-to-arc distances; diagonal straight/C0 knot preservation; independent cubic geometry; and initial/final cancellation plus each resource limit. Existing evaluator/Dual and native pipeline tests remain required.

The development circle case uses 4,094 segments and 24,561 enclosure calls at tolerance 0.001; the four sampling tests complete in 0.04 s locally (not a standalone benchmark or representative CAD throughput/memory claim). Axis-aligned interval bounds lose correlations and over-refine curved spans compared with a tighter rational-Bezier/control-polygon certificate. Do not market this as an optimized production spline mesher. Tiny tolerances/poorly conditioned weights may hit explicit budgets. No sampled-point-only test substitutes for interval acceptance.

Full source-specific tests and app compilation are recorded in [verification](issue-46-phase-28-verification.json). No hosted CI, main modification, production switch or OCCT retirement. Next gates: tighten conservative curve bounds; integrate source-bound edge sampling/endpoint agreement; bound surface patches and projection; validate p-curves/trims and full edge/surface loci before enabling native spline B-Rep import. General CAD/assemblies, PLC recovery/sliver/conditioning, P2/P3 and final parity remain open.

Final checks at `166279e61b964c846cda71f8c4add6be76287305`: **114 CAD-enabled / 97 feature-off tests and CAD-enabled app lib/tests compilation pass**, zero failures/ignored tests.

Next implementation: bounded global point-to-curve distance bounds, retaining candidate/uncertainty and refusing budgets/precision exhaustion. A global distance gap is not a unique projection or parameter certificate. Then surface patch/projection and source edge/surface/trim integration remain.

## Phase 29 global curve distance — 2026-10-16

Source `d355037` adds `nurbs_distance::distance`, a deterministic branch-and-bound query over every nonempty knot span. Interval boxes provide lower bounds on distance to each curve subdomain; point enclosures provide upper bounds for actual curve parameters. A priority queue refines the smallest remaining lower bound. Pruning uses the current upper bound; termination requires the outward-rounded global upper-minus-lower distance gap to meet the requested tolerance.

The result records the global distance bracket, a candidate parameter, its binary64 evaluated representative, the enclosure of the exact curve point at that parameter and work count. The mathematical candidate's distance is within the requested objective gap of the global minimum. This does **not** bound error in the nearest parameter/position, prove a unique minimizer, establish a regular projection, validate a trimmed edge, or enable full spline B-Rep import. Nonunique minima (e.g. circle centre) remain valid distance queries without a uniqueness claim.

Finite query/arithmetic-domain and positive tolerance validation, enclosure/queue budgets, precision exhaustion, deterministic tie ordering, monotonic progress counters and initial/adaptive/final cancellation prevent returning unconverged results. Geometry units remain explicit; distance tolerances use the control coordinate units and knot parameters are unchanged.

## Reproduced conditioning problem and correction

Initial tests passed three groups, but a constant rational curve with unequal positive weights exhausted the unchanged 100,000-enclosure budget at 1e-6 accuracy. Independent homogeneous numerator/denominator intervals discarded their exact correlation. The expected distance was known analytically; the failing development log is preserved separately.

The curve enclosure now also intersects its rational interval with the active **Cartesian** control-point hull. Positive rational basis functions form a partition of unity, so this is an exact geometric bound, including constant curves and unequal weights. It is not a tolerance increase or a budget workaround. The same constant-curve query now passes with the original budget/tolerance, and all earlier enclosure/sampling tests remain required. No acceptance limit or default resource budget changed.

Four regression groups verify rational straight-line interior/endpoint distances; a degenerate constant curve; analytic circle interior/endpoint/nonunique distances; searching a later C0 knot span; length/common-weight/parameter transformations; and work/queue/input/cancellation failures. The tests compare against independent analytic distances rather than accepting a local optimizer residual.

No new dependency, production backend switch, hosted CI, main modification or OCCT retirement. Enclosure conservatism may still exhaust budgets at tight tolerances, particularly for many near-minimizers; failure remains explicit. Not representative CAD performance or training evidence. Full checks and development failure are recorded in [verification](issue-46-phase-29-verification.json).

Next: tensor-surface patch enclosures and bounded distance/projection candidates; source-bound edge sampling and endpoint/locus verification; p-curves/trims and full spline-body admission. General CAD/assembly, constrained PLC recovery/refinement/cavities, sliver/conditioning and P2/P3/final gates remain open.

Final checks at `d355037d06abaf578410d7158ac9ac3b2ce408ce`: **118 CAD-enabled / 101 feature-off tests and CAD-enabled app lib/tests compilation pass**, zero final failures/ignored checks. The development failure remains separately recorded.

## Phase 30 tensor patch enclosures — 2026-10-16

Source `d3e81dc` adds `NurbsSurface::enclose` over a closed U/V rectangle. Both validated bases use the shared outward-rounded homogeneous recurrence; weights are retained through both directions and projection occurs only after the tensor evaluation. Active Cartesian control-net hulls retain positive-weight convexity. The result bounds the exact real surface defined by accepted binary64 data, not every rounded evaluator output.

A shared `geometry_bounds::Bounds3` now holds curve/surface boxes; the existing public `nurbs::CurveBounds` name remains a compatibility re-export. No second interval or derivative engine is introduced.

Patch count and aggregate two-bound interpolation work are charged before each patch against explicit limits and the existing constructor work cap. Two-bound work is a structural budget, not a CPU instruction/time estimate. Nonfinite/outside/reversed domains and exhausted budgets return no partial enclosure. Zero-area domains and C0 endpoint-neighbour patches remain covered; no seam/parameter snapping or tolerance increase occurs.

`Basis::span_range` uses binary searches to locate intersected knot spans, including endpoint-only neighbours. Curve enclosure now shares this path. Local distance/sampling queries no longer linearly scan unrelated knots before each recurrence. The mathematical interval and budget semantics remain unchanged; no representative speedup is claimed without a benchmark.

Three new test groups cover independent plane/cylinder/sphere references, pole/corner domains, constant nets with unequal weights, invalid domains, patch/interpolation-work refusals and all four patches meeting a two-direction C0 knot. Existing curve enclosures/sampling/distance/Dual tests remain required.

An initial test incorrectly required every rounded evaluator output to lie inside the exact-real geometric box: a cylinder evaluation returned `z=3.0000000000000004` while the exact upper boundary is `z=3`. The initial run failed (48 pass / 1 fail); a subsequent focused run reproduced the same failure. The test now uses an independent analytic reference, consistent with the stated exact-real contract. No enclosure expansion or numerical acceptance change was made. Logs preserve the failure and final corrected run.

This is not a surface regularity/injectivity, trimmed-domain or unique-projection certificate. Full spline-backed B-Rep import remains refused. Next: bounded surface distance/projection candidates and source-bound shared-edge endpoint/locus checks, followed by p-curves/trim validation and body admission. General CAD/assemblies, PLC recovery/refinement/cavities/regions, sliver/conditioning and P2/P3/final gates remain open.

No new dependency, hosted CI, production backend switch, main change or OCCT retirement. [Source-specific verification](issue-46-phase-30-verification.json).

Final checks at `d3e81dcda5ab418753a4e1779980294a9c00ac2f`: **121 CAD-enabled / 104 feature-off tests and CAD-enabled app lib/tests compilation pass**, zero final failures/ignored tests. Initial reference-test failures remain recorded separately.

Fresh-agent starting point: `nurbs_sample` and `nurbs_distance` are bounded curve APIs; `NurbsSurface::enclose` now supplies tensor patch boxes. Preserve exact-real versus rounded evaluator distinction. Next is bounded surface-distance/projection candidates, followed by source-bound edge endpoint/locus and p-curve/trim checks. Do not claim native spline body support from these standalone APIs. Source/rg fallback remains necessary: CodeGraph is unavailable. Production OCCT/main stay unchanged; representative learning/conditioning and all broader epic gates remain open.

## Phase 31 merged foundations and surface distance — 2026-10-16

User authorization on 2026-10-16 explicitly requests merging completed work and continuing all open gates. PR #65 was reviewed against its frozen evidence and merged into `main` at `37cf93220fee91cc1ced09f4acb2a7b2b6e26303`; the completed feature branch was deleted locally/remotely. Its tree matches qualified source plus documentation (121 CAD-enabled/104 feature-off tests and application compilation). No new code was introduced by the merge, and no redundant baseline/hosted workflow was dispatched. OCCT retirement was not requested or performed.

Continuation branch `feature/native-cad-spline-import` starts exactly at that merge. Source `730a878` introduces untrimmed rational-surface distance brackets using a single shared `geometry_distance` branch-and-bound implementation. Existing curve distance signatures/results stay compatible through a thin adapter; both paths use the same interval box-distance, queue, objective-gap termination, work limits and cancellation.

All nonempty tensor knot cells participate. Initialization samples their corners/centres and uses patch boxes for lower distance bounds. Refinement chooses the largest parameter width normalized by its complete axis domain, avoiding bias from different U/V units. Singular or nonunique minima can produce valid distance brackets, but candidate UV is not a unique projection or nearest-position/parameter error bound. No local Newton residual is used as a global certificate. Trimmed face domains, regularity and full B-Rep import remain separate gates.

Four new regression groups cover complete-rectangle interior/edge/corner/on-surface distances, rational cylinder and nonunique minima, later tensor cells, explicit coordinate/parameter scaling, constant singular surfaces, limits and initial/final cancellation. Existing four curve-distance groups pass through the shared engine. Default resource/tolerance limits are unchanged; low interpolation caps still refuse rather than silently expanding.

No dependency, tolerance relaxation, production CAD strategy change or OCCT retirement. Source/rg was used because CodeGraph remains unavailable. Final source-specific regression/application checks are recorded in [verification](issue-46-phase-31-verification.json). All broader CAD/trim/PLC/quality/P2/P3/final gates remain open. Next is source-bound spline-edge sampling and endpoint/orientation agreement, followed by edge/surface loci and p-curves/trim integration.

Final source checks: **125 CAD-enabled / 108 feature-off tests and application lib/tests compilation pass**, zero final failures/ignored tests.

## Phase 32 — 2026-10-16

Source `ed9757d` adds immutable `SampledSplineEdge` results, with subsequent shared-declaration reuse/diagnostic corrections qualified at the final source recorded below. The source body/edge/document context selects the spline and declared mm factor. Natural-parameter samples, topological edge direction, VERTEX_POINT references, control declarations, length context, normalized source endpoints and endpoint-distance upper bounds remain distinct and inspectable. No endpoint snapping or inferred trim occurs.

The complete clamped spline endpoints must agree with their corresponding source vertices under the existing geometry tolerance (default 1e-7 mm). Interval endpoint bounds include real-curve versus binary64 evaluation uncertainty. Sampling retains the existing whole-interval Hausdorff bound, progress, cancellation and resource limits; a final commit callback can still cancel atomically. Errors preserve actual point offsets or add the source edge offset to runtime diagnostics.

Three source integration groups use the real bracket with an endpoint-matching linear spline replacement. They verify exact-document/source/order metadata, mm/metre scaling without knot changes, both stale-reference directions, reversed valid sense, wrong orientation, inconsistent controls, unsupported interior trims, cancellation and invalid/work-limited requests. A fourth group added in Phase 33 covers body admission. This stage alone does not certify curve simplicity, p-curves, surface loci or a B-Rep.

Targeted tests passed at this increment. One combined full regression gate validates Phases 32–35 together, avoiding redundant complete suite rebuilds between closely related integration changes. See [verification](issue-46-phase-32-verification.json). OCCT stays production; general curved/piecewise spline-body and trim gates remain open.

Combined qualification at `2042c2d136829591f03cc7f4ad2d3e8c6198777a`: **130 CAD-enabled / 113 feature-off tests and CAD-enabled app lib/tests compilation pass**, zero final failures/ignored tests.

## Phase 33 — 2026-10-16

Source `23aba83` admits exactly one new native-body curve class: full-domain, two-control, positive-weight degree-one spline segments (simple polynomial or complete complex rational declarations). Such a curve traces its control segment even when weights change its parameter speed. Degree/control count, full-domain endpoint agreement, source sense and budgets are verified before reducing its geometric locus to the existing analytic line path. No form/closed/self-intersection tag is used as a geometry certificate. Interior trims and curved/piecewise/higher-order splines remain explicitly refused.

The source STEP declaration, topology IDs, fingerprint and original vertices remain intact. Existing authoritative source-vertex world samples remain shared by incident faces. Actual spline controls and source-vertex chords both undergo face-locus checks; endpoint agreement is not substituted for surface agreement. Reuse of the decoded declaration avoids repeated spline parsing, and the BodyContext is borrowed during construction rather than cloning all raw topology.

An actual bracket line replaced by the verified spline produces **bitwise-identical sampled mesh geometry** to the original analytic bracket and passes global sampled facet checks. Nonuniform rational weights also preserve the native edge samples; contradictory orientation and even a collinear quadratic with a line-like form tag still refuse. This is a strict new subset, not general spline import or a production backend switch.

Initial test compilation used nonexistent `validate_global`/`triangle_intersection::Limits` names (E0599/E0433). Source inspection corrected it to the existing consuming `check_intersections` API; no production API or assertions were weakened. Targeted suites then passed. Final combined source verification is in [verification](issue-46-phase-33-verification.json). General spline faces, curved/piecewise curves, p-curves/trims and all broader epic gates remain open.

Combined qualification at `2042c2d136829591f03cc7f4ad2d3e8c6198777a`: **130 CAD-enabled / 113 feature-off tests and CAD-enabled app lib/tests compilation pass**, zero final failures/ignored tests.

## Phase 34 — 2026-10-16

Source `4ee9b41` fixes a reproduced existing native analytic geometry bug. With default tolerance 1e-7 mm, vertices at plane offset 0.75e-7 could be individually within tolerance of a line at 1.5e-7 and of the plane at zero. The old surface check evaluated only those nearby vertices, incorrectly accepting the declared line outside surface tolerance. The new expected-error regression failed before the fix (exit 101).

The finite declared line locus between projected endpoint parameters now participates in the whole-locus surface check. Original endpoint/surface residual checks remain independently required, preventing a nearby line from hiding vertices outside surface tolerance. Plane and cylindrical-generator variants are verified, along with the inverse endpoint-outside/line-inside case. Circle-specific full-locus checks remain; no tolerance increase or world-point relocation occurs.

An initial combined qualification reached the existing malformed-radius test and failed its stable `disagrees` diagnostic assertion (28 tests passed / 1 failed in that suite). The stricter early endpoint check correctly rejected the model but used different wording. Follow-up `2042c2d` preserves the mismatch diagnostic family and reports the residual numerically; the existing assertion is unchanged. The failed run is preserved separately. [Final combined verification](issue-46-phase-34-verification.json).

Combined qualification at `2042c2d136829591f03cc7f4ad2d3e8c6198777a`: **130 CAD-enabled / 113 feature-off tests and CAD-enabled app lib/tests compilation pass**, zero final failures/ignored tests.

## Phase 35 — 2026-10-16

Source `d11d77a` corrects cancellation classification across spline sampling, the shared curve/surface distance engine and source-bound spline-edge wrappers. Callback cancellation now returns the existing `ErrorKind::Cancelled`; invalid geometry remains `Geometry` and exhausted budgets remain `Limit`. Source offsets, messages and final cancellation boundaries remain intact. No unconverged/partial result is returned.

New discriminant assertions reproduced `Geometry` instead of `Cancelled` in two unit tests (two other filtered tests passed, exit 101) and the source-edge integration test (exit 101). All affected paths were corrected; the surface distance API shares the corrected engine. These expected pre-fix failures are recorded separately from final qualification. Existing cancellation-message and atomic-result checks remain, rather than being replaced by type-only checks.

[Final combined verification](issue-46-phase-35-verification.json) covers all Phases 32–35 at the latest source, including the Phase 34 diagnostic compatibility correction. No dependency, tolerance/resource-limit increase, production backend switch, hosted workflow dispatch or OCCT retirement. Work continues through native surface/trim and broader meshing/roadmap gates.

Combined qualification at `2042c2d136829591f03cc7f4ad2d3e8c6198777a`: **130 CAD-enabled / 113 feature-off tests and CAD-enabled app lib/tests compilation pass**, zero final failures/ignored tests.

## Phase 36 merged continuation and finite spline planes — 2026-10-16

Completed PR #66 was merged, as authorized, at `3aebf992ea1861bcc0a349d09af59315fe019a18`; its local/remote branch was deleted. Current work starts at that exact merge on `feature/native-cad-planar-spline-trims`. Prior records refer to the original branches historically.

Source `1ad84b7` admits a strict new diagnostic native surface class: exactly coplanar, strictly convex, two-by-two degree-one spline patches with positive weights. The exact-sign 3D predicate is reused for coplanarity and for projected 2D orientation (z=0 points against z=1); no new predicate dependency or epsilon replaces an exact zero. Noncoplanar controls, including a warp below the ordinary geometry tolerance, refuse rather than being implicitly flattened. Degenerate/concave/folded nets and higher-order nets remain unsupported.

## Geometric domain and orientation

The positive-weight rational bilinear patch maps its rectangle onto the convex control quadrilateral. Its homogeneous Jacobian determinant is bilinear in U/V; each corner sign is a positive weight product times the corresponding adjacent control-triangle sign. Strict convexity makes all corner signs agree, hence the determinant never vanishes. The boundary maps bijectively to the quadrilateral edges; the interior local orientation and simple boundary establish the same complete quadrilateral image. No inverse-UV claim or floating Newton residual is required for this geometric reduction.

Follow-up source `25f6c25` also derives outward-rounded intervals for the exact source-control normal and its length. Point/whole-circle agreement is checked against that control plane directly, rather than relying solely on a rounded facade frame. Unresolved normal intervals refuse. The facade normal must have a certified positive dot product with the source normal, preserving U/V orientation. Shared interval square/nonnegative-square-root helpers include subnormal/rounding tests.

The analytic facade derives a plane frame in natural U/V orientation. It does not rewrite source controls, knots, UVs, face sense, topology, units or IDs. Analysis charts remain physical-mm plane charts, as already documented for the facade; they are not original spline parameter coordinates.

The finite image is enforced independently of the underlying infinite plane. Source endpoints and actual line-segment endpoints must lie inside the convex polygon under exact-sign projected tests. Native circular edges use outward-rounded support-function bounds for **the entire underlying circle** against every polygon side. This conservatively refuses an otherwise valid trimmed arc when its full circle leaves the patch; tangent uncertainty also refuses. No arc is clipped or tolerance silently expanded. Existing whole-locus surface checks, sampled trim classification, shell checks and global facet checks remain required.

## Evidence and remaining scope

The real bracket's plane replaced by an equivalent bilinear patch now builds a native sampled surface with unchanged source-face coverage and volume within 1e-12 relative to the original, and passes global sampled intersections. A tiny finite patch under the same trim loops refuses; a 1e-10 out-of-plane corner refuses. Unit tests verify nonuniform positive weights, exact boundary inclusion, circle/outside rejection, folded/degenerate/warped controls, tilted exact planes, reversed parameter grids and positive local differential orientation.

This remains **3D-locus trim reconstruction**, not full STEP p-curve or master-representation agreement validation. Associated p-curves and their parameter units are not silently certified by the new class. Curved surfaces/curves, general trimmed-curve domains, p-curve/master agreement, robust general CAD exchange and later PLC/quality/P2/P3/final gates remain open. Production stays OCCT; no backend switch, retirement, new dependency or hosted CI dispatch.

A filtered development invocation ran two patch unit tests but zero integration tests; that zero-test target is not validation. The final full feature-off/CAD suites include the actual STEP surface integration cases. No development assertion failures occurred in this phase.

The stale standalone surface-module header was aligned with its existing differential/enclosure APIs; no behavior change accompanied that documentation correction. [Source-specific verification](issue-46-phase-36-verification.json). Next: bounded source p-curve metadata/parameter decoding and verified agreement, retaining angular versus length versus spline-knot parameter distinctions.

Final checks at `25f6c25ece1ef3b7cad04354cdcd709de15bea6b`: **132 CAD-enabled / 115 feature-off tests and CAD-enabled application lib/tests compilation pass**, zero failures/ignored tests.

Concrete next gate: the root bracket has 228 associated PCURVE records in a read-only source audit (200 LINE, 13 CIRCLE, 15 complex spline geometries). At least #235 → #237 is a closed quadratic with extended, unclamped knots; current clamped-only curve declaration decoding cannot validate it. Add explicit finite-active-domain spline evaluation and parameter-space decoding, preserving no length scaling for UV/knot parameters. Do not treat declared PCURVE_S1 master agreement as verified merely because 3D loci reconstruct successfully. Keep existing clamped/body defaults strict until source binding/trim checks pass.
