# Phase 125 — curved boundary quality and fillet-rim face ownership

Epic #46 / #63. Base: `main` `4aaeed5` (Phase 124 plus spline-edge decimation). This increment addresses two defects exposed by the same native-versus-OCCT study: curved lattice meshes had tetrahedra below the solver quality gate, and the flat face beside a tangent fillet acquired extra loaded area.

## Boundary-layer repair

`fea-core::meshing::boundary_repair` works after the signed-distance snap. It tries interior smoothing and reconnection, boundary sliding, surface peeling/promotion, guarded edge collapse, penalty-mode smoothing, and finally bounded splitting/relaxation. The transactions retain positive orientation and check cavity volume and boundary-face direction. `TetOptions::repair` is opt-in at the generic kernel boundary; `solid-kernel::mesh_volume` enables it for CAD volume meshes. This keeps the existing error-driven adaptive mesher's numerical path unchanged while applying the repair where the Phase 124 curved-boundary defect was measured. `mesh_solid_reported` exposes a repair report without changing the existing `mesh_solid` return type.

A pre-existing bored-cylinder test caught an outward move during repair: a point could satisfy its selected end face while moving farther beyond the bore. The candidate check now limits the positive distance to every known face to its starting depth (with a small local allowance). The clean-target test's worst radial escape is 0.0125 at `h=0.28` and 0.00355 at `h=0.14`, both below its unchanged 0.03 limit. The opt-in repair test covers a sphere, a capped cylinder and a torus at several sizes: zero inverted or sub-gate elements, at most 10% element growth, volume within 0.5%, named-face areas within 3%, deterministic output and a valid quadratic mesh.

## Tangent fillet and default import size

`SurfaceSolid::face_dists` now gives the signed distance and direction to the nearest point on a triangle, rather than the unbounded triangle plane. An off-surface feature-snap target falls back to signed-distance projection. Boundary-face classification uses a 0.01-element-size face tolerance so a strip of tangent fillet is not tagged as the neighbouring flat face. The fresh STEP import size resolves a curved feature at about two elements per estimated radius, capped by a 25,000-element volume estimate; all-flat solids retain the previous size rule.

The regression uses a 12 × 14 × 16 mm box with 2 mm fillets. Its flat top is exactly 80 mm². At three element sizes, the tagged area stays within the asserted 2.5%/0.5% bounds and no node leaves the tested body allowance. A uniform traction on that face gives reaction within 1% of traction × 80 mm². A larger, tiny-fillet box checks that the default estimate respects the element cap. The same suite checks cylinder curvature and an all-flat solid.

## Qualification and limits

- macOS arm64, offline locked dependencies: `cargo test -p fea-core -p fea-problem -p solid-kernel --no-default-features --locked --offline -j1` passed from this worktree's own Cargo target. The focused `tetmesh_repair` suite passed 3 tests (1 diagnostic ignored); `fillet_rim_tagging` passed 5 (1 diagnostic ignored). The existing error-driven remeshing test and bored-cylinder high-order test passed at their original limits.
- One pre-existing unused `stage_times` test helper was removed after the full build reported it. A shared Cargo target had previously supplied an artifact from another worktree, so all final evidence here uses this worktree's isolated target.
- This is not a general quality proof for imported CAD or an OCCT retirement decision. The ABC solver-quality survey is recorded separately in `issue-46-abc-corpus-2026-10-18.md`; Windows, hosted CI, and OCCT-enabled qualification are not included in the command above. `occt_retirement_approved` remains false.
