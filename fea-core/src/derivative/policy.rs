//! The derivative capability registry and the cost-aware policy that picks a method from it.
//!
//! A kernel registers what it can do ([`DerivativeCapabilities`]: methods, supported requests and orders, smoothness classes,
//! scalar types, thread / determinism guarantees, a cost model). A caller describes what it needs ([`DerivativeContext`]). The
//! policy filters the candidates by mathematical compatibility first (a method that cannot give a valid derivative is *rejected
//! with the reason*, never approximated), then picks the cheapest by the estimated cost and memory. An explicit override is honoured
//! only if it survives the same compatibility filter.
//!
//! Selection is stable: it depends on the context alone (no iteration counters), and decisions are cached per context.

use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum DerivativeMethod {
    /// Hand-derived or closed-form derivative of the kernel.
    Analytic,
    ForwardAd,
    ReverseAd,
    /// Complex step: verification of smooth, analytic kernels.
    ComplexStep,
    /// Hyper-dual numbers: exact mixed second derivatives of smooth kernels.
    HyperDual,
    /// Implicit differentiation of a converged (local or global) system.
    Implicit,
    /// Generalized (Clarke) derivative / semismooth Newton element, for nonsmooth maps.
    Generalized,
    /// Central differences: verification, or a labelled fallback when allowed.
    FiniteDifferenceVerification,
}

