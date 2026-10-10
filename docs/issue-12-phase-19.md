# Issue #12 — Interactive native GPU workbench roadmap

User extension, 2026-10-10 UTC. Continue exclusively on `codex/fea-core-roadmap`; PR #13 stays draft, no merge into main. Phase 12–18 completion remains historical; the following new scope is not yet complete.

## Phase 19 — Generation reliability (first implementation)
- [x] Show job stage, elapsed time and actual integration-step progress; discard stale progress events just like results.
- [x] Stream displacement snapshots into bounded GPU samples rather than allocate a complete f64 displacement history. Preserve all 240 integration steps and every residual/energy acceptance check; adapt display sampling only.
- [x] Retain actionable errors for truly oversized geometry/adapter limits; clear old errors when a new job begins.
- [x] Verify progress and streamed results against existing recorded histories, including previously refused ordinary large models.

## Phase 20 — Self-contained result exploration
- [x] Transfer all computed modes for the current problem into the native window and switch them there. Distinguish available computed modes from uncomputed eigenpairs.
- [x] Add keyboard controls for mesh visibility, deformation, contour, camera, playback, amplification and computed-mode selection.
- [ ] Add a visible native control panel, richer scalar-field selection, units/legend and contextual help; title hints alone are not the final editor UI.
- [x] Reuse one graphics device/pipelines on mode changes, bound resident buffers and verify surface/mesh/mode changes in a real native window.
- [ ] Extract the native editor controller and operation capabilities from the window runner before adding authoring commands.

## Phase 21 — Interactive model authoring and analysis
- [ ] Introduce a versioned native workbench project based on `fea_problem::Problem`, with geometry, material, mesh, supports and loads; preserve save/load interoperability with the terminal.
- [ ] Add picking and stable entity identities for geometry/nodes/edges/faces; create/edit supported geometry and assign valid constraints/loads interactively. Include undo/redo and explicit unit context.
- [ ] Route mesh/solve operations to workers, report progress/cancellation/errors, invalidate stale results, and display verified acceptance diagnostics in the native window.
- [ ] Prioritize aircraft frame/truss authoring alongside existing solid problems: native line/member geometry, section properties, generalized translation/rotation DOF mapping and formulation-specific constraints, verified against the Phase 16 references.
- [ ] Initially support the existing verified problem formulations; refuse unsupported rotations/contact/coupling combinations instead of presenting invalid actions.

## Phase 22 — Contextual assistance and advisory ML
- [ ] Build deterministic operation/selection capability filtering first: expose only actions supported by the current formulation, entity and solve state, with explanations for unavailable operations.
- [ ] Define a versioned advisory interface, provenance and representative training/held-out/OOD datasets for contextual ranking. ML must not generate unchecked constraints or replace core acceptance.
- [ ] Add optional measured ML ranking only after demonstrating benefit against the deterministic baseline; support offline operation, opt-out and OOD fallback. Never label heuristic filtering as trained ML.

## Acceptance and handoff

Each increment must record its tested commit, meaningful regressions, required workspace/hosted CI, actual UI evidence, failures and platform limits. Native macOS testing is being performed by the user; Linux software rendering is not evidence of Metal/Windows hardware interaction. Full editor/ML work remains planned until implemented and validated; the immediate increment starts with Phase 19 and Phase 20 controls.

