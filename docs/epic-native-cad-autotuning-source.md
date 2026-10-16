# EPIC ADDENDUM — Pillar 7: Intelligent Mesh Strategy Autotuning & Performance Learning

**Priority:** High  
**Component:** `fea-core::meshing::autotune`  
**Architecture:** Native Rust, deterministic-first, performance-driven  
**Dependencies:** Hybrid Meshing, Mesh Quality Evaluation, Solver Performance Telemetry  
**Optional Integrations:** AMR, Goal-Oriented Error Estimation, ML-Assisted Prediction

## 52. Objective

Implement an intelligent, self-improving **Mesh Strategy Autotuner** that automatically identifies the most computationally efficient meshing configuration for a given geometric model, physical analysis, accuracy requirement, and hardware environment.

The autotuner shall optimize **total FEA time-to-accuracy**, not merely mesh generation time, element count, or visual mesh quality.

It must learn from verified numerical benchmarks and previously completed analyses while maintaining deterministic numerical correctness and engineering accuracy as mandatory constraints.

The autotuner must operate independently of AMR. When AMR is enabled, its convergence and refinement histories shall inform future mesh strategy selection.

### Core capabilities

- Automatically classify the geometry and analysis workload.
- Select suitable element families and mesh topology.
- Predict relative computational cost for candidate strategies.
- Recommend structured, unstructured, or hybrid meshing.
- Recommend initial size fields and suitable element orders.
- Use historical performance data to improve future decisions.
- Account for solver, memory, hardware, and accuracy requirements.
- Adapt recommendations based on AMR convergence history.
- Preserve deterministic fallback behavior.
- Reject configurations that violate solver compatibility or engineering constraints.

**Important:** The autotuner selects promising numerical strategies. It does not replace verified meshing algorithms, error estimators, or solver convergence checks.

---

# 53. System Architecture

```text
CAD / GEOMETRY / ANALYSIS INPUT
                |
                v
GEOMETRY & PHYSICS FEATURE EXTRACTOR
  - Shape complexity
  - Curvature / feature size
  - Contact and material interfaces
  - Element formulation requirements
  - Requested QoIs / tolerances
  - Solver and hardware capabilities
                |
                v
MESH STRATEGY AUTOTUNER
                |
       +--------+---------+
       |                  |
       v                  v
RULE-BASED ENGINE     PERFORMANCE MODEL
       |                  |
       |            Historical benchmarks
       |            Measured solver costs
       |            Confidence estimates
       |                  |
       +--------+---------+
                |
                v
CANDIDATE STRATEGY RANKING
  - Structured / Unstructured / Hybrid
  - Supported element families
  - Polynomial order
  - Initial size field
  - Anisotropy settings
  - Mesh optimization level
                |
                v
CAPABILITY & ACCURACY CONSTRAINT FILTER
                |
                v
SELECTED MESH STRATEGY
                |
                v
NATIVE MESH GENERATOR
                |
                v
FEA SOLVER + OPTIONAL AMR
                |
                v
VERIFIED PERFORMANCE TELEMETRY
                |
       +--------+---------+
       |                  |
       v                  v
BENCHMARK STORE       MODEL UPDATE
       |                  |
       +--------+---------+
                |
                v
FUTURE STRATEGY IMPROVEMENT
```

The autotuner shall not block normal meshing execution if the prediction system is unavailable.

---

# 54. Geometry and Analysis Feature Extraction

Implement a lightweight feature extractor that generates a compact representation of the workload.

### Geometric features

- CAD solid count.
- Surface and edge counts.
- Geometry bounding dimensions.
- Surface-to-volume ratio.
- Minimum characteristic feature size.
- Curvature distribution.
- Small-feature density.
- Hole/cavity count.
- Thin-wall indicators.
- Topological complexity.
- Structured-meshing feasibility.
- Sweepability.
- Material interface area.
- Contact surface area.

### Physics features

- Analysis type.
- Material behavior.
- Contact formulation.
- Nonlinearity level.
- Expected deformation regime.
- Requested QoIs.
- Required engineering tolerances.
- Element formulation requirements.
- Relevant boundary and load characteristics.

### Numerical features

- Available element families.
- Polynomial interpolation orders.
- Supported meshing transitions.
- Solver backend.
- Sparse direct or iterative method.
- Preconditioner.
- AMR configuration.
- Estimated DOF budget.
- Memory restrictions.

### Hardware features

