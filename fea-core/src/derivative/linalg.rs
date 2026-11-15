//! Linear operators and Krylov solvers for the sensitivity engine (Story F), and the tangent solver that both the optimisation
//! sensitivities and the goal-oriented adaptivity use for their adjoint equations.
//!
//! A [`LinearOperator`] acts on vectors over the **free** degrees of freedom and offers `A x` and `A^T x`. Implementations: the
//! assembled symmetric tangent (optionally plus the small unsymmetric blocks a frictional contact tangent or a follower load adds),
//! a Jacobian-vector-product operator built from closures (forward AD for `A x`, reverse AD or an analytic transpose for `A^T x`),
//! and the matrix-free finite-element tangent in [`super::matrix_free`]. The Krylov solvers (CG for symmetric positive definite,
//! restarted right-preconditioned GMRES for the rest) take any of them with any preconditioner closure.

use crate::assembly::{BlockMatrix, Pattern};
use crate::linear::{AnyFactor, Reduced, SolveError};
use std::cell::Cell;

pub trait LinearOperator {
    fn dim(&self) -> usize;
    /// `y = A x`.
    fn apply(&self, x: &[f64], y: &mut [f64]);
    /// `y = A^T x`.
    fn apply_transpose(&self, x: &[f64], y: &mut [f64]);
    fn is_symmetric(&self) -> bool {
        false
    }
}

/// Unsymmetric remainder of a tangent: dense blocks over small dof sets, `(dofs, row-major matrix)` (the layout of the contact and
/// follower-load defects in `nonlinear.rs`).
pub type DefectBlocks = [(Vec<usize>, Vec<f64>)];

/// The assembled tangent `K + D` over the free dofs: `K` symmetric (stored as the lower block triangle), `D` the unsymmetric blocks.
pub struct AssembledTangent<'a> {
    pub pattern: &'a Pattern,
    pub k: &'a BlockMatrix,
    pub free: &'a [usize],
    pub n_total: usize,
    pub defect: &'a DefectBlocks,
}

impl LinearOperator for AssembledTangent<'_> {
    fn dim(&self) -> usize {
        self.free.len()
    }
    fn apply(&self, x: &[f64], y: &mut [f64]) {
        let mut xf = vec![0.0; self.n_total];
        for (j, &i) in self.free.iter().enumerate() {
            xf[i] = x[j];
        }
        let mut yf = vec![0.0; self.n_total];
        self.k.matvec_add(self.pattern, &xf, &mut yf);
        for (dofs, mat) in self.defect {
            let nt = dofs.len();
            for i in 0..nt {
                let mut s = 0.0;
                for j in 0..nt {
                    s += mat[i * nt + j] * xf[dofs[j]];
                }
                yf[dofs[i]] += s;
            }
        }
        for (j, &i) in self.free.iter().enumerate() {
            y[j] = yf[i];
        }
    }
    fn apply_transpose(&self, x: &[f64], y: &mut [f64]) {
        let mut xf = vec![0.0; self.n_total];
        for (j, &i) in self.free.iter().enumerate() {
            xf[i] = x[j];
        }
        let mut yf = vec![0.0; self.n_total];
        self.k.matvec_add(self.pattern, &xf, &mut yf);
        for (dofs, mat) in self.defect {
            let nt = dofs.len();
            for i in 0..nt {
                for j in 0..nt {
                    yf[dofs[j]] += mat[i * nt + j] * xf[dofs[i]];
                }
            }
        }
        for (j, &i) in self.free.iter().enumerate() {
            y[j] = yf[i];
        }
    }
    fn is_symmetric(&self) -> bool {
        self.defect.is_empty()
    }
}

/// An operator from closures: `A x` (e.g. a forward-AD Jacobian-vector product) and `A^T x` (a vector-Jacobian product).
pub struct ClosureOperator<F: Fn(&[f64], &mut [f64]), G: Fn(&[f64], &mut [f64])> {
    pub n: usize,
    pub forward: F,
    pub transpose: G,
    pub symmetric: bool,
}

impl<F: Fn(&[f64], &mut [f64]), G: Fn(&[f64], &mut [f64])> LinearOperator for ClosureOperator<F, G> {
    fn dim(&self) -> usize {
        self.n
    }
    fn apply(&self, x: &[f64], y: &mut [f64]) {
        (self.forward)(x, y)
    }
    fn apply_transpose(&self, x: &[f64], y: &mut [f64]) {
        (self.transpose)(x, y)
    }
    fn is_symmetric(&self) -> bool {
        self.symmetric
    }
}

