//! Bushing-in-housing contact finite-element model, the one that needs the
//! fewest assumptions: both bodies are meshed (Q9, plane strain), the
//! interference is a real initial overlap resolved by penalty contact with
//! Coulomb friction, and the pin is a rigid cylinder pushed toward the free
//! edge under displacement control. Nothing is imposed about the pressure
//! distribution on the bore (no cosine load, no rigid bushing), whether
//! contact is retained on the back side (a result), or how the fit pressure
//! combines with the pin load (the fit is installed first, with its residual
//! stresses and any plastic strain, then the pin load is raised to collapse).
//!
//! Housing: elastic-perfectly-plastic J2. Bushing: elastic (its own hoop
//! margin is a separate solver check). The pin is conformed to the
//! post-fit bushing bore (zero initial gap, frictionless).
//!
//! Layout: one lattice row per angular station; each row holds the bushing
//! columns then the housing columns, so the interface couples nodes of the
//! same row and the banded Cholesky stays narrow. External forces are only
//! the far-face reaction of the (computed) pin load, as in the other FE
//! models. Solver: Newton-like on the penalty contact (the tangent is
//! refactored when it stalls), the plasticity handled by initial-stress
//! iteration, both under Anderson acceleration.

use crate::fem::{angular_lattice, element_stiffness, ray_to_boundary, shape1, MeshSpec, GAUSS3};
use crate::linalg::BandedSpd;
use crate::plastic::{gauss_points, return_map, Elastic, Ep, Gp};
use crate::types::{BushingSpec, Geometry};

const BIG: f64 = 1e30;

/// The collapse load is the load at a pin displacement of this fraction of
/// the bore diameter. In a perfectly plastic pin-in-hole model the load never
/// quite plateaus (the contact arc keeps growing), so a plateau rule scattered
/// the result by +-3 % from one edge distance to the next; a fixed
/// displacement is deterministic. With it the model reads 1-20 % below the
/// NACA TN 1503 bearing tests (see `tests/validation_naca_tn1503.rs`).
pub const COLLAPSE_D_OVER_D: f64 = 0.10;

/// A factorisation is reused while at most this many contact points have
/// changed state since it was built (the iteration still converges, a
/// little slower, and a factorisation costs as much as ~40 iterations).
const REUSE_DIFF: usize = 3;

fn set_diff(a: &[bool], b: &[bool]) -> usize {
    a.iter().zip(b).filter(|(x, y)| x != y).count()
}

pub static N_FACT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
pub static T_AND: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
pub static N_EQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// One contact Gauss point on an interface edge: the three nodes of the
/// edge on each side, their shape-function values, the outward normal and
/// tangent of the bore circle, and the integration weight.
struct Iface {
    nb: [usize; 3],
    nh: [usize; 3],
    n: [f64; 3],
    er: [f64; 2],
    et: [f64; 2],
    cos_phi: f64,
    w: f64,
}

pub struct ContactMesh {
    nodes: Vec<[f64; 2]>,
    elems: Vec<[usize; 9]>,
    /// 0 = bushing, 1 = housing, per element.
    body: Vec<u8>,
    gps: Vec<Gp>,
    ndof: usize,
    constrained: Vec<bool>,
    k_el: BandedSpd,
    /// Bushing-OD / housing-bore contact points.
    fit: Vec<Iface>,
    /// Bushing-ID contact points (against the rigid pin).
    pin: Vec<Iface>,
    /// Nodal forces of a unit far-face traction resultant (per unit thickness).
    v_far: Vec<f64>,
    mat: [Elastic; 2],
    kn: f64,
    mu: f64,
    inner_radius: f64,
    bore_radius: f64,
    centre_x: f64,
}

/// A factored iteration matrix and what goes with it, reused across load
/// steps while the contact set is unchanged (the plastic part never enters
/// the matrix, so most steps need no new factorisation).
struct Factor {
    sig: Vec<bool>,
    fact: BandedSpd,
    g: Vec<f64>,
    z: Vec<f64>,
    denom: f64,
}

#[derive(Default)]
pub struct Workspace {
    factor: Option<Factor>,
}

/// Mutable solution state: displacements and the committed history.
#[derive(Clone)]
pub struct State {
    u: Vec<f64>,
    ep: Vec<Ep>,
    slip: Vec<f64>,
    /// Friction slip limit `mu * p` per fit point, frozen within one
    /// equilibrium solve (see [`ContactMesh::equilibrate`]).
    cap: Vec<f64>,
    /// Pin-contact gap offset per pin point (the post-fit bore shape).
    pin_off: Vec<f64>,
    /// The pin exists only after the fit step (it conforms to the bore the
    /// fit leaves).
    pin_on: bool,
}

#[derive(Default, Clone)]
struct Eval {
    f_int: Vec<f64>,
    ep_trial: Vec<Ep>,
    slip_trial: Vec<f64>,
    fit_status: Vec<u8>,
    /// Tangential stiffness factor of the smoothed friction law per fit point.
    fit_slope: Vec<f64>,
    /// Normal contact pressure per fit point.
    fit_p: Vec<f64>,
    /// In-plane stress and `szz` (elastic estimate) per Gauss point.
    stress: Vec<[f64; 4]>,
    pin_status: Vec<u8>,
    /// Pin load per unit thickness (full model).
    p_pt: f64,
    yielded: bool,
    /// Norm of the contact nodal forces: the residual's reference scale.
    contact_norm: f64,
    fit_pressure: f64,
}

/// Result of a full fit-then-pin analysis.
#[derive(Debug, Clone, PartialEq)]
pub struct ContactResult {
    /// Collapse pin load, lbf (plateau of the load-displacement curve).
    pub collapse: f64,
    /// Pin load at which the housing first yields, lbf (0 = the fit alone yields it).
    pub first_yield: f64,
    /// Mean bushing/housing contact pressure after the fit step, psi.
    pub fit_pressure: f64,
    /// The ramp reached the displacement limit. If not (it stopped
    /// converging first), `collapse` is the highest converged load: a lower bound.
    pub plateau: bool,
}

struct Anderson {
    dx: Vec<Vec<f64>>,
    dg: Vec<Vec<f64>>,
    prev: Option<(Vec<f64>, Vec<f64>)>,
}

