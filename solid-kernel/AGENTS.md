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

- `src/cad.rs` owns backend-neutral profile/boolean/face contracts and the production shape handle; the existing OCCT adapter dispatches through it. `occt` retains compatibility exports. Capability availability is not a claim of exchange-entity/healing parity. Evidence: `docs/issue-46-phase-1.md`.

- `src/step.rs` / `step_units.rs`: bounded native physical-file inventory and assigned length-unit metadata. These do not construct/validate geometric B-Rep or certify CAD import. `examples/step-inventory.rs` runs without OCCT; unsupported input/unit semantics are explicit. Evidence: `docs/issue-46-phase-2.md`.

- `step_topology`: raw local body connectivity with cumulative expansion limits. Edge-use counts/closed loops are not geometric validity or a manifold certificate; explicit/unset outer-loop roles and unplaced source coordinates remain distinct. See `docs/issue-46-phase-3.md`.

- `analytic_step` normalizes one directly represented analytic body to millimetres and validates/samples LINE/CIRCLE and PLANE/CYLINDRICAL_SURFACE geometry. It is not a trimmed-solid importer. `planar_step` classifies bounded sampled planar trim polygons, not exact curve intersections. No production switch; keep OCCT. Evidence: `docs/issue-46-phase-4.md` and `phase-5.md`.

- `cylindrical_step`: bounded sampled single-bound periodic charts. All bracket cylinders pass, but a tilted export fails the strict turn certificate; no silent seam snapping. No exact trims or solid/tessellation guarantee. See `docs/issue-46-phase-6.md` for the current native support matrix and refusal regression.

- `chart_mesh` / `native_surface`: diagnostic fixed-boundary CDT, source sample keys, bounded batched curved refinement, closed oriented edges/cyclic vertex links and per-face cancellation. Short arcs require an interior sample. This is not global geometric validity or a production CAD switch. See `docs/issue-46-phase-7.md`; CLI `step-inventory FILE --surface` needs no OCCT.

- `triangle_intersection`: bounded exact-sign sampled facet checks and immutable checked single-shell ownership; native distance-solid bridge skips welding/repair and preserves source face IDs. Not exact CAD/PLC certification. Evidence: `docs/issue-46-phase-8.md`.

- `ChartPolicy` defaults to strict. Explicit cylindrical parameter canonicalization stays within the existing model geometry tolerance and records every face/bound/oriented-use/edge/sample adjustment; world/source coordinates do not change. Preserve reports through checked geometry conversion. Generated tilted-cylinder fixture runs feature-off; see `docs/issue-46-phase-9.md`.

- `orientation_repair`: explicit whole-source-face parity repair, strict default, atomic cancellation and complete face-sense reports. Reject mixed-face/nonorientable/nonmanifold/disconnected inputs; retain global geometric checks. `NativeDistanceSolid::into_parts` returns solid plus `NativeGeometryMetadata` containing all source associations/reports. See `docs/issue-46-phase-10.md`.

- Native `Inventory` fingerprints exact accepted source bytes with locked SHA-256; `EntityReference` binds numeric entity IDs to that document and rejects mismatched/missing references. Fingerprints survive analytic/sampled/checked-distance metadata. Whitespace changes invalidate references; this is not cross-edit persistent naming or a geometric certificate. See `docs/issue-46-phase-11.md`.

- `native_volume`: immutable diagnostic mesh/report/source metadata bridge using the existing lattice mesher. Resolve stored selections by document + source face ID; absent/empty generated boundary regions remain explicit unresolved references. Nonempty region is not complete CAD coverage. No PLC/new production capability. See `docs/issue-46-phase-12.md`.

- `plc::ValidatedPlc` accepts only an immutable globally checked single-shell sampled surface; retains all triangular facets/undirected edge constraints/source-face reports with explicit budgets and cancellation. Input admission only, not recovered volume meshing or exact CAD certification. See `docs/issue-46-phase-13.md`.
