//! Sensitivities of the linear transient response (Story J): the Newmark / HHT-alpha integrator of [`Model::transient`] differentiated
//! **through the time steps** (discrete sensitivities: the derivative of the computed response, not of the continuous problem).
//!
//! One step maps the state `(u, v, a)` to the next one through the predictors `u_p = u + dt v + dt^2 (1/2 - beta) a`,
//! `v_p = v + dt (1 - gamma) a`, the effective system `(M + (1+alpha) gamma dt C + (1+alpha) beta dt^2 K) a' = b` with
//! `b = (1+alpha) f' - alpha f - (1+alpha)(C v_p + K u_p) + alpha (C v + K u)` and the correctors `u' = u_p + beta dt^2 a'`,
//! `v' = v_p + gamma dt a'`. The parameter enters through `K`, `M`, `C = a0 M + a1 K` and `f`; their derivatives are exact
//! element-level dual-number products (the matrices are never differentiated as assembled arrays).
//!
//! * **Direct** sensitivities propagate `(du, dv, da)` per parameter in lock step with the response: one effective solve per
//!   step and parameter, no storage of the trajectory.
//! * **Discrete adjoint** sweeps the steps backwards for each quantity of interest: one effective solve per step and quantity.
//!   The forward states are either all kept ([`Storage::Store`]) or recomputed from checkpoints every `interval` steps
//!   ([`Storage::Checkpoint`]); both give the same bits because the forward step is deterministic.
//!
//! The effective matrix is symmetric (Rayleigh damping is a combination of `K` and `M`), so the adjoint step uses the same
//! factorisation. The response is the one of `Model::transient` (the tests compare them); constraints must be homogeneous.

use super::dual::Dual;
use super::element;
use super::linear_sens::{LinearParam, LinearSensitivityEngine};
use super::scalar::Scalar;
use crate::analysis::Model;
use crate::assembly::BlockMatrix;
use crate::dynamics::{MassKind, TransientOptions};
use crate::linear::{Dirichlet, Ordering, Reduced};
use crate::loads::{self, Loads};
use crate::mesh::Physics;
use crate::sensitivity::Param;

type D1 = Dual<f64, 1>;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TransientParam {
    /// Any parameter of the static engine that also reaches the mass through the geometry (`Thickness`, `Coord`) or only the
    /// stiffness (`ModulusScale`, `PoissonRatio`).
    Static(LinearParam),
    /// Multiplier of the density of a block, at 1.
    DensityScale(usize),
    /// The mass-proportional Rayleigh coefficient `a0` of `C = a0 M + a1 K`.
    RayleighMass,
    /// The stiffness-proportional Rayleigh coefficient `a1`.
    RayleighStiffness,
}

/// How the adjoint obtains the forward states it needs in reverse order.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Storage {
    Store,
    /// Keep every `interval`-th state and recompute the steps in between, one segment at a time.
    Checkpoint(usize),
}

/// `J = sum over samples of (gu . u_step + gv . v_step)`.
#[derive(Debug, Clone, Default)]
pub struct TransientQoi {
    pub samples: Vec<Sample>,
}

#[derive(Debug, Clone)]
pub struct Sample {
    pub step: usize,
    pub gu: Vec<f64>,
    pub gv: Vec<f64>,
}

impl TransientQoi {
    pub fn displacement(step: usize, dof: usize, n: usize) -> Self {
        let mut gu = vec![0.0; n];
        gu[dof] = 1.0;
        Self { samples: vec![Sample { step, gu, gv: vec![0.0; n] }] }
    }
    /// The time integral of one displacement by the trapezoid rule over all steps.
    pub fn displacement_integral(dof: usize, steps: usize, dt: f64, n: usize) -> Self {
        let samples = (0..=steps)
            .map(|s| {
                let mut gu = vec![0.0; n];
                gu[dof] = dt * if s == 0 || s == steps { 0.5 } else { 1.0 };
                Sample { step: s, gu, gv: vec![0.0; n] }
            })
            .collect();
        Self { samples }
    }
    fn at(&self, step: usize) -> impl Iterator<Item = &Sample> {
        self.samples.iter().filter(move |s| s.step == step)
    }
}

