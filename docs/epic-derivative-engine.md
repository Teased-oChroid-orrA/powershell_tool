# EPIC: Unified Adaptive Derivative and Sensitivity Engine

**Repository:** `powershell_tool`  
**Component:** `fea-core`  
**Priority:** High  
**Category:** Core Numerical Solver Architecture  
**Language:** Rust  
**Development:** Dedicated feature branch  
**Suggested Branch:** `feature/adaptive-derivative-engine`  
**Status:** Proposed — Audit Required

---

# 1. Executive Summary

Implement a unified, adaptive, high-performance differentiation framework for FEA-Core, supporting linear, nonlinear, material, geometric, contact, transient, and multiphysics analyses.

The architecture shall intelligently select and combine:

1. Analytic and hand-derived derivatives.
2. Forward-mode Automatic Differentiation.
3. Reverse-mode Automatic Differentiation.
4. Analytic and discrete adjoint methods.
5. Matrix-free Jacobian-vector and vector-Jacobian products.
6. Complex-step differentiation.
7. Hyper-dual differentiation.
8. Generalized derivatives and semismooth Newton methods.
9. Implicit differentiation of locally and globally converged systems.
10. Numerical differentiation for verification and carefully controlled fallback.

The objective is to maximize numerical accuracy and computational efficiency while preserving solver stability and minimizing unnecessary memory consumption.

**Fundamental principle:**

Derivative construction, sensitivity propagation, and nonlinear solution methods represent different numerical responsibilities.

They must be independently designed but interoperable.

The architecture must not assume that one derivative method is universally best for a specific physics class.

---

# 2. Primary Engineering Objectives

- Obtain accurate, consistent tangent operators for nonlinear constitutive and contact mechanics.
- Minimize derivative evaluation overhead.
- Eliminate unnecessary global AD tape recording.
- Improve nonlinear Newton convergence through mathematically consistent tangents.
- Support scalable parameter and design sensitivity analysis.
- Enable efficient adjoint-based optimization.
- Support directional derivatives and Hessian-vector products.
- Handle nonsmooth contact and material transitions correctly.
- Support matrix-free and assembled sparse solver architectures.
- Reuse numerical factorizations and derivative computations whenever valid.
- Remain compatible with the existing FEA-Core solver architecture.
- Integrate with the independently developed AMR and hybrid meshing systems.
- Preserve deterministic and auditable numerical behavior.

Do not sacrifice correctness for a target speedup.

---

# PART I — MATHEMATICAL FOUNDATION

# 3. Separate the Three Levels of Differentiation

The proposed framework must distinguish between three operations.

## 3.1 Local Derivative Construction

Computes derivatives of element, material, interface, or contact operations.

Examples:

\[
C_{\mathrm{alg}}=\frac{\partial \sigma}{\partial\varepsilon}
\]

\[
K_e=\frac{\partial r_e}{\partial u_e}
\]

Typical methods:

- Analytic differentiation.
- Forward AD.
- Reverse AD.
- Implicit differentiation.
- Validated complex-step or hyper-dual differentiation.

## 3.2 Global Sensitivity Propagation

Determines the response of the converged finite-element solution to model parameters.

Given:

\[
R(u,p)=0
\]

the direct sensitivity satisfies:

\[
R_u\frac{du}{dp}=-R_p
\]

Typical methods:

- Direct tangent sensitivity.
- Discrete adjoint.
- Matrix-free linear sensitivity solves.

## 3.3 Nonlinear and Nonsmooth Solution

Determines how the solver handles nonlinear equilibrium and contact transitions.

Typical methods:

- Newton-Raphson.
- Modified Newton.
- Semismooth Newton.
- Primal-dual active set.
- Augmented Lagrangian.
- Line search and trust region.
- Regularized contact laws.

**These levels must be independently configurable.**

For example, a contact analysis may use forward AD for a smooth material law, an analytic contact tangent on a fixed active set, semismooth Newton for contact transitions, and a global adjoint for an optimization objective.

---

# 4. Core Mathematical Formulation

For global equilibrium:

\[
R(u,p)=0
\]

define the consistent tangent:

\[
K_T=\frac{\partial R}{\partial u}
\]

For a scalar output:

\[
J(u,p)
\]

the direct total sensitivity is:

\[
\frac{dJ}{dp}
=
J_p+J_u\frac{du}{dp}
\]

Using:

\[
K_T\frac{du}{dp}=-R_p
\]

gives:

\[
\frac{dJ}{dp}=J_p-J_uK_T^{-1}R_p
\]

Define the adjoint:

\[
K_T^T\lambda=J_u^T
\]

Then:

\[
\boxed{
\frac{dJ}{dp}=J_p-\lambda^TR_p
}
\]

This formulation allows derivatives constructed analytically or by AD at the local level to feed into a global adjoint calculation.

