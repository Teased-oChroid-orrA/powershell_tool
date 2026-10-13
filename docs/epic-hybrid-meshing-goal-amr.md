# EPIC: Advanced Hybrid Meshing & Goal-Oriented Adaptive Mesh Refinement for FEA-Core

**Type:** Epic / Major Numerical Solver Enhancement  
**Repository:** `powershell_tool`  
**Target Component:** `fea-core`  
**Priority:** High  
**Implementation Language:** Rust  
**Development Strategy:** Dedicated feature branch; no direct changes to `main`  
**Status:** Ready for repository audit and phased implementation

---

## 1. Executive Summary

Design, implement, verify, and optimize two complementary but independently operable capabilities within FEA-Core:

**System A — Hybrid Multicriteria Adaptive Mesh Refinement (AMR)**

Develop a comprehensive goal-oriented adaptive refinement framework that combines:

- Always-active goal-oriented adjoint estimation during supported AMR cycles.
- Dual-weighted residual (DWR) indicators.
- SPR+ stress recovery.
- H(div)-conforming equilibrated stress reconstruction.
- Physics-specific spatial refinement criteria.
- Multicriteria error normalization and fusion.
- Constraint-preserving refinement.
- Automated mesh refinement and eventual coarsening.
- Nonlinear, contact, and eigenvalue-specific integration.

**System B — Hybrid Structured–Unstructured Mesh Generation and Optimization**

Develop a solver-aware hybrid meshing framework that automatically:

- Identifies geometrically simple and complex regions.
- Generates structured meshes wherever they provide measurable benefits.
- Generates unstructured meshes around complex geometries and physical features.
- Produces high-quality, conforming transitions between regions.
- Supports mixed element topologies when mathematically and numerically compatible.
- Optimizes element quality, orientation, distribution, and computational cost.
- Integrates seamlessly with System A without depending on it.

### Primary objective

Deliver the most computationally efficient mesh that satisfies engineering accuracy and physical fidelity requirements for the chosen analysis.

The optimization target is not simply the smallest element count.

Instead, optimize the full analysis cost:

**Mesh generation + assembly + linear/nonlinear solution + error estimation + adaptation + state transfer + repeated solves**

subject to:

- Accuracy requirements.
- Element validity.
- Mesh conformity and solver compatibility.
- Contact resolution.
- Stress and displacement fidelity.
- Acceptable numerical conditioning.
- Stability of nonlinear and eigenvalue analyses.
- Practical resource limits.

There is no universally optimal or mathematically "perfect" mesh for every geometry, solver, and quantity of interest. The objective is to approach the best verified mesh configuration for the actual analysis.

---

# 2. Mandatory Operating Instructions for Claude Code

Before making changes:

1. Read and understand the latest `main` branch of `powershell_tool`.
2. Audit the existing FEA-Core architecture, numerical methods, meshing capabilities, solver interfaces, element implementations, tests, benchmarks, and performance characteristics.
3. Compare the existing implementation against every requirement in this EPIC.
4. Identify functionality that is already implemented, partially implemented, or absent.
5. Reuse existing, verified capabilities wherever practical.
6. Avoid redundant abstractions, duplicate numerical kernels, unnecessary dependencies, and parallel implementations of existing infrastructure.
7. Verify mathematical assumptions against established finite-element formulations.
8. Evaluate performance, accuracy, memory, and architectural implications before selecting implementation strategies.
9. Document findings, limitations, dependencies, and deviations from this plan.
10. Implement incrementally on a dedicated feature branch.

**Trust, but verify.**

Do not treat suggested algorithms, default values, module names, or architectural choices as unquestionable requirements. Preserve the engineering objectives while choosing the best implementation supported by the actual repository.

### Git requirements

- Create a dedicated branch, suggested: `feature/hybrid-meshing-goal-oriented-amr`.
- Never directly modify `main`.
- Use incremental commits with descriptive messages.
- Keep Systems A and B logically separable.
- Preserve backward compatibility wherever practical.
- Do not introduce breaking public API changes without documenting the migration.
- Run relevant tests after each substantial implementation step.
- Track implementation progress in the EPIC and linked issues where repository permissions permit.
- Never claim a benchmark or test has passed without executing it.

### Engineering requirements

Prioritize:

1. Mathematical correctness.
2. Solver stability.
3. Physical accuracy.
4. Computational efficiency.
5. Memory efficiency.
6. Maintainability.
7. Extensibility.

No optimization is acceptable if it causes unacknowledged degradation in numerical correctness or required engineering accuracy.

---

# PART I — SYSTEM A: HYBRID MULTICRITERIA ADAPTIVE MESH REFINEMENT

## 3. Architecture

Implement a modular error-estimation and adaptive-refinement framework.

Conceptual execution:

```text
PRIMAL FEA SOLVER
  |
  |-- Converged displacement/state solution
  |-- Residuals and consistent tangent
  |-- Element stresses/strains
  |-- Contact and boundary state
  |
  v
ADAPTIVE ERROR ORCHESTRATOR
  |
  |----------------------------|
  |                            |
  v                            v
GOAL-ORIENTED ESTIMATOR       PHYSICS ESTIMATORS
  |                            |
  |-- Adjoint solve             |-- SPR+
  |-- DWR localization         |-- H(div) reconstruction
  |-- Goal sensitivity         |-- Contact residuals
  |                            |-- Region-specific checks
  |----------------------------|
  |
  v
ERROR NORMALIZATION AND FUSION
  |
  |-- Normalized ranking
  |-- Spatial weighting
  |-- Physics constraints
  |-- Reliability checks
  |
  v
ADAPTIVE REFINEMENT CONTROLLER
  |
  |-- Dörfler marking
  |-- Quality constraints
  |-- Element-size restrictions
  |-- DOF and memory budgets
  |-- Refinement/coarsening rules
  |
  v
MESH ADAPTATION
  |
  |-- State projection
  |-- Constraint preservation
  |-- Contact remapping
  |-- Cache invalidation
  |
  v
RE-SOLVE AND VERIFY CONVERGENCE
```