impl Anderson {
    const MEM: usize = 8;
    fn new() -> Self {
        Self { dx: Vec::new(), dg: Vec::new(), prev: None }
    }
    fn reset(&mut self) {
        *self = Self::new();
    }
    /// Next iterate for the fixed-point map `x -> x + g` with Anderson mixing.
    fn next(&mut self, x: &[f64], g: &[f64]) -> Vec<f64> {
        if let Some((px, pg)) = &self.prev {
            self.dx.push(x.iter().zip(px).map(|(a, b)| a - b).collect());
            self.dg.push(g.iter().zip(pg).map(|(a, b)| a - b).collect());
            if self.dx.len() > Self::MEM {
                self.dx.remove(0);
                self.dg.remove(0);
            }
        }
        self.prev = Some((x.to_vec(), g.to_vec()));
        let mut out: Vec<f64> = x.iter().zip(g).map(|(a, b)| a + b).collect();
        let (n, m) = (x.len(), self.dg.len());
        if m > 0 {
            let mut a = vec![0.0; n * m];
            for (j, dgj) in self.dg.iter().enumerate() {
                for i in 0..n {
                    a[i * m + j] = dgj[i];
                }
            }
            let gamma = crate::linalg::least_squares(&a, n, m, g, 1e-10);
            for (j, gj) in gamma.iter().enumerate() {
                for i in 0..n {
                    out[i] -= gj * (self.dx[j][i] + self.dg[j][i]);
                }
            }
        }
        out
    }
}

fn d_matrix(c: &Elastic) -> [[f64; 3]; 3] {
    let (e, nu) = if c.plane_strain { (c.e / (1.0 - c.nu * c.nu), c.nu / (1.0 - c.nu)) } else { (c.e, c.nu) };
    let f = e / (1.0 - nu * nu);
    [[f, f * nu, 0.0], [f * nu, f, 0.0], [0.0, 0.0, f * (1.0 - nu) / 2.0]]
}

impl ContactMesh {
    /// `geom.bore_radius` is the housing bore, `geom.edge` the distance to
    /// the free edge; the plate extent is `geom.plate_far`/`plate_half_height`.
    pub fn build(geom: &Geometry, spec: &BushingSpec, e_housing: f64, nu_housing: f64, mesh: MeshSpec, n_bushing_radial: usize, plane_strain: bool) -> Result<Self, String> {
        let a = geom.bore_radius;
        let e = geom.edge;
        let ri = spec.inner_radius;
        if !(a > 0.0 && e > a && ri > 0.0 && ri < a && geom.plate_far > a && geom.plate_half_height > a && spec.interference >= 0.0) {
            return Err("degenerate contact geometry (bushing bore must be inside the housing bore)".to_string());
        }
        let (far, h) = (geom.plate_far, geom.plate_half_height);
        let phis = angular_lattice(e, far, h, mesh.n_arc);
        let nb_rows = phis.len();
        let n_rb = n_bushing_radial.max(1);
        let na_b = 2 * n_rb + 1;
        let na_h = 2 * mesh.n_radial + 1;
        let na = na_b + na_h;
        let n_theta_el = (nb_rows - 1) / 2;

        let mut nodes = vec![[0.0; 2]; na * nb_rows];
        for (ib, &phi) in phis.iter().enumerate() {
            let t_out = ray_to_boundary(e, far, h, phi);
            let grade_at = |i: usize| (i as f64 / mesh.n_radial as f64).powf(mesh.grade);
            for ia in 0..na_b {
                let r = ri + (a - ri) * ia as f64 / (na_b - 1) as f64;
                nodes[ib * na + ia] = [e + r * phi.cos(), r * phi.sin()];
            }
            for k in 0..na_h {
                let s = if k % 2 == 0 { grade_at(k / 2) } else { 0.5 * (grade_at(k / 2) + grade_at(k / 2 + 1)) };
                let r = a + (t_out - a) * s;
                nodes[ib * na + na_b + k] = [e + r * phi.cos(), r * phi.sin()];
            }
        }
        for node in nodes.iter_mut() {
            if node[1].abs() < 1e-12 * e {
                node[1] = 0.0;
            }
        }

        let mut elems = Vec::new();
        let mut body = Vec::new();
        for j in 0..n_theta_el {
            for (nr, off, b) in [(n_rb, 0usize, 0u8), (mesh.n_radial, na_b, 1u8)] {
                for i in 0..nr {
                    let mut conn = [0usize; 9];
                    for bb in 0..3 {
                        for aa in 0..3 {
                            conn[3 * bb + aa] = (2 * j + bb) * na + off + 2 * i + aa;
                        }
                    }
                    elems.push(conn);
                    body.push(b);
                }
            }
        }

        let mat = [Elastic { e: spec.e, nu: spec.nu, plane_strain }, Elastic { e: e_housing, nu: nu_housing, plane_strain }];
        let d_mats = [d_matrix(&mat[0]), d_matrix(&mat[1])];
        let ndof = 2 * nodes.len();
        let bw = 2 * (2 * na + 3) + 1;
        let mut k = BandedSpd::zeros(ndof, bw.min(ndof - 1));
        for (conn, &b) in elems.iter().zip(&body) {
            let xy: Vec<[f64; 2]> = conn.iter().map(|&n| nodes[n]).collect();
            let ke = element_stiffness(&xy, &d_mats[b as usize])?;
            for i in 0..18 {
                let gi = 2 * conn[i / 2] + i % 2;
                for jj in 0..=i {
                    k.add(gi, 2 * conn[jj / 2] + jj % 2, ke[i][jj]);
                }
            }
        }

        // Interface Gauss points (bushing OD / housing bore) and the pin
        // contact points (bushing ID). The load line is a symmetry axis.
        let cx = e;
        let mut fit = Vec::new();
        let mut pin = Vec::new();
        for j in 0..n_theta_el {
            let rows = [2 * j, 2 * j + 1, 2 * j + 2];
            let b_od = rows.map(|r| r * na + na_b - 1);
            let h_bore = rows.map(|r| r * na + na_b);
            let b_id = rows.map(|r| r * na);
            for &(g, w) in &GAUSS3 {
                let (n1, d1) = shape1(g);
                for (is_fit, ids, radius) in [(true, b_od, a), (false, b_id, ri)] {
                    let (mut x, mut y, mut dx, mut dy) = (0.0, 0.0, 0.0, 0.0);
                    for q in 0..3 {
                        x += n1[q] * nodes[ids[q]][0];
                        y += n1[q] * nodes[ids[q]][1];
                        dx += d1[q] * nodes[ids[q]][0];
                        dy += d1[q] * nodes[ids[q]][1];
                    }
                    let _ = radius;
                    let phi = y.atan2(x - cx);
                    let (c, s) = (phi.cos(), phi.sin());
                    let iface = Iface { nb: ids, nh: h_bore, n: n1, er: [c, s], et: [-s, c], cos_phi: c, w: (dx * dx + dy * dy).sqrt() * w };
                    if is_fit {
                        fit.push(iface);
                    } else {
                        pin.push(iface);
                    }
                }
            }
        }

        // Far-face reaction of a unit pin load (per unit thickness): +x
        // traction 1/(2h) on the half model's far face.
        let mut v_far = vec![0.0; ndof];
        let sigma_far = 1.0 / (2.0 * h);
        let far_x = e + far;
        for j in 0..n_theta_el {
            let ids = [0, 1, 2].map(|q| (2 * j + q) * na + na - 1);
            if ids.iter().all(|&n| (nodes[n][0] - far_x).abs() < 1e-9 * far_x.abs().max(1.0)) {
                for &(g, w) in &GAUSS3 {
                    let (n1, d1) = shape1(g);
                    let (mut dx, mut dy) = (0.0, 0.0);
                    for q in 0..3 {
                        dx += d1[q] * nodes[ids[q]][0];
                        dy += d1[q] * nodes[ids[q]][1];
                    }
                    let jac = (dx * dx + dy * dy).sqrt();
                    for q in 0..3 {
                        v_far[2 * ids[q]] += n1[q] * sigma_far * jac * w;
                    }
                }
            }
        }

        // Constraints (penalty keeps the band): symmetry u_y = 0 on y = 0,
        // one u_x anchor on the far face; a feather-weight spring on the
        // bushing keeps the matrix definite while its contacts are open.
        let max_diag = (0..ndof).map(|i| k.diag(i)).fold(0.0, f64::max);
        let mean_diag = (0..ndof).map(|i| k.diag(i)).sum::<f64>() / ndof as f64;
        let mut constrained = vec![false; ndof];
        for (ie, conn) in elems.iter().enumerate() {
            if body[ie] == 0 {
                for &n in conn {
                    for d in 0..2 {
                        k.add(2 * n + d, 2 * n + d, 1e-9 * mean_diag / 9.0);
                    }
                }
            }
        }
        // Constraint penalty: well above every stiffness it must dominate
        // (elements and the contact penalty) but no more, to keep the
        // factorisation well conditioned.
        let w_max = fit.iter().chain(&pin).map(|f| f.w).fold(0.0, f64::max);
        let penalty = 1e4 * max_diag.max(2.0 * e_housing.max(spec.e) / a * 1e2 * w_max);
        for (n, p) in nodes.iter().enumerate() {
            if p[1] == 0.0 {
                k.add(2 * n + 1, 2 * n + 1, penalty);
                constrained[2 * n + 1] = true;
            }
        }
        let anchor = na - 1;
        k.add(2 * anchor, 2 * anchor, penalty);
        constrained[2 * anchor] = true;

        let kn = 1e2 * e_housing.max(spec.e) / a;
        let gps = gauss_points(&nodes, &elems);
        Ok(Self { nodes, elems, body, gps, ndof, constrained, k_el: k, fit, pin, v_far, mat, kn, mu: spec.friction.max(0.0), inner_radius: ri, bore_radius: a, centre_x: cx })
    }

