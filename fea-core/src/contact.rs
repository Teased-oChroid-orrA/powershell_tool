//! General contact: Gauss-point-to-surface with a rigid analytic master or a deformable one.
//!
//! * **Slave side**: Gauss points of listed boundary faces (edges in 2D) integrated over the
//!   *reference* surface (weight = reference measure x thickness or `2 pi r`), at the current
//!   position `y = sum N_a (X_a + u_a)`. This is the quadrature the validated lug solver uses.
//! * **Master side**: a rigid analytic surface (circle, sphere, plane, cylinder; from the outside or
//!   the inside, with a translation `shift + lambda * travel`) or a set of deformable master faces:
//!   the slave point is projected onto the face by Newton on its (quadratic) shape functions, so the
//!   gap `g = (y - x_c) . n` and the normal come from the exact surface.
//! * **Normal**: augmented Lagrangian, pressure `p = max(0, lam_n - eps_n g)`; the multiplier is
//!   updated between Newton solves (`update_multipliers`).
//! * **Friction**: Coulomb with an augmented tangential traction `lam_t + eps_t s` (`s` the tangential
//!   slip of the slave point over the master material point it touched at the last committed state),
//!   capped at `mu p`.
//! * **Equilibrium and tangent**: a contact point applies the force pair `-+(p n + Lambda)` at the
//!   slave point and (distributed by the master shape functions) at the contact point of the master.
//!   This residual is evaluated in closed form per point and its **tangent by central differences of
//!   that local residual** with respect to its few variables (slave position, master nodal positions):
//!   smooth, exact to rounding, consistent including curvature and the projection.

#![allow(clippy::needless_range_loop)] // index-parallel numeric kernels read clearer as loops

use crate::assembly::{BlockMatrix, Pattern};
use crate::element::ElementKind;
use crate::mesh::{Mesh, Physics};
use rayon::prelude::*;

pub type V3 = [f64; 3];

