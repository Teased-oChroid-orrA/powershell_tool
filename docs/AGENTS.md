# docs/

## Purpose
Owns: navigation index into this repo's design/history documentation - ADRs,
per-issue phase-by-phase implementation logs, epic status reports, and
standalone topic docs (architecture, deployment, FFI, benchmarking, etc.).

**This is a navigation index, not a content digest.** `docs/` is ~132k tokens
of Markdown. Do not read it all; use the tables below to jump to the one file
that has the answer.

Does not own: the narrative content itself - don't duplicate phase-doc or ADR
prose into this file or into root `CLAUDE.md`. Link to the source file.

## Code Map

### Find It Fast — by architecture decision
| Looking for the decision behind... | Go to |
|---|---|
| Where Rust-native search sits vs. the C#/.NET app | `docs/adr/ADR-001-rust-native-search-boundary.md` |
| Why Tantivy over alternatives | `docs/adr/ADR-002-tantivy-primary-search-engine.md` |
| Unified extraction architecture | `docs/adr/ADR-003-unified-extraction-architecture.md` |
| FM-Index / suffix array / bioinformatics-library rejections | `docs/adr/ADR-004`, `-005`, `-006` (all Rejected: no incremental update) |
| Where the index is persisted | `docs/adr/ADR-007-index-persistence-location.md` (**superseded by ADR-011**) |
| Incremental indexing strategy | `docs/adr/ADR-008-incremental-indexing-strategy.md` |
| FFI serialization format (JSON) | `docs/adr/ADR-009-ffi-serialization-strategy.md` |
| Multi-index vs. Tantivy-only | `docs/adr/ADR-010-multi-index-vs-tantivy-only-architecture.md` |
| Final in-folder index location (current answer) | `docs/adr/ADR-011-in-folder-index-location.md` |
| Full ADR index + status column | `docs/adr/README.md` |

### Find It Fast — by epic/issue implementation history
| Looking for how epic/issue N was implemented | Go to |
|---|---|
| #2 — Native Offline Search Engine (Tantivy) foundation | `docs/native-search-assessment.md` (Phase 1 recon) → `docs/adr/*` → `docs/issue-2-status.md` (DoD checklist; **closed**) |
| #6 — index-first/incremental search engine (80-section epic) | `docs/issue-6-status.md` (gap analysis) → `docs/issue-6-phase-1.md` .. `phase-16.md` → `docs/issue-6-validation-report.md` (**closed**, all 16 phases + final validation) |
| Edge-distance cross-check (stress superposition / FE / allowables vs the legacy `Fbru + 0.8 p` check; merged to main, including the contact FE) | `docs/edge-distance-crosscheck.md` |
| Lug Analysis: rigid-pin contact FE (options considered, method, validation numbers, performance, what is not done) | `docs/lug-analysis.md` |
| Lug solver upgrade pass (15 accuracy/robustness items, status and measured outcome of each) | `docs/lug-solver-upgrade-plan.md` |
| General FEA kernel `fea-core` (2D/3D elements, assembly, faer solver, validation, measured speed, phase plan; Phase 8: FEA Workbench, toolboxes on the kernel, point location, rigid-body check) | `docs/fea-core.md`; decisions `docs/adr/ADR-012-general-fea-kernel.md`, `docs/adr/ADR-013-eigen-and-transient-methods.md` (eigen / transient / adaptive strategy) |
| Eccentric bushing toolbox (spin capacity, maximum offset; realistic meshed pin, direct spin simulation, edge-limited housing) + the FEA core upgrades it drove (follower loads, arc-length contact, anisotropy, rotation, consistent friction tangent) and every improvement-list item's outcome | `docs/eccentric-bushing.md` (sections 8-9), `docs/fea-core.md` Phase 9 |
| MIL-HDBK-5J material library (1,375 conditions in the Bushing / Pressure Vessel pickers; provenance, conversion rules, what is estimated) | `docs/material-handbook.md` |
| Index audit 2026-10 — freshness precision, scheduling, version stamp, measured rejections | `docs/index-audit-2026-10.md` |
| #8 — perf/indexing/scalability evidence report | `docs/issue-8-status.md` (single-file report against #6's shipped state) |
| #9 — general-purpose indexed query engine (regex-aware exec) | `docs/issue-9-status.md` (single-file; initial investigation/first pass, no phase docs follow) |
| #10 — 14-toolbox "Engineering Toolbox Platform" | `docs/issue-10-status.md` (explicitly scoped to Phase 1 only) → `docs/issue-10-phase-1.md`. **Remaining 13 toolboxes are on hold, not started** — don't assume this epic is done because a `-status.md` exists |
| #11 — Pressure Vessel Stress/Failure-Mode/Min-Thickness Analyzer | `docs/issue-11-status.md` → `docs/issue-11-phase-1.md` .. `phase-15.md` (no phase-8; folded elsewhere) (**closed**, 15 phases shipped). Also summarized in root `CLAUDE.md` — that summary is being trimmed to point here instead of duplicating it |

