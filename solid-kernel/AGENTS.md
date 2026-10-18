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
| Finite p-curve trims and oriented arcs (`step_pcurve_candidates`, `step_pcurve_arc`, `step_curve_audit` `finite_arcs`) | `src/step_pcurve_{candidates,arc}.rs`, `docs/issue-46-phase-54.md`..`57` |
| Incremental 3D Delaunay engine (walk location, cavity BFS, exact predicates) | `src/delaunay3.rs` (crate-private) |
| Conforming Delaunay recovery of a closed triangulated PLC by Steiner points (`plc_conform::conform`; surrounds the shell with 8 far points so no insertion is on the hull) | `src/plc_conform.rs`, `docs/issue-46-phase-58.md` |
| Cluster-weld defeaturing of sliver faces (group transactions, cap repair, manifold/volume/angle guards) | `src/shell_defeature.rs`, `docs/issue-46-phase-76.md` |
| Shape-preserving surface Steiner refinement (terminal-edge bisection, coplanar Delaunay flips; opt-in) | `src/shell_refine.rs`, `docs/issue-46-phase-77.md` |
| One-call shell preparation (weld, defeature, then flat-patch cleanup, tolerances tied together, never worse) | `src/shell_prepare.rs`, `docs/issue-46-phase-71.md` |
| Weld of near-degenerate short shell edges (link condition, orientation, angle floor) | `src/shell_weld.rs`, `docs/issue-46-phase-70.md` |
| Flat-patch cleanup of a sampled shell (dissolve collinear/flat vertices, constrained Delaunay re-triangulation, no new points; never worsens a thin patch) | `src/shell_patch.rs`, `docs/issue-46-phase-69.md` |
| Graded boundary sampling (`AnalyticModel::graded`: per-face chart refinement splits shared edges on their own curves until no face asks for more), chart quality (`Limits::chart_quality`), `Options::max_line_mm` | `src/chart_grade.rs`, `src/chart_mesh.rs` (`shape_chart`, `boundary_splits`), `docs/issue-46-phase-100.md` |
| Analytic tests (box, tube) | `tests/surface.rs` |
| OpenCascade tests | `tests/occt.rs` (`cargo test -p solid-kernel --features occt`) |

## Contracts
- A `SurfaceSolid` is only built from a **closed manifold** (every edge in exactly two triangles, opposite directions); an open or
  non-manifold tessellation is refused with a reason, an inside-out one is turned around.
- The sign of the distance uses angle-weighted pseudo-normals, so it is right at edges and vertices.
- Face tags are B-rep face indices in the kernel's face order; the mesher names the faces `face{tag + 1}`.
- The tessellation deflection bounds the geometric error: use a small fraction of the boundary element size.

