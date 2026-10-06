//! Elastic - perfectly plastic lug (von Mises, plane stress) by the
//! initial-strain method.
//!
//! The elastic factorisation of `fe.rs` stays the iteration matrix. The
//! plastic strain `eps_p` at every Gauss point acts as an equivalent nodal
//! load `F_p = sum B^T D eps_p w`; its displacement `v = K^-1 F_p` enters the
//! condensed contact equations as an offset of the bore (`S (u_b - v_b) = f_c`),
//! so the rigid-pin contact stays the same small dense Newton solve. Each
//! iteration: solve `v`, solve the contact, recover the total field, take the
//! trial stress `D (eps - eps_p)` and return it to the yield surface; Anderson
//! mixing on `eps_p` carries the (otherwise slow) fixed point to convergence.
//!
//! Elastic-perfectly-plastic limit loads are proportional to the flow stress
//! (small-displacement limit analysis), which `solve.rs` uses to turn one run
//! at `Ftu` into the yield-level collapse as well.

use crate::contact::{ContactModel, ContactState};
use crate::fe::Condensed;
use edge_check::fem::{shape_q9, GAUSS3};

pub use mechanics_core::hardening::Hardening;

/// Equivalent plastic strain `sqrt(2/3 e:e)` of `[exx, eyy, ezz, gamma_xy]`.
pub fn equivalent_plastic_strain(ep: &[f64; 4]) -> f64 {
    (2.0 / 3.0 * (ep[0] * ep[0] + ep[1] * ep[1] + ep[2] * ep[2] + 0.5 * ep[3] * ep[3])).sqrt()
}

/// Geometry of one Gauss point: global shape-function derivatives and weight.
struct Gp {
    dn: [[f64; 2]; 9],
    w: f64,
}

/// Which 2D idealisation the plasticity uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaneMode {
    /// Thin plate, `sigma_z = 0`. Bearing crush at the bore caps the radial stress near
    /// `1.15 sigma_f`, which masks every other failure mode of a pin-loaded lug.
    Stress,
    /// `eps_z = 0`: the confinement a bearing zone really has. Used for ultimate capacity
    /// (as the validated `edge-check` contact FE does). The condensed model must then have
    /// been built with the effective constants [`PlaneMode::effective`].
    Strain,
}

impl PlaneMode {
    /// Constants to assemble the 2D stiffness with so that a plane-stress element reproduces
    /// the plane-strain response of the true `(E, nu)`.
    pub fn effective(self, e: f64, nu: f64) -> (f64, f64) {
        match self {
            PlaneMode::Stress => (e, nu),
            PlaneMode::Strain => (e / (1.0 - nu * nu), nu / (1.0 - nu)),
        }
    }
}

pub struct PlasticModel<'a> {
    cond: &'a Condensed,
    gps: Vec<Gp>,
    /// Per Gauss point: may it yield? (The bushing stays elastic.)
    yields: Vec<bool>,
    /// True material constants (not the effective ones of a plane-strain assembly).
    e: f64,
    nu: f64,
    mode: PlaneMode,
}

/// Plastic strain `[exx, eyy, ezz, gamma_xy]` at every Gauss point, element-major (9 per
/// element). `ezz` is only used in plane strain.
#[derive(Debug, Clone)]
pub struct PlasticState {
    pub eps_p: Vec<[f64; 4]>,
}

#[derive(Debug, Clone, Copy)]
pub struct PlasticOptions {
    /// Von Mises flow stress (psi).
    pub flow_stress: f64,
    /// Converged when the largest change of plastic strain is below this fraction of the yield strain.
    pub tol: f64,
    pub max_iter: usize,
    pub anderson_depth: usize,
    /// Isotropic hardening (plane strain only); `None` = perfectly plastic at `flow_stress`.
    /// When set, `flow_stress` is only the scale of the convergence tolerance.
    pub hardening: Option<Hardening>,
}

impl PlasticOptions {
    pub fn new(flow_stress: f64) -> Self {
        Self { flow_stress, tol: 2e-3, max_iter: 400, anderson_depth: 6, hardening: None }
    }
}

