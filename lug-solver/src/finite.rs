//! Finite-strain collapse of a pin-loaded lug (plane strain).
//!
//! The small-strain collapse in `plastic.rs` reuses one elastic factorisation; finite strain changes
//! the stiffness every step, so this is a direct nonlinear solve on the whole mesh instead:
//!
//! * total-Lagrangian Q9 elements, the same star mesh as the rest of the crate;
//! * multiplicative J2 plasticity in the logarithmic-strain (Hencky) form of Weber-Anand /
//!   Eterovic-Bathe: the state is the inverse plastic right Cauchy-Green tensor and the
//!   equivalent plastic strain, the return map is the small-strain radial return on the principal
//!   logarithmic elastic strains, and isotropic hardening follows a *true* stress - true strain
//!   curve (`Hardening::true_curve`);
//! * the consistent tangent `dP/dF` by forward differences of the (cheap) point update;
//! * the pin is the rigid analytic circle at its current position, contact by Gauss points on the
//!   bore edges with a penalty, an augmented-Lagrangian normal multiplier and Coulomb friction
//!   (tangential multiplier, incremental slip), including the geometric term of the contact tangent;
//! * Newton with an Armijo line search, chord reuse of the banded Cholesky, and pin-travel steps
//!   that halve on failure.
//!
//! Collapse is the peak of the load-travel curve or the load at which the equivalent plastic strain
//! anywhere reaches the failure strain, whichever comes first.

#![allow(clippy::needless_range_loop)] // index-parallel numeric kernels read clearer as loops

use crate::geometry::LugGeometry;
use crate::mesh::{Mesh, MeshSpec};
use crate::solve::LoadCase;
use crate::plastic::Hardening;
use edge_check::fem::{shape1, shape_q9, GAUSS3};
use crate::sparse::{nested_dissection, Cholesky, Pattern, SparseSym, Symbolic};
use std::sync::Arc;
use std::time::Instant;

/// Material of the finite-strain analysis: true constants and a true yield curve.
#[derive(Debug, Clone, Copy)]
pub struct FsMaterial {
    pub e_psi: f64,
    pub nu: f64,
    /// True yield stress against true equivalent plastic strain.
    pub law: Hardening,
}

/// History of one material point.
#[derive(Debug, Clone, Copy)]
pub struct Point {
    /// In-plane `[xx, yy, xy]` of the inverse plastic right Cauchy-Green tensor `Fp^-1 Fp^-T`.
    cinv: [f64; 3],
    /// Its out-of-plane component (plane strain: `Fp_zz^-2`).
    czz: f64,
    /// Equivalent plastic strain.
    pub ep: f64,
}

impl Default for Point {
    fn default() -> Self {
        Self { cinv: [1.0, 1.0, 0.0], czz: 1.0, ep: 0.0 }
    }
}

type M2 = [[f64; 2]; 2];

fn mul(a: &M2, b: &M2) -> M2 {
    [[a[0][0] * b[0][0] + a[0][1] * b[1][0], a[0][0] * b[0][1] + a[0][1] * b[1][1]], [a[1][0] * b[0][0] + a[1][1] * b[1][0], a[1][0] * b[0][1] + a[1][1] * b[1][1]]]
}

fn transpose(a: &M2) -> M2 {
    [[a[0][0], a[1][0]], [a[0][1], a[1][1]]]
}

fn inverse(a: &M2) -> (M2, f64) {
    let det = a[0][0] * a[1][1] - a[0][1] * a[1][0];
    ([[a[1][1] / det, -a[0][1] / det], [-a[1][0] / det, a[0][0] / det]], det)
}

impl FsMaterial {
    fn shear(&self) -> f64 {
        self.e_psi / (2.0 * (1.0 + self.nu))
    }

    fn bulk(&self) -> f64 {
        self.e_psi / (3.0 * (1.0 - 2.0 * self.nu))
    }

    /// Material-point update for the in-plane deformation gradient `f` (`Fzz = 1`): returns the new
    /// history and the first Piola-Kirchhoff stress `P = tau F^-T` (per unit reference thickness).
    pub fn update(&self, old: &Point, f: &M2) -> (Point, M2) {
        let (finv, j) = inverse(f);
        // Trial elastic left Cauchy-Green tensor Be = F Cinv F^T (in plane), Be_zz = czz.
        let ci = [[old.cinv[0], old.cinv[2]], [old.cinv[2], old.cinv[1]]];
        let be = mul(&mul(f, &ci), &transpose(f));
        let (a, b, c) = (be[0][0], 0.5 * (be[0][1] + be[1][0]), be[1][1]);
        let mean = 0.5 * (a + c);
        let rad = (0.25 * (a - c) * (a - c) + b * b).sqrt();
        let (l1, l2) = ((mean + rad).max(1e-300), (mean - rad).max(1e-300));
        let theta = 0.5 * (2.0 * b).atan2(a - c);
        let (ct, st) = (theta.cos(), theta.sin());
        let e = [0.5 * l1.ln(), 0.5 * l2.ln(), 0.5 * old.czz.max(1e-300).ln()];
        let tr = e[0] + e[1] + e[2];
        let dev = [e[0] - tr / 3.0, e[1] - tr / 3.0, e[2] - tr / 3.0];
        let g = self.shear();
        let dnorm = (dev[0] * dev[0] + dev[1] * dev[1] + dev[2] * dev[2]).sqrt();
        // von Mises of the Kirchhoff stress: sqrt(3/2) |2 G dev|.
        let q = 3.0f64.sqrt() * g * 2.0f64.sqrt() * dnorm;
        let mut ep = old.ep;
        let mut dev_new = dev;
        let y0 = self.law.stress(ep);
        if q > y0 && dnorm > 0.0 {
            let d = self.law.plastic_increment(ep, q, g);
            let scale = 1.0 - 3.0 * g * d / q;
            for v in &mut dev_new {
                *v *= scale;
            }
            ep += d;
        }
        let k = self.bulk();
        let en = [dev_new[0] + tr / 3.0, dev_new[1] + tr / 3.0, dev_new[2] + tr / 3.0];
        let tau = [2.0 * g * dev_new[0] + k * tr, 2.0 * g * dev_new[1] + k * tr];
        let (l1n, l2n, l3n) = ((2.0 * en[0]).exp(), (2.0 * en[1]).exp(), (2.0 * en[2]).exp());
        // Be_new and Kirchhoff stress in the principal frame (n1 = (c, s), n2 = (-s, c)).
        let be_new = [[l1n * ct * ct + l2n * st * st, (l1n - l2n) * ct * st], [(l1n - l2n) * ct * st, l1n * st * st + l2n * ct * ct]];
        let t = [[tau[0] * ct * ct + tau[1] * st * st, (tau[0] - tau[1]) * ct * st], [(tau[0] - tau[1]) * ct * st, tau[0] * st * st + tau[1] * ct * ct]];
        let ci_new = mul(&mul(&finv, &be_new), &transpose(&finv));
        let _ = j;
        let p = mul(&t, &transpose(&finv));
        (Point { cinv: [ci_new[0][0], ci_new[1][1], 0.5 * (ci_new[0][1] + ci_new[1][0])], czz: l3n, ep }, p)
    }