## Pitfalls
- Plain per-face native sampling leaves needles on most faces (sparse boundaries, no interior points): use `AnalyticModel::graded` + `Limits::chart_quality` before `plc_conform` (graded generic parts reach ~25 degrees and a radius-edge ratio of 2; plain ones are refused). What grading cannot fix is near-degenerate CAD (the bracket's 5 micrometre sliver faces): that needs B-rep healing, and shell-level `shell_defeature` rejects the clusters of a graded shell.
- `plc_conform` converges for well-conditioned PLCs (boxes, plates with holes) but NOT for sampled surface meshes with skinny triangles such as the NEMA17 bracket: every triangle edge is a protected segment and Delaunay refinement cascades past any budget (it refuses with a `Limit` error in ~2 s). Do not raise `max_steiner` to compensate; the fix is TetGen-style boundary recovery by local reconnection, or an isotropic surface remesh first. `shell_patch::dissolve_flat` (also `step-inventory --clean-shell`) removes redundant flat/collinear vertices but cannot dissolve a vertex shared with curved triangles at a tangent seam, which is where the remaining bracket needles are. `shell_weld::weld_short_edges` collapses short needle edges safely; what was left there, sliver faces, `shell_defeature` now removes by welding whole cross-section groups; the remaining bracket needles lie along a thin curved band; opt-in `shell_refine` fixes chords on flat faces but cannot raise a face's own input angle. Measured boundary (Phase 83, `tests/plc_angle_sweep.rs`): plain recovery terminates on simple/surface-mesh-like slabs down to <1 degree, but `max_radius_edge = 2` refinement fails at 18 degrees and below and succeeds at 22+; the smallest input angle is not a predictor for recovery alone.
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

- `src/step_assembly.rs`: tree-shaped STEP assemblies to placed leaf instances; the transformation direction was fixed against OpenCASCADE (`tests/step_assembly.rs`, feature `occt`), membership of the placement among the representation items decides source/target, ambiguous cases refuse (Phase 95).
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

- `plc_star::StarSeed` requires exact strict interior-side visibility from a supplied kernel point to every validated PLC facet; preserves the boundary exactly. It is a limited conforming fan seed, not Delaunay/general PLC recovery or production strategy. Holes/nonvisible facets refuse, solver quality gates remain. See `docs/issue-46-phase-14.md`.

- Current diagnostic capabilities/limits and pilot corpus: `docs/issue-46-native-support-matrix-2026-10-16.md`. Hollow tube preserves the through-hole; valid thin-wall/near-boundary positive seeds still refuse solver conversion under unchanged quality limits (Phase 15).

- `plc_star::StarSeed::into_solver_mesh` reuses unchanged quality/conformity gates and restores original PLC source-face regions; refuses unknown source documents. `NativeBoundaryMesh` shares reference selection/metadata ownership with the existing lattice wrapper. Tet4 seed and lattice Tet4/Tet10 use the same analytic bar patch. See `docs/issue-46-phase-16.md`.

- `StarSeed::try_vertex_mean_kernel` trials one reported interior-point move: exact kernel/boundary proof, strict minimum-Jacobian improvement, non-worsening worst edge ratio, staged cancellation. Boundary vertices/facets/source IDs stay unchanged; concave outside-kernel means refuse. Solver gates remain independent. Limited pilot, not general sliver elimination/optimization. See `docs/issue-46-phase-17.md`.

- Feature-off developer CLI adds `--plc`, `--star-seed X,Y,Z` (mm), `--trial-kernel`; explicit chart/orientation flags combine, strict defaults remain. Staged progress and candidate/retained quality are reported; no material or solver analysis inferred. Seven standalone cases verified. See `docs/issue-46-phase-18.md`.

- `nurbs::NurbsCurve` is bounded clamped/continuous positive-weight curve evaluation, not STEP/B-Rep import. Authoritative endpoints, no parameter clamping; degree/control/arithmetic limits and analytical rational-circle/degree-elevation tests. Native analytic body support remains unchanged until source/sampling/surface/trim gates pass. See `docs/issue-46-phase-19.md`.

- Native coordinate unit conversion now uses one helper for point declarations and raw body vertices: reject nonzero-to-zero underflow before allowing true zeros, retain source offsets and existing arithmetic envelope. Radius/unit-factor paths already reject zero underflow. Reproduced/fixed in `docs/issue-46-phase-20.md`.

- `step_nurbs::read_curve` extracts supported simple/complex rational/polynomial declarations, bounded knot expansion, Cartesian control-source refs and declared flags/form/name. Caller must explicitly bind mm scale/context; tags are not geometry certificates. Reuses corrected point/scalar path. Full AnalyticModel still rejects spline-backed bodies until sampling/surface/trim gates pass. See `docs/issue-46-phase-21.md`.

- `nurbs_surface::NurbsSurface` uses the same validated homogeneous basis as curves; global net/degree/blend budgets, V-major storage (`v*u_count+u`), authoritative corners/no parameter clamping. Analytic plane/cylinder/sphere/parameter invariance checks; poles are point evaluation only. No STEP surface/B-Rep/projection/normal/trim support yet. See `docs/issue-46-phase-22.md`.

- `step_nurbs_surface::read_surface` preserves source/Cartesian-control refs and declared flags, transposes STEP U-row/V-column nets to evaluator V-major order, scales only coordinates (not knots), and enforces aggregate caps. Shared component/knot parsing with curves; unconditional degree<control-count validation precedes allocation even with caller-relaxed caps. Equivalent bracket spline plane still refuses full body import. See `docs/issue-46-phase-23.md`.

- [Phase 24 local spline differentials](../docs/issue-46-phase-24.md): shared Dual arithmetic, one-sided endpoints, singularity/work guards and qualification.

- [Phase 25 shared body/unit-context binding](../docs/issue-46-phase-25.md): strict source representation admission reused by analytic import.

- `step_body::BodyContext::read_spline_surface` binds declarations to exact-document body-face references and assigned units; full spline B-Rep import remains refused. See [Phase 26](../docs/issue-46-phase-26.md).

- [Phase 27 source-bound spline curves](../docs/issue-46-phase-27.md): shared bounded wrapper resolution and exact-document edge/unit binding.

- [Phase 28 rational curve enclosures and sampling](../docs/issue-46-phase-28.md): shared outward-rounded recurrence, whole-interval Hausdorff bounds, progress/cancellation and resource limits.

- [Phase 29 global curve distance brackets](../docs/issue-46-phase-29.md): deterministic bounded search and positive-weight Cartesian hull conditioning.

- [Phase 30 surface patch enclosures](../docs/issue-46-phase-30.md): shared Bounds3, outward tensor recurrence, bounded patch work and indexed span queries.

- [Phase 31 merged foundations and surface distance](../docs/issue-46-phase-31.md): PR #65 transition, continuation branch and shared bounded curve/surface search.

- [Phase 32 source-bound spline edges](../docs/issue-46-phase-32.md): source scope and verification.

- [Phase 33 verified linear spline bodies](../docs/issue-46-phase-33.md): source scope and verification.

- [Phase 34 declared line/surface validation](../docs/issue-46-phase-34.md): source scope and verification.

- [Phase 35 structured spline cancellation](../docs/issue-46-phase-35.md): source scope and verification.

- [Phase 36 finite planar bilinear spline faces](../docs/issue-46-phase-36.md): exact convex/coplanar reduction and conservative 3D trim containment; p-curve agreement remains open.

- [Phase 37 finite active spline domains](../docs/issue-46-phase-37.md): explicit extended knots, evaluated endpoints and active-span sampling/distance; no inferred periodic wrapping.

- [Phase 38 source p-curve declarations](../docs/issue-46-phase-38.md): shared spline decoding, explicit raw parameter space/master choices and all-bracket declaration coverage; 3D agreement remains open.

- [Phase 39 source-plane interval maps](../docs/issue-46-phase-39.md): exact-source units/bounds and scoped agreement evidence.

- [Phase 40 finite plane/line locus agreement](../docs/issue-46-phase-40.md): exact-source units/bounds and scoped agreement evidence.

- [Phase 41 whole-interval planar spline p-curve bounds](../docs/issue-46-phase-41.md): exact-source and budget guards; no master/trim certificate.

- [Phase 42 source cylinder parameter bounds](../docs/issue-46-phase-42.md): explicit radians/lengths, bounded Taylor arithmetic and shared source-frame enclosure.

- [Phase 43 planar circle carrier comparison](../docs/issue-46-phase-43.md): unsigned whole-locus bounds, shared source association checks and explicit finite-arc/master limits.

- [Phase 44 shared curve agreement](../docs/issue-46-phase-44.md): finite line/circle carrier checks for planes and cylinders; raw direction ratios retain exact constant-coordinate admission.

- [Phase 45 quadratic spline/circle bounds](../docs/issue-46-phase-45.md): Bernstein residual and optional closed monotone angular coverage; no single winding/simple trim/master/body certificate.

- [Phase 46 bounded p-curve body diagnostics and CLI](../docs/issue-46-phase-46.md): cumulative work budgets and atomic progress/cancellation; no master/trim/body certificate.

- [Phase 47 single-winding quadratic trace certificate](../docs/issue-46-phase-47.md): <=4 certified half-plane spans; body audit requests it, while coverage-only remains explicit.

- [Phase 48 source spline parameter/composition bounds](../docs/issue-46-phase-48.md): raw UV/knots, outward physical scaling and exact-boundary arithmetic; no regularity/trim/body certificate.

- [Phase 49 source p-curve world-distance brackets](../docs/issue-46-phase-49.md): shared bounded search, decoded-declaration reuse and explicit candidate-enclosure/uniqueness limits.
