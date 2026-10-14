//! The differentiation operators: Jacobians, Jacobian-vector products, vector-Jacobian products and Hessian-vector products of a
//! [`SmoothKernel`], by every mode this module implements. The kernel is written once, generic over [`Scalar`]; each operator
//! instantiates it with the scalar type its mode needs (static dispatch, no allocation inside the kernel for the fixed-size
//! modes).

use super::complex::Complex;
use super::dual::Dual;
use super::hyperdual::HyperDual;
use super::scalar::Scalar;
use super::tape::{Rev, Tape, TapeError};

/// A smooth map `R^n -> R^m` written against [`Scalar`].
pub trait SmoothKernel {
    fn n_in(&self) -> usize;
    fn n_out(&self) -> usize;
    fn eval<S: Scalar>(&self, x: &[S], out: &mut [S]);
}

/// A dense row-major `m x n` Jacobian.
#[derive(Debug, Clone, PartialEq)]
pub struct Jacobian {
    pub m: usize,
    pub n: usize,
    pub data: Vec<f64>,
}

impl Jacobian {
    pub fn zeros(m: usize, n: usize) -> Self {
        Self { m, n, data: vec![0.0; m * n] }
    }
    pub fn at(&self, i: usize, j: usize) -> f64 {
        self.data[i * self.n + j]
    }
    /// `J v`.
    pub fn apply(&self, v: &[f64]) -> Vec<f64> {
        (0..self.m).map(|i| (0..self.n).map(|j| self.at(i, j) * v[j]).sum()).collect()
    }
    /// `J^T w`.
    pub fn apply_transpose(&self, w: &[f64]) -> Vec<f64> {
        (0..self.n).map(|j| (0..self.m).map(|i| self.at(i, j) * w[i]).sum()).collect()
    }
    /// Largest absolute entry difference to `o`, relative to the larger of the two largest entries (and 1e-300).
    pub fn relative_difference(&self, o: &Jacobian) -> f64 {
        let scale = self.data.iter().chain(&o.data).fold(0.0f64, |m, v| m.max(v.abs())).max(1e-300);
        self.data.iter().zip(&o.data).fold(0.0f64, |m, (a, b)| m.max((a - b).abs())) / scale
    }
}

/// Plain evaluation.
pub fn evaluate<K: SmoothKernel>(k: &K, x: &[f64]) -> Vec<f64> {
    let mut out = vec![0.0; k.n_out()];
    k.eval(x, &mut out);
    out
}

// ------------------------------------------------------------------------------------------- forward mode

fn jacobian_chunk<K: SmoothKernel, const W: usize>(k: &K, x: &[f64], jac: &mut Jacobian) {
    let (n, m) = (k.n_in(), k.n_out());
    let mut xs = vec![Dual::<f64, W>::cst(0.0); n];
    let mut out = vec![Dual::<f64, W>::cst(0.0); m];
    let mut start = 0;
    while start < n {
        for (i, xi) in xs.iter_mut().enumerate() {
            let mut d = [0.0; W];
            if i >= start && i < start + W {
                d[i - start] = 1.0;
            }
            *xi = Dual::with(x[i], d);
        }
        k.eval(&xs, &mut out);
        for (r, o) in out.iter().enumerate() {
            for j in 0..W.min(n - start) {
                jac.data[r * n + start + j] = o.d[j];
            }
        }
        start += W;
    }
}

/// Number of derivative directions one chunk of the forward Jacobian carries for `n` inputs.
pub fn forward_chunk_width(n: usize) -> usize {
    match n {
        0 | 1 => 1,
        2 => 2,
        3 | 4 => 4,
        _ => 8,
    }
}

/// Jacobian by chunked forward mode: `ceil(n / W)` evaluations of the kernel, the derivative directions of a chunk stored inline.
pub fn jacobian_forward<K: SmoothKernel>(k: &K, x: &[f64]) -> Jacobian {
    let mut jac = Jacobian::zeros(k.n_out(), k.n_in());
    match forward_chunk_width(k.n_in()) {
        1 => jacobian_chunk::<K, 1>(k, x, &mut jac),
        2 => jacobian_chunk::<K, 2>(k, x, &mut jac),
        4 => jacobian_chunk::<K, 4>(k, x, &mut jac),
        _ => jacobian_chunk::<K, 8>(k, x, &mut jac),
    }
    jac
}