#[derive(Debug, Clone)]
pub struct State {
    pub u: Vec<f64>,
    pub v: Vec<f64>,
    pub a: Vec<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TransientReport {
    pub effective_solves: usize,
    /// Forward states held at the peak (`steps + 1` for `Store`).
    pub peak_states: usize,
}

pub struct TransientSensitivity<'m> {
    model: &'m Model,
    loads: &'m Loads,
    bc: &'m Dirichlet,
    g: &'m dyn Fn(f64) -> f64,
    opt: &'m TransientOptions,
    init: Option<(&'m [f64], &'m [f64])>,
}

struct Ops<'a> {
    e: &'a TransientSensitivity<'a>,
    k: BlockMatrix,
    mass: BlockMatrix,
    free: Vec<usize>,
    f_ref: Vec<f64>,
    beta: f64,
    gamma: f64,
}

impl<'m> TransientSensitivity<'m> {
    pub fn new(model: &'m Model, loads: &'m Loads, g: &'m dyn Fn(f64) -> f64, bc: &'m Dirichlet, init: Option<(&'m [f64], &'m [f64])>, opt: &'m TransientOptions) -> Self {
        Self { model, loads, bc, g, opt, init }
    }

    fn thickness(&self) -> f64 {
        match self.model.mesh.physics {
            Physics::PlaneStress { thickness } | Physics::PlaneStrain { thickness } => thickness,
            _ => 1.0,
        }
    }

