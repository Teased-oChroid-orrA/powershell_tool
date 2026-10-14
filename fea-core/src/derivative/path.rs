//! Direct and discrete-adjoint sensitivities of a **path-dependent** converged solution (Story E): the small-strain J2-plastic
//! response of a model loaded in steps, differentiated through the actual discrete algorithm.
//!
//! The solution is the sequence `(u_k, s_k)`, `k = 1..N`: nodal displacements and the material state of every Gauss point after load
//! step `k`, given by
//!
//! ```text
//! R_k(u_k, s_{k-1}, p) = f_int(u_k; s_{k-1}, p) - lambda_k s f_ext = 0      (free dofs)
//! s_k = S(eps(u_k), s_{k-1}, p)                                             (every Gauss point)
//! ```
//!
//! with `lambda_k = k / N`. The quantity of interest is `J = g . u_N + sum_gp c_gp . s_N,gp`. The unconverged-state shortcut of
//! differentiating only the last tangent (`adaptivity::tangent_adjoint`, exact for one step) is *wrong* for several plastic steps:
//! the history `s_{k-1}` depends on `p`. This module propagates it.
//!
//! * **Direct** (forward) sensitivities: per parameter `K_k du_k = -(R_p + dR/ds ds_{k-1})`, `ds_k = S_eps B du_k + S_s ds_{k-1} + S_p`.
//! * **Discrete adjoint** (reverse): `K_k^T mu_k = -delta_{kN} g + B^T S_eps^T nu_k`, `nu_{k-1} = S_s^T nu_k - (dR/ds)^T mu_k`,
//!   `dJ/dp = sum_k mu_k . R_p - nu_k . S_p`. The per-Gauss-point Jacobians are either recomputed on the sweep (a bounded workspace)
//!   or stored (less arithmetic, memory proportional to the number of steps): [`JacobianStrategy`].
//!
//! Every per-Gauss-point Jacobian comes from [`super::constitutive`] (forward AD of the implemented update, implicit differentiation
//! of the plane-stress condensation), so the constitutive law, its parameters and the history enter exactly.

use super::constitutive::*;
use super::linalg::TangentSolver;
use crate::analysis::Model;
use crate::kernel::{self, Work};
use crate::linear::{Dirichlet, Ordering, Reduced};
use crate::loads::{self, Loads};
use crate::mesh::Physics;
use crate::nonlinear::{assemble_nl_public, NlState};

/// A parameter of the path-dependent analysis.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NlParam {
    /// Multiplier of the elastic modulus of a block, at 1.
    ModulusScale(usize),
    PoissonRatio(usize),
    /// Multiplier of the whole yield curve of a plastic block, at 1.
    YieldScale(usize),
    /// Multiplier of the hardening (the rise of the curve over its initial stress) of a plastic block, at 1.
    HardeningScale(usize),
    /// Multiplier of the whole load, at 1.
    LoadScale,
}

/// A scalar output `J = g . u_N + sum c . s_N`.
#[derive(Debug, Clone)]
pub struct Qoi {
    /// `dJ/du_N` over all dofs.
    pub grad_u: Vec<f64>,
    /// `c` per block, per Gauss point (`element * ngp + g`), over the 7 state components; `None` for no state dependence.
    pub state_weights: Option<Vec<Vec<[f64; STATE]>>>,
}

