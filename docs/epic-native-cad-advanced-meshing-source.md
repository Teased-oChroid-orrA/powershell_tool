# EPIC: Native Rust CAD & Advanced Solver-Aware Meshing Engine

**Repository:** `powershell_tool`  
**Primary Crate:** `fea-core`  
**Priority:** High — Strategic Core Infrastructure  
**Language:** Rust  
**Suggested Branch:** `feature/native-cad-advanced-meshing`  
**Objective:** Progressively replace OpenCascade with a validated native-Rust CAD and computational meshing architecture.  
**Status:** Proposed — Repository audit required before implementation.

---

# 1. Executive Summary

Design, implement, verify, and optimize a native-Rust geometric modeling and meshing engine that progressively removes FEA-Core's dependency on OpenCascade while expanding the application's capabilities toward those offered by Gmsh, TetGen, and Mmg.

The implementation shall consist of six major pillars:

1. **Native CAD geometry and B-Rep processing** — STEP/IGES interoperability, NURBS, topology, geometry validation, healing, and high-order geometry mapping.
2. **Robust arbitrary 3D volume meshing** — constrained tetrahedralization, PLC recovery, refinement, quality optimization, and curved high-order elements.
3. **Hybrid structured–unstructured meshing** — mapped/swept regions, conforming transitions, and mixed compatible element topologies.
4. **Metric-driven anisotropic adaptation** — spatial SPD metric fields, directional refinement, edge splitting/collapse, topology optimization, and solver-informed size control.
5. **Mesh motion and state transfer** — r-adaptivity, ALE, deformation-aware smoothing, remeshing, and consistent solution transfer.
6. **Solver-aware optimization** — mesh candidate selection and quality optimization for total FEA time-to-accuracy, numerical conditioning, and memory efficiency.

The engine shall operate independently of the FEA solver where possible while exposing efficient interfaces to the solver, AMR, nonlinear contact, and sensitivity infrastructure.

## Non-negotiable principles

- No mathematical or physical accuracy regressions.
- No blind replacement of existing verified capabilities.
- No assumption that pure Rust automatically means faster.
- No assumption that any Rust CAD dependency has complete OpenCascade feature parity.
- No unverified direct copying of restrictive-licensed meshing implementations.
- No accepted inverted or invalid finite elements.
- No broken model topology, boundary conditions, material assignments, or contact definitions.
- No large monolithic CAD/meshing subsystem.
- No direct changes to `main`.
- Maintain an independently testable migration path until native capabilities are verified.

---

# PART I — REPOSITORY AUDIT AND DEPENDENCY MIGRATION

# 2. Mandatory Initial Audit

Before modifying the implementation, examine the latest repository state.

Determine the actual status of:

- OpenCascade integration and usage.
- CAD import/export.
- B-Rep support.
- Geometry modeling operations.
- Geometry validity and healing.
- Existing 2D Delaunay implementation.
- Existing 3D meshing and extrusion/sweep capabilities.
- Constrained triangulation.
- Existing mesh smoothing and optimization.
- Structured and hybrid mesh generation.
- Element types and polynomial interpolation.
- Existing AMR and error estimation.
- Mesh quality evaluation.
- Material interface welding.
- Contact surface tagging.
- Mesh state transfer.
- Sparse assembly.
- Geometry-to-FEA metadata mapping.
- Performance tests and numerical verification.

Do not assume the modules or existing features described in this EPIC are already present or complete.

Create a capability matrix:

| Capability | Existing | Partial | Missing | Reuse/Refactor | Verification |
|---|---|---|---|---|---|
| STEP import | TBD | TBD | TBD | TBD | TBD |
| IGES import | TBD | TBD | TBD | TBD | TBD |
| Native B-Rep | TBD | TBD | TBD | TBD | TBD |
| CAD healing | TBD | TBD | TBD | TBD | TBD |
| 3D tetrahedralization | TBD | TBD | TBD | TBD | TBD |
| High-order elements | TBD | TBD | TBD | TBD | TBD |
| Anisotropic adaptation | TBD | TBD | TBD | TBD | TBD |
| ALE/state transfer | TBD | TBD | TBD | TBD | TBD |

Produce a dependency and implementation-risk report before introducing new crates.

**Exit gate:** Verified repository capability matrix and implementation dependency graph.

# 3. OpenCascade Replacement Strategy

Do not remove OpenCascade immediately.

Introduce a backend-neutral CAD abstraction.

Suggested interface responsibilities:

- Import geometry.
- Export geometry.
- Evaluate curves/surfaces.
- Query topology.
- Evaluate geometric derivatives.
- Find closest points.
- Classify points inside/outside/on solids.
- Validate solid topology.
- Perform supported modeling operations.
- Heal and stitch geometry.
- Resolve persistent geometry identifiers.

## Candidate evaluation

Evaluate:

- Truck and its component crates.
- Other credible native-Rust B-Rep implementations.
- Reuse of existing in-tree geometry code.
- Small independent Rust geometry libraries.

Candidate evaluation must distinguish:

1. Documented capabilities.
2. Actual implemented capabilities.
3. Verified behavior on engineering files.
4. Missing production requirements.
5. Toolchain/build dependencies.
6. Maintenance and licensing.
7. Performance and memory.

Truck is a preferred initial candidate for evaluation, **not a predetermined mandatory dependency**.

## Native-only objective

The final supported production path should operate without linking OpenCascade or other required C/C++ geometry kernels.

However, during migration, retain the existing verified backend as a temporary compatibility/reference adapter if needed.

