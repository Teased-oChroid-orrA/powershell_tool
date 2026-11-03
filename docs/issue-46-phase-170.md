# Phase 170: native STEP spindle-torus faces and cone faces on the far nappe

Issue #46. Follows [Phase 169](issue-46-phase-169.md). After that phase, `screw.step` was the one corpus file that OpenCASCADE meshes and native does not: it stopped on its head, a spindle torus ("horn and spindle tori are not implemented"). Measured against the owner's retirement gate (`issue-46-abc-and-retirement-gates-plan.md`). No gate, tolerance, budget, cap or timeout was changed. `occt_retirement_approved` stays false.

## Cause

Two surface features were missing, the second hidden behind the first.

1. **Spindle torus.** The head of `screw.step` is a `TOROIDAL_SURFACE` with major radius 8.25 mm below its minor radius 54.87 mm. In a plane through the axis, such a surface is two circles of radius `minor` centred `major` either side of the axis. They overlap, so the surface has an outer part (the "apple", the circle on the point's own side) and an inner part (the "lemon", the circle on the far side), which meet at two cusps on the axis. Native's torus chart is the conformal ring-torus chart and needs major > minor, so the reader refused the surface.
2. **Cone face on the far nappe.** STEP's cone radius is `radius + t tan(semi_angle)`, which turns negative past the apex: the point is then on the far side of the axis. The countersink face #944 under the head lies wholly there. Native charts a cone on the near nappe only, so the face's edges did not project ("unresolved edge endpoint/surface projection").

Correction to Phase 169: that document says face #874 holds the cusp inside it. It does not. The screwdriver slot (y in [-2.076, 0.424], the axis at y = -0.826) removes the axis from the head. Face #874 is bounded by a half circle (#879) and a spline (#18); faces #803 and #709 are bounded by a circle arc, a spline and a meridian circle (#782). All three faces lie on the lemon part, above the equator. No face of `screw.step` holds a cusp; the new hand-written fixture does.

## Change

`solid-kernel`:

- `analytic_step.rs`: `Surface::Spindle { frame, major, minor, lemon }`. A `TOROIDAL_SURFACE` with major < minor reads as a spindle; a horn torus (major == minor) is still refused by name. The per-face set-up that already re-frames a sphere for each face now also sets up a spindle (`localized_spindle`) and a cone (`localized_cone`) from the face's boundary loops.
- `curved_surface.rs`:
  - `localized_spindle` picks the part (apple or lemon) whose meridian circle the face's boundary samples fit, and the side of the equator they lie on. It turns the frame's `z` to the side the geometric normal points to, so the part is a graph over the frame's xy plane and the projection onto that plane is a chart counter-clockwise about the normal. It refuses a face whose boundary crosses the equator (not a graph), or a sample farther than the geometry tolerance from the chosen part.
  - The spindle chart is that projection. It is not conformal: lengths along the meridian shrink by the slope. The cusp on the axis is an ordinary chart point; only its normal is undefined.
  - Spindle arms in the period, raw, residual, distance, projection, point and normal functions. The residual is measured to the face's own part; the distance used by the edge-locus check is to the whole surface (a meridian circle runs over both parts, see the comment on `on_surface`).
  - The normal is STEP's `S_u x S_v`, which is OpenCASCADE's convention. On the lemon part (where `R + r cos v < 0`) it points toward the centre of the generating circle. The head faces of `screw.step` (sense `.F.`) only give a closed, outward shell with this convention.
  - `localized_cone`: a face whose boundary lies on the far nappe (and none of it past the apex on the near one) is the same point set as the cone with its axis reversed and its origin moved past the apex by its own distance from it. `S_u x S_v` is the same for both, so the face keeps its orientation. Other cone faces are returned unchanged.
- `planar_step.rs`, `chart_mesh.rs`: the spindle follows the sphere's path (projected chart, seam as a slit). `face_identity.rs`: a spindle face has the torus kind.
- `native_surface.rs`: the triangle normal lookup `normal_at_world(...).unwrap()` now returns a geometry refusal. A sampled triangle centred on a cone's or spindle's axis has no normal and used to panic.

`fea-problem/tests/occt_vs_native.rs` and `solid-kernel/src/occt.rs` (comparison evidence only): the table prints each tessellation's enclosed volume (`vol`) and the B-rep's exact volume from OpenCASCADE (`Brep::volume`, `BRepGProp::VolumeProperties`).

## Tests

- `tests/curved_step.rs`, two new cases (fixtures documented in `tests/data/README.md`):
  - `spindle-cap.step` (hand-written): a cylinder R8 x 10 capped by the lemon part of a spindle torus (major 7, minor 25), with the cusp inside the cap face. Exact volume 2340.3249 mm³ (Simpson's rule in the test); native 2338.6622 mm³ at chord 0.005 (-0.071 %).
  - `far-nappe-chamfer.step`: `cylinder-chamfered-top.step` with the chamfer cone declared on its far nappe. It reads to the original's volume exactly. Without `localized_cone` it refuses ("unresolved edge endpoint/surface projection").
- `curved_surface.rs` unit tests: a spindle face is charted on its own part and side; the spindle normal equals `S_u x S_v` numerically and is counter-clockwise in the chart; a spindle face across its equator, or off its part, is refused; a cone face on the far nappe keeps its points and normal.
- Suites, macOS arm64, release: `solid-kernel` 321 passed, 0 failed, 4 ignored; `fea-problem` 220 passed, 0 failed, 7 ignored (Phase 169: 317 and 220).

## `screw.step`

- `step-inventory --surface --geometry-tolerance 5e-3`: closed shell, 408 vertices, 812 triangles, signed volume +3753.70 mm³, facet intersection check passed. With `--max-line-mm` 1, 0.3 and 0.1 the volume is 3761.93, 3761.76 and 3762.23 mm³.
- At the strict default tolerance (1e-7 mm) the read refuses: spline edge samples lie 1.2e-4 mm off the torus. That refusal is correct. Production reads use the tolerant re-read (`Options::foreign_cad`, tolerance chord / 10).
- Same-STEP comparison with solve (`table`, default size, macOS arm64, release):

| Path | Surface volume | Tets | Min sJ | Max edge ratio | Gate (cells < 0.1, all on the boundary) | Mesh | Solve | umax | U | Reaction z |
|---|---|---|---|---|---|---|---|---|---|---|
| OCCT | 3764.43 mm³ | 167,236 | 0.019 | 19.1 | fails (43) | 99 s | 59 s | 2.138e-3 | 0.2140 | -383.3 |
| native | 3753.65 mm³ | 166,669 | 0.019 | 18.7 | fails (46) | 142 s | 66 s | 2.078e-3 | 0.2010 | -371.7 |

  The B-rep's exact volume (OpenCASCADE) is 3788.27 mm³. Both tessellations lie below it (OCCT -0.63 %, native -0.91 %).
- The reaction is the 10 MPa traction times the loaded faces' tessellated area, so native's loaded area is 3.0 % smaller than OCCT's. That accounts for most of the 2.8 % lower umax and the 6 % lower strain energy. The likely cause is that native samples edges to the chord alone, while OpenCASCADE also limits the angle per segment, so native's polygons cut more off small circles; this was not measured face by face.

## Corpus

Same 35 files as Phases 165, 168 and 169 (local copy, not committed), mesh only (`NO_SOLVE=1`), macOS arm64, release, 978 s, OpenCASCADE 8.0.1 from cadrum's prebuilt `occt-8_0_1_rev3-aarch64_apple_darwin` (downloaded with the owner's approval).

Summary: **35 models; OCCT meshed 3, passed the default gate 1; native meshed 3, passed the gate 1** (Phase 169: native meshed 2). Native now meshes every corpus file that OpenCASCADE meshes.

| File | Exact volume | OCCT surface volume | Native surface volume | OCCT tets / gate | Native tets / gate |
|---|---|---|---|---|---|
| `linkrods` | 3.84700 | 3.83843 (-0.22 %) | 3.84269 (-0.11 %) | 84,014 / ok | 84,011 / ok |
| `nist_ftc_11_asme1_rb` | 5129.39 | 5106.87 (-0.44 %) | 5082.50 (-0.91 %) | 651,236 / fails (43) | 647,264 / fails (32) |
| `screw` | 3788.27 | 3764.43 (-0.63 %) | 3753.65 (-0.91 %) | 167,236 / fails (43) | 166,669 / fails (46) |

The other 32 rows are unchanged from Phase 169: OpenCASCADE refuses them at the mesher (too many elements at the default size, or an open tessellation), native at the read. The exact volume column also shows that OpenCASCADE reads `nist_ftc_08_asme1_ap242-e1-tg` as an inside-out solid (exact volume -5.03e5 mm³, against +5.04e5 for the other `ftc_08` files).

## What this shows and does not show

- It closes the measured gap on the retirement gate's mesh criterion: on this corpus, native meshes every file OpenCASCADE meshes.
- It does not make native read every file OpenCASCADE *imports*. OpenCASCADE imports all 35 (it computes an exact volume for each); native's read refuses 32, on the classes listed in Phases 168 and 169 (edge/surface disagreement, the surface chart work budget, unsupported radius or placement, and others).
- Native's surface volumes are up to 0.91 % below the exact volume at the harness's chord, against OpenCASCADE's 0.63 % at the same nominal chord. Both lie on the same side; neither changes the gate result.
- The spindle chart is a projection, so a long, steep meridian band would mesh with stretched triangles. `screw.step`'s faces are not steep enough to show that; no corpus file tests it.
- Measured on macOS only. The hosted Windows STEP workflow runs on the pull request.

## Follow-ups

- Chord policy for small circles (native samples to the chord only): measure the loaded-face area difference on `screw.step` face by face before changing anything.
- The remaining native strict-read classes on the corpus (Phase 168 table, updated in Phase 169).