#[derive(Debug, Clone)]
pub struct EquilibriumResult {
    /// Total contact force on the lug (per unit thickness).
    pub force: [f64; 2],
    pub iterations: usize,
    /// Gauss points with any plastic strain / all Gauss points.
    pub plastic_fraction: f64,
    /// Largest plastic strain magnitude (|exx|, |eyy|, |gamma|/2).
    pub max_plastic_strain: f64,
    /// Largest equivalent plastic strain at any Gauss point.
    pub max_equivalent_strain: f64,
}

impl<'a> PlasticModel<'a> {
    /// `true_e` / `true_nu` are the real material constants; `cond` must have been built with
    /// `mode.effective(true_e, true_nu)`.
    pub fn new(cond: &'a Condensed, mode: PlaneMode, true_e: f64, true_nu: f64) -> Result<Self, String> {
        let (ee, en) = mode.effective(true_e, true_nu);
        if (cond.material.e_psi - ee).abs() > 1e-9 * ee || (cond.material.nu - en).abs() > 1e-9 {
            return Err("the condensed model was not built with the constants this plane mode needs".into());
        }
        let mesh = &cond.mesh;
        let mut gps = Vec::with_capacity(mesh.elems.len() * 9);
        let mut yields = Vec::with_capacity(mesh.elems.len() * 9);
        for (ei, e) in mesh.elems.iter().enumerate() {
            yields.extend(std::iter::repeat_n(mesh.elem_group[ei] == 0, 9));
            let xy: Vec<[f64; 2]> = e.iter().map(|&n| mesh.nodes[n]).collect();
            for &(gx, wx) in &GAUSS3 {
                for &(gy, wy) in &GAUSS3 {
                    let (_, dxi, deta) = shape_q9(gx, gy);
                    let (mut j11, mut j12, mut j21, mut j22) = (0.0, 0.0, 0.0, 0.0);
                    for n in 0..9 {
                        j11 += dxi[n] * xy[n][0];
                        j12 += dxi[n] * xy[n][1];
                        j21 += deta[n] * xy[n][0];
                        j22 += deta[n] * xy[n][1];
                    }
                    let det = j11 * j22 - j12 * j21;
                    if det <= 0.0 {
                        return Err("inverted element".into());
                    }
                    let inv = [[j22 / det, -j12 / det], [-j21 / det, j11 / det]];
                    let mut dn = [[0.0; 2]; 9];
                    for n in 0..9 {
                        dn[n] = [inv[0][0] * dxi[n] + inv[0][1] * deta[n], inv[1][0] * dxi[n] + inv[1][1] * deta[n]];
                    }
                    gps.push(Gp { dn, w: det * wx * wy });
                }
            }
        }
        Ok(Self { cond, gps, yields, e: true_e, nu: true_nu, mode })
    }

    pub fn gauss_points(&self) -> usize {
        self.gps.len()
    }

    pub fn fresh_state(&self) -> PlasticState {
        PlasticState { eps_p: vec![[0.0; 4]; self.gps.len()] }
    }

    /// Plane-stress stress of an in-plane strain `[exx, eyy, gamma]`.
    fn stress_from(&self, eps: [f64; 3]) -> [f64; 3] {
        let c = self.e / (1.0 - self.nu * self.nu);
        [c * (eps[0] + self.nu * eps[1]), c * (eps[1] + self.nu * eps[0]), c * (1.0 - self.nu) / 2.0 * eps[2]]
    }

    fn strain_from(&self, s: [f64; 3]) -> [f64; 3] {
        [(s[0] - self.nu * s[1]) / self.e, (s[1] - self.nu * s[0]) / self.e, s[2] * 2.0 * (1.0 + self.nu) / self.e]
    }

    /// In-plane stress carried by a plastic strain in the initial-strain load: plane stress
    /// `D eps_p`; plane strain `2 G eps_p` (plastic flow is traceless, so only the deviatoric
    /// part of the response changes).
    fn plastic_stress(&self, ep: [f64; 4]) -> [f64; 3] {
        match self.mode {
            PlaneMode::Stress => self.stress_from([ep[0], ep[1], ep[3]]),
            PlaneMode::Strain => {
                let g = self.e / (2.0 * (1.0 + self.nu));
                [2.0 * g * ep[0], 2.0 * g * ep[1], g * ep[3]]
            }
        }
    }