Estimator implementations must not directly depend on the mesh generator.

The mesh generator must not contain the physics-specific error-estimation logic.

## 4. Goal-Oriented Adjoint Estimation

The adjoint estimator shall be the primary goal-oriented refinement mechanism for supported analysis types.

Given the discretized nonlinear residual equation:

\[
R(u_h,\lambda)=0
\]

and quantity of interest:

\[
J(u_h)
\]

solve the appropriate discrete adjoint equation:

\[
K_T^T z_h=J_u^T
\]

with the consistent tangent:

\[
K_T=\frac{\partial R}{\partial u}
\]

using an explicitly documented residual and adjoint sign convention.

Implement dual-weighted residual estimation with appropriate adjoint enrichment or defect reconstruction.

Conceptually:

\[
\eta_e^{adj}
\approx
\left|
R_e(u_h)[z^+-I_h z^+]
\right|
\]

The complete implementation must account for relevant boundary terms, interface terms, contact terms, and mathematical assumptions.

### Requirements

- Configurable quantities of interest.
- Evaluation of QoI values and derivatives.
- Tangent-transpose solution.
- Reuse of suitable solver factorizations where possible.
- Enriched adjoint or verified defect reconstruction.
- Localized error contributions.
- Preservation of signed contributions.
- Separate nonnegative marking indicators.
- Estimated QoI error.
- Error-effectivity diagnostics.
- Handling of multiple QoIs.
- Support for nonlinear extensions.
- Explicit unsupported-feature reporting.

### Initial supported QoIs

- Nodal displacement.
- Relative displacement.
- Reaction force.
- Averaged stress over a defined region.
- Fastener or joint load.
- Contact force.
- Compliance.

Additional QoIs must be supported through extensible interfaces.

Do not assume that unregularized pointwise stresses or singular peak values admit reliable adjoint error estimates.

## 5. Physics Estimators

### 5.1 SPR+ Stress Recovery

Implement or reuse a verified Superconvergent Patch Recovery enhancement.

Required capabilities:

- Patch topology construction.
- Polynomial stress recovery.
- Appropriate element family support.
- Interior and boundary patches.
- Material-interface handling.
- Ill-conditioned patch detection.
- Appropriate fallback procedures.
- Recovery-based local error indicators.
- Energy-norm estimation under stated assumptions.

Stress discontinuities caused by distinct materials, boundary conditions, or physical interfaces must not be smoothed indiscriminately.

### 5.2 H(div)-Conforming Equilibrated Stress Reconstruction

Implement an equilibrated stress-reconstruction estimator suitable for the supported structural formulation.

Requirements:

- Appropriate tensor stress spaces.
- Compatible element families.
- Equilibrium constraints.
- Interelement normal-traction compatibility.
- Boundary traction handling.
- Contact interface treatment.
- Patch reconstruction or suitable localized mixed problems.
- Local and global estimator calculations.
- Mathematical validity checks.
- Reliability diagnostics.

Implement the formulation first for supported linear-elastic cases.

Extend to nonlinear/contact problems only after the prerequisites have been verified.

Do not incorrectly substitute a scalar H(div) flux reconstruction for a structural stress tensor reconstruction.

### 5.3 Physics-Aware Region Selection

Develop a region-classification mechanism supporting:

| Region | Default Estimation Strategy |
|---|---|
| Smooth continuum | DWR + SPR+ |
| Contact region | DWR + equilibrated stress reconstruction |
| Contact perimeter | DWR + contact-specific residual checks |
| Fastener bearing region | DWR + localized physics estimation |
| Geometric discontinuity | DWR + suitable recovery/residual estimator |
| Buckling-sensitive region | Eigenvalue-specific estimator |
| Singular region | Specialized error behavior and termination policies |

Estimator assignment must be configurable and based on supported formulations.

---

## 6. Multicriteria Error Fusion

Implement a general error-fusion pipeline.

The proposed weighted blend is:

\[
\eta_e^{blend}
=
\alpha\bar{\eta}_e^{adj}
+
(1-\alpha)\beta_e\bar{\eta}_e^{phys}
\]

where:

- \(\alpha\) is the global adjoint priority.
- \(\beta_e\) is the spatial physics weighting.
- \(\bar{\eta}^{adj}\) is the normalized adjoint marking indicator.
- \(\bar{\eta}^{phys}\) is the normalized physics marking indicator.

This is a dimensionless **refinement-ranking score**, not automatically a rigorous global error bound.

### Required fusion strategies

**Weighted Sum**

Balances goal sensitivity with physical estimation.

**Weighted Maximum**

Retains significant contributions from either estimator.

**Constraint-Preserving Fusion**

Uses goal-oriented ranking while enforcing mandatory refinement for regions with unresolved physics-error requirements.

Make the constraint-preserving strategy the intended production configuration after validation.

### Normalization

Support:

- Maximum normalization.
- RMS-based scaling.
- Percentile-based scaling.
- Zero-error handling.
- Invalid-value handling.
- Reproducible parallel reductions.

