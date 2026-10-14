//! Levels 1 of the derivative suite: scalar kernels, every differentiation mode against analytic references and against each other,
//! JVP / VJP consistency, Hessian-vector products, complex-step and hyper-dual verification kernels, smoothness checks, and the
//! policy engine's selections and rejections.

use fea_core::derivative::ops::*;
use fea_core::derivative::scalar::Scalar;
use fea_core::derivative::tape::TapeError;
use fea_core::derivative::verify::*;
use fea_core::derivative::*;

/// R^3 -> R^2: f0 = x0 exp(x1) + sin(x2) x0^2, f1 = sqrt(1 + x0^2 + x1^2) / (1 + atan(x2)) + tanh(x1 x2) + x2^1.5.
struct Mixed;

impl SmoothKernel for Mixed {
    fn n_in(&self) -> usize {
        3
    }
    fn n_out(&self) -> usize {
        2
    }
    fn eval<S: Scalar>(&self, x: &[S], out: &mut [S]) {
        out[0] = x[0] * x[1].exp() + x[2].sin() * x[0].powi(2);
        out[1] = (S::cst(1.0) + x[0] * x[0] + x[1] * x[1]).sqrt() / (S::cst(1.0) + x[2].atan()) + (x[1] * x[2]).tanh() + x[2].powf(1.5);
    }
}

fn mixed_jacobian(x: &[f64]) -> Jacobian {
    let (a, b, c) = (x[0], x[1], x[2]);
    let r = (1.0 + a * a + b * b).sqrt();
    let den = 1.0 + c.atan();
    let th = (b * c).tanh();
    let sech2 = 1.0 - th * th;
    Jacobian {
        m: 2,
        n: 3,
        data: vec![
            b.exp() + 2.0 * a * c.sin(),
            a * b.exp(),
            c.cos() * a * a,
            a / (r * den),
            b / (r * den) + sech2 * c,
            -r / (den * den) / (1.0 + c * c) + sech2 * b + 1.5 * c.sqrt(),
        ],
    }
}

const X: [f64; 3] = [0.7, -0.4, 1.3];

#[test]
fn every_mode_reproduces_the_analytic_jacobian() {
    let want = mixed_jacobian(&X);
    let fwd = jacobian_forward(&Mixed, &X);
    let mut ws = DerivativeWorkspace::new(1000);
    let (rev, used) = jacobian_reverse(&Mixed, &X, &mut ws).unwrap();
    let cs = jacobian_complex_step(&Mixed, &X, 1e-30);
    let fd = jacobian_central_difference(&Mixed, &X, 1e-6);
    assert!(fwd.relative_difference(&want) < 1e-14, "forward {:e}", fwd.relative_difference(&want));
    assert!(rev.relative_difference(&want) < 1e-14, "reverse {:e}", rev.relative_difference(&want));
    assert!(cs.relative_difference(&want) < 1e-14, "complex step {:e}", cs.relative_difference(&want));
    // the difference reference is only as good as its step: much worse than the others
    assert!(fd.relative_difference(&want) < 1e-8 && fd.relative_difference(&want) > cs.relative_difference(&want));
    assert!(used.ops > 10 && used.bytes == used.ops * 24, "{used:?}");
}

/// n inputs, 3 outputs: sums, products and a nonlinear mix, to exercise the chunking (n = 1..13 crosses every chunk width).
struct Wide(usize);

impl SmoothKernel for Wide {
    fn n_in(&self) -> usize {
        self.0
    }
    fn n_out(&self) -> usize {
        3
    }
    fn eval<S: Scalar>(&self, x: &[S], out: &mut [S]) {
        let mut s = S::cst(0.0);
        let mut p = S::cst(1.0);
        let mut q = S::cst(0.0);
        for (i, &v) in x.iter().enumerate() {
            s = s + v * S::cst(1.0 + i as f64);
            p = p * (S::cst(1.0) + v.scale(0.1));
            q = q + (v * v).sin();
        }
        out[0] = s;
        out[1] = p;
        out[2] = q + s.exp().scale(1e-3);
    }
}