OpenCascade removal is permitted only after explicit acceptance gates are satisfied.

---

# PART II — PILLAR 1: NATIVE CAD AND B-REP GEOMETRY

# 4. CAD Exchange Infrastructure

Implement a layered, extensible CAD import architecture.

Initial priorities:

- STEP AP203/AP214 geometry and topology where supported.
- STEP AP242 geometry, assembly metadata, and PMI only where independently verified.
- IGES through a validated native reader if feasible.
- STL/OBJ surface geometry as faceted input, explicitly distinguished from exact CAD.
- Existing application-native geometry formats.

Never label partial STEP or IGES support as full format compatibility.

## Requirements

- [ ] Parse supported exchange-file entities.
- [ ] Map imported geometry into a canonical native representation.
- [ ] Preserve face/edge/solid topology.
- [ ] Preserve units and coordinate transforms.
- [ ] Preserve assembly placements and instance relationships.
- [ ] Preserve material/body identifiers where available.
- [ ] Preserve geometry identity for downstream FEA assignments.
- [ ] Identify unsupported entities and metadata.
- [ ] Return structured import diagnostics.
- [ ] Prevent partial import from silently creating an apparently valid solid.

A parser must not be considered a complete CAD importer merely because it can read file syntax.

It must also reconstruct usable geometric and topological entities.

# 5. Canonical B-Rep Geometry Model

Represent:

- Vertices.
- Oriented edges.
- Wires/loops.
- Faces with trimming loops.
- Shells.
- Closed and open solids.
- Multi-body assemblies.
- Analytic surfaces.
- B-spline/NURBS geometry.
- Orientation and adjacency.
- Geometric tolerance metadata.
- Persistent topology references.

Define an explicit distinction between:

**Geometric objects:** curves, surfaces, and volumetric regions.

**Topological objects:** vertices, edges, wires, faces, shells, and solids.

**Discrete objects:** nodes, facets, and finite elements.

This separation is essential to robust CAD-conforming meshing.

## Core operations

- Curve evaluation and derivatives.
- Surface evaluation and derivatives.
- Surface normals.
- Closest-point projection.
- Surface parameter inversion.
- Trim-domain classification.
- Edge/face adjacency queries.
- Solid orientation.
- Point-in-solid classification.
- Bounding volumes and spatial indexing.
- Geometric tolerance handling.

# 6. Geometry Validation and Healing

Implement a staged repair pipeline.

Checks must include:

- Open edges.
- Non-manifold topology.
- Duplicate vertices.
- Near-coincident edges.
- Degenerate edges.
- Missing/incorrect trimming loops.
- Self-intersections.
- Incorrect shell orientation.
- Inconsistent face normals.
- Gaps between adjacent surfaces.
- Invalid solids.
- Excessively small features.

Healing operations must be conservative and configurable.

Examples:

- Tolerance-aware vertex/edge sewing.
- Removal of provably redundant entities.
- Edge/loop reconstruction where valid.
- Face orientation correction.
- Small-gap closure within authorized tolerance.
- Duplicate entity resolution.

**Never silently delete engineering-relevant features**, especially holes, small radii, load application regions, contact interfaces, and narrow ligaments.

Record all modifications to imported geometry.

Provide a before/after topology and tolerance report.

# 7. Persistent Geometry-to-Mesh Association

Implement persistent references between CAD entities and finite-element entities.

Each generated mesh entity should retain appropriate associations to:

- Original body.
- Original CAD face/edge/vertex.
- Material region.
- Boundary-condition region.
- Contact interface.
- Geometric feature classification.

These associations must survive:

- Initial mesh generation.
- Local refinement.
- Coarsening.
- Node repositioning.
- Mesh regeneration.
- High-order elevation.

Use stable identifiers and explicit remapping rules.

Do not use raw array indices as persistent CAD entity identity.

# 8. High-Order CAD-Conforming Elements

Develop geometry elevation independently from element-solver implementation.

Target supported families incrementally:

- Tri3 → Tri6.
- Quad4 → Quad8/Quad9.
- Tet4 → Tet10.
- Hex8 → Hex20/Hex27.
- Wedge6 → supported higher-order variants.

Higher-order elements must be supported by the FEA interpolation, quadrature, assembly, and contact infrastructure before being enabled for solution.

## Curved geometry mapping

For CAD-boundary nodes:

1. Identify the owning geometric entity.
2. Evaluate the appropriate parameter-space location.
3. Project onto the exact curve or trimmed surface.
4. Preserve shared interface node identity.
5. Check orientation and geometric validity.
6. Verify the isoparametric mapping Jacobian over suitable points.
7. Optimize/reduce curvature where required to prevent inversion.

Do not independently project shared nodes in ways that create cracks.

Do not assume midpoint projection is sufficient for cubic or complex high-order elements.

### Acceptance

- Positive valid element mapping.
- CAD geometry tolerance compliance.
- Correct high-order node connectivity.
- Numerical patch tests.
- Verified higher-order convergence.
- Compatible mixed-element interfaces.

---

# PART III — PILLAR 2: ROBUST 3D UNSTRUCTURED MESHING

# 9. Robust Computational Geometry Foundation

Before implementing tetrahedralization, establish robust geometric predicates.

Required operations:

- 2D orientation.
- 3D orientation.
- In-circle.
- In-sphere.
- Segment/triangle intersection.
- Point-in-tetrahedron.
- Face intersection.
- Topological consistency checks.