    pub fn dof_count(&self) -> usize {
        self.ndof
    }

    fn new_state(&self) -> State {
        State {
            u: vec![0.0; self.ndof],
            ep: vec![[0.0; 4]; self.gps.len()],
            slip: vec![0.0; self.fit.len()],
            cap: vec![BIG; self.fit.len()],
            pin_off: vec![0.0; self.pin.len()],
            pin_on: false,
        }
    }

    /// Residual pieces at displacement `u` for overlap `delta` and pin
    /// displacement `d`, with committed history `st`.
    fn eval(&self, u: &[f64], d: f64, delta: f64, st: &State, sigma0: f64, out: &mut Eval) {
        let n = self.ndof;
        out.f_int.clear();
        out.f_int.resize(n, 0.0);
        out.ep_trial.clone_from(&st.ep);
        out.slip_trial.clone_from(&st.slip);
        out.fit_status.clear();
        out.fit_status.resize(self.fit.len(), 0);
        out.stress.clear();
        out.stress.resize(self.gps.len(), [0.0; 4]);
        out.fit_p.clear();
        out.fit_p.resize(self.fit.len(), 0.0);
        out.fit_slope.clear();
        out.fit_slope.resize(self.fit.len(), 0.0);
        out.pin_status.clear();
        out.pin_status.resize(self.pin.len(), 0);
        out.yielded = false;

        for (ie, conn) in self.elems.iter().enumerate() {
            let b = self.body[ie] as usize;
            let s0 = if b == 1 { sigma0 } else { BIG };
            for g in 0..9 {
                let gp = &self.gps[ie * 9 + g];
                let (mut exx, mut eyy, mut gxy) = (0.0, 0.0, 0.0);
                for k in 0..9 {
                    let (ux, uy) = (u[2 * conn[k]], u[2 * conn[k] + 1]);
                    exx += gp.dnx[k] * ux;
                    eyy += gp.dny[k] * uy;
                    gxy += gp.dny[k] * ux + gp.dnx[k] * uy;
                }
                let (s, epn) = return_map([exx, eyy, gxy], st.ep[ie * 9 + g], self.mat[b], s0);
                if b == 1 && epn != st.ep[ie * 9 + g] {
                    out.yielded = true;
                }
                out.ep_trial[ie * 9 + g] = epn;
                if b == 1 {
                    let szz = if self.mat[1].plane_strain { self.mat[1].nu * (s[0] + s[1]) } else { 0.0 };
                    out.stress[ie * 9 + g] = [s[0], s[1], s[2], szz];
                }
                for k in 0..9 {
                    out.f_int[2 * conn[k]] += (gp.dnx[k] * s[0] + gp.dny[k] * s[2]) * gp.w;
                    out.f_int[2 * conn[k] + 1] += (gp.dny[k] * s[1] + gp.dnx[k] * s[2]) * gp.w;
                }
            }
        }

        let mut contact_sq = 0.0;
        let (mut p_sum, mut w_sum) = (0.0, 0.0);
        for (i, f) in self.fit.iter().enumerate() {
            let (mut wn, mut wt) = (0.0, 0.0);
            for q in 0..3 {
                let rel = [u[2 * f.nb[q]] - u[2 * f.nh[q]], u[2 * f.nb[q] + 1] - u[2 * f.nh[q] + 1]];
                wn += f.n[q] * (rel[0] * f.er[0] + rel[1] * f.er[1]);
                wt += f.n[q] * (rel[0] * f.et[0] + rel[1] * f.et[1]);
            }
            let pen = delta + wn;
            if pen <= 0.0 {
                out.slip_trial[i] = wt;
                continue;
            }
            let p = self.kn * pen;
            let tau = self.kn * (wt - st.slip[i]);
            // Coulomb friction: exact return map for the committed history,
            // C1-smoothed traction (power-8 saturation) inside the iteration
            // so the stick/slip switch cannot make the Newton iteration chatter.
            let cap = st.cap[i];
            let (t, status) = if cap <= 0.0 {
                (0.0, 2)
            } else {
                let u = tau.abs() / cap;
                if tau.abs() > cap {
                    out.slip_trial[i] = wt - cap * tau.signum() / self.kn;
                }
                (tau / (1.0 + u.powi(8)).powf(0.125), if u >= 1.0 { 2 } else { 1 })
            };
            out.fit_slope[i] = if cap <= 0.0 { 0.0 } else { (1.0 + (tau.abs() / cap).powi(8)).powf(-1.125) };
            out.fit_status[i] = status;
            out.fit_p[i] = p;
            p_sum += p * f.w;
            w_sum += f.w;
            for q in 0..3 {
                let fx = (p * f.er[0] + t * f.et[0]) * f.n[q] * f.w;
                let fy = (p * f.er[1] + t * f.et[1]) * f.n[q] * f.w;
                out.f_int[2 * f.nb[q]] += fx;
                out.f_int[2 * f.nb[q] + 1] += fy;
                out.f_int[2 * f.nh[q]] -= fx;
                out.f_int[2 * f.nh[q] + 1] -= fy;
                contact_sq += 2.0 * (fx * fx + fy * fy);
            }
        }
        out.fit_pressure = if w_sum > 0.0 { p_sum / w_sum } else { 0.0 };

        let mut p_pt = 0.0;
        for (i, f) in self.pin.iter().enumerate().filter(|_| st.pin_on) {
            let mut un = 0.0;
            for q in 0..3 {
                un += f.n[q] * (u[2 * f.nb[q]] * f.er[0] + u[2 * f.nb[q] + 1] * f.er[1]);
            }
            let pen = -d * f.cos_phi - (un - st.pin_off[i]);
            if pen <= 0.0 {
                continue;
            }
            out.pin_status[i] = 1;
            let p = self.kn * pen;
            p_pt -= 2.0 * p * f.cos_phi * f.w;
            for q in 0..3 {
                let fx = -p * f.er[0] * f.n[q] * f.w;
                let fy = -p * f.er[1] * f.n[q] * f.w;
                out.f_int[2 * f.nb[q]] += fx;
                out.f_int[2 * f.nb[q] + 1] += fy;
                contact_sq += fx * fx + fy * fy;
            }
        }
        out.p_pt = p_pt;
        out.contact_norm = contact_sq.sqrt();
    }

