//! Differentiable constitutive models and consistent algorithmic tangents (Story D).
//!
//! A model implements [`DifferentiableMaterial`]: a pure update `(parameters, strain, committed state) -> (stress, trial state)`
//! written against [`Scalar`], plus, for models whose update solves a local nonlinear system `F(q, x) = 0`, the residual of that
//! system and a converged f64 solution. The **trial** state is the update's return value; the **committed** state is the caller's,
//! replaced only by [`MaterialPoint::commit`], so a rejected step costs nothing.
//!
//! The tangent of the *implemented algorithm* is computed one of three ways, chosen by the model:
//! - explicit updates (a closed-form return map): forward AD of the update straight through;
//! - implicit updates (a local Newton solve): the solve runs in `f64` to convergence, then the converged system is differentiated
//!   implicitly, `dq/dx = -F_q^-1 F_x`. The iterations themselves are never differentiated, so an unconverged local solve cannot
//!   leak into the tangent (and a local solve that did not converge is an error, not a tangent);
//! - elastic and hyperelastic laws: forward AD of the closed-form stress.
//!
//! [`MaterialSensitivities`] carries every partial derivative the global sensitivity engine needs: stress and new state with respect
//! to strain, to the committed state and to the material parameters.

use super::dual::Dual;
use super::scalar::Scalar;
use mechanics_core::hardening::Hardening;

/// Number of independent entries of the committed state: plastic strain `(xx, yy, zz, xy, yz, zx)` and the equivalent plastic strain.
pub const STATE: usize = 7;
/// Strain / stress components in the library's Voigt order `(xx, yy, zz, xy, yz, zx)` with **tensor** shear.
pub const VOIGT: usize = 6;

/// Which side of a nonsmooth transition the evaluation is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Branch {
    Elastic,
    Plastic,
    /// Within the tolerance of the yield surface: the tangent differs on the two sides; the returned one is the plastic side if the
    /// point is on or above the surface, the elastic one below. Both one-sided tangents are available from [`MaterialPoint::one_sided`].
    Transition,
}

/// Every first derivative of one constitutive update.
#[derive(Debug, Clone, PartialEq)]
pub struct MaterialSensitivities {
    pub stress: [f64; VOIGT],
    pub new_state: [f64; STATE],
    pub branch: Branch,
    /// `d stress / d strain`, row-major `6 x 6`: the consistent algorithmic tangent.
    pub d_stress_d_strain: Vec<f64>,
    /// `d stress / d committed state`, `6 x 7`.
    pub d_stress_d_state: Vec<f64>,
    /// `d stress / d parameters`, `6 x np`.
    pub d_stress_d_params: Vec<f64>,
    /// `d new state / d strain`, `7 x 6`.
    pub d_new_state_d_strain: Vec<f64>,
    /// `d new state / d committed state`, `7 x 7`.
    pub d_new_state_d_state: Vec<f64>,
    /// `d new state / d parameters`, `7 x np`.
    pub d_new_state_d_params: Vec<f64>,
    pub n_params: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ConstitutiveError {
    /// The local nonlinear solve did not converge: there is no derivative of an unconverged map.
    LocalSolveFailed(String),
    /// The local Jacobian is singular (a perfectly plastic limit state, a mechanism).
    SingularLocalJacobian,
    Invalid(String),
}

impl std::fmt::Display for ConstitutiveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LocalSolveFailed(s) => write!(f, "the local constitutive solve did not converge ({s}): its derivative does not exist"),
            Self::SingularLocalJacobian => write!(f, "the local constitutive Jacobian is singular"),
            Self::Invalid(s) => f.write_str(s),
        }
    }
}

impl std::error::Error for ConstitutiveError {}

/// Output of a generic update.
pub struct Update<S: Scalar> {
    pub stress: [S; VOIGT],
    pub new_state: [S; STATE],
    /// Residual of the local system at the supplied unknowns (empty for explicit models).
    pub residual: Vec<S>,
}

pub trait DifferentiableMaterial {
    /// Names of the differentiable parameters, in the order of the parameter vector.
    fn parameter_names(&self) -> Vec<&'static str>;
    /// Nominal parameter values.
    fn parameters(&self) -> Vec<f64>;
    /// Number of unknowns of the local nonlinear system (0 for an explicit update).
    fn n_local(&self) -> usize {
        0
    }
    /// Solve the local system in `f64` to convergence for plain inputs; `Err` when it does not converge.
    fn solve_local(&self, _p: &[f64], _strain: &[f64; VOIGT], _old: &[f64; STATE]) -> Result<Vec<f64>, ConstitutiveError> {
        Ok(Vec::new())
    }
    /// The update with the local unknowns `q` supplied (empty for explicit models).
    fn update<S: Scalar>(&self, p: &[S], strain: &[S; VOIGT], old: &[S; STATE], q: &[S]) -> Update<S>;
    /// How close the point is to a nonsmooth transition, `< 0` elastic side, `> 0` plastic side, relative to the yield stress.
    fn yield_margin(&self, _p: &[f64], _strain: &[f64; VOIGT], _old: &[f64; STATE]) -> Option<f64> {
        None
    }
}