#[test]
fn chunked_forward_mode_matches_reverse_for_every_input_dimension() {
    for n in 1..=13 {
        let x: Vec<f64> = (0..n).map(|i| 0.3 + 0.11 * i as f64).collect();
        let k = Wide(n);
        let fwd = jacobian_forward(&k, &x);
        let (rev, _) = jacobian_reverse(&k, &x, &mut DerivativeWorkspace::new(10_000)).unwrap();
        assert!(fwd.relative_difference(&rev) < 1e-13, "n = {n}: {:e}", fwd.relative_difference(&rev));
        assert!(fwd.relative_difference(&jacobian_central_difference(&k, &x, 1e-6)) < 1e-7, "n = {n}");
    }
}

#[test]
fn jvp_and_vjp_are_the_actions_of_the_jacobian_and_adjoint_to_each_other() {
    let j = mixed_jacobian(&X);
    let (v, w) = ([0.3, -1.1, 0.8], [1.4, -0.6]);
    let jv = jvp(&Mixed, &X, &v);
    let (jtw, _) = vjp(&Mixed, &X, &w, &mut DerivativeWorkspace::new(1000)).unwrap();
    for (a, b) in jv.iter().zip(j.apply(&v)) {
        assert!((a - b).abs() < 1e-14);
    }
    for (a, b) in jtw.iter().zip(j.apply_transpose(&w)) {
        assert!((a - b).abs() < 1e-14);
    }
    assert!(adjoint_consistency(&jv, &w, &jtw, &v) < 1e-14);
}

#[test]
fn the_taylor_test_sees_order_two_for_a_correct_action_and_order_one_for_a_wrong_one() {
    let v = [0.4, 0.9, -0.5];
    let good = taylor_check_kernel(&Mixed, &X, &v);
    assert!(good.observed_order > 1.8, "{}", good.observed_order);
    let steps: Vec<f64> = (1..=7).map(|p| 10f64.powi(-p)).collect();
    let bad = taylor_test(&|y| evaluate(&Mixed, y), &|y, d| jacobian_forward(&Mixed, y).apply(d).iter().map(|a| a * 1.001).collect(), &X, &v, &steps, 1e-13);
    assert!(bad.observed_order < 1.3, "{}", bad.observed_order);
}

/// Scalar f(x) = x0^3 x1 + exp(x0 x2) + sin(x1) x2^2.
struct Cubic;

impl SmoothKernel for Cubic {
    fn n_in(&self) -> usize {
        3
    }
    fn n_out(&self) -> usize {
        1
    }
    fn eval<S: Scalar>(&self, x: &[S], out: &mut [S]) {
        out[0] = x[0].powi(3) * x[1] + (x[0] * x[2]).exp() + x[1].sin() * x[2] * x[2];
    }
}

fn cubic_hessian(x: &[f64]) -> [[f64; 3]; 3] {
    let (a, b, c) = (x[0], x[1], x[2]);
    let e = (a * c).exp();
    [[6.0 * a * b + c * c * e, 3.0 * a * a, e + a * c * e], [3.0 * a * a, -b.sin() * c * c, 2.0 * b.cos() * c], [e + a * c * e, 2.0 * b.cos() * c, a * a * e + 2.0 * b.sin()]]
}