fn dot(a: &V3, b: &V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn sub(a: &V3, b: &V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn add(a: &V3, b: &V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn scale(a: &V3, s: f64) -> V3 {
    [a[0] * s, a[1] * s, a[2] * s]
}

fn cross(a: &V3, b: &V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn norm(a: &V3) -> f64 {
    dot(a, a).sqrt()
}

// ------------------------------------------------------------------ rigid masters

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RigidShape {
    /// 2D circle (the slave surface lies outside unless [`RigidMaster::inside`]).
    Circle { c: [f64; 2], r: f64 },
    Sphere { c: V3, r: f64 },
    /// Plane (a line in 2D) through `p` with unit normal `n` pointing to the slave side.
    Plane { p: V3, n: V3 },
    /// Infinite cylinder about the axis through `p` with unit direction `a`.
    Cylinder { p: V3, a: V3, r: f64 },
}

/// A rigid analytic master with a prescribed translation `shift + lambda * travel`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RigidMaster {
    pub shape: RigidShape,
    /// The slave lies inside the circle / sphere / cylinder (a rigid bore around a shaft).
    pub inside: bool,
    pub shift: V3,
    /// Translation per unit load factor (pin travel).
    pub travel: V3,
    /// Translation components that are unknowns of the analysis (a free rigid body): each is in
    /// equilibrium between the contact force and the applied force `load * lambda`.
    pub free: [bool; 3],
    /// Force applied to the rigid body per unit load factor (used for its free components).
    pub load: V3,
}

impl RigidMaster {
    pub fn new(shape: RigidShape) -> Self {
        Self { shape, inside: false, shift: [0.0; 3], travel: [0.0; 3], free: [false; 3], load: [0.0; 3] }
    }

    /// Let the body translate freely along the `free` axes under the applied force `load` (per unit
    /// load factor): a force-controlled pin.
    pub fn with_free_translation(mut self, free: [bool; 3], load: V3) -> Self {
        self.free = free;
        self.load = load;
        self
    }

    /// Let a 2D circular master rotate about its own centre under the applied `moment` (per unit load factor): the third
    /// extra-unknown slot of a 2D master is its rotation (radians). The shape does not change under the rotation, so it
    /// only acts through friction (the slip of the slave over the turning surface and the torque it transmits).
    pub fn with_free_rotation(mut self, moment: f64) -> Self {
        self.free[2] = true;
        self.load[2] = moment;
        self
    }

    pub fn with_travel(mut self, travel: V3) -> Self {
        self.travel = travel;
        self
    }

    pub fn translation(&self, lambda: f64) -> V3 {
        add(&self.shift, &scale(&self.travel, lambda))
    }

    /// `(gap, outward normal, contact point relative to the body)` for the slave point `y`.
    pub fn gap(&self, y: &V3, lambda: f64, dim: usize) -> (f64, V3, V3) {
        self.gap_q(y, lambda, &[0.0; 3], dim)
    }

    /// As [`gap`](Self::gap) with the free translation `q` added to the prescribed one.
    pub fn gap_q(&self, y: &V3, lambda: f64, q: &V3, dim: usize) -> (f64, V3, V3) {
        // In 2D the third free component is the rotation about the circle's centre (see `with_free_rotation`): the
        // slave point is brought into the body's frame, and the contact point comes back in it (the slip history lives
        // there); the normal is returned in the world frame.
        let theta = if dim == 2 { q[2] } else { 0.0 };
        let q = if dim == 2 { [q[0], q[1], 0.0] } else { *q };
        let mut q = sub(y, &add(&self.translation(lambda), &q));
        if theta != 0.0 {
            if let RigidShape::Circle { c, .. } = self.shape {
                let (s, co) = (-theta).sin_cos();
                let d = [q[0] - c[0], q[1] - c[1]];
                q = [c[0] + co * d[0] - s * d[1], c[1] + s * d[0] + co * d[1], 0.0];
            }
        }
        let (g, n) = match self.shape {
            RigidShape::Circle { c, r } => {
                let d = [q[0] - c[0], q[1] - c[1], 0.0];
                let rho = norm(&d).max(1e-300);
                (rho - r, scale(&d, 1.0 / rho))
            }
            RigidShape::Sphere { c, r } => {
                let d = sub(&q, &c);
                let rho = norm(&d).max(1e-300);
                (rho - r, scale(&d, 1.0 / rho))
            }
            RigidShape::Plane { p, n } => (dot(&sub(&q, &p), &n), n),
            RigidShape::Cylinder { p, a, r } => {
                let w = sub(&q, &p);
                let wp = sub(&w, &scale(&a, dot(&w, &a)));
                let rho = norm(&wp).max(1e-300);
                (rho - r, scale(&wp, 1.0 / rho))
            }
        };
        let (g, n) = match self.shape {
            RigidShape::Plane { .. } => (g, n),
            _ if self.inside => (-g, scale(&n, -1.0)),
            _ => (g, n),
        };
        let _ = dim;
        let xc = sub(&q, &scale(&n, g));
        // The normal in the world frame (the frame of the body rotated by `theta`).
        let n = if theta != 0.0 { let (s, co) = theta.sin_cos(); [co * n[0] - s * n[1], s * n[0] + co * n[1], n[2]] } else { n };
        (g, n, xc)
    }
}

// ------------------------------------------------------------------ master faces

/// Parametrisation of a master face: a 2- or 3-node edge (2D) or a triangle / quadrilateral (3D).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FaceGeom {
    pub nn: usize,
    /// Parametric dimension (1 for edges, 2 for faces).
    pub p: usize,
    kind: Option<ElementKind>,
}

impl FaceGeom {
    pub fn new(dim: usize, nn: usize) -> Result<Self, String> {
        if dim == 2 {
            if nn == 2 || nn == 3 {
                return Ok(Self { nn, p: 1, kind: None });
            }
            return Err(format!("a 2D contact edge needs 2 or 3 nodes, not {nn}"));
        }
        let kind = match nn {
            3 => ElementKind::Tri3,
            6 => ElementKind::Tri6,
            4 => ElementKind::Quad4,
            8 => ElementKind::Quad8,
            9 => ElementKind::Quad9,
            n => return Err(format!("a contact face with {n} nodes is not supported")),
        };
        Ok(Self { nn, p: 2, kind: Some(kind) })
    }

    /// Shape functions and first derivatives at `xi`.
    pub fn shape(&self, xi: [f64; 2]) -> ([f64; 9], [[f64; 2]; 9]) {
        let mut n = [0.0; 9];
        let mut dn = [[0.0; 2]; 9];
        match self.kind {
            None => {
                let x = xi[0];
                if self.nn == 2 {
                    n[0] = 0.5 * (1.0 - x);
                    n[1] = 0.5 * (1.0 + x);
                    dn[0][0] = -0.5;
                    dn[1][0] = 0.5;
                } else {
                    n[0] = 0.5 * x * (x - 1.0);
                    n[1] = 0.5 * x * (x + 1.0);
                    n[2] = 1.0 - x * x;
                    dn[0][0] = x - 0.5;
                    dn[1][0] = x + 0.5;
                    dn[2][0] = -2.0 * x;
                }
            }
            Some(k) => {
                let (nn, d) = k.shape([xi[0], xi[1], 0.0]);
                for a in 0..self.nn {
                    n[a] = nn[a];
                    dn[a] = [d[a][0], d[a][1]];
                }
            }
        }
        (n, dn)
    }

    /// Second derivatives `d2[a][i][j]` by central differences of the (exact) first derivatives.
    pub fn second(&self, xi: [f64; 2]) -> [[[f64; 2]; 2]; 9] {
        let mut out = [[[0.0; 2]; 2]; 9];
        // An edge: the second derivatives of the (linear / quadratic) shape functions are constants.
        if self.kind.is_none() {
            if self.nn == 3 {
                out[0][0][0] = 1.0;
                out[1][0][0] = 1.0;
                out[2][0][0] = -2.0;
            }
            return out;
        }
        let h = 1e-5;
        for j in 0..self.p {
            let (mut xp, mut xm) = (xi, xi);
            xp[j] += h;
            xm[j] -= h;
            let ((_, dp), (_, dm)) = (self.shape(xp), self.shape(xm));
            for a in 0..self.nn {
                for i in 0..self.p {
                    out[a][i][j] = (dp[a][i] - dm[a][i]) / (2.0 * h);
                }
            }
        }
        out
    }

    /// Parametric centre.
    pub fn centre(&self) -> [f64; 2] {
        match self.kind {
            Some(k) if k.is_simplex() => [1.0 / 3.0, 1.0 / 3.0],
            _ => [0.0, 0.0],
        }
    }

    /// Whether `xi` lies within the face domain extended by `tol`.
    pub fn contains(&self, xi: [f64; 2], tol: f64) -> bool {
        match self.kind {
            None => xi[0].abs() <= 1.0 + tol,
            Some(k) if k.is_simplex() => xi[0] >= -tol && xi[1] >= -tol && xi[0] + xi[1] <= 1.0 + tol,
            _ => xi[0].abs() <= 1.0 + tol && xi[1].abs() <= 1.0 + tol,
        }
    }
}

/// Closest-point projection of a point onto a master face.
#[derive(Debug, Clone, Copy)]
pub struct Projection {
    pub xi: [f64; 2],
    pub xc: V3,
    pub n: V3,
    pub g: f64,
}

/// Position, tangents `tau[alpha] = d x / d xi_alpha` of the face at `xi`.
fn face_point(geom: &FaceGeom, x: &[V3], xi: [f64; 2]) -> (V3, [V3; 2], [f64; 9]) {
    let (n, dn) = geom.shape(xi);
    let mut xc = [0.0; 3];
    let mut tau = [[0.0; 3]; 2];
    for a in 0..geom.nn {
        for i in 0..3 {
            xc[i] += n[a] * x[a][i];
            for al in 0..geom.p {
                tau[al][i] += dn[a][al] * x[a][i];
            }
        }
    }
    (xc, tau, n)
}

/// Outward normal of the face from its tangents (2D: `(ty, -tx)`; 3D: `tau1 x tau2`).
fn face_normal(p: usize, tau: &[V3; 2]) -> V3 {
    if p == 1 {
        let l = (tau[0][0] * tau[0][0] + tau[0][1] * tau[0][1]).sqrt().max(1e-300);
        [tau[0][1] / l, -tau[0][0] / l, 0.0]
    } else {
        let c = cross(&tau[0], &tau[1]);
        scale(&c, 1.0 / norm(&c).max(1e-300))
    }
}

/// Project `y` onto the face (Newton on `(y - x_c) . tau_alpha = 0`) starting from `xi0`.
pub fn project(geom: &FaceGeom, x: &[V3], y: &V3, xi0: [f64; 2]) -> Option<Projection> {
    let p = geom.p;
    let mut xi = xi0;
    for _ in 0..40 {
        let (xc, tau, _) = face_point(geom, x, xi);
        let r = sub(y, &xc);
        let d2 = geom.second(xi);
        let mut f = [0.0; 2];
        let mut j = [[0.0; 2]; 2];
        for a in 0..p {
            f[a] = dot(&r, &tau[a]);
            for b in 0..p {
                let mut dtau = [0.0; 3];
                for k in 0..geom.nn {
                    for i in 0..3 {
                        dtau[i] += d2[k][a][b] * x[k][i];
                    }
                }
                j[a][b] = -dot(&tau[a], &tau[b]) + dot(&r, &dtau);
            }
        }
        let dxi = if p == 1 {
            if j[0][0].abs() < 1e-300 {
                return None;
            }
            [-f[0] / j[0][0], 0.0]
        } else {
            let det = j[0][0] * j[1][1] - j[0][1] * j[1][0];
            if det.abs() < 1e-300 {
                return None;
            }
            [-(j[1][1] * f[0] - j[0][1] * f[1]) / det, -(-j[1][0] * f[0] + j[0][0] * f[1]) / det]
        };
        // Damp wild steps (a point far from the face).
        let m = dxi[0].abs().max(dxi[1].abs());
        let s = if m > 0.5 { 0.5 / m } else { 1.0 };
        xi[0] += s * dxi[0];
        xi[1] += s * dxi[1];
        // Converged when the step is below the round-off of the geometry: `r = y - x_c` carries an absolute error of order
        // `eps |x|`, which the tangent turns into `eps |x| / |tau|` in the parameter. A fixed 1e-14 is below that as soon as
        // the coordinates are of order one (the projection then never converged and the point silently left contact).
        let reach = (0..3).map(|i| y[i].abs().max(xc[i].abs())).fold(0.0f64, f64::max);
        let tau_len = (0..p).map(|a| norm(&tau[a])).fold(0.0f64, f64::max).max(1e-300);
        let tol = (1e-14f64).max(64.0 * f64::EPSILON * reach / tau_len);
        if m * s < tol {
            let (xc, tau, _) = face_point(geom, x, xi);
            let n = face_normal(p, &tau);
            return Some(Projection { xi, xc, n, g: dot(&sub(y, &xc), &n) });
        }
    }
    None
}

// ------------------------------------------------------------------ specification and state

#[derive(Debug, Clone)]
pub enum Master {
    Rigid(RigidMaster),
    /// Deformable master faces (node lists in load orientation: outward normal, body on the left in 2D).
    Faces(Vec<Vec<usize>>),
}

/// Where the contact constraint is collocated on a slave face.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ContactRule {
    /// The Gauss rule of the face (3 points per quadratic edge, 3 x 3 on a quadrilateral).
    Gauss,
    /// Two Gauss points per edge / 2 x 2 on a quadrilateral: avoids most of the oscillation of full
    /// Gauss collocation in 2D, but still shows a checkerboard in 3D.
    Reduced,
    /// The face nodes with Simpson-type weights (nodal collocation): as many constraints as there
    /// are independent surface nodes, smooth pressure in 2D and 3D (Hertz validated) against a rigid
    /// master.
    Nodal,
    /// Nodal against a rigid master, reduced Gauss against a deformable one (nodal collocation on a
    /// deformable master fails the contact patch test; Gauss-based integration keeps it consistent).
    #[default]
    Auto,
}

/// One contact interface.
#[derive(Debug, Clone)]
pub struct ContactSpec {
    pub name: String,
    /// Slave faces (edges in 2D) of the mesh.
    pub slave: Vec<Vec<usize>>,
    pub master: Master,
    /// Normal penalty (pressure per unit gap).
    pub eps_n: f64,
    /// Tangential penalty (traction per unit slip).
    pub eps_t: f64,
    /// Coulomb friction coefficient (0 = frictionless).
    pub mu: f64,
    /// Deformable masters: slave points farther than this from a master face are not in contact, and
    /// slave / master faces closer than this at the start are coupled in the matrix pattern. `None`
    /// takes one slave-face diameter.
    pub margin: Option<f64>,
    /// Collocation points on the slave faces.
    pub rule: ContactRule,
    /// Points not yet in contact but closer than this to the master add their normal penalty
    /// stiffness to the tangent (no force), so a model that only touches at the start has a
    /// nonsingular first iteration. `None` takes one thousandth of the margin.
    pub activation_gap: Option<f64>,
    /// Deformable masters: the master surface is taken to overlap the slave surface by this much
    /// (an interference fit: the gap is `projected gap - overlap`).
    pub overlap: f64,
}

impl ContactSpec {
    /// The margin in force: the given one, else the largest slave-face extent.
    pub fn effective_margin(&self, mesh: &Mesh) -> f64 {
        self.margin.unwrap_or_else(|| {
            let diam = self
                .slave
                .iter()
                .map(|f| {
                    let mut m = 0.0f64;
                    for a in f {
                        for b in f {
                            m = m.max(norm(&sub(&mesh.nodes[*a], &mesh.nodes[*b])));
                        }
                    }
                    m
                })
                .fold(0.0f64, f64::max);
            diam.max(1e-12)
        })
    }

    pub fn rigid(name: &str, slave: Vec<Vec<usize>>, master: RigidMaster, eps_n: f64) -> Self {
        Self { name: name.to_string(), slave, master: Master::Rigid(master), eps_n, eps_t: eps_n, mu: 0.0, margin: None, rule: ContactRule::default(), activation_gap: None, overlap: 0.0 }
    }

    pub fn deformable(name: &str, slave: Vec<Vec<usize>>, master: Vec<Vec<usize>>, eps_n: f64) -> Self {
        Self { name: name.to_string(), slave, master: Master::Faces(master), eps_n, eps_t: eps_n, mu: 0.0, margin: None, rule: ContactRule::default(), activation_gap: None, overlap: 0.0 }
    }

    /// Symmetric two-pass contact between surfaces `a` and `b`: each side acts once as the slave
    /// (with half the penalty each), so neither mesh is privileged. The contact pressure is then
    /// shared between the two interfaces (their sum is the physical pressure).
    pub fn two_pass(name: &str, a: Vec<Vec<usize>>, b: Vec<Vec<usize>>, eps_n: f64) -> [ContactSpec; 2] {
        [Self::deformable(&format!("{name}/a"), a.clone(), b.clone(), 0.5 * eps_n), Self::deformable(&format!("{name}/b"), b, a, 0.5 * eps_n)]
    }

    pub fn with_friction(mut self, mu: f64, eps_t: f64) -> Self {
        self.mu = mu;
        self.eps_t = eps_t;
        self
    }

    pub fn with_rule(mut self, rule: ContactRule) -> Self {
        self.rule = rule;
        self
    }

    /// An interference fit: the master overlaps the slave surface by `overlap` before any load.
    pub fn with_overlap(mut self, overlap: f64) -> Self {
        self.overlap = overlap;
        self
    }

    pub fn with_margin(mut self, margin: f64) -> Self {
        self.margin = Some(margin);
        self
    }
}

/// The traction of one slave collocation point of a contact interface, as read from a converged solution.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ContactTraction {
    /// Reference position of the point.
    pub x: V3,
    /// Unit contact normal (from the master towards the slave).
    pub normal: V3,
    /// Normal pressure (0 when not in contact).
    pub pressure: f64,
    /// Friction traction on the slave surface, in the tangent plane.
    pub friction: V3,
    /// Integration weight: pressure times weight is the normal force the point carries (thickness included).
    pub weight: f64,
    pub active: bool,
    /// The point is at its Coulomb limit (`|friction| >= mu p`).
    pub slipping: bool,
}