/// Jacobian-vector product `J v` in one forward evaluation.
pub fn jvp<K: SmoothKernel>(k: &K, x: &[f64], v: &[f64]) -> Vec<f64> {
    let xs: Vec<Dual<f64, 1>> = x.iter().zip(v).map(|(&a, &b)| Dual::with(a, [b])).collect();
    let mut out = vec![Dual::<f64, 1>::cst(0.0); k.n_out()];
    k.eval(&xs, &mut out);
    out.iter().map(|o| o.d[0]).collect()
}

// ------------------------------------------------------------------------------------------- reverse mode

/// Reusable memory of the reverse-mode operators: one tape, kept between evaluations.
pub struct DerivativeWorkspace {
    pub tape: Tape<f64>,
}

impl DerivativeWorkspace {
    /// A workspace whose tape holds at most `max_ops` recorded operations.
    pub fn new(max_ops: usize) -> Self {
        Self { tape: Tape::with_limit(max_ops) }
    }
}

/// What a reverse evaluation cost.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TapeUse {
    pub ops: usize,
    pub bytes: usize,
}

/// Jacobian by reverse mode: one recording of the kernel, `m` sweeps.
pub fn jacobian_reverse<K: SmoothKernel>(k: &K, x: &[f64], ws: &mut DerivativeWorkspace) -> Result<(Jacobian, TapeUse), TapeError> {
    ws.tape.reset();
    let (n, m) = (k.n_in(), k.n_out());
    let vars: Vec<Rev<f64>> = x.iter().map(|&v| ws.tape.var(v)).collect();
    let mut out = vec![Rev::<f64>::cst(0.0); m];
    k.eval(&vars, &mut out);
    ws.tape.check()?;
    let mut jac = Jacobian::zeros(m, n);
    for (r, o) in out.iter().enumerate() {
        let adj = ws.tape.sweep(o, 1.0);
        for (j, v) in vars.iter().enumerate() {
            jac.data[r * n + j] = v.index().map_or(0.0, |i| adj[i]);
        }
    }
    Ok((jac, TapeUse { ops: ws.tape.len(), bytes: ws.tape.bytes() }))
}

/// Vector-Jacobian product `J^T w` in one recording and one sweep.
pub fn vjp<K: SmoothKernel>(k: &K, x: &[f64], w: &[f64], ws: &mut DerivativeWorkspace) -> Result<(Vec<f64>, TapeUse), TapeError> {
    ws.tape.reset();
    let vars: Vec<Rev<f64>> = x.iter().map(|&v| ws.tape.var(v)).collect();
    let mut out = vec![Rev::<f64>::cst(0.0); k.n_out()];
    k.eval(&vars, &mut out);
    let mut s = Rev::<f64>::cst(0.0);
    for (o, &wi) in out.iter().zip(w) {
        s = s + *o * Rev::cst(wi);
    }
    ws.tape.check()?;
    let adj = ws.tape.sweep(&s, 1.0);
    Ok((vars.iter().map(|v| v.index().map_or(0.0, |i| adj[i])).collect(), TapeUse { ops: ws.tape.len(), bytes: ws.tape.bytes() }))
}

// ------------------------------------------------------------------------------------------- second order

/// Hessian-vector product of `w^T f` by forward-over-reverse: a tape of dual values, swept once. `H v` for `f` scalar and `w = 1`.
pub fn hvp_forward_over_reverse<K: SmoothKernel>(k: &K, x: &[f64], w: &[f64], v: &[f64], max_ops: usize) -> Result<Vec<f64>, TapeError> {
    type D = Dual<f64, 1>;
    let tape: Tape<D> = Tape::with_limit(max_ops);
    let vars: Vec<Rev<D>> = x.iter().zip(v).map(|(&a, &b)| tape.var(D::with(a, [b]))).collect();
    let mut out = vec![Rev::<D>::cst(0.0); k.n_out()];
    k.eval(&vars, &mut out);
    let mut s = Rev::<D>::cst(0.0);
    for (o, &wi) in out.iter().zip(w) {
        s = s + *o * Rev::cst(wi);
    }
    tape.check()?;
    let adj = tape.sweep(&s, D::cst(1.0));
    Ok(vars.iter().map(|q| q.index().map_or(0.0, |i| adj[i].d[0])).collect())
}