Maintain original dimensioned values separately from normalized ranking fields.

### Reliability safeguards

- Never silently discard failed estimators.
- Do not consider a low adjoint contribution sufficient evidence of physical accuracy.
- Preserve mandatory contact-resolution requirements.
- Prevent isolated singularities from forcing indefinite refinement.
- Preserve separate stopping conditions for QoI and physics accuracy.
- Detect incompatible estimator assumptions.
- Report unreliable estimates explicitly.

---

## 7. Adaptive Refinement Controller

Implement Dörfler bulk marking and configurable refinement strategies.

For nonnegative indicators \(q_e\), select a marked set satisfying:

\[
\sum_{e\in M}q_e
\ge
\theta\sum_{e\in\mathcal T_h}q_e
\]

When using squared indicators, define the convention explicitly.

Initial experimental configuration:

| Setting | Initial Value |
|---|---|
| Adjoint priority | 0.70 |
| Dörfler fraction | 0.50 |
| Maximum AMR cycles | 8 |
| QoI tolerance | User-configurable |
| Physics tolerance | User-configurable |
| Maximum DOFs | Resource/configuration dependent |
| Maximum/minimum element sizes | Geometry and physics dependent |
| Contact protection | Enabled where supported |

These are preliminary configuration defaults requiring benchmark validation.

### Required refinement controls

- h-refinement.
- Element quality preservation.
- Conformity and hanging-node treatment.
- Geometric feature protection.
- Contact interface compatibility.
- DOF and memory budgets.
- Refinement stagnation detection.
- Refinement history.
- Mesh-invalidity detection.
- Convergence checking.

### Future extensions

Evaluate, but do not prematurely implement:

- p-refinement.
- hp-adaptivity.
- Anisotropic refinement.
- Goal-oriented coarsening.
- Error-equidistribution strategies.

---

# PART II — SYSTEM B: HYBRID STRUCTURED–UNSTRUCTURED MESHING

## 8. Objective

Implement a completely independent meshing subsystem capable of producing an efficient, high-quality computational mesh through automatic selection and combination of structured and unstructured regions.

**Fundamental strategy:**

Use structured mesh topology in simple, regular regions.

Use unstructured mesh topology in geometrically complex regions, around stress concentrations, around contact surfaces, and wherever conforming structured meshing becomes inefficient or impractical.

Construct robust transitions between these regions.

Optimize the resulting mesh according to measurable solver performance, numerical quality, and physical-accuracy criteria.

Hybrid meshing must function without AMR enabled.

When AMR is enabled, the systems shall cooperate without sacrificing their independent responsibilities.

## 9. Automatic Geometry and Region Classification

Develop a geometry-aware domain partitioner that examines:

- Geometric regularity.
- Curvature.
- Topological complexity.
- Feature dimensions.
- Holes and cutouts.
- Fillets and radii.
- Thickness variations.
- Sharp corners.
- Material interfaces.
- Contact boundaries.
- Load and restraint locations.
- Expected stress concentrations.
- Structured-mesh feasibility.
- Desired element orientation.

Classify candidate regions into:

```text
STRUCTURED REGION
  - Simple and regular geometry
  - Sweepable or mappable topology
  - Favorable element alignment
  - Low geometric complexity

UNSTRUCTURED REGION
  - Irregular geometry
  - Complex interfaces
  - Holes, cutouts, and fillets
  - Complex loading/contact
  - Unfavorable structured topology

TRANSITION REGION
  - Connects distinct mesh topologies
  - Controls element-size progression
  - Preserves geometry and compatibility
```

Classification must consider meshing cost and expected solver cost, not geometric complexity alone.

A highly regular region may still require localized refinement due to loads, materials, or engineering outputs.

## 10. Structured Mesh Generation

Where supported, favor regular quadrilateral or hexahedral topology.

### 2D

- Structured quadrilateral meshing.
- Mapped grids.
- Multi-block structured meshing.
- Boundary-aligned discretization.

### 3D

- Structured hexahedral meshing.
- Swept meshes.
- Multi-block decompositions.
- Appropriate boundary-aligned topology.

### Desired characteristics

- Regular connectivity.
- Low element distortion.
- Controlled aspect ratios.
- Consistent orientation.
- Predictable indexing.
- Reduced connectivity overhead where feasible.
- Favorable sparse-matrix characteristics.
- Good cache locality.

Do not assume structured meshes are universally faster.

Performance depends on element formulation, integration rules, mesh orientation, conditioning, sparse ordering, and solver implementation.

## 11. Unstructured Mesh Generation

Use unstructured meshing where required by geometry or physics.

Support appropriate available element families, with incremental expansion to:

- Triangles.
- Quadrilaterals.
- Tetrahedra.
- Hexahedra.
- Prisms/wedges.
- Pyramids, when compatible transition elements are supported.

Use verified algorithms or existing meshing libraries as appropriate.

Potential algorithm families include:

- Constrained Delaunay methods.
- Advancing-front methods.
- Octree-based methods.
- Local remeshing.
- Boundary-conforming meshing.
- Region-dependent size fields.

Selection must be based on the existing implementation, licensing, maintainability, numerical compatibility, and verified performance.

Do not add a new meshing dependency without evaluating existing capabilities.

## 12. Structured–Unstructured Transitions

This is a critical numerical and architectural requirement.

The mesh must transition smoothly between different element topologies.

For example:

