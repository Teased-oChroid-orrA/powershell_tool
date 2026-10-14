//! Verification and smoothness checks for derivative operators: directional Taylor remainder tests, adjoint (transpose)
//! consistency, and a probe scalar that records which branches a kernel takes so smoothness can be *checked* rather than assumed.

use super::ops::{evaluate, jacobian_forward, SmoothKernel};
use super::scalar::Scalar;
use std::cell::{Cell, RefCell};
use std::ops::{Add, Div, Mul, Neg, Sub};

/// What a probe saw while a kernel ran.
#[derive(Debug, Default)]
pub struct ProbeLog {
    /// The outcome of every comparison, in order.
    pub branches: RefCell<Vec<bool>>,
    /// Calls to operations that are not analytic at their kink (`abs`, `max`, `min`).
    pub nonanalytic: Cell<usize>,
}

/// A real scalar that logs comparisons and kinks to a shared [`ProbeLog`]. Constants carry no log.
#[derive(Clone, Copy)]
pub struct Probe<'a> {
    v: f64,
    log: Option<&'a ProbeLog>,
}

impl<'a> Probe<'a> {
    pub fn var(v: f64, log: &'a ProbeLog) -> Self {
        Self { v, log: Some(log) }
    }
    fn join(a: Self, b: Self, v: f64) -> Self {
        Self { v, log: a.log.or(b.log) }
    }
    fn un(self, v: f64) -> Self {
        Self { v, log: self.log }
    }
    fn note(&self, other: Option<&ProbeLog>, outcome: bool) -> bool {
        if let Some(l) = self.log.or(other) {
            l.branches.borrow_mut().push(outcome);
        }
        outcome
    }
}

impl<'a> Add for Probe<'a> {
    type Output = Self;
    fn add(self, o: Self) -> Self {
        Self::join(self, o, self.v + o.v)
    }
}
impl<'a> Sub for Probe<'a> {
    type Output = Self;
    fn sub(self, o: Self) -> Self {
        Self::join(self, o, self.v - o.v)
    }
}
impl<'a> Mul for Probe<'a> {
    type Output = Self;
    fn mul(self, o: Self) -> Self {
        Self::join(self, o, self.v * o.v)
    }
}
impl<'a> Div for Probe<'a> {
    type Output = Self;
    fn div(self, o: Self) -> Self {
        Self::join(self, o, self.v / o.v)
    }
}
impl<'a> Neg for Probe<'a> {
    type Output = Self;
    fn neg(self) -> Self {
        self.un(-self.v)
    }
}

impl<'a> Scalar for Probe<'a> {
    fn cst(v: f64) -> Self {
        Self { v, log: None }
    }
    fn value(&self) -> f64 {
        self.v
    }
    fn sqrt(self) -> Self {
        self.un(self.v.sqrt())
    }
    fn exp(self) -> Self {
        self.un(self.v.exp())
    }
    fn ln(self) -> Self {
        self.un(self.v.ln())
    }
    fn sin(self) -> Self {
        self.un(self.v.sin())
    }
    fn cos(self) -> Self {
        self.un(self.v.cos())
    }
    fn atan(self) -> Self {
        self.un(self.v.atan())
    }
    fn tanh(self) -> Self {
        self.un(self.v.tanh())
    }
    fn powi(self, n: i32) -> Self {
        self.un(self.v.powi(n))
    }
    fn powf(self, c: f64) -> Self {
        self.un(self.v.powf(c))
    }
    fn lt(&self, o: &Self) -> bool {
        self.note(o.log, self.v < o.v)
    }
    fn gt(&self, o: &Self) -> bool {
        self.note(o.log, self.v > o.v)
    }
    fn le(&self, o: &Self) -> bool {
        self.note(o.log, self.v <= o.v)
    }
    fn ge(&self, o: &Self) -> bool {
        self.note(o.log, self.v >= o.v)
    }
    fn abs(self) -> Self {
        if let Some(l) = self.log {
            l.nonanalytic.set(l.nonanalytic.get() + 1);
        }
        if self.lt(&Self::cst(0.0)) {
            -self
        } else {
            self
        }
    }
    fn max(self, other: Self) -> Self {
        if let Some(l) = self.log.or(other.log) {
            l.nonanalytic.set(l.nonanalytic.get() + 1);
        }
        if self.ge(&other) {
            self
        } else {
            other
        }
    }
    fn min(self, other: Self) -> Self {
        if let Some(l) = self.log.or(other.log) {
            l.nonanalytic.set(l.nonanalytic.get() + 1);
        }
        if self.le(&other) {
            self
        } else {
            other
        }
    }
}

/// The branches `k` takes at `x` and the number of non-analytic operations it calls.
pub fn branch_signature<K: SmoothKernel>(k: &K, x: &[f64]) -> (Vec<bool>, usize) {
    let log = ProbeLog::default();
    let xs: Vec<Probe> = x.iter().map(|&v| Probe::var(v, &log)).collect();
    let mut out = vec![Probe::cst(0.0); k.n_out()];
    k.eval(&xs, &mut out);
    (log.branches.into_inner(), log.nonanalytic.get())
}