#[test]
fn hessian_vector_products_agree_across_forward_over_reverse_reverse_over_forward_and_hyper_dual() {
    let x = [0.6, -0.8, 0.5];
    let v = [1.0, 0.3, -0.7];
    let h = cubic_hessian(&x);
    let want: Vec<f64> = (0..3).map(|i| (0..3).map(|j| h[i][j] * v[j]).sum()).collect();
    let for_ = hvp_forward_over_reverse(&Cubic, &x, &[1.0], &v, 1000).unwrap();
    let rof = hvp_reverse_over_forward(&Cubic, &x, &[1.0], &v, 1000).unwrap();
    let hd = hessian_hyperdual(&Cubic, &x, 0);
    for i in 0..3 {
        assert!((for_[i] - want[i]).abs() < 1e-13, "forward-over-reverse {} vs {}", for_[i], want[i]);
        assert!((rof[i] - want[i]).abs() < 1e-13, "reverse-over-forward {} vs {}", rof[i], want[i]);
        for j in 0..3 {
            assert!((hd[i * 3 + j] - h[i][j]).abs() < 1e-13, "hyper-dual H[{i}][{j}]");
        }
    }
    // the Hessian is symmetric and the product is consistent with a Taylor test of the gradient
    for i in 0..3 {
        for j in 0..3 {
            assert!((hd[i * 3 + j] - hd[j * 3 + i]).abs() < 1e-14);
        }
    }
    let grad = |y: &[f64]| vjp(&Cubic, y, &[1.0], &mut DerivativeWorkspace::new(1000)).unwrap().0;
    let t = taylor_test(&grad, &|y, d| hvp_forward_over_reverse(&Cubic, y, &[1.0], d, 1000).unwrap(), &x, &v, &[1e-1, 1e-2, 1e-3, 1e-4], 1e-13);
    assert!(t.observed_order > 1.8, "{}", t.observed_order);
}

#[test]
fn a_weighted_hessian_vector_product_of_a_vector_kernel_is_the_hessian_of_the_weighted_sum() {
    let (x, v, w) = ([0.7, -0.4, 1.3], [0.2, 0.5, -0.9], [1.5, -0.8]);
    let a = hvp_forward_over_reverse(&Mixed, &x, &w, &v, 4000).unwrap();
    let b = hvp_reverse_over_forward(&Mixed, &x, &w, &v, 4000).unwrap();
    // central difference of the weighted gradient along v
    let g = |y: &[f64]| vjp(&Mixed, y, &w, &mut DerivativeWorkspace::new(4000)).unwrap().0;
    let h = 1e-6;
    let (gp, gm) = (g(&[x[0] + h * v[0], x[1] + h * v[1], x[2] + h * v[2]]), g(&[x[0] - h * v[0], x[1] - h * v[1], x[2] - h * v[2]]));
    for i in 0..3 {
        assert!((a[i] - b[i]).abs() < 1e-12, "{} vs {}", a[i], b[i]);
        assert!((a[i] - (gp[i] - gm[i]) / (2.0 * h)).abs() < 1e-7);
    }
}

#[test]
fn a_tape_that_overflows_is_an_error_and_a_reused_workspace_keeps_its_memory() {
    let mut small = DerivativeWorkspace::new(5);
    assert_eq!(jacobian_reverse(&Mixed, &X, &mut small).unwrap_err(), TapeError::Overflow { limit: 5 });
    let mut ws = DerivativeWorkspace::new(1000);
    jacobian_reverse(&Mixed, &X, &mut ws).unwrap();
    let cap = ws.tape.capacity_bytes();
    for _ in 0..50 {
        jacobian_reverse(&Mixed, &X, &mut ws).unwrap();
    }
    assert_eq!(ws.tape.capacity_bytes(), cap, "the tape is reused, not regrown");
}

/// A branching kernel: f = x0 > 1 ? x0^2 : 2 x0 - 1 (continuous, kinked in the derivative at 1).
struct Branchy;

impl SmoothKernel for Branchy {
    fn n_in(&self) -> usize {
        1
    }
    fn n_out(&self) -> usize {
        1
    }
    fn eval<S: Scalar>(&self, x: &[S], out: &mut [S]) {
        out[0] = if x[0].gt(&S::cst(1.0)) { x[0] * x[0] } else { x[0].scale(2.0) - S::cst(1.0) };
    }
}

/// |x0| * x1
struct Absolute;

impl SmoothKernel for Absolute {
    fn n_in(&self) -> usize {
        2
    }
    fn n_out(&self) -> usize {
        1
    }
    fn eval<S: Scalar>(&self, x: &[S], out: &mut [S]) {
        out[0] = x[0].abs() * x[1];
    }
}

