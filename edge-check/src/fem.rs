//! Plane-stress finite-element model of a plate with a bore near a free
//! edge: 9-node Lagrange quadrilaterals (3x3 Gauss), a structured O-grid
//! around the bore that conforms exactly to the free edge, the far face and
//! the top face, graded toward the bore. The load line is a symmetry axis,
//! so only the half plate `y >= 0` is meshed.
//!
//! Three unit load cases are solved against ONE factorisation (linearity):
//! uniform fit pressure, a cosine pin load over the loaded half of the bore
//! (contact lost on the back side), and a cosine pin load over the full bore
//! (contact retained by the fit). Stress at any point for any combination of
//! `(fit_pressure, pin_load)` is then a linear combination - which is what
//! makes Monte-Carlo sampling and the edge-distance search cheap.

use crate::linalg::BandedSpd;
use crate::types::{Geometry, Stress};
use std::f64::consts::PI;

/// Mesh density. `n_arc` are element counts on the three angular arcs
/// (far face, top face, free-edge face) of the half bore; `n_radial` the
/// radial count; `grade` the radial power-law grading (>1 clusters toward
/// the bore).
#[derive(Debug, Clone, Copy)]
pub struct MeshSpec {
    pub n_radial: usize,
    pub n_arc: [usize; 3],
    pub grade: f64,
}

impl Default for MeshSpec {
    fn default() -> Self {
        Self { n_radial: 16, n_arc: [4, 6, 18], grade: 2.0 }
    }
}

/// Which unit load case a solution belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnitCase {
    /// Uniform 1 psi on the bore.
    Fit = 0,
    /// Cosine pin load, loaded half of the bore only, resultant 1 lbf.
    PinHalf = 1,
    /// Cosine pin load over the whole bore, resultant 1 lbf.
    PinFull = 2,
}

pub struct FemSolution {
    pub(crate) nodes: Vec<[f64; 2]>,
    pub(crate) elems: Vec<[usize; 9]>,
    /// Displacements per unit case, 2 dofs per node.
    u: [Vec<f64>; 3],
    d: [[f64; 3]; 3],
    centre: [f64; 2],
    bore_radius: f64,
    /// Element bounding boxes for point location.
    boxes: Vec<[f64; 4]>,
    // Kept for the elastic-plastic collapse solver (`plastic.rs`), which
    // re-uses the elastic factorisation as its iteration matrix.
    pub(crate) k_factor: BandedSpd,
    /// Unit load vectors (fit 1 psi, half pin 1 lbf, full pin 1 lbf).
    pub(crate) loads: [Vec<f64>; 3],
    pub(crate) constrained: Vec<bool>,
    /// True material constants (the in-plane stiffness used for assembly
    /// differs under plane strain).
    pub(crate) e_psi: f64,
    pub(crate) nu: f64,
    pub(crate) plane_strain: bool,
}

#[inline]
fn shape1(xi: f64) -> ([f64; 3], [f64; 3]) {
    (
        [0.5 * xi * (xi - 1.0), 1.0 - xi * xi, 0.5 * xi * (xi + 1.0)],
        [xi - 0.5, -2.0 * xi, xi + 0.5],
    )
}

/// Q9 shape functions and derivatives at `(xi, eta)`; local node index is
/// `3*b + a` with `a` along xi (radial) and `b` along eta (angular).
#[inline]
pub(crate) fn shape_q9(xi: f64, eta: f64) -> ([f64; 9], [f64; 9], [f64; 9]) {
    let (nx, dx) = shape1(xi);
    let (ny, dy) = shape1(eta);
    let mut n = [0.0; 9];
    let mut dn_dxi = [0.0; 9];
    let mut dn_deta = [0.0; 9];
    for b in 0..3 {
        for a in 0..3 {
            let k = 3 * b + a;
            n[k] = nx[a] * ny[b];
            dn_dxi[k] = dx[a] * ny[b];
            dn_deta[k] = nx[a] * dy[b];
        }
    }
    (n, dn_dxi, dn_deta)
}

