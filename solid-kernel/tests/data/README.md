# Native exchange regression fixtures

`tilted-cylinder.step` was generated for this project through `cadrum` 0.9 /
OpenCASCADE 8.0.1 using `Brep::cylinder([7,-4,2], 2, [1,2,3], 3)` and
`to_step_text()`. It declares millimetres; material is unspecified. The header's
fixed 1979 timestamp is exporter metadata, not provenance of an external design.

The rounded export is accepted by the analytic geometry checks but fails the
strict single-turn chart certificate. Explicit chart canonicalization within the
existing geometry tolerance must report every parameter adjustment, preserve all
world boundary samples, and pass sampled-facet intersection checks. The committed
fixture runs those regressions without requiring an OCCT-enabled test build.

## Sphere, cone and torus fixtures

`ball` (a sphere, refused), `filleted-box`, `cylinder-filleted-top`, `cylinder-chamfered-top`, `cone`, `frustum`, `hemisphere`, `capsule` and `torus` were written by
`cargo run -p solid-kernel --features occt --example write-shapes -- DIR` (OpenCASCADE 8.0.1 through `cadrum`; the shapes are
listed in that example: a 2 x 3 x 4 box with every edge rounded by 0.2, a R5 x 10 cylinder with its top edge rounded by 1.5 or
chamfered by 1.5, a cone R5 x 8, a hemisphere R4, a cylinder R3 x 8 capped by two spheres, a torus R6 / r2). They declare
millimetres and are exact analytic shapes, so `tests/curved_step.rs` compares the native import with closed-form volumes
without needing an OCCT-enabled test build.

## Corpus scope and continuation

The root [`nema17-motor-bracket.step`](../../../nema17-motor-bracket.step) has unknown exporter/provenance and explicitly declares millimetres; it is not established as NX/inches. Generated spline declaration/edge fixtures in Rust tests check their documented local contracts and do not automatically represent valid edited solids. Supported native diagnostic cases and refusals are indexed in the [support matrix](../../../docs/issue-46-native-support-matrix-2026-10-16.md). For current development/qualification and Claude Code continuation, use the [resume instructions](../../../docs/issue-46-resume.md) and [live pointer](../../../docs/issue-46-current.json).