The number of required linear sensitivity solves generally scales with the number of independent parameters for direct differentiation, or with the number of scalar outputs for adjoints.

Adjoints can be particularly advantageous when the number of design parameters greatly exceeds the number of objectives.

However, adjoints do not make the entire optimization computation constant-time. Element derivative evaluation, matrix assembly, global solves, and gradient accumulation still incur costs.

---

# PART II — ARCHITECTURE

# 5. Unified Differentiation Architecture

```text
SOLVER EXECUTIVE / SENSITIVITY REQUEST
                    |
                    v
DERIVATIVE CONTEXT ANALYZER
  - Physics and material model
  - Differentiability/smoothness
  - Requested derivative order
  - Input/output dimensions
  - Contact/active-set state
  - Sparse matrix capabilities
  - Memory and performance budget
                    |
                    v
DERIVATIVE POLICY ENGINE
                    |
        +-----------+-----------+
        |           |           |
        v           v           v
  ANALYTIC       AUTOMATIC   SPECIALIZED
  METHODS        DIFF        METHODS
        |           |           |
  Hand tangent   Forward AD   Semismooth
  Closed-form    Reverse AD   Active-set
  Direct solve   Mixed AD     Complex-step*
  Adjoint        JVP / VJP    Hyper-dual*
        |           |           |
        +-----------+-----------+
                    |
                    v
LOCAL DERIVATIVE OPERATORS
  - Material constitutive Jacobians
  - Element residual Jacobians
  - Contact/interface tangents
  - Parameter derivatives
                    |
                    v
GLOBAL SENSITIVITY ENGINE
  - Sparse assembled operations
  - Matrix-free JVP/VJP
  - Direct sensitivities
  - Discrete adjoints
  - Implicit differentiation
                    |
                    v
VALIDATION / RELIABILITY LAYER
  - Derivative consistency
  - Smoothness verification
  - Finite-difference checks
  - Active-set validity
  - Numerical diagnostics
                    |
                    v
SOLVER / OPTIMIZER / AMR CONSUMER
```

*Complex-step and hyper-dual kernels are permitted only for mathematically compatible operations.

They must not be advertised as exact derivatives through discontinuous or nonanalytic transitions.

---

# 6. Adaptive Derivative Policy Engine

Implement a central `DerivativePolicyEngine` or equivalent abstraction integrated with existing solver infrastructure.

The policy engine must select differentiation strategies according to actual mathematical and computational requirements.

## Required context information

- Physics formulation.
- Linear/nonlinear analysis state.
- Requested derivative order.
- Number of differentiation inputs.
- Number of outputs.
- Differentiability classification.
- Material update algorithm.
- Current contact active-set status.
- Available analytic derivatives.
- AD kernel compatibility.
- Sparse/matrix-free solver support.
- Estimated memory cost.
- Estimated computation cost.
- Existing reusable factorizations.
- Required numerical precision.
- Determinism requirements.

## Initial method-selection policy

| Context | Preferred strategy |
|---|---|
| Constant linear-elastic material | Analytic tangent |
| Small smooth constitutive model | Forward AD or analytic |
| Complex smooth constitutive model | Forward/reverse AD selected by cost |
| Large number of parameters, few outputs | Global adjoint |
| Few parameters, many outputs | Direct/forward sensitivity |
| Matrix-free Newton-Krylov | JVP |
| Reverse sensitivity propagation | VJP |
| Smooth second derivatives | Hyper-dual or higher-order AD |
| Smooth holomorphic verification kernel | Complex-step |
| Fixed contact active set | Consistent analytic/AD tangent of the active branch |
| Contact active-set transition | Generalized derivative / semismooth Newton |
| Plastic return mapping | Consistent algorithmic tangent, analytic/AD/implicit |
| Damage evolution | Constitutive-consistent derivatives with event handling |
| Unsupported derivative | Explicit error or controlled validated fallback |

Do not introduce method switching merely because an iteration number changes.

Prefer stable method selection at material/element kernel or solve-phase boundaries. Switching within an active Newton iteration is permitted only when compatible with solver convergence requirements.

---

# PART III — IMPLEMENTATION STORIES

# 7. Story A — Derivative Capability Registry

**Goal:** Establish a single mechanism through which existing and future numerical kernels expose derivative capabilities.

## Requirements

- [ ] Create a derivative method registry.
- [ ] Register analytic, AD, adjoint, and specialized kernel capabilities.
- [ ] Define differentiability categories.
- [ ] Define supported derivative orders.
- [ ] Define supported scalar representations.
- [ ] Define device/thread safety capabilities.
- [ ] Support explicit strategy overrides.
- [ ] Support cost-aware automatic selection.
- [ ] Reject mathematically incompatible selections.
- [ ] Expose diagnostics identifying the selected method and reasons.

### Acceptance criteria

The router selects a verified method for each supported differentiation request.

Unsupported combinations must be reported explicitly rather than silently approximated.

