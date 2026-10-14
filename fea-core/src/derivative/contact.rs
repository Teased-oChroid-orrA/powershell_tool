//! Derivatives and solution of contact (Story G): the state of a contact interface, the exact tangent of the implemented contact law,
//! and sensitivities of the **penalty-regularised** contact problem with their semantics reported.
//!
//! Contact is nonsmooth: gaps open and close, friction sticks and slides. A classical derivative of the solution exists only where the
//! contact state is locally constant. This module therefore
//! * classifies every contact point (open / stick / slip) and detects points *near* a transition ([`classify_contact`]), and says
//!   what the derivative it can return means ([`DerivativeSemantics`]): the derivative of the active branch where nothing is near
//!   a transition, an element of the generalized Jacobian (a one-sided derivative) where something is;
//! * evaluates the residual and the **exact** tangent of the implemented law at a state, including the unsymmetric friction part
//!   as separate blocks ([`PenaltyContact::evaluate`]), so a directional Taylor test can verify it;
//! * solves the penalty problem (the law with the augmented-Lagrangian multipliers frozen at the stated values) by Newton with that
//!   tangent, and differentiates it directly or by the adjoint through the (possibly unsymmetric) tangent
//!   ([`PenaltyContact::sensitivities`]).
//!
//! The sensitivities are those of the *regularised* (penalty) law at the stated multipliers, not of the unilateral limit: the two
//! differ by `O(stress / eps_n)`, which [`PenaltyContact::regularisation_report`] states. The sensitivities of the unilateral problem
//! itself (with generalized derivatives at degenerate points) are in [`super::complementarity`].

use super::linalg::TangentSolver;
use super::policy::{ContactStatus, DerivativeSemantics, SensitivityMethod};
use crate::assembly::{BlockMatrix, Pattern};
use crate::contact::{ContactSet, ContactSpec, ContactStats, CpState};
use crate::analysis::Model;
use crate::linear::{Dirichlet, Ordering, Reduced};
use crate::loads::{self, Loads};
use crate::nonlinear::{assemble_nl_public, NlState};
use std::sync::Arc;

/// Where a contact point is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointStatus {
    Open,
    /// In contact and adhering (friction below the Coulomb limit, or frictionless contact).
    Stick,
    /// In contact and sliding (friction at the Coulomb limit).
    Slip,
}

/// Which transition a point is close to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transition {
    /// The gap is about to close or the pressure about to vanish.
    Normal,
    /// The point is at the edge of the friction cone.
    Friction,
}

