# Issue #12 — Phase 12, first increment and Claude Code handoff

Started 2026-10-09. Phase 12 is **in progress**, not complete. Issue:
https://github.com/Teased-oChroid-orrA/powershell_tool/issues/12
Work only on `codex/fea-core-roadmap`; do not commit to or merge into `main`.
Audited baseline: `d5748135188cd6f196a2f6310ea9934251260f93`, identical to the remote
roadmap branch when this session began. Initial checkout was clean. No worktree was created.

- [x] Source audit and repository-required checks identified.
- [x] Complete untouched CI baseline executed.
- [x] First numerical acceptance change committed and pushed.
- [x] Failure regressions reproduced, thermal suite and explicit partial-slip check passed.
- [x] Draft PR and initial issue progress posted.
- [x] Full post-change workspace validation and committed handoff (results below).
- [ ] Remaining Phase 12 programme; do not mark the whole phase complete.

## Audit verified against current source

Read root `AGENTS.md`/`CLAUDE.md`, `README.md`, `fea-core/AGENTS.md`, `docs/AGENTS.md`,
CI, Cargo manifests/lockfile, ADR-012/013, and the implementations/tests below.
CodeGraph tools and index are unavailable here; direct source tracing is the documented fallback
(`docs/codegraph-workflow.md`, section 15). No CodeGraph results are claimed.
The historical audit's “49 files across 8 modules” cannot be reconstructed without its file list;
capabilities and relationships were rechecked directly rather than treating that count as evidence.

| Audit claim | Current source and evidence | Assessment |
|---|---|---|
| Modal, buckling, mass and implicit transient exist | `dynamics.rs`; `tests/dynamics.rs`: axial/bar bending frequencies, Euler columns, modal superposition, energy and second-order time convergence | Confirmed; preserve and harden |
| Heat and one-way thermomechanical loading exist | `thermal.rs`, `loads.rs`, `analysis.rs`; `tests/thermal.rs` | Confirmed; steady residual was reported but not enforced |
| Verification reports / strategy selection exist | `strategy.rs`, `validate.rs`, `report.rs`; `tests/strategy.rs`, `tests/report.rs` | Confirmed, analysis-specific rather than unified |
| Sensitivities / sizing exist | `sensitivity.rs`, `optimize.rs`; global finite-difference and series-bar tests | Confirmed; ADR-013's consequences are stale |
| Neo-Hookean exists | `material.rs::neo_hookean_update`, nonlinear material dispatch; `tests/hyperelastic.rs` | Confirmed; no need to reimplement |
| 3D partial-slip fix exists | `nonlinear.rs::refine_consistent`: right-preconditioned GMRES, retain only a lower true residual; `tests/contact_sweep.rs` | Source confirmed; explicit 24-case run recorded below |
| Adaptive remeshing is separate | `adapt.rs::refine` takes caller mesher/solver callbacks; `Requirements::max_zz_error` cannot remesh | Confirmed; integrating this is later roadmap work |
| Nonlinear ladder delegates acceptance | `solve_nonlinear_ladder` checks completion and invokes caller `verify` | Confirmed; common physical acceptance remains future work |
| Generalized fields / structural elements are missing | `Physics::dim` maps translations to 2/3; thermal pattern is scalar; `ElementKind` lists only continuum triangles/quads/tets/hexes | Confirmed; generalized DOFs and beam/shell/plate/truss work remain Phases 13/14 |

Direct callers of `solve_heat_steady` currently occur only in `fea-core/tests/thermal.rs`.
`Loads::temperature` consumes its output as a structural load. Kernel dependents include
`fea-problem`, `lug-solver`, `edge-check`, `eccentric-bushing` and `app-tui`; the full workspace
suite covers those dependents. No GUI, element formulation, dependency or lockfile changes are needed.

## First small improvement

Steady heat acceptance must enforce the documented final-system residual, require a finite positive
tolerance, and reject non-finite temperature/residual/heat outputs. Use the normwise backward error `||K T - f|| / || |K| |T| + |f| ||` on free nodes,
and report its value and allowed tolerance when rejecting a finite result. Picard's field-change test
alone is insufficient. Remove the existing thermal test's NaN/zero-input escape hatch.