- CPU architecture.
- Physical/logical core count.
- Available memory budget.
- Parallel execution configuration.
- Relevant solver acceleration capabilities.

Hardware features must be collected through supported, privacy-conscious local APIs.

Do not assume a benchmark recorded on one hardware configuration is directly transferable to another.

---

# 55. Candidate Strategy Generation

The autotuner shall generate only strategies that are compatible with the existing solver and available meshing infrastructure.

Example candidates:

| Strategy | Candidate Configuration |
|---|---|
| Structured | Hex8, mapped/swept, geometry-aligned |
| High-order structured | Hex20/Hex27 where supported |
| Unstructured linear | Tet4 |
| Unstructured quadratic | Tet10 |
| Hybrid | Hex/wedge/pyramid/tet, supported transitions |
| Anisotropic | Metric-driven refinement with bounded anisotropy |
| Contact optimized | Interface-conforming mesh with localized refinement |
| Goal-oriented | Initial mesh informed by verified historical QoI information |

Do not assume Hex8 is always faster than Tet10, or vice versa.

Element formulation, integration scheme, conditioning, geometry, solver, and desired accuracy must determine the preferred configuration.

### Candidate filtering

Reject strategies that:

- Require unavailable element formulations.
- Require unsupported transition elements.
- Cannot satisfy geometry requirements.
- Violate imposed resource budgets.
- Cannot preserve required material/contact interfaces.
- Depend on unsupported CAD functionality.
- Have known numerical incompatibilities.
- Fail configured engineering validity constraints.

---

# 56. Solver-Aware Cost Prediction

Implement a cost estimation interface.

For each candidate strategy \(s\), predict:

\[
\widehat C(s)=
\widehat T_{\mathrm{mesh}}
+\widehat T_{\mathrm{assembly}}
+\widehat T_{\mathrm{solve}}
+\widehat T_{\mathrm{estimate}}
+\widehat T_{\mathrm{adapt}}
+\widehat T_{\mathrm{transfer}}
\]

Include relevant repeated solve and optimization costs where applicable.

The autotuner must separately predict or estimate:

- Mesh generation time.
- Expected element count.
- Expected DOF count.
- Sparse matrix complexity.
- Assembly cost.
- Linear/nonlinear solve cost.
- Peak memory.
- Expected AMR cycles, when supported.
- Numerical convergence risk.
- Geometry/mesh failure risk.

Predictions must be treated as uncertain estimates.

No prediction may be represented as a verified result before the analysis is performed.

### Accuracy-constrained selection

Select:

\[
s^*=
\underset{s\in\mathcal S_{\mathrm{valid}}}{\operatorname{argmin}}
\widehat C(s)
\]

subject to relevant accuracy and resource constraints.

Where accuracy cannot be predicted reliably, use a conservative strategy and allow verified error estimation to determine the required refinement.

---

# 57. Deterministic Rule-Based Foundation

The first implementation shall use a deterministic heuristic policy.

Examples:

- Prefer swept/structured elements for suitable sweepable volumes.
- Prefer verified tetrahedral meshing for irregular solid regions.
- Evaluate hybrid strategies for mixed-complexity geometries.
- Favor contact-conforming surfaces where required.
- Prefer higher-order elements when curved geometry and smooth solution behavior justify them.
- Avoid high-order elements when unsupported contact/material behavior or invalid mappings make them unsuitable.
- Bound initial mesh density according to feature size, tolerances, and resource constraints.

All heuristic selections must include a reason code.

The autotuner must support:

- Automatic mode.
- Deterministic-only mode.
- Fixed user-selected strategy.
- Benchmark/experimental mode.

Explicit user choices take precedence unless they violate hard validity constraints.

---

# 58. Performance Learning Engine

After collecting sufficient verified benchmark data, implement an optional performance-based learning model.

Begin with low-complexity, interpretable approaches before evaluating more elaborate machine learning.

Potential techniques:

1. Nearest-neighbor performance retrieval.
2. Workload-class performance tables.
3. Regularized regression.
4. Gradient-boosted performance predictors.
5. Contextual bandit selection for controlled experimentation.

Model selection must be driven by measured predictive quality, runtime cost, maintenance overhead, and data availability.

Avoid introducing a heavyweight ML runtime as a mandatory FEA-Core dependency.

### Learning objectives

Predict:

- Relative speed of supported mesh strategies.
- DOF requirements for specified accuracy targets.
- Expected remeshing and AMR costs.
- Memory usage.
- Probability of meshing failure.
- Probability of solver convergence difficulty.

