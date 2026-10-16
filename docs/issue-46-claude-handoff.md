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
