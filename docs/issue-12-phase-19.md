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
- [x] Add a visible native control panel, richer scalar-field selection, units/legend and contextual help; title hints alone are not the final editor UI. (CPU-rasterized HUD; static results expose all 11 fields; legend units are "consistent with the model inputs" because there is no project-wide unit system.)
- [x] Reuse one graphics device/pipelines on mode changes, bound resident buffers and verify surface/mesh/mode changes in a real native window.
- [x] Extract the native editor controller and operation capabilities from the window runner before adding authoring commands. (`controller.rs`, `editor/*`)

## Phase 21 — Interactive model authoring and analysis
- [x] (members) Versioned native project `fea_problem::members::MemberProblem` (JSON, atomic save, validated load; Workbench key `f` and `--gpu-editor`). **Solid `fea_problem::Problem` authoring in the native window is not implemented**: solids stay in the terminal Workbench.
- [x] (members) Picking, stable never-reused ids, tools to create nodes/members, support/load assignment, typed property editing, undo/redo (200 deep). Unit context is a display-only note; nothing is converted.
- [x] (members) Solve runs on a worker with elapsed time, cancel (the kernel solve is not interruptible: cancel discards the answer), error text, stale-result invalidation by exact problem equality, and acceptance diagnostics (residual, force and moment balance) shown in the panel. There is no step-level progress inside the single linear solve.
- [x] Aircraft frame/truss authoring: line members, section properties, translation/rotation DOFs, planar mode, verified against closed forms (tripod, Timoshenko cantilever, uniform-load cantilever, rotation/translation invariance, two-bar truss).
- [x] Unsupported combinations are refused with an explanation (rotations/moments on truss-only nodes, parallel reference axis, off-plane nodes or loads in planar mode, mechanisms). Contact, coupling and shells are not offered.

## Phase 22 — Contextual assistance and advisory ML
- [x] Deterministic operation/selection capability filtering (`editor/caps.rs`) with reasons; tools are always listed, unavailable operations show `[--]`.
- [x] Versioned advisory interface, provenance-carrying trace format, evaluation harness with held-out and OOD scoring (`editor/advisory.rs`); ML can only reorder allowed actions. [ ] **No representative real datasets exist**: the tests use clearly labelled synthetic fixtures.
- [ ] Measured benefit of a learned ranker over the rules is **not demonstrated** (no usage data). Implemented and tested: offline operation, opt-out (`FEA_EDITOR_ADVISOR=off`), OOD/low-support fallback, rejection of invalid rankings. The learned ranker is off unless `FEA_EDITOR_MODEL` is set; the default is the rule-based baseline, labelled "rules".

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

## Phases 20-22 increment — 2026-10-10 (Claude Code session, uncommitted at time of writing)

Delivered: pure controller and click-vs-drag pointer, CPU HUD (legend, field selection, click targets, auto-shrinking panels), multi-field scenes (`Scene::fields`, packed transport, storage buffer in the shader), the member editor (`fea_problem::members`, `editor/*`, `--gpu-editor`, Workbench `f`), deterministic capability filtering and the advisory interface. Also fixed: arrow-key frame stepping on an empty frame list underflowed; the ignored headless GPU tests hard-coded a software adapter and now fall back to the platform adapter.

Evidence on **Apple M1 / Metal** (new: previous GPU evidence was Linux llvmpipe only): ignored headless GPU tests 2/2 including field selection and overlay pixels; `hud_snapshot` and `editor_snapshot` renders inspected visually (both panels, legend, solved tripod and cantilever); `app-tui --gpu-editor` and `--gpu-viewer` windows open on Metal, print `FEA_GPU_READY` and run without stderr output. Local `cargo test --workspace --locked -j4 --no-fail-fast` on macOS (before the last two members tests and clippy-only edits): **1,903 passed / 0 failed / 62 ignored / 94 targets**, exit 0 (previous recorded head: 1,819 / 0 / 60 / 93). After those edits: app-tui `gpu_viewer` 83 passed, `fea-problem` members 14 passed. Clippy is clean for the new code (pre-existing warnings elsewhere remain).