Add a separate analytical generation/convection case: `-k T'' = q`, `T(0) = T0`,
`-k T'(L) = h (T(L) - Ta)`. Check the full quadratic temperature profile and independently derived
source, fixed-boundary and convection heat flows, plus residual and global heat balance.
Add a uniform-temperature, zero-heat-flow regression: the old load/reaction normalization
degenerates to round-off divided by round-off and falsely rejects this valid state when enforced.
The backward-error scale uses matrix/vector magnitudes before cancellation; `hypot` accumulates
norms without squaring overflow, and a non-finite denominator is rejected. This changes the meaning
of `HeatSolution::rel_residual` as documented on the field; physical heat balance remains a separate
diagnostic/check, not silently certified by a backward error.
Add failure regressions for non-finite loads, invalid tolerances and a tolerance below the measured
residual of the same deterministic solve. Acceptance adds no iteration-budget increase or tolerance relaxation.

## Validation and environment

Linux x86_64; rustc 1.99.0 (`b940084d7`, 2026-09-28), cargo 1.99.0. CI uses stable.
Five available CPUs; compilation limited to four jobs. Approximately 30 GiB initially free.
Rust was absent and installed under `/workspace/toolchains` using official rustup over verified TLS;
Cargo verifies registry checksums. No system package or application secret is required.

Activate each shell before the commands below:

```sh
export CARGO_HOME=/workspace/toolchains/cargo
export RUSTUP_HOME=/workspace/toolchains/rustup
export PATH="$CARGO_HOME/bin:$PATH"
cd /workspace/powershell_tool
```

| Command | Result |
|---|---|
| `cargo test --workspace --locked -j 4` on untouched baseline | Exit 0; 1,763 passed, 0 failed, 57 ignored, 89 completed targets including zero-test doc targets |
| `cargo test -p fea-core --locked -j 4 --test thermal` before solver fix, new regressions added | Exit 101; 13 passed, 3 failed; reproduced acceptance of invalid tolerance, NaN output and unmet residual |
| `cargo test -p fea-core --locked -j 4 --test thermal` after normalization correction | Exit 0; 17 passed, 0 failed, 0 ignored |
| Thermal suite after initial guard | Exit 0; 16 passed, 0 failed; followed by a new zero-flow regression that exposed degenerate normalization |
| `cargo test -p fea-core --locked -j 4 --test thermal a_uniform_steady_temperature_is_accepted_without_heat_flow` before scale correction | Exit 101; 0 passed, 1 failed; valid uniform field rejected with old residual 1.13 |
| `cargo test -p fea-core --locked -j 4 --test contact_sweep partial_slip_3d_measurement -- --ignored --nocapture` | Exit 0; 1 test passed, 5 filtered out; 0 of 24 seeded 3D partial-slip cases failed (4.94 s test time) |
| `cargo test --workspace --locked -j 4 --no-fail-fast` after changes | Exit 0; 1,768 passed, 0 failed, 57 ignored, 89 completed targets |
| `git diff --check` | Passed |
| `bash /workspace/toolchains/setup-rust.sh` | Exit 0; repeatable tool activation/install and `cargo fetch --locked` verified |

Full current-machine logs and exit files are in `/workspace/phase12-evidence/` (outside checkout,
retained with the prepared machine). The baseline includes every default CI test, but **not** ignored
soaks, benchmarks, or manual replay cases. Zero-test targets are not counted as validated behavior.
The existing `LanczosCtx::kind` dead-code warning is a warning, not a failure. CI intentionally has
no whole-tree fmt/clippy gate because of pre-existing drift/warnings.
Windows reference/release builds, real-terminal GUI rendering and the full ignored performance/soak
suite are not run. No new performance claim is made. Exact historical M1 timings are not reproduced.

## Tracking and remaining work

Git reads work through the supplied HTTPS proxy. GitHub API read requests (`gh issue view`,
`gh api` and a curl CONNECT probe) initially returned `Forbidden`; `GH_TOKEN` is present (value never inspected).
The issue body was read through its accessible GitHub web page. `api.github.com` was added to the
configuration draft; retry after the user confirmed saving still returned Forbidden, including an
outside-sandbox read. No duplicate credential requirement was added. Access subsequently propagated: draft PR #13
and the initial progress comment on issue #12 were successfully created. The earlier Forbidden
responses were a temporary environment-network blocker, not missing GitHub credentials.

Reusable `install_script` and `start_skill` were saved, including explicit Rust activation, existing
isolated checkout/no worktree guidance, branch restriction and test commands. Saving is not execution
or publication; the user must review/save the final settings and publish the environment.