// ----------------------------------------------------------------------------------------------- small dense algebra

/// Solve `A X = B` for a dense `n x n` matrix by Gaussian elimination with partial pivoting; `Err` if singular. `a` is row-major,
/// `b` is `n x m` row-major.
pub fn lu_solve(n: usize, m: usize, a: &[f64], b: &[f64]) -> Result<Vec<f64>, ConstitutiveError> {
    let mut a = a.to_vec();
    let mut x = b.to_vec();
    let scale = a.iter().fold(0.0f64, |s, v| s.max(v.abs())).max(1e-300);
    for c in 0..n {
        let piv = (c..n).max_by(|&i, &j| a[i * n + c].abs().total_cmp(&a[j * n + c].abs())).expect("non-empty");
        if a[piv * n + c].abs() < 1e-13 * scale {
            return Err(ConstitutiveError::SingularLocalJacobian);
        }
        if piv != c {
            for k in 0..n {
                a.swap(c * n + k, piv * n + k);
            }
            for k in 0..m {
                x.swap(c * m + k, piv * m + k);
            }
        }
        for i in c + 1..n {
            let f = a[i * n + c] / a[c * n + c];
            if f != 0.0 {
                for k in c..n {
                    a[i * n + k] -= f * a[c * n + k];
                }
                for k in 0..m {
                    x[i * m + k] -= f * x[c * m + k];
                }
            }
        }
    }
    for c in (0..n).rev() {
        for k in 0..m {
            let mut s = x[c * m + k];
            for j in c + 1..n {
                s -= a[c * n + j] * x[j * m + k];
            }
            x[c * m + k] = s / a[c * n + c];
        }
    }
    Ok(x)
}

// ----------------------------------------------------------------------------------------------- the generic Jacobian of an update

// ----------------------------------------------------------------------------------------------- the material point

/// A material point: the committed state and the trial update. `commit` is the only mutation.
#[derive(Debug, Clone)]
pub struct MaterialPoint<M: DifferentiableMaterial> {
    pub model: M,
    pub params: Vec<f64>,
    pub committed: [f64; STATE],
}

/// A trial update: not yet committed.
#[derive(Debug, Clone, PartialEq)]
pub struct Trial {
    pub stress: [f64; VOIGT],
    pub new_state: [f64; STATE],
}

/// Relative distance to the yield surface (of the yield stress) within which a point is called a transition.
pub const TRANSITION_TOLERANCE: f64 = 1e-9;

impl<M: DifferentiableMaterial> MaterialPoint<M> {
    pub fn new(model: M) -> Self {
        let params = model.parameters();
        Self { model, params, committed: [0.0; STATE] }
    }

    /// The stress and trial state for `strain`; leaves the committed state alone.
    pub fn trial(&self, strain: &[f64; VOIGT]) -> Result<Trial, ConstitutiveError> {
        let q = self.model.solve_local(&self.params, strain, &self.committed)?;
        let u = self.model.update::<f64>(&self.params, strain, &self.committed, &q);
        Ok(Trial { stress: u.stress, new_state: u.new_state })
    }

    /// Accept a trial: the new state becomes the committed one.
    pub fn commit(&mut self, trial: &Trial) {
        self.committed = trial.new_state;
    }

    /// Every first derivative of the update at `strain` from the committed state.
    pub fn sensitivities(&self, strain: &[f64; VOIGT]) -> Result<MaterialSensitivities, ConstitutiveError> {
        sensitivities(&self.model, &self.params, strain, &self.committed)
    }

    /// The tangent on the elastic and the plastic side of a transition point, `None` if the model has no yield surface.
    pub fn one_sided(&self, strain: &[f64; VOIGT]) -> Result<Option<(MaterialSensitivities, MaterialSensitivities)>, ConstitutiveError> {
        // step the strain a hair toward each side along the radial direction of the current strain increment
        let base = self.model.yield_margin(&self.params, strain, &self.committed);
        let Some(m0) = base else { return Ok(None) };
        let scale = strain.iter().fold(0.0f64, |a, v| a.max(v.abs())).max(1e-12);
        let mut best: Option<(MaterialSensitivities, MaterialSensitivities)> = None;
        for &t in &[1e-6, 1e-4] {
            let lo: [f64; VOIGT] = std::array::from_fn(|i| strain[i] * (1.0 - t));
            let hi: [f64; VOIGT] = std::array::from_fn(|i| strain[i] * (1.0 + t));
            let (ml, mh) = (self.model.yield_margin(&self.params, &lo, &self.committed).unwrap_or(m0), self.model.yield_margin(&self.params, &hi, &self.committed).unwrap_or(m0));
            if ml < 0.0 && mh > 0.0 {
                best = Some((sensitivities(&self.model, &self.params, &lo, &self.committed)?, sensitivities(&self.model, &self.params, &hi, &self.committed)?));
                break;
            }
        }
        let _ = scale;
        Ok(best)
    }
}