/// How smooth `k` is around `x`, from the branches it takes at `x` and at `x` moved by `radius` along each axis and along the
/// diagonal in both senses. Same branches everywhere: [`Smooth`](super::Smoothness::Smooth) within that radius; different branches:
/// [`PiecewiseSmooth`](super::Smoothness::PiecewiseSmooth) (a branch boundary lies within the radius).
pub fn classify_smoothness<K: SmoothKernel>(k: &K, x: &[f64], radius: f64) -> super::Smoothness {
    let base = branch_signature(k, x).0;
    let n = x.len();
    let probe = |dir: &dyn Fn(usize) -> f64, sign: f64| {
        let xp: Vec<f64> = (0..n).map(|i| x[i] + sign * radius * dir(i) * x[i].abs().max(1.0)).collect();
        branch_signature(k, &xp).0
    };
    for s in [-1.0, 1.0] {
        for j in 0..n {
            if probe(&|i| if i == j { 1.0 } else { 0.0 }, s) != base {
                return super::Smoothness::PiecewiseSmooth;
            }
        }
        if probe(&|_| 1.0, s) != base {
            return super::Smoothness::PiecewiseSmooth;
        }
    }
    super::Smoothness::Smooth
}

/// Why a verification kernel cannot be used at a point.
#[derive(Debug, Clone, PartialEq)]
pub enum Incompatible {
    /// The kernel calls `abs`, `max` or `min`: not analytic at their kinks.
    NonAnalyticOperation(usize),
    /// A branch changes within the probe radius.
    BranchBoundary,
}

impl std::fmt::Display for Incompatible {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NonAnalyticOperation(n) => write!(f, "the kernel calls {n} non-analytic operation(s) (abs / max / min): complex-step and hyper-dual derivatives are not valid through them"),
            Self::BranchBoundary => write!(f, "the kernel changes branch within the probe radius: the point is on or near a kink"),
        }
    }
}

/// Complex-step / hyper-dual compatibility of `k` at `x`: no non-analytic operation and no branch change within `radius`.
pub fn check_analytic<K: SmoothKernel>(k: &K, x: &[f64], radius: f64) -> Result<(), Incompatible> {
    let (_, na) = branch_signature(k, x);
    if na > 0 {
        return Err(Incompatible::NonAnalyticOperation(na));
    }
    match classify_smoothness(k, x, radius) {
        super::Smoothness::Smooth => Ok(()),
        _ => Err(Incompatible::BranchBoundary),
    }
}

// ------------------------------------------------------------------------------------------- Taylor and adjoint tests

/// Result of a directional Taylor remainder sweep.
#[derive(Debug, Clone)]
pub struct TaylorReport {
    pub steps: Vec<f64>,
    /// `|f(x + e v) - f(x)|`: first-order in `e` whenever `J v != 0`.
    pub zeroth: Vec<f64>,
    /// `|f(x + e v) - f(x) - e J v|`: second-order in `e` when `J v` is the true directional derivative.
    pub first: Vec<f64>,
    /// Least-squares slope of `log first` against `log e` over the steps above the rounding floor.
    pub observed_order: f64,
}

/// Directional Taylor test of a Jacobian action: `f(x + e v) = f(x) + e Jv + O(e^2)`. `jv` returns `J(x) v`. The order is fitted over
/// the steps whose remainder is above `floor` times the scale of `f` (below it the rounding of `f` dominates), so a correct `Jv`
/// shows order near 2 and a wrong one near 1, whatever the conditioning.
pub fn taylor_test(f: &dyn Fn(&[f64]) -> Vec<f64>, jv: &dyn Fn(&[f64], &[f64]) -> Vec<f64>, x: &[f64], v: &[f64], steps: &[f64], floor: f64) -> TaylorReport {
    let f0 = f(x);
    let j = jv(x, v);
    let scale = f0.iter().fold(0.0f64, |m, a| m.max(a.abs())).max(1e-300);
    let (mut zeroth, mut first) = (Vec::new(), Vec::new());
    for &e in steps {
        let xe: Vec<f64> = x.iter().zip(v).map(|(a, b)| a + e * b).collect();
        let fe = f(&xe);
        let z = fe.iter().zip(&f0).fold(0.0f64, |m, (a, b)| m.max((a - b).abs()));
        let r = fe.iter().zip(&f0).zip(&j).fold(0.0f64, |m, ((a, b), c)| m.max((a - b - e * c).abs()));
        zeroth.push(z);
        first.push(r);
    }
    let pts: Vec<(f64, f64)> = steps.iter().zip(&first).filter(|(_, r)| **r > floor * scale).map(|(e, r)| (e.ln(), r.ln())).collect();
    let observed_order = if pts.len() >= 2 {
        let n = pts.len() as f64;
        let (sx, sy): (f64, f64) = pts.iter().fold((0.0, 0.0), |a, p| (a.0 + p.0, a.1 + p.1));
        let (sxx, sxy): (f64, f64) = pts.iter().fold((0.0, 0.0), |a, p| (a.0 + p.0 * p.0, a.1 + p.0 * p.1));
        (n * sxy - sx * sy) / (n * sxx - sx * sx)
    } else {
        f64::INFINITY // the remainder is below the rounding floor at every step: exact to working precision
    };
    TaylorReport { steps: steps.to_vec(), zeroth, first, observed_order }
}

/// `w^T (J v)` against `(J^T w)^T v`, relative: the forward and transpose actions of one operator must be adjoint.
pub fn adjoint_consistency(jv: &[f64], w: &[f64], jtw: &[f64], v: &[f64]) -> f64 {
    let a: f64 = w.iter().zip(jv).map(|(x, y)| x * y).sum();
    let b: f64 = jtw.iter().zip(v).map(|(x, y)| x * y).sum();
    (a - b).abs() / a.abs().max(b.abs()).max(1e-300)
}

/// Check the forward-mode Jacobian of `k` at `x` by a Taylor sweep along `v`.
pub fn taylor_check_kernel<K: SmoothKernel>(k: &K, x: &[f64], v: &[f64]) -> TaylorReport {
    let steps: Vec<f64> = (1..=7).map(|p| 10f64.powi(-p)).collect();
    taylor_test(&|y| evaluate(k, y), &|y, d| jacobian_forward(k, y).apply(d), x, v, &steps, 1e-13)
}