    /// Residual `F_ext - F_int` (zero on constrained dofs) into `r`.
    fn residual(&self, ev: &Eval, r: &mut [f64]) {
        for i in 0..self.ndof {
            r[i] = if self.constrained[i] { 0.0 } else { ev.p_pt * self.v_far[i] - ev.f_int[i] };
        }
    }

    /// Iteration matrix: the elastic stiffness plus the contact stiffness of
    /// the points that are closed in `ev` (the tangential stiffness follows
    /// the smoothed friction law).
    fn assemble(&self, ev: &Eval) -> BandedSpd {
        let mut m = self.k_el.clone();
        let mut add_pt = |ids_b: &[usize; 3], ids_h: Option<&[usize; 3]>, n: &[f64; 3], dir: [f64; 2], k: f64, w: f64| {
            // d(w_dir)/du over 12 dofs: bushing +n e, housing -n e.
            let mut dofs = [0usize; 12];
            let mut val = [0.0f64; 12];
            let mut c = 0;
            for q in 0..3 {
                dofs[c] = 2 * ids_b[q];
                val[c] = n[q] * dir[0];
                dofs[c + 1] = 2 * ids_b[q] + 1;
                val[c + 1] = n[q] * dir[1];
                c += 2;
            }
            if let Some(h) = ids_h {
                for q in 0..3 {
                    dofs[c] = 2 * h[q];
                    val[c] = -n[q] * dir[0];
                    dofs[c + 1] = 2 * h[q] + 1;
                    val[c + 1] = -n[q] * dir[1];
                    c += 2;
                }
            }
            for i in 0..c {
                for j in 0..=i {
                    let v = k * w * val[i] * val[j];
                    if dofs[i] >= dofs[j] {
                        m.add(dofs[i], dofs[j], v);
                    } else {
                        m.add(dofs[j], dofs[i], v);
                    }
                }
            }
        };
        for ((f, &s), &slope) in self.fit.iter().zip(&ev.fit_status).zip(&ev.fit_slope) {
            if s >= 1 {
                add_pt(&f.nb, Some(&f.nh), &f.n, f.er, self.kn, f.w);
                // Tangential stiffness of the smoothed law (a floor keeps the
                // matrix definite when a point is fully slipping).
                add_pt(&f.nb, Some(&f.nh), &f.n, f.et, self.kn * slope.max(1e-3), f.w);
            }
        }
        for (f, &s) in self.pin.iter().zip(&ev.pin_status) {
            if s == 1 {
                add_pt(&f.nb, None, &f.n, f.er, self.kn, f.w);
            }
        }
        m
    }

    /// `d(pin load)/du` at the closed pin points: the far-face reaction is a
    /// follower load of the (penalty-stiff) pin contact, so its derivative
    /// must be in the iteration matrix (rank one, applied by
    /// Sherman-Morrison) or the iteration cannot converge.
    fn pin_load_gradient(&self, ev: &Eval) -> Vec<f64> {
        let mut g = vec![0.0; self.ndof];
        for (f, &s) in self.pin.iter().zip(&ev.pin_status) {
            if s == 1 {
                for q in 0..3 {
                    g[2 * f.nb[q]] += 2.0 * self.kn * f.cos_phi * f.w * f.n[q] * f.er[0];
                    g[2 * f.nb[q] + 1] += 2.0 * self.kn * f.cos_phi * f.w * f.n[q] * f.er[1];
                }
            }
        }
        g
    }