/// The derivatives of the update of `model` (see [`MaterialSensitivities`]).
pub fn sensitivities<M: DifferentiableMaterial>(model: &M, p: &[f64], strain: &[f64; VOIGT], old: &[f64; STATE]) -> Result<MaterialSensitivities, ConstitutiveError> {
    let np = p.len();
    let nq = model.n_local();
    let q = model.solve_local(p, strain, old)?;
    // plain evaluation and convergence check of the local system
    let plain = model.update::<f64>(p, strain, old, &q);
    if nq > 0 {
        let scale = q.iter().fold(1e-300f64, |m, v| m.max(v.abs())).max(plain.stress.iter().fold(0.0f64, |m, v| m.max(v.abs())) * 1e-3).max(1e-300);
        let rn = plain.residual.iter().fold(0.0f64, |m, v| m.max(v.abs()));
        let ref_scale = plain.stress.iter().fold(0.0f64, |m, v| m.max(v.abs())).max(scale);
        if !(rn <= 1e-8 * ref_scale) {
            return Err(ConstitutiveError::LocalSolveFailed(format!("residual {rn:e} against stress scale {ref_scale:e}")));
        }
    }
    // inputs ordered [strain(6), state(7), params(np), q(nq)]
    let n_in = VOIGT + STATE + np + nq;
    let n_out = VOIGT + STATE + plain.residual.len();
    let mut x = Vec::with_capacity(n_in);
    x.extend_from_slice(strain);
    x.extend_from_slice(old);
    x.extend_from_slice(p);
    x.extend_from_slice(&q);
    let mut jac = vec![0.0; n_out * n_in];
    // chunked forward mode, 8 directions at a time
    let mut start = 0;
    while start < n_in {
        let xs: Vec<Dual<f64, 8>> = (0..n_in)
            .map(|i| {
                let mut d = [0.0; 8];
                if i >= start && i < start + 8 {
                    d[i - start] = 1.0;
                }
                Dual::with(x[i], d)
            })
            .collect();
        let eps: [Dual<f64, 8>; VOIGT] = std::array::from_fn(|i| xs[i]);
        let st: [Dual<f64, 8>; STATE] = std::array::from_fn(|i| xs[VOIGT + i]);
        let ps = &xs[VOIGT + STATE..VOIGT + STATE + np];
        let qs = &xs[VOIGT + STATE + np..];
        let out = model.update::<Dual<f64, 8>>(ps, &eps, &st, qs);
        for r in 0..n_out {
            let o = if r < VOIGT {
                out.stress[r]
            } else if r < VOIGT + STATE {
                out.new_state[r - VOIGT]
            } else {
                out.residual[r - VOIGT - STATE]
            };
            for j in 0..8.min(n_in - start) {
                jac[r * n_in + start + j] = o.d[j];
            }
        }
        start += 8;
    }
    let j = |r: usize, c: usize| jac[r * n_in + c];
    let n_o = VOIGT + STATE;
    // partial derivatives: outputs wrt x = [strain, state, params] (columns 0..n_x) and wrt q
    let n_x = VOIGT + STATE + np;
    let mut total = vec![0.0; n_o * n_x];
    for r in 0..n_o {
        for c in 0..n_x {
            total[r * n_x + c] = j(r, c);
        }
    }
    if nq > 0 {
        // F_q (nq x nq), F_x (nq x n_x), out_q (n_o x nq): total = out_x - out_q F_q^-1 F_x
        let fq: Vec<f64> = (0..nq).flat_map(|r| (0..nq).map(move |c| (r, c))).map(|(r, c)| j(n_o + r, n_x + c)).collect();
        let fx: Vec<f64> = (0..nq).flat_map(|r| (0..n_x).map(move |c| (r, c))).map(|(r, c)| j(n_o + r, c)).collect();
        let dq = lu_solve(nq, n_x, &fq, &fx)?; // F_q^-1 F_x
        for r in 0..n_o {
            for c in 0..n_x {
                let mut s = 0.0;
                for k in 0..nq {
                    s += j(r, n_x + k) * dq[k * n_x + c];
                }
                total[r * n_x + c] -= s;
            }
        }
    }
    let take = |rows: std::ops::Range<usize>, cols: std::ops::Range<usize>| -> Vec<f64> { rows.flat_map(|r| cols.clone().map(move |c| (r, c))).map(|(r, c)| total[r * n_x + c]).collect() };
    let margin = model.yield_margin(p, strain, old);
    let branch = match margin {
        Some(m) if m.abs() <= TRANSITION_TOLERANCE => Branch::Transition,
        Some(m) if m > 0.0 => Branch::Plastic,
        _ => Branch::Elastic,
    };
    Ok(MaterialSensitivities {
        stress: plain.stress,
        new_state: plain.new_state,
        branch,
        d_stress_d_strain: take(0..VOIGT, 0..VOIGT),
        d_stress_d_state: take(0..VOIGT, VOIGT..VOIGT + STATE),
        d_stress_d_params: take(0..VOIGT, VOIGT + STATE..n_x),
        d_new_state_d_strain: take(VOIGT..n_o, 0..VOIGT),
        d_new_state_d_state: take(VOIGT..n_o, VOIGT..VOIGT + STATE),
        d_new_state_d_params: take(VOIGT..n_o, VOIGT + STATE..n_x),
        n_params: np,
    })
}

