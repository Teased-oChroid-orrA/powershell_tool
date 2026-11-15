//! The unilateral contact problem itself (Story G): `K u = f + C^T lambda`, `g = g0 + C u >= 0`, `lambda >= 0`, `lambda_i g_i = 0`,
//! solved by a primal-dual active-set method (the semismooth Newton method for `min(lambda, c g) = 0`) with a dual-QP fallback, and
//! differentiated with the right derivative notion at each kind of point.
//!
//! * **strictly complementary** points (`lambda_i > 0 = g_i` or `g_i > 0 = lambda_i`): the solution is differentiable and its
//!   derivative solves the KKT system of the active set ([`ObstacleAnalysis::sensitivity`] with no biactive point);
//! * **biactive** points (`lambda_i = 0 = g_i`): the solution is only *directionally* differentiable. The directional derivative in
//!   the direction `+dp` solves a linear complementarity problem on the biactive set; the one in `-dp` is a different problem, and
//!   the two disagree exactly when the solution is not differentiable (`differentiable = false`). Each is an element of the
//!   generalized (Clarke) derivative and is labelled as such.
//!
//! The regularised law ([`SoftplusLaw`], Scalar-generic so its derivative comes from AD) replaces the complementarity condition by a
//! smooth penalty `p(g) = eps rho ln(1 + exp(-g / rho))`; [`RegularisedObstacle`] solves and differentiates it, so the distinction
//! between the regularised and the original law is something measured, not asserted.

use super::constitutive::{lu_solve, ConstitutiveError};
use super::dual::Dual;
use super::linalg::TangentSolver;
use super::policy::DerivativeSemantics;
use super::scalar::Scalar;
use crate::assembly::{BlockMatrix, Pattern};
use crate::linear::Reduced;
use std::cell::RefCell;

/// `g = g0 + sum coef * u[dof]`.
#[derive(Debug, Clone, PartialEq)]
pub struct Constraint {
    pub terms: Vec<(usize, f64)>,
    pub g0: f64,
}