#[test]
fn derivatives_follow_the_branch_taken_and_smoothness_is_checked_not_assumed() {
    // every mode differentiates the branch the evaluation is on
    assert!((jacobian_forward(&Branchy, &[1.5]).at(0, 0) - 3.0).abs() < 1e-15);
    assert!((jacobian_forward(&Branchy, &[0.5]).at(0, 0) - 2.0).abs() < 1e-15);
    let (r, _) = jacobian_reverse(&Branchy, &[1.5], &mut DerivativeWorkspace::new(100)).unwrap();
    assert!((r.at(0, 0) - 3.0).abs() < 1e-15);
    // the probe records the comparison, and classifies a point near the boundary as piecewise smooth
    assert_eq!(branch_signature(&Branchy, &[1.5]).0, vec![true]);
    assert_eq!(classify_smoothness(&Branchy, &[1.5], 1e-3), Smoothness::Smooth);
    assert_eq!(classify_smoothness(&Branchy, &[1.0005], 1e-3), Smoothness::PiecewiseSmooth);
    // complex step / hyper-dual are refused through a kink and through any abs / max / min
    assert_eq!(check_analytic(&Branchy, &[1.0005], 1e-3), Err(Incompatible::BranchBoundary));
    assert!(check_analytic(&Branchy, &[1.5], 1e-3).is_ok());
    assert_eq!(check_analytic(&Absolute, &[0.4, 1.0], 1e-3), Err(Incompatible::NonAnalyticOperation(1)));
    // still, away from the kink the abs kernel has a valid derivative in the AD modes
    assert!((jacobian_forward(&Absolute, &[-0.4, 2.0]).at(0, 0) + 2.0).abs() < 1e-15);
}

#[test]
fn the_hyper_dual_mixed_second_derivative_is_exact_and_the_complex_step_has_no_cancellation() {
    let x = [0.6, -0.8, 0.5];
    let h = cubic_hessian(&x);
    for i in 0..3 {
        for j in 0..3 {
            let m = mixed_second_hyperdual(&Cubic, &x, i, j)[0];
            assert!((m - h[i][j]).abs() < 1e-13, "d2f/dx{i}dx{j}: {m} vs {}", h[i][j]);
        }
    }
    // an absurdly small step is fine for the complex step and ruins a real difference quotient
    let want = mixed_jacobian(&X);
    assert!(jacobian_complex_step(&Mixed, &X, 1e-100).relative_difference(&want) < 1e-14);
    assert!(jacobian_central_difference(&Mixed, &X, 1e-100).relative_difference(&want) > 1e-2 || jacobian_central_difference(&Mixed, &X, 1e-100).data.iter().all(|v| *v == 0.0));
}

// ---------------------------------------------------------------------------------------------- the policy engine

fn engine() -> DerivativePolicyEngine {
    let mut e = DerivativePolicyEngine::default();
    e.register_scalar_kernel("hooke", Some(0.05));
    e.register_scalar_kernel("smooth_material", None);
    e.register_scalar_kernel("plastic_return", None);
    e.register_generalized("contact_law");
    let mut a = DerivativeCapabilities::new("only_jacobian", DerivativeMethod::Analytic);
    a.requests = vec![DerivativeRequest::Jacobian];
    e.register(a);
    e
}

fn ctx(req: DerivativeRequest, n: usize, m: usize) -> DerivativeContext {
    let mut c = DerivativeContext::new(req, n, m);
    c.function_ops = 200;
    c.function_cost = 200.0;
    c
}