// ----------------------------------------------------------------------------------------------- linear elasticity

/// Hooke's law, isotropic. The tangent is constant: the analytic route is preferred and AD agrees.
#[derive(Debug, Clone, Copy)]
pub struct Hooke {
    pub e: f64,
    pub nu: f64,
}

impl DifferentiableMaterial for Hooke {
    fn parameter_names(&self) -> Vec<&'static str> {
        vec!["E", "nu"]
    }
    fn parameters(&self) -> Vec<f64> {
        vec![self.e, self.nu]
    }
    fn update<S: Scalar>(&self, p: &[S], strain: &[S; VOIGT], old: &[S; STATE], _q: &[S]) -> Update<S> {
        let (e, nu) = (p[0], p[1]);
        let g = e / (S::cst(2.0) * (S::cst(1.0) + nu));
        let lam = e * nu / ((S::cst(1.0) + nu) * (S::cst(1.0) - nu.scale(2.0)));
        let tr = strain[0] + strain[1] + strain[2];
        let mut s = [S::cst(0.0); VOIGT];
        for i in 0..3 {
            s[i] = lam * tr + g.scale(2.0) * strain[i];
            s[3 + i] = g.scale(2.0) * strain[3 + i];
        }
        Update { stress: s, new_state: *old, residual: Vec::new() }
    }
}

impl Hooke {
    /// The analytic tangent `d stress / d strain` (6 x 6, row-major) in tensor-shear Voigt order.
    pub fn analytic_tangent(&self) -> Vec<f64> {
        let g = self.e / (2.0 * (1.0 + self.nu));
        let lam = self.e * self.nu / ((1.0 + self.nu) * (1.0 - 2.0 * self.nu));
        let mut t = vec![0.0; 36];
        for i in 0..3 {
            for j in 0..3 {
                t[i * 6 + j] = lam + if i == j { 2.0 * g } else { 0.0 };
            }
            t[(3 + i) * 6 + 3 + i] = 2.0 * g;
        }
        t
    }
}

// ----------------------------------------------------------------------------------------------- J2 with a tabulated law

/// Small-strain J2 plasticity, isotropic piecewise-linear hardening: the algorithm of `material::small_strain_update`, written
/// against [`Scalar`] so that its derivative is exact and the committed-state and parameter dependences are available.
///
/// Parameters: `E`, `nu`, a yield multiplier `sy` (every tabulated stress is scaled by it) and a hardening multiplier `sh`
/// (the rise of the stress over its initial value is scaled by it): `sigma_k(p) = sy (sigma_0 + sh (sigma_k - sigma_0))`.
#[derive(Debug, Clone)]
pub struct J2Table {
    pub e: f64,
    pub nu: f64,
    pub law: Vec<(f64, f64)>,
}

impl J2Table {
    pub fn new(e: f64, nu: f64, law: &Hardening) -> Self {
        Self { e, nu, law: law.points().to_vec() }
    }

    fn stresses<S: Scalar>(&self, sy: S, sh: S) -> Vec<S> {
        let s0 = self.law[0].1;
        self.law.iter().map(|&(_, s)| sy * (S::cst(s0) + sh * S::cst(s - s0))).collect()
    }
}

/// Yield stress `sigma_y(ep)` of a tabulated law with differentiable stresses.
fn table_stress<S: Scalar>(eps: &[f64], sig: &[S], ep: S) -> S {
    if ep.le(&S::cst(0.0)) {
        return sig[0];
    }
    for k in 0..eps.len() - 1 {
        if ep.le(&S::cst(eps[k + 1])) {
            return sig[k] + (sig[k + 1] - sig[k]) * (ep - S::cst(eps[k])) / S::cst(eps[k + 1] - eps[k]);
        }
    }
    sig[eps.len() - 1]
}