impl Constraint {
    pub fn gap(&self, u: &[f64]) -> f64 {
        self.g0 + self.terms.iter().map(|&(d, c)| c * u[d]).sum::<f64>()
    }
    fn dot(&self, v: &[f64]) -> f64 {
        self.terms.iter().map(|&(d, c)| c * v[d]).sum()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Complementarity {
    /// `lambda > 0`, `g = 0`.
    Active,
    /// `lambda = 0`, `g > 0`.
    Inactive,
    /// `lambda = 0 = g`.
    Biactive,
}

#[derive(Debug, Clone)]
pub struct ObstacleSolution {
    pub u: Vec<f64>,
    pub lambda: Vec<f64>,
    pub gap: Vec<f64>,
    pub status: Vec<Complementarity>,
    pub iterations: usize,
    /// Number of constraints whose active/inactive membership changed at each iteration.
    pub changes: Vec<usize>,
    /// The primal-dual iteration cycled and the dual problem was solved instead.
    pub used_dual_fallback: bool,
    /// Largest violation among: equilibrium residual (relative), negative multiplier, penetration, and `lambda * g` (relative).
    pub kkt_violation: f64,
}

impl ObstacleSolution {
    pub fn count(&self, s: Complementarity) -> usize {
        self.status.iter().filter(|&&x| x == s).count()
    }
}

/// What a sensitivity analysis found.
#[derive(Debug, Clone)]
pub struct ObstacleSensitivity {
    pub du: Vec<f64>,
    pub dlambda: Vec<f64>,
    pub semantics: DerivativeSemantics,
    pub biactive: usize,
    /// `false` when the one-sided derivatives in `+dp` and `-dp` differ: the solution has a kink in this parameter.
    pub differentiable: bool,
    /// The derivative in the opposite direction `-dp`, when the solution is not differentiable (negated `du` otherwise equal).
    pub du_opposite: Option<Vec<f64>>,
}

#[derive(Debug, Clone, Copy)]
pub struct ObstacleOptions {
    /// Scale `c` of the natural function `min(lambda, c g)`.
    pub c: f64,
    pub max_iter: usize,
    /// Relative tolerance of the status classification.
    pub tol: f64,
}

impl Default for ObstacleOptions {
    fn default() -> Self {
        Self { c: 1.0, max_iter: 60, tol: 1e-9 }
    }
}

/// Projected successive over-relaxation for `min 1/2 x^T M x + q^T x, x >= 0` with `M` symmetric positive (semi)definite: the solution
/// of the linear complementarity problem `x >= 0, M x + q >= 0, x_i (M x + q)_i = 0`. Convergent for SPD `M`.
pub fn psor(m: &[f64], q: &[f64], n: usize, tol: f64, max_iter: usize) -> (Vec<f64>, usize) {
    let mut x = vec![0.0; n];
    let scale = q.iter().fold(0.0f64, |a, v| a.max(v.abs())).max(1e-300);
    for it in 0..max_iter {
        let mut change = 0.0f64;
        for i in 0..n {
            let mut r = q[i];
            for j in 0..n {
                r += m[i * n + j] * x[j];
            }
            let new = (x[i] - r / m[i * n + i]).max(0.0);
            change = change.max((new - x[i]).abs());
            x[i] = new;
        }
        if change <= tol * scale / m.iter().fold(0.0f64, |a, v| a.max(v.abs())).max(1e-300) {
            return (x, it + 1);
        }
    }
    (x, max_iter)
}

pub struct ObstacleAnalysis<'a> {
    solver: TangentSolver<'a>,
    n_total: usize,
    free: &'a [usize],
    constraints: Vec<Constraint>,
    u0: Vec<f64>,
    columns: RefCell<Vec<Option<Vec<f64>>>>,
}

impl<'a> ObstacleAnalysis<'a> {
    /// `f` is the full load vector; constrained dofs of `red` keep zero displacement.
    pub fn new(red: &'a Reduced, pattern: &'a Pattern, k: &'a BlockMatrix, n_total: usize, free: &'a [usize], f: &[f64], constraints: Vec<Constraint>) -> Result<Self, String> {
        let solver = TangentSolver::new(red, pattern, k, n_total, free, &[]).map_err(|e| e.to_string())?;
        let rhs: Vec<f64> = free.iter().map(|&d| f[d]).collect();
        let x = solver.solve(&rhs)?;
        let mut u0 = vec![0.0; n_total];
        for (j, &d) in free.iter().enumerate() {
            u0[d] = x[j];
        }
        let m = constraints.len();
        Ok(Self { solver, n_total, free, constraints, u0, columns: RefCell::new(vec![None; m]) })
    }

    pub fn constraints(&self) -> &[Constraint] {
        &self.constraints
    }

    fn expand(&self, x: &[f64]) -> Vec<f64> {
        let mut v = vec![0.0; self.n_total];
        for (j, &d) in self.free.iter().enumerate() {
            v[d] = x[j];
        }
        v
    }

    /// `K^-1` of a full-length vector (constrained dofs ignored).
    pub fn k_inverse(&self, v: &[f64]) -> Result<Vec<f64>, String> {
        let rhs: Vec<f64> = self.free.iter().map(|&d| v[d]).collect();
        Ok(self.expand(&self.solver.solve(&rhs)?))
    }

    /// `W_i = K^-1 C_i^T`, computed once.
    fn column(&self, i: usize) -> Result<Vec<f64>, String> {
        if let Some(c) = &self.columns.borrow()[i] {
            return Ok(c.clone());
        }
        let mut v = vec![0.0; self.n_total];
        for &(d, c) in &self.constraints[i].terms {
            v[d] += c;
        }
        let w = self.k_inverse(&v)?;
        self.columns.borrow_mut()[i] = Some(w.clone());
        Ok(w)
    }

    /// `S_ab = C_a . W_b` for index sets `a`, `b` (row-major `|a| x |b|`).
    fn schur(&self, a: &[usize], b: &[usize]) -> Result<Vec<f64>, String> {
        let wb: Vec<Vec<f64>> = b.iter().map(|&j| self.column(j)).collect::<Result<_, _>>()?;
        Ok(a.iter().flat_map(|&i| wb.iter().map(move |w| self.constraints[i].dot(w))).collect())
    }

    fn assemble_u(&self, lambda: &[f64], active: &[usize]) -> Result<Vec<f64>, String> {
        let mut u = self.u0.clone();
        for &i in active {
            let w = self.column(i)?;
            for (a, b) in u.iter_mut().zip(&w) {
                *a += lambda[i] * b;
            }
        }
        Ok(u)
    }