### Find It Fast — by current/live status
| Looking for current status of... | Go to |
|---|---|
| app-egui feature parity vs. `app/` and the approved mockup | `docs/app-egui-parity-checklist.md` — **CLOSED/OBSOLETE**, `app/`+`app-egui/` both deleted (see root `CLAUDE.md`) |
| Rust/Dioxus rewrite, overall (`app/`-specific) | `docs/rust-rewrite-status.md` — **CLOSED/OBSOLETE**, `app/` deleted |
| Toolbench (multi-tool dashboard shell, `app/`-specific) | `docs/toolbench-status.md` — **CLOSED/OBSOLETE**, `app/` deleted; current dashboard shell is `app-tui/src/widgets/` |
| Bushing Workbench tool (`app/`-specific) | `docs/bushing-workbench-status.md` — **CLOSED/OBSOLETE**, `app/` deleted; current implementation is `app-tui/src/toolboxes/bushing/` |
| UI performance / visual-polish rework | `docs/epic-ui-performance-and-design.md` — epic doc that fed the issue-11 phase 10-15 UI rework and supersedes rust-rewrite-status's old polish TODOs |

### Find It Fast — standalone topic docs
| Looking for... | Go to |
|---|---|
| Overall project structure / what goes where | `docs/architecture.md` |
| FFI contract (native-search ⟷ app) | `docs/ffi.md` |
| Benchmarking methodology/results | `docs/benchmarking.md` |
| Search matching semantics (Unicode, literal/regex) | `docs/search-semantics.md` |
| .NET/WinUI (`src/`) deployment | `docs/deployment.md` |
| Rust (`app/`/`cli/`) deployment | `docs/deployment-rust.md` |
| Offline build (`native-search/` crate) | `docs/offline-build.md` |
| Pre-Rust native-search architecture recon | `docs/native-search-assessment.md` |

## Entry Points
| Task | Start Here |
|------|------------|
| Onboarding to this repo's history/architecture | `docs/architecture.md`, then `docs/adr/README.md` |
| Full CodeGraph-first engineering runbook (summarized in root `CLAUDE.md`) | `docs/codegraph-workflow.md` |
| Checking if a proposed change conflicts with a past decision | `docs/adr/README.md` (status column) before writing code |
| Finding what's still outstanding on app-egui | `docs/app-egui-parity-checklist.md` (not the issue-11 phase docs) |
| Understanding why root `CLAUDE.md`'s epic-#11 section is short | This file's Contracts section, then `docs/issue-11-status.md` |
| Writing up a new epic/issue's status doc | Follow the shape of `docs/issue-11-status.md` (status) + `issue-11-phase-N.md` (per-phase log) |

## Contracts
- **Phase docs and status docs are an append-only historical record.** Never
  delete or rewrite one when the work it describes is superseded — add a new
  phase/status file, or a note at the top of the old one, instead. Precedent:
  ADR-007 was superseded by ADR-011 by adding ADR-011 and marking 007
  "Superseded", not by deleting or editing 007's content.
- **Don't duplicate phase-doc/epic narrative into `CLAUDE.md` or any
  `AGENTS.md` node.** Root `CLAUDE.md` ballooned to 2273 lines by inlining
  issue #11's narrative in full; it's being slimmed to link here instead.
  This file must stay an index, not a digest, or it repeats that mistake.
- `issue-N-status.md` naming usually (not always) means a closed, point-in-
  time epic summary. `docs/issue-10-status.md` is the exception — it's a
  status doc for a *partially executed, on-hold* epic (Phase 1 shipped, rest
  explicitly not started). Verify with the file's own opening lines before
  treating any `-status.md` as "done".
- Standalone `*-status.md` / checklist files without an issue number
  (`toolbench-status.md`, `rust-rewrite-status.md`,
  `bushing-workbench-status.md`, `app-egui-parity-checklist.md`) are living
  documents, updated in place as that area evolves — not historical record.

## Boundaries

### Always
- Add new phase/status content as a new file (`issue-N-phase-M.md`) or a
  dated addendum inside a live status doc; keep prior phases intact.
- Link to the specific ADR/phase/status file rather than re-explaining its
  contents elsewhere in the repo.

### Never
- Delete or overwrite a completed `issue-N-phase-*.md` or a closed
  `issue-N-status.md` when the work it describes is superseded.
- Copy phase-doc or ADR narrative into `CLAUDE.md`, this file, or any other
  `AGENTS.md` node — link instead.

### Verify First
- Before treating any `*-status.md` as closed/superseded, check its own
  opening lines and `git log -1 --format=%ai -- <file>` — the filename
  pattern alone is not reliable (see `issue-10-status.md`).

## Pitfalls
- `docs/issue-11-phase-*.md` numbering skips phase 8 (jumps 7 → 9) — that's
  not a missing file; phase 8's work was folded into another phase.
- `docs/issue-10-status.md` looks like a closed epic summary (same filename
  pattern as #2/#6/#8/#11) but explicitly scopes itself to "Phase 1 only",
  with the rest of the 14-toolbox platform "on hold (not started, not
  planned further here)" per its own text.
- ADR-007 (index-persistence-location) reads as a normal Accepted decision
  but is superseded by ADR-011 — read 011 for the actual current answer.
- Reading only `issue-11-phase-14.md`/`-15.md` for "what's left" on
  `app-egui` gives a stale answer — the current, maintained list is
  `app-egui-parity-checklist.md`, which exists specifically because the
  phase docs' deferred-item prose aged silently.
- A status/phase doc's numeric section refs (e.g. "epic §54") point into the
  *external* GitHub issue's own section numbering, not to anything inside
  `docs/` — don't go looking for a "§54" file here.

## Downlinks
| Area | Node | What's There |
|---|---|---|
| ADRs | `docs/adr/README.md` | Ordered index of ADR-001..011 with a status column (Accepted/Rejected/Superseded) |