#[derive(Debug, Clone, PartialEq)]
pub struct InterfaceStatus {
    pub name: String,
    pub open: usize,
    pub stick: usize,
    pub slip: usize,
    pub near_normal_transition: usize,
    pub near_friction_transition: usize,
    pub min_gap: f64,
    pub peak_pressure: f64,
    /// Largest `|trial friction| / (mu p)` over the active points (`0` without friction): above 1 a point slides.
    pub max_friction_ratio: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ContactStateReport {
    pub interfaces: Vec<InterfaceStatus>,
    /// What a derivative computed at this state means.
    pub semantics: DerivativeSemantics,
    pub contact: ContactStatus,
    pub notes: Vec<String>,
}

impl ContactStateReport {
    pub fn total(&self, f: impl Fn(&InterfaceStatus) -> usize) -> usize {
        self.interfaces.iter().map(f).sum()
    }
}

/// Classify every point of every interface and decide the derivative semantics. `tol` is the relative distance to a transition below
/// which a point counts as near it (relative to the interface's peak pressure, and to the penetration that pressure causes).
pub fn classify_contact(specs: &[ContactSpec], state: &[Vec<CpState>], tol: f64) -> ContactStateReport {
    let mut interfaces = Vec::new();
    let mut notes = Vec::new();
    for (spec, pts) in specs.iter().zip(state) {
        let peak = pts.iter().filter(|s| s.active).fold(0.0f64, |m, s| m.max(s.p));
        // characteristic gap: the penetration the peak pressure causes through the normal penalty
        let g_scale = if spec.eps_n > 0.0 && peak > 0.0 { peak / spec.eps_n } else { f64::INFINITY };
        let mut st = InterfaceStatus { name: spec.name.clone(), open: 0, stick: 0, slip: 0, near_normal_transition: 0, near_friction_transition: 0, min_gap: f64::INFINITY, peak_pressure: peak, max_friction_ratio: 0.0 };
        for s in pts {
            st.min_gap = st.min_gap.min(s.g);
            if !s.active || s.p <= 0.0 {
                st.open += 1;
                // an open point whose gap is a tiny fraction of the characteristic penetration is about to close
                if s.g.abs() <= tol * g_scale && g_scale.is_finite() {
                    st.near_normal_transition += 1;
                }
                continue;
            }
            if s.p <= tol * peak {
                st.near_normal_transition += 1;
            }
            if spec.mu > 0.0 {
                // The trial traction of the return map, `lam_t + eps_t (slip in the tangent plane)`: stick when it is inside the
                // Coulomb cone, slip when it is outside. A point that slides always has a *capped* traction exactly on the cone, so
                // the capped traction cannot tell how near a point is to sticking again; the trial traction can.
                let n = s.n;
                let mut slip = if s.has_hist { [s.cur[0] - s.hist[0], s.cur[1] - s.hist[1], s.cur[2] - s.hist[2]] } else { [0.0; 3] };
                let dn = slip[0] * n[0] + slip[1] * n[1] + slip[2] * n[2];
                for i in 0..3 {
                    slip[i] -= dn * n[i];
                }
                let trial: [f64; 3] = std::array::from_fn(|i| s.lam_t[i] + spec.eps_t * slip[i]);
                let tn = (trial[0] * trial[0] + trial[1] * trial[1] + trial[2] * trial[2]).sqrt();
                let cone = (spec.mu * s.p).max(1e-300);
                let ratio = tn / cone;
                st.max_friction_ratio = st.max_friction_ratio.max(ratio);
                if (ratio - 1.0).abs() <= tol {
                    st.near_friction_transition += 1;
                }
                if ratio >= 1.0 {
                    st.slip += 1;
                } else {
                    st.stick += 1;
                }
            } else {
                st.stick += 1;
            }
        }
        if st.min_gap == f64::INFINITY {
            st.min_gap = 0.0;
        }
        interfaces.push(st);
    }
    let near: usize = interfaces.iter().map(|i| i.near_normal_transition + i.near_friction_transition).sum();
    let slips: usize = interfaces.iter().map(|i| i.slip).sum();
    let any_contact = interfaces.iter().any(|i| i.stick + i.slip > 0);
    let (semantics, contact) = if near > 0 {
        notes.push(format!("{near} point(s) lie within {tol:.0e} of a contact transition: the solution is not differentiable in the classical sense there; only one-sided (generalized) derivatives exist"));
        (DerivativeSemantics::Generalized, ContactStatus::Transition)
    } else if any_contact {
        if slips > 0 {
            notes.push(format!("{slips} sliding point(s): the friction traction depends on the slip history, which a single-state sensitivity treats as frozen"));
        }
        (DerivativeSemantics::ActiveBranch, ContactStatus::FixedActiveSet)
    } else {
        (DerivativeSemantics::Classical, ContactStatus::None)
    };
    ContactStateReport { interfaces, semantics, contact, notes }
}

/// A parameter of the penalty-contact problem.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ContactParam {
    /// Multiplier of the applied loads only (at 1).
    LoadScale,
    /// The load factor `lambda` itself: scales the applied loads and moves a rigid master along its prescribed travel.
    LoadFactor,
    /// Multiplier of the elastic modulus of a block, at 1.
    ModulusScale(usize),
}

#[derive(Debug, Clone)]
pub struct ContactSolution {
    pub u: Vec<f64>,
    pub lambda: f64,
    pub iterations: usize,
    pub residual: f64,
    pub stats: ContactStats,
}

