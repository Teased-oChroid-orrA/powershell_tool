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