    /// Nodal load `sum B^T sigma_p w` of a plastic strain field (`2 * node + comp`).
    pub fn plastic_load(&self, eps_p: &[[f64; 4]]) -> Vec<f64> {
        let mesh = &self.cond.mesh;
        let mut f = vec![0.0; 2 * mesh.nodes.len()];
        for (ei, e) in mesh.elems.iter().enumerate() {
            for gi in 0..9 {
                let ep = eps_p[ei * 9 + gi];
                if ep == [0.0; 4] {
                    continue;
                }
                let s = self.plastic_stress(ep);
                let gp = &self.gps[ei * 9 + gi];
                for (n, &node) in e.iter().enumerate() {
                    f[2 * node] += gp.w * (gp.dn[n][0] * s[0] + gp.dn[n][1] * s[2]);
                    f[2 * node + 1] += gp.w * (gp.dn[n][1] * s[1] + gp.dn[n][0] * s[2]);
                }
            }
        }
        f
    }

    /// Total strain at every Gauss point of the displacement field `u`.
    pub fn strains(&self, u: &[f64]) -> Vec<[f64; 3]> {
        let mesh = &self.cond.mesh;
        let mut out = Vec::with_capacity(self.gps.len());
        for (ei, e) in mesh.elems.iter().enumerate() {
            for gi in 0..9 {
                let gp = &self.gps[ei * 9 + gi];
                let (mut exx, mut eyy, mut gxy) = (0.0, 0.0, 0.0);
                for (n, &node) in e.iter().enumerate() {
                    let (ux, uy) = (u[2 * node], u[2 * node + 1]);
                    exx += gp.dn[n][0] * ux;
                    eyy += gp.dn[n][1] * uy;
                    gxy += gp.dn[n][1] * ux + gp.dn[n][0] * uy;
                }
                out.push([exx, eyy, gxy]);
            }
        }
        out
    }

    /// Radial return of one Gauss point. `eps` is the total strain, `eps_p` the
    /// plastic strain before the step; returns the new plastic strain.
    pub fn return_map(&self, eps: [f64; 3], eps_p: [f64; 4], flow: f64) -> [f64; 4] {
        match self.mode {
            PlaneMode::Stress => self.return_map_stress(eps, eps_p, flow),
            PlaneMode::Strain => self.return_map_strain(eps, eps_p, flow, None),
        }
    }

    /// As [`return_map`](Self::return_map) with the options' hardening law (plane strain; plane
    /// stress is perfectly plastic at `flow_stress` only).
    pub fn return_map_with(&self, eps: [f64; 3], eps_p: [f64; 4], opts: &PlasticOptions) -> [f64; 4] {
        match self.mode {
            PlaneMode::Stress => self.return_map_stress(eps, eps_p, opts.flow_stress),
            PlaneMode::Strain => self.return_map_strain(eps, eps_p, opts.flow_stress, opts.hardening.as_ref()),
        }
    }