impl Qoi {
    pub fn displacement(grad_u: Vec<f64>) -> Self {
        Self { grad_u, state_weights: None }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PathOptions {
    pub steps: usize,
    /// Relative residual of the Newton iteration of every step.
    pub tol: f64,
    pub max_iter: usize,
}

impl Default for PathOptions {
    fn default() -> Self {
        Self { steps: 4, tol: 1e-13, max_iter: 30 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JacobianStrategy {
    /// Recompute the per-Gauss-point Jacobians on the backward sweep: memory independent of the number of steps.
    Recompute,
    /// Keep them from the forward pass: no recomputation, memory proportional to steps x Gauss points.
    Store,
}

/// The converged states of every step.
#[derive(Debug, Clone)]
pub struct PathSolution {
    pub steps: usize,
    /// `u[k]` after step `k + 1`.
    pub u: Vec<Vec<f64>>,
    /// `states[k]` is the committed state at the start of step `k + 1` (`states[0]` virgin); `states[steps]` is the final one.
    pub states: Vec<NlState>,
    pub newton_iterations: usize,
    pub factorizations: usize,
}

fn dotv(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// Solve the path by Newton's method at every load step (proportional loading, no step cutting: a step that does not converge is an
/// error, so the sensitivities always belong to the sequence of steps they describe). Constraints must be homogeneous.
pub fn solve_path(model: &Model, loads: &Loads, bc: &Dirichlet, opt: &PathOptions) -> Result<PathSolution, String> {
    if bc.fixed.iter().zip(&bc.value).any(|(f, v)| *f && *v != 0.0) {
        return Err("path sensitivities need homogeneous constraints (prescribed displacements are not scaled with the load)".into());
    }
    model.check_constrained(bc)?;
    let n = model.mesh.n_dofs();
    let f_ext = loads::assemble(&model.mesh, loads)?;
    let red = Reduced::with_ordering(&model.pattern, bc, Some(&model.mesh.nodes), Ordering::Auto).map_err(|e| e.to_string())?;
    let free: Vec<usize> = red.free_dofs().iter().map(|&d| d as usize).collect();
    let mut u = vec![0.0; n];
    let mut state = NlState::new(&model.mesh);
    let mut out = PathSolution { steps: opt.steps, u: Vec::new(), states: vec![state.clone()], newton_iterations: 0, factorizations: 0 };
    for step in 1..=opt.steps {
        let lambda = step as f64 / opt.steps as f64;
        let mut converged = false;
        for _ in 0..opt.max_iter {
            let mut new = state.clone();
            let (f_int, k) = assemble_nl_public(&model.mesh, &model.pattern, &u, &state, &mut new, true)?;
            let k = k.ok_or("no tangent")?;
            let r: Vec<f64> = free.iter().map(|&d| f_int[d] - lambda * f_ext[d]).collect();
            let scale = free.iter().map(|&d| (lambda * f_ext[d]).abs().max(f_int[d].abs())).fold(0.0f64, f64::max).max(1e-300);
            let rn = r.iter().fold(0.0f64, |m, v| m.max(v.abs()));
            if rn <= opt.tol * scale {
                converged = true;
                break;
            }
            let fac = red.factor_any(&k).map_err(|e| e.to_string())?;
            out.factorizations += 1;
            let mut du = r;
            fac.solve_reduced(&mut du);
            for (j, &d) in free.iter().enumerate() {
                u[d] -= du[j];
            }
            out.newton_iterations += 1;
        }
        if !converged {
            return Err(format!("load step {step} of {} did not converge in {} iterations", opt.steps, opt.max_iter));
        }
        let mut new = state.clone();
        assemble_nl_public(&model.mesh, &model.pattern, &u, &state, &mut new, false)?;
        state = new;
        out.u.push(u.clone());
        out.states.push(state.clone());
    }
    Ok(out)
}

/// One Gauss point with everything needed to evaluate its strain and to assemble its forces.
struct Gp {
    block: usize,
    elem: usize,
    /// Position of the point in its element.
    g: usize,
    /// Integration weight (`w |J|`, thickness, `2 pi r` already in).
    w: f64,
    /// Strain per unit displacement of each of the element's dofs, tensor-shear Voigt.
    b: Vec<[f64; VOIGT]>,
    dofs: Vec<usize>,
}

/// Work conjugacy of the tensor-shear Voigt order: `f . du = w sigma^T W eps(du)`.
const W: [f64; VOIGT] = [1.0, 1.0, 1.0, 2.0, 2.0, 2.0];

enum Law {
    Elastic { plane_stress: bool },
    Plastic { plane_stress: bool, table: J2Table },
}

struct Setup<'m> {
    model: &'m Model,
    gps: Vec<Gp>,
    laws: Vec<Law>,
    free: Vec<usize>,
    f_ext: Vec<f64>,
}

impl<'m> Setup<'m> {
    fn new(model: &'m Model, loads: &Loads, red: &Reduced) -> Result<Self, String> {
        let physics = model.mesh.physics;
        let d = model.mesh.dim();
        let mut gps = Vec::new();
        let mut laws = Vec::new();
        let mut work = Work::new();
        for (bi, blk) in model.mesh.blocks.iter().enumerate() {
            if blk.material.aniso.is_some() {
                return Err(format!("block {bi}: anisotropic elasticity is not supported by the path sensitivities"));
            }
            if blk.plasticity.is_some_and(|j| j.large_strain || j.neo_hookean) {
                return Err(format!("block {bi}: only small-strain J2 plasticity is supported by the path sensitivities"));
            }
            let plane_stress = matches!(physics, Physics::PlaneStress { .. });
            laws.push(match blk.plasticity {
                Some(j2) => Law::Plastic { plane_stress, table: J2Table::new(blk.material.e, blk.material.nu, &j2.law) },
                None => Law::Elastic { plane_stress },
            });
            let t = blk.kind.table();
            let (nn, ngp) = (t.nn, t.ngp);
            for e in 0..blk.n_elems() {
                let conn = blk.elem(e);
                let xyz: Vec<[f64; 3]> = conn.iter().map(|&n| model.mesh.nodes[n]).collect();
                kernel::geometry(blk.kind, physics, &xyz, &mut work).map_err(|er| er.to_string())?;
                for g in 0..ngp {
                    let gr = &work.grad[g * nn * d..(g + 1) * nn * d];
                    let nsh = t.n_at(g);
                    let mut b = vec![[0.0; VOIGT]; nn * d];
                    for a in 0..nn {
                        for i in 0..d {
                            let col = &mut b[a * d + i];
                            let gi = |k: usize| gr[a * d + k];
                            if d == 3 {
                                col[i] = gi(i);
                                // tensor shear: eps_xy = (du_x/dy + du_y/dx) / 2 and cyclic
                                match i {
                                    0 => {
                                        col[3] = 0.5 * gi(1);
                                        col[5] = 0.5 * gi(2);
                                    }
                                    1 => {
                                        col[3] = 0.5 * gi(0);
                                        col[4] = 0.5 * gi(2);
                                    }
                                    _ => {
                                        col[4] = 0.5 * gi(1);
                                        col[5] = 0.5 * gi(0);
                                    }
                                }
                            } else {
                                col[i] = gi(i);
                                col[3] = 0.5 * gi(1 - i);
                                if matches!(physics, Physics::Axisymmetric) && i == 0 {
                                    col[2] = nsh[a] / work.radius[g];
                                }
                            }
                        }
                    }
                    let dofs: Vec<usize> = conn.iter().flat_map(|&n| (0..d).map(move |i| n * d + i)).collect();
                    gps.push(Gp { block: bi, elem: e, g, w: work.wdet[g], b, dofs });
                }
            }
        }
        Ok(Self { model, gps, laws, free: red.free_dofs().iter().map(|&d| d as usize).collect(), f_ext: loads::assemble(&model.mesh, loads)? })
    }

    fn strain(&self, gp: &Gp, u: &[f64]) -> [f64; VOIGT] {
        let mut e = [0.0; VOIGT];
        for (col, &dof) in gp.b.iter().zip(&gp.dofs) {
            let ud = u[dof];
            if ud != 0.0 {
                for v in 0..VOIGT {
                    e[v] += col[v] * ud;
                }
            }
        }
        e
    }

    fn sens(&self, gp: &Gp, u: &[f64], old: &[f64; STATE]) -> Result<MaterialSensitivities, String> {
        let strain = self.strain(gp, u);
        let blk = &self.model.mesh.blocks[gp.block];
        let r = match &self.laws[gp.block] {
            Law::Elastic { plane_stress: false } => sensitivities(&Hooke { e: blk.material.e, nu: blk.material.nu }, &[blk.material.e, blk.material.nu], &strain, old),
            Law::Elastic { plane_stress: true } => {
                let m = PlaneStress { inner: Hooke { e: blk.material.e, nu: blk.material.nu } };
                sensitivities(&m, &[blk.material.e, blk.material.nu], &strain, old)
            }
            Law::Plastic { plane_stress: false, table } => sensitivities(table, &table.parameters(), &strain, old),
            Law::Plastic { plane_stress: true, table } => sensitivities(&PlaneStress { inner: table.clone() }, &table.parameters(), &strain, old),
        };
        r.map_err(|e| format!("block {} element {}: {e}", gp.block, gp.elem))
    }

    /// Index into a block's parameter vector of `p` and the factor from the parameter to that entry (`d/dp = factor * d/dentry`).
    fn param_entry(&self, p: NlParam) -> Result<Option<(usize, usize, f64)>, String> {
        let check = |b: usize, plastic_only: bool| -> Result<(), String> {
            let law = self.laws.get(b).ok_or(format!("block {b} does not exist"))?;
            if plastic_only && matches!(law, Law::Elastic { .. }) {
                return Err(format!("block {b} is elastic: it has no yield curve"));
            }
            Ok(())
        };
        Ok(match p {
            NlParam::LoadScale => None,
            NlParam::ModulusScale(b) => {
                check(b, false)?;
                Some((b, 0, self.model.mesh.blocks[b].material.e))
            }
            NlParam::PoissonRatio(b) => {
                check(b, false)?;
                Some((b, 1, 1.0))
            }
            NlParam::YieldScale(b) => {
                check(b, true)?;
                Some((b, 2, 1.0))
            }
            NlParam::HardeningScale(b) => {
                check(b, true)?;
                Some((b, 3, 1.0))
            }
        })
    }

    /// `sum_gp B^T a_gp` (strain-conjugate, no weights) for a per-Gauss-point vector `a`.
    fn assemble_strain_adjoint(&self, a: &[[f64; VOIGT]]) -> Vec<f64> {
        let mut f = vec![0.0; self.model.mesh.n_dofs()];
        for (gp, av) in self.gps.iter().zip(a) {
            for (col, &dof) in gp.b.iter().zip(&gp.dofs) {
                let mut v = 0.0;
                for c in 0..VOIGT {
                    v += col[c] * av[c];
                }
                f[dof] += v;
            }
        }
        f
    }

    fn gp_states(&self, st: &NlState) -> Vec<[f64; STATE]> {
        self.gps.iter().map(|gp| st.gp[gp.block].get(gp.elem * self.model.mesh.blocks[gp.block].kind.table().ngp + gp.g).copied().unwrap_or([0.0; STATE])).collect()
    }
}

/// How a sensitivity analysis was carried out.
#[derive(Debug, Clone, PartialEq)]
pub struct PathReport {
    pub method: super::SensitivityMethod,
    pub tangent_factorizations: usize,
    pub tangent_solves: usize,
    pub jacobian_evaluations: usize,
    /// Bytes of per-Gauss-point Jacobians kept at once.
    pub jacobian_bytes: usize,
    pub strategy: JacobianStrategy,
}

/// `dJ/dp` for every quantity of interest and parameter, `result[q][p]`.
pub type SensitivityMatrix = Vec<Vec<f64>>;

fn jacobian_bytes(s: &MaterialSensitivities) -> usize {
    8 * (s.d_stress_d_strain.len() + s.d_stress_d_state.len() + s.d_stress_d_params.len() + s.d_new_state_d_strain.len() + s.d_new_state_d_state.len() + s.d_new_state_d_params.len() + VOIGT + STATE)
}

fn matvec(m: &[f64], rows: usize, cols: usize, v: &[f64]) -> Vec<f64> {
    (0..rows).map(|r| (0..cols).map(|c| m[r * cols + c] * v[c]).sum()).collect()
}

fn matvec_t(m: &[f64], rows: usize, cols: usize, v: &[f64]) -> Vec<f64> {
    (0..cols).map(|c| (0..rows).map(|r| m[r * cols + c] * v[r]).sum()).collect()
}

/// A column of a row-major `rows x cols` matrix.
fn column(m: &[f64], rows: usize, cols: usize, c: usize) -> Vec<f64> {
    (0..rows).map(|r| m[r * cols + c]).collect()
}

pub struct PathSensitivityEngine<'m> {
    setup: Setup<'m>,
    red: Reduced,
}

impl<'m> PathSensitivityEngine<'m> {
    pub fn new(model: &'m Model, loads: &Loads, bc: &Dirichlet) -> Result<Self, String> {
        let red = Reduced::with_ordering(&model.pattern, bc, Some(&model.mesh.nodes), Ordering::Auto).map_err(|e| e.to_string())?;
        let setup = Setup::new(model, loads, &red)?;
        Ok(Self { setup, red })
    }

    fn tangent<'a>(&'a self, u: &[f64], old: &NlState, k_store: &'a mut Option<crate::assembly::BlockMatrix>) -> Result<TangentSolver<'a>, String> {
        let m = self.setup.model;
        let mut new = old.clone();
        let (_, k) = assemble_nl_public(&m.mesh, &m.pattern, u, old, &mut new, true)?;
        *k_store = k;
        let k = k_store.as_ref().ok_or("no tangent")?;
        TangentSolver::new(&self.red, &m.pattern, k, m.mesh.n_dofs(), &self.setup.free, &[]).map_err(|e| e.to_string())
    }

    fn restrict(&self, v: &[f64]) -> Vec<f64> {
        self.setup.free.iter().map(|&d| v[d]).collect()
    }

    fn expand(&self, v: &[f64]) -> Vec<f64> {
        let mut out = vec![0.0; self.setup.model.mesh.n_dofs()];
        for (j, &d) in self.setup.free.iter().enumerate() {
            out[d] = v[j];
        }
        out
    }

    /// Explicit residual derivative `R_p` and the state-update derivative `S_p` of one parameter at one step.
    fn explicit_terms(&self, p: NlParam, lambda: f64, sens: &[MaterialSensitivities]) -> Result<(Vec<f64>, Vec<[f64; STATE]>), String> {
        let n = self.setup.model.mesh.n_dofs();
        match self.setup.param_entry(p)? {
            None => Ok((self.setup.f_ext.iter().map(|v| -lambda * v).collect(), vec![[0.0; STATE]; self.setup.gps.len()])),
            Some((block, entry, factor)) => {
                let np = |s: &MaterialSensitivities| s.n_params;
                let sigma_p = |gp_idx: usize, gp: &Gp| -> Option<[f64; VOIGT]> {
                    (gp.block == block).then(|| {
                        let s = &sens[gp_idx];
                        let col = column(&s.d_stress_d_params, VOIGT, np(s), entry);
                        std::array::from_fn(|i| factor * col[i])
                    })
                };
                let mut f = vec![0.0; n];
                for (gi, gp) in self.setup.gps.iter().enumerate() {
                    if let Some(s) = sigma_p(gi, gp) {
                        for (col, &dof) in gp.b.iter().zip(&gp.dofs) {
                            let mut v = 0.0;
                            for c in 0..VOIGT {
                                v += col[c] * W[c] * s[c];
                            }
                            f[dof] += gp.w * v;
                        }
                    }
                }
                let sp: Vec<[f64; STATE]> = self
                    .setup
                    .gps
                    .iter()
                    .enumerate()
                    .map(|(gi, gp)| {
                        if gp.block != block {
                            return [0.0; STATE];
                        }
                        let s = &sens[gi];
                        let col = column(&s.d_new_state_d_params, STATE, np(s), entry);
                        std::array::from_fn(|i| factor * col[i])
                    })
                    .collect();
                Ok((f, sp))
            }
        }
    }

    fn all_sens(&self, u: &[f64], old: &[[f64; STATE]]) -> Result<Vec<MaterialSensitivities>, String> {
        self.setup.gps.iter().zip(old).map(|(gp, o)| self.setup.sens(gp, u, o)).collect()
    }

    /// Direct (forward) sensitivities of every quantity for every parameter.
    pub fn direct(&self, sol: &PathSolution, params: &[NlParam], qois: &[Qoi]) -> Result<(SensitivityMatrix, PathReport), String> {
        let n_gp = self.setup.gps.len();
        let mut ds: Vec<Vec<[f64; STATE]>> = vec![vec![[0.0; STATE]; n_gp]; params.len()];
        let mut du_last = vec![vec![0.0; self.setup.model.mesh.n_dofs()]; params.len()];
        let (mut solves, mut facs, mut evals, mut peak) = (0, 0, 0, 0);
        for k in 1..=sol.steps {
            let lambda = k as f64 / sol.steps as f64;
            let u = &sol.u[k - 1];
            let old = self.setup.gp_states(&sol.states[k - 1]);
            let sens = self.all_sens(u, &old)?;
            evals += n_gp;
            peak = peak.max(sens.iter().map(jacobian_bytes).sum());
            let mut kstore = None;
            let solver = self.tangent(u, &sol.states[k - 1], &mut kstore)?;
            facs += 1;
            for (pi, &p) in params.iter().enumerate() {
                let (rp, sp) = self.explicit_terms(p, lambda, &sens)?;
                // dR/ds_{k-1} ds_{k-1}: per Gauss point, w B^T W (dsigma/ds_old) ds_old
                let hist = self.setup.assemble_stress_indexed(|gi, _| Some(matvec(&sens[gi].d_stress_d_state, VOIGT, STATE, &ds[pi][gi])));
                let rhs: Vec<f64> = self.restrict(&rp.iter().zip(&hist).map(|(a, b)| -(a + b)).collect::<Vec<_>>());
                let dur = solver.solve(&rhs)?;
                solves += 1;
                let du = self.expand(&dur);
                // state update
                let mut new_ds = vec![[0.0; STATE]; n_gp];
                for (gi, gp) in self.setup.gps.iter().enumerate() {
                    let eps_du = self.setup.strain(gp, &du);
                    let s = &sens[gi];
                    let a = matvec(&s.d_new_state_d_strain, STATE, VOIGT, &eps_du);
                    let b = matvec(&s.d_new_state_d_state, STATE, STATE, &ds[pi][gi]);
                    for i in 0..STATE {
                        new_ds[gi][i] = a[i] + b[i] + sp[gi][i];
                    }
                }
                ds[pi] = new_ds;
                du_last[pi] = du;
            }
        }
        let mut out = vec![vec![0.0; params.len()]; qois.len()];
        for (qi, q) in qois.iter().enumerate() {
            let weights = q.state_weights.as_ref().map(|w| self.flatten_weights(w));
            for pi in 0..params.len() {
                let mut v = dotv(&q.grad_u, &du_last[pi]);
                if let Some(w) = &weights {
                    for gi in 0..n_gp {
                        v += dotv(&w[gi], &ds[pi][gi]);
                    }
                }
                out[qi][pi] = v;
            }
        }
        Ok((out, PathReport { method: super::SensitivityMethod::Direct, tangent_factorizations: facs, tangent_solves: solves, jacobian_evaluations: evals, jacobian_bytes: peak, strategy: JacobianStrategy::Recompute }))
    }

    fn flatten_weights(&self, w: &[Vec<[f64; STATE]>]) -> Vec<[f64; STATE]> {
        self.setup
            .gps
            .iter()
            .map(|gp| {
                let ngp = self.setup.model.mesh.blocks[gp.block].kind.table().ngp;
                w.get(gp.block).and_then(|b| b.get(gp.elem * ngp + gp.g)).copied().unwrap_or([0.0; STATE])
            })
            .collect()
    }

    /// Discrete adjoint sensitivities of every quantity for every parameter: one backward sweep per quantity, whatever the number of
    /// parameters.
    pub fn adjoint(&self, sol: &PathSolution, params: &[NlParam], qois: &[Qoi], strategy: JacobianStrategy) -> Result<(SensitivityMatrix, PathReport), String> {
        let n_gp = self.setup.gps.len();
        let nq = qois.len();
        let weights: Vec<Option<Vec<[f64; STATE]>>> = qois.iter().map(|q| q.state_weights.as_ref().map(|w| self.flatten_weights(w))).collect();
        // nu_N = -c per quantity
        let mut nu: Vec<Vec<[f64; STATE]>> = (0..nq).map(|q| (0..n_gp).map(|gi| weights[q].as_ref().map_or([0.0; STATE], |w| std::array::from_fn(|i| -w[gi][i]))).collect()).collect();
        let mut grad = vec![vec![0.0; params.len()]; nq];
        let (mut solves, mut facs, mut evals, mut peak) = (0, 0, 0, 0);
        // optional store of the Jacobians of every step
        let mut stored: Vec<Option<Vec<MaterialSensitivities>>> = vec![None; sol.steps];
        if strategy == JacobianStrategy::Store {
            for k in 1..=sol.steps {
                let old = self.setup.gp_states(&sol.states[k - 1]);
                stored[k - 1] = Some(self.all_sens(&sol.u[k - 1], &old)?);
                evals += n_gp;
            }
            peak = stored.iter().flatten().map(|v| v.iter().map(jacobian_bytes).sum::<usize>()).sum();
        }
        for k in (1..=sol.steps).rev() {
            let lambda = k as f64 / sol.steps as f64;
            let u = &sol.u[k - 1];
            let sens_owned;
            let sens: &Vec<MaterialSensitivities> = match &stored[k - 1] {
                Some(s) => s,
                None => {
                    let old = self.setup.gp_states(&sol.states[k - 1]);
                    sens_owned = self.all_sens(u, &old)?;
                    evals += n_gp;
                    peak = peak.max(sens_owned.iter().map(jacobian_bytes).sum());
                    &sens_owned
                }
            };
            let mut kstore = None;
            let solver = self.tangent(u, &sol.states[k - 1], &mut kstore)?;
            facs += 1;
            let explicit: Vec<(Vec<f64>, Vec<[f64; STATE]>)> = params.iter().map(|&p| self.explicit_terms(p, lambda, sens)).collect::<Result<_, _>>()?;
            for q in 0..nq {
                // rhs = -delta_kN grad_u + sum_gp B^T (S_eps^T nu)
                let a: Vec<[f64; VOIGT]> = (0..n_gp).map(|gi| {
                    let t = matvec_t(&sens[gi].d_new_state_d_strain, STATE, VOIGT, &nu[q][gi]);
                    std::array::from_fn(|i| t[i])
                }).collect();
                let mut rhs_full = self.setup.assemble_strain_adjoint(&a);
                if k == sol.steps {
                    for (r, g) in rhs_full.iter_mut().zip(&qois[q].grad_u) {
                        *r -= g;
                    }
                }
                let mu_r = solver.solve_transpose(&self.restrict(&rhs_full))?;
                solves += 1;
                let mu = self.expand(&mu_r);
                for (pi, (rp, sp)) in explicit.iter().enumerate() {
                    let mut v = dotv(&mu, rp);
                    for gi in 0..n_gp {
                        v -= dotv(&nu[q][gi], &sp[gi]);
                    }
                    grad[q][pi] += v;
                }
                // nu_{k-1} = S_s^T nu_k - (dR/ds)^T mu
                let mut new_nu = vec![[0.0; STATE]; n_gp];
                for (gi, gp) in self.setup.gps.iter().enumerate() {
                    let s = &sens[gi];
                    let t1 = matvec_t(&s.d_new_state_d_state, STATE, STATE, &nu[q][gi]);
                    let eps_mu = self.setup.strain(gp, &mu);
                    let weighted: Vec<f64> = (0..VOIGT).map(|c| W[c] * eps_mu[c]).collect();
                    let t2 = matvec_t(&s.d_stress_d_state, VOIGT, STATE, &weighted);
                    for i in 0..STATE {
                        new_nu[gi][i] = t1[i] - gp.w * t2[i];
                    }
                }
                nu[q] = new_nu;
            }
        }
        Ok((grad, PathReport { method: super::SensitivityMethod::DiscreteAdjoint, tangent_factorizations: facs, tangent_solves: solves, jacobian_evaluations: evals, jacobian_bytes: peak, strategy }))
    }
}

impl Setup<'_> {
    /// Like [`Self::assemble_stress`], with the Gauss point's global index.
    fn assemble_stress_indexed(&self, stress: impl Fn(usize, &Gp) -> Option<Vec<f64>>) -> Vec<f64> {
        let mut f = vec![0.0; self.model.mesh.n_dofs()];
        for (gi, gp) in self.gps.iter().enumerate() {
            if let Some(s) = stress(gi, gp) {
                for (col, &dof) in gp.b.iter().zip(&gp.dofs) {
                    let mut v = 0.0;
                    for c in 0..VOIGT {
                        v += col[c] * W[c] * s[c];
                    }
                    f[dof] += gp.w * v;
                }
            }
        }
        f
    }
}