#[test]
fn the_policy_selects_the_documented_strategy_for_each_context() {
    use DerivativeMethod::*;
    use DerivativeRequest::*;
    let e = engine();
    let pick = |k: &str, c: DerivativeContext| e.select(k, &c).map(|p| p.method);
    // constant linear-elastic: the analytic tangent
    assert_eq!(pick("hooke", ctx(Jacobian, 6, 6)), Ok(Analytic));
    // small smooth constitutive law without a closed form: forward AD (6 inputs, 6 outputs)
    assert_eq!(pick("smooth_material", ctx(Jacobian, 6, 6)), Ok(ForwardAd));
    // many parameters, one output: reverse; few parameters, many outputs: forward
    assert_eq!(pick("smooth_material", ctx(ParameterGradient, 500, 1)), Ok(ReverseAd));
    assert_eq!(pick("smooth_material", ctx(Jacobian, 2, 400)), Ok(ForwardAd));
    // directional actions: JVP forward, VJP reverse
    assert_eq!(pick("smooth_material", ctx(JacobianVectorProduct, 500, 40)), Ok(ForwardAd));
    assert_eq!(pick("smooth_material", ctx(VectorJacobianProduct, 500, 1)), Ok(ReverseAd));
    // second order: a Hessian-vector product is a single pass in forward-over-forward, never a dense Hessian
    assert_eq!(pick("smooth_material", ctx(HessianVectorProduct, 500, 1)), Ok(ForwardAd));
    // verification uses a method independent of the ones verified: complex step (exact) against forward and reverse AD
    let mut v = ctx(Jacobian, 4, 2);
    v.purpose = Purpose::Verification;
    v.verify_against = vec![ForwardAd, ReverseAd];
    assert_eq!(pick("smooth_material", v.clone()), Ok(ComplexStep));
    // complex step is a verification method: not chosen for production, but an explicit override is honoured on a smooth map...
    assert_ne!(pick("smooth_material", ctx(Jacobian, 4, 2)), Ok(ComplexStep));
    let mut forced = ctx(Jacobian, 4, 2);
    forced.override_method = Some(ComplexStep);
    assert_eq!(pick("smooth_material", forced.clone()), Ok(ComplexStep));
    // ...and refused, with the reason, where the map is not analytic
    forced.smoothness = Smoothness::PiecewiseSmooth;
    let err = e.select("smooth_material", &forced).unwrap_err();
    assert!(matches!(err, DerivativeError::Incompatible { method: ComplexStep, .. }) && err.to_string().contains("analytic"), "{err}");
    // a memory budget is enforced: forward mode needs 8 lanes per live value here, reverse mode one tape node per operation
    let mut tight = ctx(ParameterGradient, 500, 1);
    tight.memory_budget_bytes = 6000;
    assert_eq!(pick("smooth_material", tight.clone()), Ok(ReverseAd));
    tight.memory_budget_bytes = 1000;
    let err = e.select("smooth_material", &tight).unwrap_err();
    assert!(matches!(err, DerivativeError::NoCandidate { .. }) && err.to_string().contains("budget"), "{err}");
}