### Confidence-aware decisions

Each recommendation should carry:

- Selected strategy.
- Expected cost.
- Prediction confidence or uncertainty.
- Applicable benchmark support.
- Compatibility checks.
- Known risk indicators.
- Fallback strategy.

Use the deterministic baseline when training data are insufficient, out-of-distribution, incompatible with the current solver version, or otherwise unreliable.

---

# 59. Benchmark and Telemetry Storage

Implement a structured local benchmark repository.

Store:

- Workload fingerprint.
- Geometry feature summary.
- Analysis configuration.
- Mesh strategy.
- Element formulation.
- Size-field settings.
- Optimization settings.
- Solver configuration.
- Hardware fingerprint.
- Application and schema version.
- Mesher and solver version identifiers.
- Verification status.
- Accuracy/error metrics.
- Timing breakdown.
- Peak memory.
- Failure classification.
- AMR convergence history.
- Reproducibility metadata.

Never use unverified or failed analyses as successful performance training examples.

Failures may be retained separately to train compatibility or failure-risk predictors.

### Storage requirements

- Versioned schema.
- Local-first persistence.
- Bounded storage.
- Duplicate detection.
- Corrupt-entry handling.
- Safe migration between schema versions.
- Export/import support.
- User-controlled deletion and reset.
- Optional telemetry collection controls.

Avoid storing full proprietary CAD geometry or sensitive model contents in performance records by default.

---

# 60. Integration with Adaptive Mesh Refinement

AMR shall remain the authority for evaluating solution resolution.

The autotuner may use validated AMR feedback to improve future initial meshes.

Relevant measurements:

- Initial QoI error estimate.
- Final verified QoI error.
- Error reduction per cycle.
- DOF growth.
- Regional refinement distributions.
- Contact-zone refinement frequency.
- Anisotropy effectiveness.
- Total AMR cost.
- Convergence stagnation.
- Budget-limited termination.

### Adaptive feedback example

If a class of bolted joints repeatedly requires substantial refinement around bearing interfaces, future candidate meshes may allocate more initial resolution to those features.

However, the autotuner must not infer that the same mesh is accurate for every superficially similar joint.

Material behavior, geometry, boundary conditions, contact conditions, and requested QoIs must remain part of strategy selection.

### Important boundary

The autotuner recommends an initial mesh and optional adaptation parameters.

The AMR controller independently determines where refinement is required.

The autotuner must never suppress a validated physics-based refinement request to satisfy a predicted performance target.

---

# 61. Controlled Exploration and Continuous Improvement

Allow optional experimental comparison between candidate strategies.

### Default production mode

Select one compatible strategy using the best supported prediction or deterministic heuristic.

Do not perform multiple full FEA solves simply to explore alternatives.

### Experimental benchmark mode

When explicitly enabled:

- Generate multiple candidate meshes.
- Execute compatible verification problems.
- Measure performance and accuracy.
- Compare results under equivalent tolerances.
- Record successful and unsuccessful candidates.
- Update the performance dataset.

### Exploration policy

Use resource budgets covering:

- Maximum candidate count.
- Maximum compute time.
- Memory usage.
- Maximum total DOFs.
- Maximum benchmark solves.

Exploration must not silently consume significant computation.

---

# 62. Proposed Rust Module Structure

```text
fea-core/
  src/
    meshing/
      autotune/
        mod.rs
        context.rs
        features.rs
        fingerprint.rs
        candidates.rs
        compatibility.rs
        heuristics.rs
        cost_model.rs
        prediction.rs
        uncertainty.rs
        ranking.rs
        telemetry.rs
        benchmark_store.rs
        learning.rs
        diagnostics.rs
```

Adapt names and boundaries to the actual repository.

### Suggested interfaces

```rust
pub enum AutotuneMode {
    Disabled,
    Deterministic,
    Learned,
    Experimental,
}

pub struct MeshStrategyCandidate {
    pub configuration: MeshingOptions,
    pub predicted_cost: Option<CostPrediction>,
    pub compatibility: CompatibilityStatus,
}

pub struct CostPrediction {
    pub estimated_seconds: f64,
    pub estimated_peak_bytes: usize,
    pub uncertainty: f64,
}

pub struct AutotuneRecommendation {
    pub selected: MeshStrategyCandidate,
    pub alternatives: Vec<MeshStrategyCandidate>,
    pub rationale: Vec<SelectionReason>,
}

pub trait MeshStrategySelector {
    fn select(
        &self,
        context: &MeshingWorkloadContext,
        candidates: &[MeshStrategyCandidate],
    ) -> Result<AutotuneRecommendation, AutotuneError>;
}

pub trait MeshPerformanceRecorder {
    fn record(
        &mut self,
        context: &MeshingWorkloadContext,
        result: &VerifiedAnalysisResult,
    ) -> Result<(), TelemetryError>;
}
```

