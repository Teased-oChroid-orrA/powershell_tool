//! Adaptive solution strategy: start from the most complete formulation, take the cheaper path only when its result can
//! be verified, and go back when it cannot.
//!
//! Linear statics ([`Model::solve_adaptive`]): validate the model; solve by the reference (sparse direct) or, where the
//! predicted cost justifies it, the iterative path; verify the result with checks that do not depend on the solver
//! (a-posteriori residual and global force balance, optionally the ZZ discretisation-error estimate); on a failed
//! verification restore the direct solve. Nonlinear analyses ([`Model::solve_nonlinear_ladder`]): an ordered ladder of
//! option sets from cheapest to most robust, escalated when a rung fails or its result fails verification. Every decision
//! is a [`SolveEvent`]; nothing is substituted silently, and a result that cannot be verified is an error, not a result.

use crate::analysis::{Model, Solution, SolveMethod, AUTO_ANALYSE_DOFS, AUTO_ITERATIVE_FACTOR_NNZ};
use crate::contact::ContactSpec;
use crate::linear::Dirichlet;
use crate::loads::{self, Loads};
use crate::nonlinear::{NlOptions, NlSolution};
use crate::report::{num, EventKind, SolveEvent};
use crate::validate::{errors_of, Issue, Severity};
use std::fmt::Write;
use std::time::Instant;

/// What the caller requires of the answer.
#[derive(Debug, Clone)]
pub struct Requirements {
    /// Largest accepted `|K_ff u - f_f| / |f_f|` and global force-balance error.
    pub residual_tol: f64,
    /// Largest accepted ZZ relative energy-norm error estimate (`None`: report it, do not require it). The kernel cannot
    /// remesh by itself: a failed discretisation requirement is an error that says so (`adapt::refine` does the loop for
    /// callers that own a mesher).
    pub max_zz_error: Option<f64>,
    /// Allow the iterative solver where it is predicted cheaper (verified, and reverted when verification fails).
    pub allow_iterative: bool,
    /// Use this method instead of choosing one. A forced reduction is verified like a chosen one and reverted to the direct
    /// solve when its result fails, so forcing can cost time but never accuracy.
    pub force_method: Option<SolveMethod>,
}

impl Default for Requirements {
    fn default() -> Self {
        Self { residual_tol: 1e-8, max_zz_error: None, allow_iterative: true, force_method: None }
    }
}

/// Independent checks of a linear static result.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Verification {
    /// `|K_ff u - f_f| / |f_f|`.
    pub rel_residual: f64,
    /// `|sum of reactions + sum of loads| / sum |loads|` over the translational components (global force balance).
    pub equilibrium: f64,
    /// ZZ relative energy-norm error estimate (`None` when not computed).
    pub zz_error: Option<f64>,
    pub passed: bool,
}

#[derive(Debug, Clone)]
pub struct Adaptive {
    pub solution: Solution,
    pub issues: Vec<Issue>,
    /// The decisions taken, in order.
    pub events: Vec<SolveEvent>,
    pub verification: Verification,
    /// The strategy that produced `solution` (after any reversion).
    pub method: SolveMethod,
    pub elapsed_ms: f64,
}

/// Does the iterative solver pay off? A 3D system whose Cholesky factor (known exactly from the symbolic analysis, before
/// any numeric work) holds [`AUTO_ITERATIVE_FACTOR_NNZ`] entries or more; the threshold is the measured crossover.
pub fn iterative_pays_off(dim: usize, n_free: usize, factor_nnz: usize) -> bool {
    dim == 3 && n_free >= AUTO_ANALYSE_DOFS && factor_nnz >= AUTO_ITERATIVE_FACTOR_NNZ
}