    /// Equilibrium at `(d, delta)` including Coulomb friction. The slip
    /// limits `mu p` are frozen inside the solve (so the tangential law is
    /// plain elastic-perfectly-plastic and the iteration matrix exact) and
    /// refreshed from the converged pressures for the next load step: a
    /// one-step lag in the pressure that limits friction. The steps are
    /// small compared with the pressure they change, and friction only
    /// redistributes shear on the interface, so the lag is a second-order
    /// effect (checked against `mu = 0` and against an iterated pass in the
    /// tests).
    fn equilibrate(&self, ws: &mut Workspace, st: &mut State, d: f64, delta: f64, sigma0: f64, tol: f64) -> Result<Eval, String> {
        let ev = self.equilibrate_fixed(ws, st, d, delta, sigma0, tol)?;
        for (c, p) in st.cap.iter_mut().zip(&ev.fit_p) {
            *c = self.mu * p;
        }
        Ok(ev)
    }

    /// Equilibrium at `(d, delta)` with frozen friction limits from the
    /// displacement in `st`; on success commits the history into `st` and
    /// returns the converged evaluation.
    fn equilibrate_fixed(&self, ws: &mut Workspace, st: &mut State, d: f64, delta: f64, sigma0: f64, tol: f64) -> Result<Eval, String> {
        let closed = |e: &Eval| -> Vec<bool> { e.fit_status.iter().map(|&x| x >= 1).chain(e.pin_status.iter().map(|&x| x >= 1)).collect() };
        let mut ev = Eval::default();
        let mut u = st.u.clone();
        self.eval(&u, d, delta, st, sigma0, &mut ev);
        let mut r = vec![0.0; self.ndof];
        let mut anderson = Anderson::new();
        // Best iterate seen (relative residual, displacement, evaluation).
        let mut best: Option<(f64, Vec<f64>, Eval)> = None;
        let mut force_refactor = false;
        for _outer in 0..14 {
            if force_refactor || ws.factor.as_ref().is_none_or(|f| set_diff(&f.sig, &closed(&ev)) > REUSE_DIFF) {
                let mut fact = self.assemble(&ev);
                let okf = fact.factor();
                if !okf {
                    return Err("contact stiffness matrix is not positive definite".to_string());
                }
                let g = self.pin_load_gradient(&ev);
                let mut z: Vec<f64> = (0..self.ndof).map(|i| if self.constrained[i] { 0.0 } else { self.v_far[i] }).collect();
                fact.solve_in_place(&mut z);
                let denom = 1.0 - g.iter().zip(&z).map(|(a, b)| a * b).sum::<f64>();
                ws.factor = Some(Factor { sig: closed(&ev), fact, g, z, denom });
                force_refactor = false;
            }
            let f = ws.factor.as_ref().expect("factor built above");
            anderson.reset();
            let (mut stall, mut stall_ref) = (0, f64::INFINITY);
            for it in 0..40 {
                self.eval(&u, d, delta, st, sigma0, &mut ev);
                self.residual(&ev, &mut r);
                let rn = r.iter().map(|v| v * v).sum::<f64>().sqrt();
                if !rn.is_finite() {
                    return Err("equilibrium iteration diverged".to_string());
                }
                // Reference scale: contact forces, floored by the far-face reaction.
                let rel = rn / ev.contact_norm.max(ev.p_pt.abs()).max(1e-30);
                if rel <= tol {
                    st.u = u;
                    st.ep.clone_from(&ev.ep_trial);
                    st.slip.clone_from(&ev.slip_trial);
                    return Ok(ev);
                }
                if best.as_ref().is_none_or(|b| rel < b.0) {
                    best = Some((rel, u.clone(), ev.clone()));
                }
                if it >= 1 && set_diff(&f.sig, &closed(&ev)) > REUSE_DIFF {
                    break; // the contact set changed: re-linearise
                }
                if rn < 0.7 * stall_ref {
                    stall_ref = rn;
                    stall = 0;
                } else {
                    stall += 1;
                    if stall >= 12 {
                        force_refactor = true;
                        break;
                    }
                }
                f.fact.solve_in_place(&mut r);
                if f.denom.abs() > 1e-12 {
                    // Sherman-Morrison for the follower far-face load.
                    let k = f.g.iter().zip(&r).map(|(a, b)| a * b).sum::<f64>() / f.denom;
                    for (ri, zi) in r.iter_mut().zip(&f.z) {
                        *ri += k * zi;
                    }
                }
                if it < 2 {
                    // Damped Newton while the contact set is still settling:
                    // an undamped step can swap two contact sets forever.
                    let mut alpha = 1.0;
                    let mut trial_ev = Eval::default();
                    let mut rr = vec![0.0; self.ndof];
                    for _ in 0..5 {
                        let cand: Vec<f64> = u.iter().zip(&r).map(|(a, b)| a + alpha * b).collect();
                        self.eval(&cand, d, delta, st, sigma0, &mut trial_ev);
                        self.residual(&trial_ev, &mut rr);
                        let rt = rr.iter().map(|v| v * v).sum::<f64>().sqrt();
                        if rt < (1.0 - 1e-4 * alpha) * rn {
                            break;
                        }
                        alpha *= 0.5;
                    }
                    for (a, b) in u.iter_mut().zip(&r) {
                        *a += alpha * b;
                    }
                } else {
                    u = anderson.next(&u, &r);
                }
            }
            // Re-linearise on the contact state reached so far.
            self.eval(&u, d, delta, st, sigma0, &mut ev);
        }
        match best {
            // Contact points at the edge of the contact zone can chatter
            // between open and closed; an out-of-balance of at most 0.5 % of
            // the contact force is below the discretisation error.
            Some((rel, bu, bev)) if rel <= 5e-3 => {
                st.u = bu;
                st.ep.clone_from(&bev.ep_trial);
                st.slip.clone_from(&bev.slip_trial);
                Ok(bev)
            }
            _ => Err("equilibrium iteration did not converge".to_string()),
        }
    }