---

# 8. Story B — Forward-Mode Automatic Differentiation

**Goal:** Implement low-overhead forward AD for smooth local derivative calculations.

Forward AD must be evaluated as a first-class capability, not merely a fallback behind reverse AD.

## Requirements

- [ ] Support dual-number arithmetic.
- [ ] Support common scalar operations used in constitutive laws.
- [ ] Support fixed-size derivative storage for small dimensions.
- [ ] Evaluate chunked forward AD where input dimensions are larger.
- [ ] Support element and integration-point calculations.
- [ ] Support directional derivatives.
- [ ] Support Jacobian-vector products.
- [ ] Avoid heap allocation in common fixed-size kernels.
- [ ] Evaluate static versus dynamic derivative dimensions.
- [ ] Preserve efficient Rust numerical data layouts.

### Acceptance criteria

Correct derivatives against analytic references and meaningful performance comparisons against finite differences, reverse AD, and handwritten tangents.

---

# 9. Story C — Reverse-Mode Automatic Differentiation

**Goal:** Enable efficient gradient computation for complex smooth local computations with favorable input/output dimensions.

## Requirements

- [ ] Audit Rust-native and interoperable AD implementations.
- [ ] Evaluate Enzyme-backed Rust integration where technically suitable.
- [ ] Compare reverse-mode AD with forward-mode AD.
- [ ] Support local computation graph recording where required.
- [ ] Avoid global solver-wide tape recording by default.
- [ ] Reuse local tape structures when safe.
- [ ] Support bounded workspace memory.
- [ ] Support VJP operations.
- [ ] Handle branching consistently with the differentiated execution path.
- [ ] Support reproducible parallel execution.
- [ ] Verify gradients against analytic and directional references.

### Important design constraint

Do not force every constitutive operation through reverse AD.

Many material models have small strain input dimensions and benefit from forward AD or an analytic consistent tangent.

A general reverse AD dependency must demonstrate sufficient numerical and performance benefits before adoption.

---

# 10. Story D — Nonlinear Constitutive Differentiation

**Goal:** Compute consistent algorithmic tangents for supported material models.

Initial targets should be selected based on models actually present in FEA-Core.

Candidate formulations include:

- Linear elasticity.
- Hyperelasticity.
- J2 elastoplasticity.
- Isotropic/kinematic hardening.
- Viscoelasticity.
- Damage and degradation.
- Thermal expansion/coupled thermomechanics.

## Requirements

- [ ] Define differentiable material evaluation interfaces.
- [ ] Separate trial and committed constitutive states.
- [ ] Support constitutive residual evaluation.
- [ ] Support local return mapping.
- [ ] Support analytic or AD-assisted local Jacobians.
- [ ] Apply implicit differentiation to converged local constitutive systems when appropriate.
- [ ] Compute consistent algorithmic tangents.
- [ ] Preserve material state history.
- [ ] Detect unsupported nonsmooth model transitions.
- [ ] Avoid differentiating unconverged local iterations as if they represented the exact constitutive mapping.
- [ ] Integrate with global tangent assembly.

For a local implicit constitutive state \(q\):

\[
F(q,\varepsilon,p)=0
\]

the derivative satisfies:

\[
F_q\frac{dq}{d\varepsilon}=-F_\varepsilon
\]

Thus:

\[
\frac{dq}{d\varepsilon}
=
-F_q^{-1}F_\varepsilon
\]

The algorithmic tangent must account for this dependence.

### Acceptance criteria

- Consistent local tangents.
- Correct stress updates.
- Stable material state evolution.
- Numerical agreement with independent reference implementations.
- Verified Newton convergence on supported smooth branches.
- Correct handling of elastic-plastic transitions.

---

# 11. Story E — Global Adjoint and Direct Sensitivity Engine

**Goal:** Provide scalable sensitivities of converged FEA solutions.

## Requirements

- [ ] Reuse compatible primal tangent matrices.
- [ ] Support transpose sparse solves.
- [ ] Support direct tangent sensitivity equations.
- [ ] Support discrete adjoints.
- [ ] Support parameter residual derivatives.
- [ ] Support multiple QoIs.
- [ ] Support multiple design parameters.
- [ ] Reuse factorization/preconditioner structures where valid.
- [ ] Support matrix-free operators where available.
- [ ] Handle nonsymmetric tangent matrices.
- [ ] Support geometric and material design variables.
- [ ] Preserve solver constraints and boundary-condition treatment.
- [ ] Verify sensitivities against independent perturbation tests.

### Optimization strategy

Select direct or adjoint sensitivities based on parameter/output dimensions and measured cost.

The selection engine must account for factorization reuse, repeated right-hand sides, matrix structure, and available parallelism.

### Acceptance criteria

Converged sensitivity results satisfy independent reference checks, and performance improvements are demonstrated on representative parameter-count scenarios.