/// Hessian-vector product of `w^T f` by reverse-over-forward: the directional derivative `w^T J v` is itself recorded on a tape and
/// swept. Same result as [`hvp_forward_over_reverse`]; the cost trade is memory (the tape holds the dual arithmetic).
pub fn hvp_reverse_over_forward<K: SmoothKernel>(k: &K, x: &[f64], w: &[f64], v: &[f64], max_ops: usize) -> Result<Vec<f64>, TapeError> {
    let tape: Tape<f64> = Tape::with_limit(max_ops);
    type D<'t> = Dual<Rev<'t, f64>, 1>;
    let vars: Vec<Rev<f64>> = x.iter().map(|&a| tape.var(a)).collect();
    let xs: Vec<D> = vars.iter().zip(v).map(|(&a, &b)| Dual::with(a, [Rev::cst(b)])).collect();
    let mut out = vec![D::cst(0.0); k.n_out()];
    k.eval(&xs, &mut out);
    let mut s = Rev::<f64>::cst(0.0);
    for (o, &wi) in out.iter().zip(w) {
        s = s + o.d[0] * Rev::cst(wi);
    }
    tape.check()?;
    let adj = tape.sweep(&s, 1.0);
    Ok(vars.iter().map(|q| q.index().map_or(0.0, |i| adj[i])).collect())
}

/// Mixed second derivative `d^2 f_r / dx_i dx_j` of every output by hyper-dual numbers (one evaluation, exact).
pub fn mixed_second_hyperdual<K: SmoothKernel>(k: &K, x: &[f64], i: usize, j: usize) -> Vec<f64> {
    let xs: Vec<HyperDual> = x.iter().enumerate().map(|(a, &v)| HyperDual::seeded(v, a == i, a == j)).collect();
    let mut out = vec![HyperDual::cst(0.0); k.n_out()];
    k.eval(&xs, &mut out);
    out.iter().map(|o| o.f12).collect()
}

/// Dense Hessian of output `r` by hyper-dual numbers: `n (n + 1) / 2` evaluations. For small systems and verification only; the
/// global Hessian is never formed.
pub fn hessian_hyperdual<K: SmoothKernel>(k: &K, x: &[f64], r: usize) -> Vec<f64> {
    let n = k.n_in();
    let mut h = vec![0.0; n * n];
    for i in 0..n {
        for j in i..n {
            let v = mixed_second_hyperdual(k, x, i, j)[r];
            h[i * n + j] = v;
            h[j * n + i] = v;
        }
    }
    h
}

// ------------------------------------------------------------------------------------------- complex step and references

/// Jacobian by the complex step `Im f(x + i h e_j) / h`. Only for kernels analytic at `x` (see [`super::verify::check_complex_step`]).
pub fn jacobian_complex_step<K: SmoothKernel>(k: &K, x: &[f64], h: f64) -> Jacobian {
    let (n, m) = (k.n_in(), k.n_out());
    let mut jac = Jacobian::zeros(m, n);
    let mut xs: Vec<Complex> = x.iter().map(|&v| Complex::cst(v)).collect();
    let mut out = vec![Complex::cst(0.0); m];
    for j in 0..n {
        xs[j] = Complex::new(x[j], h);
        k.eval(&xs, &mut out);
        for (r, o) in out.iter().enumerate() {
            jac.data[r * n + j] = o.im / h;
        }
        xs[j] = Complex::cst(x[j]);
    }
    jac
}

/// Central finite differences, step `h_rel * max(|x_j|, 1)`. A reference for verification, never a production derivative.
pub fn jacobian_central_difference<K: SmoothKernel>(k: &K, x: &[f64], h_rel: f64) -> Jacobian {
    let (n, m) = (k.n_in(), k.n_out());
    let mut jac = Jacobian::zeros(m, n);
    let mut xp = x.to_vec();
    for j in 0..n {
        let h = h_rel * x[j].abs().max(1.0);
        xp[j] = x[j] + h;
        let fp = evaluate(k, &xp);
        xp[j] = x[j] - h;
        let fm = evaluate(k, &xp);
        xp[j] = x[j];
        for r in 0..m {
            jac.data[r * n + j] = (fp[r] - fm[r]) / (2.0 * h);
        }
    }
    jac
}