Use adaptive-precision predicates where available.

Evaluate suitable permissively licensed crates before implementing equivalent algorithms manually.

Distinguish numerical predicates from approximate geometric measurements.

Robust predicate signs are critical because incorrect signs can corrupt tetrahedral mesh topology.

## Degeneracy handling

Explicitly handle:

- Coplanar points.
- Cospherical points.
- Nearly coincident vertices.
- Duplicate points.
- Thin and nearly degenerate features.
- Non-general-position configurations.

Evaluate deterministic symbolic perturbation where appropriate.

Do not rely on arbitrary floating-point epsilon offsets for topological correctness.

# 10. Incremental 3D Delaunay Core

Implement or reuse a robust 3D Delaunay tetrahedralization capability.

Candidate approach: incremental Bowyer–Watson.

Requirements:

- [ ] Initial enclosing tetrahedral domain/superstructure.
- [ ] Efficient point location.
- [ ] Conflict cavity discovery.
- [ ] Boundary face extraction.
- [ ] Valid tetrahedral cavity reconstruction.
- [ ] Neighbor adjacency maintenance.
- [ ] Robust orientation.
- [ ] Degeneracy handling.
- [ ] Local topology verification.
- [ ] Removal of initial superstructure.
- [ ] Efficient incremental updates.

Use a memory-conscious data layout suitable for large meshes.

Avoid full-mesh scans for every inserted vertex.

Evaluate spatial search structures and walking-based point location.

## Critical limitation

Unconstrained Bowyer–Watson is not sufficient for arbitrary CAD solids.

It can tetrahedralize a point set while failing to preserve the required CAD boundary.

Therefore, the volume mesher must support a constrained or conforming boundary-recovery stage.

# 11. PLC and Constrained Volume Meshing

Introduce a Piecewise Linear Complex representation for faceted boundary geometry.

Represent:

- Vertices.
- Protected segments.
- Boundary facets.
- Region markers.
- Holes/cavities.
- Material partitions.
- Interface boundaries.

Requirements:

- [ ] Input boundary validation.
- [ ] Closed volume detection.
- [ ] Surface intersection detection.
- [ ] Constrained segment recovery.
- [ ] Constrained facet recovery.
- [ ] Boundary-conforming tetrahedralization.
- [ ] Interior/exterior tetrahedron classification.
- [ ] Hole and cavity preservation.
- [ ] Material interface preservation.
- [ ] Multiple connected components.
- [ ] Reproducible failure diagnostics.

Do not assume constrained facets can always be recovered without Steiner vertices.

Do not promise unconditional termination or quality guarantees for arbitrary small-feature geometries.

Document assumptions and supported PLC classes.

# 12. 3D Delaunay Refinement

Implement a quality-driven refinement algorithm with feature-protection rules.

Potential operations:

- Circumcenter insertion.
- Off-center insertion.
- Segment splitting.
- Facet refinement.
- Encroachment checks.
- Protected feature handling.
- Local cavity retriangulation.

Use appropriate 3D constrained refinement theory.

Do not directly generalize 2D Ruppert guarantees to arbitrary 3D PLC domains.

Refinement must respect:

- Minimum feature size.
- CAD curvature.
- Narrow gaps.
- Material interfaces.
- Contact surfaces.
- Requested size fields.
- DOF/memory budgets.

# 13. Tetrahedral Quality and Sliver Removal

Implement multiple quality indicators.

- Radius-edge ratio.
- Radius-ratio or related shape metrics.
- Minimum/maximum dihedral angles.
- Volume.
- Signed Jacobian.
- Mean ratio.
- Aspect ratio.
- Normalized shape quality.

A tetrahedron may have an acceptable radius-edge ratio but still be a sliver.

Therefore, radius-edge refinement alone is insufficient.

## Optimization strategies

Evaluate:

- 2–3 and 3–2 flips.
- Supported higher-order local flips.
- Edge/face reconnection.
- Sliver exudation where feasible.
- Optimization-based smoothing.
- Geometry-constrained smoothing.
- Local remeshing.
- Boundary-preserving vertex relocation.

Use quality objectives that reflect finite-element mapping behavior.

Reject operations that invert elements, violate constraints, destroy material interfaces, or invalidate contact geometry.

All topology-changing operations must support rollback.

# 14. Efficient Mesh Data Structures

Evaluate a connectivity representation using:

- Compact node arrays.
- Compact tetrahedron arrays.
- Local neighbor indices.
- Stable external entity identifiers.
- Region/material tags.
- Sparse adjacency generation.
- Optional half-facet connectivity for editing.
- Spatial search indices.

Prefer data layouts optimized for common operations, not maximum abstraction.

Evaluate structure-of-arrays versus array-of-structures based on actual profiling.

Maintain separation between mutable construction topology and compact solver-ready storage.

---

# PART IV — PILLAR 3: HYBRID STRUCTURED–UNSTRUCTURED MESHING

# 15. Integration with the Existing Hybrid Meshing EPIC

This work must extend the independently planned hybrid meshing subsystem, not reimplement it.

Reuse verified capabilities for:

- Mapped quadrilateral meshes.
- Swept hexahedral meshes.
- Wedge/prism sweeps.
- Geometry partitioning.
- Region classification.
- Transition elements.
- Mesh optimization.
- High-order interpolation support.

Where previous work already exists, treat it as shared infrastructure.

## Hybrid topology selection

Use structured topology when it demonstrably improves:

- Element quality.
- Mesh alignment.
- Connectivity regularity.
- Solver conditioning.
- Sparse matrix structure.
- Memory utilization.
- Time-to-accuracy.

Use unstructured topology when:

- Geometry complexity makes structured mapping inefficient.
- Complex holes and fillets require localized geometry resolution.
- Localized adaptation requires more flexible connectivity.
- Contact geometry is complex.
- Structured blocks would become highly distorted.

Do not force structured meshes in all simple-looking regions.

# 16. Mixed-Element Transitions

Support transitions between compatible element families.

Potential combinations:

- Quad–triangle.
- Hex–wedge.
- Hex–pyramid–tet.
- Prism–tet.
- Other mathematically verified compatible combinations.

Every transition must satisfy:

- Geometric conformity.
- Interpolation compatibility.
- Correct orientation.
- Shared interface consistency.
- Material/contact tag preservation.
- Positive element mapping.
- Solver quadrature compatibility.

If no supported conforming transition exists, select a verified fallback meshing strategy rather than silently inserting invalid interfaces.

# 17. Boundary-Aligned and Layered Meshing

Add controlled layered meshing where beneficial.

Applications include:

- Contact-interface refinement.
- Thin structural regions.
- Through-thickness discretization.
- Near-wall CFD boundary layers, when applicable.

Distinguish:

- Structural layer generation.
- Swept prismatic/hex layers.
- Advancing-front surface triangulation.
- CFD-oriented boundary-layer inflation.

Implement each only where a demonstrated requirement exists.

Avoid producing extremely thin or distorted elements solely to mimic structured layering.

---

# PART V — PILLAR 4: METRIC-DRIVEN ANISOTROPIC ADAPTATION

# 18. Riemannian Metric Field

Introduce a symmetric positive-definite metric tensor field.

For 2D:

\[
M(x)\in\mathbb{R}^{2\times2}
\]

For 3D:

\[
M(x)\in\mathbb{R}^{3\times3}
\]

with:

\[
M=M^T,\qquad M\succ0
\]

The metric defines target directional element lengths.

For displacement vector \(d\), a local metric length is:

\[
\ell_M=\sqrt{d^TMd}
\]

For spatially varying metrics, use a suitable edge-path approximation to:

\[
L_M(e)=\int_e\sqrt{dx^TM(x)dx}
\]

## Requirements

- [ ] SPD metric representation.
- [ ] Eigenvalue decomposition or equivalent robust operations.
- [ ] Positive-definiteness enforcement.
- [ ] Minimum/maximum target sizes.
- [ ] Maximum anisotropy ratio.
- [ ] Metric interpolation.
- [ ] Metric smoothing.
- [ ] Spatial gradation limits.
- [ ] Metric-space edge length.
- [ ] Metric-space element quality.
- [ ] Metric composition/intersection.
- [ ] Deterministic invalid-metric handling.

Do not permit zero, negative, NaN, or infinite metric eigenvalues.

# 19. Metric Generation

Support independent and combinable metric sources.

### Geometry metric

- Surface curvature.
- Edge curvature.
- Feature proximity.
- Thickness.
- Small gaps.
- Contact-interface geometry.

### Physics metric

- Validated solution Hessian recovery.
- Stress/strain recovery.
- Temperature gradients.
- Contact traction variation.
- Error-estimator information.
- Goal-oriented directional sensitivity.

### User metric

- Prescribed element sizes.
- Anisotropy directions.
- Protected zones.
- Local accuracy requirements.

### AMR-derived metric

Consume error information from the existing goal-oriented and physics-aware AMR engine.

Do not simply make element length inversely proportional to raw stress gradient magnitude.

The metric generation process must account for scale, physics, reliability, directional information, and required discretization accuracy.

# 20. Metric-Space Mesh Operations

Implement or reuse:

- Edge splitting.
- Edge collapsing.
- Edge swapping.
- Local reconnection.
- Node relocation.
- Metric-driven smoothing.

Use configurable metric-space length targets, for example:

- Split edges above an experimentally selected upper threshold.
- Collapse eligible edges below an experimentally selected lower threshold.
- Optimize elements toward acceptable metric-space shape and size.

Require hysteresis between split/collapse thresholds to avoid refinement oscillations.

Reject topology changes violating:

- CAD conformity.
- Boundary constraints.
- Contact compatibility.
- Minimum feature sizes.
- Protected geometry.
- Positive Jacobians.
- Material interface preservation.
- State transfer requirements.

# 21. Metric Complexity and Budget Control

Anisotropic metric fields can imply excessive numbers of elements.

Introduce metric complexity estimation and global rescaling.

Requirements:

- Estimate mesh complexity from the metric field.
- Respect DOF/element-count budgets.
- Bound anisotropy.
- Restrict size gradation.
- Preserve mandatory protected physics constraints.
- Report infeasible accuracy/resource combinations.

When error constraints cannot be satisfied within resource limits, return an explicit budget-limited status.

Never silently relax a protected physics requirement.

---

# PART VI — PILLAR 5: MESH MOTION, ALE AND STATE TRANSFER

# 22. r-Adaptivity and Mesh Motion

Develop a mesh motion subsystem that can relocate nodes while preserving connectivity when feasible.

Candidate techniques:

- Elasticity-based mesh motion.
- Spring analogy.
- Harmonic/Laplacian smoothing.
- Optimization-based displacement.
- Boundary-constrained mesh motion.

Choose based on quality, robustness, and cost.

For large or strongly anisotropic deformations, simple Laplacian smoothing may be inadequate.