    /// `dP/dF` by forward differences (`a[2 i + j][2 k + l] = dP_ij / dF_kl`).
    pub fn tangent(&self, old: &Point, f: &M2, p0: &M2) -> [[f64; 4]; 4] {
        let mut a = [[0.0; 4]; 4];
        let h = 1e-7;
        for k in 0..2 {
            for l in 0..2 {
                let mut fp = *f;
                fp[k][l] += h;
                let (_, p) = self.update(old, &fp);
                for i in 0..2 {
                    for jx in 0..2 {
                        a[2 * i + jx][2 * k + l] = (p[i][jx] - p0[i][jx]) / h;
                    }
                }
            }
        }
        // The exact tangent is symmetric for this model; the differences are not quite.
        for i in 0..4 {
            for j in 0..i {
                let v = 0.5 * (a[i][j] + a[j][i]);
                a[i][j] = v;
                a[j][i] = v;
            }
        }
        a
    }
}


// ---------------------------------------------------------------------------
// The lug
// ---------------------------------------------------------------------------

/// Reference-configuration data of one element Gauss point.
struct Gp {
    /// `dN_a / dX` for the nine nodes.
    dn: [[f64; 2]; 9],
    w: f64,
}

/// One contact integration point on a bore edge (reference position, shape values, length weight).
struct Cp {
    nodes: [usize; 3],
    n: [f64; 3],
    w: f64,
    x0: [f64; 2],
}

#[derive(Debug, Clone, Copy)]
pub struct FsOptions {
    /// Largest pin travel past the first contact, as a multiple of the bore radius.
    pub travel_cap_over_a: f64,
    /// Failure: stop where the equivalent plastic strain anywhere reaches this.
    pub strain_limit: Option<f64>,
    /// Coulomb friction between pin and bore.
    pub friction: f64,
    pub pin_dia: f64,
    /// Normal penalty as a multiple of `E / a`.
    pub penalty_factor: f64,
}