    fn ops(&'m self) -> Result<Ops<'m>, String> {
        let (m, opt) = (self.model, self.opt);
        if !opt.dt.is_finite() || opt.dt <= 0.0 || !(-1.0 / 3.0 - 1e-12..=1e-12).contains(&opt.alpha) {
            return Err("transient sensitivities: dt must be positive and alpha within [-1/3, 0]".into());
        }
        if self.bc.fixed.iter().zip(&self.bc.value).any(|(f, v)| *f && *v != 0.0) {
            return Err("transient sensitivities: prescribed non-zero displacements are not supported".into());
        }
        m.check_constrained(self.bc)?;
        let k = m.assemble()?;
        let mass = m.assemble_mass(opt.mass)?;
        let red = Reduced::with_ordering(&m.pattern, self.bc, Some(&m.mesh.nodes), Ordering::Auto).map_err(|e| e.to_string())?;
        let free: Vec<usize> = red.free_dofs().iter().map(|&d| d as usize).collect();
        let f_ref = loads::assemble(&m.mesh, self.loads)?;
        let alpha = opt.alpha;
        Ok(Ops { e: self, k, mass, free, f_ref, beta: (1.0 - alpha) * (1.0 - alpha) / 4.0, gamma: 0.5 - alpha })
    }

    /// `(dM/dp) x` with exact element derivatives; the Rayleigh scalars have no mass derivative.
    pub fn dm_x(&self, p: TransientParam, x: &[f64]) -> Result<Vec<f64>, String> {
        let m = self.model;
        let d = m.mesh.dim();
        let mut out = vec![0.0; m.mesh.n_dofs()];
        let (density_block, st) = match p {
            TransientParam::DensityScale(b) => (Some(b), None),
            TransientParam::Static(s @ (LinearParam::Thickness | LinearParam::Coord { .. })) => (None, Some(s)),
            _ => return Ok(out),
        };
        if matches!(st, Some(LinearParam::Thickness)) && !matches!(m.mesh.physics, Physics::PlaneStress { .. } | Physics::PlaneStrain { .. }) {
            return Err("the thickness is a parameter of plane analyses only".into());
        }
        for (bi, blk) in m.mesh.blocks.iter().enumerate() {
            if density_block.is_some_and(|b| b != bi) {
                continue;
            }
            let nn = blk.kind.n_nodes();
            for e in 0..blk.n_elems() {
                let conn = blk.elem(e);
                let mut coords: Vec<[D1; 3]> = conn.iter().map(|&n| m.mesh.nodes[n].map(D1::cst)).collect();
                let mut rho = D1::cst(blk.density);
                let mut t = D1::cst(self.thickness());
                if density_block.is_some() {
                    rho = D1::with(blk.density, [blk.density]);
                }
                match st {
                    Some(LinearParam::Thickness) => t = D1::var(self.thickness(), 0),
                    Some(LinearParam::Coord { node, axis }) => match conn.iter().position(|&n| n == node) {
                        Some(l) => coords[l][axis] = D1::var(m.mesh.nodes[node][axis], 0),
                        None => continue,
                    },
                    _ => {}
                }
                let mut me = element::mass(blk.kind, m.mesh.physics, &coords, rho, t).map_err(|er| er.to_string())?;
                if self.opt.mass == MassKind::Lumped {
                    let total = me.iter().fold(D1::cst(0.0), |s, v| s + *v);
                    let diag = (0..nn).fold(D1::cst(0.0), |s, a| s + me[a * nn + a]);
                    let scale = total / diag;
                    for a in 0..nn {
                        for b in 0..nn {
                            me[a * nn + b] = if a == b { me[a * nn + a] * scale } else { D1::cst(0.0) };
                        }
                    }
                }
                for a in 0..nn {
                    for i in 0..d {
                        let mut s = 0.0;
                        for b in 0..nn {
                            s += me[a * nn + b].d[0] * x[conn[b] * d + i];
                        }
                        out[conn[a] * d + i] += s;
                    }
                }
            }
        }
        Ok(out)
    }
}

fn mul(mat: &BlockMatrix, pattern: &crate::assembly::Pattern, x: &[f64]) -> Vec<f64> {
    let mut y = vec![0.0; x.len()];
    mat.matvec_add(pattern, x, &mut y);
    y
}

impl<'m> Ops<'m> {
    fn n(&self) -> usize {
        self.e.model.mesh.n_dofs()
    }
    fn mulk(&self, x: &[f64]) -> Vec<f64> {
        mul(&self.k, &self.e.model.pattern, x)
    }
    fn mulm(&self, x: &[f64]) -> Vec<f64> {
        mul(&self.mass, &self.e.model.pattern, x)
    }
    /// `C x = a0 M x + a1 K x`.
    fn mulc(&self, x: &[f64]) -> Vec<f64> {
        let (a0, a1) = self.e.opt.rayleigh;
        let (mx, kx) = (self.mulm(x), self.mulk(x));
        mx.iter().zip(&kx).map(|(m, k)| a0 * m + a1 * k).collect()
    }
    fn restrict_free(&self, v: &mut [f64]) {
        let fixed = &self.e.bc.fixed;
        for (i, x) in v.iter_mut().enumerate() {
            if fixed[i] {
                *x = 0.0;
            }
        }
    }
    fn solve_eff(&self, fac: &crate::linear::Factor<'_>, full: &[f64]) -> Vec<f64> {
        let mut r: Vec<f64> = self.free.iter().map(|&d| full[d]).collect();
        fac.solve_reduced(&mut r);
        let mut out = vec![0.0; self.n()];
        for (&d, x) in self.free.iter().zip(&r) {
            out[d] = *x;
        }
        out
    }
    fn load(&self, t: f64) -> Vec<f64> {
        let s = (self.e.g)(t);
        self.f_ref.iter().map(|x| x * s).collect()
    }
    fn initial(&self, fac_m: &crate::linear::Factor<'_>) -> State {
        let n = self.n();
        let (mut u, mut v) = match self.e.init {
            Some((u0, v0)) => (u0.to_vec(), v0.to_vec()),
            None => (vec![0.0; n], vec![0.0; n]),
        };
        self.restrict_free(&mut u);
        self.restrict_free(&mut v);
        let f0 = self.load(0.0);
        let (cv, ku) = (self.mulc(&v), self.mulk(&u));
        let r: Vec<f64> = (0..n).map(|d| f0[d] - cv[d] - ku[d]).collect();
        let a = self.solve_eff(fac_m, &r);
        State { u, v, a }
    }
    fn predictors(&self, x: &State) -> (Vec<f64>, Vec<f64>) {
        let (dt, beta, gamma) = (self.e.opt.dt, self.beta, self.gamma);
        let up = (0..x.u.len()).map(|i| x.u[i] + dt * x.v[i] + dt * dt * (0.5 - beta) * x.a[i]).collect();
        let vp = (0..x.u.len()).map(|i| x.v[i] + dt * (1.0 - gamma) * x.a[i]).collect();
        (up, vp)
    }
    /// One forward step from `x` at `step` to `step + 1`.
    fn advance(&self, fac_eff: &crate::linear::Factor<'_>, x: &State, step: usize) -> State {
        let (alpha, dt, beta, gamma) = (self.e.opt.alpha, self.e.opt.dt, self.beta, self.gamma);
        let n = self.n();
        let (f_new, f_old) = (self.load((step + 1) as f64 * dt), self.load(step as f64 * dt));
        let (up, vp) = self.predictors(x);
        let (cvp, cv, kup, ku) = (self.mulc(&vp), self.mulc(&x.v), self.mulk(&up), self.mulk(&x.u));
        let b: Vec<f64> = (0..n).map(|i| (1.0 + alpha) * f_new[i] - alpha * f_old[i] - (1.0 + alpha) * cvp[i] + alpha * cv[i] - (1.0 + alpha) * kup[i] + alpha * ku[i]).collect();
        let a = self.solve_eff(fac_eff, &b);
        let u = (0..n).map(|i| up[i] + beta * dt * dt * a[i]).collect();
        let v = (0..n).map(|i| vp[i] + gamma * dt * a[i]).collect();
        State { u, v, a }
    }
    fn eff(&self) -> BlockMatrix {
        let (alpha, dt) = (self.e.opt.alpha, self.e.opt.dt);
        let (a0, a1) = self.e.opt.rayleigh;
        let (c0, c1) = ((1.0 + alpha) * self.gamma * dt, (1.0 + alpha) * self.beta * dt * dt);
        BlockMatrix { d: self.k.d, vals: (0..self.k.vals.len()).map(|i| self.mass.vals[i] * (1.0 + c0 * a0) + self.k.vals[i] * (c0 * a1 + c1)).collect() }
    }

