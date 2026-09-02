# Issue #11 status: Pressure Vessel Stress, Failure Mode & Minimum Thickness Analyzer

A 14-phase epic asking for a new tool: given vessel geometry/material/
pressure/boundary conditions and a required minimum Margin of Safety,
evaluate the full applicable Lamé stress state, determine which failure
modes apply, compute a margin for each, identify the controlling (minimum)
margin, and solve for the minimum wall thickness satisfying the requirement
across every applicable mode - always via the full Lamé thick-wall
solution, never a silently-substituted thin-wall approximation. The epic
explicitly demands verification before implementation and names several
pieces of infrastructure it assumes already exist. This doc is that
verification pass, in the same spirit as `docs/issue-6-status.md`/
`docs/issue-8-status.md`/`docs/issue-9-status.md` - a "bottom line" verdict
up front, evidence below, willing to cut scope where the evidence says so.

**Bottom line: the core ask is acceptable and buildable now, directly on
top of real work already in this repo (the `lame.rs` Lamé module this repo
gained from the bushing-workbench session immediately preceding this one
is already general enough to reuse as-is). The full 14-phase scope is not
realistic as a single push - several failure modes the epic lists need
data or infrastructure this repo does not have and has no way to validate
against yet. See "Verdict" below for the v1 cut and the explicit backlog.**

## What the epic assumes exists, checked against real code