## Requirements

- [ ] Boundary displacement application.
- [ ] Geometry-constrained node movement.
- [ ] Interior displacement solution.
- [ ] Quality monitoring.
- [ ] Inversion prevention.
- [ ] Distortion detection.
- [ ] Rollback of failed motion.
- [ ] Transition to local remeshing when required.

Do not equate mesh smoothing alone with complete ALE support.

# 23. ALE Infrastructure

For analyses requiring Arbitrary Lagrangian-Eulerian formulations:

- Maintain material and mesh motion separately.
- Compute mesh velocity.
- Account for relative material/mesh transport where applicable.
- Integrate with the actual governing equations.
- Preserve compatible discretization.
- Track geometry and mesh states over time.

Implement true ALE only for supported physics requiring it.

Structural Lagrangian deformation with mesh smoothing should remain a separate capability.

# 24. Solution and History Transfer

When mesh topology changes, transfer relevant fields.

Candidate fields:

- Displacements.
- Velocities.
- Accelerations.
- Temperature.
- Stress.
- Strain.
- Plastic strain.
- Damage variables.
- Internal state variables.
- Contact history.
- Material region assignments.
- Boundary and constraint metadata.

Use transfer methods appropriate to the physical quantity.

Possible approaches:

- Nodal interpolation.
- Element projection.
- Conservative L2 projection.
- Integration-point history mapping.
- Constitutive-state reconstruction.

No single transfer method is valid for every field.

## Validation

- Preserve required conserved quantities.
- Preserve admissible internal states.
- Prevent invalid damage/plastic state regression.
- Verify interface/boundary conditions.
- Check post-transfer equilibrium.
- Re-equilibrate when necessary.

A failed state transfer must invalidate the proposed remesh operation.

---

# PART VII — PILLAR 6: SOLVER-AWARE MESH OPTIMIZATION

# 25. Optimize Total Computational Cost

Use a solver-aware objective.

Conceptually:

\[
C_{\mathrm{total}} =
C_{\mathrm{CAD}}
+C_{\mathrm{mesh}}
+C_{\mathrm{assembly}}
+C_{\mathrm{solve}}
+C_{\mathrm{error}}
+C_{\mathrm{adapt}}
+C_{\mathrm{transfer}}
\]

For repeated solves, account for reuse and the number of analysis runs.

The objective is:

\[
\min C_{\mathrm{total}}
\]

subject to:

\[
E_{\mathrm{QoI}}\leq\epsilon_{\mathrm{QoI}}
\]

\[
E_{\mathrm{physics}}\leq\epsilon_{\mathrm{physics}}
\]

and all mesh-validity/resource constraints.

Where true errors are unavailable, use validated estimates while retaining uncertainty and reliability diagnostics.

# 26. Solver-Relevant Mesh Metrics

Evaluate:

- DOF count.
- Nonzero matrix entries.
- Bandwidth/profile.
- Sparsity structure.
- Assembly cost.
- Fill-in after factorization.
- Iterative solver convergence.
- Matrix conditioning estimates.
- Element integration cost.
- Contact convergence.
- Peak memory.
- Parallel assembly behavior.
- Cache locality.
- Mesh adaptation overhead.
- Total time-to-accuracy.

Avoid optimizing only geometric mesh quality or element count.

# 27. Adaptive Mesh Strategy Selection

The engine should be capable of comparing candidate strategies.

Potential choices:

1. Structured or swept mesh.
2. Unstructured tetrahedral mesh.
3. Hybrid structured–unstructured mesh.
4. Anisotropic hybrid mesh.
5. High-order curved mesh.
6. Mesh adapted to the selected QoI.

Use inexpensive heuristics to select initial candidates.

Use measured solver performance and convergence history to improve later decisions.

Do not run multiple expensive full FEA analyses solely for mesh selection unless explicitly authorized or justified by expected repeated-use savings.

---

# PART VIII — PROPOSED RUST MODULE ARCHITECTURE

# 28. Component Organization

Adapt this structure to actual repository conventions after Phase 0.

```text
fea-core/
  src/
    cad/
      mod.rs
      backend.rs
      import/
        step.rs
        iges.rs
        faceted.rs
      brep/
        topology.rs
        geometry.rs
        trim.rs
        identifiers.rs
      validation/
        solid.rs
        healing.rs
        tolerance.rs
      evaluation/
        curves.rs
        surfaces.rs
        projection.rs

    meshing/
      predicates.rs
      topology.rs
      metadata.rs
      spatial.rs

      surface/
        constrained_2d.rs
        advancing_front.rs
        cad_tessellation.rs

      volume/
        delaunay_3d.rs
        plc.rs
        boundary_recovery.rs
        refinement.rs
        sliver_removal.rs

      high_order/
        elevation.rs
        projection.rs
        jacobian.rs

      hybrid/
        partitioning.rs
        structured.rs
        transitions.rs

      metric/
        tensor.rs
        interpolation.rs
        composition.rs
        geometry.rs
        physics.rs

      operators/
        split.rs
        collapse.rs
        flip.rs
        relocate.rs

      optimization/
        quality.rs
        smoothing.rs
        solver_cost.rs

      motion/
        elasticity.rs
        harmonic.rs
        ale.rs

      transfer/
        projection.rs
        conservative.rs
        history.rs

    adaptivity/
      metric_bridge.rs
      controller.rs
```

Do not duplicate already implemented functionality.

Use focused modules and stable interfaces.

# 29. Core Data Interfaces