    /// Scale `lambda` such that `sigma_fit + lambda (sigma_a - sigma_fit)`
    /// first reaches the flow stress at some housing Gauss point: the
    /// elastic first-yield multiple of the load step that produced `ev_a`
    /// on top of the fit state `ev_f` (stress is linear in the load while
    /// the contact set does not change). `0` if the fit alone yields.
    fn first_yield_scale(&self, ev_f: &Eval, ev_a: &Eval, sigma0: f64) -> f64 {
        let vmf = |a: [f64; 4], b: [f64; 4]| 0.5 * ((a[0] - a[1]) * (b[0] - b[1]) + (a[1] - a[3]) * (b[1] - b[3]) + (a[3] - a[0]) * (b[3] - b[0])) + 3.0 * a[2] * b[2];
        let mut lambda_y = f64::INFINITY;
        for (ie, &b) in self.body.iter().enumerate() {
            if b != 1 {
                continue;
            }
            for g in 0..9 {
                let (sf, sa) = (ev_f.stress[ie * 9 + g], ev_a.stress[ie * 9 + g]);
                let ds = [sa[0] - sf[0], sa[1] - sf[1], sa[2] - sf[2], sa[3] - sf[3]];
                let (qa, qb, qc) = (vmf(ds, ds), 2.0 * vmf(sf, ds), vmf(sf, sf) - sigma0 * sigma0);
                if qc >= 0.0 {
                    return 0.0;
                }
                let disc = qb * qb - 4.0 * qa * qc;
                if qa > 0.0 && disc >= 0.0 {
                    let root = (-qb + disc.sqrt()) / (2.0 * qa);
                    if root > 0.0 {
                        lambda_y = lambda_y.min(root);
                    }
                }
            }
        }
        lambda_y
    }

    /// Fit-then-pin analysis: install the interference (with residual
    /// stress and any plastic flow), conform the pin to the resulting
    /// bushing bore, then push the pin toward the free edge under
    /// displacement control until the load plateaus. The plateau is the
    /// collapse load: the first pin displacement at which a further
    /// geometric step of displacement adds less than 2 % of load, twice
    /// running. The first-yield load is extrapolated from one elastic pin
    /// step (exact while the contact set is unchanged).
    pub fn collapse(&self, delta: f64, sigma0: f64, thickness: f64) -> Result<ContactResult, String> {
        // A bushing with no interference still has to touch the bore.
        let delta = delta.max(1e-9 * self.bore_radius);
        let tol = 1e-4;
        let mut ws = Workspace::default();
        let mut st = self.new_state();
        // The fit: in one step if it stays elastic, else in four.
        let mut ev_f = match self.equilibrate(&mut ws, &mut st, 0.0, delta, sigma0, tol) {
            Ok(ev) if !ev.yielded => ev,
            _ => {
                st = self.new_state();
                let mut ev = Eval::default();
                for k in 1..=4 {
                    ev = self.equilibrate(&mut ws, &mut st, 0.0, delta * k as f64 / 4.0, sigma0, tol)?;
                }
                ev
            }
        };
        let fit_yields = st.ep.iter().any(|e| e.iter().any(|&x| x != 0.0)) || self.first_yield_scale(&ev_f, &ev_f, sigma0) == 0.0;
        let fit_pressure = ev_f.fit_pressure;
        self.conform_pin(&mut st);
        ev_f.p_pt = 0.0;

        // One small elastic pin step gives the first-yield load.
        let d_ref = sigma0 / self.mat[1].e * self.bore_radius;
        let mut d_a = 0.3 * d_ref;
        let st_fit = st.clone();
        let (mut ev_a, mut st_a) = (None, st_fit.clone());
        let mut fallback = None;
        for _ in 0..6 {
            let mut trial = st_fit.clone();
            match self.equilibrate(&mut ws, &mut trial, d_a, delta, sigma0, tol) {
                Ok(ev) if fit_yields || !ev.yielded => {
                    ev_a = Some(ev);
                    st_a = trial;
                    break;
                }
                Ok(ev) => {
                    fallback = Some((ev, trial));
                    d_a *= 0.5;
                }
                Err(_) => d_a *= 0.5,
            }
        }
        // The fit can leave the housing a hair under yield, so that any pin
        // load yields it: then there is no elastic step and first yield is
        // (to within the step) zero.
        let mut fit_yields = fit_yields;
        let ev_a = match (ev_a, fallback) {
            (Some(ev), _) => ev,
            (None, Some((ev, trial))) => {
                fit_yields = true;
                st_a = trial;
                ev
            }
            (None, None) => return Err("no pin step converged (the contact is unstable)".to_string()),
        };
        let f_a = ev_a.p_pt * thickness;
        let lambda_y = self.first_yield_scale(&ev_f, &ev_a, sigma0);
        let first_yield = if fit_yields { 0.0 } else { lambda_y * f_a };

        // Plastic ramp: geometric growth of the pin displacement from just
        // past first yield up to the displacement limit `COLLAPSE_D_OVER_D * D`,
        // where the collapse load is read (the last step lands on it exactly).
        let d_limit = COLLAPSE_D_OVER_D * 2.0 * self.bore_radius;
        let d_y = if lambda_y.is_finite() { lambda_y * d_a } else { 10.0 * d_a };
        let mut st = st_a;
        let mut d = (d_y.max(d_a) * 1.1).min(d_limit);
        let mut ratio = 1.6f64;
        let (mut f_best, mut steps_ok) = (f_a, 0);
        let mut f_at_limit: Option<f64> = None;
        let mut last_err = String::new();
        for _ in 0..40 {
            let saved = st.clone();
            match self.equilibrate(&mut ws, &mut st, d, delta, sigma0, tol) {
                Ok(ev) => {
                    let f = ev.p_pt * thickness;
                    steps_ok += 1;
                    f_best = f_best.max(f);
                    if d >= d_limit * (1.0 - 1e-9) {
                        f_at_limit = Some(f);
                        break;
                    }
                    d = (d * ratio).min(d_limit);
                }
                Err(err) => {
                    last_err = err;
                    st = saved;
                    d /= ratio;
                    ratio = 1.0 + (ratio - 1.0) * 0.5;
                    if ratio < 1.05 {
                        break;
                    }
                    d = (d * ratio).min(d_limit);
                }
            }
        }
        if steps_ok == 0 {
            return Err(format!("the plastic ramp did not converge ({last_err})"));
        }
        // Not reaching the limit leaves the highest converged load: a lower bound.
        let collapse = f_at_limit.unwrap_or(f_best);
        Ok(ContactResult { collapse, first_yield: first_yield.min(collapse), fit_pressure, plateau: f_at_limit.is_some() })
    }