pub(crate) const GAUSS3: [(f64, f64); 3] = [(-0.774_596_669_241_483_4, 5.0 / 9.0), (0.0, 8.0 / 9.0), (0.774_596_669_241_483_4, 5.0 / 9.0)];

/// Distance from `(e, 0)` along direction `phi` to the plate boundary
/// (free edge `x = 0`, far face `x = e + far`, top face `y = h`).
fn ray_to_boundary(e: f64, far: f64, h: f64, phi: f64) -> f64 {
    let (c, s) = (phi.cos(), phi.sin());
    let mut t = f64::INFINITY;
    if c > 1e-12 {
        t = t.min(far / c);
    }
    if c < -1e-12 {
        t = t.min(e / -c);
    }
    if s > 1e-12 {
        t = t.min(h / s);
    }
    t
}

/// Angular lattice (2 lattice points per element) over `[0, pi]`, split at
/// the plate's corner rays so element edges coincide with the far / top /
/// free-edge faces.
fn angular_lattice(e: f64, far: f64, h: f64, n_arc: [usize; 3]) -> Vec<f64> {
    let phi_far = h.atan2(far);
    let phi_top = h.atan2(-e);
    let arcs = [(0.0, phi_far, n_arc[0]), (phi_far, phi_top, n_arc[1]), (phi_top, PI, n_arc[2])];
    let mut phis = vec![0.0];
    for (lo, hi, n) in arcs {
        let n = n.max(1);
        for k in 1..=(2 * n) {
            phis.push(lo + (hi - lo) * k as f64 / (2 * n) as f64);
        }
    }
    *phis.last_mut().unwrap() = PI;
    phis
}

impl FemSolution {
    pub fn solve(geom: &Geometry, e_psi: f64, nu: f64, spec: MeshSpec) -> Result<Self, String> {
        Self::solve_with(geom, e_psi, nu, spec, false)
    }