The API must prevent a result from being classified as verified based only on a caller-provided boolean.

Verification status should originate from the numerical validation pipeline.

---

# 63. Implementation Roadmap

Add the following phases to the main EPIC.

## Phase A — Telemetry and Workload Classification

**Prerequisites:** Stable mesh configuration APIs and benchmark infrastructure.

- [ ] Define workload feature schema.
- [ ] Implement geometry feature extraction.
- [ ] Implement solver/hardware capability descriptors.
- [ ] Add verified performance telemetry.
- [ ] Add versioned benchmark persistence.
- [ ] Establish initial benchmark corpus.

**Exit gate:** Repeatable workload descriptions and trustworthy performance measurements.

## Phase B — Deterministic Mesh Strategy Selection

**Prerequisites:** Phase A and supported meshing strategies.

- [ ] Implement compatibility filtering.
- [ ] Implement deterministic heuristic selector.
- [ ] Add candidate strategy generation.
- [ ] Implement resource budget checks.
- [ ] Add selection rationale.
- [ ] Add fixed-strategy override.
- [ ] Integrate with native mesh generator.

**Exit gate:** The autotuner selects valid strategies without requiring ML or AMR.

## Phase C — Cost Modeling

**Prerequisites:** Representative verified benchmark history.

- [ ] Implement baseline cost predictors.
- [ ] Predict element/DOF counts.
- [ ] Estimate assembly/solve cost.
- [ ] Estimate memory usage.
- [ ] Include expected AMR overhead when supported.
- [ ] Calibrate against measured data.
- [ ] Add uncertainty handling.

**Exit gate:** Cost estimates are benchmarked and clearly distinguished from verified measurements.

## Phase D — Performance Learning

**Prerequisites:** Sufficient diverse, verified benchmark data.

- [ ] Implement benchmark similarity matching.
- [ ] Evaluate simple interpretable models.
- [ ] Add model versioning.
- [ ] Add out-of-distribution detection.
- [ ] Evaluate predictive error.
- [ ] Compare against deterministic baseline.
- [ ] Enable confidence-aware selection.
- [ ] Preserve safe fallback behavior.

**Exit gate:** Learned selection improves decision quality on held-out workloads without increasing unacceptable numerical or operational risk.

## Phase E — AMR Feedback Integration

**Prerequisites:** Verified AMR convergence telemetry.

- [ ] Consume AMR history.
- [ ] Summarize regional refinement demand.
- [ ] Associate refinement with workload features.
- [ ] Improve initial size-field recommendations.
- [ ] Estimate likely adaptation cost.
- [ ] Validate recommended meshes through normal AMR checks.

**Exit gate:** Improved initial mesh recommendations reduce time-to-accuracy on representative benchmark classes.

## Phase F — Production Optimization

- [ ] Profile autotuner overhead.
- [ ] Bound prediction time and memory.
- [ ] Verify deterministic fallback.
- [ ] Verify benchmark data integrity.
- [ ] Test schema migrations.
- [ ] Add regression detection.
- [ ] Validate software version compatibility.
- [ ] Complete end-to-end comparisons.
- [ ] Document supported recommendations.

**Exit gate:** Autotuning provides measurable benefits without compromising solver reliability.

---

# 64. Verification and Acceptance Criteria

Test the autotuner against independent workload groups.

### Required geometry classes

- Simple prismatic block.
- Swept beam or bracket.
- Plate with holes.
- Curved thin-wall structure.
- Filleted mechanical bracket.
- Multi-material assembly.
- Bolted joint.
- Frictional contact assembly.
- Irregular 3D solid.
- Large deformation case.

### Required comparisons

1. Fixed structured meshing.
2. Fixed unstructured meshing.
3. Fixed hybrid meshing.
4. Deterministic autotuner.
5. Learned autotuner when available.
6. Autotuner with AMR feedback.

### Primary metrics

- Total time-to-target-accuracy.
- Mesh generation time.
- Solver execution time.
- Peak memory.
- Number of DOFs.
- Number of AMR cycles.
- Verified QoI accuracy.
- Failure rate.
- Prediction overhead.
- Prediction calibration error.
- Strategy-selection regret relative to evaluated alternatives.