/// How a Krylov solve ended.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct KrylovReport {
    pub iterations: usize,
    /// `|b - A x| / |b|` at the end (true residual).
    pub rel_residual: f64,
    pub converged: bool,
}

fn dot(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

fn norm(a: &[f64]) -> f64 {
    dot(a, a).sqrt()
}

fn residual_norm(op: &dyn LinearOperator, transposed: bool, b: &[f64], x: &[f64]) -> f64 {
    let mut ax = vec![0.0; b.len()];
    if transposed {
        op.apply_transpose(x, &mut ax);
    } else {
        op.apply(x, &mut ax);
    }
    let r: Vec<f64> = b.iter().zip(&ax).map(|(a, c)| a - c).collect();
    norm(&r) / norm(b).max(1e-300)
}

/// Preconditioned conjugate gradients for a symmetric positive definite operator; `precond(r, z)` writes `z = M^-1 r`.
pub fn cg(op: &dyn LinearOperator, b: &[f64], x: &mut [f64], precond: &dyn Fn(&[f64], &mut [f64]), tol: f64, max_iter: usize) -> KrylovReport {
    let n = b.len();
    let bn = norm(b).max(1e-300);
    let mut ax = vec![0.0; n];
    op.apply(x, &mut ax);
    let mut r: Vec<f64> = b.iter().zip(&ax).map(|(a, c)| a - c).collect();
    let mut z = vec![0.0; n];
    precond(&r, &mut z);
    let mut p = z.clone();
    let mut rz = dot(&r, &z);
    let mut ap = vec![0.0; n];
    for it in 0..max_iter {
        if norm(&r) <= tol * bn {
            return KrylovReport { iterations: it, rel_residual: residual_norm(op, false, b, x), converged: true };
        }
        op.apply(&p, &mut ap);
        let alpha = rz / dot(&p, &ap);
        for i in 0..n {
            x[i] += alpha * p[i];
            r[i] -= alpha * ap[i];
        }
        precond(&r, &mut z);
        let rz_new = dot(&r, &z);
        let beta = rz_new / rz;
        rz = rz_new;
        for i in 0..n {
            p[i] = z[i] + beta * p[i];
        }
    }
    let rel = residual_norm(op, false, b, x);
    KrylovReport { iterations: max_iter, rel_residual: rel, converged: rel <= tol }
}

/// Restarted right-preconditioned GMRES for `A x = b` (or `A^T x = b` with `transposed`). `precond(v, z)` writes `z = M^-1 v` (it
/// may be the factorisation of the symmetric part of the operator).
#[allow(clippy::too_many_arguments)]
pub fn gmres(op: &dyn LinearOperator, transposed: bool, b: &[f64], x: &mut [f64], precond: &dyn Fn(&[f64], &mut [f64]), tol: f64, restart: usize, max_iter: usize) -> KrylovReport {
    let n = b.len();
    let bn = norm(b).max(1e-300);
    let apply = |v: &[f64], y: &mut [f64]| {
        if transposed {
            op.apply_transpose(v, y)
        } else {
            op.apply(v, y)
        }
    };
    let mut total = 0;
    let m = restart.max(2).min(n.max(2));
    while total < max_iter {
        let mut ax = vec![0.0; n];
        apply(x, &mut ax);
        let r0: Vec<f64> = b.iter().zip(&ax).map(|(a, c)| a - c).collect();
        let beta = norm(&r0);
        if beta <= tol * bn {
            return KrylovReport { iterations: total, rel_residual: beta / bn, converged: true };
        }
        let mut v: Vec<Vec<f64>> = vec![r0.iter().map(|t| t / beta).collect()];
        let mut z: Vec<Vec<f64>> = Vec::new();
        let mut h = vec![vec![0.0f64; m]; m + 1];
        let (mut cs, mut sn) = (vec![0.0f64; m], vec![0.0f64; m]);
        let mut g = vec![0.0f64; m + 1];
        g[0] = beta;
        let mut used = 0;
        for j in 0..m {
            let mut zj = vec![0.0; n];
            precond(&v[j], &mut zj);
            let mut w = vec![0.0; n];
            apply(&zj, &mut w);
            z.push(zj);
            for i in 0..=j {
                h[i][j] = dot(&w, &v[i]);
                for (wv, vv) in w.iter_mut().zip(&v[i]) {
                    *wv -= h[i][j] * vv;
                }
            }
            h[j + 1][j] = norm(&w);
            for i in 0..j {
                let t = cs[i] * h[i][j] + sn[i] * h[i + 1][j];
                h[i + 1][j] = -sn[i] * h[i][j] + cs[i] * h[i + 1][j];
                h[i][j] = t;
            }
            let denom = h[j][j].hypot(h[j + 1][j]);
            if denom == 0.0 || !denom.is_finite() {
                break;
            }
            cs[j] = h[j][j] / denom;
            sn[j] = h[j + 1][j] / denom;
            h[j][j] = denom;
            g[j + 1] = -sn[j] * g[j];
            g[j] *= cs[j];
            used = j + 1;
            total += 1;
            let breakdown = h[j + 1][j] < 1e-14 * beta;
            if g[j + 1].abs() <= tol * bn * 0.1 || breakdown || total >= max_iter {
                break;
            }
            let nxt: Vec<f64> = w.iter().map(|t| t / h[j + 1][j]).collect();
            v.push(nxt);
        }
        // back substitution
        let mut y = vec![0.0; used];
        for i in (0..used).rev() {
            let mut s = g[i];
            for k in i + 1..used {
                s -= h[i][k] * y[k];
            }
            y[i] = s / h[i][i];
        }
        for (i, yi) in y.iter().enumerate() {
            for (xv, zv) in x.iter_mut().zip(&z[i]) {
                *xv += yi * zv;
            }
        }
        if used == 0 {
            break;
        }
    }
    let rel = residual_norm(op, transposed, b, x);
    KrylovReport { iterations: total, rel_residual: rel, converged: rel <= tol }
}

/// Solves with the (possibly unsymmetric) tangent over the free dofs, direct and transposed, from one factorisation of its symmetric
/// part. With no unsymmetric blocks both directions are plain back-substitutions. With them, GMRES preconditioned by the factorisation
/// solves `(K + D) x = b` and `(K + D)^T x = b`: the transposed solve is what an adjoint through a frictional-contact or follower-load
/// tangent needs, and the same factorisation serves both.
pub struct TangentSolver<'a> {
    fac: AnyFactor<'a>,
    op: AssembledTangent<'a>,
    solves: Cell<usize>,
    krylov_iterations: Cell<usize>,
    /// Relative residual a Krylov solve must reach.
    pub tol: f64,
}