/// What regularisation the sensitivities belong to.
#[derive(Debug, Clone, PartialEq)]
pub struct RegularisationReport {
    /// Normal penalty of each interface.
    pub eps_n: Vec<f64>,
    /// Largest remaining penetration of an active point (the law's departure from the unilateral condition `g >= 0`).
    pub max_penetration: f64,
    /// Peak pressure over the penalty: the order of the penetration the regularisation allows.
    pub penetration_scale: f64,
    pub note: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ContactSensitivityReport {
    pub method: SensitivityMethod,
    pub semantics: DerivativeSemantics,
    pub state: ContactStateReport,
    pub tangent_solves: usize,
    pub krylov_iterations: usize,
    pub unsymmetric: bool,
}

/// The penalty contact problem of a model: residual `R(u; lambda) = f_int(u) + f_contact(u) - lambda f_ext` with the contact
/// multipliers frozen at the state given at construction (zero history by default).
pub struct PenaltyContact<'m> {
    model: &'m Model,
    set: ContactSet,
    pat: Arc<Pattern>,
    bc: &'m Dirichlet,
    f_ext: Vec<f64>,
    old: NlState,
    specs: Vec<ContactSpec>,
}

impl<'m> PenaltyContact<'m> {
    pub fn new(model: &'m Model, specs: Vec<ContactSpec>, loads: &Loads, bc: &'m Dirichlet) -> Result<Self, String> {
        if model.mesh.blocks.iter().any(|b| b.plasticity.is_some()) {
            return Err("contact sensitivities are for elastic bodies (the plastic path engine has no contact)".into());
        }
        let set = ContactSet::new(&model.mesh, specs.clone())?;
        if set.n_extra() > 0 {
            return Err("free rigid-body translations are not supported by the contact sensitivities".into());
        }
        let pat = Arc::new(Pattern::new_with_extra(&model.mesh, &set.pattern_pairs(&model.mesh))?);
        let mut old = NlState::new(&model.mesh);
        old.contact = set.initial_state(&model.mesh, &pat);
        Ok(Self { model, set, pat, bc, f_ext: loads::assemble(&model.mesh, loads)?, old, specs })
    }

    /// Freeze the multipliers and slip history at a converged state (`NlSolution::state.contact`) instead of the virgin one.
    pub fn with_contact_state(mut self, contact: Vec<Vec<CpState>>) -> Self {
        self.old.contact = contact;
        self
    }

    pub fn contact_state(&self) -> &[Vec<CpState>] {
        &self.old.contact
    }

    /// `R(u; lambda)` over all dofs (no constraint rows removed), the tangent `K` (symmetric part) and the unsymmetric blocks `D`
    /// of the exact tangent `K + D`, and the contact statistics. The tangent is only built with `want_k`.
    #[allow(clippy::type_complexity)]
    pub fn evaluate(&self, u: &[f64], lambda: f64, scale: f64, want_k: bool) -> Result<(Vec<f64>, Option<BlockMatrix>, Vec<(Vec<usize>, Vec<f64>)>, ContactStats, Vec<Vec<CpState>>), String> {
        let mut state = self.old.clone();
        let (mut f, mut k) = assemble_nl_public(&self.model.mesh, &self.pat, u, &self.old, &mut state, want_k)?;
        let st = self.set.eval(&self.model.mesh, &self.pat, u, &[], lambda, &self.old.contact, &mut state.contact, &mut f, k.as_mut(), None)?;
        if !st.missing_pairs.is_empty() {
            return Err(format!("contact reached node pairs outside the matrix pattern: {:?}", &st.missing_pairs[..st.missing_pairs.len().min(4)]));
        }
        for (r, e) in f.iter_mut().zip(&self.f_ext) {
            *r -= lambda * scale * e;
        }
        let defect = st.defect.clone();
        Ok((f, k, defect, st, state.contact))
    }