Suggested conceptual Rust structures:

```rust
pub struct MetricTensor3 {
    pub xx: f64,
    pub yy: f64,
    pub zz: f64,
    pub xy: f64,
    pub xz: f64,
    pub yz: f64,
}

pub struct GeometryAssociation {
    pub body_id: BodyId,
    pub entity_id: GeometryEntityId,
}

pub struct MeshQualityReport {
    pub min_scaled_jacobian: f64,
    pub min_dihedral_angle: Option<f64>,
    pub inverted_elements: usize,
    pub invalid_elements: usize,
}

pub enum MeshGenerationStrategy {
    Structured,
    Unstructured,
    Hybrid,
    Automatic,
}

pub trait CadGeometryBackend {
    fn capabilities(&self) -> CadCapabilities;

    fn import(
        &self,
        source: &CadSource,
        options: &CadImportOptions,
    ) -> Result<CadModel, CadError>;
}

pub trait MeshGenerator {
    fn generate(
        &self,
        model: &CadModel,
        options: &MeshingOptions,
    ) -> Result<GeneratedMesh, MeshingError>;
}

pub trait MeshAdaptationOperator {
    fn apply(
        &self,
        mesh: &mut Mesh,
        metric: &MetricField,
        constraints: &MeshConstraints,
    ) -> Result<AdaptationReport, AdaptationError>;
}
```

Actual APIs should be adapted to existing mesh ownership, identifiers, dimensionality, scalar types, and error-reporting conventions.

`MetricTensor3` values must be constructed through validated SPD-preserving operations when used as a physical metric.

Avoid copying large mesh structures merely to satisfy high-level interfaces.

---

# PART IX — PHASED IMPLEMENTATION ROADMAP

# 30. Phase 0 — Audit and Baselines

- [ ] Complete repository audit.
- [ ] Inventory OpenCascade API usage.
- [ ] Review CAD/meshing architecture.
- [ ] Review solver element compatibility.
- [ ] Inspect existing AMR and derivative work.
- [ ] Evaluate native Rust dependencies.
- [ ] Build CAD regression corpus.
- [ ] Establish memory/build/runtime baselines.
- [ ] Establish mesh-quality baselines.
- [ ] Produce detailed implementation dependency graph.

**Exit:** Verified technical baseline.

# 31. Phase 1 — CAD Backend Isolation

- [ ] Introduce backend-neutral CAD interfaces.
- [ ] Wrap existing CAD functionality.
- [ ] Create import/geometry regression fixtures.
- [ ] Implement backend capability reporting.
- [ ] Preserve stable geometry identities.
- [ ] Prevent cross-backend numerical behavior changes.
- [ ] Add compatibility tests.

**Exit:** Current application behavior preserved behind an abstracted backend.

# 32. Phase 2 — Native B-Rep Foundation

- [ ] Evaluate and select candidate Rust CAD kernel.
- [ ] Integrate supported STEP geometry.
- [ ] Implement canonical B-Rep mapping.
- [ ] Add topological validation.
- [ ] Add geometry projection/evaluation.
- [ ] Implement conservative healing.
- [ ] Preserve assembly/metadata where supported.
- [ ] Establish supported-file feature matrix.

**Exit:** Native backend handles its documented CAD subset reliably.

# 33. Phase 3 — Robust Geometric Predicates and 3D Delaunay

- [ ] Integrate robust predicates.
- [ ] Establish tetrahedral connectivity.
- [ ] Implement point location.
- [ ] Implement incremental insertion.
- [ ] Implement cavity reconstruction.
- [ ] Handle degeneracies.
- [ ] Add topology consistency tests.
- [ ] Add memory/performance benchmarks.

**Exit:** Verified point-set tetrahedralization.

# 34. Phase 4 — Constrained Volume Meshing

- [ ] Implement PLC representation.
- [ ] Validate boundary complexes.
- [ ] Implement constrained segment recovery.
- [ ] Implement facet recovery.
- [ ] Preserve cavities/material regions.
- [ ] Implement constrained refinement.
- [ ] Add diagnostics for unsupported inputs.
- [ ] Validate CAD-boundary conformity.

**Exit:** Verified meshing of supported arbitrary closed polyhedral domains.

# 35. Phase 5 — Quality Optimization and Sliver Removal

- [ ] Add 3D quality metrics.
- [ ] Implement quality-safe local flips.
- [ ] Implement constrained smoothing.
- [ ] Implement local remeshing.
- [ ] Add sliver detection/removal strategies.
- [ ] Add transactional rollback.
- [ ] Benchmark optimization efficiency.

**Exit:** Measurable quality improvements without topology/geometry corruption.

# 36. Phase 6 — High-Order Elements

- [ ] Implement geometric node elevation.
- [ ] Add CAD-parameter projection.
- [ ] Preserve shared curved interfaces.
- [ ] Check element mapping validity.
- [ ] Integrate supported high-order FEA elements.
- [ ] Verify high-order convergence.
- [ ] Evaluate memory and assembly cost.

**Exit:** Verified high-order mesh generation for supported element families.

# 37. Phase 7 — Hybrid Meshing Integration

- [ ] Reuse existing structured mesh generation.
- [ ] Integrate native unstructured volume meshes.
- [ ] Implement/verify transitions.
- [ ] Add region classification.
- [ ] Preserve material/contact metadata.
- [ ] Optimize regional element selection.
- [ ] Verify mixed-element solver compatibility.

**Exit:** Complete independently usable hybrid meshing path.