    /// Everything the parameter does to one step: `(dK x, dM x)` products through the static engine and the mass derivative,
    /// plus the Rayleigh scalars' own terms. Returned as closures' values for the vectors asked for.
    fn dk(&self, lin: &LinearSensitivityEngine<'_>, p: TransientParam, x: &[f64]) -> Result<Vec<f64>, String> {
        match p {
            TransientParam::Static(s) => lin.dk_u(s, x),
            _ => Ok(vec![0.0; x.len()]),
        }
    }
    /// `(dC/dp) x`.
    fn dc(&self, lin: &LinearSensitivityEngine<'_>, p: TransientParam, x: &[f64]) -> Result<Vec<f64>, String> {
        let (a0, a1) = self.e.opt.rayleigh;
        let (dmx, dkx) = (self.e.dm_x(p, x)?, self.dk(lin, p, x)?);
        let mut out: Vec<f64> = dmx.iter().zip(&dkx).map(|(m, k)| a0 * m + a1 * k).collect();
        match p {
            TransientParam::RayleighMass => out.iter_mut().zip(self.mulm(x)).for_each(|(o, v)| *o += v),
            TransientParam::RayleighStiffness => out.iter_mut().zip(self.mulk(x)).for_each(|(o, v)| *o += v),
            _ => {}
        }
        Ok(out)
    }
    /// `(d eff/dp) x`.
    fn deff(&self, lin: &LinearSensitivityEngine<'_>, p: TransientParam, x: &[f64]) -> Result<Vec<f64>, String> {
        let (alpha, dt) = (self.e.opt.alpha, self.e.opt.dt);
        let (c0, c1) = ((1.0 + alpha) * self.gamma * dt, (1.0 + alpha) * self.beta * dt * dt);
        let (dmx, dcx, dkx) = (self.e.dm_x(p, x)?, self.dc(lin, p, x)?, self.dk(lin, p, x)?);
        Ok((0..x.len()).map(|i| dmx[i] + c0 * dcx[i] + c1 * dkx[i]).collect())
    }
    /// `df/dp` at time `t`.
    fn df(&self, p: TransientParam, t: f64) -> Result<Vec<f64>, String> {
        let m = self.e.model;
        let raw = match p {
            TransientParam::Static(LinearParam::Thickness) => m.df_dp(self.e.loads, Param::Thickness)?,
            TransientParam::Static(LinearParam::Coord { node, axis }) => m.df_dp(self.e.loads, Param::Coord { node, axis })?,
            _ => vec![0.0; self.n()],
        };
        let s = (self.e.g)(t);
        Ok(raw.iter().map(|x| x * s).collect())
    }
}

impl<'m> TransientSensitivity<'m> {
    /// The response at every step (the integrator of `Model::transient`, with the accelerations).
    pub fn response(&'m self) -> Result<Vec<State>, String> {
        let ops = self.ops()?;
        let red = Reduced::with_ordering(&self.model.pattern, self.bc, Some(&self.model.mesh.nodes), Ordering::Auto).map_err(|e| e.to_string())?;
        let (fac_eff, fac_m) = (red.factor(&ops.eff()).map_err(|e| e.to_string())?, red.factor(&ops.mass).map_err(|e| e.to_string())?);
        let mut states = vec![ops.initial(&fac_m)];
        for s in 0..self.opt.steps {
            let next = ops.advance(&fac_eff, &states[s], s);
            states.push(next);
        }
        Ok(states)
    }