    /// Plane strain (`eps_z = 0`): 3D J2 radial return of the deviator; the mean stress is
    /// unchanged by (traceless) plastic flow.
    ///
    /// With a hardening law the yield stress follows the current equivalent plastic strain
    /// (deformation-theory form, exact for the monotonic loading of a limit-load run) and the
    /// consistency condition `vm_trial - 3 G d = sigma_y(eps_eq + d)` is solved for the increment `d`.
    fn return_map_strain(&self, eps: [f64; 3], ep: [f64; 4], flow: f64, hardening: Option<&Hardening>) -> [f64; 4] {
        let g = self.e / (2.0 * (1.0 + self.nu));
        let lam = self.e * self.nu / ((1.0 + self.nu) * (1.0 - 2.0 * self.nu));
        let (ex, ey, ez, gx) = (eps[0] - ep[0], eps[1] - ep[1], -ep[2], eps[2] - ep[3]);
        let tr = ex + ey + ez;
        let (sxx, syy, szz, txy) = (lam * tr + 2.0 * g * ex, lam * tr + 2.0 * g * ey, lam * tr + 2.0 * g * ez, g * gx);
        let p = (sxx + syy + szz) / 3.0;
        let (dx, dy, dz) = (sxx - p, syy - p, szz - p);
        let vm = (1.5 * (dx * dx + dy * dy + dz * dz + 2.0 * txy * txy)).sqrt();
        let eq0 = equivalent_plastic_strain(&ep);
        let y0 = hardening.map_or(flow, |h| h.stress(eq0));
        if vm <= y0 {
            return ep;
        }
        let sigma_new = match hardening {
            None => flow,
            Some(h) => {
                // f(d) = vm - 3 G d - sigma_y(eq0 + d) falls monotonically from f(0) > 0.
                let d = h.plastic_increment(eq0, vm, g);
                vm - 3.0 * g * d
            }
        };
        let k = sigma_new / vm;
        let r = (1.0 - k) / (2.0 * g);
        [ep[0] + dx * r, ep[1] + dy * r, ep[2] + dz * r, ep[3] + 2.0 * txy * r]
    }