/// State of one slave Gauss point.
#[derive(Debug, Clone, Copy, Default)]
pub struct CpState {
    /// Normal multiplier (committed pressure of the previous multiplier pass).
    pub lam_n: f64,
    /// Tangential multiplier (friction traction of the previous pass).
    pub lam_t: V3,
    /// Contact point at the last committed state: rigid master: relative to the body; deformable:
    /// the master parametric coordinates (and `face`).
    pub hist: V3,
    pub face: u32,
    pub has_hist: bool,
    // Trial results of the latest evaluation.
    pub p: f64,
    pub fric: V3,
    pub g: f64,
    pub active: bool,
    pub cur: V3,
    pub cur_face: u32,
    /// Unit contact normal of the latest evaluation (pointing from the master towards the slave).
    pub n: V3,
}

/// Per spec, per slave point: whether it is held in contact and the master face it is held on (`ContactSet::hold_active_set`).
type HeldPoints = Vec<Vec<(bool, u32)>>;

/// Parametric tolerance within which a slave point still counts as lying over its committed master face.
const STICKY_TOL: f64 = 1e-6;

struct SlavePoint {
    nodes: Vec<usize>,
    n: [f64; 9],
    w: f64,
    /// Position in the reference configuration.
    x0: V3,
}

/// What an evaluation reports.
#[derive(Debug, Clone, Default)]
pub struct ContactStats {
    /// Smallest gap among active points (negative = penetration).
    pub min_gap: f64,
    pub n_active: usize,
    /// Total force on the master of each spec (rigid: the pin load).
    pub master_force: Vec<V3>,
    /// Slave / master node couplings found missing from the matrix pattern.
    pub missing_pairs: Vec<(usize, usize)>,
    /// The unsymmetric remainder of the frictional tangent as `(dofs, row-major matrix)` per point: the exact tangent is
    /// `K + sum E^T D E` (see `Ctx::newton_step`). Empty without friction (or with free rigid translations).
    pub defect: Vec<(Vec<usize>, Vec<f64>)>,
}

/// Outputs for the free rigid-body translations: the residual of each extra unknown, its coupling
/// with the mesh dofs (`kuq[idx * n_dofs + dof]`) and with itself (`kqq[i * m + j]`).
#[derive(Debug, Clone, Default)]
pub struct ExtraOut {
    pub rq: Vec<f64>,
    pub kuq: Vec<f64>,
    pub kqq: Vec<f64>,
}

impl ExtraOut {
    pub fn new(n_dofs: usize, m: usize) -> Self {
        Self { rq: vec![0.0; m], kuq: vec![0.0; n_dofs * m], kqq: vec![0.0; m * m] }
    }
}

pub struct ContactSet {
    pub specs: Vec<ContactSpec>,
    dim: usize,
    /// Per spec: the free rigid translation components as `(component, extra index)`.
    extra: Vec<Vec<(usize, usize)>>,
    /// Applied force per unit load factor of every extra unknown.
    extra_load: Vec<f64>,
    pts: Vec<Vec<SlavePoint>>,
    faces: Vec<Vec<(FaceGeom, Vec<usize>)>>,
    margins: Vec<f64>,
    /// Add the stabilising stiffness of merely-near points (see `eval`); off for the tangent predictor of prescribed
    /// displacements, which would drag a body that is only approaching along with the one it moves.
    stabilise: std::sync::atomic::AtomicBool,
    /// A held active set (`hold_active_set`): per spec, per slave point, whether the point counts as in contact. Set by a
    /// Newton solve whose iteration chatters at the edge of a contact patch (points of ~zero pressure entering and leaving),
    /// released when that solve ends.
    hold: std::sync::RwLock<Option<HeldPoints>>,
}

/// Collocation points `(xi, weight)` on a slave face of `nn` nodes in `dim` dimensions.
fn collocation(dim: usize, nn: usize, rule: ContactRule) -> Vec<([f64; 2], f64)> {
    let g2 = 1.0 / 3.0f64.sqrt();
    if dim == 2 {
        return match (rule, nn) {
            (ContactRule::Auto, _) => unreachable!("resolved before collocation"),
            (ContactRule::Gauss, _) => GL3.iter().map(|&(x, w)| ([x, 0.0], w)).collect(),
            (ContactRule::Reduced, _) => vec![([-g2, 0.0], 1.0), ([g2, 0.0], 1.0)],
            // Trapezoid on a straight edge, Simpson on a quadratic one.
            (ContactRule::Nodal, 2) => vec![([-1.0, 0.0], 1.0), ([1.0, 0.0], 1.0)],
            (ContactRule::Nodal, _) => vec![([-1.0, 0.0], 1.0 / 3.0), ([0.0, 0.0], 4.0 / 3.0), ([1.0, 0.0], 1.0 / 3.0)],
        };
    }
    let kind = match nn {
        3 => ElementKind::Tri3,
        6 => ElementKind::Tri6,
        4 => ElementKind::Quad4,
        8 => ElementKind::Quad8,
        _ => ElementKind::Quad9,
    };
    let simplex = kind.is_simplex();
    match rule {
        ContactRule::Auto => unreachable!("resolved before collocation"),
        ContactRule::Gauss => {
            let t = kind.table();
            (0..t.ngp).map(|g| ([t.xi[g][0], t.xi[g][1]], t.w[g])).collect()
        }
        ContactRule::Reduced if !simplex => [(-g2, -g2), (g2, -g2), (g2, g2), (-g2, g2)].iter().map(|&(a, b)| ([a, b], 1.0)).collect(),
        ContactRule::Reduced => {
            // Three-point interior rule on the triangle (exact to degree 2).
            vec![([1.0 / 6.0, 1.0 / 6.0], 1.0 / 6.0), ([2.0 / 3.0, 1.0 / 6.0], 1.0 / 6.0), ([1.0 / 6.0, 2.0 / 3.0], 1.0 / 6.0)]
        }
        ContactRule::Nodal => {
            let w = [1.0 / 3.0, 4.0 / 3.0, 1.0 / 3.0];
            let x = [-1.0, 0.0, 1.0];
            match kind {
                ElementKind::Quad4 => [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)].iter().map(|&(a, b)| ([a, b], 1.0)).collect(),
                ElementKind::Tri3 => [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0)].iter().map(|&(a, b)| ([a, b], 1.0 / 6.0)).collect(),
                // Quadratic triangle: the edge midpoints carry the whole area (exact for quadratics).
                ElementKind::Tri6 => [(0.5, 0.0), (0.5, 0.5), (0.0, 0.5)].iter().map(|&(a, b)| ([a, b], 1.0 / 6.0)).collect(),
                _ => (0..3).flat_map(|i| (0..3).map(move |j| ([x[i], x[j]], w[i] * w[j]))).collect(),
            }
        }
    }
}

const GL3: [(f64, f64); 3] = [(-0.774_596_669_241_483_4, 5.0 / 9.0), (0.0, 8.0 / 9.0), (0.774_596_669_241_483_4, 5.0 / 9.0)];

fn slave_points(mesh: &Mesh, faces: &[Vec<usize>], rule: ContactRule) -> Result<Vec<SlavePoint>, String> {
    let d = mesh.dim();
    let scale_of = |xr: f64| -> f64 {
        match mesh.physics {
            Physics::PlaneStress { thickness } | Physics::PlaneStrain { thickness } => thickness,
            Physics::Axisymmetric => 2.0 * std::f64::consts::PI * xr,
            Physics::Solid => 1.0,
        }
    };
    let mut out = Vec::new();
    for nodes in faces {
        if let Some(&bad) = nodes.iter().find(|&&n| n >= mesh.nodes.len()) {
            return Err(format!("contact face refers to missing node {bad}"));
        }
        let geom = FaceGeom::new(d, nodes.len())?;
        let x0: Vec<V3> = nodes.iter().map(|&n| mesh.nodes[n]).collect();
        let gp: Vec<([f64; 2], f64)> = collocation(d, nodes.len(), rule);
        for (xi, w) in gp {
            let (xc, tau, n) = face_point(&geom, &x0, xi);
            let measure = if d == 2 { norm(&tau[0]) } else { norm(&cross(&tau[0], &tau[1])) };
            if measure <= 0.0 {
                return Err("degenerate contact face".into());
            }
            out.push(SlavePoint { nodes: nodes.clone(), n, w: w * measure * scale_of(xc[0]), x0: xc });
        }
    }
    Ok(out)
}