    /// `(K + D) v` of the exact tangent at `u` (with `include_defect = false`, only the symmetric part `K`): the Jacobian action a
    /// directional Taylor test compares with differences of the residual.
    pub fn tangent_product(&self, u: &[f64], lambda: f64, v: &[f64], include_defect: bool) -> Result<Vec<f64>, String> {
        let (_, k, defect, ..) = self.evaluate(u, lambda, 1.0, true)?;
        let k = k.expect("tangent");
        let mut y = vec![0.0; v.len()];
        k.matvec_add(&self.pat, v, &mut y);
        if include_defect {
            for (dofs, mat) in &defect {
                let nt = dofs.len();
                for i in 0..nt {
                    let mut sum = 0.0;
                    for j in 0..nt {
                        sum += mat[i * nt + j] * v[dofs[j]];
                    }
                    y[dofs[i]] += sum;
                }
            }
        }
        Ok(y)
    }

    /// Solve `R(u; lambda) = 0` on the free dofs by Newton's method with the exact tangent and backtracking.
    pub fn solve(&self, lambda: f64, tol: f64, max_iter: usize) -> Result<ContactSolution, String> {
        let n = self.model.mesh.n_dofs();
        self.model.check_constrained(self.bc).ok();
        let red = Reduced::with_ordering(&self.pat, self.bc, Some(&self.model.mesh.nodes), Ordering::Auto).map_err(|e| e.to_string())?;
        let free: Vec<usize> = red.free_dofs().iter().map(|&d| d as usize).collect();
        let mut u = vec![0.0; n];
        let norm_free = |r: &[f64]| free.iter().fold(0.0f64, |m, &d| m.max(r[d].abs()));
        for it in 0..=max_iter {
            let (r, k, defect, stats, _) = self.evaluate(&u, lambda, 1.0, true)?;
            let rn = norm_free(&r);
            let scale = free.iter().map(|&d| (lambda * self.f_ext[d]).abs()).fold(0.0f64, f64::max).max(norm_free(&r)).max(1e-300);
            if rn <= tol * scale {
                return Ok(ContactSolution { u, lambda, iterations: it, residual: rn / scale, stats });
            }
            if it == max_iter {
                break;
            }
            let k = k.expect("tangent");
            let solver = TangentSolver::new(&red, &self.pat, &k, n, &free, &defect).map_err(|e| e.to_string())?;
            let dx = solver.solve(&free.iter().map(|&d| r[d]).collect::<Vec<_>>())?;
            // backtracking on the residual norm
            let mut alpha = 1.0;
            loop {
                let ut: Vec<f64> = {
                    let mut t = u.clone();
                    for (j, &d) in free.iter().enumerate() {
                        t[d] -= alpha * dx[j];
                    }
                    t
                };
                let rt = self.evaluate(&ut, lambda, 1.0, false).map(|e| norm_free(&e.0));
                if let Ok(v) = rt {
                    if v < rn || alpha < 1e-4 {
                        u = ut;
                        break;
                    }
                }
                alpha *= 0.5;
                if alpha < 1e-8 {
                    return Err("the penalty contact Newton iteration could not reduce the residual".into());
                }
            }
        }
        Err(format!("the penalty contact solve did not converge in {max_iter} iterations"))
    }

    /// The report of what regularisation the derivatives belong to.
    pub fn regularisation_report(&self, sol: &ContactSolution) -> RegularisationReport {
        let peak = sol.stats.master_force.iter().map(|f| (f[0] * f[0] + f[1] * f[1] + f[2] * f[2]).sqrt()).fold(0.0f64, f64::max);
        let eps: Vec<f64> = self.specs.iter().map(|s| s.eps_n).collect();
        let emin = eps.iter().cloned().fold(f64::INFINITY, f64::min);
        RegularisationReport {
            eps_n: eps,
            max_penetration: (-sol.stats.min_gap).max(0.0),
            penetration_scale: if emin.is_finite() && emin > 0.0 { peak / emin } else { 0.0 },
            note: "penalty-regularised contact law at frozen multipliers: its derivatives differ from those of the unilateral problem (g >= 0, p >= 0, p g = 0) by O(stress / eps_n)".into(),
        }
    }

