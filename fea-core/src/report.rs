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

/// An explicit acceptance check. Nonfinite measured values or limits always fail.
#[derive(Debug, Clone, PartialEq)]
pub struct AcceptanceCheck {
    pub name: String,
    pub value: f64,
    pub limit: f64,
}
impl AcceptanceCheck {
    pub fn passed(&self) -> bool {
        self.value.is_finite()
            && self.value >= 0.0
            && self.limit.is_finite()
            && self.limit >= 0.0
            && self.value <= self.limit
    }
}
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AcceptanceReport {
    pub checks: Vec<AcceptanceCheck>,
    pub diagnostics: Vec<String>,
}
impl AcceptanceReport {
    pub fn passed(&self) -> bool {
        !self.checks.is_empty() && self.checks.iter().all(AcceptanceCheck::passed)
    }
    pub fn require(&self) -> Result<(), String> {
        if self.passed() {
            Ok(())
        } else {
            Err(format!(
                "acceptance failed: {:?}; {:?}",
                self.checks
                    .iter()
                    .filter(|c| !c.passed())
                    .collect::<Vec<_>>(),
                self.diagnostics
            ))
        }
    }
    pub fn json(&self) -> String {
        let checks = self
            .checks
            .iter()
            .map(|c| {
                format!(
                    "{{\"name\":{},\"value\":{},\"limit\":{},\"passed\":{}}}",
                    json_str(&c.name),
                    num(c.value),
                    num(c.limit),
                    c.passed()
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        let diagnostics = self
            .diagnostics
            .iter()
            .map(|s| json_str(s))
            .collect::<Vec<_>>()
            .join(",");
        format!(
            "{{\"passed\":{},\"checks\":[{}],\"diagnostics\":[{}]}}",
            self.passed(),
            checks,
            diagnostics
        )
    }
}

/// Only request mechanical-energy conservation for an undamped, unloaded average-acceleration
/// run. HHT/damped unloaded runs may instead request an upper bound by the initial energy.
#[derive(Debug, Clone, Copy)]
pub enum TransientEnergy {
    Conserved { tol: f64 },
    BoundedByInitial { tol: f64 },
}
impl AcceptanceReport {
    pub fn modal(result: &crate::Modal, tol: f64) -> Self {
        let mut checks = Vec::new();
        for (i, mode) in result.modes.iter().enumerate() {
            checks.push(AcceptanceCheck {
                name: format!("mode {i} residual"),
                value: mode.residual,
                limit: tol,
            });
            checks.push(AcceptanceCheck {
                name: format!("mode {i} finite state"),
                value: if !mode.shape.is_empty()
                    && mode
                        .shape
                        .iter()
                        .chain(&mode.effective_mass)
                        .all(|v| v.is_finite())
                    && mode.omega2.is_finite()
                    && mode.frequency_hz.is_finite()
                {
                    0.0
                } else {
                    f64::INFINITY
                },
                limit: 0.0,
            });
        }
        Self {
            checks,
            diagnostics: vec![format!(
                "eigen iterations {}, subspace {}, fallback {:?}",
                result.iterations, result.subspace, result.fallback
            )],
        }
    }
    pub fn heat(result: &crate::HeatSolution, tol: f64) -> Self {
        Self {
            checks: vec![
                AcceptanceCheck {
                    name: "thermal residual".into(),
                    value: result.rel_residual,
                    limit: tol,
                },
                AcceptanceCheck {
                    name: "heat balance (roundoff allowance)".into(),
                    value: result.accepted_balance_error,
                    limit: tol,
                },
                AcceptanceCheck {
                    name: "finite temperature".into(),
                    value: if !result.temperature.is_empty()
                        && result.temperature.iter().all(|v| v.is_finite())
                        && [
                            result.heat_in,
                            result.heat_out_fixed,
                            result.heat_out_convection,
                        ]
                        .iter()
                        .all(|v| v.is_finite())
                    {
                        0.0
                    } else {
                        f64::INFINITY
                    },
                    limit: 0.0,
                },
            ],
            diagnostics: vec![format!(
                "raw heat balance {:.6e}, iterations {}",
                result.balance_error(),
                result.iterations
            )],
        }
    }
    pub fn transient(result: &crate::Transient, tol: f64, energy: Option<TransientEnergy>) -> Self {
        let finite = result
            .u
            .iter()
            .chain(&result.v)
            .chain(&result.energy)
            .chain(&result.times)
            .chain(result.history.iter().flatten())
            .all(|v| v.is_finite())
            && result.energy.iter().all(|e| *e >= 0.0);
        let mut checks = vec![AcceptanceCheck {
            name: "finite transient state".into(),
            value: if finite { 0.0 } else { f64::INFINITY },
            limit: 0.0,
        }];
        for (step, &value) in result.residuals.iter().enumerate() {
            checks.push(AcceptanceCheck {
                name: format!("step {step} effective residual"),
                value,
                limit: tol,
            });
        }
        let lengths = !result.times.is_empty()
            && result.residuals.len() == result.times.len()
            && result.energy.len() == result.times.len()
            && result.history.iter().all(|h| h.len() == result.times.len());
        checks.push(AcceptanceCheck {
            name: "complete transient history".into(),
            value: if lengths { 0.0 } else { f64::INFINITY },
            limit: 0.0,
        });
        if let Some(requirement) = energy {
            let initial = result.energy.first().copied().unwrap_or(f64::NAN);
            let (name, limit) = match requirement {
                TransientEnergy::Conserved { tol } => ("conserved mechanical energy", tol),
                TransientEnergy::BoundedByInitial { tol } => {
                    ("mechanical energy bounded by initial", tol)
                }
            };
            let value = if !initial.is_finite() || initial < 0.0 || !finite {
                f64::INFINITY
            } else {
                result
                    .energy
                    .iter()
                    .map(|e| match requirement {
                        TransientEnergy::Conserved { .. } => (e - initial).abs(),
                        TransientEnergy::BoundedByInitial { .. } => (e - initial).max(0.0),
                    })
                    .fold(0.0_f64, f64::max)
                    / initial.max(1e-300)
            };
            checks.push(AcceptanceCheck {
                name: name.into(),
                value,
                limit,
            });
        }
        Self {checks,diagnostics:vec!["Energy acceptance is conditional on the caller's declared loading/integration regime".into()]}
    }
}

impl AcceptanceReport {
    pub fn buckling(result: &crate::Buckling, tol: f64) -> Self {
        let mut checks = Vec::new();
        for (i, mode) in result.modes.iter().enumerate() {
            checks.push(AcceptanceCheck {
                name: format!("buckling mode {i} residual"),
                value: mode.residual,
                limit: tol,
            });
            let valid = mode.load_factor.is_finite()
                && mode.load_factor > 0.0
                && !mode.shape.is_empty()
                && mode.shape.iter().all(|v| v.is_finite());
            checks.push(AcceptanceCheck {
                name: format!("buckling mode {i} positive finite state"),
                value: if valid { 0.0 } else { f64::INFINITY },
                limit: 0.0,
            });
        }
        Self {
            checks,
            diagnostics: vec![format!(
                "eigen iterations {}, fallback {:?}",
                result.iterations, result.fallback
            )],
        }
    }
    pub fn fields(result: &crate::fields::FieldResult, tol: f64) -> Self {
        let valid = result.values.len() == result.map.n_dofs()
            && result.reactions.len() == result.map.n_dofs()
            && result
                .values
                .iter()
                .chain(&result.reactions)
                .all(|v| v.is_finite())
            && result.quadratic_energy.is_finite();
        Self {checks:vec![AcceptanceCheck {name:"component-wise field residual".into(),value:result.rel_residual,limit:tol},
            AcceptanceCheck {name:"finite field state/energy and metadata".into(),value:if valid {0.0}else{f64::INFINITY},limit:0.0}],diagnostics:vec!["Quadratic energy is a diagnostic; constitutive/physical benchmarks are independent acceptance evidence".into()]}
    }
}