fn equilibrium_error(model: &Model, loads: &Loads, sol: &Solution) -> Result<f64, String> {
    let f = loads::assemble(&model.mesh, loads)?;
    let d = model.mesh.dim();
    let first = usize::from(matches!(model.mesh.physics, crate::mesh::Physics::Axisymmetric)); // radial motion is not rigid
    let (mut sr, mut sf, mut sabs) = (vec![0.0; d], vec![0.0; d], 0.0);
    for i in 0..f.len() {
        sr[i % d] += sol.reactions[i];
        sf[i % d] += f[i];
        sabs += f[i].abs();
    }
    // (Weak spots: a prescribed displacement alone also loads the structure; the reactions then carry the whole balance,
    // and the sum of loads is zero, so the check degenerates to |sum of reactions| / reaction scale.)
    let rabs: f64 = sol.reactions.iter().map(|v| v.abs()).sum();
    let scale = sabs.max(rabs).max(1e-300);
    Ok((first..d).map(|c| (sr[c] + sf[c]).abs()).fold(0.0, f64::max) / scale)
}

impl Model {
    /// Linear static solve under the adaptive protocol (see the module documentation).
    pub fn solve_adaptive(&self, loads: &Loads, bc: &Dirichlet, req: &Requirements) -> Result<Adaptive, String> {
        let clock = Instant::now();
        if !req.residual_tol.is_finite() || req.residual_tol <= 0.0 || req.max_zz_error.is_some_and(|e| !e.is_finite() || e < 0.0) {
            return Err("adaptive solve: finite positive residual tolerance and finite nonnegative ZZ limit required".into());
        }
        if req.max_zz_error.is_some() && loads.temperature.is_some() {
            return Err("adaptive solve: ZZ verification of nodal temperature fields is not supported".into());
        }
        let issues = self.validate(loads, bc);
        errors_of(&issues)?;
        let mut events = Vec::new();
        let d = self.mesh.dim();
        let n_free = self.mesh.n_dofs() - bc.n_fixed();
        // Candidate reduction: the iterative solver for a large 3D system, where the direct factorisation is predicted to cost
        // more. (Small and 2D systems: the direct solve is both the reference and the cheapest, nothing to decide.)
        let mut method = SolveMethod::Direct;
        if let Some(forced) = req.force_method {
            method = forced.resolve(d, n_free);
            if method != SolveMethod::Direct {
                events.push(SolveEvent { kind: EventKind::StrategyChange, lambda: 0.0, residual: f64::NAN, detail: format!("method forced by the caller: {method:?}") });
            }
        } else if req.allow_iterative && d == 3 && n_free >= AUTO_ANALYSE_DOFS {
            let nnz = crate::linear::Reduced::with_ordering(&self.pattern, bc, Some(&self.mesh.nodes), crate::linear::Ordering::Auto).map_err(|e| e.to_string())?.factor_nnz();
            if iterative_pays_off(d, n_free, nnz) {
                method = SolveMethod::Iterative { tol: (req.residual_tol * 0.1).clamp(1e-14, 1e-6), max_iter: 400 };
                events.push(SolveEvent { kind: EventKind::StrategyChange, lambda: 0.0, residual: f64::NAN, detail: format!("iterative PCG + AMG chosen over the direct solve: {n_free} free dofs, factor would hold {nnz} entries") });
            }
        }
        let verify = |sol: &Solution| -> Result<Verification, String> {
            let equilibrium = equilibrium_error(self, loads, sol)?;
            let zz_error = match req.max_zz_error {
                Some(_) => Some(self.zz_estimate(&sol.u, loads.delta_t)?),
                None => None,
            };
            let passed = sol.u.iter().chain(&sol.reactions).all(|v| v.is_finite()) && sol.rel_residual.is_finite() && equilibrium.is_finite() && zz_error.is_none_or(|e| e.is_finite()) && sol.rel_residual <= req.residual_tol && equilibrium <= req.residual_tol.max(1e-9) && zz_error.zip(req.max_zz_error).is_none_or(|(e, m)| e <= m);
            Ok(Verification { rel_residual: sol.rel_residual, equilibrium, zz_error, passed })
        };
        let mut attempt = self.solve_static_with(loads, bc, method);
        if let SolveMethod::Iterative { .. } = method {
            let bad = match &attempt {
                Err(e) => Some(format!("the iterative solve failed: {e}")),
                Ok(s) => match verify(s) {
                    Ok(v) if v.passed => None,
                    Ok(v) => Some(format!("the iterative result failed verification (residual {:.2e}, balance {:.2e})", v.rel_residual, v.equilibrium)),
                    Err(e) => Some(e),
                },
            };
            if let Some(why) = bad {
                events.push(SolveEvent { kind: EventKind::StrategyChange, lambda: 0.0, residual: f64::NAN, detail: format!("{why}: reverting to the direct solve") });
                method = SolveMethod::Direct;
                attempt = self.solve_static_with(loads, bc, SolveMethod::Direct);
            }
        }
        let solution = attempt?;
        let verification = verify(&solution)?;
        if !verification.passed {
            let zz = verification.zz_error.map_or(String::new(), |e| format!(", ZZ error estimate {e:.2e} against the allowed {:?}", req.max_zz_error));
            return Err(format!("the result does not meet the requirements: residual {:.2e}, force balance {:.2e}{zz} (allowed residual {:.1e}); refine the mesh or relax the requirement", verification.rel_residual, verification.equilibrium, req.residual_tol));
        }
        for i in issues.iter().filter(|i| i.severity == Severity::Warning) {
            events.push(SolveEvent { kind: EventKind::Warning, lambda: 0.0, residual: f64::NAN, detail: format!("{}: {}", i.code, i.message) });
        }
        Ok(Adaptive { solution, issues, events, verification, method, elapsed_ms: clock.elapsed().as_secs_f64() * 1e3 })
    }