    /// Sensitivities `result[q][p]` of `J_q = grad_q . u` at `sol` to `params`, direct or adjoint through the exact (possibly
    /// unsymmetric) tangent. Nothing is differentiated through a contact transition: the report carries the semantics.
    pub fn sensitivities(&self, sol: &ContactSolution, qois: &[Vec<f64>], params: &[ContactParam], method: SensitivityMethod, tol: f64) -> Result<(Vec<Vec<f64>>, ContactSensitivityReport), String> {
        let n = self.model.mesh.n_dofs();
        let (_, k, defect, stats, contact_state) = self.evaluate(&sol.u, sol.lambda, 1.0, true)?;
        let k = k.expect("tangent");
        let red = Reduced::with_ordering(&self.pat, self.bc, Some(&self.model.mesh.nodes), Ordering::Auto).map_err(|e| e.to_string())?;
        let free: Vec<usize> = red.free_dofs().iter().map(|&d| d as usize).collect();
        let solver = TangentSolver::new(&red, &self.pat, &k, n, &free, &defect).map_err(|e| e.to_string())?;
        let restrict = |v: &[f64]| -> Vec<f64> { free.iter().map(|&d| v[d]).collect() };
        // dR/dp for each parameter
        let engine_loads = Loads::default();
        let lin = super::linear_sens::LinearSensitivityEngine::new(self.model, &engine_loads, self.bc);
        let drdp: Vec<Vec<f64>> = params
            .iter()
            .map(|&p| match p {
                ContactParam::LoadScale => Ok(self.f_ext.iter().map(|v| -sol.lambda * v).collect()),
                ContactParam::LoadFactor => {
                    // dR/dlambda = d f_contact / d lambda (the rigid master's travel; the gap is linear in lambda) - f_ext
                    let h = 1e-6 * sol.lambda.abs().max(1.0);
                    let rp = self.evaluate(&sol.u, sol.lambda + h, 1.0, false)?.0;
                    let rm = self.evaluate(&sol.u, sol.lambda - h, 1.0, false)?.0;
                    // both include -lambda f_ext: subtract it back out and add the exact -f_ext
                    let d_total: Vec<f64> = rp.iter().zip(&rm).map(|(a, b)| (a - b) / (2.0 * h)).collect();
                    Ok(d_total)
                }
                ContactParam::ModulusScale(b) => lin.dk_u(super::linear_sens::LinearParam::ModulusScale(b), &sol.u),
            })
            .collect::<Result<_, String>>()?;
        let mut out = vec![vec![0.0; params.len()]; qois.len()];
        match method {
            SensitivityMethod::Direct => {
                for (pi, v) in drdp.iter().enumerate() {
                    let rhs: Vec<f64> = restrict(v).iter().map(|x| -x).collect();
                    let du = solver.solve(&rhs)?;
                    for (qi, g) in qois.iter().enumerate() {
                        out[qi][pi] = free.iter().zip(&du).map(|(&d, x)| g[d] * x).sum();
                    }
                }
            }
            _ => {
                for (qi, g) in qois.iter().enumerate() {
                    let lam = solver.solve_transpose(&restrict(g))?;
                    for (pi, v) in drdp.iter().enumerate() {
                        out[qi][pi] = -free.iter().zip(&lam).map(|(&d, x)| v[d] * x).sum::<f64>();
                    }
                }
            }
        }
        let state = classify_contact(&self.specs, &contact_state, tol);
        let _ = stats;
        Ok((out, ContactSensitivityReport { method, semantics: state.semantics, state, tangent_solves: solver.n_solves(), krylov_iterations: solver.n_krylov_iterations(), unsymmetric: !defect.is_empty() }))
    }
}