    /// Conform the rigid pin to the bushing bore the fit left (zero initial
    /// gap everywhere) and switch the pin contact on.
    fn conform_pin(&self, st: &mut State) {
        for (i, f) in self.pin.iter().enumerate() {
            let mut un = 0.0;
            for q in 0..3 {
                un += f.n[q] * (st.u[2 * f.nb[q]] * f.er[0] + st.u[2 * f.nb[q] + 1] * f.er[1]);
            }
            st.pin_off[i] = un;
        }
        st.pin_on = true;
    }

    /// Elastic fit step only: mean contact pressure and the peak housing
    /// hoop-direction displacement are enough for the validation tests.
    pub fn fit_only(&self, delta: f64) -> Result<(f64, State), String> {
        let mut ws = Workspace::default();
        let mut st = self.new_state();
        let mut p = 0.0;
        for k in 1..=4 {
            p = self.equilibrate(&mut ws, &mut st, 0.0, delta * k as f64 / 4.0, BIG, 1e-6)?.fit_pressure;
        }
        Ok((p, st))
    }

    /// Elastic pin push at displacement `d` after an elastic fit of `delta`.
    /// Returns `(pin load per unit thickness, contact pressure at each
    /// fit point as (cos phi, p, weight))`.
    pub fn elastic_push(&self, delta: f64, d: f64) -> Result<(f64, Vec<(f64, f64, f64)>), String> {
        let mut ws = Workspace::default();
        let mut st = self.new_state();
        for k in 1..=4 {
            self.equilibrate(&mut ws, &mut st, 0.0, delta * k as f64 / 4.0, BIG, 1e-6)?;
        }
        self.conform_pin(&mut st);
        let ev = self.equilibrate(&mut ws, &mut st, d, delta, BIG, 1e-6)?;
        let mut dist = Vec::new();
        for f in self.fit.iter() {
            let (mut wn, mut _wt) = (0.0, 0.0);
            for q in 0..3 {
                let rel = [st.u[2 * f.nb[q]] - st.u[2 * f.nh[q]], st.u[2 * f.nb[q] + 1] - st.u[2 * f.nh[q] + 1]];
                wn += f.n[q] * (rel[0] * f.er[0] + rel[1] * f.er[1]);
                _wt += 0.0;
            }
            dist.push((f.cos_phi, self.kn * (delta + wn).max(0.0), f.w));
        }
        Ok((ev.p_pt, dist))
    }

    pub fn centre_x(&self) -> f64 {
        self.centre_x
    }

