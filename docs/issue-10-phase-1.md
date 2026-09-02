# Issue #10 Phase 1: `engineering-math` foundation

Implements issue #10's Phase 1 exactly as scoped in `docs/issue-10-status.md`
- units, `PrecisionPolicy`, calculation trace, serializable display
config - as a new, standalone, zero-engineering-domain-dependency
workspace member. Nothing beyond Phase 1 (toolboxes, "engineering-core"
constraint/tolerance/geometry work) is touched.

## What was built

New crate `engineering-math/`, four modules:

- **`units.rs`** - `Unit`/`Quantity`, covering exactly the families issue
  #10 names (length, force, pressure, torque, angle, area, volume, mass).
  Each family has one base unit (Inch/PoundForce/Psi/InchPoundForce/
  Degree/SquareInch/CubicInch/PoundMass, matching this workspace's
  existing imperial-first convention); every conversion factor is an
  exact, cited constant (1 in = 25.4 mm, 1 lbf = 4.4482216152605 N, 1 lbm
  = 0.45359237 kg - all exact by the 1959 international yard-and-pound
  agreement; 1 psi = 6894.757293168361 Pa, derived from the length/force
  constants, not independently asserted). Cross-family conversion is a
  typed `UnitMismatch` error, never a silent garbage value.
- **`precision.rs`** - `PrecisionPolicy`/`RoundingRule`, implementing
  issue #10's own split: addition/subtraction rounds by decimal places,
  multiplication/division by significant figures, and a separate
  `display` rule for the one place a value's precision is actually
  reduced for a human to read. The absolute precision rule ("no
  intermediate rounding") shapes the API itself: nothing here runs
  automatically mid-calculation - callers write plain `f64` arithmetic as
  every other crate in this workspace already does, and only explicitly
  opt into rounding at a named boundary. Significant-figures rounding and
  display formatting are real, tested implementations (magnitude-aware,
  correct across positive/negative/very small/very large values), not a
  naive `.round()` call.
- **`trace.rs`** - `CalcTrace`/`CalcStep`, a generic, toolbox-agnostic
  record of label/formula/named-inputs/unrounded-result per step. Unlike
  `bushing_workbench.rs`'s existing `DerivationBlock` (real, working, but
  hand-written per-tool), any future toolbox can build one of these and a
  shared UI component can render it the same way.
- **`config.rs`** - `DisplayConfig`, JSON-serializable (via `serde_json`,
  already a workspace-standard dependency - no new dependency introduced)
  user display preferences, kept as a genuinely separate type from
  `PrecisionPolicy` per issue #10's "calculation precision != display
  preferences" rule - nothing in `DisplayConfig` can feed back into a
  calculation's math.

Added to the workspace `Cargo.toml`'s `members`. Zero dependents yet -
`bushing-solver`/`app` are unchanged in this phase; wiring the pressure-
vessel work (issue #11) onto this foundation happens next, once #11
resumes.

## Verification

- `cargo build -p engineering-math`: clean, zero warnings.
- `cargo test -p engineering-math`: **25/25 passing**, covering: exact
  unit-conversion constants (inch/mm, psi/Pa, degree/radian, lbm/kg,
  in^2/mm^2), round-trip conversion (in -> mm -> in returns the original
  value), cross-family conversion producing a typed error, significant-
  figure rounding across magnitudes (including negative values, zero, and
  a magnitude-crossing case like 9.999 -> 10.0 at 3 sig figs), the
  decimal-place vs. significant-figure split actually producing different
  numbers (not both silently defaulting to one rule), display formatting
  preserving trailing zeros a bare rounded `f64` would lose, calculation-
  trace step ordering and unrounded-result storage, and JSON round-trips
  for `PrecisionPolicy`/`CalcTrace`/`DisplayConfig` (including a
  malformed-JSON case producing a real error, not a silent default).
- `cargo build --workspace` / `cargo test --workspace`: clean, all
  pre-existing tests (53 bushing-solver, 149 search-core, both
  differential golden fixtures, etc.) unaffected - this phase adds a new,
  currently-unreferenced crate and touches nothing else.

## What's deliberately not here

Issue #10's Phase 2 ("engineering-core": constraint solving, tolerance
primitives, shared geometry) and all 14 toolboxes remain on hold per
direct user decision - see `docs/issue-10-status.md`. `bushing-solver`'s
own `tolerance.rs`/`geometry.rs` are untouched; no attempt was made to
migrate or consolidate them onto this new foundation in this phase.