| Epic assumption | Status | Evidence |
|---|---|---|
| "the existing full Lamé equations module" | **Confirmed, and better than assumed** | `bushing-solver/src/lame.rs`'s `lame_stress_at_radius`/`lame_constants` take arbitrary inner/outer radius and **independent** `p_inner`/`p_outer` - internal, external, and combined pressure already work, not just internal-only. Result of this repo's own recent refactor (commit `702df00`), verified against real TS engine golden output, 9 unit tests including an independent textbook cross-check (`diametral_interference_compliance_matches_the_textbook_shrink_fit_formula`). |
| "the existing materials library/database" | **Confirmed** | `bushing-solver/src/materials.rs`: `Material { e_ksi, sy_ksi, fbru_ksi, fsu_ksi, ftu_ksi, nu, alpha_u_f }`, 17 real materials, zero Dioxus/UI coupling. Already distinguishes yield (`sy_ksi`) from ultimate (`ftu_ksi`) from bearing/shear ultimate - the exact distinction the epic's Materials Module Requirements section asks for. |
| "existing pressure-vessel calculations" | **Not found** | Repo-wide grep for `PressureVessel`/`pressure_vessel` - zero matches. `lame.rs` computes the right physics but has never been applied to a standalone pressure-vessel problem; every current caller is bushing-specific. |
| "precision/engineering-math infrastructure" (Phase 12) | **Not found** | Repo-wide grep for `PrecisionPolicy`/`precision_policy` - zero matches. No shared precision system exists anywhere in the repo to reuse. |
| "calculation trace functionality" (Phase 11) | **Not found as generic infra** | No `CalcTrace`/`CalculationTrace` type anywhere. The closest analog is `bushing_workbench.rs`'s own "Show more detail" derivation view (`DerivationBlock`/`FORMULAS`/`derivation_value`) - real, working, but hand-built per-tool, not a reusable system. |
| "units infrastructure" | **Not found** | No generic units/unit-conversion module. `bushing-solver` is imperial-only by explicit, documented design (`CLAUDE.md`'s own "Why dioxus-native" section doesn't cover this, but `bushing-solver/Cargo.toml`'s module doc does: "imperial units only for v1"). |
| Failure criteria (Von Mises, Tresca, buckling, fatigue, creep) | **Not found** | Zero matches for `VonMises`/`Tresca` anywhere. No failure-mode evaluation code of any kind exists yet - this is genuinely new territory for the repo. |
| "the Bushing Toolbox" | **Confirmed, and directly reusable** | `app/src/bushing_workbench.rs` - the Step-workflow shell, `DesignStatusRail`/`CheckGauge` margin-visualization components, and the worst-case Lamé derivation view built in the immediately preceding session are a proven template for exactly this kind of "geometry → stress → margins → governing check" tool. |

## A gap the epic doesn't mention: the axial-stress formula doesn't transfer

`lame.rs`'s only axial-stress formula is `axial_scale * nu * (sigma_r +
sigma_theta)`, where `axial_scale = axial_constraint_factor *
axial_length_factor` - a **bushing-specific mechanical end-constraint**
model (how much a press-fit bushing's ends are pinned). A real pressure
vessel's axial/longitudinal stress under a **closed end** comes from a
different physical mechanism entirely - internal pressure pushing on the
end caps, transmitted as a uniform axial stress through the wall:
`sigma_z = p * a^2 / (b^2 - a^2)` (a real, general force-equilibrium
result, not a Poisson-coupling estimate). An **open end** (e.g. a pipe
with pressure but no end caps to react against) has zero axial stress from
pressure. Reusing the bushing formula for a pressure vessel would be
physically wrong. This is new physics this repo needs to add, not
something to port from an existing module.

## No live reference engine to validate against

`bushing-solver` was ported from a real, running TypeScript engine
(`~/Claude/Projects/engineering.toolbox`) and validated against its real
golden output (`tests/differential*.rs`) - the gold-standard verification
this repo's own engineering culture prefers. No equivalent exists for a
pressure-vessel/failure-mode tool. Correctness here has to be established
against textbook analytical reference cases (Shigley's *Mechanical
Engineering Design*, Roark's *Formulas for Stress and Strain*, or
equivalent) instead, with the source cited per adopted equation - weaker
proof than a live differential test, and worth being honest about rather
than presenting with the same confidence.

## Verdict: acceptable, with an evidence-based scope cut for v1

The core ask - full-Lamé stress, yield/Von Mises/Tresca/ultimate margins,
a controlling-mode-aware minimum-thickness solver, an explanation system,
reusing the now-general `lame.rs`/`materials.rs` - is well-scoped and
buildable now. Worth doing.

The full 14-phase scope is not realistic as a single push:
fatigue/creep/thermal-stress need data the `Material` struct doesn't carry
(S-N curves, creep parameters, temperature-dependent properties);
buckling/collapse/spherical vessels are separate physics with no existing
foothold in this repo; and there's no live reference engine to validate
any of it against, so each addition has to earn its own analytical proof
rather than being included because the epic listed it. This mirrors
`bushing-solver`'s own precedent (its `Cargo.toml`'s comment: "the duty/
process/approval layers are not ported... see
docs/bushing-workbench-status.md for the exact scope decision") and issue
#8's own conclusion that a successful outcome can be "no code change" when
the evidence doesn't support the full ask.

**v1:** cylindrical vessels; internal + external + combined pressure;
closed-end and open-end axial stress (new); yield + Von Mises + Tresca +
ultimate margins (all backed by data the materials library already has);
minimum-thickness solver across all v1 failure modes with controlling-mode
tracking; per-mode applicability explanation; worst-case step-by-step Lamé
derivation view (generalizing the pattern just proven for the Bushing
Workbench).

**Explicit backlog (documented, not silently dropped):** spherical
vessels, buckling/shell instability/collapse, fatigue, creep, thermal
stress, code-compliance certification framing.

**Correction after cross-checking issue #10:** the paragraph above
originally also put a shared `PrecisionPolicy`/units system in this
backlog, reasoned as "nothing in the repo needs one yet beyond this single
new tool." That reasoning no longer holds - issue #10 (a separate,
0-comment, not-yet-started "Engineering Toolbox Platform" epic) already
specifies exactly this module (units/quantity system, `PrecisionPolicy`,
a reusable calculation-trace system) as its own Phase 1, explicitly
"prerequisite for all others," and names the Pressure Vessel Calculator as
one of its 14 planned toolboxes. Per direct user decision, issue #10's
Phase 1 foundation is being built first (as its own tracked unit of work -
see `docs/issue-10-status.md`), then issue #10 is put on hold and this
epic (#11) resumes on top of it. The pressure-vessel tool's precision/unit
display and calculation trace will consume that foundation rather than
inventing tool-specific formatting, matching issue #10's own "no toolbox
may independently implement rounding/significant figures/unit conversion"
requirement.

## Architecture decision: extract a shared `mechanics-core` crate

`lame.rs` and `materials.rs` currently live inside `bushing-solver`, which
also carries bushing-specific geometry/countersink/bearing/reamer/
tolerance code a pressure-vessel tool has no use for. Per the epic's own
"there must ultimately be one authoritative Lamé implementation" and
materials-module requirements, both modules move into a new, zero-
dependency workspace member, `mechanics-core` - moved verbatim (already
general, no bushing-specific types in either module), a **safe-refactor**
with zero intended behavior change, proven by the full existing
differential/golden test suite staying green after the move.
`bushing-solver` becomes a `mechanics-core` consumer, same as the new
`pressure-vessel-solver` crate this epic adds. See
`docs/issue-11-phase-1.md` for the executed move and its verification.

## Trackable stages

Per this repo's own convention for large epics (a status doc plus one
`docs/issue-N-phase-M.md` per completed phase):

- Phase 0 (this doc) - gap analysis and scope decision.
- Phase 0.5 - issue #10 Phase 1 foundation (`engineering-math` crate:
  units, `PrecisionPolicy`, calculation trace), built and verified as its
  own unit before this epic resumes - see `docs/issue-10-status.md` and
  `docs/issue-10-phase-1.md`.
- Phase 1 - `mechanics-core` extraction (foundational, unblocks the rest).
- Phase 2 - pressure-vessel geometry model + closed/open-end axial-stress
  research and implementation.
- Phase 3 - materials audit for failure-mode data needs.
- Phase 4 - failure-mode + margin-of-safety engine (yield, Von Mises,
  Tresca, ultimate).
- Phase 5 - minimum-thickness solver.
- Phase 6 - UI (`pressure_vessel_workbench.rs`, `ToolId::PressureVessel`).
- Phase 7 - full-suite validation, build, CI.
- Phase 8 - docs rollup / final completion report.

Each phase's `docs/issue-11-phase-N.md` records what was built, what was
verified, and what evidence supports it.

## Final completion report (v1 scope)

```text
PRESSURE VESSEL STRESS, FAILURE MODE &
MINIMUM THICKNESS ANALYZER (v1)