Not verified: a Windows compile of this increment (cross `cargo check --target x86_64-pc-windows-msvc` fails in `zstd-sys` without the MSVC toolchain; hosted CI builds it), real mouse/keyboard interaction in a native window (no Accessibility or Screen Recording permission for automation here; the user's macOS testing is the next gate), Windows, any measured ML benefit, solid-problem authoring in the native window, workspace-wide and hosted CI for this increment.

## Menus, advisor loop and shared row actions — 2026-10-10 (Claude Code session, uncommitted)

Delivered: self-drawn menu bar + popup + right-click context menu (`gpu_viewer/menu.rs` pure state machine with 13 tests,
`menus.rs` contents with 7 tests; F10, arrows, Enter, Esc, hover, click-outside; contents derive from `editor::caps`),
renderer overlay slots 2-3 with explicit origins and `set_overlay_top`, optional viewer/editor panels (View menu), opaque menu
background. Advisor loop (`editor/learning.rs`, 9 tests + a session-level test): opt-in recording, explicit training, gated
use, Advisor menu, persisted settings, honest labels, `FEA_EDITOR_ADVISOR=off` wins. `fea_workbench::model::apply_row_action`
extracted from the terminal Workbench (all 23 Workbench tests unchanged and green) as the base for native solid authoring.

Evidence: `cargo test -p app-tui -p fea-problem --locked`: 918 + fea-problem targets passed / 0 failed. `editor_snapshot` on
Apple M1 / Metal rendered the bar, the open Analysis menu with the highlighted Solve row, and both panels beneath it (reviewed visually).
The advisor gate was exercised only on synthetic fixtures (clearly marked in the tests): a population that saves before solving
becomes Eligible, one that follows the rules never does, thin data is Insufficient. No real session data exists; no benefit is claimed.

Not verified: real mouse/keyboard in the native window (user testing), Windows, hosted CI. Not done: solid-problem authoring in the
native window (needs an `Authoring` abstraction over `Session` in `window.rs`/`menus.rs`, mesh/edge/hole preview scene and picking,
overlay geometry for supports/loads), command palette, keyboard mnemonics, default panel visibility (both panels still start visible).

## CAD-style chrome, solid authoring and the single viewer — 2026-10-10 (Claude Code session, uncommitted)

Supersedes the corner-panel HUD described above (removed). Delivered:

- **CAD chrome** (SolidWorks-style): menu bar (File, Edit, View, Insert, Tools, Simulation, Window, Help), tabbed command ribbon
  with grouped buttons, feature tree (with a Results folder listing every result and field), property manager, status bar and
  a viewport the renderer confines the scene to (`DisplayOptions::viewport`). Standard views (front/back/left/right/top/bottom/
  isometric), dock toggles (Window menu), per-dock scrolling/folding, hover descriptions. Modules: `layout`, `chrome`, `dock`,
  `ribbon`, `menu`, `menus`, `ui`.
- **`Authoring` trait** over the member `Session` and the new **`SolidSession`**: native authoring of `fea_problem::Problem`
  (sketch shapes + holes, extrusion, imported mesh, material search over the catalog, mesh settings, supports/loads/bushings by
  surface name, undo/redo, save/load in the Workbench JSON, mesh preview + solve on workers, results with all 11 fields, surface
  picking in the viewport). Reuses the terminal Workbench's editing core, extended and shared (`fea_workbench::model`).
- **One viewer**: Workbench `g` and `t` open the same window with static + modal/buckling + transient results (`ViewerProject::append`);
  `w` opens the solid editor on the current problem and reads the edits back.
- **Input seam**: `Viewer::input(Input)` lets tests drive clicks, keys, wheel and drags without a window.

Evidence (macOS, Apple M1 / Metal): `cargo test -p app-tui -p fea-problem --locked`: app-tui 973+ passed / 0 failed (before the last
additions; viewer/Workbench subsets re-run green afterwards: 174 + 27); GPU viewport-confinement test (`--ignored`) passed; chrome
snapshots of the member editor, solid editor (mesh preview and solved von Mises) reviewed visually; `--gpu-solid` and
`--gpu-editor` windows open on Metal, print `FEA_GPU_READY` and run without stderr output.

Not verified: real mouse/keyboard on a desktop (user testing), Windows, hosted CI, performance of chrome rebuilds on very large
displays (throttled to 100 ms idle). Known gaps: no command palette or menu mnemonics; text is ASCII-only 8x8 bitmap font; solid
edge/hole picking uses surface names only (no box-select or face-level 3D picking beyond named surfaces); transient `t` still
needs the density row; per-element contours for solids remain nodal; advisor benefit unmeasured (no real recordings).
