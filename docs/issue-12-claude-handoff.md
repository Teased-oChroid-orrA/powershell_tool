# Issue #12 — Claude Code handoff

2026-10-10 UTC. Repository `Teased-oChroid-orrA/powershell_tool`; branch **codex/fea-core-roadmap**.
Draft [PR #13](https://github.com/Teased-oChroid-orrA/powershell_tool/pull/13) references
[issue #12](https://github.com/Teased-oChroid-orrA/powershell_tool/issues/12).
**Do not modify or merge into main.** Main's audited tip is
`d5748135188cd6f196a2f6310ea9934251260f93`. Keep the PR draft pending review.

The user authorized incorporating the full revised roadmap and selected **aircraft frame and truss
members** for the aerospace scope, then explicitly requested **direct native Rust wgpu** visualization
and dynamic deformation animation (no game engine/high-level 3D wrapper). These instructions supersede
the historical first-small-increment stopping point and old terminal-only descriptions.

## What is incorporated

| Phase | Implemented scope and evidence source |
|---|---|
| 12 | Current-source audit, complete untouched baseline, modal/buckling/transient/thermal/hyperelastic/partial-slip independent regressions and finite/residual/physical acceptance hardening. `issue-12-phase-12.md`; ADR-013 dated addenda. |
| 13 | Generalized translation/rotation/temperature maps, constraints bound to their exact layout, sparse symmetric assembly, finite componentwise acceptance, continuum migration regressions and explicit one-way thermal-to-structural coupling. `issue-12-phase-13.md`. |
| 14 | Spatial trusses and Euler-Bernoulli/Timoshenko frames, transformations/consistent distributed loads/end-action recovery; restricted flat rectangular MITC4 membrane/plate/shell; independent patch, rigid motion, analytical and thickness/convergence checks. `issue-12-phase-14.md`. |
| 15 | Connected-component support validation, deterministic selection/fallback, bounded remeshing, strict nonlinear completion/residual/physics acceptance, finite common analysis reports/JSON and explicit failure reasons. `issue-12-phase-15.md`. |
| 16 | User-selected member load paths: independent aircraft-like spatial tripod and combined-load longeron references. `issue-12-phase-16.md`. |
| 17 | Repeated runtime/process-memory/accuracy profiles; benchmark-only timing-trained advisory selector with held-out/OOD checks. Other ML roles evaluated; production ML remains off absent broader measured benefit. `issue-12-phase-17.md`. |
| 18 | Direct wgpu/WGSL native companion window, GPU frame interpolation, static/modal/buckling scenes, actual verified load-release free-vibration history, bounded inputs/allocations and stale jobs. `issue-12-phase-18.md`. |

## Reviewable commits

- `a852085`: first small steady-heat acceptance fix; `31b4d72`: audit/baseline handoff.
- `31b76be`: dynamic and transient finite/acceptance hardening plus independent references.
- `7197139`: generalized field assembly and explicit one-way coupling.
- `f4444ef`: spatial members and restricted flat shell/plate formulations.
- `40f44de`: bounded refinement/strict nonlinear orchestration.
- `2506dd5`: selected aircraft member references.
- `656cfd5`: common acceptance, exact constraint-layout binding and final core checks.
- `8b9b786`: reproducible performance/advisory ML evaluation.
- `b7d4f33`: direct wgpu viewport, verified time-history playback and worker integration.
- Final evidence commit SHA is resolved with the document-only
  handoff commit via `git log -1 --format=%H -- docs/issue-12-claude-handoff.md`.

## Environment and exact commands

Use the existing checkout; do not create a worktree unless requested. Official Rust toolchain and
repeatable setup live outside the repo:

```sh
export CARGO_HOME=/workspace/toolchains/cargo RUSTUP_HOME=/workspace/toolchains/rustup
export PATH="$CARGO_HOME/bin:$PATH"
cd /workspace/powershell_tool
cargo test --workspace --locked -j4 --no-fail-fast
cargo check -p app-tui --bins --lib --tests --locked -j4
cargo test -p app-tui --lib --locked -j4
cargo test -p app-tui --test rendering --locked -j4
cargo check -p app-tui --no-default-features --bins --lib --tests --locked -j4
mkdir -p /workspace/gpu-runtime /workspace/gpu-cache
chmod 700 /workspace/gpu-runtime
EGL_PLATFORM=surfaceless XDG_RUNTIME_DIR=/workspace/gpu-runtime MESA_SHADER_CACHE_DIR=/workspace/gpu-cache cargo test -p app-tui --lib gpu_viewer --locked -j4 -- --ignored --nocapture --test-threads=1
cargo test -p fea-core --test contact_sweep partial_slip_3d_measurement --locked -j4 -- --ignored --nocapture
cargo test -p fea-core --test bench roadmap_profiles_and_advisory_selection --locked -j4 -- --ignored --nocapture
```

Rust 1.99.0 stable; 5 CPUs, 4 build jobs. Reusable cloud install/start configuration was saved and
setup/fetch verified. GPU viewport needs a desktop/display and native driver; software surfaceless
OpenGL can exercise offscreen rendering. GitHub API permission/network blocker was resolved after
the user's environment-settings confirmation. CodeGraph is unavailable; follow documented direct
source fallback, not invented CodeGraph outputs. `gh pr edit` fails legacy projectCards GraphQL;
use REST PATCH for PR title/body. Do not expose credentials in logs.

## Evidence before final suite

All underlying logs/exit files/machine summaries are in `/workspace/phase12-evidence/` in this
workspace; exact numerical evidence also lives in the tracked phase documents.

- Untouched audit baseline: 1,763 passed, 0 failed, 57 ignored, 89 targets, exit 0.
- First thermal increment full workspace: 1,768 passed, 0 failed, 57 ignored, 89 targets, exit 0.
- Hardening commit `31b76be` full workspace: 1,776 passed, 0 failed, 57 ignored, 89 targets, exit 0
  (`phase12-full.log/.exit/.summary.json`). Later core changes have separate targeted regression logs.
- Current core targeted coverage: 86 passed, 0 failed, 5 ignored across 11 targets
  (`core-final-regressions.log`); no tolerance weakened.
- Two final core profile repeats: `roadmap-performance-final-{1,2}.{log,json}`; both exit 0,
  wall 1.351/1.297 s, process peak RSS 51,004/48,480 KiB. Dev profile with optimized fea-core;
  these are not release/Windows speed claims. Held-out advice gained ~1.22x in repeat 1 with
  displacement difference 6.97e-12; OOD guards use deterministic selection and timing varies.

Final current-source suite and GPU evidence are appended below. Historical validation above does
not validate later edits by itself. Count passed/failed/ignored separately; ignored soak/measurement
benchmarks are not silently counted as passes.

## Limits and exact next steps for review

Core formulations are small-displacement linear members and **flat axis-aligned rectangular**
MITC4 shells/plates; no curved/distorted shell, laminate, drilling rotation, warping, member release,
plastic hinge, local/overall member buckling or aircraft certification is claimed. Aerospace references
are representative independent geometry/load cases, not an actual certified airframe or allowables.
The coupling direction is one-way; no monolithic coupled nonlinear multiphysics/time/order adapter.
Strict nonlinear acceptance still requires the caller's independent physical criterion appropriate to
its problem. A raw nonlinear `complete()` engineering termination is not strict verification.

ML is advisory benchmark-only, production off. GPU scene conversion visualizes continuum meshes
currently exposed by FEA Workbench; it does not add a member-model editor. Higher-order surfaces use
piecewise triangles through supplied nodes, not continuous shape-function evaluation; transient
colors are initial displacement magnitude, not transient stresses. Modal amplitude is arbitrary
mass-normalized, distinct from excitation response. No native Windows window/icon interaction or
hardware throughput is claimed by Linux/offscreen checks.

1. Read root/affected AGENTS, per-phase scope and final evidence below; verify branch and clean tree.
2. Review commits in order, particularly units/layout binding, componentwise mixed-field residuals,
   frame reference axes/signs, MITC4 assumptions, strict completion and conditional energy checks.
3. Re-run required commands from the current head. In a real desktop launch Toolbench, solve current
   inputs, test `g/G` and `t/T`, pause/step/orbit/zoom and closing only the companion window. Verify
   Windows Terminal and native driver behavior before a Windows release claim.
4. Keep draft PR #13 and issue #12 progress aligned with measured scope. Broader formulations/real
   aircraft loads/production ML require independent references and measured acceptance; do not
   infer them from the incorporated scoped roadmap. Do not merge into main without explicit authorization.

## GPU and onboarding evidence

GPU commit `b7d4f33`: current native bins/lib/tests check passed; terminal-only bins/lib/tests
check passed. Focused GPU module: 9 passed, 0 failed, 2 adapter tests ignored by default; both were
explicitly run serially and passed on Mesa llvmpipe/OpenGL (`gpu-native-serial.log/.exit`).
Endpoint and midpoint frames produce distinct colored-pixel buffers through the actual WGSL pipeline.
The full first app-lib attempt reported 807 passed / 1 failed / 3 ignored: the failed success fixture
correctly hit the memory budget. A bounded cantilever succeeds; oversized history refusal remains a
separate regression. The first exact retry before fixture editing still failed for the same reason.
Concurrent adapter tests aborted with EGL BadDisplay; serial context execution fixed the environment
test constraint. No numerical tolerance or budget was weakened. Full current-source verification below
supersedes these diagnosed attempts; preserve their logs rather than describing them as green.

Reusable `start_skill` configuration was updated and saved in the environment draft to include the
new handoff path, scoped app builds, terminal-only compile, workspace-local runtime/cache and serial
software GPU tests. `install_script` and existing network rules were preserved. The draft requires
review/save in environment settings to apply to future tasks; current-instance operations were tested.
A first draft update call had an argument-schema error; retry using `config.start_skill` saved it.
No new credentials or background service were required.

The final combined-feature build approached the 32 GiB overlay limit. Only inactive incremental
build caches (no file modified within the previous ten minutes) were removed: 94 cache directories,
~3,259 MiB logical bytes (`build-cache-cleanup.txt`). Source, lockfile, test binaries and evidence
were preserved. The workspace build reached test execution without ENOSPC; this was an environment
resource correction, not a disabled test or changed compiler/test criterion.

The viewport's fixed-step transient uses the fundamental period; its residual/energy checks do not
certify temporal-error bounds or resolution of every high-frequency mode. Independent kernel time
convergence is covered, but a production history needs its own step refinement and relevant excitation/
damping/stress scope. The displayed scalar is initial displacement magnitude as labelled.

## Final current-source validation and review status

Tested source: **b7d4f33b342469c227a21ebad20f31066bceafa1**. The final following commit changes
only documentation/navigation; production/test source remains exactly the tested source.

| Check | Result |
|---|---|
| Required `cargo test --workspace --locked -j4 --no-fail-fast` | Exit 0; **1,811 passed, 0 failed, 60 ignored, 93 targets**; 816.405 s including rebuild |
| App-lib target within that complete run | 809 passed, 0 failed, 3 ignored; 120.53 s |
| `cargo check -p app-tui --bins --lib --tests --locked -j4` | Exit 0 |
| Terminal-only `cargo check -p app-tui --no-default-features --bins --lib --tests --locked -j4` | Exit 0 |
| Scoped `cargo test -p app-tui --test rendering --locked -j4` | Exit 0; 38 passed, 0 failed |
| Explicit `partial_slip_3d_measurement --ignored --nocapture` | Exit 0; **0 of 24 cases failed**, harness 1 passed / 0 failed; 6.58 s |
| Native GPU endpoint/interpolated pixel comparison, final workspace binary | Exit 0; 1 passed / 0 failed |
| Native GPU playback profile, final workspace binary | Exit 0; 1 passed / 0 failed; 241 frames, software OpenGL |
| `git diff --check` | Passed |

Commands/exits/counts: `roadmap-final-commands.json`, `roadmap-final-workspace.summary.json` and
associated `.log/.exit` files. GPU: `gpu-pixels-final.log/.exit`, `gpu-profile-final.log/.json`.
Isolated software batch profile: pipeline/upload 55.056 ms, playback/completion 150.187 ms, whole
process 0.410 s, peak RSS 113,732 KiB. Native pixel buffers differ at both endpoints and midpoint.
Previously ignored GPU tests and the 24-case replay are reported separately, not added into default
workspace counts. Other measurement/soak ignores remain unrun; optional ML profile was explicitly
executed twice as recorded in Phase 17. Existing unused `LanczosCtx::kind` warning remains; fmt/clippy
are not repository gates because of preexisting drift/warnings.

No unresolved local required-test failure or environment blocker remains. GitHub-hosted CI for
`b7d4f33` was still in progress when this handoff was written:
https://github.com/Teased-oChroid-orrA/powershell_tool/actions/runs/38010291366
Do not claim a hosted CI pass. Documentation push may supersede that run; check the latest PR #13
status before review/merge. **Do not merge** without explicit user authorization. Native Windows
window/terminal/icon behavior and graphics hardware throughput remain unverified; software offscreen
checks establish the real rendering pipeline, not those platform claims. Scope restrictions above
remain acceptance boundaries, not missing implementations silently claimed by the roadmap.

Issue progress: https://github.com/Teased-oChroid-orrA/powershell_tool/issues/12#issuecomment-6091769141
Completion comment and final documentation commit follow this source-validation record. Resolve
handoff commit with `git log -1 --format=%H -- docs/issue-12-claude-handoff.md` and inspect the draft PR.

## Hosted CI completion — 2026-10-10 UTC

The pending hosted gate above is now resolved: GitHub Actions run
[38011445843](https://github.com/Teased-oChroid-orrA/powershell_tool/actions/runs/38011445843)
completed successfully for **fbc4938fc206a99745f7371857b8f0cfd0c6d67c**. The required `test`
job passed in 17m30s. That commit contains the exact production/test source previously validated
locally at `b7d4f33`. All Phase 12–18 implementation checklists are complete within the documented
acceptance boundaries; no required Linux test failure remains. This addendum changes documentation
only and does not require repeating the unchanged numerical baseline. Check any subsequent hosted
run against its own head SHA rather than attributing this pass to a different commit.

Issue #12 remains open for draft PR #13 review/integration. The user explicitly requires the PR
to remain draft and prohibits merging into or modifying main; implementation completion does not
authorize integration. Main remains `d5748135188cd6f196a2f6310ea9934251260f93`.
Native Windows window/terminal interaction and physical GPU throughput remain external validation
limits, requiring a Windows machine and graphics hardware. They are not covered by the Linux
software-rendering evidence. Claude Code should review the documented formulation limits, run the
Windows/hardware checks before making platform claims, and await explicit merge authorization.

## User-reported build warning follow-up — 2026-10-10 UTC

Removed the unused private `LanczosCtx.kind` field and its initializer. The `lanczos` function
still uses its `kind` argument to select the modal/buckling metric before constructing the context;
no numerical operation, acceptance criterion, public API or test tolerance changes. The original
build warning is resolved rather than suppressed. `cargo test -p fea-core --test dynamics --locked
-j4` passed: **18 passed, 0 failed**, including independent modal/buckling references and agreement
between Lanczos and subspace iteration. Full workspace and subsequent hosted validation are tracked
in issue #12 / PR #13 against this follow-up's actual commit. The earlier full-suite counts remain
historical evidence until that new run completes. Local evidence: `lanczos-warning-dynamics.log`
and `.exit` under `/workspace/phase12-evidence/`.

## Interactive GPU extension — 2026-10-10 UTC

The user added new requirements after Phase 12–18 completion: visible generation progress, removal of ordinary full-history memory failures, in-window mode/display controls, and an interactive model/constraint editor with context-aware advisory ML. **Issue #12 is no longer implementation-complete for this expanded scope.** See [Phase 19–22 roadmap and current increment](issue-12-phase-19.md).

The first increment implements streaming/visible generation and native keyboard result controls, preserving all 240 integration/acceptance steps. It also supplies packed companion transfer, launch readiness/error diagnostics, compact modal shapes, shared mesh snapshots and one reused graphics device. Tested core dynamics 19/19, app library 816/816 (3 ignored), explicit GPU checks 2/2, terminal-only compile, and actual tmux/xterm/native-window interaction. New workspace/hosted results belong to the published increment's SHA, tracked in issue #12 / draft PR #13; earlier passes are historical.

The original large plate now animates. In the native modal window, Tab or [/] switches all already-computed modes; M toggles mesh, C contour, D deformation, R resets camera. Actual screenshot/pixel evidence is stored under `/workspace/phase12-evidence/phase19-*`. Cloud display setup uses signed/verified Debian packages extracted into `/workspace/ui-tools`; `/workspace/ui-tools/phase19-smoke.py` runs/cleans its own Xvfb (MIT-SHM disabled), xterm and tmux. Native macOS Metal and Windows hardware interaction remain unverified here.

Next implementation: visible native panels/legend and a pure native editor controller; versioned model authoring (prioritize aircraft frame/truss members and valid translational/rotational constraints), stable picking/entity identities, undo/redo, save/load and verified analysis workers. Build exact formulation/operation capability filters before adding an optional measured ML ranker over allowed actions. No editor, trained contextual ML or new in-window solve command is claimed in this increment. Preserve deterministic numerical/physical acceptance and offline fallback. Continue exclusively on `codex/fea-core-roadmap`; keep PR #13 draft and main unchanged.

## Superseded-generation follow-up — 2026-10-10 UTC

Published increment `447f187d04474be7cccd19b6cf01f492f2ca76b4` passed the complete local workspace gate: **1,819 passed / 0 failed / 60 ignored / 93 targets**, exit 0. Hosted [run 38042753267](https://github.com/Teased-oChroid-orrA/powershell_tool/actions/runs/38042753267) also succeeded at that SHA.

A subsequent small correction keeps an explicit “Inputs changed; previous animation finishing” elapsed-time status visible when an edited analysis supersedes a running job. It suppresses stale numerical progress and still discards stale results; another generation remains blocked until the worker finishes. The extended regression passed (1/1). The actual terminal/native smoke replay exited 0 and verified the caption during an analysis edit, then verified no stale native window opened (`phase19-stale-smoke.log/.exit`, `phase19-tui-stale-progress.png/.txt`). An initial smoke attempt changed the template, which resets the job instead of exercising the retained-worker path; the corrected replay changes Analysis. Full workspace and hosted checks for this follow-up are recorded against its final SHA in issue #12 / draft PR #13.

Remaining scope stays explicit: Phase 20 visible panels/legend/field selection, Phase 21 interactive authoring/constraints/analysis, and Phase 22 measured advisory ML. Continue on `codex/fea-core-roadmap`, preserving deterministic offline behavior; do not merge main or close the expanded issue as complete.

## Claude Code session — Phases 20-22 increment (2026-10-10, uncommitted)

State: branch `codex/fea-core-roadmap` at `ae05501` plus a **working-tree increment that has not been committed or pushed** (the user commits). Details, evidence and limits: `docs/issue-12-phase-19.md` (last section) and `app-tui/AGENTS.md` ("Native viewer and member editor"). New/changed: `app-tui/src/gpu_viewer/{controller,overlay,render,window,scene,transport,transient}.rs`, `gpu_viewer/editor/*`, `gpu_viewer/overlay.wgsl`, `shader.wgsl`, `fea-problem/src/members.rs` + `tests/members.rs`, `fea-core/src/structural.rs` (`member_axes` made public), Workbench key `f`, `--gpu-editor`, `Cargo.toml`/`Cargo.lock` (`font8x8 0.3`, pure, embedded font).

Exact next steps: (1) the user drives the editor by hand on macOS (`app-tui --gpu-editor demo.json`, or Workbench `f`): tools 1-6, click/drag, E to type values, Enter solve, Ctrl/Cmd+Z/S/O/N, panel clicks; report anything off. (2) Commit when asked, let hosted CI (including the Windows build) run. (3) Open: native authoring of solid `Problem`s (needs a product decision: duplicate or share the terminal Workbench editor), measured ML benefit (needs real recorded sessions; none exist), Windows hardware checks, `NlOptions::stall_tol` and the other items listed in the earlier handoff.


## Claude Code session — menus, advisor loop, shared row actions (2026-10-10, uncommitted)

Still uncommitted on `codex/fea-core-roadmap` (user commits). Added on top of the increment above: self-drawn menu bar / popup / right-click menus (`gpu_viewer/menu.rs`, `menus.rs`), advisor loop with opt-in recording, explicit training and a measured gate (`editor/learning.rs`, Advisor menu), `fea_workbench::model::apply_row_action` shared with the terminal Workbench. Details and evidence: `docs/issue-12-phase-19.md` (last section), `app-tui/AGENTS.md`.

Exact next steps: (1) user hand-tests the native window: F10, menu navigation, right-click on nodes/members/empty space, View > panels, Advisor > Record / Train (training refuses with a stated reason until >= 20 sessions). (2) Solid authoring in the native window: introduce an `Authoring` trait implemented by the member `Session` and a new `SolidSession` (rows from `fea_workbench::model::field_rows` + `apply_row_action`/`set_number`, preview mesh via `fea_problem::solve::preview`, solve worker like `editor/job.rs`, scenes via `ProjectSource`); make `window.rs`/`menus.rs` depend on the trait (menus currently take `Option<&Session>`); add overlay geometry for edges/holes/supports/loads and picking by `bottom/right/top/left`, `edgeN`, `outer`, `holeN`. (3) Command palette, menu mnemonics, default panel visibility (product decision), input-injection test seam for CI. (4) Real advisor benefit stays unproven until real recordings exist.

## Claude Code session — CAD chrome, solid authoring, single viewer (2026-10-10, uncommitted)

Still uncommitted on `codex/fea-core-roadmap`. Details: `docs/issue-12-phase-19.md` (last section) and `app-tui/AGENTS.md` ("Native window: CAD chrome, viewer and editors"). New: `gpu_viewer/{layout,chrome,dock,ribbon,ui}.rs`, `editor/{authoring,solid,worker}.rs` + `editor/solid/`, `editor/session/ui.rs`, renderer viewport + 6 overlay slots, `EditorKind` + `--gpu-solid`, Workbench `w`, consolidated `g`/`t`.

Exact next steps: (1) user hand-tests `app-tui --gpu-solid`, `--gpu-editor` and the consolidated viewer (menus, ribbon, tree, property manager, right-click, picking surfaces, solve). (2) Commit when asked and let hosted CI (Windows) run. (3) Open: command palette + menu mnemonics; face-level 3D picking and box-select; unit system; autosave/dirty-quit prompt; real text/glyph pipeline; per-element contours; advisor needs real recordings before any default-on.

## Claude Code session — benches, palette, selection (Phase 23, 2026-10-10, uncommitted)

Still uncommitted on `codex/fea-core-roadmap`. Read `docs/issue-12-phase-23.md` (status table, verification, known gaps) and
`docs/adr/ADR-014-native-cad-process-model.md`, then `app-tui/AGENTS.md` last section.

Exact next steps: (1) user hand-tests `cargo run -p app-tui --release`, Workbench `w`/`f`: Window > Bench, Ctrl+K, Alt+letter, Hole tool
click, Support/Load tool click lists, Shift-click and Shift-drag, Simulation studies, close-with-unsaved warning; report anything off with
`toolbench-debug.log`/stderr. (2) Commit when asked. (3) Next work: more benches (per-crate), advisor rules for solids, GUI polish list,
delete the dead terminal animation path.

## Claude Code session — parametric sketcher, feature history, recommendations (Phase 24, 2026-10-10, uncommitted)

Still uncommitted on `codex/fea-core-roadmap` on top of `ae05501` (user commits; never merge to main). Read, in order:
`docs/issue-12-phase-24.md` (what exists vs what was asked, verification, known limits, next steps),
`docs/adr/ADR-015-parametric-sketch-history.md`, `fea-problem/AGENTS.md` (History / Sketch solver contracts),
`app-tui/AGENTS.md` last section ("parametric sketch, feature history, recommendations").

New: `fea-problem/src/{sketch/{mod,solve,profile,infer}.rs,history.rs,advice.rs}`, `Problem::history` + `apply_history`, plane-frame /
revolve in `build::sketch_mesh`; `app-tui/src/gpu_viewer/editor/{sketcher/,rank.rs,solid/{sketching,advising}.rs}`; window seam
`Authoring::{hover,grab,drag,release_grab,take_view}`; models with a history hide row-based geometry editing.

Verified here: 1000+ app-tui lib tests, 38 rendering, fea-problem suites (`tests/sketch.rs`, `tests/advice.rs` new), clippy clean for
`gpu_viewer` and `fea-problem`, `--no-default-features` warning-free, a rendered sketch snapshot, window opens with a history file and
runs 8 s with empty stderr. NOT verified: real mouse / keyboard on the desktop, Windows, hosted CI. An independent review found no real
defect (its two MAJOR claims were checked: the arc-endpoint claim was false - test `arcs::arc_end_points_are_polygon_vertices_in_both_directions`;
the tolerance note led to `VIOLATION_TOL`). Self-found and fixed: replacing the model (template / Load) while sketching left a dangling
sketch mode; solving while sketching used stale geometry; added `Cancel sketch`.

Exact next steps: (1) user hand-tests Sketch mode: Insert > Sketch on Front plane (or the `Features` tab), L/R/C/A/P tools, click-click
drawing, snapping, D then click an edge and type a value, drag a corner, Fix point, Finish sketch, Features > Extrude, tree click on
Extrude/Sketch to edit parameters, Support on selection from the Sketch tab, Advisor menu; report with stderr / `toolbench-debug.log`.
(2) Commit when asked. (3) Next work in `docs/issue-12-phase-24.md` "Next steps": per-entity DOF colour + in-viewport constraint glyphs, Sweep and
datum/face planes, stable topology names + native arc `Shape`, thermal problem type before thermal advice; real recordings are needed
before the learned ranker can ever pass its gate.

## Phase 25 status (2026-10-11)

Done and verified (1200 tests): sketch visible after Finish; New / Templates / Open / Clear and real delete; manual-only
meshing with explicit states; AMR default 2 with a result report; refinement hints (rules, rings, panel, level ranker);
simulated training corpus + weighted prior + continual refits with an honest evidence label. Records: `docs/issue-12-phase-25.md`,
`docs/adr/ADR-016-manual-mesh-model-lifecycle-and-learning-prior.md`. NOT started: the item-4 feature list (table with
proposed approaches and the product decisions needed is in the phase-25 doc). Still uncommitted on top of `ae05501`; commit/push only when asked.


## 2026-10-16 — Current statement-of-work continuation entry

Resume the ongoing native CAD/advanced meshing epic #46 from [issue-46-resume.md](issue-46-resume.md) and [issue-46-current.json](issue-46-current.json). These identify current qualification/integration/next work; earlier dated branch/no-merge instructions above are historical. The user subsequently authorized merging qualified increments and continuing on new branches. Preserve OCCT and all unfulfilled priority gates. Phase 51 source `273e722` passes 180 CAD-enabled / 163 feature-off tests and app CAD lib/tests compilation, zero failures/ignored/warnings; [verification](issue-46-phase-51-verification.json). No development failure or external blocker. Shared analytic/spline source p-curve queries are next, followed by source finite/master/trim/body and the remaining roadmap.