# 38. Phase 8 — Metric Tensor Infrastructure

- [ ] Define validated SPD metrics.
- [ ] Implement metric-space edge evaluation.
- [ ] Implement metric interpolation.
- [ ] Implement metric smoothing/gradation.
- [ ] Implement metric composition.
- [ ] Add geometry-driven size metrics.
- [ ] Integrate supported AMR error fields.
- [ ] Add complexity/budget control.

**Exit:** Verified spatial metric fields.

# 39. Phase 9 — Anisotropic Adaptation

- [ ] Metric-space splitting.
- [ ] Metric-space collapsing.
- [ ] Metric-space flipping.
- [ ] Metric-aware smoothing.
- [ ] CAD-constrained operations.
- [ ] Feature preservation.
- [ ] Remeshing rollback.
- [ ] AMR convergence integration.

**Exit:** Valid adaptive meshes with measured error/efficiency benefits.

# 40. Phase 10 — Mesh Motion and State Transfer

- [ ] Geometry-constrained mesh motion.
- [ ] Distortion monitoring.
- [ ] Elasticity-based smoothing.
- [ ] Remeshing triggers.
- [ ] State projection.
- [ ] Conservative transfer where required.
- [ ] History variable remapping.
- [ ] Relevant ALE support.
- [ ] Nonlinear equilibrium verification.

**Exit:** Supported moving/remeshed analyses preserve required state.

# 41. Phase 11 — Solver-Aware Optimization

- [ ] Profile assembly and sparse solves.
- [ ] Establish total-cost metrics.
- [ ] Add candidate mesh selection.
- [ ] Optimize size/quality objectives.
- [ ] Evaluate anisotropic mesh benefits.
- [ ] Evaluate memory efficiency.
- [ ] Verify quality versus solver cost.
- [ ] Compare against existing meshers.

**Exit:** Demonstrated time-to-accuracy improvements for target workloads.

# 42. Phase 12 — OpenCascade Removal

**Conditional final milestone.**

- [ ] Confirm CAD feature parity for committed supported workflows.
- [ ] Confirm STEP/IGES test coverage.
- [ ] Verify geometry healing and topology.
- [ ] Verify assembly metadata behavior.
- [ ] Verify native mesh validity.
- [ ] Verify full solver regression suite.
- [ ] Compare performance/resource usage.
- [ ] Remove production OpenCascade dependencies.
- [ ] Verify clean Rust build.
- [ ] Verify target platforms.
- [ ] Document remaining unsupported CAD features.
- [ ] Obtain explicit project approval before final retirement.

**Exit:** Production native-Rust CAD/meshing path no longer requires OpenCascade for the documented supported feature set.

---

# PART X — VERIFICATION AND PERFORMANCE

# 43. Mandatory CAD Test Corpus

Create or reuse models including:

- Rectangular block.
- Cylinder.
- Sphere and trimmed surfaces.
- Plate with holes.
- Countersunk fastener hole.
- Filleted bracket.
- Thin-walled shell.
- Multi-body bolted joint.
- Interfering/overlapping solids.
- Small-radius fillets.
- Sliver faces.
- Complex trimmed NURBS surfaces.
- Multi-material assembly.
- Open and malformed solids.
- CAD files with unit differences.
- Geometrically near-degenerate models.

For imported CAD, compare:

- Volume.
- Surface area.
- Bounding box.
- Face/edge/solid counts.
- Orientation.
- Watertightness.
- Mass-property consistency.
- Assembly transforms.
- Topology identifiers where meaningful.
- Geometry projection error.

# 44. Mandatory Meshing Benchmarks

| Benchmark | Primary Purpose |
|---|---|
| Cube tetrahedralization | 3D mesher correctness |
| Sphere | Curved surface fidelity |
| Cylinder with hole | Constrained boundary recovery |
| Thin plate with cutout | Small-feature preservation |
| Filleted bracket | Complex volume meshing |
| Multi-material body | Interface conservation |
| Bolted joint | Contact interface accuracy |
| Distorted tetrahedral mesh | Sliver elimination |
| Curved Tet10 mesh | High-order geometry/Jacobian |
| Large deformation | Mesh motion and remeshing |
| Anisotropic manufactured field | Metric adaptation |
| Cantilever and plate-with-hole FEA | Time-to-accuracy |

# 45. Numerical Acceptance Criteria

Required:

- Valid mesh topology.
- Correct domain representation.
- No unintended loss of geometric features.
- No accepted inverted elements.
- Supported element interpolation compatibility.
- CAD-boundary approximation within configured tolerance.
- Material interfaces preserved.
- Contact surfaces preserved.
- Successful integration with supported FEA solver cases.
- Error convergence on verified problems.
- Deterministic behavior under documented settings.

Mesh quality thresholds must be calibrated by element family, order, and formulation.

Avoid a single universal scaled-Jacobian or dihedral-angle threshold for all element types.

# 46. Performance Metrics

Record:

- CAD import time.
- Geometry healing time.
- Mesh generation time.
- Element count.
- DOFs.
- Peak memory.
- Allocation count.
- Mesh quality distribution.
- Sliver fraction.
- CAD approximation error.
- Solver assembly time.
- Sparse factorization time.
- Solver iteration count.
- Total FEA solve time.
- AMR cycles.
- Adaptation time.
- Time-to-target-accuracy.
- Build time.
- Binary size.
- Native dependency count.

Compare baseline and updated implementations at matched geometry and accuracy requirements.