impl DerivativeMethod {
    pub const ALL: [DerivativeMethod; 8] = [Self::Analytic, Self::ForwardAd, Self::ReverseAd, Self::ComplexStep, Self::HyperDual, Self::Implicit, Self::Generalized, Self::FiniteDifferenceVerification];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SensitivityMethod {
    Direct,
    DiscreteAdjoint,
    MatrixFree,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Smoothness {
    Smooth,
    PiecewiseSmooth,
    Semismooth,
    Nondifferentiable,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DerivativeRequest {
    Jacobian,
    JacobianVectorProduct,
    VectorJacobianProduct,
    HessianVectorProduct,
    ParameterGradient,
}

impl DerivativeRequest {
    /// Order of differentiation the request needs.
    pub fn order(self) -> u8 {
        if self == Self::HessianVectorProduct {
            2
        } else {
            1
        }
    }
}

/// What the derivative is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Purpose {
    Production,
    /// Checking another derivative: verification-only methods (complex step, finite differences) become eligible.
    Verification,
}

/// Contact state of the differentiated point.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ContactStatus {
    /// No contact in the differentiated map.
    None,
    /// Contact present and the active set does not change in the neighbourhood: the active branch is smooth.
    FixedActiveSet,
    /// The active set changes: complementarity, friction-cone boundaries.
    Transition,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScalarKind {
    F64,
    Dual,
    Complex,
    HyperDual,
    Reverse,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Device {
    Cpu,
}

/// What the derivative a plan returns *means*.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DerivativeSemantics {
    /// The classical derivative (the map is differentiable at the point).
    Classical,
    /// The derivative of the branch the evaluation is on: one of the one-sided derivatives at a kink.
    ActiveBranch,
    /// An element of the generalized (Clarke) Jacobian.
    Generalized,
    /// An approximation by differences, not a derivative of the algorithm.
    Approximate,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DerivativeCapabilities {
    pub kernel: String,
    pub method: DerivativeMethod,
    pub requests: Vec<DerivativeRequest>,
    pub max_order: u8,
    pub smoothness: Vec<Smoothness>,
    pub scalars: Vec<ScalarKind>,
    pub thread_safe: bool,
    pub deterministic: bool,
    pub devices: Vec<Device>,
    /// Only eligible for [`Purpose::Verification`] (or as an explicit, labelled fallback).
    pub verification_only: bool,
    /// Exact to the rounding of the algorithm it differentiates (not an approximation by differences).
    pub exact: bool,
    /// Cost of one analytic derivative evaluation relative to one kernel evaluation (`Analytic` only).
    pub analytic_cost: Option<f64>,
}

impl DerivativeCapabilities {
    pub fn new(kernel: &str, method: DerivativeMethod) -> Self {
        Self { kernel: kernel.to_string(), method, requests: vec![DerivativeRequest::Jacobian], max_order: 1, smoothness: vec![Smoothness::Smooth], scalars: vec![ScalarKind::F64], thread_safe: true, deterministic: true, devices: vec![Device::Cpu], verification_only: false, exact: true, analytic_cost: None }
    }
}

/// What a caller needs.
#[derive(Debug, Clone, PartialEq)]
pub struct DerivativeContext {
    pub smoothness: Smoothness,
    pub input_dimension: usize,
    pub output_dimension: usize,
    pub requested: DerivativeRequest,
    pub memory_budget_bytes: usize,
    pub purpose: Purpose,
    pub contact: ContactStatus,
    /// Operations in one evaluation of the kernel (sizes the tape) and its relative cost (defaults to the operation count).
    pub function_ops: usize,
    pub function_cost: f64,
    pub parallel: bool,
    pub deterministic_required: bool,
    /// An explicit choice; honoured only if it is compatible.
    pub override_method: Option<DerivativeMethod>,
    /// The methods whose result is being checked: a verification must not use it (it would agree with itself).
    pub verify_against: Vec<DerivativeMethod>,
    /// Allow a labelled finite-difference fallback when no exact method applies.
    pub allow_difference_fallback: bool,
}

impl DerivativeContext {
    pub fn new(requested: DerivativeRequest, input_dimension: usize, output_dimension: usize) -> Self {
        Self { smoothness: Smoothness::Smooth, input_dimension, output_dimension, requested, memory_budget_bytes: 64 << 20, purpose: Purpose::Production, contact: ContactStatus::None, function_ops: 100, function_cost: 100.0, parallel: false, deterministic_required: true, override_method: None, verify_against: Vec::new(), allow_difference_fallback: false }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Rejection {
    pub method: DerivativeMethod,
    pub kernel: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DerivativePlan {
    pub kernel: String,
    pub method: DerivativeMethod,
    pub request: DerivativeRequest,
    pub semantics: DerivativeSemantics,
    pub estimated_cost: f64,
    pub estimated_memory_bytes: usize,
    /// Why this method was chosen.
    pub reasons: Vec<String>,
    /// The methods that were considered and why each was rejected.
    pub rejected: Vec<Rejection>,
}

impl DerivativePlan {
    /// A readable account of the decision.
    pub fn report(&self) -> String {
        let mut s = format!("{:?} for {:?} of '{}' ({:?}); cost {:.3e}, memory {} B\n", self.method, self.request, self.kernel, self.semantics, self.estimated_cost, self.estimated_memory_bytes);
        for r in &self.reasons {
            s.push_str(&format!("  because {r}\n"));
        }
        for r in &self.rejected {
            s.push_str(&format!("  not {:?} ({}): {}\n", r.method, r.kernel, r.reason));
        }
        s
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum DerivativeError {
    UnknownKernel(String),
    /// Nothing registered can give a valid derivative for the request.
    NoCandidate { kernel: String, request: DerivativeRequest, rejected: Vec<Rejection> },
    /// An explicit override that is mathematically incompatible with the context.
    Incompatible { method: DerivativeMethod, reason: String },
    Unsupported(String),
    Numerical(String),
}

impl std::fmt::Display for DerivativeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownKernel(k) => write!(f, "no derivative capabilities are registered for '{k}'"),
            Self::NoCandidate { kernel, request, rejected } => {
                write!(f, "no derivative method can serve {request:?} of '{kernel}':")?;
                for r in rejected {
                    write!(f, " [{:?}: {}]", r.method, r.reason)?;
                }
                Ok(())
            }
            Self::Incompatible { method, reason } => write!(f, "{method:?} is not valid here: {reason}"),
            Self::Unsupported(s) | Self::Numerical(s) => f.write_str(s),
        }
    }
}

impl std::error::Error for DerivativeError {}

/// The decision rule. Implementations must be pure functions of their arguments.
pub trait DerivativePolicy {
    fn select(&self, context: &DerivativeContext, candidates: &[DerivativeCapabilities]) -> Result<DerivativePlan, DerivativeError>;
}

/// Relative costs of the AD modes, in units of one kernel evaluation. The defaults are measured on the Quad8 element stiffness by
/// `tests/derivative_bench.rs` (which prints the refit for the machine it runs on): a dual lane adds about a third of a plain
/// evaluation (the lanes vectorise), recording a tape costs about 13 and one sweep about 11.5, complex step 1.75, hyper-dual 3.3.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CostCalibration {
    pub dual_lane: f64,
    pub tape_record: f64,
    pub tape_sweep: f64,
    pub complex: f64,
    pub hyperdual: f64,
    pub tape_node_bytes: f64,
    pub dual_lane_bytes: f64,
}

impl Default for CostCalibration {
    fn default() -> Self {
        Self { dual_lane: 0.33, tape_record: 13.0, tape_sweep: 11.5, complex: 1.75, hyperdual: 3.3, tape_node_bytes: 24.0, dual_lane_bytes: 8.0 }
    }
}

#[derive(Debug, Clone, Default)]
pub struct DefaultPolicy {
    pub calibration: CostCalibration,
}

fn why_not(c: &DerivativeCapabilities, ctx: &DerivativeContext) -> Option<String> {
    if !c.requests.contains(&ctx.requested) {
        return Some(format!("does not provide {:?}", ctx.requested));
    }
    let order = ctx.requested.order();
    if c.max_order < order {
        return Some(format!("provides derivatives to order {} but order {order} is requested", c.max_order));
    }
    if c.verification_only && ctx.purpose != Purpose::Verification && !(ctx.allow_difference_fallback && c.method == DerivativeMethod::FiniteDifferenceVerification) && ctx.override_method != Some(c.method) {
        return Some("is a verification method (not eligible for production derivatives)".into());
    }
    if ctx.verify_against.contains(&c.method) {
        return Some("is the method being verified (an independent check is needed)".into());
    }
    if ctx.parallel && !c.thread_safe {
        return Some("is not thread safe and the caller runs in parallel".into());
    }
    if ctx.deterministic_required && !c.deterministic {
        return Some("is not deterministic and determinism is required".into());
    }
    // mathematical compatibility with the smoothness of the map
    let smooth_ok = c.smoothness.contains(&ctx.smoothness);
    match ctx.smoothness {
        Smoothness::Unknown => return Some("the smoothness of the map is unknown: classify it first (verify::classify_smoothness)".into()),
        Smoothness::Nondifferentiable => return Some("the map is nondifferentiable at the point: no derivative exists".into()),
        _ if !smooth_ok => {
            return Some(match (c.method, ctx.smoothness) {
                (DerivativeMethod::ComplexStep, _) => "complex step is valid for analytic maps only (a kink, branch change or complementarity condition breaks it)".into(),
                (DerivativeMethod::HyperDual, _) => "hyper-dual second derivatives need a map that is twice differentiable at the point".into(),
                (DerivativeMethod::Generalized, _) => "a generalized derivative is for nonsmooth maps".into(),
                (_, s) => format!("does not support {s:?} maps"),
            })
        }
        _ => {}
    }
    // contact: a changing active set needs a generalized derivative; a fixed one is smooth on its branch
    if ctx.contact == ContactStatus::Transition && c.method != DerivativeMethod::Generalized && c.method != DerivativeMethod::FiniteDifferenceVerification {
        return Some("the contact active set changes here: only a generalized derivative is meaningful".into());
    }
    None
}

impl DefaultPolicy {
    fn cost(&self, c: &DerivativeCapabilities, ctx: &DerivativeContext) -> (f64, usize) {
        let (n, m) = (ctx.input_dimension.max(1) as f64, ctx.output_dimension.max(1) as f64);
        let (f, ops) = (ctx.function_cost.max(1.0), ctx.function_ops.max(1) as f64);
        let cal = &self.calibration;
        let req = ctx.requested;
        let w = super::ops::forward_chunk_width(ctx.input_dimension) as f64;
        match c.method {
            DerivativeMethod::Analytic | DerivativeMethod::Implicit => {
                let rel = c.analytic_cost.unwrap_or(0.3);
                let times = match req {
                    DerivativeRequest::Jacobian => 1.0,
                    DerivativeRequest::JacobianVectorProduct | DerivativeRequest::VectorJacobianProduct => 0.5,
                    DerivativeRequest::HessianVectorProduct => 2.0,
                    DerivativeRequest::ParameterGradient => 1.0,
                };
                (f * rel * times * if c.method == DerivativeMethod::Implicit { 2.0 } else { 1.0 }, (ops * 8.0) as usize)
            }
            DerivativeMethod::ForwardAd => match req {
                DerivativeRequest::JacobianVectorProduct => (f * (1.0 + cal.dual_lane), (ops * cal.dual_lane_bytes) as usize),
                DerivativeRequest::HessianVectorProduct => (f * (1.0 + cal.dual_lane) * (1.0 + cal.dual_lane), (ops * cal.dual_lane_bytes * 2.0) as usize),
                _ => ((n / w).ceil() * f * (1.0 + w * cal.dual_lane), (ops * w * cal.dual_lane_bytes) as usize),
            },
            DerivativeMethod::ReverseAd => {
                let record = f * cal.tape_record;
                let mem = (ops * cal.tape_node_bytes) as usize;
                match req {
                    DerivativeRequest::VectorJacobianProduct | DerivativeRequest::ParameterGradient => (record + f * cal.tape_sweep, mem),
                    DerivativeRequest::HessianVectorProduct => (3.0 * (record + f * cal.tape_sweep), mem * 3),
                    _ => (record + m * f * cal.tape_sweep, mem),
                }
            }
            DerivativeMethod::ComplexStep => (n * f * cal.complex, (ops * 16.0) as usize),
            DerivativeMethod::HyperDual => (n * (n + 1.0) / 2.0 * f * cal.hyperdual, (ops * 32.0) as usize),
            DerivativeMethod::Generalized => (f * 2.0, (ops * 8.0) as usize),
            DerivativeMethod::FiniteDifferenceVerification => (2.0 * n * f, (ops * 8.0) as usize),
        }
    }
}

impl DerivativePolicy for DefaultPolicy {
    fn select(&self, ctx: &DerivativeContext, candidates: &[DerivativeCapabilities]) -> Result<DerivativePlan, DerivativeError> {
        let kernel = candidates.first().map(|c| c.kernel.clone()).unwrap_or_default();
        let mut rejected: Vec<Rejection> = Vec::new();
        let mut viable: Vec<(&DerivativeCapabilities, f64, usize)> = Vec::new();
        for c in candidates {
            if let Some(why) = why_not(c, ctx) {
                rejected.push(Rejection { method: c.method, kernel: c.kernel.clone(), reason: why });
                continue;
            }
            let (cost, mem) = self.cost(c, ctx);
            if mem > ctx.memory_budget_bytes {
                rejected.push(Rejection { method: c.method, kernel: c.kernel.clone(), reason: format!("needs about {mem} B, over the budget of {} B", ctx.memory_budget_bytes) });
                continue;
            }
            viable.push((c, cost, mem));
        }
        if let Some(want) = ctx.override_method {
            return match viable.iter().find(|(c, ..)| c.method == want) {
                Some((c, cost, mem)) => Ok(self.plan(c, *cost, *mem, ctx, vec![format!("{want:?} was requested explicitly and is compatible")], rejected)),
                None => {
                    let reason = rejected.iter().find(|r| r.method == want).map(|r| r.reason.clone()).unwrap_or_else(|| "no registered kernel offers it".into());
                    Err(DerivativeError::Incompatible { method: want, reason })
                }
            };
        }
        // exact methods first; a difference fallback only when nothing exact is viable
        let exact: Vec<_> = viable.iter().filter(|(c, ..)| c.exact).copied().collect();
        let pool = if exact.is_empty() { viable.clone() } else { exact };
        // cheapest; ties go to the earlier (more specific) registration
        let best = pool.iter().min_by(|a, b| a.1.total_cmp(&b.1));
        match best {
            Some((c, cost, mem)) => {
                let mut reasons = vec![format!("the cheapest of {} viable method(s) by estimated cost", pool.len())];
                if !c.exact {
                    reasons.push("no exact method is viable: this is a labelled difference fallback, not a derivative of the algorithm".into());
                }
                Ok(self.plan(c, *cost, *mem, ctx, reasons, rejected))
            }
            None => Err(DerivativeError::NoCandidate { kernel, request: ctx.requested, rejected }),
        }
    }
}

impl DefaultPolicy {
    fn plan(&self, c: &DerivativeCapabilities, cost: f64, mem: usize, ctx: &DerivativeContext, reasons: Vec<String>, rejected: Vec<Rejection>) -> DerivativePlan {
        let semantics = match (c.method, ctx.smoothness, ctx.contact) {
            (DerivativeMethod::FiniteDifferenceVerification, ..) => DerivativeSemantics::Approximate,
            (DerivativeMethod::Generalized, ..) => DerivativeSemantics::Generalized,
            (_, Smoothness::PiecewiseSmooth, _) | (_, _, ContactStatus::FixedActiveSet) => DerivativeSemantics::ActiveBranch,
            _ => DerivativeSemantics::Classical,
        };
        DerivativePlan { kernel: c.kernel.clone(), method: c.method, request: ctx.requested, semantics, estimated_cost: cost, estimated_memory_bytes: mem, reasons, rejected }
    }
}

/// A recorded decision, for diagnostics.
#[derive(Debug, Clone, PartialEq)]
pub struct TraceEvent {
    pub kernel: String,
    pub request: DerivativeRequest,
    pub method: Option<DerivativeMethod>,
    pub cached: bool,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct CacheKey {
    kernel: String,
    request: DerivativeRequest,
    smoothness: Smoothness,
    dims: (usize, usize, usize, usize),
    purpose: Purpose,
    contact: ContactStatus,
    cost_bits: u64,
    flags: (bool, bool, bool),
    override_method: Option<DerivativeMethod>,
    verify_against: Vec<DerivativeMethod>,
}

fn key_of(kernel: &str, c: &DerivativeContext) -> CacheKey {
    CacheKey {
        kernel: kernel.to_string(),
        request: c.requested,
        smoothness: c.smoothness,
        dims: (c.input_dimension, c.output_dimension, c.memory_budget_bytes, c.function_ops),
        purpose: c.purpose,
        contact: c.contact,
        cost_bits: c.function_cost.to_bits(),
        flags: (c.parallel, c.deterministic_required, c.allow_difference_fallback),
        override_method: c.override_method,
        verify_against: c.verify_against.clone(),
    }
}

/// The registry, the policy and a decision cache with a trace.
pub struct DerivativePolicyEngine {
    registry: Vec<DerivativeCapabilities>,
    policy: Box<dyn DerivativePolicy + Send + Sync>,
    cache: Mutex<HashMap<CacheKey, Result<DerivativePlan, DerivativeError>>>,
    trace: Mutex<Vec<TraceEvent>>,
}

impl Default for DerivativePolicyEngine {
    fn default() -> Self {
        Self::new(Box::new(DefaultPolicy::default()))
    }
}

impl DerivativePolicyEngine {
    pub fn new(policy: Box<dyn DerivativePolicy + Send + Sync>) -> Self {
        Self { registry: Vec::new(), policy, cache: Mutex::new(HashMap::new()), trace: Mutex::new(Vec::new()) }
    }

    /// Register one capability. Earlier registrations win cost ties, so register the specific (analytic) ones first.
    pub fn register(&mut self, caps: DerivativeCapabilities) {
        self.cache.lock().expect("cache").clear();
        self.registry.push(caps);
    }

    /// Register the standard set for a kernel written against [`Scalar`](super::Scalar): forward and reverse AD, hyper-dual and
    /// complex-step verification, and the difference reference; plus an analytic derivative if `analytic_cost` is given.
    pub fn register_scalar_kernel(&mut self, kernel: &str, analytic_cost: Option<f64>) {
        use DerivativeMethod::*;
        use DerivativeRequest::*;
        if let Some(cost) = analytic_cost {
            let mut a = DerivativeCapabilities::new(kernel, Analytic);
            a.requests = vec![Jacobian, JacobianVectorProduct, VectorJacobianProduct, HessianVectorProduct, ParameterGradient];
            a.max_order = 2;
            a.smoothness = vec![Smoothness::Smooth, Smoothness::PiecewiseSmooth];
            a.analytic_cost = Some(cost);
            self.register(a);
        }
        let mut fwd = DerivativeCapabilities::new(kernel, ForwardAd);
        fwd.requests = vec![Jacobian, JacobianVectorProduct, ParameterGradient, HessianVectorProduct];
        fwd.max_order = 2;
        fwd.smoothness = vec![Smoothness::Smooth, Smoothness::PiecewiseSmooth];
        fwd.scalars = vec![ScalarKind::F64, ScalarKind::Dual];
        self.register(fwd);
        let mut rev = DerivativeCapabilities::new(kernel, ReverseAd);
        rev.requests = vec![Jacobian, VectorJacobianProduct, ParameterGradient, HessianVectorProduct];
        rev.max_order = 2;
        rev.smoothness = vec![Smoothness::Smooth, Smoothness::PiecewiseSmooth];
        rev.scalars = vec![ScalarKind::F64, ScalarKind::Reverse, ScalarKind::Dual];
        self.register(rev);
        let mut hd = DerivativeCapabilities::new(kernel, HyperDual);
        hd.requests = vec![HessianVectorProduct, Jacobian];
        hd.max_order = 2;
        hd.scalars = vec![ScalarKind::HyperDual];
        self.register(hd);
        let mut cs = DerivativeCapabilities::new(kernel, ComplexStep);
        cs.requests = vec![Jacobian, JacobianVectorProduct];
        cs.scalars = vec![ScalarKind::Complex];
        cs.verification_only = true;
        self.register(cs);
        let mut fd = DerivativeCapabilities::new(kernel, FiniteDifferenceVerification);
        fd.requests = vec![Jacobian, JacobianVectorProduct, ParameterGradient, VectorJacobianProduct];
        fd.smoothness = vec![Smoothness::Smooth, Smoothness::PiecewiseSmooth];
        fd.verification_only = true;
        fd.exact = false;
        self.register(fd);
    }

    /// Register a generalized-derivative provider for a nonsmooth kernel.
    pub fn register_generalized(&mut self, kernel: &str) {
        let mut g = DerivativeCapabilities::new(kernel, DerivativeMethod::Generalized);
        g.requests = vec![DerivativeRequest::Jacobian, DerivativeRequest::JacobianVectorProduct, DerivativeRequest::VectorJacobianProduct, DerivativeRequest::ParameterGradient];
        g.smoothness = vec![Smoothness::Smooth, Smoothness::PiecewiseSmooth, Smoothness::Semismooth];
        self.register(g);
    }

    pub fn kernels(&self) -> Vec<String> {
        let mut v: Vec<String> = self.registry.iter().map(|c| c.kernel.clone()).collect();
        v.sort();
        v.dedup();
        v
    }

    pub fn capabilities_of(&self, kernel: &str) -> Vec<&DerivativeCapabilities> {
        self.registry.iter().filter(|c| c.kernel == kernel).collect()
    }

    /// Choose a method for `kernel` in `context`. Identical contexts return the cached decision.
    pub fn select(&self, kernel: &str, context: &DerivativeContext) -> Result<DerivativePlan, DerivativeError> {
        let key = key_of(kernel, context);
        if let Some(hit) = self.cache.lock().expect("cache").get(&key).cloned() {
            self.note(kernel, context, hit.as_ref().ok().map(|p| p.method), true, "cached decision".into());
            return hit;
        }
        let candidates: Vec<DerivativeCapabilities> = self.registry.iter().filter(|c| c.kernel == kernel).cloned().collect();
        let result = if candidates.is_empty() { Err(DerivativeError::UnknownKernel(kernel.to_string())) } else { self.policy.select(context, &candidates) };
        let message = match &result {
            Ok(p) => p.report(),
            Err(e) => e.to_string(),
        };
        self.note(kernel, context, result.as_ref().ok().map(|p| p.method), false, message);
        self.cache.lock().expect("cache").insert(key, result.clone());
        result
    }

    fn note(&self, kernel: &str, c: &DerivativeContext, method: Option<DerivativeMethod>, cached: bool, message: String) {
        self.trace.lock().expect("trace").push(TraceEvent { kernel: kernel.to_string(), request: c.requested, method, cached, message });
    }

    /// The recorded decisions since the last call.
    pub fn drain_trace(&self) -> Vec<TraceEvent> {
        std::mem::take(&mut *self.trace.lock().expect("trace"))
    }
}

// ----------------------------------------------------------------------------------------- global sensitivities

/// What a sensitivity analysis of a converged system needs to decide between direct, adjoint and matrix-free solves.
#[derive(Debug, Clone, PartialEq)]
pub struct SensitivityContext {
    pub n_dofs: usize,
    pub n_parameters: usize,
    pub n_outputs: usize,
    /// Cost of one solve with an available factorization (back-substitution), and of the factorization itself.
    pub solve_cost: f64,
    pub factor_cost: f64,
    /// A factorization of the tangent (or its transpose) already exists and can be reused.
    pub factorization_available: bool,
    /// Cost of one residual derivative `dR/dp` (one parameter).
    pub residual_derivative_cost: f64,
    pub matrix_free_available: bool,
    pub assembled_available: bool,
    /// Cost of one tangent-vector product and the Krylov iterations a matrix-free solve takes.
    pub matvec_cost: f64,
    pub krylov_iterations: f64,
    /// Steps of a path-dependent analysis the sensitivity must be propagated through (1 for a static one).
    pub steps: usize,
    pub symmetric_tangent: bool,
}

impl SensitivityContext {
    pub fn static_linear(n_dofs: usize, n_parameters: usize, n_outputs: usize) -> Self {
        let n = n_dofs as f64;
        Self { n_dofs, n_parameters, n_outputs, solve_cost: 4.0 * n.powf(1.2), factor_cost: 20.0 * n.powf(1.5), factorization_available: false, residual_derivative_cost: 6.0 * n, matrix_free_available: true, assembled_available: true, matvec_cost: 20.0 * n, krylov_iterations: 300.0, steps: 1, symmetric_tangent: true }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SensitivityPlan {
    pub method: SensitivityMethod,
    pub cost: f64,
    /// The costs of every method that was admissible, for the report.
    pub alternatives: Vec<(SensitivityMethod, f64)>,
    pub reasons: Vec<String>,
}

/// Choose direct, adjoint or matrix-free sensitivity solves from the dimensions and the measured or modelled costs. Direct needs one
/// tangent solve per parameter, adjoint one per output; both need `dR/dp` per parameter. A factorization is paid once (or not at
/// all when one exists and can be reused). Matrix-free replaces factorization by Krylov iterations and needs a matrix-free operator.
pub fn select_sensitivity(c: &SensitivityContext) -> Result<SensitivityPlan, DerivativeError> {
    if c.n_parameters == 0 || c.n_outputs == 0 {
        return Err(DerivativeError::Unsupported("a sensitivity analysis needs at least one parameter and one output".into()));
    }
    let steps = c.steps.max(1) as f64;
    let (np, no) = (c.n_parameters as f64, c.n_outputs as f64);
    let factor = if c.factorization_available { 0.0 } else { c.factor_cost } * steps;
    let rp = np * c.residual_derivative_cost * steps;
    let mut alt: Vec<(SensitivityMethod, f64)> = Vec::new();
    if c.assembled_available {
        alt.push((SensitivityMethod::Direct, factor + np * c.solve_cost * steps + rp));
        alt.push((SensitivityMethod::DiscreteAdjoint, factor + no * c.solve_cost * steps + rp));
    }
    if c.matrix_free_available {
        let per_solve = c.krylov_iterations * c.matvec_cost;
        // a matrix-free adjoint needs the transposed operator; the same count of solves as the assembled adjoint
        alt.push((SensitivityMethod::MatrixFree, np.min(no) * per_solve * steps + rp));
    }
    if alt.is_empty() {
        return Err(DerivativeError::Unsupported("neither an assembled nor a matrix-free tangent is available".into()));
    }
    let (method, cost) = *alt.iter().min_by(|a, b| a.1.total_cmp(&b.1)).expect("non-empty");
    let mut reasons = vec![format!("{} parameter(s), {} output(s): {:?} is the cheapest admissible", c.n_parameters, c.n_outputs, method)];
    if c.factorization_available {
        reasons.push("an existing factorization is reused".into());
    }
    if !c.symmetric_tangent && method != SensitivityMethod::MatrixFree {
        reasons.push("the tangent is nonsymmetric: the adjoint solve uses the transpose".into());
    }
    Ok(SensitivityPlan { method, cost, alternatives: alt, reasons })
}
