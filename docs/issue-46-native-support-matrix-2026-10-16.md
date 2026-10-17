# Native CAD/meshing support snapshot — 2026-10-16

Diagnostic branch `feature/native-cad-advanced-meshing`, draft PR #65, epic #46. This snapshot supersedes earlier diagnostic support summaries for the listed increments; historical phase evidence remains unchanged. Production CAD still uses the isolated OCCT adapter. No native replacement/retirement claim.

| Area | Verified diagnostic scope | Still unsupported / not established |
|---|---|---|
| STEP physical file | Bounded headers/data/entities/reference graph; unknown declarations retained | General physical-file variants, IGES, encoded-string interpretation |
| Units | Explicit SI/conversion-based assigned length contexts, mm normalization, ambiguity/cycle refusal | Guessing units/provenance/material from labels |
| Body/topology | One directly represented unplaced body; source vertices/edge uses/loops/faces, geometric locus checks | Assemblies, mapped placements, multiple/nested shells and material regions |
| Analytic geometry | LINE/CIRCLE, PLANE/CYLINDRICAL_SURFACE; evaluation/normals/shared boundary samples | Spline/NURBS/cone/sphere/torus and general projection |
| Trim domains | Sampled planar polygons with holes; single-bound contractible seamed cylindrical charts | Exact curved trim intersection certificates; cylindrical holes/unseamed winding bands/multiple turns |
| Native sampled surface | Fixed-boundary chart CDT, bounded curved interior refinement, source identity; closed opposite edges/cyclic links/one shell | Exact CAD-solid certificate, arbitrary curved-face tessellation |
| Global sampled geometry | Exact-sign facet intersection checks, immutable checked ownership, unchanged distance/normal/BVH path | Exact original B-Rep self-intersection and multi-shell/cavity qualification |
| Explicit conservative repair | Bounded cylindrical parameter canonicalization; whole-face orientation parity, every modification reported, cancellation atomic | Sewing, gap closure, feature deletion, arbitrary geometry repair |
| Associations | Exact-source SHA-256 plus entity ID, identical reload, face remapping across coarse/fine generation, Tet4/Tet10 volume boundaries; unresolved regions explicit | Naming across CAD edits/re-exports, complete CAD coverage proof, general AMR/coarsening/history/material/contact transfer |
| PLC input | Immutable globally checked single-shell triangulated boundary, every edge/facet retained, source references/budgets/cancellation | Cavity/multi-region/multiple-component admission and exact curved constraints |
| Conforming volume seed | Strictly star-shaped PLC with supplied kernel point; exact halfspace certificate, every boundary facet recovered; quality/conformity gate reused | General constrained Delaunay/segment/facet recovery, automatic kernel search, refinement/sliver elimination, production selection |
| Existing volume bridge | Sampled distance-solid through the existing lattice mesher, with source metadata | New PLC tetrahedralizer; complete resolution of arbitrarily small CAD faces |
| Learning | Earlier opt-in independently verified template telemetry and deterministic registry foundations | Representative corpus, held-out/OOD qualification and learned production predictions |

## Regression cases and what they establish

- **Actual NEMA17 bracket:** unknown provenance/material, declared mm. 38 supported analytic faces; 469-vertex/978-facet coarse sampled shell passes global facet checks. Fine sampled volume converges toward independent OCCT tessellation. The hole-bearing domain refuses the limited star seed; this does not claim new constrained volume meshing of the bracket.
- **Generated tilted cylinder:** known dimensions/placement, explicit parameter canonicalization reports 22 changed samples within unchanged geometry tolerance. Strict default still refuses the periodic certificate. Committed fixture runs without OCCT.
- **Generated box/bar:** exact sampled volume and source-face identity, native distance bridge, Tet4/Tet10 load/support reference tension patches (existing 1e-6 displacement criterion), 12-cell conforming star seed and solver quality/conformity acceptance.
- **Generated hollow tube:** analytic volume and signed distance distinguish through-hole, wall and exterior; annular planar trims and both cylindrical walls remain present. Phase 15 records final verification.
- **Generated thin plate and near-boundary bar seed:** valid surface/positive seed topology does not bypass the existing solver quality limits. These are refusal regressions, not optimized meshes or trusted training timings.