impl FsOptions {
    pub fn new(pin_dia: f64, friction: f64) -> Self {
        Self { travel_cap_over_a: 1.0, strain_limit: None, friction, pin_dia, penalty_factor: 100.0 }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct FsPoint {
    /// Pin travel from first contact (in).
    pub travel: f64,
    pub load_lbf: f64,
    pub max_ep: f64,
}

#[derive(Debug, Clone)]
pub struct FsResult {
    pub curve: Vec<FsPoint>,
    /// Peak of the curve, or the load at the failure strain.
    pub collapse_lbf: f64,
    /// The load fell after its peak (a true limit).
    pub peak_reached: bool,
    /// The failure strain was reached first.
    pub strain_limited: bool,
    pub max_ep: f64,
    pub newton_iterations: usize,
    pub factorisations: usize,
    pub elapsed_ms: f64,
    pub note: Option<String>,
}

pub struct FiniteLug {
    mesh: Mesh,
    geom: LugGeometry,
    mat: FsMaterial,
    half: bool,
    gps: Vec<Gp>,
    cps: Vec<Cp>,
    pattern: Arc<Pattern>,
    symbolic: Arc<Symbolic>,
    constrained: Vec<bool>,
}

/// Committed state between pin-travel steps.
#[derive(Clone)]
struct State {
    u: Vec<f64>,
    q: f64,
    pts: Vec<Point>,
    lam: Vec<f64>,
    lam_t: Vec<f64>,
    phi: Vec<f64>,
    /// The previous converged state's displacements and pin position: the predictor's secant.
    prev: Option<(Vec<f64>, f64, f64)>,
    /// Pin travel of this state.
    s: f64,
}

struct Eval {
    /// Residual `f_int - f_contact` in node layout (`2 * node + comp`), zero on constrained dofs.
    r: Vec<f64>,
    r_q: f64,
    k: Option<SparseSym>,
    /// `dR_u / dq` and `dR_q / dq` (pin sideways coordinate).
    h: Vec<f64>,
    c_qq: f64,
    pts: Vec<Point>,
    pressure: Vec<f64>,
    shear: Vec<f64>,
    phi: Vec<f64>,
    /// Total contact force on the lug (per unit thickness).
    force: [f64; 2],
    max_ep: f64,
}

impl FiniteLug {
    /// Mesh the lug and set up the element data. `half` is the symmetric `y >= 0` model (axial loads).
    pub fn build(geom: &LugGeometry, spec: MeshSpec, mat: FsMaterial, half: bool) -> Result<Self, String> {
        let mesh = Mesh::build(geom, spec, half)?;
        let mut gps = Vec::with_capacity(mesh.elems.len() * 9);
        for e in &mesh.elems {
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
        let mut cps = Vec::new();
        for edge in &mesh.bore_edges {
            let p = [mesh.nodes[edge[0]], mesh.nodes[edge[1]], mesh.nodes[edge[2]]];
            for &(eta, wq) in &GAUSS3 {
                let (n, dn) = shape1(eta);
                let (mut x0, mut dx) = ([0.0; 2], [0.0; 2]);
                for k in 0..3 {
                    for c in 0..2 {
                        x0[c] += n[k] * p[k][c];
                        dx[c] += dn[k] * p[k][c];
                    }
                }
                cps.push(Cp { nodes: *edge, n, w: wq * dx[0].hypot(dx[1]), x0 });
            }
        }
        // Fill-reducing ordering: geometric nested dissection of the node graph.
        let nn = mesh.nodes.len();
        let mut adj: Vec<Vec<usize>> = vec![Vec::new(); nn];
        for e in &mesh.elems {
            for &a in e {
                for &b in e {
                    if a != b {
                        adj[a].push(b);
                    }
                }
            }
        }
        for l in &mut adj {
            l.sort_unstable();
            l.dedup();
        }
        let node_order = nested_dissection(&mesh.nodes, &adj);
        let dof_order: Vec<usize> = node_order.iter().flat_map(|&n| [2 * n, 2 * n + 1]).collect();
        let pairs = mesh.elems.iter().flat_map(|e| {
            e.iter().flat_map(move |&a| e.iter().flat_map(move |&b| (0..2).flat_map(move |i| (0..2).map(move |j| (2 * a + i, 2 * b + j)))))
        });
        let pattern = Arc::new(Pattern::new(2 * nn, &dof_order, pairs));
        let symbolic = Arc::new(Symbolic::new(&pattern));
        let mut constrained = vec![false; 2 * mesh.nodes.len()];
        for (id, p) in mesh.nodes.iter().enumerate() {
            if p[0] >= geom.length - 1e-9 {
                constrained[2 * id] = true;
                constrained[2 * id + 1] = true;
            }
            if half && p[1].abs() < 1e-12 {
                constrained[2 * id + 1] = true;
            }
        }
        Ok(Self { mesh, geom: *geom, mat, half, gps, cps, pattern, symbolic, constrained })
    }

    pub fn nodes(&self) -> usize {
        self.mesh.nodes.len()
    }

    /// Milliseconds of one assembly of the undeformed tangent and one factorisation (diagnostics).
    pub fn time_factor(&self) -> (f64, f64) {
        let nn = self.mesh.nodes.len();
        let pts = vec![Point::default(); self.gps.len()];
        let ng = self.cps.len();
        let (lam, lam_t, phi) = (vec![0.0; ng], vec![0.0; ng], vec![0.0; ng]);
        let ctx = Ctx { pts: &pts, lam: &lam, lam_t: &lam_t, phi: &phi, pin_radius: 0.0, kn: 0.0, kt: 0.0, mu: 0.0 };
        let t0 = Instant::now();
        let ev = self.evaluate(&vec![0.0; 2 * nn], [0.0, 0.0], [0.0, 1.0], &ctx, true, false);
        let assemble = t0.elapsed().as_secs_f64() * 1e3;
        let mut k = ev.k.expect("tangent");
        for (d, &fixed) in self.constrained.iter().enumerate() {
            if fixed {
                k.add(d, d, 1e12);
            }
        }
        let t1 = Instant::now();
        let _ = Cholesky::factor(&k, &self.symbolic);
        (assemble, t1.elapsed().as_secs_f64() * 1e3)
    }

    /// `(nnz(L), multiply-adds per factorisation)` of the sparse factor.
    pub fn factor_size(&self) -> (usize, f64) {
        (self.symbolic.nnz(), self.symbolic.flops())
    }

    /// Residual (and, when asked, the tangent) of the displacement field `u` with the pin centre at
    /// `centre`. `ctx` carries the multipliers and history of the current step.
    fn evaluate(&self, u: &[f64], centre: [f64; 2], perp: [f64; 2], ctx: &Ctx, with_tangent: bool, geo: bool) -> Eval {
        self.evaluate_i(u, centre, perp, ctx, with_tangent, geo)
    }

    fn evaluate_i(&self, u: &[f64], centre: [f64; 2], perp: [f64; 2], ctx: &Ctx, with_tangent: bool, geo: bool) -> Eval {
        let nn = self.mesh.nodes.len();
        let mut r = vec![0.0; 2 * nn];
        let mut k = if with_tangent { Some(SparseSym::zeros(&self.pattern)) } else { None };
        let mut pts: Vec<Point> = Vec::with_capacity(self.gps.len());
        let ne = self.mesh.elems.len();
        let workers = std::thread::available_parallelism().map_or(1, |n| n.get()).min(8).min(ne.max(1));
        let chunk = ne.div_ceil(workers.max(1));
        // Element results: nodal force (18), optional stiffness (18 x 18), nine new points.
        struct ElemOut {
            fe: [f64; 18],
            ke: Vec<f64>,
            pts: [Point; 9],
        }
        let compute = |ei: usize| -> ElemOut {
            let e = &self.mesh.elems[ei];
            let ue: Vec<[f64; 2]> = e.iter().map(|&n| [u[2 * n], u[2 * n + 1]]).collect();
            let mut ke = if with_tangent { vec![0.0; 18 * 18] } else { Vec::new() };
            let mut fe = [0.0; 18];
            let mut out_pts = [Point::default(); 9];
            for g in 0..9 {
                let gp = &self.gps[ei * 9 + g];
                let mut f = [[1.0, 0.0], [0.0, 1.0]];
                for a in 0..9 {
                    for i in 0..2 {
                        for j in 0..2 {
                            f[i][j] += ue[a][i] * gp.dn[a][j];
                        }
                    }
                }
                let old = &ctx.pts[ei * 9 + g];
                let (new, p) = self.mat.update(old, &f);
                out_pts[g] = new;
                for a in 0..9 {
                    for i in 0..2 {
                        fe[2 * a + i] += gp.w * (p[i][0] * gp.dn[a][0] + p[i][1] * gp.dn[a][1]);
                    }
                }
                if with_tangent {
                    let at = self.mat.tangent(old, &f, &p);
                    for a in 0..9 {
                        for b in 0..9 {
                            for i in 0..2 {
                                for kk in 0..2 {
                                    let mut v = 0.0;
                                    for j in 0..2 {
                                        for l in 0..2 {
                                            v += at[2 * i + j][2 * kk + l] * gp.dn[a][j] * gp.dn[b][l];
                                        }
                                    }
                                    ke[(2 * a + i) * 18 + 2 * b + kk] += gp.w * v;
                                }
                            }
                        }
                    }
                }
            }
            ElemOut { fe, ke, pts: out_pts }
        };
        let outs: Vec<ElemOut> = std::thread::scope(|sc| {
            let hs: Vec<_> = (0..workers)
                .map(|w| {
                    let compute = &compute;
                    sc.spawn(move || ((w * chunk)..((w + 1) * chunk).min(ne)).map(compute).collect::<Vec<_>>())
                })
                .collect();
            hs.into_iter().flat_map(|h| h.join().unwrap()).collect()
        });
        let mut max_ep = 0.0f64;
        for (ei, out) in outs.iter().enumerate() {
            let e = &self.mesh.elems[ei];
            for a in 0..9 {
                for i in 0..2 {
                    r[2 * e[a] + i] += out.fe[2 * a + i];
                }
            }
            for p in &out.pts {
                max_ep = max_ep.max(p.ep);
            }
            pts.extend_from_slice(&out.pts);
            if let Some(km) = k.as_mut() {
                self.scatter(km, e, &out.ke);
            }
        }

        // Pin contact.
        let ng = self.cps.len();
        let mut pressure = vec![0.0; ng];
        let mut shear = vec![0.0; ng];
        let mut phi_new = vec![0.0; ng];
        let mut force = [0.0, 0.0];
        let mut r_q = 0.0;
        let mut c_qq = 0.0;
        let mut h = vec![0.0; 2 * nn];
        for (gi, cp) in self.cps.iter().enumerate() {
            let mut d = [0.0; 2];
            for kk in 0..3 {
                for c in 0..2 {
                    d[c] += cp.n[kk] * u[2 * cp.nodes[kk] + c];
                }
            }
            let rel = [cp.x0[0] + d[0] - centre[0], cp.x0[1] + d[1] - centre[1]];
            let dist = rel[0].hypot(rel[1]).max(1e-300);
            let nrm = [rel[0] / dist, rel[1] / dist];
            let tau = [-nrm[1], nrm[0]];
            let gap = dist - ctx.pin_radius;
            let p = (ctx.lam[gi] + ctx.kn * (-gap)).max(0.0);
            let phi = rel[1].atan2(rel[0]);
            phi_new[gi] = phi;
            let (mut t, mut stick) = (0.0, false);
            if ctx.mu > 0.0 && p > 0.0 {
                let mut dphi = phi - ctx.phi[gi];
                if dphi > std::f64::consts::PI {
                    dphi -= 2.0 * std::f64::consts::PI;
                } else if dphi < -std::f64::consts::PI {
                    dphi += 2.0 * std::f64::consts::PI;
                }
                let trial = ctx.lam_t[gi] - ctx.kt * dist * dphi;
                let cap = ctx.mu * p;
                if trial.abs() > cap {
                    t = cap * trial.signum();
                } else {
                    t = trial;
                    stick = true;
                }
            }
            pressure[gi] = p;
            shear[gi] = t;
            if p <= 0.0 {
                continue;
            }
            let f = [cp.w * (p * nrm[0] + t * tau[0]), cp.w * (p * nrm[1] + t * tau[1])];
            force[0] += f[0];
            force[1] += f[1];
            r_q += f[0] * perp[0] + f[1] * perp[1];
            for kk in 0..3 {
                for c in 0..2 {
                    r[2 * cp.nodes[kk] + c] -= cp.n[kk] * f[c];
                }
            }
            // d f_contact = -M d rel with M = w (kn n n^T + kt tau tau^T [stick] - p / dist (I - n n^T)).
            let mut m = [[0.0; 2]; 2];
            for i in 0..2 {
                for j in 0..2 {
                    m[i][j] = cp.w * ctx.kn * nrm[i] * nrm[j] + if stick { cp.w * ctx.kt * tau[i] * tau[j] } else { 0.0 };
                    if geo {
                        m[i][j] -= cp.w * p / dist * ((i == j) as u8 as f64 - nrm[i] * nrm[j]);
                    }
                }
            }
            if let Some(km) = k.as_mut() {
                for a in 0..3 {
                    for b in 0..3 {
                        for i in 0..2 {
                            for j in 0..2 {
                                let v = cp.n[a] * cp.n[b] * m[i][j];
                                self.add(km, cp.nodes[a], i, cp.nodes[b], j, v);
                            }
                        }
                    }
                }
            }
            // Sideways pin coordinate: d rel / dq = -perp.
            let mp = [m[0][0] * perp[0] + m[0][1] * perp[1], m[1][0] * perp[0] + m[1][1] * perp[1]];
            c_qq += perp[0] * mp[0] + perp[1] * mp[1];
            for kk in 0..3 {
                for c in 0..2 {
                    h[2 * cp.nodes[kk] + c] -= cp.n[kk] * mp[c];
                }
            }
        }
        for (i, &fixed) in self.constrained.iter().enumerate() {
            if fixed {
                r[i] = 0.0;
                h[i] = 0.0;
            }
        }
        Eval { r, r_q, k, h, c_qq, pts, pressure, shear, phi: phi_new, force, max_ep }
    }

    fn add(&self, k: &mut SparseSym, na: usize, ca: usize, nb: usize, cb: usize, v: f64) {
        let (i, j) = (2 * na + ca, 2 * nb + cb);
        if self.constrained[i] || self.constrained[j] || j > i {
            return; // constrained dofs get a unit diagonal later; one triangle only
        }
        k.add(i, j, v);
    }

    fn scatter(&self, k: &mut SparseSym, e: &[usize; 9], ke: &[f64]) {
        for a in 0..9 {
            for i in 0..2 {
                for b in 0..9 {
                    for j in 0..2 {
                        self.add(k, e[a], i, e[b], j, ke[(2 * a + i) * 18 + 2 * b + j]);
                    }
                }
            }
        }
    }

}

/// Everything fixed during one pin-travel step.
struct Ctx<'a> {
    pts: &'a [Point],
    lam: &'a [f64],
    lam_t: &'a [f64],
    phi: &'a [f64],
    pin_radius: f64,
    kn: f64,
    kt: f64,
    mu: f64,
}


const MAX_OUTER: usize = 3;
const FRICTION_EXTRA_PASSES: usize = 4;
const STICK_RATIO: f64 = 0.005;

impl FiniteLug {
    /// Quasi-Newton on `(u, q)` for fixed multipliers: the consistent tangent is factored rarely
    /// (the factor in `chord` survives between calls) and the L-BFGS pairs of this solve correct its
    /// staleness, so a pin-travel step costs about one factorisation instead of one per iteration.
    #[allow(clippy::too_many_arguments)]
    fn newton(&self, u: &mut Vec<f64>, q: &mut f64, s: f64, dir: [f64; 2], ctx: &Ctx, free_perp: bool, tol: f64, quasi: bool, stats: &mut (usize, usize), chord: &mut Option<Cholesky>) -> Result<(), String> {
        let perp = [-dir[1], dir[0]];
        let centre_of = |q: f64| [s * dir[0] + q * perp[0], s * dir[1] + q * perp[1]];
        let f_scale = (self.mat.e_psi * self.mesh.bore_radius * 1e-6).max(1e-12);
        let nu = u.len();
        // Residual as one vector `(R_u, R_q)`.
        let pack = |ev: &Eval| -> Vec<f64> {
            let mut g = ev.r.clone();
            g.push(if free_perp { ev.r_q } else { 0.0 });
            g
        };
        let norm2 = |g: &[f64]| g.iter().map(|v| v * v).sum::<f64>().sqrt();
        let norm_inf = |g: &[f64]| g.iter().fold(0.0f64, |m, v| m.max(v.abs()));
        let mut geo = true;
        let mut ev = self.evaluate(u, centre_of(*q), perp, ctx, false, geo);
        let mut g = pack(&ev);
        // Bordered coupling of the last factorisation.
        let (mut h, mut c_qq) = (vec![0.0; nu], 0.0);
        let mut hist: Vec<(Vec<f64>, Vec<f64>, f64)> = Vec::new();
        let mut since_factor = 0;
        let mut slow = 0;
        let mut prev_res = f64::INFINITY;
        for _ in 0..80 {
            stats.0 += 1;
            let scale = f_scale.max(ev.force[0].hypot(ev.force[1]) * 0.1);
            let res = norm_inf(&g) / scale;
            if res < tol {
                return Ok(());
            }
            if !res.is_finite() {
                return Err("the residual is not finite".into());
            }
            slow = if res > 0.7 * prev_res { slow + 1 } else { 0 };
            prev_res = res;
            // While the contact is still establishing itself the tangent changes every iteration (a
            // sideways pin coordinate with no stiffness): plain Newton, one factorisation each.
            if chord.is_none() || !quasi || slow >= 3 || since_factor >= 14 {
                loop {
                    let with_k = self.evaluate(u, centre_of(*q), perp, ctx, true, geo);
                    let mut k = with_k.k.expect("tangent requested");
                    let dmax = (0..nu).map(|i| k.diag(i)).fold(0.0, f64::max).max(1.0);
                    for (d, &fixed) in self.constrained.iter().enumerate() {
                        if fixed {
                            k.add(d, d, dmax);
                        }
                    }
                    stats.1 += 1;
                    let factor = Cholesky::factor(&k, &self.symbolic);
                    if let Some(f) = factor {
                        *chord = Some(f);
                        h = with_k.h;
                        c_qq = with_k.c_qq;
                        break;
                    }
                    if geo {
                        geo = false; // drop the (possibly softening) geometric contact term
                        continue;
                    }
                    *chord = None;
                    return Err("the tangent is not positive definite".into());
                }
                hist.clear();
                since_factor = 0;
                slow = 0;
            }
            let k = chord.as_ref().expect("factor present");
            // y2 = K^-1 h for the bordered scalar, computed once per factor use.
            let y2 = if free_perp {
                let mut y = h.clone();
                k.solve_in_place(&mut y);
                Some(y)
            } else {
                None
            };
            let hy2: f64 = y2.as_ref().map_or(0.0, |y| h.iter().zip(y).map(|(a, b)| a * b).sum());
            // H0 g: the (stale) tangent's step for a residual `(g_u, g_q)`, negated.
            let h0 = |g: &[f64]| -> Vec<f64> {
                let mut du: Vec<f64> = g[..nu].iter().map(|v| -v).collect();
                k.solve_in_place(&mut du);
                let mut dq = 0.0;
                if let Some(y2) = &y2 {
                    let hy1: f64 = h.iter().zip(&du).map(|(a, b)| a * b).sum();
                    let den = c_qq - hy2 + 1e-5 * self.mat.e_psi;
                    if den.abs() > 1e-12 * (c_qq.abs() + 1.0) {
                        dq = (-g[nu] - hy1) / den;
                        for (d, y) in du.iter_mut().zip(y2) {
                            *d -= y * dq;
                        }
                    }
                }
                du.push(dq);
                du.iter().map(|v| -v).collect() // H0 g = -(Newton step)
            };
            // L-BFGS two-loop recursion for d = -H g.
            let mut qv = g.clone();
            let mut alphas = vec![0.0; hist.len()];
            for (i, (sv, yv, rho)) in hist.iter().enumerate().rev() {
                let a = rho * sv.iter().zip(&qv).map(|(a, b)| a * b).sum::<f64>();
                alphas[i] = a;
                for (x, y) in qv.iter_mut().zip(yv) {
                    *x -= a * y;
                }
            }
            let mut rv = h0(&qv);
            for (i, (sv, yv, rho)) in hist.iter().enumerate() {
                let b = rho * yv.iter().zip(&rv).map(|(a, b)| a * b).sum::<f64>();
                for (x, sx) in rv.iter_mut().zip(sv) {
                    *x += (alphas[i] - b) * sx;
                }
            }
            let mut d: Vec<f64> = rv.iter().map(|v| -v).collect();
            // Trust region on the pin's sideways coordinate: with no contact it has no stiffness and an
            // unbounded step would walk the pin away (a spurious zero-force equilibrium).
            let q_cap = 0.05 * self.mesh.bore_radius;
            if d[nu].abs() > q_cap {
                let f = q_cap / d[nu].abs();
                d.iter_mut().for_each(|v| *v *= f);
            }
            let r0 = norm2(&g);
            let mut alpha = 1.0;
            let mut accepted = false;
            for _ in 0..25 {
                let un: Vec<f64> = u.iter().zip(&d).map(|(a, b)| a + alpha * b).collect();
                let qn = *q + alpha * d[nu];
                let trial = self.evaluate(&un, centre_of(qn), perp, ctx, false, geo);
                let gt = pack(&trial);
                let rt = norm2(&gt);
                if rt.is_finite() && rt < (1.0 - 1e-4 * alpha) * r0 {
                    let sv: Vec<f64> = d.iter().map(|v| alpha * v).collect();
                    let yv: Vec<f64> = gt.iter().zip(&g).map(|(a, b)| a - b).collect();
                    let sy: f64 = sv.iter().zip(&yv).map(|(a, b)| a * b).sum();
                    if sy > 1e-12 * norm2(&sv) * norm2(&yv) {
                        hist.push((sv, yv, 1.0 / sy));
                        if hist.len() > 12 {
                            hist.remove(0);
                        }
                    }
                    *u = un;
                    *q = qn;
                    ev = trial;
                    g = gt;
                    accepted = true;
                    break;
                }
                alpha *= 0.5;
            }
            since_factor += 1;
            if !accepted {
                if res < 1e-3 {
                    return Ok(());
                }
                if hist.is_empty() && since_factor <= 1 {
                    *chord = None;
                    return Err(format!("the line search stalled (residual {res:.2e})"));
                }
                // The correction memory or the stale factor misled the step: start afresh.
                *chord = None;
                hist.clear();
            }
        }
        Err(format!("Newton did not converge (residual {:.2e})", prev_res))
    }

    /// One pin-travel step: augmented-Lagrangian passes around the Newton solve.
    #[allow(clippy::too_many_arguments)]
    fn step(&self, st: &State, s: f64, dir: [f64; 2], opts: &FsOptions, quasi: bool, stats: &mut (usize, usize), chord: &mut Option<Cholesky>) -> Result<(State, Eval), String> {
        let free_perp = !self.half;
        let a = self.mesh.bore_radius;
        let kn = opts.penalty_factor * self.mat.e_psi / a;
        let mut u = st.u.clone();
        let mut q = st.q;
        // Predictor: continue along the secant of the last two converged states (the pin pushes the
        // bore ahead of it; starting from the old field would begin deep in the penalty).
        if let Some((up, qp, sp)) = &st.prev {
            let ratio = ((s - st.s) / (st.s - sp).max(1e-12)).clamp(0.0, 2.0);
            for (x, xp) in u.iter_mut().zip(up) {
                *x += ratio * (*x - xp);
            }
            q += ratio * (q - qp);
        }
        let mut lam = st.lam.clone();
        let mut lam_t = st.lam_t.clone();
        let passes = if opts.friction > 0.0 { MAX_OUTER + FRICTION_EXTRA_PASSES } else { MAX_OUTER };
        let perp = [-dir[1], dir[0]];
        let mut last = None;
        for pass in 0..passes {
            let ctx = Ctx { pts: &st.pts, lam: &lam, lam_t: &lam_t, phi: &st.phi, pin_radius: opts.pin_dia / 2.0, kn, kt: STICK_RATIO * kn, mu: opts.friction };
            // Early passes only need to be near; the last one is converged.
            let tol = if pass + 1 == passes { 1e-6 } else { 1e-2 };
            self.newton(&mut u, &mut q, s, dir, &ctx, free_perp, tol, quasi, stats, chord)?;
            let ev = self.evaluate(&u, [s * dir[0] + q * perp[0], s * dir[1] + q * perp[1]], perp, &ctx, false, true);
            let peak = ev.pressure.iter().fold(0.0f64, |m, p| m.max(*p));
            let change = ev.pressure.iter().zip(&lam).fold(0.0f64, |m, (p, l)| m.max((p - l).abs()));
            let change_t = ev.shear.iter().zip(&lam_t).fold(0.0f64, |m, (t, l)| m.max((t - l).abs()));
            lam.clone_from(&ev.pressure);
            lam_t.clone_from(&ev.shear);
            let done = change <= 1e-2 * peak.max(1e-12) && change_t <= 2e-3 * (opts.friction * peak).max(1e-12);
            last = Some(ev);
            if done {
                break;
            }
        }
        let ev = last.expect("at least one pass");
        let next = State { u, q, pts: ev.pts.clone(), lam, lam_t, phi: ev.phi.clone(), prev: Some((st.u.clone(), st.q, st.s)), s };
        Ok((next, ev))
    }

    /// Drive the pin along the load direction until the lug collapses.
    pub fn collapse(&self, case: LoadCase, opts: FsOptions) -> Result<FsResult, String> {
        if !(opts.pin_dia.is_finite() && opts.pin_dia > 0.0) {
            return Err("pin diameter must be positive".into());
        }
        if self.half && !case.is_axial() {
            return Err("a symmetric (half) model only supports axial loads".into());
        }
        let t0 = Instant::now();
        let dir = case.direction();
        let a = self.mesh.bore_radius;
        let t = self.geom.thickness;
        let share = if self.half { 2.0 } else { 1.0 };
        let clearance = (a - opts.pin_dia / 2.0).max(0.0);
        let ng = self.cps.len();
        let mut st = State { u: vec![0.0; 2 * self.mesh.nodes.len()], q: 0.0, pts: vec![Point::default(); self.gps.len()], lam: vec![0.0; ng], lam_t: vec![0.0; ng], phi: self.cps.iter().map(|c| c.x0[1].atan2(c.x0[0])).collect(), prev: None, s: 0.0 };
        let mut stats = (0usize, 0usize);
        let mut chord: Option<Cholesky> = None;
        let mut curve: Vec<FsPoint> = Vec::new();
        let mut note = None;
        let (mut s, s_cap) = (0.0, clearance + opts.travel_cap_over_a * a);
        // Interference: settle the fit at the start.
        if opts.pin_dia / 2.0 > a {
            let (n, _) = self.step(&st, 0.0, dir, &opts, false, &mut stats, &mut chord)?;
            st = n;
        }
        let mut step = if clearance > 0.0 { (0.5 * clearance).max(0.005 * a) } else { 0.01 * a };
        let (mut retries, mut strain_limited, mut peak_reached) = (0, false, false);
        let mut best = 0.0f64;
        let mut max_ep = 0.0f64;
        while s < s_cap - 1e-12 {
            let s_try = (s + step).min(s_cap);
            let quasi = curve.len() >= 5 && retries == 0;
            let mut outcome = self.step(&st, s_try, dir, &opts, quasi, &mut stats, &mut chord);
            if outcome.is_err() && quasi {
                // The correction memory misled the solve: plain Newton at the same step before halving it.
                chord = None;
                outcome = self.step(&st, s_try, dir, &opts, false, &mut stats, &mut chord);
            }
            match outcome {
                Err(e) => {
                    retries += 1;
                    step *= 0.5;
                    chord = None;
                    if retries > 8 || step < 1e-4 * a {
                        note = Some(format!("stopped at {:.4} in of pin travel: {e}", (s - clearance).max(0.0)));
                        break;
                    }
                    continue;
                }
                Ok((next, ev)) => {
                    retries = 0;
                    let load = share * (ev.force[0] * dir[0] + ev.force[1] * dir[1]) * t;
                    let point = FsPoint { travel: (s_try - clearance).max(0.0), load_lbf: load, max_ep: ev.max_ep };
                    max_ep = max_ep.max(ev.max_ep);
                    if let (Some(limit), Some(prev)) = (opts.strain_limit, curve.last().copied()) {
                        if point.max_ep >= limit {
                            let f = ((limit - prev.max_ep) / (point.max_ep - prev.max_ep).max(1e-300)).clamp(0.0, 1.0);
                            curve.push(FsPoint { travel: prev.travel + f * (point.travel - prev.travel), load_lbf: prev.load_lbf + f * (point.load_lbf - prev.load_lbf), max_ep: limit });
                            strain_limited = true;
                            break;
                        }
                    }
                    curve.push(point);
                    st = next;
                    s = s_try;
                    best = best.max(load);
                    step = (step * 1.25).min(0.04 * a);
                    // Past its peak and falling: the collapse load is the peak.
                    let n = curve.len();
                    if n >= 4 && max_ep > 1e-4 && best > 0.0 && curve[n - 1].load_lbf < 0.97 * best && curve[n - 2].load_lbf < 0.99 * best {
                        peak_reached = true;
                        break;
                    }
                }
            }
        }
        let collapse = curve.iter().map(|p| p.load_lbf).fold(0.0, f64::max);
        if collapse <= 0.0 {
            return Err(note.unwrap_or_else(|| "no load was carried".into()));
        }
        if !peak_reached && !strain_limited && note.is_none() {
            note = Some("the pin travel cap ended the run before the load peaked or the failure strain was reached: a lower bound".into());
        }
        Ok(FsResult { curve, collapse_lbf: collapse, peak_reached, strain_limited, max_ep, newton_iterations: stats.0, factorisations: stats.1, elapsed_ms: t0.elapsed().as_secs_f64() * 1e3, note })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn steel() -> FsMaterial {
        FsMaterial { e_psi: 29.0e6, nu: 0.3, law: Hardening::true_curve(60_000.0, 90_000.0, 0.15, 1.0).unwrap() }
    }

    fn rot(a: f64) -> M2 {
        [[a.cos(), -a.sin()], [a.sin(), a.cos()]]
    }

    #[test]
    fn the_true_curve_is_continuous_monotone_and_reaches_the_true_ultimate_at_the_uniform_strain() {
        let h = Hardening::true_curve(60_000.0, 90_000.0, 0.15, 1.0).unwrap();
        assert!(h.initial() > 0.5 * 60_000.0 && h.initial() < 60_000.0 * 1.01);
        let sigma_u = 90_000.0 * 1.15;
        let n = (1.15f64).ln();
        assert!((h.stress(n) / sigma_u - 1.0).abs() < 0.03, "{} vs {sigma_u}", h.stress(n));
        let mut last = 0.0;
        for i in 0..=100 {
            let s = h.stress(i as f64 * 0.01);
            assert!(s >= last - 1e-9);
            last = s;
        }
        assert!(h.stress(1.0) > sigma_u);
    }

    #[test]
    fn a_stretch_below_yield_is_elastic_with_the_small_strain_stress() {
        let m = steel();
        let f = [[1.0 + 5e-4, 0.0], [0.0, 1.0]];
        let (pt, p) = m.update(&Point::default(), &f);
        assert_eq!(pt.ep, 0.0);
        // Plane strain uniaxial strain: sigma_xx = (lambda + 2 mu) eps.
        let (lam, mu) = (m.e_psi * m.nu / ((1.0 + m.nu) * (1.0 - 2.0 * m.nu)), m.shear());
        assert!((p[0][0] / ((lam + 2.0 * mu) * 5e-4) - 1.0).abs() < 2e-3, "{}", p[0][0]);
    }

    #[test]
    fn the_response_is_objective_a_rigid_rotation_rotates_the_stress_and_changes_nothing_else() {
        let m = steel();
        let f0 = [[1.04, 0.03], [0.0, 0.97]];
        let (s0, p0) = m.update(&Point::default(), &f0);
        let r = rot(0.7);
        let (s1, p1) = m.update(&Point::default(), &mul(&r, &f0));
        assert!((s0.ep - s1.ep).abs() < 1e-12 && s0.ep > 0.0);
        // P = R P0 for F = R F0.
        let rp = mul(&r, &p0);
        for i in 0..2 {
            for j in 0..2 {
                assert!((p1[i][j] - rp[i][j]).abs() < 1e-6 * p0[0][0].abs().max(1.0), "{p1:?} vs {rp:?}");
            }
        }
    }

    #[test]
    fn ideal_plastic_simple_shear_saturates_at_sigma_y_over_root_three() {
        let flat = FsMaterial { law: Hardening::linear(50_000.0, 1.0, 5.0).unwrap(), ..steel() };
        let mut pt = Point::default();
        let mut tau = 0.0;
        // Shear in many small increments (the state carries the history).
        for k in 1..=200 {
            let gamma = 0.01 * k as f64;
            let f = [[1.0, gamma], [0.0, 1.0]];
            let (n, p) = flat.update(&pt, &f);
            pt = n;
            tau = p[0][1];
        }
        // Kirchhoff shear in the current frame; for J = 1 it equals the Cauchy shear stress.
        assert!((tau / (50_000.0 / 3f64.sqrt()) - 1.0).abs() < 0.08, "{tau}");
        assert!(pt.ep > 0.5);
    }

    #[test]
    fn the_numerical_tangent_matches_the_elastic_modulus_and_is_symmetric() {
        let m = steel();
        let f = [[1.0 + 1e-4, 0.0], [0.0, 1.0]];
        let (_, p) = m.update(&Point::default(), &f);
        let a = m.tangent(&Point::default(), &f, &p);
        let (lam, mu) = (m.e_psi * m.nu / ((1.0 + m.nu) * (1.0 - 2.0 * m.nu)), m.shear());
        assert!((a[0][0] / (lam + 2.0 * mu) - 1.0).abs() < 1e-2, "{}", a[0][0]);
        for i in 0..4 {
            for j in 0..4 {
                assert!((a[i][j] - a[j][i]).abs() < 1e-6 * (lam + 2.0 * mu));
            }
        }
    }

    #[test]
    fn a_plastic_tangent_is_softer_than_the_elastic_one() {
        let m = steel();
        let f = [[1.02, 0.0], [0.0, 1.0]];
        let (pt, p) = m.update(&Point::default(), &f);
        assert!(pt.ep > 0.0);
        let a = m.tangent(&Point::default(), &f, &p);
        let (lam, mu) = (m.e_psi * m.nu / ((1.0 + m.nu) * (1.0 - 2.0 * m.nu)), m.shear());
        // Uniaxial strain: the deviatoric stiffness collapses to the hardening slope, the bulk part stays.
        assert!(a[0][0] < 0.75 * (lam + 2.0 * mu) && a[0][0] > 0.9 * m.bulk(), "{} (bulk {})", a[0][0], m.bulk());
    }
}