    /// Direct sensitivities `result[q][p]` of the quantities to the parameters.
    pub fn direct(&'m self, params: &[TransientParam], qois: &[TransientQoi]) -> Result<(Vec<Vec<f64>>, TransientReport), String> {
        let ops = self.ops()?;
        let lin = LinearSensitivityEngine::new(self.model, self.loads, self.bc);
        let red = Reduced::with_ordering(&self.model.pattern, self.bc, Some(&self.model.mesh.nodes), Ordering::Auto).map_err(|e| e.to_string())?;
        let (fac_eff, fac_m) = (red.factor(&ops.eff()).map_err(|e| e.to_string())?, red.factor(&ops.mass).map_err(|e| e.to_string())?);
        let (alpha, dt, beta, gamma) = (self.opt.alpha, self.opt.dt, ops.beta, ops.gamma);
        let n = ops.n();
        let mut out = vec![vec![0.0; params.len()]; qois.len()];
        let mut solves = 0;
        let x0 = ops.initial(&fac_m);
        for (pi, &p) in params.iter().enumerate() {
            let mut x = x0.clone();
            // d a0 = M^-1 (df0 - dC v0 - dK u0 - dM a0)
            let (dcv, dku, dma, df0) = (ops.dc(&lin, p, &x.v)?, ops.dk(&lin, p, &x.u)?, self.dm_x(p, &x.a)?, ops.df(p, 0.0)?);
            let r: Vec<f64> = (0..n).map(|i| df0[i] - dcv[i] - dku[i] - dma[i]).collect();
            let mut dx = State { u: vec![0.0; n], v: vec![0.0; n], a: ops.solve_eff(&fac_m, &r) };
            solves += 1;
            let accumulate = |out: &mut Vec<Vec<f64>>, step: usize, dx: &State| {
                for (qi, q) in qois.iter().enumerate() {
                    for sm in q.at(step) {
                        out[qi][pi] += sm.gu.iter().zip(&dx.u).map(|(a, b)| a * b).sum::<f64>() + sm.gv.iter().zip(&dx.v).map(|(a, b)| a * b).sum::<f64>();
                    }
                }
            };
            accumulate(&mut out, 0, &dx);
            for s in 0..self.opt.steps {
                let t_new = (s + 1) as f64 * dt;
                let (up, vp) = ops.predictors(&x);
                let (dup, dvp) = ops.predictors(&dx);
                let next = ops.advance(&fac_eff, &x, s);
                let (df_new, df_old) = (ops.df(p, t_new)?, ops.df(p, s as f64 * dt)?);
                let (cdvp, dcvp, cdv, dcv) = (ops.mulc(&dvp), ops.dc(&lin, p, &vp)?, ops.mulc(&dx.v), ops.dc(&lin, p, &x.v)?);
                let (kdup, dkup, kdu, dku) = (ops.mulk(&dup), ops.dk(&lin, p, &up)?, ops.mulk(&dx.u), ops.dk(&lin, p, &x.u)?);
                let deff_a = ops.deff(&lin, p, &next.a)?;
                let b: Vec<f64> = (0..n)
                    .map(|i| {
                        (1.0 + alpha) * df_new[i] - alpha * df_old[i] - (1.0 + alpha) * (cdvp[i] + dcvp[i]) + alpha * (cdv[i] + dcv[i]) - (1.0 + alpha) * (kdup[i] + dkup[i]) + alpha * (kdu[i] + dku[i])
                            - deff_a[i]
                    })
                    .collect();
                let da = ops.solve_eff(&fac_eff, &b);
                solves += 1;
                dx = State { u: (0..n).map(|i| dup[i] + beta * dt * dt * da[i]).collect(), v: (0..n).map(|i| dvp[i] + gamma * dt * da[i]).collect(), a: da };
                x = next;
                accumulate(&mut out, s + 1, &dx);
            }
        }
        Ok((out, TransientReport { effective_solves: solves, peak_states: 2 }))
    }

