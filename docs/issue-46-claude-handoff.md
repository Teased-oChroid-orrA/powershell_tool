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
