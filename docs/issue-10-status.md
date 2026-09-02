# Issue #10 status: Engineering Toolbox Platform (scoped to Phase 1 only)

Issue #10 is a much larger epic than issue #11: a 14-toolbox "Engineering
Toolbox Platform" (fastener/hole/bushing repair, countersink, limits &
fits, GD&T, torque/preload, bearing loads, material removal, a machinist
calculator, measurement uncertainty, thread milling, and - toolbox #14 -
"Pressure Vessel/Tube Calculator, porting existing Bushing Toolbox
derivations"). Its own stated process is explicit: one toolbox at a time,
user authorization required between each, Phase 1 ("engineering-math":
units, `PrecisionPolicy`, calculation trace) is the "prerequisite for all
others."

**Scope of this doc, by direct user decision:** issue #11 (the standalone
Pressure Vessel epic) surfaced the same units/precision/calc-trace gap
issue #10 Phase 1 describes. Rather than build that foundation twice (once
scoped-down for #11 alone, once "properly" if #10 is ever picked up), the
user chose to build issue #10's real Phase 1 now, then put the rest of
issue #10 (Phase 2 "engineering-core", and all 14 toolboxes) on hold and
resume #11 on top of it. **Nothing beyond Phase 1 is in scope here** - no
toolbox #2-14, no constraint/tolerance/geometry consolidation, no GD&T, no
migration of `bushing-solver`'s existing tolerance/geometry code. Re-verify
against real repository state before resuming any of that later; this doc
does not attempt to plan it.

## Verification against real code

- `profile_capabilities` (issue #10 Phase 0 names it as an audit target
  alongside the Bushing Toolbox): a real sibling project at
  `~/Claude/Projects/profile_capabilities`, confirmed to exist - but a
  grep of its own source for `PrecisionPolicy`/`CalcTrace`/`UnitSystem`/
  `units` found nothing. This repo's own UI already borrowed its visual
  theme (glassmorphism/color tokens - see `docs/epic-ui-performance-and-
  design.md`), not any calculation infrastructure; there is none there to
  reuse for Phase 1.
- No `engineering-math`/`engineering-core` module, crate, or file exists
  anywhere in `powershell_tool` (confirmed by filename search).
- Repo-wide grep for `PrecisionPolicy`/`CalcTrace`/generic units types -
  zero matches, same finding as `docs/issue-11-status.md` reached
  independently for issue #11's own Phase 12. Both epics are asking for
  infrastructure that plainly does not exist yet, from two different
  documents - consistent, not contradictory.
- `serde`/`serde_json` (v1, `derive` feature) are already workspace-
  standard dependencies (`app`, `native-search`, `search-core`, `cli` all
  use them) - the natural choice for Phase 1's "serializable configuration
  surviving application restarts" requirement, not a new dependency to
  introduce.

## Phase 1 scope actually being built

Per issue #10's own Phase 1 description, kept to exactly what it names -
no more:

- **Units/quantity system**: "at minimum investigate/support: inch;
  millimeter; force; pressure; torque; angle; area; volume; length; mass
  where required." A `Quantity` value-with-unit type plus conversion, for
  the unit families actually named.
- **`PrecisionPolicy`**: decimal places, significant figures,
  operation-aware rounding (issue #10's own split: addition/subtraction
  governed by decimal places, multiplication/division by significant
  figures), full internal precision with no intermediate rounding, only
  final display rounding.
- **Calculation trace**: a reusable structure recording inputs, formula,
  intermediates, and the unrounded final result - generic, not tied to any
  one toolbox's specific fields (unlike `bushing_workbench.rs`'s existing
  `DerivationBlock`, which is real and works but is hand-built per-tool).
- **Serializable configuration**: user display preferences (decimal
  places, unit choice, etc.) persisted independently of calculation logic,
  via `serde_json`, matching this workspace's existing convention.

New crate: `engineering-math` (name matches the epic's own terminology
verbatim), zero dependencies beyond `serde`/`serde_json`, added as a new
workspace member. Nothing in it is bushing- or pressure-vessel-specific;
it is a leaf crate other crates (`mechanics-core`, `pressure-vessel-solver`,
eventually `bushing-solver` if a later pass chooses to migrate its own
display formatting onto it) depend on, never the other way around.

See `docs/issue-10-phase-1.md` for what was actually implemented and how
it was verified.

## Explicitly on hold (not started, not planned further here)

Toolboxes #2-14, "engineering-core" (Phase 2: constraint solving,
tolerance primitives, shared geometry), GD&T, and every other section of
issue #10 beyond Phase 1. Re-run this same verification discipline before
picking any of it up - do not assume this doc's Phase-1-only snapshot
still describes the repository by then.