    /// ZZ relative energy-norm error estimate of a displacement field.
    fn zz_estimate(&self, u: &[f64], delta_t: f64) -> Result<f64, String> {
        Ok(self.zz_error(u, delta_t)?.relative())
    }
}

impl Adaptive {
    pub fn report_json(&self) -> String {
        let mut o = String::new();
        let v = &self.verification;
        let _ = write!(o, "{{\"method\":{},\"n_free\":{},\"factor_nnz\":{},\"elapsed_ms\":{},\"verification\":{{\"rel_residual\":{},\"equilibrium\":{},\"zz_error\":{},\"passed\":{}}},", crate::report::json_str(&format!("{:?}", self.method)), self.solution.n_free, self.solution.factor_nnz, num(self.elapsed_ms), num(v.rel_residual), num(v.equilibrium), v.zz_error.map_or("null".into(), num), v.passed);
        o.push_str("\"issues\":[");
        for (i, s) in self.issues.iter().enumerate() {
            if i > 0 {
                o.push(',');
            }
            let _ = write!(o, "{{\"severity\":\"{:?}\",\"code\":\"{}\",\"message\":{}}}", s.severity, s.code, crate::report::json_str(&s.message));
        }
        o.push_str("],\"events\":[");
        for (i, e) in self.events.iter().enumerate() {
            if i > 0 {
                o.push(',');
            }
            let _ = write!(o, "{{\"kind\":\"{}\",\"detail\":{}}}", e.kind.name(), crate::report::json_str(&e.detail));
        }
        o.push_str("]}");
        o
    }
}

/// One rung of a nonlinear ladder.
#[derive(Clone)]
pub struct Rung {
    pub name: String,
    pub options: NlOptions,
}

/// Outcome of a ladder: the accepted solution, the index of the rung that produced it and the escalations on the way.
pub struct Ladder {
    pub solution: NlSolution,
    pub rung: usize,
    pub events: Vec<SolveEvent>,
}