    /// `plane_strain = true` models a thick plate (`eps_zz = 0`): the in-plane
    /// problem is the plane-stress one with `E' = E/(1-nu^2)`,
    /// `nu' = nu/(1-nu)`; `e_psi`/`nu` are always the true material constants.
    pub fn solve_with(geom: &Geometry, e_true: f64, nu_true: f64, spec: MeshSpec, plane_strain: bool) -> Result<Self, String> {
        let (e_psi, nu) = if plane_strain { (e_true / (1.0 - nu_true * nu_true), nu_true / (1.0 - nu_true)) } else { (e_true, nu_true) };
        let a = geom.bore_radius;
        let e = geom.edge;
        if !(a > 0.0 && e > a && geom.thickness > 0.0 && geom.plate_far > a && geom.plate_half_height > a) {
            return Err("degenerate plate geometry".to_string());
        }
        let (far, h) = (geom.plate_far, geom.plate_half_height);
        let phis = angular_lattice(e, far, h, spec.n_arc);
        let nb = phis.len(); // lattice rows in the angular direction
        let na = 2 * spec.n_radial + 1; // lattice columns in the radial direction
        let n_theta_el = (nb - 1) / 2;
        let n_node = na * nb;

        let mut nodes = vec![[0.0; 2]; n_node];
        for (ib, &phi) in phis.iter().enumerate() {
            let t_out = ray_to_boundary(e, far, h, phi);
            // Element boundaries graded by a power law; midside nodes sit at
            // the element midpoint (a quarter-point or off-centre midside
            // node makes the Jacobian vanish or go negative at the bore).
            let grade_at = |i: usize| (i as f64 / spec.n_radial as f64).powf(spec.grade);
            for ia in 0..na {
                let s = if ia % 2 == 0 { grade_at(ia / 2) } else { 0.5 * (grade_at(ia / 2) + grade_at(ia / 2 + 1)) };
                let r = a + (t_out - a) * s;
                nodes[ib * na + ia] = [e + r * phi.cos(), r * phi.sin()];
            }
        }
        // The load line is a symmetry axis: exact zeros on y = 0.
        for node in nodes.iter_mut() {
            if node[1].abs() < 1e-12 * e {
                node[1] = 0.0;
            }
        }

        let mut elems = Vec::with_capacity(spec.n_radial * n_theta_el);
        for j in 0..n_theta_el {
            for i in 0..spec.n_radial {
                let mut conn = [0usize; 9];
                for b in 0..3 {
                    for aa in 0..3 {
                        conn[3 * b + aa] = (2 * j + b) * na + (2 * i + aa);
                    }
                }
                elems.push(conn);
            }
        }

        let d_mat = {
            let f = e_psi / (1.0 - nu * nu);
            [[f, f * nu, 0.0], [f * nu, f, 0.0], [0.0, 0.0, f * (1.0 - nu) / 2.0]]
        };

        // Half-bandwidth in dofs: an element spans 3 lattice rows.
        let bw = 2 * (2 * na + 3) + 1;
        let ndof = 2 * n_node;
        let mut k = BandedSpd::zeros(ndof, bw.min(ndof - 1));
        let mut boxes = Vec::with_capacity(elems.len());

        for conn in &elems {
            let xy: Vec<[f64; 2]> = conn.iter().map(|&n| nodes[n]).collect();
            let mut ke = [[0.0f64; 18]; 18];
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
                        return Err("inverted element".to_string());
                    }
                    let inv = [[j22 / det, -j12 / det], [-j21 / det, j11 / det]];
                    let mut bm = [[0.0f64; 18]; 3];
                    for n in 0..9 {
                        let dnx = inv[0][0] * dxi[n] + inv[0][1] * deta[n];
                        let dny = inv[1][0] * dxi[n] + inv[1][1] * deta[n];
                        bm[0][2 * n] = dnx;
                        bm[1][2 * n + 1] = dny;
                        bm[2][2 * n] = dny;
                        bm[2][2 * n + 1] = dnx;
                    }
                    let wgt = det * wx * wy;
                    for r in 0..3 {
                        let mut db = [0.0; 18];
                        for c in 0..18 {
                            db[c] = d_mat[r][0] * bm[0][c] + d_mat[r][1] * bm[1][c] + d_mat[r][2] * bm[2][c];
                        }
                        for i in 0..18 {
                            let bi = bm[r][i] * wgt;
                            if bi == 0.0 {
                                continue;
                            }
                            for jj in 0..18 {
                                ke[i][jj] += bi * db[jj];
                            }
                        }
                    }
                }
            }
            for i in 0..18 {
                let gi = 2 * conn[i / 2] + i % 2;
                for jj in 0..=i {
                    let gj = 2 * conn[jj / 2] + jj % 2;
                    k.add(gi, gj, ke[i][jj]);
                }
            }
            let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
            for p in &xy {
                x0 = x0.min(p[0]);
                x1 = x1.max(p[0]);
                y0 = y0.min(p[1]);
                y1 = y1.max(p[1]);
            }
            boxes.push([x0, y0, x1, y1]);
        }

        // Loads (per unit thickness: tractions are force per unit area x 1).
        let mut rhs: [Vec<f64>; 3] = [vec![0.0; ndof], vec![0.0; ndof], vec![0.0; ndof]];
        let cx = e;
        // Bore edge (radial lattice index 0): traction t = p(phi) * e_r on
        // the housing, per unit thickness, per unit pin resultant (lbf, so
        // the per-thickness resultant is 1/t).
        let per_t = 1.0 / geom.thickness;
        for j in 0..n_theta_el {
            let ids = [(2 * j) * na, (2 * j + 1) * na, (2 * j + 2) * na];
            for &(g, w) in &GAUSS3 {
                let (n1, d1) = shape1(g);
                let (mut x, mut y, mut dx, mut dy) = (0.0, 0.0, 0.0, 0.0);
                for q in 0..3 {
                    x += n1[q] * nodes[ids[q]][0];
                    y += n1[q] * nodes[ids[q]][1];
                    dx += d1[q] * nodes[ids[q]][0];
                    dy += d1[q] * nodes[ids[q]][1];
                }
                let jac = (dx * dx + dy * dy).sqrt();
                let phi = y.atan2(x - cx);
                let (c, s) = (phi.cos(), phi.sin());
                // Fit: 1 psi radial. Pin: cosine about the loaded point
                // (phi = pi): p = p0 * cos(phi - pi) = -p0 cos(phi).
                //   Half: loaded half only, resultant 1 lbf -> p0 = 2/(pi a) per unit t.
                //   Full: whole bore, resultant 1 lbf       -> p0 = 1/(pi a) per unit t.
                let cos_load = -c;
                let p_half = if cos_load > 0.0 { 2.0 / (PI * a) * cos_load * per_t } else { 0.0 };
                let p_full = 1.0 / (PI * a) * cos_load * per_t;
                for (case, p) in [(0usize, 1.0), (1, p_half), (2, p_full)] {
                    for q in 0..3 {
                        let f = n1[q] * p * jac * w;
                        rhs[case][2 * ids[q]] += f * c;
                        rhs[case][2 * ids[q] + 1] += f * s;
                    }
                }
            }
        }
        // Far-face reaction of the pin resultant: traction +x of
        // (1 lbf / t) / (2h) over the full face height (half model: P/2 over h).
        let sigma_far = per_t / (2.0 * h);
        let far_x = e + far;
        for j in 0..n_theta_el {
            let ids = [(2 * j) * na + (na - 1), (2 * j + 1) * na + (na - 1), (2 * j + 2) * na + (na - 1)];
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
                        let f = n1[q] * sigma_far * jac * w;
                        rhs[1][2 * ids[q]] += f;
                        rhs[2][2 * ids[q]] += f;
                    }
                }
            }
        }

        // Constraints: symmetry (u_y = 0 on y = 0) and one u_x pin on the
        // symmetry axis at the far face. Penalty keeps the band intact.
        let max_diag = (0..ndof).map(|i| k.diag(i)).fold(0.0, f64::max);
        let penalty = 1e9 * max_diag;
        let mut constrained = vec![false; ndof];
        for (n, p) in nodes.iter().enumerate() {
            if p[1] == 0.0 {
                k.add(2 * n + 1, 2 * n + 1, penalty);
                constrained[2 * n + 1] = true;
            }
        }
        let anchor = na - 1; // lattice row 0 (phi = 0), last radial column: far face on y = 0
        k.add(2 * anchor, 2 * anchor, penalty);
        constrained[2 * anchor] = true;

        if !k.factor() {
            return Err("stiffness matrix is not positive definite".to_string());
        }
        let loads = rhs.clone();
        for r in rhs.iter_mut() {
            k.solve_in_place(r);
        }
        let [u0, u1, u2] = rhs;
        Ok(Self { nodes, elems, u: [u0, u1, u2], d: d_mat, centre: [cx, 0.0], bore_radius: a, boxes, k_factor: k, loads, constrained, e_psi: e_true, nu: nu_true, plane_strain })
    }

    /// Stress at `(x, y)` for unit case `case`, averaged over every element
    /// containing the point (so a point on an element edge is the mean of
    /// both sides). `None` if the point is outside the mesh. The solution
    /// is symmetric about `y = 0`; negative `y` is mirrored (`xy` flips).
    pub fn stress_at(&self, case: UnitCase, x: f64, y: f64) -> Option<Stress> {
        let (y, flip) = if y < 0.0 { (-y, -1.0) } else { (y, 1.0) };
        let u = &self.u[case as usize];
        let tol = 1e-9 * (1.0 + x.abs() + y.abs());
        // Elements containing the point (to 1e-7 in local coordinates);
        // failing that the nearest one within 2% of an element (the bore
        // edge of a Q9 element is a quadratic through three on-circle
        // nodes, so a point exactly on the true circle can sit a hair
        // outside the element edge).
        let mut acc = Stress::ZERO;
        let mut count = 0.0;
        let mut nearest: Option<(f64, usize, f64, f64)> = None;
        for (ie, conn) in self.elems.iter().enumerate() {
            let b = &self.boxes[ie];
            let pad = tol + 0.02 * (b[2] - b[0]).max(b[3] - b[1]);
            if x < b[0] - pad || x > b[2] + pad || y < b[1] - pad || y > b[3] + pad {
                continue;
            }
            if let Some((xi, eta, excess)) = self.locate(conn, x, y) {
                if excess <= 1e-7 {
                    acc = acc.plus(self.stress_in(conn, u, xi.clamp(-1.0, 1.0), eta.clamp(-1.0, 1.0)));
                    count += 1.0;
                } else if excess <= 0.02 && nearest.is_none_or(|n| excess < n.0) {
                    nearest = Some((excess, ie, xi, eta));
                }
            }
        }
        if count == 0.0 {
            let (_, ie, xi, eta) = nearest?;
            acc = self.stress_in(&self.elems[ie], u, xi.clamp(-1.0, 1.0), eta.clamp(-1.0, 1.0));
            count = 1.0;
        }
        let s = acc.scaled(1.0 / count);
        Some(Stress { xx: s.xx, yy: s.yy, xy: s.xy * flip })
    }

    /// Newton inverse isoparametric map. Returns the converged local
    /// coordinates and how far outside the element they lie
    /// (`max(|xi|,|eta|) - 1`, negative inside). `None` if Newton does not
    /// converge.
    fn locate(&self, conn: &[usize; 9], x: f64, y: f64) -> Option<(f64, f64, f64)> {
        let (mut xi, mut eta) = (0.0f64, 0.0f64);
        let mut converged = false;
        for _ in 0..40 {
            let (n, dxi, deta) = shape_q9(xi, eta);
            let (mut px, mut py, mut j11, mut j12, mut j21, mut j22) = (0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
            for k in 0..9 {
                let p = self.nodes[conn[k]];
                px += n[k] * p[0];
                py += n[k] * p[1];
                j11 += dxi[k] * p[0];
                j12 += dxi[k] * p[1];
                j21 += deta[k] * p[0];
                j22 += deta[k] * p[1];
            }
            let (rx, ry) = (x - px, y - py);
            let det = j11 * j22 - j12 * j21;
            if det.abs() < 1e-300 {
                return None;
            }
            let dxi_step = (rx * j22 - ry * j21) / det;
            let deta_step = (-rx * j12 + ry * j11) / det;
            xi += dxi_step;
            eta += deta_step;
            if xi.abs() > 4.0 || eta.abs() > 4.0 {
                return None;
            }
            if dxi_step.abs() < 1e-12 && deta_step.abs() < 1e-12 {
                converged = true;
                break;
            }
        }
        converged.then_some((xi, eta, xi.abs().max(eta.abs()) - 1.0))
    }

    fn stress_in(&self, conn: &[usize; 9], u: &[f64], xi: f64, eta: f64) -> Stress {
        let (_, dxi, deta) = shape_q9(xi, eta);
        let (mut j11, mut j12, mut j21, mut j22) = (0.0, 0.0, 0.0, 0.0);
        for k in 0..9 {
            let p = self.nodes[conn[k]];
            j11 += dxi[k] * p[0];
            j12 += dxi[k] * p[1];
            j21 += deta[k] * p[0];
            j22 += deta[k] * p[1];
        }
        let det = j11 * j22 - j12 * j21;
        let inv = [[j22 / det, -j12 / det], [-j21 / det, j11 / det]];
        let (mut exx, mut eyy, mut gxy) = (0.0, 0.0, 0.0);
        for k in 0..9 {
            let dnx = inv[0][0] * dxi[k] + inv[0][1] * deta[k];
            let dny = inv[1][0] * dxi[k] + inv[1][1] * deta[k];
            let (ux, uy) = (u[2 * conn[k]], u[2 * conn[k] + 1]);
            exx += dnx * ux;
            eyy += dny * uy;
            gxy += dny * ux + dnx * uy;
        }
        let d = &self.d;
        Stress {
            xx: d[0][0] * exx + d[0][1] * eyy,
            yy: d[1][0] * exx + d[1][1] * eyy,
            xy: d[2][2] * gxy,
        }
    }

    /// Elastic displacement field of a unit case (for the plastic solver's
    /// jump to first yield).
    pub(crate) fn u_unit(&self, case: usize) -> &Vec<f64> {
        &self.u[case]
    }

    /// Index of the node at the bore's loaded point (`phi = pi`, the point
    /// nearest the free edge on the load line).
    pub(crate) fn loaded_point_node(&self) -> usize {
        let target = self.centre[0] - self.bore_radius;
        (0..self.nodes.len())
            .filter(|&n| self.nodes[n][1] == 0.0)
            .min_by(|&a, &b| (self.nodes[a][0] - target).abs().total_cmp(&(self.nodes[b][0] - target).abs()))
            .expect("the load line has nodes")
    }

    pub fn bore_radius(&self) -> f64 {
        self.bore_radius
    }

    pub fn centre(&self) -> [f64; 2] {
        self.centre
    }

    pub fn element_count(&self) -> usize {
        self.elems.len()
    }

    pub fn dof_count(&self) -> usize {
        2 * self.nodes.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn big_plate(e: f64) -> Geometry {
        Geometry { bore_radius: 0.25, edge: e, thickness: 0.5, plate_far: 40.0 * 0.25, plate_half_height: 40.0 * 0.25, plane_angle_deg: 40.0 }
    }

    #[test]
    fn uniform_bore_pressure_in_a_large_plate_reproduces_lame_at_the_bore() {
        // Infinite plate, hole pressure p: sigma_theta(a) = +p, sigma_r(a) = -p.
        let g = big_plate(30.0 * 0.25);
        let sol = FemSolution::solve(&g, 10.3e6, 0.33, MeshSpec::default()).unwrap();
        let [cx, _] = sol.centre();
        let a = g.bore_radius;
        // Away from the free edge: bore point at phi = 90deg (top).
        let s = sol.stress_at(UnitCase::Fit, cx, a).unwrap();
        // At phi = 90deg: sigma_xx is the hoop stress, sigma_yy the radial.
        assert!((s.xx - 1.0).abs() < 0.03, "hoop {s:?}");
        assert!((s.yy + 1.0).abs() < 0.03, "radial {s:?}");
    }

    #[test]
    fn the_pin_load_resultant_equals_the_applied_load_across_a_cut() {
        // Equilibrium: integrate sigma_xx * t across x = const between the
        // bore and the far face; resultant must equal the 1 lbf pin load.
        let g = big_plate(4.0 * 0.25);
        let sol = FemSolution::solve(&g, 10.3e6, 0.33, MeshSpec::default()).unwrap();
        let [cx, _] = sol.centre();
        let x_cut = cx + 0.8 * g.plate_far;
        let n = 400;
        let mut f = 0.0;
        for k in 0..n {
            let y = (k as f64 + 0.5) / n as f64 * g.plate_half_height;
            f += sol.stress_at(UnitCase::PinHalf, x_cut, y).unwrap().xx * (g.plate_half_height / n as f64);
        }
        let total = 2.0 * f * g.thickness; // both halves
        assert!((total - 1.0).abs() < 0.02, "cut resultant {total}");
    }

    #[test]
    fn refining_the_mesh_changes_the_peak_bore_stress_only_slightly() {
        let g = big_plate(3.0 * 0.25);
        let coarse = FemSolution::solve(&g, 10.3e6, 0.33, MeshSpec::default()).unwrap();
        let fine = FemSolution::solve(&g, 10.3e6, 0.33, MeshSpec { n_radial: 24, n_arc: [6, 9, 27], grade: 2.0 }).unwrap();
        let [cx, _] = coarse.centre();
        let a = g.bore_radius;
        let (sc, sf) = (coarse.stress_at(UnitCase::PinHalf, cx - a, 0.0).unwrap(), fine.stress_at(UnitCase::PinHalf, cx - a, 0.0).unwrap());
        assert!((sc.yy - sf.yy).abs() / sf.yy.abs() < 0.03, "{sc:?} vs {sf:?}");
    }
}