```text
STRUCTURED REGION
  |
  | Regular quadrilateral/hexahedral cells
  |
  v
TRANSITION REGION
  |
  | Conforming mixed-element topology
  | Compatible shape functions
  | Controlled size and distortion
  |
  v
UNSTRUCTURED REGION
  |
  | Geometry-conforming elements
  | Localized feature resolution
```

### Requirements

- Valid shared interfaces.
- Conforming nodal connectivity where supported.
- Consistent geometry representation.
- Compatible finite-element interpolation.
- Correct face orientation.
- Appropriate quadrature.
- Compatible mechanical fields.
- No unintended cracks or disconnected nodes.
- Proper material and boundary tagging.
- Appropriate treatment of hanging nodes if used.
- Valid contact surface discretization.
- Bounded and configurable element-size growth.

In 3D, evaluate supported hex-to-tet transition strategies, including wedge/pyramid transition elements or compatible all-tet transition layers.

**Important:** A geometrically matching interface does not automatically imply a mathematically compatible discretization. Finite-element continuity and constraint requirements must be verified for every supported mixed-element pairing.

Do not introduce unsupported transition element families merely to achieve an aesthetically desirable mesh.

## 13. Automatic Mesh Quality Optimization

Implement a solver-aware mesh-quality evaluation and optimization pipeline.

### Geometry-based criteria

- Jacobian determinant and scaled Jacobian.
- Element inversion.
- Aspect ratio.
- Skewness.
- Warpage where relevant.
- Minimum angles.
- Shape regularity.
- Element-size progression.
- Curvature approximation.
- Surface conformity.
- Local orientation.
- Edge and face compatibility.

### Solver-aware criteria

Investigate correlations with:

- Tangent/matrix conditioning.
- Sparse matrix nonzero count.
- Assembly cost.
- Factorization fill-in.
- Iterative solver convergence.
- Preconditioner effectiveness.
- Cache/memory behavior.
- Contact convergence.
- Integration cost.
- Overall solve time.

Only use validated, practically measurable criteria in runtime optimization.

### Optimization techniques

Evaluate and implement appropriate combinations of:

- Node smoothing.
- Topological optimization.
- Edge flipping.
- Edge collapsing.
- Edge splitting.
- Element relocation.
- Local reconnection.
- Size-field smoothing.
- Boundary-constrained smoothing.
- Quality-preserving remeshing.

Geometry modification must respect CAD/geometry tolerances, protected features, loads, materials, and contact boundaries.

## 14. Solver-Aware Mesh Cost Optimization

Introduce a configurable cost model.

Conceptually:

\[
C_{total} =
w_g C_{generation}
+w_a C_{assembly}
+w_s C_{solve}
+w_e C_{estimation}
+w_r C_{adaptation}
\]

The weights must reflect an actual intended workload or be replaced by measured times where available.

The optimization is subject to:

\[
E_{QoI}\leq\epsilon_{QoI}
\]

\[
E_{physics}\leq\epsilon_{physics}
\]

and all applicable validity and resource constraints.

A low-cost mesh that violates engineering accuracy constraints is unacceptable.

### Candidate mesh evaluation

Where practical, compare:

1. Predominantly structured.
2. Predominantly unstructured.
3. Hybrid structured–unstructured.
4. Hybrid mesh with targeted local refinement.

Do not perform expensive full-scale candidate solves by default merely to choose a mesh. Use inexpensive geometric cost estimates and cached performance data initially, with measured solver performance guiding subsequent adaptation.

The optimizer should prefer the best validated candidate, not always the hybrid candidate.

## 15. Element Orientation and Physics Alignment

Mesh generation should consider dominant physical and geometric directions when reliable information is available.

Examples:

- Primary loading directions.
- Thin-wall directions.
- Bending-dominated regions.
- Curved boundaries.
- Contact interfaces.
- Expected stress gradients.

Prefer physically useful orientation when it improves accuracy or conditioning.

Do not automatically elongate elements along stress directions without a verified anisotropic meshing strategy.

Structured elements should not be retained if they become excessively distorted, poorly aligned, or numerically inferior.

## 16. Initial Mesh Density Prediction

Implement configurable initial mesh-size fields incorporating:

- Geometry curvature.
- Feature proximity.
- Contact interface size.
- Material discontinuities.
- Loading and restraint locations.
- User-defined minimum/maximum size.
- Boundary-resolution requirements.
- Optional historical solution information.

The initial mesh-size predictor should reduce unnecessary AMR cycles while remaining inexpensive.

It must not assume an unknown stress field is already known.

If a previous compatible solution exists, it may be used as a guide after appropriate validation.

---

# PART III — INTEGRATION OF BOTH SYSTEMS

## 17. Unified Mesh Lifecycle

Both capabilities must remain independently configurable.

```text
CAD / GEOMETRY / ANALYSIS DEFINITION
              |
              v
HYBRID MESH GENERATOR
              |
     +--------+---------+
     |                  |
     v                  v
STRUCTURED          UNSTRUCTURED
REGIONS             REGIONS
     |                  |
     +--------+---------+
              |
              v
CONFORMING TRANSITION GENERATOR
              |
              v
MESH QUALITY / SOLVER-COST CHECK
              |
              v
INITIAL HYBRID MESH
              |
              v
PRIMAL FEA SOLUTION
              |
              v
HYBRID ERROR ESTIMATION (OPTIONAL AMR)
              |
     +--------+---------+
     |                  |
     v                  v
ADJOINT DWR        PHYSICS RECOVERY
     |                  |
     +--------+---------+
              |
              v
MULTICRITERIA ERROR FUSION
              |
              v
REFINEMENT MARKING
              |
              v
HYBRID-AWARE LOCAL MESH ADAPTATION
              |
              v
QUALITY / CONFORMITY / STATE TRANSFER
              |
              v
RE-SOLVE AND VERIFY
              |
       [UNTIL CRITERIA MET]
```