impl<'a> TangentSolver<'a> {
    /// Factor the symmetric part `k` over the reduced system `red`.
    pub fn new(red: &'a Reduced, pattern: &'a Pattern, k: &'a BlockMatrix, n_total: usize, free: &'a [usize], defect: &'a DefectBlocks) -> Result<Self, SolveError> {
        let fac = red.factor_any(k)?;
        Ok(Self { fac, op: AssembledTangent { pattern, k, free, n_total, defect }, solves: Cell::new(0), krylov_iterations: Cell::new(0), tol: 1e-12 })
    }

    pub fn n_solves(&self) -> usize {
        self.solves.get()
    }

    pub fn n_krylov_iterations(&self) -> usize {
        self.krylov_iterations.get()
    }

    pub fn dim(&self) -> usize {
        self.op.dim()
    }

    pub fn operator(&self) -> &AssembledTangent<'a> {
        &self.op
    }

    fn solve_with(&self, rhs: &[f64], transposed: bool) -> Result<Vec<f64>, String> {
        self.solves.set(self.solves.get() + 1);
        let mut x = rhs.to_vec();
        self.fac.solve_reduced(&mut x);
        if self.op.defect.is_empty() {
            return Ok(x);
        }
        let precond = |v: &[f64], z: &mut [f64]| {
            z.copy_from_slice(v);
            self.fac.solve_reduced(z);
        };
        let rep = gmres(&self.op, transposed, rhs, &mut x, &precond, self.tol, 60, 400);
        self.krylov_iterations.set(self.krylov_iterations.get() + rep.iterations);
        if rep.converged {
            Ok(x)
        } else {
            Err(format!("the {} tangent solve did not converge (relative residual {:.2e} after {} iterations)", if transposed { "adjoint" } else { "direct" }, rep.rel_residual, rep.iterations))
        }
    }

    /// `x = (K + D)^-1 rhs` (direct sensitivity equation).
    pub fn solve(&self, rhs: &[f64]) -> Result<Vec<f64>, String> {
        self.solve_with(rhs, false)
    }

    /// `x = (K + D)^-T rhs` (adjoint equation).
    pub fn solve_transpose(&self, rhs: &[f64]) -> Result<Vec<f64>, String> {
        self.solve_with(rhs, true)
    }
}