STATUS: COMPLETE (v1 scope - see explicit backlog above)

Repository Analysis: PASS - existing Lame module and materials library
  located, evaluated, and found already general enough to reuse verbatim
  (better than the epic itself assumed).
Research: PASS - closed-end axial stress derived from first principles
  (axial force equilibrium) and verified against a cited textbook
  reference case (Shigley, ID 4in/OD 6in/5000psi -> 4000psi axial,
  13000psi hoop, exact match).
Lame Validation: PASS - mechanics_core::lame unchanged from its own
  prior verification (real TS engine golden output) plus 3 new tests for
  closed_end_axial_stress.
Materials Module: PASS - audited (Phase 3), confirmed sufficient,
  no new fields added.
Stress Analysis: PASS - internal, external, and combined pressure;
  closed and open end conditions; inner and outer surface evaluation.
Failure-Mode Validation: PASS - yield, Von Mises, Tresca, ultimate;
  critical location genuinely evaluated per mode (a wrong assumption
  about which surface governs was caught by running tests, not by
  inspection - see docs/issue-11-phase-4.md).
Thickness Solver: PASS - bisection, monotonicity verified before being
  relied on, a genuine infeasibility case (asymptotic pressure limit)
  confirmed correct.
Precision Validation: PARTIAL - engineering-math (issue #10 Phase 1) was
  built and is available; NOT yet wired into pressure-vessel-solver's own
  calculations (v1 uses plain f64 + fixed-decimal display, same as
  bushing-solver/bushing_workbench.rs already do) - a real, explicit gap,
  not silently skipped. See "Known Issues" below.
Regression Tests: PASS - full workspace suite green throughout every
  phase, including both differential.rs/differential_countersink.rs
  golden fixtures against real TS engine output (unaffected).
UI Validation: PARTIAL - built and code-reviewed
  (cargo build/test clean); actual rendering in the real dioxus-native
  window NOT independently verified - no local GUI capability in this
  environment (same standing limitation as every Bushing Workbench UI
  phase this session). Needs a screenshot round-trip with the user.

Controlling Failure Mode Analysis: VALIDATED (governing() picks the true
  minimum across all four modes at both surfaces, per-mode, re-evaluated
  at every thickness-solver candidate)

Known Issues / explicit gaps (not silently dropped):
- engineering-math (units/PrecisionPolicy/calc-trace) is built but not
  yet consumed by pressure-vessel-solver or its UI - v1 uses the same
  plain-f64-plus-fixed-decimals approach bushing-solver already uses.
  Wiring it in is real follow-up work, not done here.
- No formal calculation-trace UI (issue #11 Phase 11) - the derivation-
  view pattern proven for the Bushing Workbench was not ported to this
  tool in v1.
- UI not visually verified (see UI Validation above).
- CI not triggered for this epic's commits - per explicit user
  instruction, deferred until the user says issue #11 is complete.
- Explicit backlog (see "Verdict" above, unchanged): spherical vessels,
  buckling/collapse, fatigue, creep, thermal stress, code-compliance
  framing.

Existing Modules Reused:
  mechanics_core::lame (Lame thick-wall equations, already general)
  mechanics_core::materials (material property library, already general)
  app::components::{NumberField, MaterialField, CheckGauge, CheckRowData,
    margin_class, fmt_margin, margin_dot_class} (extracted from
    bushing_workbench.rs for this purpose)

Existing Modules Refactored:
  bushing-solver (lame.rs/materials.rs extracted out to mechanics-core;
    every internal call site updated, zero behavior change, proven by
    the existing test suite staying green)
  app/src/bushing_workbench.rs (5 components extracted to components.rs,
    zero behavior change, proven the same way)

Existing Tools Migrated:
  None required migration - bushing-solver was refactored to depend on
  the extracted mechanics-core rather than "migrated" in the sense of
  changing its own behavior.

READY FOR USER REVIEW:
YES - pending a screenshot round-trip to verify the UI actually renders
as intended (this environment cannot run the app directly).
```