Do not claim performance superiority based solely on smaller binaries or faster meshing.

# 47. External Reference Benchmarking

Gmsh, TetGen, Mmg, and OpenCascade may be used as external development/reference tools where license and distribution requirements permit.

Do not copy their implementation code into the native engine without explicit license analysis.

For each external comparison:

- Use equivalent inputs.
- Specify meshing settings.
- Record tool versions.
- Record hardware.
- Compare mesh validity.
- Compare solver accuracy.
- Compare full runtime.
- Compare memory.
- Explain unsupported feature differences.

Reference use does not justify introducing restricted dependencies into production.

---

# PART XI — PERFORMANCE ENGINEERING

# 48. Required Optimization Principles

Favor:

- Compact connectivity representation.
- Spatial acceleration structures.
- Robust adaptive-precision predicates.
- Incremental cavity updates.
- Local topology edits.
- In-place optimization where safe.
- Thread-local reusable workspaces.
- Parallel independent-region processing.
- Stable identifiers with compact solver indexing.
- Incremental adjacency/cache updates.
- Cache-aware mesh traversal.
- Minimal geometry projection recomputation.
- Measured SIMD opportunities.
- Explicit memory budgets.

Avoid:

- Full-mesh recomputation for every insertion.
- Global mesh cloning for routine local edits.
- Repeated CAD projection without caching.
- Naive O(N²) neighborhood searches.
- Unnecessary dynamic dispatch in hot loops.
- Uncontrolled metric anisotropy.
- Extremely aggressive optimization that outweighs solve-time benefits.

Assess whether parallelization introduces nondeterminism or excessive synchronization costs.

---

# PART XII — DELIVERABLES

# 49. Definition of Deliverables

- [ ] Repository audit and architectural gap report.
- [ ] CAD backend abstraction.
- [ ] Validated native B-Rep foundation.
- [ ] Supported STEP/IGES import pipeline.
- [ ] Geometry validation and conservative healing.
- [ ] Persistent CAD-to-mesh associations.
- [ ] Robust geometric predicates.
- [ ] 3D tetrahedralization.
- [ ] PLC/constrained boundary recovery.
- [ ] Quality refinement and sliver handling.
- [ ] High-order CAD-conforming mesh support.
- [ ] Hybrid meshing integration.
- [ ] SPD metric tensor infrastructure.
- [ ] Anisotropic adaptation.
- [ ] Mesh motion and supported ALE capability.
- [ ] Consistent state transfer.
- [ ] Solver-aware mesh optimizer.
- [ ] CAD and numerical verification corpus.
- [ ] Reproducible benchmark suite.
- [ ] Documented feature/support matrix.
- [ ] Dependency and license audit.
- [ ] Validated OpenCascade retirement plan.
- [ ] Final production readiness report.

# 50. Definition of Done

This EPIC is complete when all committed supported capabilities:

1. Operate through documented native-Rust production paths.
2. Have been verified with relevant engineering CAD inputs.
3. Generate valid compatible FEA meshes.
4. Preserve materials, loads, contacts, and geometric features.
5. Pass geometric, numerical, and solver integration tests.
6. Support independent hybrid mesh generation without AMR.
7. Support AMR-informed metric adaptation where implemented.
8. Preserve solver accuracy and state consistency.
9. Demonstrate benchmarked performance characteristics.
10. Do not require OpenCascade for the committed production feature set.
11. Explicitly document unsupported features and limitations.
12. Pass the full applicable regression suite.
13. Remain on a dedicated feature branch pending merge approval.

Do not mark incomplete advanced CAD capabilities as implemented based only on successful parsing or simple model demonstrations.

---

# PART XIII — CLAUDE CODE EXECUTION DIRECTIVE

# 51. Immediate Action

Read the latest repository `main` and carry out Phase 0.

**Trust, but verify.**

Treat all suggested algorithms and module names as design candidates until compared with the actual implementation.

## Execution instructions

1. Audit the current repository.
2. Identify OpenCascade usage and dependency chains.
3. Map existing meshing capabilities.
4. Evaluate candidate native Rust CAD libraries.
5. Produce a verified technical gap report.
6. Decompose this EPIC into smaller implementation issues.
7. Establish a dedicated feature branch.
8. Record baseline numerical tests and performance.
9. Start with CAD backend isolation and robust computational geometry.
10. Develop the native CAD and 3D meshing foundations independently where possible.
11. Integrate progressively with the existing solver and AMR infrastructure.
12. Run applicable tests after every meaningful implementation change.
13. Benchmark correctness, accuracy, memory, and runtime.
14. Commit verified milestones.
15. Continue through unblocked implementation tasks.
16. Keep `main` unchanged.
17. Do not remove OpenCascade until acceptance requirements are satisfied.

## Required progress reporting

At each milestone report:

- Files/modules changed.
- Capabilities implemented.
- Tests executed.
- Numerical accuracy results.
- Mesh-quality results.
- Performance results.
- Current limitations.
- Remaining dependencies.
- Next implementation tasks.

## Final directive

**Develop a native-Rust CAD and advanced meshing engine that optimizes the entire finite-element computational workflow, from geometric import through error-controlled solution, rather than optimizing mesh generation in isolation.**

Preserve mathematical validity, accurate CAD boundaries, element compatibility, material/contact associations, and solver correctness at every stage.

The final architecture must be modular, benchmark-driven, auditable, and extensible, providing a production foundation for future nonlinear structural mechanics, multiphysics, design optimization, and adaptive meshing capabilities.