### Critical principle

**Hybrid meshing determines the preferred mesh topology. AMR determines where numerical resolution must improve.**

Both systems cooperate but do not duplicate responsibilities.

## 18. Hybrid-Aware AMR Behavior

Refinement policies must respect the local mesh family and structure.

### Structured regions

Prefer topology-preserving refinement where practical.

Possible strategies:

- Regular cell subdivision.
- Structured-block refinement.
- Compatible local refinement.
- Transition-zone enlargement where necessary.

### Unstructured regions

Use supported local refinement and remeshing strategies.

Possible techniques:

- Element subdivision.
- Edge splitting.
- Local reconnection.
- Local retriangulation/remeshing.
- Geometry-constrained refinement.

### Transition regions

Ensure that refinement does not break conformity or introduce unsupported interfaces.

The adaptation controller must be capable of:

- Updating transition topology.
- Maintaining compatible interpolation.
- Preserving valid geometry.
- Rebuilding affected adjacency information.
- Updating boundary/contact tags.
- Reconstructing required patch caches.

### Important qualification

AMR must not be restricted exclusively to unstructured regions.

The primary refinement effort may be concentrated around complex features, but structured regions must remain refinable when a verified estimator identifies insufficient resolution.

A strict unstructured-only AMR policy would risk missing bending, bulk deformation, buckling, and other errors in otherwise simple regions.

## 19. Mesh State and Data Preservation

After adaptation, preserve or correctly reconstruct:

- Node and element identifiers/mappings.
- Geometric classification.
- Material assignments.
- Boundary conditions.
- Applied loads.
- Contact pairs.
- Displacement fields.
- Stress/strain data.
- Internal nonlinear variables.
- Plastic history variables, where supported.
- Solver metadata.
- Error history.
- Mesh refinement ancestry.

History-dependent internal variables require physically appropriate transfer operators, not arbitrary interpolation.

Conservation, admissibility, and re-equilibration checks must be applied as required by the constitutive/contact formulation.

Invalid state transfer must prevent unsafe continuation.

## 20. Shared Infrastructure

Reuse a common set of abstractions:

- Mesh representation.
- Geometry topology.
- Element family metadata.
- Mesh adjacency.
- Patch topology.
- Region classification.
- Mesh quality evaluation.
- Refinement operators.
- Mesh constraints.
- State transfer.
- Convergence history.
- Performance diagnostics.

Avoid unnecessary duplication between the initial hybrid mesher and AMR mesh modification backend.

However, do not collapse the mesh generator and AMR controller into a single monolithic implementation.

---

# PART IV — RUST ARCHITECTURE

## 21. Suggested Module Structure

This is conceptual and must be reconciled with the actual repository architecture.

```text
fea-core/
  src/
    meshing/
      mod.rs
      geometry/
        classification.rs
        topology.rs
        feature_detection.rs
      structured/
        mapped.rs
        swept.rs
        multiblock.rs
      unstructured/
        generator.rs
        remeshing.rs
      transition/
        connectivity.rs
        compatibility.rs
        conformity.rs
      optimization/
        quality.rs
        smoothing.rs
        cost_model.rs
        size_field.rs
      hybrid/
        partitioner.rs
        orchestrator.rs
        metadata.rs

    adaptivity/
      mod.rs
      orchestrator.rs
      context.rs
      error_field.rs
      diagnostics.rs
      goal/
        functional.rs
        adjoint.rs
        dwr.rs
      physics/
        spr_plus.rs
        equilibrated_stress.rs
        patch_cache.rs
        zone_resolver.rs
      fusion/
        normalization.rs
        weighted.rs
        constraints.rs
      marking/
        dorfler.rs
        budget.rs
      controller/
        convergence.rs
        state_transfer.rs
        history.rs
```

Adapt existing module boundaries if doing so reduces complexity or improves compatibility.

## 22. Required Abstractions

Provide appropriately scoped interfaces representing:

- `MeshGenerator`
- `HybridMeshGenerator`
- `GeometryRegionClassifier`
- `MeshQualityEvaluator`
- `MeshOptimizer`
- `MeshCostModel`
- `MeshTransitionBuilder`
- `ErrorEstimator`
- `GoalFunctional`
- `PhysicsZoneResolver`
- `ErrorFusionStrategy`
- `RefinementMarker`
- `AdaptiveController`
- `StateTransferOperator`

Use Rust traits when multiple implementations or extensibility justify them.

Avoid excessive dynamic dispatch or unnecessary trait-object allocations in high-frequency numerical loops.

Prefer contiguous data structures, efficient immutable views, and reusable buffers.

---

# PART V — IMPLEMENTATION ROADMAP

## 23. Phase 0 — Comprehensive Repository Audit

**Mandatory before implementation.**

- [ ] Read current `main` branch.
- [ ] Identify actual FEA-Core crate/module organization.
- [ ] Audit element families and interpolation orders.
- [ ] Audit existing structured/unstructured mesh generation.
- [ ] Audit geometry input and topology support.
- [ ] Audit existing AMR or refinement utilities.
- [ ] Audit sparse solvers and matrix-transpose support.
- [ ] Audit nonlinear/contact implementations.
- [ ] Audit eigenvalue capabilities.
- [ ] Audit stress recovery.
- [ ] Audit mesh quality checks.
- [ ] Audit state transfer.
- [ ] Establish numerical regression baselines.
- [ ] Establish performance baselines.
- [ ] Produce a dependency graph.
- [ ] Map each EPIC requirement to existing code or required work.
- [ ] Identify limitations and revise sequencing accordingly.