/// The plastic increment `d >= 0` solving `q - 3 g d = sigma_y(ep + d)`: the exact segment-by-segment solution of
/// `Hardening::plastic_increment`, with every quantity a scalar.
fn table_increment<S: Scalar>(eps: &[f64], sig: &[S], ep: S, q: S, g: S) -> S {
    let n = eps.len();
    let mut e = ep;
    let mut y = table_stress(eps, sig, ep);
    let mut k = (0..n - 1).position(|k| ep.lt(&S::cst(eps[k + 1]))).unwrap_or(n - 1);
    loop {
        let (slope, end) = if k + 1 < n { ((sig[k + 1] - sig[k]) / S::cst(eps[k + 1] - eps[k]), eps[k + 1]) } else { (S::cst(0.0), f64::INFINITY) };
        let three_g = g.scale(3.0);
        let t = (q - y - three_g * (e - ep)) / (three_g + slope);
        if (e + t).le(&S::cst(end)) || k + 1 >= n {
            return (e + t - ep).max(S::cst(0.0));
        }
        y = y + slope * (S::cst(end) - e);
        e = S::cst(end);
        k += 1;
    }
}

/// Shared radial-return algebra of the two J2 models: from the trial deviatoric strain, the plastic increment `d` (a scalar, zero when
/// elastic) and the moduli, the stress and the state update.
#[allow(clippy::too_many_arguments)]
fn j2_stress_and_state<S: Scalar>(g: S, k: S, strain: &[S; VOIGT], old: &[S; STATE], d: S, plastic: bool, dn: S, dev: &[S; VOIGT], tr: S, q: S) -> ([S; VOIGT], [S; STATE]) {
    let one = S::cst(1.0);
    let theta = if plastic { one - g.scale(3.0) * d / q } else { one };
    let mut stress = [S::cst(0.0); VOIGT];
    for i in 0..3 {
        stress[i] = g.scale(2.0) * theta * dev[i] + k * tr;
        stress[3 + i] = g.scale(2.0) * theta * dev[3 + i];
    }
    let mut state = *old;
    if plastic {
        let f = d * S::cst(1.5f64.sqrt());
        for i in 0..VOIGT {
            state[i] = state[i] + f * dev[i] / dn;
        }
        state[6] = state[6] + d;
    }
    let _ = strain;
    (stress, state)
}

/// Deviatoric elastic strain `e - ep`, its trace and the Frobenius norm of the deviator (off-diagonals counted twice).
fn deviator<S: Scalar>(strain: &[S; VOIGT], old: &[S; STATE]) -> ([S; VOIGT], S, S) {
    let e: [S; VOIGT] = std::array::from_fn(|i| strain[i] - old[i]);
    let tr = e[0] + e[1] + e[2];
    let third = tr / S::cst(3.0);
    let dev: [S; VOIGT] = [e[0] - third, e[1] - third, e[2] - third, e[3], e[4], e[5]];
    let sq = dev[0] * dev[0] + dev[1] * dev[1] + dev[2] * dev[2] + (dev[3] * dev[3] + dev[4] * dev[4] + dev[5] * dev[5]).scale(2.0);
    (dev, tr, if sq.gt(&S::cst(0.0)) { sq.sqrt() } else { S::cst(0.0) })
}

impl DifferentiableMaterial for J2Table {
    fn parameter_names(&self) -> Vec<&'static str> {
        vec!["E", "nu", "yield_scale", "hardening_scale"]
    }
    fn parameters(&self) -> Vec<f64> {
        vec![self.e, self.nu, 1.0, 1.0]
    }
    fn update<S: Scalar>(&self, p: &[S], strain: &[S; VOIGT], old: &[S; STATE], _q: &[S]) -> Update<S> {
        let (e, nu) = (p[0], p[1]);
        let g = e / (S::cst(2.0) * (S::cst(1.0) + nu));
        let k = e / (S::cst(3.0) * (S::cst(1.0) - nu.scale(2.0)));
        let eps: Vec<f64> = self.law.iter().map(|l| l.0).collect();
        let sig = self.stresses(p[2], p[3]);
        let ep = old[6];
        let (dev, tr, dn) = deviator(strain, old);
        let q = S::cst(1.5f64.sqrt()) * g.scale(2.0) * dn;
        let plastic = q.gt(&table_stress(&eps, &sig, ep)) && dn.gt(&S::cst(0.0));
        let d = if plastic { table_increment(&eps, &sig, ep, q, g) } else { S::cst(0.0) };
        let (stress, new_state) = j2_stress_and_state(g, k, strain, old, d, plastic, dn, &dev, tr, q);
        Update { stress, new_state, residual: Vec::new() }
    }
    fn yield_margin(&self, p: &[f64], strain: &[f64; VOIGT], old: &[f64; STATE]) -> Option<f64> {
        let eps: Vec<f64> = self.law.iter().map(|l| l.0).collect();
        let sig = self.stresses::<f64>(p[2], p[3]);
        let g = p[0] / (2.0 * (1.0 + p[1]));
        let (_, _, dn) = deviator(strain, old);
        let q = 1.5f64.sqrt() * 2.0 * g * dn;
        let y = table_stress(&eps, &sig, old[6]);
        Some((q - y) / y)
    }
}