#[test]
fn nonsmooth_and_contact_contexts_select_generalized_derivatives_or_say_why_not() {
    use DerivativeMethod::*;
    use DerivativeRequest::*;
    let e = engine();
    // plastic return mapping is piecewise smooth: the AD modes give the derivative of the active branch, and say so
    let mut pw = ctx(Jacobian, 6, 6);
    pw.smoothness = Smoothness::PiecewiseSmooth;
    let plan = e.select("plastic_return", &pw).unwrap();
    assert_eq!((plan.method, plan.semantics), (ForwardAd, DerivativeSemantics::ActiveBranch));
    assert!(plan.rejected.iter().any(|r| r.method == ComplexStep) && plan.rejected.iter().any(|r| r.method == HyperDual));
    // a fixed contact active set is smooth on its branch; a transition needs the generalized derivative
    let mut fixed = ctx(Jacobian, 12, 12);
    fixed.contact = ContactStatus::FixedActiveSet;
    assert_eq!(e.select("smooth_material", &fixed).unwrap().semantics, DerivativeSemantics::ActiveBranch);
    let mut trans = ctx(Jacobian, 12, 12);
    trans.contact = ContactStatus::Transition;
    trans.smoothness = Smoothness::Semismooth;
    let plan = e.select("contact_law", &trans).unwrap();
    assert_eq!((plan.method, plan.semantics), (Generalized, DerivativeSemantics::Generalized));
    // AD kernels refuse a changing active set, with the reason
    let err = e.select("smooth_material", &trans).unwrap_err();
    assert!(matches!(err, DerivativeError::NoCandidate { .. }) && err.to_string().contains("generalized") || err.to_string().contains("semismooth") || err.to_string().contains("Semismooth"), "{err}");
    // nondifferentiable and unknown maps get no derivative, the unknown one is told to classify first
    let mut nd = ctx(Jacobian, 3, 3);
    nd.smoothness = Smoothness::Nondifferentiable;
    assert!(e.select("smooth_material", &nd).unwrap_err().to_string().contains("nondifferentiable"));
    let mut unk = ctx(Jacobian, 3, 3);
    unk.smoothness = Smoothness::Unknown;
    assert!(e.select("smooth_material", &unk).unwrap_err().to_string().contains("classify"));
    // an unsupported request and an unknown kernel are explicit errors
    assert!(matches!(e.select("only_jacobian", &ctx(HessianVectorProduct, 3, 1)), Err(DerivativeError::NoCandidate { .. })));
    assert_eq!(e.select("missing", &ctx(Jacobian, 3, 3)), Err(DerivativeError::UnknownKernel("missing".into())));
    // a difference fallback is used only when allowed, and is labelled approximate
    let mut p = engine();
    let mut fd_only = DerivativeCapabilities::new("opaque", FiniteDifferenceVerification);
    fd_only.requests = vec![Jacobian];
    fd_only.verification_only = true;
    fd_only.exact = false;
    p.register(fd_only);
    assert!(p.select("opaque", &ctx(Jacobian, 3, 3)).is_err());
    let mut allowed = ctx(Jacobian, 3, 3);
    allowed.allow_difference_fallback = true;
    let plan = p.select("opaque", &allowed).unwrap();
    assert_eq!((plan.method, plan.semantics), (FiniteDifferenceVerification, DerivativeSemantics::Approximate));
}

#[test]
fn decisions_are_cached_traced_and_independent_of_anything_but_the_context() {
    let e = engine();
    let c = ctx(DerivativeRequest::Jacobian, 6, 6);
    let first = e.select("smooth_material", &c).unwrap();
    let second = e.select("smooth_material", &c).unwrap();
    assert_eq!(first, second);
    let trace = e.drain_trace();
    assert_eq!(trace.len(), 2);
    assert!(!trace[0].cached && trace[1].cached && trace[0].message.contains("ForwardAd"), "{trace:?}");
    assert!(e.drain_trace().is_empty());
    assert!(first.report().contains("because"));
    assert_eq!(e.kernels(), vec!["contact_law", "hooke", "only_jacobian", "plastic_return", "smooth_material"]);
}

#[test]
fn the_sensitivity_policy_picks_direct_adjoint_or_matrix_free_from_the_dimensions_and_reuse() {
    use SensitivityMethod::*;
    // many parameters, one output: adjoint; one parameter, many outputs: direct
    let adj = select_sensitivity(&SensitivityContext::static_linear(20_000, 400, 1)).unwrap();
    assert_eq!(adj.method, DiscreteAdjoint, "{adj:?}");
    let dir = select_sensitivity(&SensitivityContext::static_linear(20_000, 1, 50)).unwrap();
    assert_eq!(dir.method, Direct, "{dir:?}");
    // a reusable factorization removes the factorization cost from every alternative and does not change the winner's logic
    let mut reuse = SensitivityContext::static_linear(20_000, 400, 1);
    reuse.factorization_available = true;
    assert!(select_sensitivity(&reuse).unwrap().cost < adj.cost);
    // no assembled tangent: matrix-free is the only choice
    let mut mf = SensitivityContext::static_linear(20_000, 10, 1);
    mf.assembled_available = false;
    assert_eq!(select_sensitivity(&mf).unwrap().method, MatrixFree);
    // nothing available is an error
    mf.matrix_free_available = false;
    assert!(select_sensitivity(&mf).is_err());
    assert!(select_sensitivity(&SensitivityContext::static_linear(10, 0, 1)).is_err());
}