---

# 12. Story F — Matrix-Free Derivative Operators

**Goal:** Enable derivative actions without requiring explicit construction of full Jacobian matrices when appropriate.

Support:

\[
Jv
\]

and:

\[
J^Tv
\]

## Requirements

- [ ] Jacobian-vector product interface.
- [ ] Vector-Jacobian product interface.
- [ ] Forward AD JVP implementations.
- [ ] Reverse AD or analytic VJP implementations.
- [ ] Matrix-free operator compatibility.
- [ ] Krylov solver integration where supported.
- [ ] Preconditioner compatibility.
- [ ] Sparse assembled fallback.
- [ ] Product consistency tests.
- [ ] Performance comparison against assembled operators.

Matrix-free execution must not automatically replace assembled sparse operations.

Select the strategy according to solver compatibility and demonstrated cost.

---

# 13. Story G — Nonsmooth Contact Derivative Engine

**Goal:** Support robust derivatives and nonlinear solution behavior across contact-state changes.

**This story supersedes the original proposal to use hyper-dual/complex-step differentiation directly through contact discontinuities.**

Contact mechanics may involve:

- Opening and closing.
- Stick-slip transitions.
- Friction-cone boundaries.
- Complementarity constraints.
- Contact activation/deactivation.
- Nonsmooth projections.
- Changing active constraints.

A classical derivative may fail to exist at these transitions.

Complex-step methods require appropriate holomorphic dependence and therefore cannot generally provide exact derivatives through these nonsmooth operations.

Hyper-dual numbers also do not remove the underlying nondifferentiability.

## Required strategies

### Fixed active-set differentiation

When contact status remains locally unchanged:

- Use consistent analytic derivatives.
- Permit compatible forward or reverse AD.
- Compute contact tangent contributions.
- Preserve normal/tangential coupling.
- Verify contact Jacobian blocks.

### Active-set transitions

Implement or reuse supported:

- Primal-dual active-set methods.
- Semismooth Newton methods.
- Generalized Jacobians.
- Complementarity-based formulations.
- Appropriate line-search/globalization safeguards.

### Regularized contact

When a smooth contact regularization is explicitly selected:

- Allow compatible AD.
- Track regularization parameters.
- Verify approximation accuracy.
- Report the distinction between regularized and original contact laws.

## Requirements

- [ ] Track open/closed/stick/slip states.
- [ ] Support branch-consistent contact tangents.
- [ ] Detect contact regime transitions.
- [ ] Integrate generalized derivative operators where mathematically appropriate.
- [ ] Preserve solver convergence controls.
- [ ] Support supported penalty and augmented-Lagrangian formulations.
- [ ] Support Lagrange multiplier formulations when present.
- [ ] Validate frictional/contact residuals.
- [ ] Avoid unsafe mid-iteration scalar-type changes.
- [ ] Support contact event diagnostics.
- [ ] Handle contact-state chattering/stagnation.
- [ ] Verify energy, force, and complementarity behavior.

### Acceptance criteria

Demonstrate reliable contact convergence and verified sensitivities on smooth active branches.

At nonsmooth transition points, report the type of derivative or directional sensitivity being returned.

Do not claim a unique exact derivative exists where the mathematical model does not support one.

---

# 14. Story H — Complex-Step and Hyper-Dual Verification Kernels

**Goal:** Provide high-accuracy differentiation and verification for compatible smooth operations.

## Complex-step

For a holomorphic scalar function implemented with complex-compatible operations:

\[
f'(x)\approx
\frac{\operatorname{Im}(f(x+ih))}{h}
\]

Complex-step avoids the subtractive cancellation inherent in ordinary finite differences.

It does not automatically apply to:

- Absolute-value operations.
- Min/max branches.
- Contact complementarity.
- Discontinuous state transitions.
- Nonanalytic complex operations.
- General iterative algorithms with branch changes.

## Hyper-dual

Hyper-dual arithmetic can provide first- and mixed-second-order derivative information for appropriately smooth operations.

## Requirements

- [ ] Complex-compatible verification kernels.
- [ ] Hyper-dual scalar support where justified.
- [ ] Smoothness/compatibility checks.
- [ ] First-derivative testing.
- [ ] Mixed-second-derivative testing.
- [ ] Comparisons against analytic references.
- [ ] Clear rejection of incompatible kernels.
- [ ] Configurable numerical test tolerances.

Do not claim machine-precision agreement universally.

Acceptance tolerances must account for arithmetic conditioning, solver tolerances, iterative convergence, and reference accuracy.

---

# 15. Story I — Higher-Order Sensitivities

**Goal:** Establish an extensible interface for second derivatives and curvature information.

Potential applications:

- Second-order optimization.
- Stability sensitivity.
- Parameter identification.
- Uncertainty quantification.
- Hessian-vector products.
- Material calibration.

## Requirements