    fn finish(&self, lambda: Vec<f64>, active: &[usize], opt: &ObstacleOptions, iterations: usize, changes: Vec<usize>, fallback: bool, f: Option<&[f64]>) -> Result<ObstacleSolution, String> {
        let u = self.assemble_u(&lambda, active)?;
        let gap: Vec<f64> = self.constraints.iter().map(|c| c.gap(&u)).collect();
        let lmax = lambda.iter().cloned().fold(0.0f64, f64::max);
        let gscale = self.constraints.iter().map(|c| c.g0.abs()).chain(gap.iter().map(|g| g.abs())).fold(0.0f64, f64::max).max(1e-300);
        let (lt, gt) = (opt.tol * lmax.max(1e-300), opt.tol * gscale);
        let status: Vec<Complementarity> = (0..lambda.len())
            .map(|i| match (lambda[i] > lt, gap[i].abs() <= gt) {
                (true, _) => Complementarity::Active,
                (false, true) => Complementarity::Biactive,
                (false, false) => Complementarity::Inactive,
            })
            .collect();
        let mut v = 0.0f64;
        for i in 0..lambda.len() {
            v = v.max((-lambda[i]).max(0.0) / lmax.max(1e-300)).max((-gap[i]).max(0.0) / gscale).max((lambda[i] * gap[i]).abs() / (lmax.max(1e-300) * gscale));
        }
        let _ = f;
        Ok(ObstacleSolution { u, lambda, gap, status, iterations, changes, used_dual_fallback: fallback, kkt_violation: v })
    }

    /// Solve the unilateral problem by the primal-dual active-set method.
    pub fn solve(&self, opt: &ObstacleOptions) -> Result<ObstacleSolution, String> {
        let m = self.constraints.len();
        let g_of = |u: &[f64]| -> Vec<f64> { self.constraints.iter().map(|c| c.gap(u)).collect() };
        let mut active: Vec<usize> = (0..m).filter(|&i| self.constraints[i].gap(&self.u0) < 0.0).collect();
        let mut seen: Vec<Vec<usize>> = Vec::new();
        let mut changes = Vec::new();
        for it in 0..opt.max_iter {
            let mut lambda = vec![0.0; m];
            if !active.is_empty() {
                let s = self.schur(&active, &active).map_err(|e| e.to_string())?;
                let rhs: Vec<f64> = active.iter().map(|&i| -self.constraints[i].gap(&self.u0)).collect();
                let l = lu_solve(active.len(), 1, &s, &rhs).map_err(|e: ConstitutiveError| e.to_string())?;
                for (k, &i) in active.iter().enumerate() {
                    lambda[i] = l[k];
                }
            }
            let u = self.assemble_u(&lambda, &active)?;
            let gap = g_of(&u);
            let next: Vec<usize> = (0..m).filter(|&i| lambda[i] - opt.c * gap[i] > 0.0).collect();
            let diff = next.iter().filter(|i| !active.contains(i)).count() + active.iter().filter(|i| !next.contains(i)).count();
            changes.push(diff);
            if next == active {
                return self.finish(lambda, &active, opt, it + 1, changes, false, None);
            }
            if seen.contains(&next) {
                break;
            }
            seen.push(active);
            active = next;
        }
        // cycling or the budget: the dual problem min 1/2 lambda^T S lambda + (g0 + C u0)^T lambda, lambda >= 0 is convex and PSOR converges
        let all: Vec<usize> = (0..m).collect();
        let s = self.schur(&all, &all)?;
        let q: Vec<f64> = (0..m).map(|i| self.constraints[i].gap(&self.u0)).collect();
        let (lambda, _) = psor(&s, &q, m, 1e-14, 200_000);
        let act: Vec<usize> = (0..m).filter(|&i| lambda[i] > 0.0).collect();
        self.finish(lambda, &act, opt, opt.max_iter, changes, true, None)
    }