impl ContactSet {
    /// Switch the stabilising stiffness of near points on or off for the following evaluations.
    pub fn set_stabilise(&self, on: bool) {
        self.stabilise.store(on, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn new(mesh: &Mesh, specs: Vec<ContactSpec>) -> Result<Self, String> {
        let dim = mesh.dim();
        let mut pts = Vec::new();
        let mut faces = Vec::new();
        let mut margins = Vec::new();
        for s in &specs {
            if s.eps_n.is_nan() || s.eps_n <= 0.0 {
                return Err(format!("contact '{}': the normal penalty must be positive", s.name));
            }
            if s.mu < 0.0 || (s.mu > 0.0 && (s.eps_t.is_nan() || s.eps_t <= 0.0)) {
                return Err(format!("contact '{}': friction needs mu >= 0 and a positive tangential penalty", s.name));
            }
            let rule = match (s.rule, &s.master) {
                (ContactRule::Auto, Master::Rigid(_)) => ContactRule::Nodal,
                (ContactRule::Auto, Master::Faces(_)) => ContactRule::Reduced,
                (r, _) => r,
            };
            pts.push(slave_points(mesh, &s.slave, rule)?);
            let mut mf = Vec::new();
            if let Master::Faces(f) = &s.master {
                for nodes in f {
                    if let Some(&bad) = nodes.iter().find(|&&n| n >= mesh.nodes.len()) {
                        return Err(format!("contact '{}': master face refers to missing node {bad}", s.name));
                    }
                    mf.push((FaceGeom::new(dim, nodes.len())?, nodes.clone()));
                }
            }
            faces.push(mf);
            margins.push(s.effective_margin(mesh));
        }
        let mut extra = Vec::new();
        let mut extra_load = Vec::new();
        for s in &specs {
            let mut e = Vec::new();
            if let Master::Rigid(r) = &s.master {
                for c in 0..dim {
                    if r.free[c] {
                        e.push((c, extra_load.len()));
                        extra_load.push(r.load[c]);
                    }
                }
                if r.free[2] && dim == 2 {
                    // The rotation of a circle about its centre (see `RigidMaster::with_free_rotation`).
                    if !matches!(r.shape, RigidShape::Circle { .. }) {
                        return Err(format!("contact '{}': only a circular 2D master can rotate", s.name));
                    }
                    e.push((2, extra_load.len()));
                    extra_load.push(r.load[2]);
                }
            }
            extra.push(e);
        }
        Ok(Self { specs, dim, extra, extra_load, pts, faces, margins, stabilise: std::sync::atomic::AtomicBool::new(true), hold: std::sync::RwLock::new(None) })
    }

    /// Gap below which a point counts as touching for the slip history and the stabilising stiffness.
    fn activation_gap(&self, spec: usize) -> f64 {
        self.specs[spec].activation_gap.unwrap_or(1e-3 * self.margins[spec])
    }

    /// Initial state: zero multipliers, with the slip history seeded for points that touch the
    /// master at the undeformed configuration (so their first load step already counts slip).
    /// Number of free rigid-body translations (extra unknowns of the analysis).
    pub fn n_extra(&self) -> usize {
        self.extra_load.len()
    }

    /// Applied force of extra unknown `idx` per unit load factor.
    pub fn extra_load(&self, idx: usize) -> f64 {
        self.extra_load[idx]
    }

    /// Translation of every rigid master at load factor `lambda` with free translations `q` (zero for
    /// deformable masters).
    pub fn rigid_translations(&self, lambda: f64, q: &[f64]) -> Vec<V3> {
        self.specs
            .iter()
            .enumerate()
            .map(|(si, s)| match &s.master {
                Master::Rigid(r) => {
                    let mut t = r.translation(lambda);
                    for &(c, idx) in &self.extra[si] {
                        t[c] += q.get(idx).copied().unwrap_or(0.0);
                    }
                    t
                }
                Master::Faces(_) => [0.0; 3],
            })
            .collect()
    }

    /// `(spec, component, extra index)` of every free rigid translation.
    pub fn extra_unknowns(&self) -> Vec<(usize, usize, usize)> {
        self.extra.iter().enumerate().flat_map(|(si, e)| e.iter().map(move |&(c, idx)| (si, c, idx))).collect()
    }

    /// Reference positions of the slave collocation points of interface `spec`, in the order of its
    /// state vector (for mapping contact pressures back to the geometry).
    pub fn slave_positions(&self, spec: usize) -> Vec<V3> {
        self.pts[spec].iter().map(|p| p.x0).collect()
    }

    /// Integration weights (reference measure x thickness or `2 pi r`) of the slave collocation points of `spec`:
    /// pressure times weight is the force each point carries.
    pub fn slave_weights(&self, spec: usize) -> Vec<f64> {
        self.pts[spec].iter().map(|p| p.w).collect()
    }

    pub fn initial_state(&self, mesh: &Mesh, pat: &Pattern) -> Vec<Vec<CpState>> {
        let s0: Vec<Vec<CpState>> = self.pts.iter().map(|p| vec![CpState::default(); p.len()]).collect();
        let mut s1 = s0.clone();
        let zero = vec![0.0; mesh.n_dofs()];
        let mut scratch = vec![0.0; mesh.n_dofs()];
        let q0 = vec![0.0; self.n_extra()];
        if self.eval(mesh, pat, &zero, &q0, 0.0, &s0, &mut s1, &mut scratch, None, None).is_ok() {
            self.commit_history(&mut s1);
        }
        for sp in s1.iter_mut().flatten() {
            sp.lam_n = 0.0;
            sp.lam_t = [0.0; 3];
            sp.p = 0.0;
            sp.fric = [0.0; 3];
            sp.active = false;
        }
        s1
    }

    /// Node couplings the matrix pattern needs: every slave-face node with every master-face node
    /// of faces within the margin at the start.
    pub fn pattern_pairs(&self, mesh: &Mesh) -> Vec<(usize, usize)> {
        let mut pairs = Vec::new();
        for (si, s) in self.specs.iter().enumerate() {
            let Master::Faces(_) = s.master else { continue };
            let m = self.margins[si];
            let bbox = |nodes: &[usize]| -> (V3, V3) {
                let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
                for &n in nodes {
                    for i in 0..3 {
                        lo[i] = lo[i].min(mesh.nodes[n][i]);
                        hi[i] = hi[i].max(mesh.nodes[n][i]);
                    }
                }
                (lo, hi)
            };
            let mboxes: Vec<(V3, V3)> = self.faces[si].iter().map(|(_, n)| bbox(n)).collect();
            let bvh = Bvh::build(&mboxes);
            let mut cand = Vec::new();
            for sf in &s.slave {
                let (slo, shi) = bbox(sf);
                cand.clear();
                bvh.query_box(&scale(&slo, 1.0).map(|v| v - m), &scale(&shi, 1.0).map(|v| v + m), &mut cand);
                for &k in &cand {
                    for &a in sf {
                        for &b in &self.faces[si][k as usize].1 {
                            if a != b {
                                pairs.push((a, b));
                            }
                        }
                    }
                }
            }
        }
        pairs.sort_unstable();
        pairs.dedup();
        pairs
    }
}

// ------------------------------------------------------------------ broad-phase search

/// Binary bounding-volume hierarchy over master face boxes (median split on the longest axis),
/// rebuilt at every evaluation from the current positions: building is `O(n log n)` and cheap next to
/// the solve, and a query costs `O(log n + k)` instead of a scan of every face.
struct Bvh {
    nodes: Vec<BvhNode>,
}

struct BvhNode {
    lo: V3,
    hi: V3,
    /// Leaf: the face index; inner node: `u32::MAX`.
    face: u32,
    left: u32,
    right: u32,
}

impl Bvh {
    fn build(boxes: &[(V3, V3)]) -> Self {
        let mut idx: Vec<u32> = (0..boxes.len() as u32).collect();
        let mut nodes = Vec::with_capacity(2 * boxes.len());
        if !boxes.is_empty() {
            Self::split(boxes, &mut idx, &mut nodes);
        }
        Self { nodes }
    }

    fn split(boxes: &[(V3, V3)], idx: &mut [u32], nodes: &mut Vec<BvhNode>) -> u32 {
        let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
        for &i in idx.iter() {
            for k in 0..3 {
                lo[k] = lo[k].min(boxes[i as usize].0[k]);
                hi[k] = hi[k].max(boxes[i as usize].1[k]);
            }
        }
        let me = nodes.len() as u32;
        if idx.len() == 1 {
            nodes.push(BvhNode { lo, hi, face: idx[0], left: u32::MAX, right: u32::MAX });
            return me;
        }
        nodes.push(BvhNode { lo, hi, face: u32::MAX, left: 0, right: 0 });
        let axis = (0..3).max_by(|&a, &b| (hi[a] - lo[a]).total_cmp(&(hi[b] - lo[b]))).unwrap();
        idx.sort_unstable_by(|&a, &b| {
            let (ca, cb) = (boxes[a as usize].0[axis] + boxes[a as usize].1[axis], boxes[b as usize].0[axis] + boxes[b as usize].1[axis]);
            ca.total_cmp(&cb)
        });
        let mid = idx.len() / 2;
        let (l, r) = idx.split_at_mut(mid);
        let (li, ri) = (Self::split(boxes, l, nodes), Self::split(boxes, r, nodes));
        nodes[me as usize].left = li;
        nodes[me as usize].right = ri;
        me
    }

    /// Faces whose box overlaps the box `[lo, hi]`.
    fn query_box(&self, lo: &V3, hi: &V3, out: &mut Vec<u32>) {
        if self.nodes.is_empty() {
            return;
        }
        let mut stack = vec![0u32];
        while let Some(n) = stack.pop() {
            let nd = &self.nodes[n as usize];
            if (0..3).any(|k| hi[k] < nd.lo[k] || lo[k] > nd.hi[k]) {
                continue;
            }
            if nd.face != u32::MAX {
                out.push(nd.face);
            } else {
                stack.push(nd.left);
                stack.push(nd.right);
            }
        }
    }

    /// Faces whose box contains `y`.
    fn query(&self, y: &V3, out: &mut Vec<u32>) {
        if self.nodes.is_empty() {
            return;
        }
        let mut stack = vec![0u32];
        while let Some(n) = stack.pop() {
            let nd = &self.nodes[n as usize];
            if (0..3).any(|k| y[k] < nd.lo[k] || y[k] > nd.hi[k]) {
                continue;
            }
            if nd.face != u32::MAX {
                out.push(nd.face);
            } else {
                stack.push(nd.left);
                stack.push(nd.right);
            }
        }
    }
}

// ------------------------------------------------------------------ the local point residual

struct PointCtx<'a> {
    d: usize,
    w: f64,
    eps_n: f64,
    eps_t: f64,
    mu: f64,
    os: &'a CpState,
    rigid: Option<&'a RigidMaster>,
    lambda: f64,
    geom: Option<FaceGeom>,
    /// Parametric start of the projection (previous contact point on this face, else the centre).
    xi0: [f64; 2],
    /// The history refers to this face (its parametric coordinates are meaningful).
    same_face: bool,
    /// Use this friction traction instead of computing it (a sliding point's traction is frozen
    /// when forming the tangent).
    frozen: Option<V3>,
    /// Do not clamp the normal pressure at zero: the tangent of an active point is the derivative of the
    /// smooth branch, not a central difference straddling the kink.
    smooth: bool,
    /// Interference of a deformable master (see `ContactSpec::overlap`).
    overlap: f64,
    /// Free rigid translation components (their values are the tail of the local variables).
    free_comps: [usize; 3],
    n_free: usize,
}

struct LocalOut {
    p: f64,
    g: f64,
    fric: V3,
    cur: V3,
    active: bool,
    /// Friction is capped (sliding).
    slip: bool,
    /// Normal and master shape values (for the stabilising tangent of near-contact points).
    n: V3,
    m: [f64; 9],
}

impl PointCtx<'_> {
    /// Residual (force-like contribution to `f_int - f_ext`) over the local variables
    /// `v = [y, X_1, ..., X_nm]`.
    fn residual(&self, v: &[f64], r: &mut [f64]) -> LocalOut {
        let d = self.d;
        r.fill(0.0);
        let mut y = [0.0; 3];
        y[..d].copy_from_slice(&v[..d]);
        let nm = self.geom.map_or(0, |g| g.nn);
        let mut x = [[0.0f64; 3]; 9];
        for b in 0..nm {
            x[b][..d].copy_from_slice(&v[d * (1 + b)..d * (2 + b)]);
        }
        let qoff = d * (1 + nm);
        let mut qv = [0.0; 3];
        for j in 0..self.n_free {
            qv[self.free_comps[j]] = v[qoff + j];
        }
        let (g, n, xc, mshape, xi) = match (self.rigid, self.geom) {
            (Some(rm), _) => {
                let (g, n, xc) = rm.gap_q(&y, self.lambda, &qv, d);
                (g, n, xc, [0.0; 9], [0.0; 2])
            }
            (None, Some(geom)) => match project(&geom, &x[..nm], &y, self.xi0) {
                Some(pr) => {
                    let (m, _) = geom.shape(pr.xi);
                    (pr.g - self.overlap, pr.n, pr.xc, m, pr.xi)
                }
                None => return LocalOut { p: 0.0, g: f64::INFINITY, fric: [0.0; 3], cur: [0.0; 3], active: false, slip: false, n: [0.0; 3], m: [0.0; 9] },
            },
            _ => unreachable!("a contact point has a rigid or a deformable master"),
        };
        let praw = self.os.lam_n - self.eps_n * g;
        let p = if self.smooth { praw } else { praw.max(0.0) };
        let cur = if self.rigid.is_some() { xc } else { [xi[0], xi[1], 0.0] };
        if p <= 0.0 && !self.smooth {
            return LocalOut { p: 0.0, g, fric: [0.0; 3], cur, active: false, slip: false, n, m: mshape };
        }
        let w = self.w;
        for i in 0..d {
            r[i] += -p * w * n[i];
        }
        for b in 0..nm {
            for i in 0..d {
                r[d * (1 + b) + i] += p * w * mshape[b] * n[i];
            }
        }
        let mut fric = [0.0; 3];
        let mut slip = false;
        if self.mu > 0.0 {
            // Slip over the master material point of the last committed state, in the tangent plane.
            let prev = if !self.os.has_hist {
                None
            } else if let Some(_rm) = self.rigid {
                Some(self.os.hist)
            } else if self.same_face {
                let geom = self.geom.unwrap();
                let (xp, _, _) = face_point(&geom, &x[..nm], [self.os.hist[0], self.os.hist[1]]);
                Some(xp)
            } else {
                None
            };
            let mut s = prev.map_or([0.0; 3], |pv| sub(&xc, &pv));
            // The slip was measured in the body's frame: bring it to the world frame when the body turns.
            if d == 2 && self.rigid.is_some() && qv[2] != 0.0 {
                let (sn, cs) = qv[2].sin_cos();
                s = [cs * s[0] - sn * s[1], sn * s[0] + cs * s[1], 0.0];
            }
            let st = sub(&s, &scale(&n, dot(&s, &n)));
            let lt = sub(&self.os.lam_t, &scale(&n, dot(&self.os.lam_t, &n)));
            let trial = add(&lt, &scale(&st, self.eps_t));
            // A smooth-branch evaluation (the tangent) can see a tiny negative pressure at a point on the
            // active-set boundary: no friction capacity then, never a negative one (which flipped the traction
            // and, with no slip, divided zero by zero).
            let cap = (self.mu * p).max(0.0);
            let tn = norm(&trial);
            slip = tn > cap;
            fric = match self.frozen {
                Some(f) => f,
                None if slip => scale(&trial, cap / tn),
                None => trial,
            };
            for i in 0..d {
                r[i] += w * fric[i];
            }
            for b in 0..nm {
                for i in 0..d {
                    r[d * (1 + b) + i] -= w * mshape[b] * fric[i];
                }
            }
        }
        // The rigid body feels the opposite of the slave-point force: its residual is minus the y residual.
        for j in 0..self.n_free {
            let comp = self.free_comps[j];
            if d == 2 && comp == 2 {
                // The master's rotation: minus the moment of the slave-point force about the circle's centre.
                let (c, shift) = match self.rigid.map(|rm| rm.shape) {
                    Some(RigidShape::Circle { c, .. }) => (c, self.rigid.map_or([0.0; 3], |rm| rm.translation(self.lambda))),
                    _ => ([0.0; 2], [0.0; 3]),
                };
                let cw = [c[0] + shift[0] + qv[0], c[1] + shift[1] + qv[1]];
                r[qoff + j] = -((y[0] - cw[0]) * r[1] - (y[1] - cw[1]) * r[0]);
            } else {
                r[qoff + j] = -r[comp];
            }
        }
        LocalOut { p, g, fric, cur, active: true, slip, n, m: mshape }
    }
}