// ----------------------------------------------------------------------------------------------- J2 with a nonlinear (Voce) law

/// Small-strain J2 with the saturating Voce law `sigma_y(ep) = s0 + h ep + q (1 - exp(-b ep))`. The radial return has no closed form:
/// the plastic increment solves `F(d) = q_trial - 3 g d - sigma_y(ep + d) = 0` by a local Newton iteration, and the tangent comes
/// from differentiating the converged equation, `dd/dx = -F_d^-1 F_x`.
///
/// Parameters: `E`, `nu`, `s0`, `h`, `q`, `b`.
#[derive(Debug, Clone, Copy)]
pub struct J2Voce {
    pub e: f64,
    pub nu: f64,
    pub s0: f64,
    pub h: f64,
    pub q: f64,
    pub b: f64,
    /// Newton iterations the local solve is allowed (a smaller number leaves it unconverged, which is an error for the tangent).
    pub max_iter: usize,
}

impl J2Voce {
    pub fn new(e: f64, nu: f64, s0: f64, h: f64, q: f64, b: f64) -> Self {
        Self { e, nu, s0, h, q, b, max_iter: 50 }
    }
}

fn voce<S: Scalar>(p: &[S], ep: S) -> S {
    p[2] + p[3] * ep + p[4] * (S::cst(1.0) - (-(p[5] * ep)).exp())
}

fn voce_slope<S: Scalar>(p: &[S], ep: S) -> S {
    p[3] + p[4] * p[5] * (-(p[5] * ep)).exp()
}

impl DifferentiableMaterial for J2Voce {
    fn parameter_names(&self) -> Vec<&'static str> {
        vec!["E", "nu", "sigma0", "hardening", "saturation", "rate"]
    }
    fn parameters(&self) -> Vec<f64> {
        vec![self.e, self.nu, self.s0, self.h, self.q, self.b]
    }
    fn n_local(&self) -> usize {
        1
    }
    fn solve_local(&self, p: &[f64], strain: &[f64; VOIGT], old: &[f64; STATE]) -> Result<Vec<f64>, ConstitutiveError> {
        let g = p[0] / (2.0 * (1.0 + p[1]));
        let (_, _, dn) = deviator(strain, old);
        let q = 1.5f64.sqrt() * 2.0 * g * dn;
        let ep = old[6];
        if !(q > voce(p, ep) && dn > 0.0) {
            return Ok(vec![0.0]);
        }
        // Newton on F(d) = q - 3 g d - sigma_y(ep + d); F is concave-decreasing in d, so Newton from d = 0 converges monotonically
        let mut d = 0.0;
        for _ in 0..self.max_iter {
            let f = q - 3.0 * g * d - voce(p, ep + d);
            if f.abs() <= 1e-14 * q.abs().max(1.0) {
                return Ok(vec![d]);
            }
            d -= f / (-3.0 * g - voce_slope(p, ep + d));
        }
        let f = q - 3.0 * g * d - voce(p, ep + d);
        if f.abs() <= 1e-9 * q.abs().max(1.0) {
            return Ok(vec![d]);
        }
        Err(ConstitutiveError::LocalSolveFailed(format!("{} Newton iterations, residual {f:e}", self.max_iter)))
    }
    fn update<S: Scalar>(&self, p: &[S], strain: &[S; VOIGT], old: &[S; STATE], q: &[S]) -> Update<S> {
        let g = p[0] / (S::cst(2.0) * (S::cst(1.0) + p[1]));
        let k = p[0] / (S::cst(3.0) * (S::cst(1.0) - p[1].scale(2.0)));
        let (dev, tr, dn) = deviator(strain, old);
        let qtr = S::cst(1.5f64.sqrt()) * g.scale(2.0) * dn;
        let ep = old[6];
        let plastic = qtr.gt(&voce(p, ep)) && dn.gt(&S::cst(0.0));
        let d = q[0];
        let residual = if plastic { vec![qtr - g.scale(3.0) * d - voce(p, ep + d)] } else { vec![d] };
        let (stress, new_state) = j2_stress_and_state(g, k, strain, old, if plastic { d } else { S::cst(0.0) }, plastic, dn, &dev, tr, qtr);
        Update { stress, new_state, residual }
    }
    fn yield_margin(&self, p: &[f64], strain: &[f64; VOIGT], old: &[f64; STATE]) -> Option<f64> {
        let g = p[0] / (2.0 * (1.0 + p[1]));
        let (_, _, dn) = deviator(strain, old);
        let q = 1.5f64.sqrt() * 2.0 * g * dn;
        let y = voce(p, old[6]);
        Some((q - y) / y)
    }
}