    /// Sensitivity of the solution to a parameter `p` whose derivatives are `rhs = df/dp - (dK/dp) u` (full vector) and `dg0` (per
    /// constraint). With no biactive constraint this is the classical derivative of the active branch; with biactive ones it is the
    /// directional derivative in `+dp` (a linear complementarity problem), and `du_opposite` the one in `-dp`.
    pub fn sensitivity(&self, sol: &ObstacleSolution, rhs: &[f64], dg0: &[f64]) -> Result<ObstacleSensitivity, String> {
        let m = self.constraints.len();
        let a: Vec<usize> = (0..m).filter(|&i| sol.status[i] == Complementarity::Active).collect();
        let b: Vec<usize> = (0..m).filter(|&i| sol.status[i] == Complementarity::Biactive).collect();
        let one_direction = |sign: f64| -> Result<(Vec<f64>, Vec<f64>), String> {
            let r: Vec<f64> = rhs.iter().map(|v| sign * v).collect();
            let u1 = self.k_inverse(&r)?;
            let c_u1 = |i: usize| self.constraints[i].dot(&u1);
            // A-equations: S_AA dl_A + S_AB dl_B = -dg0_A - C_A u1
            let (saa, sab) = (self.schur(&a, &a)?, self.schur(&a, &b)?);
            let rhs_a: Vec<f64> = a.iter().map(|&i| -sign * dg0[i] - c_u1(i)).collect();
            // dl_A = S_AA^-1 (rhs_a - S_AB dl_B): solve for the columns we need
            let sinv_rhs = if a.is_empty() { Vec::new() } else { lu_solve(a.len(), 1, &saa, &rhs_a).map_err(|e| e.to_string())? };
            let sinv_sab = if a.is_empty() || b.is_empty() { Vec::new() } else { lu_solve(a.len(), b.len(), &saa, &sab).map_err(|e| e.to_string())? };
            let mut dlb = vec![0.0; b.len()];
            if !b.is_empty() {
                // dg_B = q + M dl_B with M = S_BB - S_BA S_AA^-1 S_AB, q = C_B u1 + dg0_B + S_BA S_AA^-1 (-dg0_A - C_A u1)
                let sbb = self.schur(&b, &b)?;
                let sba = self.schur(&b, &a)?;
                let nb = b.len();
                let mut mm = sbb.clone();
                for i in 0..nb {
                    for j in 0..nb {
                        let mut s = 0.0;
                        for k in 0..a.len() {
                            s += sba[i * a.len() + k] * sinv_sab[k * nb + j];
                        }
                        mm[i * nb + j] -= s;
                    }
                }
                let q: Vec<f64> = (0..nb)
                    .map(|i| {
                        let mut v = c_u1(b[i]) + sign * dg0[b[i]];
                        for k in 0..a.len() {
                            v += sba[i * a.len() + k] * sinv_rhs[k];
                        }
                        v
                    })
                    .collect();
                let (x, _) = psor(&mm, &q, nb, 1e-15, 100_000);
                dlb = x;
            }
            let mut dla = sinv_rhs.clone();
            for k in 0..a.len() {
                for j in 0..b.len() {
                    dla[k] -= sinv_sab[k * b.len() + j] * dlb[j];
                }
            }
            let mut du = u1;
            for (k, &i) in a.iter().enumerate() {
                let w = self.column(i)?;
                for (x, y) in du.iter_mut().zip(&w) {
                    *x += dla[k] * y;
                }
            }
            for (k, &i) in b.iter().enumerate() {
                let w = self.column(i)?;
                for (x, y) in du.iter_mut().zip(&w) {
                    *x += dlb[k] * y;
                }
            }
            let mut dl = vec![0.0; m];
            for (k, &i) in a.iter().enumerate() {
                dl[i] = dla[k];
            }
            for (k, &i) in b.iter().enumerate() {
                dl[i] = dlb[k];
            }
            Ok((du, dl))
        };
        let (du, dlambda) = one_direction(1.0)?;
        if b.is_empty() {
            return Ok(ObstacleSensitivity { du, dlambda, semantics: DerivativeSemantics::ActiveBranch, biactive: 0, differentiable: true, du_opposite: None });
        }
        let (dum, _) = one_direction(-1.0)?;
        let scale = du.iter().chain(&dum).fold(0.0f64, |m, v| m.max(v.abs())).max(1e-300);
        let differentiable = du.iter().zip(&dum).all(|(p, q)| (p + q).abs() <= 1e-9 * scale);
        Ok(ObstacleSensitivity { du, dlambda, semantics: DerivativeSemantics::Generalized, biactive: b.len(), differentiable, du_opposite: Some(dum.iter().map(|v| -v).collect()) })
    }