fn ctx_copy<'a>(c: &PointCtx<'a>) -> PointCtx<'a> {
    PointCtx { d: c.d, w: c.w, eps_n: c.eps_n, eps_t: c.eps_t, mu: c.mu, os: c.os, rigid: c.rigid, lambda: c.lambda, geom: c.geom, xi0: c.xi0, same_face: c.same_face, frozen: c.frozen, overlap: c.overlap, free_comps: c.free_comps, n_free: c.n_free, smooth: c.smooth }
}

struct PointOut {
    /// A point that is only close to the master: its stiffness is used only if nothing is in contact.
    near_only: bool,
    dofs: Vec<usize>,
    r: Vec<f64>,
    k: Vec<f64>,
    /// Exact minus factorised tangent of a frictional point (see the tangent comment), over the same dofs.
    kdef: Vec<f64>,
    st: CpState,
    master_force: V3,
    missing: Vec<(usize, usize)>,
}

impl ContactSet {
    /// Hold the discrete state of `state`: until [`Self::release_active_set`] a point is in contact exactly when it is in
    /// `state` (its pressure then follows the smooth branch, a tiny negative value included, instead of toggling at zero),
    /// and a point in contact stays on the master face it is on (a point at an element edge of the master surface would
    /// otherwise flip between two faces whose normals differ).
    pub fn hold_active_set(&self, state: &[Vec<CpState>]) {
        if let Ok(mut h) = self.hold.write() {
            *h = Some(state.iter().map(|v| v.iter().map(|c| (c.active, c.cur_face)).collect()).collect());
        }
    }

    pub fn release_active_set(&self) {
        if let Ok(mut h) = self.hold.write() {
            *h = None;
        }
    }

    pub fn active_set_held(&self) -> bool {
        self.hold.read().is_ok_and(|h| h.is_some())
    }