- [ ] Hessian-vector product abstraction.
- [ ] Forward-over-reverse differentiation where appropriate.
- [ ] Reverse-over-forward evaluation where appropriate.
- [ ] Hyper-dual support for small compatible systems.
- [ ] Avoid explicit dense global Hessians by default.
- [ ] Reuse linear operators.
- [ ] Validate symmetry when mathematically expected.
- [ ] Profile cost versus finite-difference gradient perturbations.

Higher-order sensitivity methods should be phased in after reliable first-order derivatives.

---

# 16. Story J — Transient and Multiphysics Sensitivities

**Goal:** Extend the framework beyond static equilibrium.

Potential cases:

- Linear elastodynamics.
- Nonlinear transient structural response.
- Thermal-structural coupling.
- Time-dependent material models.
- Coupled field equations.
- Path-dependent optimization.

## Requirements

- [ ] Differentiate the actual discrete time integration scheme.
- [ ] Support direct transient sensitivities.
- [ ] Support discrete transient adjoints.
- [ ] Handle history-dependent state.
- [ ] Account for initial conditions and time-dependent loads.
- [ ] Evaluate checkpoint/recomputation strategies.
- [ ] Support event handling and invalid sensitivity detection.
- [ ] Maintain consistency with staggered or monolithic coupling.
- [ ] Track sensitivity validity across contact events.

Discrete transient adjoints must reflect the implemented time discretization and state-update algorithms, rather than assuming the static adjoint equation applies unchanged.

Advanced transient sensitivities are a separate maturity milestone and should not delay foundational implementation.

---

# PART IV — PERFORMANCE ARCHITECTURE

# 17. Rust-Native Performance Requirements

The framework must minimize runtime overhead.

Prioritize:

- Static dispatch in hot numerical kernels.
- Generic scalar arithmetic where justified.
- Fixed-size derivative arrays for small problems.
- Contiguous integration-point storage.
- Cache-friendly element processing.
- Thread-local reusable workspaces.
- Minimal heap allocation.
- Reusable sparse solver structures.
- Compatible factorization reuse.
- Matrix-free derivative actions.
- Bounded AD memory.
- Efficient parallel element assembly.
- Deterministic reductions when requested.

Avoid:

- Global reverse AD tapes by default.
- Unnecessary scalar-type switching.
- Repeated allocation in integration loops.
- Dynamic dispatch per scalar arithmetic operation.
- Excessive cloning of element/matrix data.
- Recording derivatives of operations that do not affect requested outputs.
- Repeatedly differentiating identical invariant material operations.

### AD dependency evaluation

Compare candidate Rust-compatible approaches for:

- Numerical correctness.
- Maintenance status.
- Licensing.
- Toolchain requirements.
- Platform portability.
- Thread safety.
- SIMD compatibility.
- Compilation time.
- Runtime throughput.
- Memory overhead.

Do not adopt JAX or C++ AD infrastructure solely because it appears in another engineering software stack.

A foreign-language implementation must demonstrate a measurable advantage that justifies the integration complexity.

---

# 18. Proposed Rust Interfaces

Conceptual interfaces; reconcile with the existing code architecture.

```rust
pub enum DerivativeMethod {
    Analytic,
    ForwardAd,
    ReverseAd,
    ComplexStep,
    HyperDual,
    Generalized,
    FiniteDifferenceVerification,
}

pub enum SensitivityMethod {
    Direct,
    DiscreteAdjoint,
    MatrixFree,
}

pub enum Smoothness {
    Smooth,
    PiecewiseSmooth,
    Semismooth,
    Nondifferentiable,
    Unknown,
}

pub enum DerivativeRequest {
    Jacobian,
    JacobianVectorProduct,
    VectorJacobianProduct,
    HessianVectorProduct,
    ParameterGradient,
}

pub struct DerivativeContext {
    pub smoothness: Smoothness,
    pub input_dimension: usize,
    pub output_dimension: usize,
    pub requested: DerivativeRequest,
    pub memory_budget_bytes: usize,
}

pub trait DerivativeKernel {
    fn capabilities(&self) -> DerivativeCapabilities;

    fn evaluate(
        &self,
        context: &DerivativeContext,
        workspace: &mut DerivativeWorkspace,
    ) -> Result<DerivativeOutput, DerivativeError>;
}

pub trait DerivativePolicy {
    fn select(
        &self,
        request: &DerivativeContext,
        candidates: &[DerivativeCapabilities],
    ) -> Result<DerivativePlan, DerivativeError>;
}
```

Optimize the concrete APIs after auditing FEA-Core.

Keep typed differentiation operators separate from derivative policy selection.

Avoid unnecessary runtime dispatch inside inner numerical loops.

---

# PART V — INTEGRATION WITH EXISTING AND PLANNED CAPABILITIES

# 19. Integration with Adaptive Mesh Refinement

