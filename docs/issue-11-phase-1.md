# Issue #11 Phase 1: `mechanics-core` extraction

Moves `lame.rs` and `materials.rs` out of `bushing-solver` into a new,
standalone, zero-dependency crate `mechanics-core` - the architecture
decision from `docs/issue-11-status.md`. A pressure-vessel tool (or any
future one) can now depend on the general Lamé/materials math without
pulling in `bushing-solver`'s own bushing-specific geometry/countersink/
bearing/reamer/tolerance code.

## What changed

- `git mv bushing-solver/src/{lame,materials}.rs mechanics-core/src/` -
  moved verbatim, not rewritten (both were already general - no bushing-
  specific types in either, confirmed in `docs/issue-11-status.md`'s own
  gap analysis).
- New `mechanics-core/Cargo.toml` (zero dependencies) and `src/lib.rs`
  (`pub mod lame; pub mod materials;`).
- `bushing-solver/lib.rs`: dropped its own `pub mod lame;`/`pub mod
  materials;`; `bushing-solver/Cargo.toml` gained a `mechanics-core = {
  path = "../mechanics-core" }` dependency.
- `bushing-solver/src/solve.rs` (the only internal consumer): every
  `crate::lame::...`/`crate::materials::...` call site rewritten to
  `mechanics_core::lame::...`/`mechanics_core::materials::...` - no
  re-export shim, per the approved plan ("re-exports nothing new").
- `app/Cargo.toml` gained a direct `mechanics-core` dependency (it calls
  the crate directly now, not just transitively through `bushing-solver`).
  `app/src/bushing_visualizer.rs`/`bushing_workbench.rs`/`main.rs`: every
  `bushing_solver::lame::...`/`bushing_solver::materials::...` reference
  (code and doc comments) rewritten to `mechanics_core::...`.
- Root `Cargo.toml`'s workspace `members` gained `mechanics-core`.

Every rewrite was mechanical (a rename, confirmed by reading the full
`git diff` before committing - no logic changed anywhere).

## Verification

- `cargo build -p bushing-solver` / `-p mechanics-core` / `-p app` /
  `--workspace`: all clean, zero warnings introduced.
- `cargo test -p bushing-solver`: **43/43** (down from the pre-refactor
  53 by exactly the 10 tests that moved with `lame.rs`/`materials.rs` -
  9 lame tests + 1 materials test - confirming nothing was lost, only
  relocated).
- `cargo test -p mechanics-core`: **10/10** - the same tests, same
  assertions, passing in their new home.
- `cargo test --workspace`: unchanged pass counts everywhere else
  (149 search-core, both `differential*.rs` golden fixtures against real
  TS engine output, etc.) - this was a pure move, and the full suite
  proves it.

## Next

Phase 2 (pressure-vessel geometry model + closed/open-end axial-stress
research) - see `docs/issue-11-status.md`'s stage list.