/// The tangent of `J2Voce` by differentiating *through* `iterations` Newton steps from `d = 0` (the thing the implicit route avoids).
/// With enough iterations it equals the implicit result; with too few it is wrong, which is the point of the comparison.
pub fn voce_tangent_through_iterations(m: &J2Voce, strain: &[f64; VOIGT], old: &[f64; STATE], iterations: usize) -> Vec<f64> {
    type D = Dual<f64, 6>;
    let p: Vec<D> = m.parameters().into_iter().map(D::cst).collect();
    let eps: [D; VOIGT] = std::array::from_fn(|i| Dual::var(strain[i], i));
    let st: [D; STATE] = std::array::from_fn(|i| D::cst(old[i]));
    let g = p[0] / (D::cst(2.0) * (D::cst(1.0) + p[1]));
    let k = p[0] / (D::cst(3.0) * (D::cst(1.0) - p[1].scale(2.0)));
    let (dev, tr, dn) = deviator(&eps, &st);
    let qtr = D::cst(1.5f64.sqrt()) * g.scale(2.0) * dn;
    let ep = st[6];
    let plastic = qtr.gt(&voce(&p, ep)) && dn.gt(&D::cst(0.0));
    let mut d = D::cst(0.0);
    if plastic {
        for _ in 0..iterations {
            let f = qtr - g.scale(3.0) * d - voce(&p, ep + d);
            d = d - f / (-g.scale(3.0) - voce_slope(&p, ep + d));
        }
    }
    let (stress, _) = j2_stress_and_state(g, k, &eps, &st, d, plastic, dn, &dev, tr, qtr);
    (0..VOIGT).flat_map(|r| (0..VOIGT).map(move |c| (r, c))).map(|(r, c)| stress[r].d[c]).collect()
}

// ----------------------------------------------------------------------------------------------- hyperelastic (neo-Hookean)

fn det3<S: Scalar>(a: &[[S; 3]; 3]) -> S {
    a[0][0] * (a[1][1] * a[2][2] - a[1][2] * a[2][1]) - a[0][1] * (a[1][0] * a[2][2] - a[1][2] * a[2][0]) + a[0][2] * (a[1][0] * a[2][1] - a[1][1] * a[2][0])
}

fn inverse3<S: Scalar>(a: &[[S; 3]; 3]) -> [[S; 3]; 3] {
    let d = det3(a);
    let c = |i: usize, j: usize| {
        let (i1, i2, j1, j2) = ((i + 1) % 3, (i + 2) % 3, (j + 1) % 3, (j + 2) % 3);
        a[i1][j1] * a[i2][j2] - a[i1][j2] * a[i2][j1]
    };
    // inverse = adjugate / det, adjugate_ij = cofactor_ji
    std::array::from_fn(|i| std::array::from_fn(|j| c(j, i) / d))
}

/// First Piola-Kirchhoff stress of the compressible neo-Hookean law `W = G/2 (I1 - 3) - G ln J + lambda/2 (ln J)^2` for the
/// displacement gradient `h` (row-major `3 x 3`), against any scalar: `P = G (F - F^-T) + lambda ln J F^-T`.
pub fn neo_hookean_pk1<S: Scalar>(g: S, lam: S, h: &[S; 9]) -> [S; 9] {
    let f: [[S; 3]; 3] = std::array::from_fn(|i| std::array::from_fn(|j| h[3 * i + j] + S::cst(if i == j { 1.0 } else { 0.0 })));
    let finv = inverse3(&f);
    let lnj = det3(&f).ln();
    std::array::from_fn(|n| {
        let (i, j) = (n / 3, n % 3);
        g * (f[i][j] - finv[j][i]) + lam * lnj * finv[j][i]
    })
}

// ----------------------------------------------------------------------------------------------- plane stress

/// Plane-stress version of an explicit model: the out-of-plane strain is the local unknown of `sigma_zz(eps_zz) = 0`, solved by a
/// scalar Newton iteration in `f64` and differentiated implicitly like any other local system (the kernel does the same condensation
/// in `material::small_strain_plane_stress`). The strain components `zz`, `yz` and `zx` of the input are ignored.
#[derive(Debug, Clone)]
pub struct PlaneStress<M: DifferentiableMaterial> {
    pub inner: M,
}

