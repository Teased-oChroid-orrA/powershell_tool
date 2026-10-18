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

## Assembly and edit fixtures

`assembly-mapped-items.step` is the box of `assembly-two-instances.step` instanced by two `MAPPED_ITEM`s of one `REPRESENTATION_MAP`
(`examples/assembly-probe.rs -- mapped write`). The `edit-plate*.step` files are a 40 x 30 x 10 plate with a 4 mm hole at (12, 15)
and four edits of it (`examples/write-shapes.rs`): the hole moved to x = 14, a second 3 mm hole at x = 30, the plate 12 thick, the
hole 5 mm; `edit-plate-centre-hole` / `edit-plate-twin-holes` are one hole at x = 20 and two at x = 15 and 25. They test
`face_identity`.

## Edges that are not lines or circles

`tilted-hole` (a 40 x 30 x 20 block, 4 mm hole tilted along (0.5, 0.2, 1): ellipse edges), `t-junction` (cylinder R6 x 30 with a
perpendicular branch R4) and `cross-bore` (cylinder R8 x 30 with a cross bore R3): degree-6 B-spline intersection curves and faces with
holes (`examples/write-shapes.rs`, tested by `tests/curved_edges.rs`).

## Fillets on intersection curves (spline-surface faces)

`cross-bore-filleted` (R8 x 30 cylinder, R3 cross bore, 0.5 fillets), `t-junction-filleted` (R6 cylinder, R4 branch, 1.0) and `tilted-hole-filleted` (the tilted-hole block, 0.5) come from `examples/write-shapes.rs`: the fillet along each intersection curve is a rational B-spline surface (degrees 2 x 13, C0 at every interior knot, closed in the second parameter). They need `geometry_tolerance_mm: 1e-4` (tested by `tests/spline_surfaces.rs`, `tests/face_identity.rs`).

## Corpus scope and continuation

The root [`nema17-motor-bracket.step`](../../../nema17-motor-bracket.step) has unknown exporter/provenance and explicitly declares millimetres; it is not established as NX/inches. Generated spline declaration/edge fixtures in Rust tests check their documented local contracts and do not automatically represent valid edited solids. Supported native diagnostic cases and refusals are indexed in the [support matrix](../../../docs/issue-46-native-support-matrix-2026-10-16.md). For current development/qualification and Claude Code continuation, use the [resume instructions](../../../docs/issue-46-resume.md) and [live pointer](../../../docs/issue-46-current.json).
