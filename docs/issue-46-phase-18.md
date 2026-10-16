# Issue 46 — Phase 18: runnable PLC/seed diagnostics with progress

Source `53a223f`, corrected/final source `f349ed3`, exposes the previously verified diagnostic stages in the feature-off `step-inventory` example. `--plc` admits the globally checked sampled boundary; `--star-seed X,Y,Z` requires explicit finite kernel coordinates in **millimetres**; `--trial-kernel` optionally requests the one reported interior trial. The existing explicit `--canonical-surface` and `--repair-orientation` policies combine with these stages; strict defaults remain unchanged. `--help` prints usage without opening a file.

Progress identifies PLC admission, seed construction and each kernel trial phase. Repeated start updates are suppressed. Both candidate and retained quality are visible, so a declined move does not appear unattempted. No material properties are inferred, no solver analysis is performed and a diagnostic seed is not advertised as a trusted production mesh. The fixed developer seed budget is 100,000 cells; exhaustion remains an error.

Examples, from the repository root (no OCCT feature):

```sh
cargo run -p solid-kernel --example step-inventory --locked --offline -j1 -- nema17-motor-bracket.step --plc
cargo run -p solid-kernel --example step-inventory --locked --offline -j1 -- solid-kernel/tests/data/tilted-cylinder.step --canonical-surface --star-seed 7.4,-3.2,3.2 --trial-kernel
```

Seven standalone cases pass: actual bracket PLC (469 vertices/978 facets/1,467 segments); explicitly canonicalized tilted-cylinder seed (60 points/114 cells/114 preserved facets); expected strict-chart refusal; expected hole-bearing bracket seed refusal; nonfinite kernel rejection; missing kernel rejection before file access; help success. The tilted trial explicitly declines a candidate that does not satisfy both quality guards, preserves the supplied point and prints candidate metrics. Unique phase progress and candidate output are checked from captured process streams.

During progress-output cleanup, two intermediate example builds failed with stale variable names/type inference (E0425/E0282); both were corrected. Final locked/offline example build and all seven standalone cases pass. No library implementation changed in this increment: reuse Phase 17's 70 CAD-enabled/53 feature-off checks and Phase 16's app compilation. [Verification](issue-46-phase-18-verification.json).

No new dependency, tolerance/cap increase, hosted CI, production capability/main change or OCCT retirement. General native CAD/exact trims/assemblies, general PLC recovery/refinement/cavities/regions, sliver/conditioning qualification, P2/P3/final parity and separate merge approval remain open. This CLI is a diagnostic tool, not a replacement for the production CAD/FEA workbench.
