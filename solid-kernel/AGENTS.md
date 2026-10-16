# solid-kernel

> TL;DR: B-rep solids for the FEA preprocessor (ADR-019). Always built: `SurfaceSolid` (a closed, outward, face-tagged
> surface mesh with BVH signed-distance and per-face distance queries) and `mesh_volume` (tetrahedra through
> `fea-core::tetmesh`). Feature `occt` (off by default; the `cadrum` crate links a prebuilt static OpenCASCADE 8.0.1, no C++
> toolchain; set `OCCT_ROOT` to an extracted tarball if its build-script download fails): `occt::Brep` (extrude a sketch profile, union / cut / intersect,
> fillet, STEP in and out, face-tagged tessellation).

## Purpose
Owns: the surface-mesh solid and its distance queries, the OpenCascade modelling wrapper, the tessellation into face-tagged triangles.
Does not own: the volume mesher (`fea-core/src/tetmesh.rs`), the problem schema (`fea-problem`), any UI.

## Code Map
| Looking for... | Go to |
|---|---|
| `SurfaceMesh`, `SurfaceSolid` (weld, closed-manifold check, outward orientation, `signed_distance`, `faces_near`, `nearest_face`, `face_areas`), `mesh_volume` | `src/lib.rs` |
| Triangle BVH, closest point on a triangle with its region (face / edge / vertex) | `src/bvh.rs` |
| OpenCascade: `Brep::extrude/boolean/fillet_all/faces/tessellate/solid/write_step/read_step` | `src/occt.rs` (feature `occt`) |
| Analytic tests (box, tube) | `tests/surface.rs` |
| OpenCascade tests | `tests/occt.rs` (`cargo test -p solid-kernel --features occt`) |

## Contracts
- A `SurfaceSolid` is only built from a **closed manifold** (every edge in exactly two triangles, opposite directions); an open or
  non-manifold tessellation is refused with a reason, an inside-out one is turned around.
- The sign of the distance uses angle-weighted pseudo-normals, so it is right at edges and vertices.
- Face tags are B-rep face indices in the kernel's face order; the mesher names the faces `face{tag + 1}`.
- The tessellation deflection bounds the geometric error: use a small fraction of the boundary element size.

## Pitfalls
- `cadrum`'s `Face::id` is not unique (an extrusion's two caps share one) and is session-local (not kept by STEP). `tessellate`
  splits a shared id into connected pieces and matches them to faces by centre; a face without triangles makes it fail
  instead of guessing. Never use the kernel id as a persistent face name.
- Meshing a solid thinner than about two boundary elements is not resolved (see `fea-core/src/tetmesh.rs`).
- The `occt` feature downloads or needs a ~30 MB prebuilt OCCT: keep it opt-in so default builds and tests stay offline. ADR-019 addendum records why `cadrum` replaced `opencascade`.

- **Tessellation/welding correction (2026-10-15)**: welding checks Euclidean distance in neighbouring grid cells, not bucket
  equality, and volume integration uses a translated origin. `occt::Brep::tessellate` locally splits coincident interior
  curved-face chords by projecting their midpoints onto the exact trimmed kernel face; face tags and manifold acceptance
  remain intact. The NEMA17 fixture needs eight extra triangles; global very fine tessellation was prohibitively expensive.
  Kernel validity alone does not prove a particular triangle tessellation is a closed manifold.