**Exit gate:** Verified audit with no assumptions about nonexistent functionality.

## 24. Phase 1 — Shared Mesh and Numerical Infrastructure

- [ ] Establish common mesh interfaces.
- [ ] Define element topology/compatibility metadata.
- [ ] Implement or reuse adjacency infrastructure.
- [ ] Implement mesh-quality metrics.
- [ ] Create geometry/physics region metadata.
- [ ] Create solver-state snapshots.
- [ ] Establish cache invalidation rules.
- [ ] Create mesh adaptation transaction/rollback safeguards.
- [ ] Add foundational unit tests.

**Exit gate:** Shared infrastructure functions without altering validated solver results.

## 25. Phase 2 — Hybrid Meshing Foundation

- [ ] Automatic geometry classification.
- [ ] Structured meshing for supported regions.
- [ ] Unstructured meshing for complex regions.
- [ ] Domain partitioning.
- [ ] Mixed-region connectivity.
- [ ] Conforming transition generation.
- [ ] Element compatibility validation.
- [ ] Mesh quality validation.
- [ ] Boundary/material tag preservation.
- [ ] Initial mesh-size field generation.
- [ ] Mesh generation diagnostics.

**Exit gate:** Verified mixed structured–unstructured meshes solve representative problems correctly without AMR.

## 26. Phase 3 — Hybrid Mesh Optimization

- [ ] Geometry-based mesh quality optimization.
- [ ] Local topology improvement.
- [ ] Boundary-preserving smoothing.
- [ ] Element-size progression control.
- [ ] Structured/unstructured partition optimization.
- [ ] Solver-cost measurements.
- [ ] Mesh candidate comparison.
- [ ] Automatic fallback for invalid transitions.
- [ ] Performance benchmarks.

**Exit gate:** Demonstrate measurable improvement in appropriate cases without accuracy regression.

## 27. Phase 4 — Error Estimation Infrastructure

- [ ] Common estimator interface.
- [ ] Immutable estimator context.
- [ ] Error-field storage.
- [ ] Estimator diagnostics.
- [ ] Error reliability status.
- [ ] Goal functional interfaces.
- [ ] Patch topology cache.
- [ ] Mock-estimator tests.
- [ ] Deterministic element indexing.

**Exit gate:** Independently executing error estimators produce correct, reproducible fields.

## 28. Phase 5 — Adjoint/DWR Estimator

- [ ] QoI value/derivative evaluation.
- [ ] Tangent transpose solution.
- [ ] Adjoint enrichment/defect reconstruction.
- [ ] Local residual evaluation.
- [ ] Signed error contribution tracking.
- [ ] Nonnegative marking indicators.
- [ ] QoI error estimation.
- [ ] Sensitivity tests.
- [ ] Convergence tests.

**Exit gate:** Demonstrated QoI convergence and validated adjoint derivatives.

## 29. Phase 6 — Physics Estimators

- [ ] SPR+ implementation/integration.
- [ ] Stress recovery validation.
- [ ] Equilibrated stress reconstruction for supported elasticity.
- [ ] Equilibrium tests.
- [ ] Boundary traction tests.
- [ ] Contact-specific reconstruction prerequisites.
- [ ] Physics region selection.
- [ ] Estimator reliability handling.

**Exit gate:** Verified recovery/equilibrium behavior and clearly documented supported physics.

## 30. Phase 7 — Multicriteria Fusion

- [ ] Weighted-sum fusion.
- [ ] Weighted-maximum fusion.
- [ ] Constraint-preserving fusion.
- [ ] Normalization strategies.
- [ ] Spatial physics weights.
- [ ] Contact protection.
- [ ] Invalid-estimator handling.
- [ ] Singular-field handling.
- [ ] Fusion correctness tests.

**Exit gate:** Combined ranking respects goal priority and independently specified physics constraints.

## 31. Phase 8 — Hybrid-Aware AMR

- [ ] Dörfler marking.
- [ ] Structured-region refinement.
- [ ] Unstructured-region refinement.
- [ ] Transition-region update.
- [ ] Mesh conformity checks.
- [ ] Element quality preservation.
- [ ] Mesh-size controls.
- [ ] DOF/memory budgets.
- [ ] State transfer.
- [ ] Re-solve integration.
- [ ] Convergence/stagnation termination.

**Exit gate:** Full solve-estimate-refine-re-solve cycle completes correctly for supported linear cases.

## 32. Phase 9 — Advanced Solver Integration

- [ ] Nonlinear load increment adaptation.
- [ ] Consistent tangent/adjoint integration.
- [ ] Contact active-set handling.
- [ ] Contact reconstruction and remapping.
- [ ] Nonlinear history transfer.
- [ ] Contact traction verification.
- [ ] Eigenvalue-specific sensitivity support.
- [ ] Mode tracking and repeated-eigenvalue safeguards.
- [ ] Robust recovery/failure handling.

**Exit gate:** Advanced adaptive analyses preserve physical consistency and converge under verified benchmark conditions.

## 33. Phase 10 — Production Performance and Hardening

