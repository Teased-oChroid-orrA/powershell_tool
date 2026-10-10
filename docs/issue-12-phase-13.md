# Issue #12 — Phase 13 generalized fields and coupling

2026-10-10 UTC. Work only on `codex/fea-core-roadmap`; draft PR #13, no main merge.

`fields.rs` provides validated node-major translation/rotation/temperature mapping, sparse
symmetric element assembly, named field constraints, and result metadata. The continuum mapping
is exactly `node * dim + axis`; the existing block layout remains in use for continuum solvers.
Generalized assembly stores the lower sparse scalar triangle and reuses sparse Dirichlet
elimination/Cholesky. Duplicate fields/element DOFs, nonsymmetric or nonfinite matrices, conflicting
prescriptions and overflow are errors. Nonzero prescribed values are eliminated exactly.
Finite-state, quadratic-energy and normwise backward-error acceptance precede returned results.

`coupling.rs` exposes one-way steady thermal-to-linear-structural loading on the same mesh.
Absolute heat temperatures are converted using the explicit stress-free reference temperature.
A preexisting nodal temperature load is refused rather than overwritten. Uniform `delta_t` is
additive. No structural feedback on conductivity, transient, plastic or monolithic coupling is
implemented or implied. Stress recovery must use the resulting temperature-change field.

Verification: `cargo test -p fea-core --locked -j 4 --test fields` passed 5/5, including
mapping validation, analytical spring/bar assembly with nonzero boundary displacement, continuum
migration displacement/reaction equivalence, malformed input rejection, and the independent
clamped-bar reaction `-E alpha mean(delta T)`. Evidence `fields.log`/`structural.log` under
`/workspace/phase12-evidence`. Full current-source workspace validation remains required before
marking the milestone complete. Claude: inspect these interfaces before structural consumer edits;
retain existing continuum numbering and distinguish termination from verified acceptance.

## Final interface review addendum

Constraints now retain their full field layout: equal-sized thermal and mechanical layouts cannot
be reinterpreted. Sparse system internals are encapsulated. Generalized residual acceptance is
component-wise `max_i |(K u-f)_i|/(|K| |u|+|f|)_i`, so mixed equation units cannot mask one another.
The earlier normwise description is historical. A regression verifies cross-layout rejection.