Remaining Phase 12 work: independent added modal/buckling/transient/hyperelastic/partial-slip cases,
unified residual/equilibrium/energy acceptance where applicable, and broader capability documentation.
The new thermal benchmark addresses only one part of the checklist. Phases 13-17 are not begun.
Inspect finite-data/option acceptance in other analysis entry points as a separate reviewable increment;
this session does not certify all entry points against malformed or overflowing input.

## Next steps for Claude Code

1. Read this handoff, issue #12 and the local instructions; verify branch and clean state. Never reset
   local work to match a remote or merge into main.
2. Reuse the Rust activation above; rerun relevant checks after any edit. Inspect full logs when present.
3. Confirm remote tracking/PR/issue results recorded in the dated addendum below. If GitHub API access
   remains blocked, restore the environment's api.github.com proxy access and retry the prepared PR/comment;
   do not duplicate an existing PR or replace issue text blindly.
4. Choose the next small Phase 12 analytical/acceptance increment. Preserve existing numerical tolerances
   and evidence; keep generalized DOFs, new structural elements and GUI changes for their roadmap phases.

## Tracking addendum (2026-10-09)

Implementation commit: `a852085950b88d0159d323d9eb8f76bd378c3e4a`
(`fea-core: enforce verified steady heat acceptance (#12)`), pushed only to
`codex/fea-core-roadmap`. Draft PR: https://github.com/Teased-oChroid-orrA/powershell_tool/pull/13
(base `main`, draft only; no merge). Initial issue progress:
https://github.com/Teased-oChroid-orrA/powershell_tool/issues/12#issuecomment-6091146519

Full post-change command completed with exit 0: **1,768 passed, 0 failed, 57 ignored**.
All 89 test/doc targets completed; the five additional thermal tests account for the difference from
baseline. Required Linux CI validation of this increment is complete; the broader Phase 12 checklist
is still incomplete. There are no unresolved environment blockers for this increment.

Documentation/handoff commit title: `docs: record Phase 12 audit and validation handoff (#12)`.
Resolve its exact SHA with `git log -1 --format=%H -- docs/issue-12-phase-12.md`; the final issue
progress comment also records both commits. Keep the PR as a draft for review.

An environment reconnection occurred after validation finished. The branch, uncommitted documentation,
implementation commit and all evidence/exit files were checked afterward and preserved. This is not
an independent fresh-task rerun or a claim of Windows/GUI validation.

## Continuation addendum (2026-10-10 UTC)

The user authorized continuing through the remaining issue #12 roadmap. Aircraft frame and
truss members were selected explicitly as Phase 16's actual aerospace use case. The original
first-increment scope above is historical; this continuation preserves the branch and draft PR.

Commit `31b76be` hardens modal/buckling option and constraint validation, finite eigenpair
acceptance, transient initial-state/history/damping/time/load validation, and finite transient
state/energy acceptance. Structural transient results now expose effective-system backward
errors (required <= 1e-8) at initialization and every step. Load callbacks run once per time.
Thermal transient rejects nonfinite capacities/time/initial fields/constraints/output.
Additional independent tests cover fixed-fixed axial frequencies and mass orthogonality,
pinned Euler buckling and reference-force scaling, second-order free harmonic response with
energy conservation, hyperelastic rigid-rotation/objectivity, and a default nonmatching 3D
partial-slip resultant case. No tolerance was weakened.

Targeted `cargo test -p fea-core --locked -j 4 --test dynamics --test thermal --test hyperelastic
--test contact_sweep` passed: 44 passed, 0 failed, 5 ignored. Evidence:
`/workspace/phase12-evidence/phase12-regressions.log`. A full workspace run of commit `31b76be`
was started before generalized-field edits; its results will be recorded when complete.
Generalized-field tests passed (5/5), including continuum displacement/reaction equivalence
and independent clamped-bar one-way thermomechanical reaction. The coupling API explicitly
converts absolute temperatures to changes from a finite reference temperature and identifies
only ThermalToStructural direction; no monolithic/two-way coupling is claimed.

Remaining acceptance work includes unified orchestration/strict nonlinear acceptance in Phase 15,
full milestone validation and continued roadmap documentation. Do not infer full roadmap completion
from targeted checks or from a nonlinear driver's `complete()` termination classification.