    /// Discrete-adjoint sensitivities `result[q][p]`.
    pub fn adjoint(&'m self, params: &[TransientParam], qois: &[TransientQoi], storage: Storage) -> Result<(Vec<Vec<f64>>, TransientReport), String> {
        if matches!(storage, Storage::Checkpoint(0)) {
            return Err("a checkpoint interval must be positive".into());
        }
        let ops = self.ops()?;
        let lin = LinearSensitivityEngine::new(self.model, self.loads, self.bc);
        let red = Reduced::with_ordering(&self.model.pattern, self.bc, Some(&self.model.mesh.nodes), Ordering::Auto).map_err(|e| e.to_string())?;
        let (fac_eff, fac_m) = (red.factor(&ops.eff()).map_err(|e| e.to_string())?, red.factor(&ops.mass).map_err(|e| e.to_string())?);
        let (alpha, dt, beta, gamma) = (self.opt.alpha, self.opt.dt, ops.beta, ops.gamma);
        let (n, steps) = (ops.n(), self.opt.steps);
        // forward states: all of them, or checkpoints at multiples of the interval
        let interval = match storage {
            Storage::Store => 1,
            Storage::Checkpoint(k) => k,
        };
        let mut checkpoints: Vec<State> = vec![ops.initial(&fac_m)];
        {
            let mut x = checkpoints[0].clone();
            for s in 0..steps {
                x = ops.advance(&fac_eff, &x, s);
                if (s + 1) % interval == 0 {
                    checkpoints.push(x.clone());
                }
            }
        }
        let mut peak = checkpoints.len();
        let mut out = vec![vec![0.0; params.len()]; qois.len()];
        let mut solves = 0;
        for (qi, q) in qois.iter().enumerate() {
            let mut lam = State { u: vec![0.0; n], v: vec![0.0; n], a: vec![0.0; n] };
            let seed = |lam: &mut State, step: usize| {
                for sm in q.at(step) {
                    for i in 0..n {
                        lam.u[i] += sm.gu[i];
                        lam.v[i] += sm.gv[i];
                    }
                }
            };
            seed(&mut lam, steps);
            // segments from the last checkpoint backwards
            let last_segment = (steps - 1) / interval.max(1);
            for seg in (0..=last_segment).rev() {
                if steps == 0 {
                    break;
                }
                let start = seg * interval;
                let end = ((seg + 1) * interval).min(steps);
                let mut states = vec![checkpoints[seg].clone()];
                for s in start..end {
                    let next = ops.advance(&fac_eff, &states[s - start], s);
                    states.push(next);
                }
                peak = peak.max(checkpoints.len() + states.len());
                for s in (start..end).rev() {
                    let (x, next) = (&states[s - start], &states[s + 1 - start]);
                    let (up, vp) = ops.predictors(x);
                    let mut tot = lam.a.clone();
                    for i in 0..n {
                        tot[i] += beta * dt * dt * lam.u[i] + gamma * dt * lam.v[i];
                    }
                    let w = ops.solve_eff(&fac_eff, &tot);
                    solves += 1;
                    let (cw, kw) = (ops.mulc(&w), ops.mulk(&w));
                    // parameter terms of this step
                    let t_new = (s + 1) as f64 * dt;
                    for (pi, &p) in params.iter().enumerate() {
                        let (df_new, df_old) = (ops.df(p, t_new)?, ops.df(p, s as f64 * dt)?);
                        let (dcvp, dcv, dkup, dku) = (ops.dc(&lin, p, &vp)?, ops.dc(&lin, p, &x.v)?, ops.dk(&lin, p, &up)?, ops.dk(&lin, p, &x.u)?);
                        let deff_a = ops.deff(&lin, p, &next.a)?;
                        let mut acc = 0.0;
                        for i in 0..n {
                            acc += w[i] * ((1.0 + alpha) * df_new[i] - alpha * df_old[i] - (1.0 + alpha) * dcvp[i] + alpha * dcv[i] - (1.0 + alpha) * dkup[i] + alpha * dku[i] - deff_a[i]);
                        }
                        out[qi][pi] += acc;
                    }
                    // state adjoints
                    let mut lup: Vec<f64> = (0..n).map(|i| lam.u[i] - (1.0 + alpha) * kw[i]).collect();
                    let mut lvp: Vec<f64> = (0..n).map(|i| lam.v[i] - (1.0 + alpha) * cw[i]).collect();
                    ops.restrict_free(&mut lup);
                    ops.restrict_free(&mut lvp);
                    let mut prev = State { u: vec![0.0; n], v: vec![0.0; n], a: vec![0.0; n] };
                    for i in 0..n {
                        prev.u[i] = lup[i] + alpha * kw[i];
                        prev.v[i] = lvp[i] + dt * lup[i] + alpha * cw[i];
                        prev.a[i] = dt * dt * (0.5 - beta) * lup[i] + dt * (1.0 - gamma) * lvp[i];
                    }
                    ops.restrict_free(&mut prev.u);
                    ops.restrict_free(&mut prev.v);
                    lam = prev;
                    seed(&mut lam, s);
                }
            }
            // initial acceleration: a0 = M^-1 (f0 - C v0 - K u0)
            let w0 = ops.solve_eff(&fac_m, &lam.a);
            solves += 1;
            let x0 = &checkpoints[0];
            for (pi, &p) in params.iter().enumerate() {
                let (df0, dcv, dku, dma) = (ops.df(p, 0.0)?, ops.dc(&lin, p, &x0.v)?, ops.dk(&lin, p, &x0.u)?, self.dm_x(p, &x0.a)?);
                out[qi][pi] += (0..n).map(|i| w0[i] * (df0[i] - dcv[i] - dku[i] - dma[i])).sum::<f64>();
            }
        }
        Ok((out, TransientReport { effective_solves: solves, peak_states: peak }))
    }
}