impl Model {
    /// Solve a nonlinear (optionally contact) analysis with the first rung of `rungs` that both completes and passes
    /// `verify`; the rungs run from the cheapest to the most robust. A failure of the last rung is the error. `verify` is the
    /// caller's independent check (force balance, a physical bound): convergence of the Newton iteration alone is not
    /// accepted as proof of accuracy.
    pub fn solve_nonlinear_ladder(&self, loads: &Loads, bc: &Dirichlet, contacts: &[ContactSpec], rungs: &[Rung], verify: &dyn Fn(&NlSolution) -> Result<(), String>) -> Result<Ladder, String> {
        if rungs.is_empty() {
            return Err("an empty ladder".into());
        }
        let mut events = Vec::new();
        let mut last_err = String::new();
        for (i, rung) in rungs.iter().enumerate() {
            let outcome = self.solve_nonlinear_contact(loads, bc, contacts.to_vec(), &rung.options).and_then(|s| {
                if !s.complete() {
                    Err(format!("stopped: {:?}", s.stop))
                } else {
                    verify(&s).map(|_| s)
                }
            });
            match outcome {
                Ok(solution) => return Ok(Ladder { solution, rung: i, events }),
                Err(e) => {
                    events.push(SolveEvent { kind: EventKind::StrategyChange, lambda: 0.0, residual: f64::NAN, detail: format!("rung {i} ({}) rejected: {e}{}", rung.name, if i + 1 < rungs.len() { "; escalating" } else { "" }) });
                    last_err = e;
                }
            }
        }
        Err(format!("every rung of the ladder failed; the last: {last_err}"))
    }
}

/// A verified mesh pass; the caller still owns geometry and boundary/load remapping.
pub struct RefinedPass {
    pub model: Model,
    pub loads: Loads,
    pub bc: Dirichlet,
    pub solve: Adaptive,
    pub estimate: crate::recover::ZzEstimate,
}
impl crate::adapt::Pass for RefinedPass {
    fn mesh(&self) -> &crate::Mesh {
        &self.model.mesh
    }
    fn zz(&self) -> &crate::recover::ZzEstimate {
        &self.estimate
    }
}
pub struct Refined {
    pub best: RefinedPass,
    pub attempts: usize,
    pub report: crate::report::AcceptanceReport,
    pub events: Vec<SolveEvent>,
}
fn verified_pass(
    model: Model,
    loads: Loads,
    bc: Dirichlet,
    req: &Requirements,
) -> Result<RefinedPass, String> {
    if loads.temperature.is_some() {
        return Err("refinement: nodal-temperature ZZ recovery is unsupported".into());
    }
    let solve = model.solve_adaptive(
        &loads,
        &bc,
        &Requirements {
            max_zz_error: None,
            ..req.clone()
        },
    )?;
    let estimate = model.zz_error(&solve.solution.u, loads.delta_t)?;
    if !estimate.relative().is_finite() {
        return Err("refinement: nonfinite ZZ estimate".into());
    }
    Ok(RefinedPass {
        model,
        loads,
        bc,
        solve,
        estimate,
    })
}
/// Integrates validation, deterministic verified selection/fallback and bounded ZZ remeshing.
/// Every mesher callback must rebuild supports and loads on its returned mesh. Exhausting the
/// budget without meeting the final discretization requirement is an error, never success.
pub fn solve_refined(
    model: Model,
    loads: Loads,
    bc: Dirichlet,
    req: &Requirements,
    opt: &crate::adapt::AdaptOptions,
    max_passes: usize,
    quad_split: bool,
    mut mesh_next: impl FnMut(&crate::adapt::SizeField) -> Result<(Model, Loads, Dirichlet), String>,
) -> Result<Refined, String> {
    if [
        opt.target_rel_error,
        opt.order,
        opt.min_factor,
        opt.max_factor,
        opt.grading,
        opt.h_min,
    ]
    .iter()
    .any(|v| !v.is_finite())
        || opt.target_rel_error <= 0.0
        || opt.order <= 0.0
        || opt.min_factor <= 0.0
        || opt.min_factor > 1.0
        || opt.max_factor < 1.0
        || opt.grading < 0.0
        || opt.h_min < 0.0
        || opt.h_max.is_nan()
        || opt.h_max <= opt.h_min
    {
        return Err("refinement: invalid size-field options".into());
    }
    let first = verified_pass(model, loads, bc, req)?;
    let mut attempts = 0;
    let mut events = Vec::new();
    let best = crate::adapt::refine(
        first,
        max_passes,
        quad_split,
        opt,
        |field| {
            let (model, loads, bc) = mesh_next(field)?;
            verified_pass(model, loads, bc, req)
        },
        |pass| {
            attempts += 1;
            events.extend(pass.solve.events.clone());
            events.push(SolveEvent {
                kind: EventKind::StrategyChange,
                lambda: 0.0,
                residual: pass.solve.verification.rel_residual,
                detail: format!(
                    "mesh attempt {attempts}: {} DOFs, ZZ {:.6e}, verified {:?}",
                    pass.model.mesh.n_dofs(),
                    pass.estimate.relative(),
                    pass.solve.method
                ),
            });
        },
    )?;
    let v = best.solve.verification;
    let report = crate::report::AcceptanceReport {
        checks: vec![
            crate::report::AcceptanceCheck {
                name: "residual".into(),
                value: v.rel_residual,
                limit: req.residual_tol,
            },
            crate::report::AcceptanceCheck {
                name: "equilibrium".into(),
                value: v.equilibrium,
                limit: req.residual_tol.max(1e-9),
            },
            crate::report::AcceptanceCheck {
                name: "ZZ discretization".into(),
                value: best.estimate.relative(),
                limit: req.max_zz_error.unwrap_or(opt.target_rel_error),
            },
        ],
        diagnostics: events.iter().map(|e| e.detail.clone()).collect(),
    };
    report.require().map_err(|e| {
        format!("refinement after {attempts} attempts (budget {max_passes} additional): {e}")
    })?;
    Ok(Refined {
        best,
        attempts,
        report,
        events,
    })
}