    /// Adjoint sensitivities of `J = grad . u` on the strictly active set (no biactive constraint): `dJ/dp = psi . rhs - mu . dg0_A`
    /// with `psi = K^-1 grad - W_A mu` and `S_AA mu = C_A K^-1 grad`; the adjoint of the KKT system, shared by every parameter.
    pub fn adjoint(&self, sol: &ObstacleSolution, grad: &[f64]) -> Result<AdjointFields, String> {
        if sol.count(Complementarity::Biactive) > 0 {
            return Err("an adjoint needs a differentiable branch: there are biactive constraints (use the directional derivatives)".into());
        }
        let a: Vec<usize> = (0..self.constraints.len()).filter(|&i| sol.status[i] == Complementarity::Active).collect();
        let v = self.k_inverse(grad)?;
        let mut psi = v.clone();
        let mut mu = vec![0.0; self.constraints.len()];
        if !a.is_empty() {
            let s = self.schur(&a, &a)?;
            let rhs: Vec<f64> = a.iter().map(|&i| self.constraints[i].dot(&v)).collect();
            let m = lu_solve(a.len(), 1, &s, &rhs).map_err(|e| e.to_string())?;
            for (k, &i) in a.iter().enumerate() {
                mu[i] = m[k];
                let w = self.column(i)?;
                for (x, y) in psi.iter_mut().zip(&w) {
                    *x -= m[k] * y;
                }
            }
        }
        Ok(AdjointFields { psi, mu })
    }
}

/// The adjoint fields of a quantity of interest on a fixed active set.
#[derive(Debug, Clone)]
pub struct AdjointFields {
    pub psi: Vec<f64>,
    pub mu: Vec<f64>,
}

impl AdjointFields {
    /// `dJ/dp` for the parameter with `rhs = df/dp - (dK/dp) u` and gap derivatives `dg0`.
    pub fn gradient(&self, rhs: &[f64], dg0: &[f64]) -> f64 {
        self.psi.iter().zip(rhs).map(|(a, b)| a * b).sum::<f64>() - self.mu.iter().zip(dg0).map(|(a, b)| a * b).sum::<f64>()
    }
}

// ---------------------------------------------------------------------------------------------------------------- regularised law

/// Contact pressure of the softplus law, `p(g) = eps rho ln(1 + exp(-g / rho))`: smooth, convex, tends to `eps max(0, -g)` as `rho -> 0`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SoftplusLaw {
    pub eps: f64,
    pub rho: f64,
}

impl SoftplusLaw {
    /// The pressure from the gap, for any scalar (so its derivative is exact by AD).
    pub fn pressure<S: Scalar>(&self, g: S) -> S {
        // stable form: ln(1 + e^x) = max(x, 0) + ln(1 + e^-|x|)
        let x = -(g.scale(1.0 / self.rho));
        let pos = x.max(S::cst(0.0));
        (pos + (S::cst(1.0) + (-(x.abs())).exp()).ln()).scale(self.eps * self.rho)
    }

    /// `(p, dp/dg)`.
    pub fn with_derivative(&self, g: f64) -> (f64, f64) {
        let d = self.pressure(Dual::<f64, 1>::var(g, 0));
        (d.v, d.d[0])
    }
}

#[derive(Debug, Clone)]
pub struct RegularisedSolution {
    pub u: Vec<f64>,
    pub gap: Vec<f64>,
    pub pressure: Vec<f64>,
    pub newton_iterations: usize,
    /// Largest penetration `max(0, -g)`: how far the regularised law departs from the unilateral condition.
    pub max_penetration: f64,
}

pub struct RegularisedObstacle<'a> {
    red: &'a Reduced,
    pattern: &'a Pattern,
    k: &'a BlockMatrix,
    n_total: usize,
    free: &'a [usize],
    f: Vec<f64>,
    constraints: Vec<Constraint>,
    pub law: SoftplusLaw,
}

impl<'a> RegularisedObstacle<'a> {
    pub fn new(red: &'a Reduced, pattern: &'a Pattern, k: &'a BlockMatrix, n_total: usize, free: &'a [usize], f: Vec<f64>, constraints: Vec<Constraint>, law: SoftplusLaw) -> Self {
        Self { red, pattern, k, n_total, free, f, constraints, law }
    }

