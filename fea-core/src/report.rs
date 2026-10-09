//! Solve reports: what a solve did and why, as data. The nonlinear driver records its decisions (step cuts, held active
//! sets, accepted stalls, indefinite factorisations, widened contact margins) as [`SolveEvent`]s next to the per-step
//! residual histories; [`NlSolution::report_json`](crate::NlSolution::report_json) renders both deterministically, so a
//! run can be reproduced, compared and audited without the `NL_*` environment traces. Also the conditioning estimate of
//! a constrained stiffness matrix.

use crate::analysis::Model;
use crate::linear::{Dirichlet, Reduced};
use std::fmt::Write;

/// A decision or notable event of a solve, in the order it happened.
#[derive(Debug, Clone, PartialEq)]
pub struct SolveEvent {
    pub kind: EventKind,
    /// Load factor the event happened at (the target of a failed step for a cut).
    pub lambda: f64,
    /// Residual norm when it applies (`NaN` otherwise).
    pub residual: f64,
    pub detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventKind {
    /// A load / arc step failed and was halved.
    StepCut,
    /// A stagnating frictional Newton solve froze its active set and went on.
    ActiveSetHeld,
    /// A solve was accepted on its lowest iterate under `NlOptions::stall_tol`, not on the tolerance.
    StallAccepted,
    /// The tangent was not positive definite and the indefinite factorisation was used.
    IndefiniteTangent,
    /// A node pair left the contact matrix pattern: the whole analysis was repeated with doubled margins.
    MarginWidened,
    /// The adaptive protocol changed strategy (chose a cheaper path, reverted it, or escalated a ladder rung).
    StrategyChange,
    /// The augmented-Lagrangian passes of an accepted step ended without meeting `outer_tol` (the last pass failed, or `max_outer` ran out).
    OuterNotConverged,
    /// A validation warning the user should see with the result.
    Warning,
}

impl EventKind {
    pub fn name(self) -> &'static str {
        match self {
            EventKind::StepCut => "step_cut",
            EventKind::ActiveSetHeld => "active_set_held",
            EventKind::StallAccepted => "stall_accepted",
            EventKind::IndefiniteTangent => "indefinite_tangent",
            EventKind::MarginWidened => "margin_widened",
            EventKind::StrategyChange => "strategy_change",
            EventKind::OuterNotConverged => "outer_not_converged",
            EventKind::Warning => "warning",
        }
    }
}

/// Where the time of a nonlinear solve went (milliseconds).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Profile {
    pub evaluation_ms: f64,
    /// Of `evaluation_ms`: element assembly and contact evaluation.
    pub assembly_ms: f64,
    pub contact_ms: f64,
    pub factorisation_ms: f64,
    pub solve_ms: f64,
}

/// Estimated spectral condition number of the constrained stiffness matrix `K_ff`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Condition {
    /// Rayleigh-quotient estimates (power iteration: a lower bound of the largest eigenvalue; inverse iteration: an
    /// upper bound of the smallest), so `cond` is a slight underestimate that converges from below.
    pub lambda_max: f64,
    pub lambda_min: f64,
    pub cond: f64,
    pub iterations: usize,
    pub converged: bool,
}

impl Condition {
    /// Digits of a double lost to the conditioning: a solve of this matrix carries a relative error of order
    /// `cond * 1e-16`.
    pub fn digits_lost(&self) -> f64 {
        self.cond.max(1.0).log10()
    }
}