The derivative engine must integrate with the previously planned goal-oriented AMR architecture.

The shared tangent and adjoint facilities should support:

- Primal nonlinear tangent construction.
- Adjoint equations for goal-oriented error estimation.
- QoI sensitivity evaluation.
- Element-local residual derivatives.
- Contact-aware adjoint linearizations.
- Compatible matrix-free operators.

**Do not implement a second independent adjoint solver inside AMR.**

Where appropriate, both optimization and AMR should reuse the same global adjoint linear algebra infrastructure while maintaining separate mathematical objectives.

The AMR estimator shall remain responsible for error localization and refinement marking.

The derivative engine shall remain responsible for computing and applying derivatives.

If a required enriched adjoint discretization is unavailable, the derivative engine must not silently substitute an incompatible solution.

---

# 20. Integration with Hybrid Meshing

The derivative engine should also support mesh-dependent sensitivity operations.

Potential requirements:

- Geometry parameter derivatives.
- Element coordinate sensitivities.
- Shape-function mapping derivatives.
- Jacobian determinant sensitivities.
- Element stiffness derivatives.
- Mesh deformation sensitivities.
- Shape and topology optimization.

Do not assume that discrete remeshing operations are differentiable.

Changing element connectivity introduces a discrete topology change, which generally requires specialized shape-sensitivity treatment or an explicitly defined differentiable surrogate.

Keep mesh topology modification separate from continuous geometric differentiation.

---

# PART VI — VERIFICATION AND BENCHMARKING

# 21. Mandatory Numerical Validation

Implement a hierarchical derivative test suite.

### Level 1 — Scalar/kernel tests

- Elementary smooth functions.
- Dual-number propagation.
- Reverse-mode gradients.
- JVP/VJP consistency.
- Hessian-vector products.
- Complex-step verification.
- Hyper-dual verification.

### Level 2 — Material tests

- Linear elastic tangent.
- Hyperelastic tangent.
- Plastic return mapping.
- Consistent algorithmic tangent.
- Material transition behavior.
- History variable sensitivity.

### Level 3 — Element tests

- Linear element stiffness.
- Nonlinear element tangent.
- Geometric stiffness.
- Parameter derivatives.
- Directional residual checks.

### Level 4 — Contact tests

- Open contact.
- Closed frictionless contact.
- Sticking friction.
- Sliding friction.
- Active-set transition.
- Contact complementarity.
- Regularized versus unregularized laws.

### Level 5 — Global sensitivity tests

- Cantilever displacement sensitivity.
- Compliance optimization.
- Material parameter sensitivity.
- Shape perturbation.
- Joint preload sensitivity.
- Nonlinear load-response sensitivity.
- Supported transient sensitivity.

Use independent analytical solutions, manufactured problems, finite-difference convergence studies, directional Taylor tests, and high-accuracy reference computations as appropriate.

---

# 22. Derivative Consistency and Accuracy

For a smooth differentiable residual:

\[
R(u+\epsilon v)
=
R(u)+\epsilon Jv+O(\epsilon^2)
\]

Use directional Taylor remainder tests with step-size sweeps to verify Jacobian actions.

Validate adjoint consistency through:

\[
w^T(Jv)=(J^Tw)^Tv
\]

within appropriate numerical tolerances.

For nonlinear constitutive models, verify that the consistent tangent matches the derivative of the implemented stress-update algorithm.

Do not impose a universal absolute error threshold of \(10^{-15}\).

Use method-specific, scale-aware tolerances derived from numerical precision and reference accuracy.

For nonsmooth models, use appropriate one-sided, directional, generalized derivative, and complementarity checks.

---

# 23. Performance Benchmarks

Compare:

1. Existing derivative implementation.
2. Hand-derived analytic tangent.
3. Forward AD.
4. Reverse AD.
5. Complex-step on compatible smooth kernels.
6. Hyper-dual on compatible smooth kernels.
7. Finite differences as a verification/reference method.
8. Direct sensitivity.
9. Global adjoint sensitivity.
10. Matrix-free derivative actions.

Capture:

- Derivative execution time.
- Element assembly time.
- Peak memory.
- Allocation count.
- Tape/workspace size.
- Sparse factorization time.
- Number of Newton iterations.
- Number of residual evaluations.
- Number of linear solver iterations.
- Total time-to-converged-solution.
- Sensitivity error.
- Convergence failure rate.
- Thread scaling.
- Deterministic repeatability.

A fast derivative kernel is not automatically a faster solver if it increases nonlinear iterations or worsens conditioning.

Benchmark total solution cost.

### Performance objective

Use a **5× speedup against finite-difference-based sensitivity workflows as an aspirational benchmark**, not a universal acceptance criterion.

Production acceptance requires demonstrated performance benefits on the workloads targeted by each derivative method and no unacceptable accuracy or stability regressions.

---