All generated CAD cases use the retained OCCT export as an interchange source and independent analytic checks where stated. This is a small pilot corpus, not representative aerospace qualification. No benchmark/training success is inferred from parser success, geometry checks or expected solver-quality rejection. No macOS/Windows native diagnostic execution is claimed by these Linux checks.

## Remaining completion gates

#51 remains open for broader exchange/geometry/placement/assembly/exact trims/healing/persistent association coverage. #52 remains open for general constrained recovery/refinement/cavities/regions. #53 requires sliver elimination/quality and solver-conditioning evidence, not merely rejection tests. P2/P3 work and final parity remain open under the execution index. Production merge and OCCT retirement retain their separate approval gates. Do not close the epic based on this snapshot.

## Later same-day increments

Phase 16 (`93f0fb6`) supersedes the earlier seed-tagging limitation: the quality-gated native Tet4 seed now retains original source-face solver regions through the same immutable boundary-reference lookup as the lattice wrapper. Shared bar tension checks cover seed Tet4 and lattice Tet4/Tet10. Unknown provenance still refuses source-tagged conversion.

Phase 17 (`bfc91ef`) adds a reported single interior kernel trial with exact visibility/boundary preservation and guarded quality improvement. Near-boundary bar quality improves enough to pass unchanged solver gates; a concave L-prism's outside-kernel mean is refused. This does not establish general sliver elimination, constrained recovery, representative conditioning/performance data or any later gate.

Phase 19 (`6cf9dfe`) adds bounded clamped rational curve evaluation with analytic circle/degree-elevation tests. Phase 20 (`0f9301c`) fixes reproduced coordinate unit-underflow in both point and raw vertex paths. Phase 21 (`7e2d7a8`) maps supported simple/complex STEP spline declarations and preserves source/control IDs, declared metadata and explicit mm scaling. These are evaluator/declaration foundations only: native full-body spline surfaces/trims/assembly support remains refused pending separate geometric gates. Latest checks: 83 CAD-enabled / 66 feature-off tests and app compilation pass.

Phases 22–26 add tensor rational surfaces, STEP surface nets with safe allocation admission, local differentials reusing existing Dual arithmetic, shared strict body/unit binding and source-bound face spline declarations. Nonzero normalization underflow refuses. These remain declaration/evaluation foundations; full spline-body sampling/projection/trims remain gated. Source `0b19cd0`: 106 CAD-enabled / 89 feature-off tests and app compilation pass. [Phase 26 snapshot](issue-46-progress-phase-26.json).

Phase 27 (`69ae967`) adds the equivalent exact-source edge/unit binding for spline curves, sharing bounded wrapper resolution with the strict analytic importer. Cyclic wrappers/stale edge selections refuse. Latest qualification remains 106 CAD-enabled / 89 feature-off tests plus app compilation, now at this source. Full spline-body geometry gates remain open. [Current snapshot](issue-46-progress-phase-27.json).

Phase 28 (`166279e`) adds whole-parameter rational curve enclosures and cancellable, resource-bounded sampling with per-segment Hausdorff upper bounds. Curved box bounds are conservative (development quarter circle: 4,094 segments at 0.001), not representative production performance evidence. No full spline-body import. Latest qualification: 114 CAD-enabled / 97 feature-off tests and app compilation. [Snapshot](issue-46-progress-phase-28.json).

Phase 29 (`d355037`) adds bounded global curve-distance brackets and candidate uncertainty, not unique projection. Active Cartesian hull intersection fixes reproduced constant-curve interval conditioning with unchanged tolerance/budget. Latest qualification: 118 CAD-enabled / 101 feature-off tests and app compilation. [Snapshot](issue-46-progress-phase-29.json). Full spline-body/general CAD/PLC/quality and P2/P3/final gates remain open.