Pre-extension source `fa482c2` removes the unused Lanczos field: dynamics 18/18, full local suite 1,811 passed / 0 failed / 60 ignored / 93 targets, exit 0. Hosted CI [38014056234](https://github.com/Teased-oChroid-orrA/powershell_tool/actions/runs/38014056234) passed at that exact SHA.

## First implemented increment

Phase 19 is implemented; Phase 20 has initial keyboard controls and compact result sessions. Phases 21–22 and the remaining Phase 20 panel/controller work are not implemented.

`Model::transient_observed` streams finite states and allows observer errors to abort; the existing `transient` API delegates to it unchanged. An independent regression compares every displacement sample, timestamp, residual and energy value bit for bit to the recorded-history API. The viewer keeps all **240 integration steps**, all acceptance diagnostics, and both physical endpoints. Display-only sampling adapts to a **4,000,000-sample** GPU budget; geometry remains capped at 500,000 nodes. The original default plate (10,171 nodes / 20,342 DOFs), previously refused by the full-history check, now generates verified frames. This is not a new temporal-error guarantee.

Progress reports meshing, fundamental frequency, initial equilibrium, each integration step and verification. Stale IDs/input signatures discard progress. A separate launch state remains visible through file transfer, decoding and GPU upload until the native process sends `FEA_GPU_READY`. Duplicate launches are blocked during preparation; old failure messages clear on a new job. Driver failures return bounded diagnostic text instead of being silently discarded.

A versioned `ViewerProject` carries all already-computed modes and original problem metadata. Compact modal scenes retain one displacement vector; a sinusoidal gain follows the computed frequency, with arbitrary mass-normalized display amplitude. They are not load-response histories. Shared `Arc<Mesh>` snapshots avoid cloning a mesh per mode; the native window borrows its active result without duplicating a complete sample buffer. Mode changes reuse the device and pipelines, uploading the selected immutable buffers only. Aggregate mode samples, index counts, metadata and file size are bounded. The companion protocol uses little-endian packed f32/u32 buffers with a small versioned JSON header, plus legacy JSON compatibility; malformed counts/truncation/trailing payloads are rejected.

Native keys: **Tab** or **[ / ]** changes computed result; **M** toggles triangulated mesh edges, **C** contour, **D** deformation; Space pauses, arrows step frames/change gain, +/- changes rate, R resets the camera, drag orbits and wheel zooms. `n` in the terminal computes modal results, then `g` opens their complete set. `t` generates actual load-release vibration. Computing new analyses from inside the native window belongs to Phase 21.

## Verification before publication

- Core dynamics: **19 passed / 0 failed**, including streamed/recorded equivalence and observer-error propagation.
- App library: **816 passed / 0 failed / 3 ignored**; native project/mode transfer, progress/stale events, launch readiness, transport bounds/round trips and prior GUI regressions executed.
- Native offscreen endpoint/interpolation/mesh/contour/compact-mode pixel checks and playback measurement: **2 passed / 0 failed**, including the formerly concurrent EGL tests. A shared test mutex now prevents their native driver initialization race; no `--test-threads=1` workaround is required.
- Native app build and terminal-only compile check passed. Final current-source workspace/hosted gates are recorded in issue #12 and PR #13 against the actual commit after publication.
- Real tmux + xterm run and actual X11/wgpu native window: the original large plate generated, progress was visible, six computed modes transferred and mode 2 selected. Mesh and mode changes altered **52,003** and **66,500** screen pixels in the recorded run. Uppercase M/C/D controls worked. Actual screenshots and `phase19-native-smoke.json/.log/.exit` are under `/workspace/phase12-evidence/`. This verifies Linux **Mesa llvmpipe software**, not Metal/Windows/hardware performance.

Diagnosed intermediate failures: an initial compile check needed `ProjectSource: Debug`; a later active-scene borrowing refactor missed one `.scene()` call (saved `phase19-check-failed-scene-refactor.log`); both corrected. The first desktop probe timed out and Mesa reported X11 shared-memory attachment errors. Disabling MIT-SHM on the cloud Xvfb server restored the actual window. A smoke-run cleanup race initially returned a failure after its assertions passed; cleanup is now idempotent and the full corrected smoke script exited 0. Signed Debian metadata/verified packages were extracted under `/workspace/ui-tools`; no system install or tracked dependency/lockfile change was needed. Services started by the smoke script were stopped; defunct child entries are not running services.

Reusable cloud `install_script` and `start_skill` were saved with the tested optional software
desktop setup and smoke command. The install script was exercised again; these saves require
environment publication to become a new snapshot and do not claim a fresh-task validation.
An offscreen run without the documented cache/runtime variables passed but emitted read-only
home-cache/XDG warnings; the correctly configured replay passed without those setup warnings.

## Superseded-generation follow-up — 2026-10-10 UTC

Published increment `447f187d04474be7cccd19b6cf01f492f2ca76b4` passed the complete local workspace gate: **1,819 passed / 0 failed / 60 ignored / 93 targets**, exit 0. Hosted [run 38042753267](https://github.com/Teased-oChroid-orrA/powershell_tool/actions/runs/38042753267) also succeeded at that SHA.

A subsequent small correction keeps an explicit “Inputs changed; previous animation finishing” elapsed-time status visible when an edited analysis supersedes a running job. It suppresses stale numerical progress and still discards stale results; another generation remains blocked until the worker finishes. The extended regression passed (1/1). The actual terminal/native smoke replay exited 0 and verified the caption during an analysis edit, then verified no stale native window opened (`phase19-stale-smoke.log/.exit`, `phase19-tui-stale-progress.png/.txt`). An initial smoke attempt changed the template, which resets the job instead of exercising the retained-worker path; the corrected replay changes Analysis. Full workspace and hosted checks for this follow-up are recorded against its final SHA in issue #12 / draft PR #13.

Remaining scope stays explicit: Phase 20 visible panels/legend/field selection, Phase 21 interactive authoring/constraints/analysis, and Phase 22 measured advisory ML. Continue on `codex/fea-core-roadmap`, preserving deterministic offline behavior; do not merge main or close the expanded issue as complete.