# PART VII — IMPLEMENTATION ROADMAP

# 24. Phase 0 — Mandatory Repository Audit

- [ ] Read latest `main`.
- [ ] Identify all derivative-producing code paths.
- [ ] Audit constitutive material implementations.
- [ ] Audit element tangent assembly.
- [ ] Audit nonlinear residual evaluation.
- [ ] Audit contact formulations.
- [ ] Audit sparse matrix and transpose capabilities.
- [ ] Audit existing adjoint/AMR functionality.
- [ ] Audit parameter and sensitivity interfaces.
- [ ] Audit solver convergence behavior.
- [ ] Evaluate Rust-compatible AD libraries.
- [ ] Establish performance and accuracy baselines.
- [ ] Produce dependency-aware implementation plan.

**Exit gate:** Verified audit and architecture compatibility assessment.

# 25. Phase 1 — Core Derivative Infrastructure

- [ ] Implement derivative capability registry.
- [ ] Implement routing policy.
- [ ] Define differentiability metadata.
- [ ] Define derivative request/output types.
- [ ] Define reusable workspaces.
- [ ] Add explicit error handling.
- [ ] Add tracing and diagnostics.
- [ ] Create mock kernel tests.

**Exit gate:** Router correctly selects or rejects differentiation strategies.

# 26. Phase 2 — Forward AD Foundation

- [ ] Implement or integrate forward AD.
- [ ] Support fixed-size dual numbers.
- [ ] Implement JVP.
- [ ] Integrate with simple material kernels.
- [ ] Verify Jacobian accuracy.
- [ ] Benchmark against analytic derivatives.

**Exit gate:** Validated forward AD for supported smooth local kernels.

# 27. Phase 3 — Constitutive Derivatives

- [ ] Introduce material differentiation interfaces.
- [ ] Integrate forward AD.
- [ ] Evaluate reverse AD.
- [ ] Implement consistent tangent support.
- [ ] Support implicit constitutive differentiation.
- [ ] Validate return mapping.
- [ ] Verify global Newton consistency.

**Exit gate:** Supported nonlinear material models produce verified consistent tangents.

# 28. Phase 4 — Global Sensitivity Infrastructure

- [ ] Implement/reuse direct sensitivity solves.
- [ ] Implement/reuse discrete adjoint solves.
- [ ] Support parameter residual derivatives.
- [ ] Support JVP/VJP operators.
- [ ] Integrate with sparse backends.
- [ ] Evaluate factorization reuse.
- [ ] Validate QoI sensitivities.
- [ ] Establish shared AMR/optimization APIs.

**Exit gate:** Direct and adjoint sensitivities agree with independent references.

# 29. Phase 5 — Nonsmooth Contact Differentiation

- [ ] Audit active-set behavior.
- [ ] Integrate consistent contact tangents.
- [ ] Add generalized derivatives where required.
- [ ] Evaluate semismooth Newton.
- [ ] Validate complementarity.
- [ ] Verify frictional contact transitions.
- [ ] Add globalization safeguards.
- [ ] Establish nonsmooth sensitivity reporting.

**Exit gate:** Supported contact problems converge reliably with correct derivative semantics.

# 30. Phase 6 — Reverse AD and Higher-Order Methods

- [ ] Integrate reverse AD where justified.
- [ ] Support VJP.
- [ ] Implement selective tape reuse.
- [ ] Evaluate mixed forward/reverse differentiation.
- [ ] Add Hessian-vector products.
- [ ] Add complex-step verification kernels.
- [ ] Add hyper-dual verification kernels.
- [ ] Benchmark memory and runtime.

**Exit gate:** Advanced methods demonstrate justified utility over existing alternatives.

# 31. Phase 7 — Transient/Multiphysics Extension

- [ ] Audit supported transient integrators.
- [ ] Implement compatible discrete sensitivity propagation.
- [ ] Add transient adjoints where justified.
- [ ] Support history-state checkpointing.
- [ ] Integrate supported coupled physics.
- [ ] Validate path-dependent sensitivities.
- [ ] Handle events and active-set changes.

**Exit gate:** Supported transient and multiphysics sensitivities pass reference benchmarks.

# 32. Phase 8 — Optimization and Production Hardening

- [ ] Profile all numerical kernels.
- [ ] Optimize workspace allocation.
- [ ] Optimize thread scheduling.
- [ ] Evaluate SIMD.
- [ ] Evaluate sparse operator reuse.
- [ ] Verify deterministic behavior.
- [ ] Run complete regression suite.
- [ ] Compare total solver performance.
- [ ] Document supported methods and limitations.
- [ ] Add configuration and diagnostics APIs.
- [ ] Complete production-readiness review.

**Exit gate:** Verified numerical stability, accuracy, maintainability, and performance.

---

# PART VIII — RISKS AND MITIGATIONS

# 33. Technical Risks