impl<M: DifferentiableMaterial> DifferentiableMaterial for PlaneStress<M> {
    fn parameter_names(&self) -> Vec<&'static str> {
        self.inner.parameter_names()
    }
    fn parameters(&self) -> Vec<f64> {
        self.inner.parameters()
    }
    fn n_local(&self) -> usize {
        1
    }
    fn solve_local(&self, p: &[f64], strain: &[f64; VOIGT], old: &[f64; STATE]) -> Result<Vec<f64>, ConstitutiveError> {
        type D = Dual<f64, 1>;
        let pd: Vec<D> = p.iter().map(|&v| D::cst(v)).collect();
        let od: [D; STATE] = std::array::from_fn(|i| D::cst(old[i]));
        // start from the elastic condensation: eps_zz = -nu/(1-nu) (eps_xx + eps_yy)
        let nu = p[1];
        let mut ezz = -nu / (1.0 - nu) * (strain[0] + strain[1]);
        let mut scale = 1e-300f64;
        for it in 0..60 {
            let mut e: [D; VOIGT] = std::array::from_fn(|i| D::cst(strain[i]));
            e[2] = Dual::var(ezz, 0);
            e[4] = D::cst(0.0);
            e[5] = D::cst(0.0);
            let u = self.inner.update::<D>(&pd, &e, &od, &[]);
            let szz = u.stress[2];
            scale = scale.max(u.stress.iter().fold(0.0f64, |m, s| m.max(s.v.abs())));
            if szz.v.abs() <= 1e-14 * scale {
                return Ok(vec![ezz]);
            }
            if szz.d[0] == 0.0 || !szz.d[0].is_finite() {
                return Err(ConstitutiveError::LocalSolveFailed("the out-of-plane stiffness vanished".into()));
            }
            ezz -= szz.v / szz.d[0];
            let _ = it;
        }
        Err(ConstitutiveError::LocalSolveFailed("the plane-stress condensation did not converge".into()))
    }
    fn update<S: Scalar>(&self, p: &[S], strain: &[S; VOIGT], old: &[S; STATE], q: &[S]) -> Update<S> {
        let mut e = *strain;
        e[2] = q[0];
        e[4] = S::cst(0.0);
        e[5] = S::cst(0.0);
        let mut u = self.inner.update(p, &e, old, &[]);
        u.residual = vec![u.stress[2]];
        u
    }
    fn yield_margin(&self, p: &[f64], strain: &[f64; VOIGT], old: &[f64; STATE]) -> Option<f64> {
        // the margin of the condensed state
        let q = self.solve_local(p, strain, old).ok()?;
        let mut e = *strain;
        e[2] = q[0];
        e[4] = 0.0;
        e[5] = 0.0;
        self.inner.yield_margin(p, &e, old)
    }
}

// ----------------------------------------------------------------------------------------------- stress Jacobian-vector product

/// `d stress = C : d strain` for one strain increment, without forming `C`: one forward-mode evaluation of the update with the strain
/// seeded along `dstrain` and the local unknowns seeded along their own axes, then the implicit correction
/// `dq = -F_q^-1 F_eps dstrain`. This is the matrix-free tangent action of the constitutive law.
pub fn stress_jvp<M: DifferentiableMaterial>(model: &M, p: &[f64], strain: &[f64; VOIGT], old: &[f64; STATE], dstrain: &[f64; VOIGT]) -> Result<[f64; VOIGT], ConstitutiveError> {
    type D = Dual<f64, 8>;
    let nq = model.n_local();
    if nq > 7 {
        return Err(ConstitutiveError::Invalid("a local system of more than 7 unknowns is not supported by the matrix-free product".into()));
    }
    let q = model.solve_local(p, strain, old)?;
    let ps: Vec<D> = p.iter().map(|&v| D::cst(v)).collect();
    let eps: [D; VOIGT] = std::array::from_fn(|i| {
        let mut d = [0.0; 8];
        d[0] = dstrain[i];
        Dual::with(strain[i], d)
    });
    let st: [D; STATE] = std::array::from_fn(|i| D::cst(old[i]));
    let qs: Vec<D> = (0..nq).map(|k| Dual::var(q[k], k + 1)).collect();
    let out = model.update::<D>(&ps, &eps, &st, &qs);
    let mut dq = vec![0.0; nq];
    if nq > 0 {
        let fq: Vec<f64> = (0..nq).flat_map(|r| (0..nq).map(move |c| (r, c))).map(|(r, c)| out.residual[r].d[c + 1]).collect();
        let rhs: Vec<f64> = (0..nq).map(|r| out.residual[r].d[0]).collect();
        dq = lu_solve(nq, 1, &fq, &rhs)?.into_iter().map(|v| -v).collect();
    }
    Ok(std::array::from_fn(|i| {
        let mut s = out.stress[i].d[0];
        for k in 0..nq {
            s += out.stress[i].d[k + 1] * dq[k];
        }
        s
    }))
}