- [ ] Profile mesh generation.
- [ ] Profile element assembly.
- [ ] Profile linear/nonlinear solves.
- [ ] Profile adjoint solve.
- [ ] Profile recovery estimation.
- [ ] Profile local remeshing.
- [ ] Profile state transfer.
- [ ] Optimize allocations.
- [ ] Evaluate parallel execution.
- [ ] Evaluate factorization reuse.
- [ ] Optimize cache behavior.
- [ ] Verify deterministic behavior.
- [ ] Add structured performance reports.
- [ ] Complete benchmark suite.
- [ ] Update API/documentation.
- [ ] Perform full regression testing.
- [ ] Complete production readiness review.

**Exit gate:** Accuracy, stability, performance, and resource-consumption claims are supported by reproducible measurements.

---

# PART VI — TESTING AND VERIFICATION

## 34. Required Numerical Benchmarks

| Problem | Primary Validation |
|---|---|
| Uniform tension patch test | Element compatibility and stress recovery |
| Cantilever bending | Mesh convergence and displacement accuracy |
| Plate with circular hole | Structured–unstructured transitions and local stress resolution |
| Curved beam | Geometric conformity and orientation |
| 3D block with through-hole | Hybrid 3D topology |
| Multi-material structure | Material interface compatibility |
| Hertzian elastic contact | Contact pressure refinement |
| Bolted joint preload | Localized loading/contact behavior |
| Nonlinear contact separation | Active-set and adaptation stability |
| Linear buckling example | Eigenvalue sensitivity and mode accuracy |
| Distorted mesh benchmark | Mesh-quality effects |
| Thin structural member | Aspect ratio and anisotropy sensitivity |

Use closed-form or manufactured solutions where practical, independent reference solutions elsewhere, and documented convergence studies where exact solutions are unavailable.

## 35. Mandatory Mesh Strategy Comparisons

For representative problems, compare:

**Mesh generation**

1. Structured-only where feasible.
2. Unstructured-only.
3. Hybrid structured–unstructured.
4. Hybrid mesh with quality optimization.

**Adaptivity**

1. No AMR.
2. Adjoint-only AMR.
3. Physics-only AMR.
4. Hybrid multicriteria AMR.

**Combined**

Evaluate suitable combinations of meshing strategy and refinement strategy.

Compare on equivalent accuracy requirements, not solely equal element counts.

## 36. Performance Metrics

Capture:

- Element count.
- Total DOFs.
- Matrix nonzero count.
- Mesh generation time.
- Assembly time.
- Linear solve time.
- Nonlinear solve time.
- Adjoint solve time.
- Recovery estimation time.
- Refinement/remeshing time.
- State transfer time.
- Total time-to-accuracy.
- Peak memory.
- Solver iterations.
- Condition estimates where practical.
- QoI actual/reference error.
- Error estimator effectivity.
- Physics residual/error measures.
- Mesh quality distribution.
- Refinement cycle count.
- Percentage of structured versus unstructured elements.

Retain machine/environment configuration and benchmark inputs to support meaningful comparisons.

## 37. Performance Acceptance Policy

Do not mandate an arbitrary percentage speedup universally.

For production acceptance:

- Hybrid meshing must preserve numerical validity.
- Hybrid meshing must offer measurable benefits on at least its intended benchmark class.
- AMR must reduce relevant error as refinement proceeds in supported cases.
- Hybrid AMR must be competitive in total time-to-accuracy against relevant alternatives.
- Contact accuracy must not be traded away solely for DOF reduction.
- Unsupported configurations must fail explicitly or fall back to a verified supported strategy.
- No accepted feature may silently regress existing solver accuracy.
- Performance regressions must be measured, justified, and documented.

Maintain benchmark histories to detect future regressions.

---

# PART VII — CONFIGURATION AND USER-FACING INTEGRATION

## 38. Configuration Interfaces

The solver shall expose structured configuration for:

### Hybrid meshing

- Enable/disable hybrid meshing.
- Structured/unstructured selection mode.
- Automatic region classification.
- Preferred supported element families.
- Minimum/maximum element size.
- Target mesh quality.
- Transition restrictions.
- Geometry approximation tolerance.
- Optimization level.
- Mesh generation resource limits.

### AMR

- Enable/disable adaptivity.
- Goal functional(s).
- QoI tolerance.
- Physics tolerance.
- Estimator configuration.
- Fusion strategy.
- Adjoint priority.
- Spatial physics masks.
- Dörfler marking fraction.
- Maximum adaptation cycles.
- DOF/memory budget.
- Regional protection constraints.

### Diagnostics

Expose:

- Element-wise adjoint indicators.
- Element-wise physics indicators.
- Combined refinement scores.
- Structured/unstructured region labels.
- Transition element classifications.
- Mesh quality fields.
- Refinement markings.
- Convergence histories.
- Estimator reliability statuses.
- Time/memory breakdown.

Provide stable programmatic APIs so the GUI can be updated later without numerical logic being embedded in the frontend.

Do not make GUI redevelopment a prerequisite for implementing this EPIC.

---

# PART VIII — NONFUNCTIONAL REQUIREMENTS

## 39. Numerical Robustness

- No inverted or invalid accepted elements.
- No unintended disconnected mesh regions.
- No silent loss of loads/restraints.
- No unverified mixed-element interpolation.
- No silent failure of estimator calculations.
- No uncontrolled indefinite refinement.
- No unbounded memory growth.
- No invalid continuation after failed state transfer.
- No automatic physical-error suppression solely because adjoint impact is low.

## 40. Rust and Performance Engineering