| Risk | Mitigation |
|---|---|
| Reverse AD memory overhead | Local tape scope, checkpoint/recompute, fixed workspaces |
| Incorrect nonlinear material tangents | Implicit differentiation and consistent tangent verification |
| Nonsmooth contact transitions | Active-set/generalized derivatives rather than invalid complex-step |
| Method-routing overhead | Cache decisions and prefer static specialization |
| Sparse matrix incompatibility | JVP/VJP abstraction and explicit backend capability checks |
| Compiler/toolchain complexity | Evaluate Rust-native methods before foreign dependencies |
| Incorrect global gradients | Direct/adjoint comparison and independent perturbation checks |
| AD through unconverged algorithms | Differentiate converged implicit mappings where appropriate |
| High-order derivative memory cost | Prefer matrix-free directional products |
| Multithreading contention | Thread-local workspaces and deterministic scheduling |
| Mesh topology changes | Keep remeshing operations outside ordinary smooth differentiation |
| AD branch differentiation errors | Explicit smoothness metadata and branch-validity checks |

---

# PART IX — DELIVERABLES AND ACCEPTANCE

# 34. Deliverables

- [ ] Verified repository audit.
- [ ] Adaptive derivative policy engine.
- [ ] Common derivative kernel abstraction.
- [ ] Forward-mode AD.
- [ ] Selective reverse-mode AD.
- [ ] Consistent constitutive tangent framework.
- [ ] Direct sensitivity capability.
- [ ] Discrete adjoint capability.
- [ ] JVP/VJP operators.
- [ ] Matrix-free integration.
- [ ] Contact generalized derivative support.
- [ ] Complex-step and hyper-dual verification capability.
- [ ] Higher-order derivative interfaces.
- [ ] Supported transient sensitivity infrastructure.
- [ ] Performance benchmark suite.
- [ ] Numerical verification suite.
- [ ] Configuration API.
- [ ] Diagnostic reporting.
- [ ] AMR interoperability.
- [ ] Hybrid meshing interoperability.
- [ ] Technical documentation and support matrix.

# 35. Definition of Done

The EPIC is complete only when:

1. Derivative methods are selected according to validated numerical capabilities.
2. Linear and nonlinear derivative calculations remain correct.
3. Constitutive tangents are consistent with implemented material algorithms.
4. Global sensitivity calculations match independent references.
5. Contact transitions use mathematically appropriate generalized or branch-consistent derivatives.
6. Incompatible complex-step/hyper-dual operations are rejected.
7. AD memory consumption is controlled and measured.
8. Matrix-free and sparse operations function for supported solver backends.
9. Existing FEA regression tests continue to pass.
10. Performance and accuracy are documented using reproducible benchmarks.
11. AMR and optimization can reuse global derivative infrastructure.
12. Unsupported functionality is clearly reported.
13. Implementation remains independent of the GUI.
14. No unapproved direct modifications are made to `main`.

---

# PART X — CLAUDE CODE EXECUTION DIRECTIVE

## 36. Begin Implementation

Read the current repository and perform Phase 0 before changing numerical architecture.

Apply the following principles:

**Trust, but verify.**

- Verify the existing implementation before proposing replacements.
- Reuse tested numerical infrastructure.
- Avoid duplicate adjoint or tangent solvers.
- Preserve existing element/material APIs when possible.
- Introduce compatibility adapters where justified.
- Implement in small, testable increments.
- Prefer mathematically valid methods over convenient approximations.
- Benchmark method selection instead of assuming one AD mode is optimal.
- Run tests after every meaningful change.
- Record baseline and updated performance results.
- Keep changes on a dedicated feature branch.
- Do not automatically merge into `main`.

### Recommended implementation sequence

Begin with the derivative capability registry and forward AD foundation.

Then establish constitutive-consistent Jacobians and global sensitivity infrastructure.

Treat contact nonsmooth differentiation as a specialized numerical subsystem.

Add reverse AD, higher-order differentiation, and transient sensitivities only when foundational correctness and solver compatibility are established.

### Reporting requirements

At implementation checkpoints, report:

- Completed tasks.
- Modified files/modules.
- Numerical methods added.
- Actual test results.
- Derivative consistency results.
- Runtime/memory measurements.
- Remaining limitations.
- Next dependent tasks.

Do not stop after merely producing a plan when implementation can proceed safely.

Request an explicit decision for substantial architectural incompatibilities, high-risk numerical changes, or new dependencies with material maintenance implications.

### Final engineering objective

**Build a unified differentiation infrastructure that supplies the most appropriate verified derivative operator for each numerical task, while minimizing total computational cost and preserving solver accuracy.**

The architecture should support a future FEA-Core ecosystem in which nonlinear materials, contact, adaptive meshing, adjoint error estimation, design optimization, and multiphysics analyses share consistent, efficient numerical derivative infrastructure rather than maintaining separate, incompatible implementations.