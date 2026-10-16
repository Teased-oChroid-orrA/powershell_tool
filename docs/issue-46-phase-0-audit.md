# Issue 46 — Phase 0 capability and dependency audit

Source revision: `8fadeb425c044e1c9660687df81d05b54b914898`. Read-only source inspection and locked offline Cargo metadata, including `app-tui/occt`; no dependency added. Baseline evidence is the clean reliability qualification linked in the execution index; no duplicate full suite was run for this audit. Runtime timing/memory numbers from that run remain workload-specific, not general throughput claims.

## Capability matrix

| Capability | Source-confirmed status | Reuse and verification boundary |
|---|---|---|
| STEP import/export | Existing optional OCCT via `solid-kernel::occt::Brep` | Actual bracket plus tests/occt.rs and fea-problem/tests/solid.rs; native entity support absent |
| IGES exchange | Not exposed by the repository wrapper | Candidate/library documentation is not engineering-file verification |
| Native B-Rep | Missing production native CAD backend | SurfaceSolid is a tagged manifold tessellation, not exact B-Rep |
| CAD healing | No public validated healing contract | Chord repair fixes tessellation only; conservative geometry healing remains work |
| Projection | OCCT trimmed-face projection used internally in tessellation repair | No backend-neutral public curve/surface evaluator yet |
| Persistent geometry identity | Partial geometric remapping in fea-problem/src/solid.rs | Face indices change under edits; ambiguous/removed references must not be guessed |
| 2D constrained meshing | Existing robust orient2d/incircle, segment splitting, Ruppert refinement | Reuse predicates, size fields and verification; not a 3D algorithm |
| 3D volume meshing | Existing signed-distance Kuhn lattice / longest-edge refinement / guarded snap | Does not recover arbitrary constrained PLC facets; thin/small features limited |
| High-order elements | Tet10, Hex20/27, Wedge15 and quadratic 2D in element.rs | Supported formulations are not arbitrary CAD-conforming mapping guarantees |
| Hybrid generation | Existing 2D structured/unstructured and swept hex/wedge infrastructure | No pyramid family in ElementKind; do not offer unsupported general 3D transitions |
| Mesh validity/quality | Existing adjacency, conformity, scaled Jacobian, quality reports, weld and smoothing | Reuse; robust mutable 3D Delaunay cavity topology still required |
| Anisotropic adaptation | Existing size prediction, AMR and selected hybrid anisotropy | General SPD metric-space split/collapse/flips not established |
| Mesh motion/ALE | No complete ALE production contract found | Existing nodal transfer and transaction infrastructure is partial reuse, not arbitrary material/contact history transfer |
| Solver cost | Existing symbolic matrix/factor cost and calibrated rates in meshing/cost.rs | Reuse; not a verified learned predictor or universal hardware model |
| AMR telemetry | Existing StageSeconds, CycleRecord, QoI reliability and convergence | factor_bytes estimates factor storage, not process peak memory |
| Autotuner persistence | No bounded versioned verified workload corpus | UI action-ranking journals are not numerical mesh-performance training records |

## OCCT boundary and dependency chain

`app-tui(occt)` → `fea-problem(occt)` / `solid-kernel(occt)` → optional `cadrum 0.9.0` → OCCT 8.0.1. `occt-source` enables cadrum/source; default builds exclude this chain. Only solid-kernel/src/occt.rs directly imports cadrum; production consumers in fea-problem/src/solid.rs directly name the OCCT wrapper. UI/test direct usages create fixture boxes. Wrapper operations: polygon/circle extrusion, boxes, cylinders, union/cut/intersection, all-edge fillets, faces/areas/centres, bounds, STEP text/files and tagged tessellation/SurfaceSolid. Local face IDs are not persistent.

OCCT remains a compatibility/reference adapter. Preserve feature propagation, source-built Windows static-CRT behavior, supported OCCT_ROOT and notices. The manifest/test header incorrectly described every OCCT build as source-built; correct those comments while isolating the backend. No numerical change is required for that correction.

## Dependency/license and candidate assessment

- Current workspace crates are MIT; `robust 1.2.0` is MIT OR Apache-2.0. Reuse its 3D predicates before considering any new dependency.
- `cadrum 0.9.0` wraps the separately licensed OCCT kernel; retain THIRD-PARTY-NOTICES.md and LGPL-2.1 plus OCCT exception texts. Native migration does not permit deleting notices before dependency retirement.
- Truck upstream is Apache-2.0, non-archived, with repository activity observed 2026-09-28. Its truck-stepio README documents STEP read/write and extraction-to-mesh, not complete production exchange/healing/IGES support. ADR-019 already records failed coplanar boss and round-number boolean cases. That remains negative evidence; current engineering-file requalification is required before selection. No Truck crate is added or approved here.
- In-tree native analytic geometry and tagged SurfaceSolid are reliable reuse boundaries but do not implement exact NURBS/B-Rep import or modelling parity.
- Do not copy TetGen/Gmsh/Mmg implementation code or add candidate dependencies without a specific version/license/maintenance audit. Independent references can provide differential benchmark outputs with their versions/licensing recorded.

## Dependency graph, risks and first implementation gates

Audit → CAD isolation, robust predicates/topology, verified telemetry and deterministic compatibility registry. Isolation + topology → native B-Rep / constrained volume recovery → sliver validation → curved/hybrid/SPD adaptation → motion/ALE and advanced cost/optional learning. See the execution index for issue links and exact dependencies.

Highest risks are boundary recovery on real trimmed CAD, stable topology/assignment identity, thin/small faces, degeneracies/slivers, curved-map validity, unsupported mixed-element transitions, contact/history transfer and misleading training verification. Keep unsupported cases explicit. Do not expose an algorithm merely because its unconstrained prototype passes.

No safe native replacement is selected yet: production capability and real-file validation, not language preference, decide that gate. Final OCCT retirement additionally requires explicit approval. Start the supported backend boundary and exact-sign 3D topology foundations without altering existing solver selection.