    fn blocks(&self, gaps: &[f64]) -> Vec<(Vec<usize>, Vec<f64>)> {
        // d(-C^T p)/du = -p'(g) C^T C: rank-one dense blocks over each constraint's dofs
        self.constraints
            .iter()
            .zip(gaps)
            .map(|(c, &g)| {
                let (_, dp) = self.law.with_derivative(g);
                let dofs: Vec<usize> = c.terms.iter().map(|t| t.0).collect();
                let nt = dofs.len();
                let mut m = vec![0.0; nt * nt];
                for i in 0..nt {
                    for j in 0..nt {
                        m[i * nt + j] = -dp * c.terms[i].1 * c.terms[j].1;
                    }
                }
                (dofs, m)
            })
            .collect()
    }

    /// Residual, gaps and the force scale (largest elastic, applied or contact force on a free dof) the residual is judged against.
    fn residual(&self, u: &[f64]) -> (Vec<f64>, Vec<f64>, f64) {
        let mut ku = vec![0.0; self.n_total];
        self.k.matvec_add(self.pattern, u, &mut ku);
        let gaps: Vec<f64> = self.constraints.iter().map(|c| c.gap(u)).collect();
        let mut scale = self.free.iter().fold(0.0f64, |m, &d| m.max(ku[d].abs()).max(self.f[d].abs()));
        let mut r: Vec<f64> = ku.iter().zip(&self.f).map(|(a, b)| a - b).collect();
        for (c, &g) in self.constraints.iter().zip(&gaps) {
            let (p, _) = self.law.with_derivative(g);
            for &(d, coef) in &c.terms {
                r[d] -= coef * p;
                scale = scale.max((coef * p).abs());
            }
        }
        (r, gaps, scale.max(1e-300))
    }

    pub fn solve(&self, tol: f64, max_iter: usize) -> Result<RegularisedSolution, String> {
        let mut u = vec![0.0; self.n_total];
        for it in 0..=max_iter {
            let (r, gaps, fscale) = self.residual(&u);
            let rn = self.free.iter().fold(0.0f64, |m, &d| m.max(r[d].abs()));
            if rn <= tol * fscale {
                let pressure = gaps.iter().map(|&g| self.law.with_derivative(g).0).collect();
                let max_penetration = gaps.iter().fold(0.0f64, |m, &g| m.max(-g));
                return Ok(RegularisedSolution { u, gap: gaps, pressure, newton_iterations: it, max_penetration });
            }
            if it == max_iter {
                break;
            }
            let blocks = self.blocks(&gaps);
            let solver = TangentSolver::new(self.red, self.pattern, self.k, self.n_total, self.free, &blocks).map_err(|e| e.to_string())?;
            let dx = solver.solve(&self.free.iter().map(|&d| r[d]).collect::<Vec<_>>())?;
            let mut alpha = 1.0;
            loop {
                let mut t = u.clone();
                for (j, &d) in self.free.iter().enumerate() {
                    t[d] -= alpha * dx[j];
                }
                let (rt, _, _) = self.residual(&t);
                let rtn = self.free.iter().fold(0.0f64, |m, &d| m.max(rt[d].abs()));
                if rtn < rn || alpha < 1e-3 {
                    u = t;
                    break;
                }
                alpha *= 0.5;
            }
        }
        Err("the regularised contact Newton iteration did not converge".into())
    }

    /// `du/dp` for `rhs = df/dp - (dK/dp) u` and `dg0/dp`: `(K - C^T p' C) du = rhs + C^T p' dg0`.
    pub fn sensitivity(&self, sol: &RegularisedSolution, rhs: &[f64], dg0: &[f64]) -> Result<Vec<f64>, String> {
        let blocks = self.blocks(&sol.gap);
        let solver = TangentSolver::new(self.red, self.pattern, self.k, self.n_total, self.free, &blocks).map_err(|e| e.to_string())?;
        let mut b = rhs.to_vec();
        for (c, (&g, &d0)) in self.constraints.iter().zip(sol.gap.iter().zip(dg0)) {
            let (_, dp) = self.law.with_derivative(g);
            for &(d, coef) in &c.terms {
                b[d] += coef * dp * d0;
            }
        }
        let x = solver.solve(&self.free.iter().map(|&d| b[d]).collect::<Vec<_>>())?;
        let mut du = vec![0.0; self.n_total];
        for (j, &d) in self.free.iter().enumerate() {
            du[d] = x[j];
        }
        Ok(du)
    }
}
