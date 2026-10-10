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