fn dot(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

impl Model {
    /// Condition number estimate of the constrained stiffness (power iteration for the largest eigenvalue, inverse
    /// iteration on the Cholesky factor for the smallest; `tol` is the relative change of the Rayleigh quotient
    /// accepted as converged). Cost: one factorisation plus a few dozen solves and products.
    pub fn condition_estimate(&self, bc: &Dirichlet, tol: f64, max_iter: usize) -> Result<Condition, String> {
        let k = self.assemble()?;
        let red = Reduced::with_ordering(&self.pattern, bc, Some(&self.mesh.nodes), crate::linear::Ordering::Auto).map_err(|e| e.to_string())?;
        let fac = red.factor(&k).map_err(|e| e.to_string())?;
        let m = red.n_free();
        let free = red.free_dofs().to_vec();
        let n = self.mesh.n_dofs();
        // A deterministic start vector with energy in every mode.
        let start: Vec<f64> = (0..m).map(|i| 1.0 + ((i as f64 + 1.0) * 0.618_033_988_749_895).fract()).collect();
        let normalise = |x: &mut [f64]| {
            let s = dot(x, x).sqrt();
            x.iter_mut().for_each(|v| *v /= s);
        };
        let mut full = vec![0.0; n];
        let mut ky = vec![0.0; n];
        let mut apply = |x: &[f64], out: &mut [f64]| {
            full.iter_mut().for_each(|v| *v = 0.0);
            ky.iter_mut().for_each(|v| *v = 0.0);
            for (&d, v) in free.iter().zip(x) {
                full[d as usize] = *v;
            }
            k.matvec_add(&self.pattern, &full, &mut ky);
            for (o, &d) in out.iter_mut().zip(&free) {
                *o = ky[d as usize];
            }
        };
        let (mut iterations, mut converged) = (0usize, true);
        let mut x = start.clone();
        normalise(&mut x);
        let mut y = vec![0.0; m];
        let mut lmax = 0.0;
        for it in 0..max_iter {
            apply(&x, &mut y);
            let rq = dot(&x, &y);
            let done = it > 0 && (rq - lmax).abs() <= tol * rq.abs();
            lmax = rq;
            iterations += 1;
            if done {
                break;
            }
            x.copy_from_slice(&y);
            normalise(&mut x);
            if it + 1 == max_iter {
                converged = false;
            }
        }
        let mut x = start;
        normalise(&mut x);
        let mut lmin = f64::INFINITY;
        for it in 0..max_iter {
            let mut z = x.clone();
            fac.solve_reduced(&mut z);
            // Rayleigh quotient of the inverse: x^T K^-1 x = 1 / lambda for the dominant mode of K^-1.
            let rq_inv = dot(&x, &z);
            let l = 1.0 / rq_inv;
            let done = it > 0 && (l - lmin).abs() <= tol * l.abs();
            lmin = l;
            iterations += 1;
            if done {
                break;
            }
            x = z;
            normalise(&mut x);
            if it + 1 == max_iter {
                converged = false;
            }
        }
        Ok(Condition { lambda_max: lmax, lambda_min: lmin, cond: lmax / lmin, iterations, converged })
    }
}

/// JSON number: finite values with full round-trip precision, non-finite as `null`.
pub(crate) fn num(x: f64) -> String {
    if x.is_finite() {
        format!("{x:e}")
    } else {
        "null".into()
    }
}

pub(crate) fn json_str(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    o.push('"');
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            c if (c as u32) < 0x20 => {
                let _ = write!(o, "\\u{:04x}", c as u32);
            }
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

impl crate::nonlinear::NlSolution {
    /// The whole run as JSON: outcome, counters, profile, the decisions in order and, per converged step, the load
    /// factor, cuts, iterations and residual history. Deterministic (no timestamps; times are the only run-dependent
    /// numbers and sit under `profile` / `elapsed_ms`).
    pub fn report_json(&self) -> String {
        let mut o = String::new();
        let _ = write!(o, "{{\"stop\":{},\"complete\":{},\"lambda\":{},\"factorisations\":{},\"stalled_solves\":{},\"held_solves\":{},\"elapsed_ms\":{},", json_str(&format!("{:?}", self.stop)), self.complete(), num(self.lambda), self.factorisations, self.stalled_solves, self.held_solves, num(self.elapsed_ms));
        let p = &self.profile;
        let _ = write!(o, "\"profile\":{{\"evaluation_ms\":{},\"assembly_ms\":{},\"contact_ms\":{},\"factorisation_ms\":{},\"solve_ms\":{}}},", num(p.evaluation_ms), num(p.assembly_ms), num(p.contact_ms), num(p.factorisation_ms), num(p.solve_ms));
        o.push_str("\"events\":[");
        for (i, e) in self.events.iter().enumerate() {
            if i > 0 {
                o.push(',');
            }
            let _ = write!(o, "{{\"kind\":\"{}\",\"lambda\":{},\"residual\":{},\"detail\":{}}}", e.kind.name(), num(e.lambda), num(e.residual), json_str(&e.detail));
        }
        o.push_str("],\"steps\":[");
        for (i, s) in self.steps.iter().enumerate() {
            if i > 0 {
                o.push(',');
            }
            let res: Vec<String> = s.residuals.iter().map(|r| num(*r)).collect();
            let _ = write!(o, "{{\"lambda\":{},\"iterations\":{},\"cuts\":{},\"indefinite\":{},\"max_ep\":{},\"residuals\":[{}]}}", num(s.lambda), s.iterations, s.cuts, s.indefinite, num(s.max_ep), res.join(","));
        }
        o.push_str("]}");
        o
    }

    /// Number of recorded events of `kind`.
    pub fn count_events(&self, kind: EventKind) -> usize {
        self.events.iter().filter(|e| e.kind == kind).count()
    }
}