    /// Add the contact forces (and tangent) at displacements `u` and load factor `lambda`. `old` is the
    /// committed state; the trial results go to `new`.
    #[allow(clippy::too_many_arguments)] // one evaluation needs all of mesh, pattern, state, outputs
    pub fn eval(&self, mesh: &Mesh, pat: &Pattern, u: &[f64], q: &[f64], lambda: f64, old: &[Vec<CpState>], new: &mut [Vec<CpState>], f: &mut [f64], mut k: Option<&mut BlockMatrix>, mut extra: Option<&mut ExtraOut>) -> Result<ContactStats, String> {
        let d = self.dim;
        let ndm = mesh.n_dofs();
        let mut stats = ContactStats { min_gap: f64::INFINITY, master_force: vec![[0.0; 3]; self.specs.len()], ..Default::default() };
        let cur = |n: usize| -> V3 { std::array::from_fn(|i| if i < d { mesh.nodes[n][i] + u[n * d + i] } else { 0.0 }) };
        for (si, spec) in self.specs.iter().enumerate() {
            let rigid = match &spec.master {
                Master::Rigid(r) => Some(r),
                Master::Faces(_) => None,
            };
            let faces = &self.faces[si];
            let margin = self.margins[si];
            // Current master face positions and their search boxes, once per evaluation.
            let face_x: Vec<Vec<V3>> = faces.iter().map(|(_, mn)| mn.iter().map(|&n| cur(n)).collect()).collect();
            let boxes: Vec<(V3, V3)> = face_x
                .iter()
                .map(|x| {
                    let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
                    for p in x {
                        for i in 0..3 {
                            lo[i] = lo[i].min(p[i] - margin);
                            hi[i] = hi[i].max(p[i] + margin);
                        }
                    }
                    (lo, hi)
                })
                .collect();
            let bvh = Bvh::build(&boxes);
            let want_k = k.is_some();
            let stabilise = self.stabilise.load(std::sync::atomic::Ordering::Relaxed);
            let act_gap = self.activation_gap(si);
            let fc: &[(usize, usize)] = &self.extra[si];
            let n_free = fc.len();
            let mut free_comps = [0usize; 3];
            for (j, (c, _)) in fc.iter().enumerate() {
                free_comps[j] = *c;
            }
            let hold_guard = self.hold.read().map_err(|_| "contact hold lock poisoned".to_string())?;
            let held_flags: Option<&Vec<(bool, u32)>> = hold_guard.as_ref().map(|h| &h[si]);
            let outs: Vec<PointOut> = self.pts[si]
                .par_iter()
                .zip(old[si].par_iter())
                .enumerate()
                .map(|(pi, (sp, os))| {
                    let ns = sp.nodes.len();
                    // Slave point and, for a deformable master, the face it projects onto.
                    let mut y = [0.0; 3];
                    for (a, &nd) in sp.nodes.iter().enumerate() {
                        let c = cur(nd);
                        for i in 0..3 {
                            y[i] += sp.n[a] * c[i];
                        }
                    }
                    let mut chosen: Option<(usize, [f64; 2])> = None;
                    if rigid.is_none() {
                        let mut best = f64::INFINITY;
                        let mut cand = Vec::new();
                        // A held point stays on its face while the projection still lands on it.
                        let held_face = held_flags.and_then(|h| h.get(pi)).filter(|h| h.0).map(|h| h.1 as usize).filter(|&fi| fi < faces.len());
                        // The face of the committed state when the point still lies over it.
                        let mut sticky: Option<(usize, [f64; 2])> = None;
                        if let Some(fi) = held_face {
                            let (geom, _) = &faces[fi];
                            let xi0 = if os.has_hist && os.face as usize == fi { [os.hist[0], os.hist[1]] } else { geom.centre() };
                            if let Some(pr) = project(geom, &face_x[fi], &y, xi0) {
                                if geom.contains(pr.xi, 0.1) && pr.g.abs() < margin {
                                    chosen = Some((fi, pr.xi));
                                }
                            }
                        }
                        if chosen.is_none() {
                            bvh.query(&y, &mut cand);
                        }
                        for &fi in &cand {
                            let fi = fi as usize;
                            let (geom, _) = &faces[fi];
                            let x = &face_x[fi];
                            let committed = os.has_hist && os.face as usize == fi;
                            let xi0 = if committed { [os.hist[0], os.hist[1]] } else { geom.centre() };
                            if let Some(pr) = project(geom, x, &y, xi0) {
                                if geom.contains(pr.xi, 0.1) && pr.g.abs() < margin && pr.g.abs() < best {
                                    best = pr.g.abs();
                                    chosen = Some((fi, pr.xi));
                                }
                                if committed && geom.contains(pr.xi, STICKY_TOL) && pr.g.abs() < margin {
                                    sticky = Some((fi, pr.xi));
                                }
                            }
                        }
                        // A slave point on (or within round-off of) an element edge of the master surface is on two faces at once,
                        // whose normals differ (C0 surface): which one the minimum-gap rule picks then depends on round-off and
                        // flips between Newton iterations, jumping the residual. Stay on the committed face while the point is over it.
                        if sticky.is_some() && held_face.is_none() {
                            chosen = sticky;
                        }
                        if chosen.is_none() {
                            let mut st = *os;
                            st.active = false;
                            st.p = 0.0;
                            st.g = f64::INFINITY;
                            st.fric = [0.0; 3];
                            return PointOut { near_only: false, dofs: Vec::new(), r: Vec::new(), k: Vec::new(), kdef: Vec::new(), st, master_force: [0.0; 3], missing: Vec::new() };
                        }
                    }
                    let (mgeom, mnodes): (Option<FaceGeom>, &[usize]) = match chosen {
                        Some((fi, _)) => (Some(faces[fi].0), &faces[fi].1),
                        None => (None, &[]),
                    };
                    let nm = mnodes.len();
                    let qoff = d * (1 + nm);
                    let nv = qoff + n_free;
                    let mut free_comps = [0usize; 3];
                    for (j, (c, _)) in fc.iter().enumerate() {
                        free_comps[j] = *c;
                    }
                    let mut v = vec![0.0; nv];
                    v[..d].copy_from_slice(&y[..d]);
                    for (b, &nd) in mnodes.iter().enumerate() {
                        let c = cur(nd);
                        v[d * (1 + b)..d * (2 + b)].copy_from_slice(&c[..d]);
                    }
                    for (j, (_, idx)) in fc.iter().enumerate() {
                        v[qoff + j] = q[*idx];
                    }
                    let same_face = chosen.is_some_and(|(fi, _)| os.has_hist && os.face as usize == fi);
                    let xi0 = chosen.map_or([0.0; 2], |c| c.1);
                    // A held active set: in contact means the smooth pressure branch (no clamp at zero), out of contact means none.
                    let held = held_flags.and_then(|h| h.get(pi).map(|h| h.0));
                    if held == Some(false) {
                        let mut st = *os;
                        st.active = false;
                        st.p = 0.0;
                        st.fric = [0.0; 3];
                        return PointOut { near_only: false, dofs: Vec::new(), r: Vec::new(), k: Vec::new(), kdef: Vec::new(), st, master_force: [0.0; 3], missing: Vec::new() };
                    }
                    let ctx = PointCtx { d, w: sp.w, eps_n: spec.eps_n, eps_t: spec.eps_t, mu: spec.mu, os, rigid, lambda, geom: mgeom, xi0, same_face, frozen: None, overlap: spec.overlap, free_comps, n_free, smooth: held == Some(true) };
                    let mut r = vec![0.0; nv];
                    let out = ctx.residual(&v, &mut r);
                    let mut st = *os;
                    st.p = out.p;
                    st.g = out.g;
                    st.fric = out.fric;
                    st.active = out.active;
                    st.cur = out.cur;
                    st.cur_face = chosen.map_or(0, |c| c.0 as u32);
                    st.n = out.n;
                    let near = !out.active && out.g.is_finite() && out.g < act_gap && stabilise;
                    if !(out.active || near && want_k) {
                        return PointOut { near_only: false, dofs: Vec::new(), r: Vec::new(), k: Vec::new(), kdef: Vec::new(), st, master_force: [0.0; 3], missing: Vec::new() };
                    }
                    // Tangent: central differences of the local residual (or, for a point that is only
                    // close, the penalty stiffness along the normal).
                    let mut kv = Vec::new();
                    let mut kdef_loc: Vec<f64> = Vec::new();
                    if near {
                        kv = vec![0.0; nv * nv];
                        let mut grad = vec![0.0; nv];
                        grad[..d].copy_from_slice(&out.n[..d]);
                        for b in 0..nm {
                            for i in 0..d {
                                grad[d * (1 + b) + i] = -out.m[b] * out.n[i];
                            }
                        }
                        for j in 0..n_free {
                            grad[qoff + j] = -out.n[free_comps[j]]; // the gap falls as the body moves toward the slave point
                        }
                        for i in 0..nv {
                            for j in 0..nv {
                                kv[i * nv + j] = spec.eps_n * sp.w * grad[i] * grad[j];
                            }
                        }
                        r.fill(0.0);
                    } else if want_k {
                        kv = vec![0.0; nv * nv];
                        let (mut rp, mut rm) = (vec![0.0; nv], vec![0.0; nv]);
                        // A sliding point's capped traction is frozen in the tangent: its derivative is
                        // unsymmetric and (symmetrised) indefinite; the residual keeps the exact friction.
                        let tctx = PointCtx { frozen: if out.slip { Some(out.fric) } else { None }, smooth: true, ..ctx_copy(&ctx) };
                        for j in 0..nv {
                            let h = 1e-6 * (1.0 + v[j].abs());
                            let (mut vp, mut vm) = (v.clone(), v.clone());
                            vp[j] += h;
                            vm[j] -= h;
                            tctx.residual(&vp, &mut rp);
                            tctx.residual(&vm, &mut rm);
                            for i in 0..nv {
                                kv[i * nv + j] = (rp[i] - rm[i]) / (2.0 * h);
                            }
                        }
                        // The tangent the solver factorises is symmetric (and freezes sliding friction); the exact
                        // one differs by the sliding points' `mu dp` coupling and the unsymmetric part. That difference
                        // is kept as a defect operator and applied by iterative refinement over the factorisation
                        // (`nonlinear.rs`), which makes the Newton step consistent without an unsymmetric factorisation.
                        let mut kfull = vec![0.0; nv * nv];
                        // (Every frictional point: the sticking ones are unsymmetric too through the projection and the
                        // normal; restricting the defect to sliding points was tried and Newton stalls again.)
                        let want_defect = spec.mu > 0.0 && n_free == 0;
                        if want_defect {
                            let fctx = PointCtx { frozen: None, smooth: true, ..ctx_copy(&ctx) };
                            for j in 0..nv {
                                let h = 1e-6 * (1.0 + v[j].abs());
                                let (mut vp, mut vm) = (v.clone(), v.clone());
                                vp[j] += h;
                                vm[j] -= h;
                                fctx.residual(&vp, &mut rp);
                                fctx.residual(&vm, &mut rm);
                                for i in 0..nv {
                                    kfull[i * nv + j] = (rp[i] - rm[i]) / (2.0 * h);
                                }
                            }
                        }
                        for i in 0..nv {
                            for j in 0..i {
                                let m = 0.5 * (kv[i * nv + j] + kv[j * nv + i]);
                                kv[i * nv + j] = m;
                                kv[j * nv + i] = m;
                            }
                        }
                        if want_defect {
                            kdef_loc = (0..nv * nv).map(|ix| kfull[ix] - kv[ix]).collect();
                        }
                    }
                    // Expand the local variables to the nodal dofs: slave nodes (shape-weighted), then master nodes.
                    let nd_mesh = d * (ns + nm);
                    let nd_tot = nd_mesh + n_free;
                    let mut dofs = Vec::with_capacity(nd_tot);
                    for &nd in sp.nodes.iter().chain(mnodes.iter()) {
                        for i in 0..d {
                            dofs.push(nd * d + i);
                        }
                    }
                    for (_, idx) in fc {
                        dofs.push(ndm + idx); // an extra unknown: its index follows the mesh dofs
                    }
                    // E[var][dof]
                    let mut e = vec![0.0; nv * nd_tot];
                    for a in 0..ns {
                        for i in 0..d {
                            e[i * nd_tot + a * d + i] = sp.n[a];
                        }
                    }
                    for b in 0..nm {
                        for i in 0..d {
                            e[(d * (1 + b) + i) * nd_tot + (ns + b) * d + i] = 1.0;
                        }
                    }
                    for j in 0..n_free {
                        e[(qoff + j) * nd_tot + nd_mesh + j] = 1.0;
                    }
                    let mut rd = vec![0.0; nd_tot];
                    for var in 0..nv {
                        for c in 0..nd_tot {
                            rd[c] += e[var * nd_tot + c] * r[var];
                        }
                    }
                    // `E^T K E`: local variables to the nodal dofs.
                    let expand = |kloc: &[f64]| -> Vec<f64> {
                        let mut kd = vec![0.0; nd_tot * nd_tot];
                        let mut tmp = vec![0.0; nv * nd_tot]; // K_v E
                        for i in 0..nv {
                            for j in 0..nv {
                                let kij = kloc[i * nv + j];
                                if kij != 0.0 {
                                    for c in 0..nd_tot {
                                        tmp[i * nd_tot + c] += kij * e[j * nd_tot + c];
                                    }
                                }
                            }
                        }
                        for i in 0..nv {
                            for c1 in 0..nd_tot {
                                let eic = e[i * nd_tot + c1];
                                if eic != 0.0 {
                                    for c2 in 0..nd_tot {
                                        kd[c1 * nd_tot + c2] += eic * tmp[i * nd_tot + c2];
                                    }
                                }
                            }
                        }
                        kd
                    };
                    let kd = if want_k { expand(&kv) } else { Vec::new() };
                    let kdef = if kdef_loc.is_empty() { Vec::new() } else { expand(&kdef_loc) };
                    let mut missing = Vec::new();
                    if want_k {
                        // Matrix couplings must exist in the pattern (checked at scatter time).
                        for &a in &sp.nodes {
                            for &b in mnodes {
                                if a != b && pat.find(a.max(b), a.min(b)).is_none() {
                                    missing.push((a, b));
                                }
                            }
                        }
                    }
                    let mf: V3 = std::array::from_fn(|i| if i < d { r[i] } else { 0.0 });
                    PointOut { near_only: near, dofs, r: rd, k: kd, kdef, st, master_force: mf, missing }
                })
                .collect();
            // Stabilising stiffness of merely-near points only when nothing at all is in contact yet.
            let any_active = outs.iter().any(|o| o.st.active);
            for (kk, o) in outs.into_iter().enumerate() {
                new[si][kk] = o.st;
                if o.near_only && any_active {
                    continue;
                }
                if o.st.active {
                    stats.n_active += 1;
                    stats.min_gap = stats.min_gap.min(o.st.g);
                    for i in 0..3 {
                        stats.master_force[si][i] += o.master_force[i];
                    }
                }
                stats.missing_pairs.extend(o.missing);
                if !o.kdef.is_empty() && o.st.active {
                    stats.defect.push((o.dofs.clone(), o.kdef));
                }
                for (c, &dof) in o.dofs.iter().enumerate() {
                    if dof < ndm {
                        f[dof] += o.r[c];
                    } else if let Some(ex) = extra.as_deref_mut() {
                        ex.rq[dof - ndm] += o.r[c];
                    }
                }
                if let Some(km) = k.as_deref_mut() {
                    let nt = o.dofs.len();
                    let dd = d * d;
                    let m_all = self.n_extra();
                    for i in 0..nt {
                        let (di, i_extra) = (o.dofs[i], o.dofs[i] >= ndm);
                        for j in 0..nt {
                            let (dj, j_extra) = (o.dofs[j], o.dofs[j] >= ndm);
                            match (i_extra, j_extra) {
                                (false, false) => {
                                    let (ni, ci, nj, cj) = (di / d, di % d, dj / d, dj % d);
                                    if ni < nj {
                                        continue;
                                    }
                                    if let Some(pos) = pat.find(ni, nj) {
                                        km.vals[pos * dd + ci * d + cj] += o.k[i * nt + j];
                                    }
                                }
                                // Mesh row, extra column: the coupling block (its transpose is the same numbers).
                                (false, true) => {
                                    if let Some(ex) = extra.as_deref_mut() {
                                        ex.kuq[(dj - ndm) * ndm + di] += o.k[i * nt + j];
                                    }
                                }
                                (true, false) => {}
                                (true, true) => {
                                    if let Some(ex) = extra.as_deref_mut() {
                                        ex.kqq[(di - ndm) * m_all + (dj - ndm)] += o.k[i * nt + j];
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        stats.missing_pairs.sort_unstable();
        stats.missing_pairs.dedup();
        if stats.min_gap == f64::INFINITY {
            stats.min_gap = 0.0;
        }
        Ok(stats)
    }

    /// Augmented-Lagrangian multiplier update from the trial pressures and friction tractions of
    /// `state` (a converged evaluation). Returns the largest relative change.
    pub fn update_multipliers(&self, state: &mut [Vec<CpState>]) -> f64 {
        let pmax = state.iter().flatten().fold(0.0f64, |m, s| m.max(s.p)).max(1e-300);
        let mut change = 0.0f64;
        for sp in state.iter_mut().flatten() {
            let (pn, ft) = if sp.active { (sp.p, sp.fric) } else { (0.0, [0.0; 3]) };
            change = change.max((pn - sp.lam_n).abs() / pmax).max(norm(&sub(&ft, &sp.lam_t)) / pmax);
            sp.lam_n = pn;
            sp.lam_t = ft;
        }
        change
    }

    /// One line for `NL_TRACE`: per spec the points in contact, those at their friction limit, and those whose master
    /// face differs from the committed one.
    pub fn describe(&self, state: &[Vec<CpState>]) -> String {
        state
            .iter()
            .zip(&self.specs)
            .map(|(pts, spec)| {
                let active = pts.iter().filter(|c| c.active).count();
                let slip = pts.iter().filter(|c| c.active && spec.mu > 0.0 && norm(&c.fric) >= spec.mu * c.p * (1.0 - 1e-6)).count();
                let moved = pts.iter().filter(|c| c.active && c.has_hist && c.face != c.cur_face).count();
                format!("[{}: {active} in contact, {slip} slipping, {moved} off their face]", spec.name)
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Commit the slip history (the contact point of the converged state) for every point that is
    /// in contact or within the activation gap of its master.
    pub fn commit_history(&self, state: &mut [Vec<CpState>]) {
        for (si, pts) in state.iter_mut().enumerate() {
            let act = self.activation_gap(si);
            for sp in pts.iter_mut() {
                if sp.active || (sp.g.is_finite() && sp.g < act) {
                    sp.hist = sp.cur;
                    sp.face = sp.cur_face;
                    sp.has_hist = true;
                } else {
                    sp.has_hist = false;
                    sp.lam_t = [0.0; 3];
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quad_edge_on_circle(rad: f64, a0: f64, a1: f64) -> Vec<V3> {
        // Three-node edge (ends then midside) on a circle of radius `rad` about the origin.
        let p = |t: f64| [rad * t.cos(), rad * t.sin(), 0.0];
        vec![p(a0), p(a1), p(0.5 * (a0 + a1))]
    }

    #[test]
    fn bvh_queries_match_a_brute_force_scan() {
        let mut seed = 7u64;
        let mut rnd = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed >> 11) as f64 / (1u64 << 53) as f64
        };
        let boxes: Vec<(V3, V3)> = (0..400)
            .map(|_| {
                let c = [10.0 * rnd(), 10.0 * rnd(), 10.0 * rnd()];
                let h = [0.05 + 0.4 * rnd(), 0.05 + 0.4 * rnd(), 0.05 + 0.4 * rnd()];
                ([c[0] - h[0], c[1] - h[1], c[2] - h[2]], [c[0] + h[0], c[1] + h[1], c[2] + h[2]])
            })
            .collect();
        let bvh = Bvh::build(&boxes);
        assert_eq!(bvh.nodes.len(), 2 * boxes.len() - 1, "a full binary tree");
        for _ in 0..300 {
            let y = [10.0 * rnd(), 10.0 * rnd(), 10.0 * rnd()];
            let mut got = Vec::new();
            bvh.query(&y, &mut got);
            got.sort_unstable();
            let want: Vec<u32> = (0..boxes.len() as u32).filter(|&i| (0..3).all(|k| y[k] >= boxes[i as usize].0[k] && y[k] <= boxes[i as usize].1[k])).collect();
            assert_eq!(got, want);
            let (lo, hi) = (y, [y[0] + 1.0, y[1] + 0.5, y[2] + 2.0]);
            let mut got = Vec::new();
            bvh.query_box(&lo, &hi, &mut got);
            got.sort_unstable();
            let want: Vec<u32> = (0..boxes.len() as u32).filter(|&i| (0..3).all(|k| hi[k] >= boxes[i as usize].0[k] && lo[k] <= boxes[i as usize].1[k])).collect();
            assert_eq!(got, want);
        }
        assert!(Bvh::build(&[]).nodes.is_empty());
    }

    #[test]
    fn edge_projection_is_orthogonal_and_gives_the_signed_distance() {
        let geom = FaceGeom::new(2, 3).unwrap();
        let x = quad_edge_on_circle(2.0, 0.2, 0.6);
        for (r, th) in [(2.3, 0.4), (1.9, 0.35), (2.05, 0.5)] {
            let y = [r * f64::cos(th), r * f64::sin(th), 0.0];
            let pr = project(&geom, &x, &y, [0.0, 0.0]).unwrap();
            let (_, tau, _) = face_point(&geom, &x, pr.xi);
            assert!(dot(&sub(&y, &pr.xc), &tau[0]).abs() < 1e-12, "projection must be orthogonal to the edge");
            // The quadratic edge follows the circle to O(h^3): the distance is |r - R| to that order.
            assert!((pr.g - (r - 2.0)).abs() < 2e-3, "g {} vs {}", pr.g, r - 2.0);
            // Outward normal of a counter-clockwise edge of a body that lies inside the circle.
            assert!(dot(&pr.n, &[th.cos(), th.sin(), 0.0]) > 0.99);
        }
        // Reverse orientation flips the normal.
        let rev = vec![x[1], x[0], x[2]];
        let y = [2.1 * 0.4f64.cos(), 2.1 * 0.4f64.sin(), 0.0];
        let pr = project(&geom, &rev, &y, [0.0, 0.0]).unwrap();
        assert!(pr.g < 0.0, "the opposite orientation puts the point behind the face: {}", pr.g);
    }

    #[test]
    fn face_shapes_partition_unity_and_second_derivatives_match_exact_values() {
        for (dim, nn) in [(2, 2), (2, 3), (3, 3), (3, 6), (3, 4), (3, 8), (3, 9)] {
            let g = FaceGeom::new(dim, nn).unwrap();
            let xi = if g.p == 1 { [0.3, 0.0] } else { [0.21, 0.17] };
            let (n, dn) = g.shape(xi);
            assert!((n[..nn].iter().sum::<f64>() - 1.0).abs() < 1e-13);
            for a in 0..g.p {
                assert!(dn[..nn].iter().map(|r| r[a]).sum::<f64>().abs() < 1e-12, "derivatives sum to zero");
            }
        }
        // Line3: N_3 = 1 - x^2 has second derivative -2.
        let g = FaceGeom::new(2, 3).unwrap();
        let d2 = g.second([0.4, 0.0]);
        assert!((d2[2][0][0] + 2.0).abs() < 1e-6 && (d2[0][0][0] - 1.0).abs() < 1e-6);
    }

    fn rigid_ctx<'a>(rm: &'a RigidMaster, os: &'a CpState) -> PointCtx<'a> {
        PointCtx { d: 2, w: 0.7, eps_n: 1.0e4, eps_t: 5.0e3, mu: 0.0, os, rigid: Some(rm), lambda: 0.0, geom: None, xi0: [0.0; 2], same_face: false, frozen: None, overlap: 0.0, free_comps: [0; 3], n_free: 0, smooth: false }
    }

    fn fd_tangent(ctx: &PointCtx, v: &[f64]) -> Vec<f64> {
        let nv = v.len();
        let mut k = vec![0.0; nv * nv];
        let (mut rp, mut rm) = (vec![0.0; nv], vec![0.0; nv]);
        for j in 0..nv {
            let h = 1e-6 * (1.0 + v[j].abs());
            let (mut vp, mut vm) = (v.to_vec(), v.to_vec());
            vp[j] += h;
            vm[j] -= h;
            ctx.residual(&vp, &mut rp);
            ctx.residual(&vm, &mut rm);
            for i in 0..nv {
                k[i * nv + j] = (rp[i] - rm[i]) / (2.0 * h);
            }
        }
        k
    }

    #[test]
    fn rigid_circle_tangent_has_the_penalty_and_the_curvature_terms() {
        let rm = RigidMaster::new(RigidShape::Circle { c: [0.0, 0.0], r: 1.0 });
        let os = CpState::default();
        let ctx = rigid_ctx(&rm, &os);
        let y = [0.8 * 0.6, 0.8 * 0.8]; // radius 0.8: penetrates by 0.2 (g = -0.2)
        let mut r = [0.0; 2];
        let out = ctx.residual(&y, &mut r);
        assert!(out.active && (out.g + 0.2).abs() < 1e-14 && (out.p - 2000.0).abs() < 1e-9);
        // Force on the slave is outward (+n): the residual (minus that force) points inward.
        assert!((r[0] + out.p * 0.7 * 0.6).abs() < 1e-9 && (r[1] + out.p * 0.7 * 0.8).abs() < 1e-9);
        let k = fd_tangent(&ctx, &y);
        let n = [0.6, 0.8];
        let rho = 0.8;
        for i in 0..2 {
            for j in 0..2 {
                // K = w [ eps n n^T - p (I - n n^T) / rho ]
                let kk = 0.7 * (1.0e4 * n[i] * n[j] - out.p * ((if i == j { 1.0 } else { 0.0 }) - n[i] * n[j]) / rho);
                assert!((k[i * 2 + j] - kk).abs() < 1e-5 * 1.0e4, "({i},{j}): {} vs {kk}", k[i * 2 + j]);
            }
        }
        // No contact when the point is outside the circle.
        let out = ctx.residual(&[1.2, 0.0], &mut r);
        assert!(!out.active && r == [0.0, 0.0]);
        // The augmented multiplier carries a pressure beyond zero gap.
        let os2 = CpState { lam_n: 500.0, ..CpState::default() };
        let out = rigid_ctx(&rm, &os2).residual(&[1.02, 0.0], &mut r); // g = 0.02 -> p = 500 - 200 = 300
        assert!(out.active && (out.p - 300.0).abs() < 1e-9);
    }

    #[test]
    fn a_deformable_contact_pair_is_in_force_and_moment_equilibrium() {
        let geom = FaceGeom::new(2, 2).unwrap();
        let os = CpState::default();
        let ctx = PointCtx { d: 2, w: 0.5, eps_n: 1.0e3, eps_t: 0.0, mu: 0.0, os: &os, rigid: None, lambda: 0.0, geom: Some(geom), xi0: [0.0; 2], same_face: false, frozen: None, overlap: 0.0, free_comps: [0; 3], n_free: 0, smooth: false };
        // Master edge from (2,0) to (0,0): listed with the body on its left, so the outward normal is -y... the
        // slave point sits just below the edge.
        let (xa, xb) = ([2.0, 0.0], [0.0, 0.0]);
        let y = [0.7, -0.05];
        let v = [y[0], y[1], xa[0], xa[1], xb[0], xb[1]];
        let mut r = [0.0; 6];
        let out = ctx.residual(&v, &mut r);
        assert!(out.active && (out.g + 0.05).abs() < 1e-13, "{} {}", out.g, out.active);
        // Force balance and (collinear pair) moment balance.
        let (fx, fy) = (r[0] + r[2] + r[4], r[1] + r[3] + r[5]);
        assert!(fx.abs() < 1e-12 && fy.abs() < 1e-12, "net force ({fx}, {fy})");
        let moment = |x: f64, y: f64, fx: f64, fy: f64| x * fy - y * fx;
        let m = moment(v[0], v[1], r[0], r[1]) + moment(v[2], v[3], r[2], r[3]) + moment(v[4], v[5], r[4], r[5]);
        assert!(m.abs() < 1e-10, "net moment {m}");
        // The master receives the reaction distributed by shape functions: node at x = 2 carries 0.35 of it.
        let ratio = r[3] / r[1];
        assert!((ratio.abs() - 0.35).abs() < 1e-9, "distribution {ratio}");
        // The FD tangent is symmetric.
        let k = fd_tangent(&ctx, &v);
        for i in 0..6 {
            for j in 0..6 {
                assert!((k[i * 6 + j] - k[j * 6 + i]).abs() < 1e-4 * 1.0e3, "asymmetry ({i},{j})");
            }
        }
    }

    #[test]
    fn coulomb_friction_sticks_below_mu_p_and_slides_at_mu_p() {
        let rm = RigidMaster::new(RigidShape::Plane { p: [0.0, 0.0, 0.0], n: [0.0, 1.0, 0.0] });
        let hist = CpState { has_hist: true, hist: [0.0, 0.0, 0.0], ..CpState::default() };
        fn mk<'a>(rm: &'a RigidMaster, os: &'a CpState, mu: f64) -> PointCtx<'a> {
            PointCtx { d: 2, w: 1.0, eps_n: 1.0e4, eps_t: 2.0e3, mu, os, rigid: Some(rm), lambda: 0.0, geom: None, xi0: [0.0; 2], same_face: false, frozen: None, overlap: 0.0, free_comps: [0; 3], n_free: 0, smooth: false }
        }
        let mut r = [0.0; 2];
        // g = -0.01 -> p = 100. Slip 0.02 along x: trial traction 40 < mu p = 50: stick.
        let out = mk(&rm, &hist, 0.5).residual(&[0.02, -0.01], &mut r);
        assert!((out.p - 100.0).abs() < 1e-9 && (out.fric[0] - 40.0).abs() < 1e-9 && out.fric[1].abs() < 1e-12 && !out.slip, "stick: {:?}", out.fric);
        // Slip 0.05: trial 100 > 50: capped at mu p = 50, direction kept.
        let out = mk(&rm, &hist, 0.5).residual(&[0.05, -0.01], &mut r);
        assert!((out.fric[0] - 50.0).abs() < 1e-9 && out.slip, "slip: {:?}", out.fric);
        // Negative slip reverses the traction.
        let out = mk(&rm, &hist, 0.5).residual(&[-0.05, -0.01], &mut r);
        assert!((out.fric[0] + 50.0).abs() < 1e-9);
        // Without a history there is no slip yet.
        let out = mk(&rm, &CpState::default(), 0.5).residual(&[0.05, -0.01], &mut r);
        assert!(out.fric[0].abs() < 1e-12);
        // Frictionless: nothing.
        let out = mk(&rm, &hist, 0.0).residual(&[0.05, -0.01], &mut r);
        assert!(out.fric == [0.0; 3]);
        // A sliding point's traction can be frozen for the tangent: it then no longer depends on the position.
        let frozen = PointCtx { frozen: Some([50.0, 0.0, 0.0]), ..mk(&rm, &hist, 0.5) };
        let out = frozen.residual(&[0.09, -0.01], &mut r);
        assert!(out.fric[0] == 50.0);
    }

    /// A point on the active-set boundary evaluated on the smooth branch (as the tangent does) can see a
    /// tiny negative pressure. Its friction capacity is then zero, never negative: with no slip the old
    /// code divided 0 by 0 (a NaN tangent that stalled every frictional lug analysis near full load).
    #[test]
    fn a_slightly_negative_smooth_pressure_gives_finite_friction_with_no_capacity() {
        let rm = RigidMaster::new(RigidShape::Plane { p: [0.0, 0.0, 0.0], n: [0.0, 1.0, 0.0] });
        let no_slip = CpState { has_hist: true, hist: [0.0, 0.0, 0.0], ..CpState::default() };
        let pc = PointCtx { d: 2, w: 1.0, eps_n: 1.0e4, eps_t: 2.0e3, mu: 0.5, os: &no_slip, rigid: Some(&rm), lambda: 0.0, geom: None, xi0: [0.0; 2], same_face: false, frozen: None, overlap: 0.0, free_comps: [0; 3], n_free: 0, smooth: true };
        let mut r = [0.0; 2];
        // g = +1e-9: p = -1e-5 < 0, slip exactly zero (the contact point sits on its history point).
        let out = pc.residual(&[0.0, 1.0e-9], &mut r);
        assert!(out.p < 0.0 && out.fric.iter().all(|v| v.is_finite()) && r.iter().all(|v| v.is_finite()), "{:?} {:?}", out.fric, r);
        assert!(out.fric[0] == 0.0, "no capacity: {:?}", out.fric);
        // With slip the traction is capped at zero as well (it was flipped by a negative cap before).
        let out = pc.residual(&[0.05, 1.0e-9], &mut r);
        assert!(out.fric[0].abs() < 1e-12 && out.fric.iter().all(|v| v.is_finite()), "{:?}", out.fric);
    }
}