Prefer:

- Efficient contiguous storage.
- Minimal numerical-data copying.
- Borrowed immutable views.
- Reusable work buffers.
- Sparse solver reuse.
- Parallelism where dependencies permit.
- Deterministic reductions when requested.
- Incremental mesh/cache updates.
- Optimized graph traversal.
- Efficient element adjacency storage.
- Feature-gated optional functionality.
- Clear, typed error reporting.

Avoid:

- Excessive heap allocation inside element loops.
- Repeated topology reconstruction without reason.
- Unnecessary repeated sparse factorization.
- Premature parallelization.
- New abstractions with no measurable architectural value.
- Numerical approximations introduced only for apparent speed.

## 41. Machine Learning — Optional Future Enhancement

Do not place machine learning in the critical correctness path.

Potential future research:

- Mesh region classification acceleration.
- Initial size-field prediction.
- Refinement-cost prediction.
- Mesh optimization parameter selection.
- Solver runtime prediction.

Any ML predictor must remain subordinate to deterministic, verified quality and accuracy checks.

No ML implementation is required to close this EPIC.

---

# PART IX — DELIVERABLES AND DEFINITION OF DONE

## 42. Deliverables

- [ ] Repository capability audit.
- [ ] Verified implementation gap analysis.
- [ ] Dependency-aware implementation plan.
- [ ] Hybrid mesh generator.
- [ ] Structured/unstructured transition support.
- [ ] Mesh quality optimization framework.
- [ ] Solver-aware mesh cost diagnostics.
- [ ] Goal-oriented adjoint estimator.
- [ ] SPR+ recovery integration.
- [ ] H(div)-conforming equilibrated stress estimator for supported cases.
- [ ] Multicriteria error fusion.
- [ ] Physics-protection rules.
- [ ] Hybrid-aware AMR controller.
- [ ] Solution/state transfer support.
- [ ] Convergence and termination controls.
- [ ] Numerical verification suite.
- [ ] Performance benchmark suite.
- [ ] Configuration API.
- [ ] Diagnostics API.
- [ ] Technical documentation.
- [ ] Known limitation/support matrix.
- [ ] Regression test report.
- [ ] Production readiness report.

## 43. Definition of Done

The EPIC is complete only when:

1. Both systems operate independently.
2. Both systems integrate successfully.
3. Supported hybrid meshes satisfy quality and compatibility requirements.
4. Structured–unstructured transitions are mathematically and numerically valid.
5. AMR reduces selected engineering error under verified convergence tests.
6. Local physical accuracy constraints remain enforced.
7. Nonlinear/contact functionality is implemented only for explicitly verified supported formulations.
8. Mesh adaptation correctly preserves relevant model state.
9. The implementation passes relevant numerical and software regression tests.
10. Performance improvements or tradeoffs are documented using reproducible benchmarks.
11. Unsupported or unreliable scenarios are explicitly reported.
12. All delivered behavior is documented and covered by appropriate tests.
13. No unapproved changes have been made directly to `main`.

Partial implementation must be reported as partial implementation; do not mark the full EPIC complete if required capabilities remain unsupported.

---

# PART X — EXECUTION DIRECTIVE FOR CLAUDE CODE

## 44. Begin Implementation

**Begin by reading the latest repository state and conducting Phase 0.**

Then:

1. Produce the verified capability matrix.
2. Identify any missing foundational dependencies.
3. Revise the proposed implementation order according to the actual code.
4. Decompose the EPIC into trackable, narrowly scoped implementation tasks.
5. Establish the dedicated development branch.
6. Record baseline tests and benchmark results.
7. Implement the smallest technically coherent foundation.
8. Add tests for the new capability.
9. Execute tests and applicable benchmarks.
10. Review correctness and performance.
11. Commit the verified changes.
12. Continue through dependent tasks in a controlled sequence.

Do not force implementation of advanced H(div), nonlinear adjoints, or complicated mixed-element transitions before the underlying supported elements and numerical foundations are verified.

Keep the meshing and AMR implementations independently testable.

Preserve existing solver capabilities.

### Continuous execution policy

Continue implementing logically independent, verified tasks when no blocker exists.

Do not stop after producing only a high-level plan.

For high-risk architectural changes, unsupported numerical formulations, incompatible assumptions, or substantial new dependencies, document the issue and request an explicit decision rather than silently making a potentially damaging choice.

### Mandatory validation after meaningful changes

Run appropriate subsets of:

- Formatting and static checks.
- Build/compilation checks.
- Unit tests.
- Integration tests.
- Numerical verification.
- Mesh validity tests.
- Regression tests.
- Performance benchmarks.

Before requesting merge, execute the complete relevant validation suite.

Do not automatically merge into `main`.

### Progress reporting

At logical implementation checkpoints, report:

- Completed tasks.
- Files/modules modified.
- Numerical methods implemented.
- Validation performed and actual results.
- Performance measurements.
- Current limitations.
- Outstanding dependencies.
- Next implementation tasks.

### Final engineering directive

**Optimize for verified engineering accuracy per unit of computational cost, not merely visually regular meshes or minimum element count.**

The hybrid mesher should create an efficient initial discretization.

The adjoint estimator should direct computational effort toward the selected engineering outputs.

The physics estimators should protect important local mechanical behavior.

The adaptation controller should reconcile those objectives while preserving conformity, solver stability, and practical resource limits.

The result should be a robust, modular, production-oriented FEA mesh generation and refinement infrastructure suitable for future expansion to advanced structural mechanics applications.