    pub fn inner_radius(&self) -> f64 {
        self.inner_radius
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: f64 = 0.25;
    const RI: f64 = 0.1875;
    const DELTA: f64 = 0.00075;
    const E_H: f64 = 10.3e6;
    const NU_H: f64 = 0.33;
    const E_B: f64 = 17.0e6;
    const NU_B: f64 = 0.33;
    /// Al 7075 flow stress, min(Ftu, sqrt(3) Fsu).
    const SIGMA0: f64 = 83_000.0;

    fn spec(mu: f64) -> BushingSpec {
        BushingSpec { inner_radius: RI, interference: DELTA, e: E_B, nu: NU_B, friction: mu }
    }

    fn plate(edge: f64) -> Geometry {
        let reach = (3.0 * edge).max(10.0 * A);
        Geometry { bore_radius: A, edge, thickness: 0.5, plate_far: reach, plate_half_height: reach, plane_angle_deg: 40.0 }
    }

    fn coarse() -> MeshSpec {
        MeshSpec { n_radial: 8, n_arc: [2, 3, 8], grade: 2.0 }
    }

    fn production() -> MeshSpec {
        MeshSpec { n_radial: 10, n_arc: [3, 4, 10], grade: 2.0 }
    }

    /// Plane-strain Lame: interference pressure of a bushing in a housing of
    /// outer radius `b`.
    fn lame_pressure(b: f64) -> f64 {
        let h = (1.0 + NU_H) / E_H * A * ((1.0 - 2.0 * NU_H) * A * A + b * b) / (b * b - A * A);
        let bu = (1.0 + NU_B) / E_B * A * ((1.0 - 2.0 * NU_B) * A * A + RI * RI) / (A * A - RI * RI);
        DELTA / (h + bu)
    }

    #[test]
    fn the_iteration_matrix_is_the_derivative_of_the_residual_for_closed_sticking_contact() {
        let m = ContactMesh::build(&plate(30.0 * A), &spec(0.15), E_H, NU_H, coarse(), 3, true).unwrap();
        let st = m.new_state();
        let mut ev = Eval::default();
        let u0: Vec<f64> = (0..m.ndof).map(|i| 1e-6 * ((i * 7919) % 13) as f64 / 13.0).collect();
        m.eval(&u0, 0.0, DELTA / 4.0, &st, BIG, &mut ev);
        let mat = m.assemble(&ev);
        let v: Vec<f64> = (0..m.ndof).map(|i| if m.constrained[i] { 0.0 } else { ((i * 31) % 7) as f64 - 3.0 }).collect();
        let eps = 1e-9;
        let u1: Vec<f64> = u0.iter().zip(&v).map(|(a, b)| a + eps * b).collect();
        let mut ev1 = Eval::default();
        m.eval(&u1, 0.0, DELTA / 4.0, &st, BIG, &mut ev1);
        let mv = mat.mul(&v);
        let (mut num, mut den) = (0.0, 0.0);
        for i in (0..m.ndof).filter(|&i| !m.constrained[i]) {
            num += ((ev1.f_int[i] - ev.f_int[i]) / eps - mv[i]).powi(2);
            den += mv[i].powi(2);
        }
        assert!((num / den).sqrt() < 1e-3, "relative mismatch {}", (num / den).sqrt());
    }

    #[test]
    fn the_interference_pressure_matches_plane_strain_lame_in_a_large_plate() {
        let m = ContactMesh::build(&plate(30.0 * A), &spec(0.15), E_H, NU_H, production(), 3, true).unwrap();
        let (p, _) = m.fit_only(DELTA).unwrap();
        let want = lame_pressure(1e6);
        assert!((p / want - 1.0).abs() < 0.01, "FE {p:.1} vs Lame {want:.1} psi");
    }

    #[test]
    fn the_pressure_on_the_bore_carries_exactly_the_pin_load() {
        // Frictionless: the x-resultant of the bushing's pressure on the
        // housing (both halves) is the pin load, whatever the distribution.
        let m = ContactMesh::build(&plate(1.5 * 2.0 * A), &spec(0.0), E_H, NU_H, production(), 3, true).unwrap();
        let (p_pt, dist) = m.elastic_push(DELTA, 2e-4).unwrap();
        let fx: f64 = dist.iter().map(|(c, p, w)| 2.0 * p * c * w).sum();
        assert!(p_pt > 100.0, "pin load {p_pt}");
        assert!((fx + p_pt).abs() < 0.02 * p_pt, "housing resultant {fx} vs pin load {p_pt}");
        // The loaded side is compressed harder than the back side.
        let front = dist.iter().filter(|(c, ..)| *c < -0.9).map(|d| d.1).fold(0.0, f64::max);
        let back = dist.iter().filter(|(c, ..)| *c > 0.9).map(|d| d.1).fold(f64::INFINITY, f64::min);
        assert!(front > back + 1000.0, "front {front}, back {back}");
    }

    #[test]
    fn the_interface_loses_contact_on_the_back_side_when_the_pin_load_exceeds_the_fit() {
        let m = ContactMesh::build(&plate(3.0 * 2.0 * A), &spec(0.0), E_H, NU_H, production(), 3, true).unwrap();
        let (_, light) = m.elastic_push(DELTA, 1e-4).unwrap();
        let (p_heavy, heavy) = m.elastic_push(DELTA, 6e-3).unwrap();
        let open = |d: &[(f64, f64, f64)]| d.iter().filter(|(_, p, _)| *p <= 0.0).count();
        assert_eq!(open(&light), 0, "a small pin load must not open the interface");
        assert!(open(&heavy) > 0, "a large pin load must open the back side");
        // The cosine-load models switch off the back-side contact at
        // P = p pi a t; the real bushing keeps it to a much larger load.
        let p_fit = light.iter().map(|d| d.1).fold(0.0, f64::max);
        assert!(p_heavy * 0.5 > 2.0 * p_fit * std::f64::consts::PI * A * 0.5, "contact lost already at {} lbf", p_heavy * 0.5);
    }

    #[test]
    fn collapse_grows_with_edge_distance_and_first_yield_precedes_it() {
        let run = |ed: f64| {
            let m = ContactMesh::build(&plate(ed * 2.0 * A), &spec(0.15), E_H, NU_H, coarse(), 3, true).unwrap();
            m.collapse(DELTA, SIGMA0, 0.5).unwrap()
        };
        let (near, far) = (run(1.0), run(3.0));
        assert!(far.collapse > 1.4 * near.collapse, "{near:?} {far:?}");
        for r in [&near, &far] {
            assert!(r.plateau, "the load must plateau: {r:?}");
            assert!(r.first_yield > 0.0 && r.first_yield < 0.8 * r.collapse, "{r:?}");
        }
        // Far from the edge the pin load is limited by bearing on the bore,
        // a few flow stresses over D t (plane strain, confined).
        let bearing = far.collapse / (SIGMA0 * 2.0 * A * 0.5);
        assert!((2.0..3.6).contains(&bearing), "bearing capacity {bearing:.2} sigma0 D t");
    }

    #[test]
    fn the_collapse_load_is_mesh_converged_and_barely_depends_on_the_fit() {
        let run = |mesh: MeshSpec, delta: f64| {
            let m = ContactMesh::build(&plate(1.5 * 2.0 * A), &spec(0.0), E_H, NU_H, mesh, 3, true).unwrap();
            m.collapse(delta, SIGMA0, 0.5).unwrap().collapse
        };
        let fine = run(MeshSpec { n_radial: 14, n_arc: [4, 5, 14], grade: 2.0 }, DELTA);
        let prod = run(production(), DELTA);
        assert!((prod / fine - 1.0).abs() < 0.03, "production {prod:.0} vs fine {fine:.0}");
        // Limit-load theory: a self-equilibrated residual stress does not
        // change the collapse load (the fit is installed before the pin load).
        let no_fit = run(production(), 0.0);
        assert!((prod / no_fit - 1.0).abs() < 0.04, "with fit {prod:.0} vs without {no_fit:.0}");
    }

    #[test]
    fn friction_adds_capacity() {
        let run = |mu: f64| {
            let m = ContactMesh::build(&plate(1.5 * 2.0 * A), &spec(mu), E_H, NU_H, coarse(), 3, true).unwrap();
            m.collapse(DELTA, SIGMA0, 0.5).unwrap().collapse
        };
        let (smooth, rough) = (run(0.0), run(0.3));
        assert!(rough > 1.02 * smooth, "mu 0.3 {rough:.0} vs frictionless {smooth:.0}");
    }

    /// Robustness sweep (slow): every combination must converge and give a
    /// physically ordered result. Run with `--ignored`.
    #[test]
    #[ignore]
    fn sweep_converges_over_geometry_fit_friction_and_bushing_wall() {
        let mut cases = Vec::new();
        for ed in [0.8, 1.5, 2.5] {
            for delta in [0.0003, 0.00075, 0.002] {
                for mu in [0.0, 0.15, 0.4] {
                    for ri in [0.125, 0.1875, 0.22] {
                        cases.push((ed, delta, mu, ri));
                    }
                }
            }
        }
        let results: Vec<_> = cases
            .chunks(8)
            .flat_map(|chunk| {
                std::thread::scope(|sc| {
                    let hs: Vec<_> = chunk
                        .iter()
                        .map(|&(ed, delta, mu, ri)| {
                            sc.spawn(move || {
                                let sp = BushingSpec { inner_radius: ri, interference: delta, e: E_B, nu: NU_B, friction: mu };
                                let m = ContactMesh::build(&plate(ed * 2.0 * A), &sp, E_H, NU_H, production(), 3, true)?;
                                m.collapse(delta, SIGMA0, 0.5)
                            })
                        })
                        .collect();
                    hs.into_iter().map(|h| h.join().unwrap()).collect::<Vec<_>>()
                })
            })
            .collect();
        let mut bad = Vec::new();
        for (c, r) in cases.iter().zip(&results) {
            match r {
                Ok(r) if r.collapse >= r.first_yield && r.first_yield >= 0.0 && r.collapse > 0.0 => {}
                other => bad.push(format!("{c:?}: {other:?}")),
            }
        }
        assert!(bad.is_empty(), "{} of {} failed:\n{}", bad.len(), cases.len(), bad.join("\n"));
    }
}