Phase 30 (`d3e81dc`) adds outward tensor surface patch enclosures, shared Bounds3 and indexed local span queries. Existing CurveBounds remains a re-export. Exact-real bounds do not promise to contain every rounded evaluator output. Source-qualified checks: 121 CAD-enabled / 104 feature-off tests and app compilation. [Snapshot](issue-46-progress-phase-30.json). Surface projection/trim/full spline-body and broader epic gates remain open.

Phases 31–35 (`2042c2d` qualified source): shared bounded curve/surface distance search, source-bound spline-edge samples, verified native full-domain two-control positive degree-one curves, stricter actual line/surface and endpoint checks, and structured cancellation. A bracket linear-spline replacement preserves the sampled mesh bitwise and passes facet checks. Curved/piecewise curves and spline faces remain refused. 130 CAD-enabled / 113 feature-off tests and application compilation pass; initial failures preserved. [Snapshot](issue-46-progress-phase-35.json).

Phase 36 (`25f6c25` qualified source): exactly coplanar convex positive-weight bilinear faces can reconstruct finite 3D trim domains; actual source-control-plane interval checks and chart-orientation certification guard the facade. P-curves/master representations remain unverified; general spline import is not complete. Root bracket plane replacement passes face/volume/facet checks; tiny/warped patches refuse. 132 CAD-enabled / 115 feature-off tests and app compilation pass. PR #66 merged at 3aebf99, current branch `feature/native-cad-planar-spline-trims`. [Snapshot](issue-46-progress-phase-36.json).

Phase 37 (`6c96b31`): explicit finite active-domain extended-knot curve evaluation, actual basis endpoints and active-only sampling/distance. Existing STEP/body readers stay clamped; no periodic wrapping or p-curve/master agreement is inferred. Real bracket #237 fixture evaluates. 135 CAD-enabled / 118 feature-off tests and app compilation pass. [Snapshot](issue-46-progress-phase-37.json).

Phase 38 (`02d7cd3`): bounded p-curve/context/master declaration decoding covers all 228 root-bracket p-curves without guessed length/angle scaling. Shared physical spline reader remains clamped and preserves its units/API. Mapping/master agreement is still open; no new native body strategy is advertised. 139 CAD-enabled / 122 feature-off tests plus app compilation pass. PR #67 merged at 75e4faa; continue on `feature/native-cad-pcurve-validation`. [Snapshot](issue-46-progress-phase-38.json).

Phases 39–40 (`b35c27c`): source-bound actual-plane UV→mm interval maps and unsigned finite plane/line locus reports. 148 bracket associations certify at unchanged 1e-7 mm; declared master identity/natural-direction dot are reported without promoting full master validity. 145 CAD-enabled / 128 feature-off tests plus app compilation pass. [Live pointer](issue-46-current.json) / [snapshot](issue-46-progress-phase-40.json).

- [Phase 41: whole-interval planar spline p-curve bounds](issue-46-phase-41.md): source/budget guards and explicit unit qualification.

- [Phase 42: bounded source cylindrical parameters](issue-46-phase-42.md): explicit radian/length semantics and shared source placement; trim/master gates remain open.

- [Phase 43: unsigned planar circle carrier agreement](issue-46-phase-43.md): 13 whole-carrier checks; finite arc/master/body gates remain separate.

- [Phase 44 cylinder line/circle agreement](issue-46-phase-44.md): 28 generator and 24 circle carrier checks with shared numerical proofs.

- [Phase 45 spline/circle bounds and optional coverage](issue-46-phase-45.md): 15 complete-span quadratic checks with explicit closure/winding limits.

- [Phase 46 bounded body p-curve diagnostics](issue-46-phase-46.md): 228 scoped checks, aggregate budgets, progress, cancellation and runnable CLI.

- [Phase 47 bounded single-winding circle traces](issue-46-phase-47.md): stronger quadratic coverage and double-traversal refusal; general/finite trim gates remain open.

- [Phase 48 source spline parameter maps](issue-46-phase-48.md): composed p-curve bounds, single physical scaling and reproduced exact-boundary interval correction.