### Evaluation safeguards

- Use separate training and held-out test geometry families.
- Prevent near-duplicate benchmark leakage.
- Compare on equivalent engineering tolerances.
- Record solver/hardware versions.
- Include cold-start cases.
- Include out-of-distribution cases.
- Include insufficient-data cases.
- Test fallback behavior.
- Test user overrides.
- Test corrupt benchmark records.

The learned selector must be evaluated against the deterministic baseline, not merely against random strategy selection.

### Performance target

A reasonable initial research target is a **10–20% reduction in median total time-to-accuracy** on a representative held-out workload set compared with the deterministic strategy baseline.

This is an aspirational target, not a universal speedup guarantee or a reason to compromise accuracy.

Production acceptance requires validated improvement on its intended workload classes, bounded prediction overhead, and no unacceptable accuracy regressions.

---

# 65. Failure Handling

The autotuner must fail safely.

Required fallback conditions:

- Missing benchmark data.
- Corrupt model data.
- Unknown geometry class.
- Unavailable element formulation.
- Unsupported CAD entity.
- Solver backend mismatch.
- Hardware configuration mismatch.
- Unreliable prediction.
- Excessive resource requirement.
- Invalid generated mesh.
- Unexpected mesher failure.

Fallback sequence:

1. Reject invalid or unsupported strategies.
2. Re-evaluate remaining compatible strategies.
3. Use a deterministic verified strategy.
4. Return a structured failure if no valid strategy exists.

Never silently substitute a strategy that changes the physical modeling assumptions.

---

# 66. Updated EPIC Deliverables

Add these items to the main EPIC deliverable checklist:

- [ ] Mesh Strategy Autotuner.
- [ ] Geometry and workload feature extractor.
- [ ] Meshing strategy capability registry.
- [ ] Deterministic strategy-selection engine.
- [ ] Solver-aware cost predictor.
- [ ] Versioned benchmark/performance database.
- [ ] Verified telemetry ingestion.
- [ ] Optional lightweight performance-learning model.
- [ ] Prediction confidence and fallback system.
- [ ] AMR-informed initial mesh recommendations.
- [ ] Autotuner configuration API.
- [ ] Strategy-selection diagnostics.
- [ ] Independent autotuning benchmark suite.
- [ ] Full accuracy and regression validation.

---

# 67. Updated Definition of Done

In addition to the existing EPIC completion criteria:

1. The autotuner operates independently of AMR.
2. All selected strategies are solver-compatible.
3. Invalid mesh configurations are rejected.
4. The deterministic selector works without training data.
5. Learned predictions are optional and replaceable.
6. Numerical correctness remains controlled by existing solver and error-verification infrastructure.
7. Prediction models operate within bounded resource budgets.
8. Performance improvements are evaluated on held-out benchmarks.
9. Selection decisions are reproducible and explainable.
10. User-selected strategies are respected unless invalid.
11. Benchmark records are versioned and auditable.
12. AMR feedback improves recommendations only when supported by verified measurements.
13. The autotuner does not add mandatory heavyweight ML dependencies.
14. Total time-to-accuracy improvements are demonstrated on representative workloads.

---

# 68. Additional Claude Code Execution Instructions

Treat the Mesh Strategy Autotuner as **Pillar 7 of the Native Rust CAD & Advanced Meshing EPIC**.

During the Phase 0 repository audit:

- Identify existing performance telemetry and benchmarking.
- Determine whether mesh configuration parameters are sufficiently structured for candidate generation.
- Identify existing geometry classifiers and feature extractors.
- Review mesh quality and solver statistics.
- Inspect AMR convergence history availability.
- Determine where a stable strategy-selection interface can be inserted.

Do not implement machine learning immediately.

Implement the deterministic selection engine first and collect verified data.

Once the hybrid mesher, 3D tetrahedralizer, and relevant solver components are stable, introduce measured cost prediction and optional learning.

Do not delay essential CAD replacement, meshing correctness, or AMR implementation to build the learning subsystem.

### Final engineering directive

**The autotuner should make FEA-Core progressively better at selecting an initial mesh and numerical strategy, while deterministic verification remains responsible for proving whether the resulting solution is sufficiently accurate.**

The system must learn from measured performance, avoid unsupported assumptions, remain lightweight, and integrate naturally into the native Rust meshing architecture.