/// Strict numerical acceptance in addition to the caller's independent physical benchmark.
/// The legacy `complete()` includes engineering stop conditions; this API requires Completed.
/// Stall/unfinished outer passes are rejected rather than certified by a permissive callback.
impl Model {
    pub fn solve_nonlinear_verified(
        &self,
        loads: &Loads,
        bc: &Dirichlet,
        contacts: &[ContactSpec],
        rungs: &[Rung],
        residual_tol: f64,
        physical: &dyn Fn(&NlSolution) -> Result<(), String>,
    ) -> Result<Ladder, String> {
        if !residual_tol.is_finite() || residual_tol <= 0.0 {
            return Err("nonlinear acceptance: finite positive tolerance required".into());
        }
        let mut issues = self.validate(loads, bc);
        if !contacts.is_empty() || !loads.ground.is_empty() {
            issues.retain(|i| i.code != "rigid_body");
        }
        errors_of(&issues)?;
        if rungs.iter().any(|r| {
            !r.options.tol.is_finite()
                || r.options.tol <= 0.0
                || r.options.steps == 0
                || r.options.max_iter == 0
                || !r.options.outer_tol.is_finite()
                || r.options.outer_tol <= 0.0
                || r.options.max_outer == 0
        }) {
            return Err("nonlinear acceptance: invalid rung convergence options".into());
        }
        let f = loads::assemble(&self.mesh, loads)?;
        self.solve_nonlinear_ladder(loads, bc, contacts, rungs, &|sol| {
            if sol.stop != crate::nonlinear::Stop::Completed
                || sol.stalled_solves != 0
                || sol
                    .events
                    .iter()
                    .any(|e| e.kind == EventKind::OuterNotConverged)
            {
                return Err(format!(
                    "nonlinear acceptance: termination {:?}, {} stalls or unfinished outer passes",
                    sol.stop, sol.stalled_solves
                ));
            }
            if sol.u.iter().chain(&sol.reactions).any(|v| !v.is_finite())
                || !sol.lambda.is_finite()
                || sol.steps.is_empty()
            {
                return Err("nonlinear acceptance: missing steps or nonfinite state".into());
            }
            let scale = f
                .iter()
                .map(|x| sol.lambda * x.abs())
                .chain(sol.reactions.iter().map(|x| x.abs()))
                .fold(0.0_f64, f64::max)
                .max(1e-300);
            let last = sol
                .steps
                .last()
                .and_then(|s| s.residuals.last())
                .copied()
                .ok_or("nonlinear acceptance: missing final residual")?;
            if !last.is_finite() || last / scale > residual_tol {
                return Err(format!(
                    "nonlinear acceptance: residual {:.3e} exceeds {residual_tol:e}",
                    last / scale
                ));
            }
            physical(sol)
        })
    }
}