    fn return_map_stress(&self, eps: [f64; 3], eps_p: [f64; 4], flow: f64) -> [f64; 4] {
        let el = [eps[0] - eps_p[0], eps[1] - eps_p[1], eps[2] - eps_p[3]];
        let tr = self.stress_from(el);
        let (a, b, c) = (tr[0] + tr[1], tr[0] - tr[1], tr[2]);
        let vm2 = |ka: f64, kb: f64| (a * ka).powi(2) / 4.0 + 3.0 * (b * kb).powi(2) / 4.0 + 3.0 * (c * kb).powi(2);
        if vm2(1.0, 1.0) <= flow * flow {
            return eps_p;
        }
        // Consistency: vm(dg) = flow, with a-mode and (b, shear)-mode softened by 1 / (1 + dg k).
        let (k_a, k_b) = (self.e / (3.0 * (1.0 - self.nu)), self.e / (1.0 + self.nu));
        let f = |dg: f64| vm2(1.0 / (1.0 + dg * k_a), 1.0 / (1.0 + dg * k_b)) - flow * flow;
        let mut hi = 1e-3 / k_b;
        while f(hi) > 0.0 {
            hi *= 2.0;
        }
        let mut lo = 0.0;
        for _ in 0..70 {
            let mid = 0.5 * (lo + hi);
            if f(mid) > 0.0 {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        let dg = 0.5 * (lo + hi);
        let (aa, bb, cc) = (a / (1.0 + dg * k_a), b / (1.0 + dg * k_b), c / (1.0 + dg * k_b));
        let s = [(aa + bb) / 2.0, (aa - bb) / 2.0, cc];
        let ee = self.strain_from(s);
        [eps[0] - ee[0], eps[1] - ee[1], 0.0, eps[2] - ee[2]]
    }

    /// Converge the elastic-plastic state at pin position `s`, starting from `pstate`
    /// and the contact `state` (both updated in place on success).
    pub fn equilibrate(&self, cm: &ContactModel, state: &mut ContactState, pstate: &mut PlasticState, s: f64, opts: PlasticOptions) -> Result<EquilibriumResult, String> {
        if opts.hardening.is_some() && self.mode == PlaneMode::Stress {
            return Err("hardening is only implemented for the plane-strain collapse model".into());
        }
        let n = self.gps.len();
        let eps_y = opts.hardening.as_ref().map_or(opts.flow_stress, |h| h.initial()) / self.e;
        let start = state.clone();
        let mut work = state.clone();
        let mut anderson = Anderson::new(opts.anderson_depth);
        let mut x: Vec<f64> = pstate.eps_p.iter().flatten().copied().collect();
        let mut best = f64::INFINITY;
        for it in 1..=opts.max_iter {
            let eps_p: Vec<[f64; 4]> = x.chunks_exact(4).map(|c| [c[0], c[1], c[2], c[3]]).collect();
            // 1. plastic displacement and its bore offset.
            let v = self.cond.solve_loads(&self.plastic_load(&eps_p));
            work.offset = self.bore_values(&v);
            // 2. contact against the offset bore (friction history restarts from the step start).
            work.t = start.t.clone();
            work.phi = start.phi.clone();
            work.ti = start.ti.clone();
            work.st_prev = start.st_prev.clone();
            work.centre = start.centre;
            let res = cm.advance(&mut work, s)?;
            // 3. total field = plastic part + the response to the contact forces.
            let w = self.cond.displacements(&cm.bore_forces(&work));
            let u: Vec<f64> = v.iter().zip(&w).map(|(a, b)| a + b).collect();
            let eps = self.strains(&u);
            // 4. return map.
            let mut gx = Vec::with_capacity(4 * n);
            for (i, e) in eps.iter().enumerate() {
                gx.extend(if self.yields[i] { self.return_map_with(*e, eps_p[i], &opts) } else { [0.0; 4] });
            }
            // A plastic strain beyond 50 % (or not finite) is a runaway, never a solution.
            if gx.iter().any(|v| !v.is_finite() || v.abs() > 0.5) {
                return Err("the plastic strain ran away (non-physical state)".into());
            }
            let change = gx.iter().zip(&x).fold(0.0f64, |m, (a, b)| m.max((a - b).abs()));
            if change <= opts.tol * eps_y {
                x = gx;
                let eps_p: Vec<[f64; 4]> = x.chunks_exact(4).map(|c| [c[0], c[1], c[2], c[3]]).collect();
                let plastic = eps_p.iter().filter(|e| e.iter().any(|v| v.abs() > 1e-12)).count();
                let max_ep = eps_p.iter().fold(0.0f64, |m, e| m.max(e[0].abs()).max(e[1].abs()).max(e[2].abs()).max(e[3].abs() / 2.0));
                let max_eq = eps_p.iter().map(equivalent_plastic_strain).fold(0.0f64, f64::max);
                pstate.eps_p = eps_p;
                *state = work;
                return Ok(EquilibriumResult { force: res.force, iterations: it, plastic_fraction: plastic as f64 / n as f64, max_plastic_strain: max_ep, max_equivalent_strain: max_eq });
            }
            // Safeguarded Anderson: if the mixing made the fixed-point residual much worse
            // than the best seen, drop its history and take the plain iterate.
            if change < best {
                best = change;
            } else if change > 4.0 * best {
                anderson.clear();
                x = gx;
                continue;
            }
            x = anderson.next(&x, &gx);
        }
        Err(format!("the plastic iteration did not converge in {} iterations", opts.max_iter))
    }

    /// Values of a full displacement field at the free bore dofs (condensed ordering).
    fn bore_values(&self, u: &[f64]) -> Vec<f64> {
        let cond = self.cond;
        let mut out = vec![0.0; cond.m];
        for &node in &cond.contact_nodes {
            for comp in 0..2 {
                if let Some(i) = cond.cidx(node, comp) {
                    out[i] = u[2 * node + comp];
                }
            }
        }
        out
    }
}

/// Anderson mixing for the fixed point `x = g(x)` (type-II, small depth).
struct Anderson {
    depth: usize,
    xs: Vec<Vec<f64>>,
    gs: Vec<Vec<f64>>,
}

impl Anderson {
    fn new(depth: usize) -> Self {
        Self { depth, xs: Vec::new(), gs: Vec::new() }
    }

    fn clear(&mut self) {
        self.xs.clear();
        self.gs.clear();
    }

    fn next(&mut self, x: &[f64], gx: &[f64]) -> Vec<f64> {
        self.xs.push(x.to_vec());
        self.gs.push(gx.to_vec());
        if self.xs.len() > self.depth + 1 {
            self.xs.remove(0);
            self.gs.remove(0);
        }
        let m = self.xs.len() - 1;
        if m == 0 {
            return gx.to_vec();
        }
        let n = x.len();
        let f: Vec<Vec<f64>> = self.xs.iter().zip(&self.gs).map(|(a, g)| g.iter().zip(a).map(|(g, a)| g - a).collect()).collect();
        // Minimise |f_k - sum gamma_i (f_{i+1} - f_i)| by the (regularised) normal equations.
        let df: Vec<Vec<f64>> = (0..m).map(|i| f[i + 1].iter().zip(&f[i]).map(|(a, b)| a - b).collect()).collect();
        let mut a = vec![0.0; m * m];
        let mut rhs = vec![0.0; m];
        for i in 0..m {
            for j in 0..=i {
                let v: f64 = df[i].iter().zip(&df[j]).map(|(p, q)| p * q).sum();
                a[i * m + j] = v;
                a[j * m + i] = v;
            }
            rhs[i] = df[i].iter().zip(&f[m]).map(|(p, q)| p * q).sum();
        }
        let trace: f64 = (0..m).map(|i| a[i * m + i]).sum::<f64>().max(1e-300);
        for i in 0..m {
            a[i * m + i] += 1e-10 * trace;
        }
        let gamma = solve_small(&mut a, &mut rhs, m);
        let mut out = gx.to_vec();
        for (i, g) in gamma.iter().enumerate() {
            for (k, o) in out.iter_mut().enumerate().take(n) {
                *o -= g * (self.gs[i + 1][k] - self.gs[i][k]);
            }
        }
        out
    }
}

/// Gaussian elimination with partial pivoting for a tiny dense system.
fn solve_small(a: &mut [f64], b: &mut [f64], n: usize) -> Vec<f64> {
    for col in 0..n {
        let piv = (col..n).max_by(|&i, &j| a[i * n + col].abs().total_cmp(&a[j * n + col].abs())).unwrap();
        if a[piv * n + col].abs() < 1e-300 {
            continue;
        }
        if piv != col {
            for k in 0..n {
                a.swap(col * n + k, piv * n + k);
            }
            b.swap(col, piv);
        }
        for r in col + 1..n {
            let f = a[r * n + col] / a[col * n + col];
            for k in col..n {
                a[r * n + k] -= f * a[col * n + k];
            }
            b[r] -= f * b[col];
        }
    }
    let mut x = vec![0.0; n];
    for r in (0..n).rev() {
        let mut v = b[r];
        for k in r + 1..n {
            v -= a[r * n + k] * x[k];
        }
        x[r] = if a[r * n + r].abs() < 1e-300 { 0.0 } else { v / a[r * n + r] };
    }
    x
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fe::{FarEnd, Material};
    use crate::geometry::LugGeometry;
    use crate::mesh::MeshSpec;

    const AL: Material = Material { e_psi: 10.0e6, nu: 0.33 };

    fn model_for_tests() -> Condensed {
        let g = LugGeometry::round_head(0.5, 1.5, 0.25, 3.75);
        Condensed::build(&g, MeshSpec { elements_around: 24, ..Default::default() }, AL, true, FarEnd::Clamped).unwrap()
    }

    fn vm(s: [f64; 3]) -> f64 {
        (s[0] * s[0] - s[0] * s[1] + s[1] * s[1] + 3.0 * s[2] * s[2]).sqrt()
    }

    #[test]
    fn a_stress_inside_the_surface_is_left_alone() {
        let c = model_for_tests();
        let pm = PlasticModel::new(&c, PlaneMode::Stress, AL.e_psi, AL.nu).unwrap();
        let eps = [0.0005, -0.0002, 0.0003];
        assert_eq!(pm.return_map(eps, [0.0; 4], 70_000.0), [0.0; 4]);
    }

    #[test]
    fn the_return_lands_exactly_on_the_yield_surface_and_keeps_the_total_strain() {
        let c = model_for_tests();
        let pm = PlasticModel::new(&c, PlaneMode::Stress, AL.e_psi, AL.nu).unwrap();
        let flow = 40_000.0;
        for eps in [[0.01, 0.0, 0.0], [0.004, -0.006, 0.002], [0.0, 0.0, 0.02], [-0.01, 0.008, -0.004]] {
            let ep = pm.return_map(eps, [0.0; 4], flow);
            let el = [eps[0] - ep[0], eps[1] - ep[1], eps[2] - ep[3]];
            let s = pm.stress_from(el);
            assert!((vm(s) - flow).abs() < 1e-6 * flow, "{eps:?}: vm {} vs {flow}", vm(s));
            assert!(ep.iter().any(|v| *v != 0.0));
        }
    }

    #[test]
    fn hardening_return_map_follows_the_yield_curve_in_simple_shear() {
        // Simple shear gamma: tau = G (gamma - gamma_p), vm = sqrt 3 tau, eq. plastic strain
        // gamma_p / sqrt 3. On the curve: sqrt 3 tau = sigma_y(gamma_p / sqrt 3).
        let (e, nu) = (10.3e6, 0.33);
        let (ee, en) = PlaneMode::Strain.effective(e, nu);
        let c = Condensed::build(&LugGeometry::round_head(0.5, 1.5, 0.25, 3.75), MeshSpec { elements_around: 24, ..Default::default() }, Material { e_psi: ee, nu: en }, true, FarEnd::Clamped).unwrap();
        let pm = PlasticModel::new(&c, PlaneMode::Strain, e, nu).unwrap();
        let law = Hardening::ramberg_osgood(40_000.0, 50_000.0, 0.08).unwrap();
        let opts = PlasticOptions { hardening: Some(law), ..PlasticOptions::new(law.initial()) };
        let g = e / (2.0 * (1.0 + nu));
        for gamma in [0.004, 0.01, 0.03, 0.08] {
            let ep = pm.return_map_with([0.0, 0.0, gamma], [0.0; 4], &opts);
            if 3f64.sqrt() * g * gamma <= law.initial() {
                assert_eq!(ep, [0.0; 4], "below the initial yield the response is elastic");
                continue;
            }
            let tau = g * (gamma - ep[3]);
            let eq = equivalent_plastic_strain(&ep);
            assert!((3f64.sqrt() * tau - law.stress(eq)).abs() < 1e-4 * law.stress(eq), "gamma {gamma}: sqrt3 tau {} vs sigma_y {}", 3f64.sqrt() * tau, law.stress(eq));
            assert!(ep[0].abs() < 1e-15 && ep[1].abs() < 1e-15 && ep[2].abs() < 1e-15, "pure shear flows in pure shear: {ep:?}");
        }
    }

    #[test]
    fn hardening_tables_are_validated_and_interpolated() {
        assert!(Hardening::table(&[(0.0, 1.0)]).is_err());
        assert!(Hardening::table(&[(0.1, 1.0), (0.2, 2.0)]).is_err());
        assert!(Hardening::table(&[(0.0, 2.0), (0.1, 1.0)]).is_err());
        let h = Hardening::linear(100.0, 1000.0, 0.1).unwrap();
        assert!((h.stress(0.05) - 150.0).abs() < 1e-9 && (h.stress(1.0) - 200.0).abs() < 1e-9 && h.stress(-1.0) == 100.0);
        let ro = Hardening::ramberg_osgood(40_000.0, 50_000.0, 0.08).unwrap();
        assert!((ro.stress(0.002) - 40_000.0).abs() < 400.0, "0.2 % offset yield: {}", ro.stress(0.002));
        assert!((ro.stress(0.08) - 50_000.0).abs() < 500.0, "ultimate: {}", ro.stress(0.08));
    }

    #[test]
    fn plastic_flow_is_volume_preserving_in_the_deviatoric_sense() {
        // Plane-stress J2 flow: the in-plane plastic strains obey d_exx + d_eyy = (sx + sy)/3 * dg,
        // i.e. no shear/deviatoric leak: a pure shear strain gives pure plastic shear.
        let c = model_for_tests();
        let pm = PlasticModel::new(&c, PlaneMode::Stress, AL.e_psi, AL.nu).unwrap();
        let ep = pm.return_map([0.0, 0.0, 0.02], [0.0; 4], 30_000.0);
        assert!(ep[0].abs() < 1e-15 && ep[1].abs() < 1e-15 && ep[3] > 0.0, "{ep:?}");
    }

    #[test]
    fn with_no_plastic_strain_the_equilibrium_is_the_elastic_contact_solution() {
        let c = model_for_tests();
        let pm = PlasticModel::new(&c, PlaneMode::Stress, AL.e_psi, AL.nu).unwrap();
        let cm = ContactModel::new(&c, 0.2495, crate::contact::ContactParams::default(), [-1.0, 0.0]);
        let mut st = cm.fresh_state();
        let elastic = cm.advance(&mut st, 0.002).unwrap();
        let mut st2 = cm.fresh_state();
        let mut ps = pm.fresh_state();
        // A flow stress far above anything reached: no plasticity.
        let r = pm.equilibrate(&cm, &mut st2, &mut ps, 0.002, PlasticOptions::new(1.0e12)).unwrap();
        assert_eq!(r.plastic_fraction, 0.0);
        assert!((r.force[0] - elastic.force[0]).abs() < 1e-6 * elastic.force[0].abs(), "{} vs {}", r.force[0], elastic.force[0]);
    }

    #[test]
    fn plasticity_caps_the_load_below_the_elastic_response() {
        let c = model_for_tests();
        let pm = PlasticModel::new(&c, PlaneMode::Stress, AL.e_psi, AL.nu).unwrap();
        let cm = ContactModel::new(&c, 0.2495, crate::contact::ContactParams::default(), [-1.0, 0.0]);
        let s = 0.02;
        let mut st = cm.fresh_state();
        let elastic = cm.advance(&mut st, s).unwrap();
        let mut st2 = cm.fresh_state();
        let mut ps = pm.fresh_state();
        let plastic = pm.equilibrate(&cm, &mut st2, &mut ps, s, PlasticOptions::new(60_000.0)).unwrap();
        assert!(plastic.plastic_fraction > 0.0);
        assert!(plastic.force[0].abs() < elastic.force[0].abs(), "plastic {} must carry less than elastic {}", plastic.force[0], elastic.force[0]);
    }

    #[test]
    fn plane_strain_return_is_elastic_below_yield_is_traceless_and_lands_on_the_surface() {
        let (e, nu) = PlaneMode::Strain.effective(AL.e_psi, AL.nu);
        let g = LugGeometry::round_head(0.5, 1.5, 0.25, 3.75);
        let c = Condensed::build(&g, MeshSpec { elements_around: 24, ..Default::default() }, Material { e_psi: e, nu }, true, FarEnd::Clamped).unwrap();
        let pm = PlasticModel::new(&c, PlaneMode::Strain, AL.e_psi, AL.nu).unwrap();
        assert_eq!(pm.return_map([1e-4, 0.0, 0.0], [0.0; 4], 60_000.0), [0.0; 4]);
        let ep = pm.return_map([0.01, -0.004, 0.003], [0.0; 4], 40_000.0);
        assert!((ep[0] + ep[1] + ep[2]).abs() < 1e-14, "plastic flow is volume preserving: {ep:?}");
        assert!(ep[2].abs() > 0.0, "plane strain develops a plastic thickness strain");
    }

    #[test]
    fn a_plane_mode_needs_a_model_assembled_with_its_constants() {
        let c = model_for_tests(); // plane-stress constants
        assert!(PlasticModel::new(&c, PlaneMode::Strain, AL.e_psi, AL.nu).is_err());
        assert!(PlasticModel::new(&c, PlaneMode::Stress, AL.e_psi, AL.nu).is_ok());
    }

    #[test]
    fn anderson_accelerates_a_slow_linear_fixed_point() {
        // x = 0.95 x + 1  -> 20; plain iteration needs hundreds of steps.
        let mut a = Anderson::new(4);
        let mut x: Vec<f64> = vec![0.0];
        let mut steps = 0;
        loop {
            let gx = vec![0.95 * x[0] + 1.0];
            if (gx[0] - x[0]).abs() < 1e-9 {
                break;
            }
            x = a.next(&x, &gx);
            steps += 1;
            assert!(steps < 20, "Anderson should converge a linear map in a handful of steps");
        }
        assert!((x[0] - 20.0).abs() < 1e-6);
    }
}
