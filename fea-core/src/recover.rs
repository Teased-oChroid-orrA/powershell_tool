//! Superconvergent patch recovery (Zienkiewicz-Zhu SPR) of nodal stress and the ZZ error estimate.
//!
//! Each corner node's polynomial (complete degree 1 for linear elements, 2 for quadratic ones) is
//! the least-squares fit of the Gauss-point stresses of the elements around it, in coordinates
//! scaled to the patch. A corner node whose patch is too small or degenerate (a domain corner, a
//! one-element strip) borrows the polynomial of an edge-adjacent well-posed node. Midside, face and
//! interior nodes average the polynomials of the corner nodes of the elements that own them.

#![allow(clippy::needless_range_loop)] // index-parallel numeric kernels read clearer as loops

use crate::analysis::{GaussStress, Model};
use crate::element::ElementKind;
use crate::mesh::Elastic;
use rayon::prelude::*;

/// Node count from which the per-node stages of the recovery run in parallel. Every node's result depends only on
/// read-only data, so the output is bit-identical for any thread count.
const PARALLEL_NODES: usize = 2000;

fn per_node<T: Send>(n: usize, f: impl Fn(usize) -> T + Sync + Send) -> Vec<T> {
    if n >= PARALLEL_NODES {
        (0..n).into_par_iter().map(f).collect()
    } else {
        (0..n).map(f).collect()
    }
}

/// Number of polynomial terms of complete degree `p` in `d` variables.
fn n_terms(d: usize, p: usize) -> usize {
    match (d, p) {
        (_, 0) => 1,
        (2, 1) => 3,
        (2, _) => 6,
        (_, 1) => 4,
        _ => 10,
    }
}

fn terms(d: usize, p: usize, x: [f64; 3], out: &mut [f64]) {
    out[0] = 1.0;
    if p >= 1 {
        out[1..=d].copy_from_slice(&x[..d]);
    }
    if p >= 2 {
        let mut k = d + 1;
        for i in 0..d {
            for j in i..d {
                out[k] = x[i] * x[j];
                k += 1;
            }
        }
    }
}

/// Fit polynomial of degree `p` per stress component to `samples` about `centre`; `None` if the
/// samples do not determine it.
struct Fit {
    centre: [f64; 3],
    scale: f64,
    degree: usize,
    /// `coef[term][comp]` for the first `n_terms(d, degree)` terms (at most 10).
    coef: [[f64; 6]; 10],
    nt: usize,
}

fn fit(d: usize, p: usize, centre: [f64; 3], samples: &[&GaussStress]) -> Option<Fit> {
    let nt = n_terms(d, p);
    // Require an over-determined fit: with exactly `nt` samples the polynomial interpolates the
    // data and extrapolates noise (a two-triangle Tri6 patch has 6 samples for 6 unknowns).
    if samples.len() < nt + 2 {
        return None;
    }
    let scale = samples.iter().map(|s| (0..d).map(|i| (s.0[i] - centre[i]).powi(2)).sum::<f64>().sqrt()).fold(0.0f64, f64::max);
    if scale <= 0.0 {
        return None;
    }
    let mut a = [0.0; 100];
    let mut rhs = [[0.0f64; 6]; 10];
    let mut t = [0.0; 10];
    for s in samples {
        let x: [f64; 3] = std::array::from_fn(|i| if i < d { (s.0[i] - centre[i]) / scale } else { 0.0 });
        terms(d, p, x, &mut t);
        for i in 0..nt {
            for j in 0..nt {
                a[i * nt + j] += t[i] * t[j];
            }
            for c in 0..6 {
                rhs[i][c] += t[i] * s.1[c];
            }
        }
    }
    // Cholesky with a conditioning check (a collinear or too-small sample set shows as a tiny pivot).
    let mut l = [0.0; 100];
    let dmax = (0..nt).map(|i| a[i * nt + i]).fold(0.0f64, f64::max);
    for i in 0..nt {
        for j in 0..=i {
            let mut s = a[i * nt + j];
            for k in 0..j {
                s -= l[i * nt + k] * l[j * nt + k];
            }
            if i == j {
                if s <= 1e-9 * dmax {
                    return None;
                }
                l[i * nt + i] = s.sqrt();
            } else {
                l[i * nt + j] = s / l[j * nt + j];
            }
        }
    }
    let mut coef = [[0.0f64; 6]; 10];
    for c in 0..6 {
        let mut y = [0.0; 10];
        for i in 0..nt {
            let mut s = rhs[i][c];
            for k in 0..i {
                s -= l[i * nt + k] * y[k];
            }
            y[i] = s / l[i * nt + i];
        }
        for i in (0..nt).rev() {
            let mut s = y[i];
            for k in i + 1..nt {
                s -= l[k * nt + i] * coef[k][c];
            }
            coef[i][c] = s / l[i * nt + i];
        }
    }
    Some(Fit { centre, scale, degree: p, coef, nt })
}

impl Fit {
    fn eval(&self, d: usize, x: [f64; 3]) -> [f64; 6] {
        let xs: [f64; 3] = std::array::from_fn(|i| if i < d { (x[i] - self.centre[i]) / self.scale } else { 0.0 });
        let mut t = [0.0; 10];
        terms(d, self.degree, xs, &mut t);
        let mut out = [0.0; 6];
        for (k, c) in self.coef[..self.nt].iter().enumerate() {
            for q in 0..6 {
                out[q] += t[k] * c[q];
            }
        }
        out
    }
}

fn poly_degree(kind: ElementKind) -> usize {
    if kind.n_nodes() > kind.n_corners() {
        2
    } else {
        1
    }
}

/// Complementary strain energy density `1/2 sigma : C^-1 : sigma` of a stress in the library order.
pub fn complementary_energy(mat: &Elastic, s: &[f64; 6]) -> f64 {
    if let Some(an) = &mat.aniso {
        let mut w = 0.0;
        for i in 0..6 {
            for j in 0..6 {
                w += s[i] * an.c[i][j] * s[j];
            }
        }
        return 0.5 * w;
    }
    let tr = s[0] + s[1] + s[2];
    let sq = s[0] * s[0] + s[1] * s[1] + s[2] * s[2] + 2.0 * (s[3] * s[3] + s[4] * s[4] + s[5] * s[5]);
    0.5 * ((1.0 + mat.nu) * sq - mat.nu * tr * tr) / mat.e
}

/// ZZ error estimate.
#[derive(Debug, Clone)]
pub struct ZzEstimate {
    /// Estimated energy-norm error of every element (block order).
    pub eta: Vec<f64>,
    /// `sqrt(sum eta^2)`.
    pub total: f64,
    /// Energy norm of the recovered stress `sqrt(int sigma* : C^-1 : sigma*)`.
    pub norm: f64,
}

impl ZzEstimate {
    /// Relative error `eta / sqrt(eta^2 + |sigma*|^2)`.
    pub fn relative(&self) -> f64 {
        if !self.total.is_finite() || !self.norm.is_finite() || self.total < 0.0 || self.norm < 0.0 { return f64::NAN; }
        let denominator = self.total.hypot(self.norm);
        if denominator == 0.0 { 0.0 } else { self.total / denominator }
    }
}

/// How the corner nodes of a stress recovery were resolved (`recover_spr_diag`): a node whose patch is too small or degenerate
/// borrows a neighbour's polynomial, and in the worst case gets the plain average of its Gauss-point stresses.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SprDiagnostics {
    pub corner_nodes: usize,
    /// Own over-determined polynomial.
    pub fitted: usize,
    /// Polynomial of an edge-adjacent well-posed node (boundary nodes and under-determined patches).
    pub borrowed: usize,
    /// Own patch fitted at a lower degree.
    pub lower_degree: usize,
    /// Plain average: no polynomial was determined at all.
    pub averaged: usize,
}

impl SprDiagnostics {
    /// Fraction of corner nodes that fell back to the plain average (an unreliable stress).
    pub fn averaged_fraction(&self) -> f64 {
        if self.corner_nodes == 0 { 0.0 } else { self.averaged as f64 / self.corner_nodes as f64 }
    }

    fn add(&mut self, o: &SprDiagnostics) {
        self.corner_nodes += o.corner_nodes;
        self.fitted += o.fitted;
        self.borrowed += o.borrowed;
        self.lower_degree += o.lower_degree;
        self.averaged += o.averaged;
    }
}

/// Recovered nodal stress with one patch family per material: the stress is discontinuous across a material interface, so a
/// node shared by elements of different materials holds a different recovered value for each material. For a mesh of one
/// material this is exactly [`Model::recover_spr`].
#[derive(Debug, Clone)]
pub struct SprRecovery {
    group_of_block: Vec<usize>,
    groups: Vec<Vec<[f64; 6]>>,
    /// Nodes shared by blocks of different materials (where the recovery is one-sided).
    pub interface_nodes: usize,
    pub diagnostics: SprDiagnostics,
}

impl SprRecovery {
    /// Recovered stress at `node` as seen from the elements of `block`.
    pub fn at(&self, block: usize, node: usize) -> [f64; 6] {
        self.groups[self.group_of_block[block]][node]
    }
}

use crate::element::gauss_legendre_unit;

/// Quadrature for the error integrals: richer than the stiffness rule. The stiffness Gauss points are (nearly) the points where
/// the finite-element stress is superconvergent, so a recovered-minus-finite-element difference sampled there misses the error
/// between them (an eigenvalue test showed an energy error five to seven times too small). Points in natural coordinates and
/// weights summing to the reference measure of the element.
fn error_rule(kind: ElementKind) -> Vec<([f64; 3], f64)> {
    let quadratic = kind.n_nodes() > kind.n_corners();
    let n = if quadratic { 5 } else { 4 };
    let gl = gauss_legendre_unit(n);
    let mut out = Vec::new();
    match kind {
        ElementKind::Quad4 | ElementKind::Quad8 | ElementKind::Quad9 => {
            for &(a, wa) in &gl {
                for &(b, wb) in &gl {
                    out.push(([2.0 * a - 1.0, 2.0 * b - 1.0, 0.0], 4.0 * wa * wb));
                }
            }
        }
        ElementKind::Hex8 | ElementKind::Hex20 | ElementKind::Hex27 => {
            let gl = gauss_legendre_unit(n - 1);
            for &(a, wa) in &gl {
                for &(b, wb) in &gl {
                    for &(c, wc) in &gl {
                        out.push(([2.0 * a - 1.0, 2.0 * b - 1.0, 2.0 * c - 1.0], 8.0 * wa * wb * wc));
                    }
                }
            }
        }
        ElementKind::Tri3 | ElementKind::Tri6 => {
            for &(u, wu) in &gl {
                for &(v, wv) in &gl {
                    out.push(([u * (1.0 - v), v, 0.0], wu * wv * (1.0 - v)));
                }
            }
        }
        ElementKind::Tet4 | ElementKind::Tet10 => {
            let gl = gauss_legendre_unit(n - 1);
            for &(a, wa) in &gl {
                for &(b, wb) in &gl {
                    for &(c, wc) in &gl {
                        out.push(([a * (1.0 - b) * (1.0 - c), b * (1.0 - c), c], wa * wb * wc * (1.0 - b) * (1.0 - c) * (1.0 - c)));
                    }
                }
            }
        }
    }
    out
}

/// One point of the error rule inside an element: shape functions, physical weight (Jacobian, thickness or `2 pi r` included).
struct ErrorPoint {
    n: Vec<f64>,
    weight: f64,
    xi: [f64; 3],
}

fn error_points(kind: ElementKind, physics: crate::mesh::Physics, xyz: &[[f64; 3]]) -> Result<Vec<ErrorPoint>, String> {
    let (nn, d) = (kind.n_nodes(), kind.dim());
    let scale = match physics {
        crate::mesh::Physics::PlaneStress { thickness } | crate::mesh::Physics::PlaneStrain { thickness } => thickness,
        _ => 1.0,
    };
    let mut out = Vec::new();
    for (xi, wref) in error_rule(kind) {
        let (n, dn) = kind.shape(xi);
        let mut jac = [[0.0f64; 3]; 3];
        for a in 0..nn {
            for i in 0..d {
                for k in 0..d {
                    jac[i][k] += xyz[a][i] * dn[a][k];
                }
            }
        }
        let det = if d == 2 {
            jac[0][0] * jac[1][1] - jac[0][1] * jac[1][0]
        } else {
            jac[0][0] * (jac[1][1] * jac[2][2] - jac[1][2] * jac[2][1]) - jac[0][1] * (jac[1][0] * jac[2][2] - jac[1][2] * jac[2][0]) + jac[0][2] * (jac[1][0] * jac[2][1] - jac[1][1] * jac[2][0])
        };
        if det.is_nan() || det <= 0.0 || !det.is_finite() {
            return Err(format!("inverted or degenerate element (Jacobian {det:.3e} in the error quadrature)"));
        }
        let mut w = wref * det * scale;
        if matches!(physics, crate::mesh::Physics::Axisymmetric) {
            let r: f64 = (0..nn).map(|a| n[a] * xyz[a][0]).sum();
            if r.is_nan() || r <= 0.0 {
                return Err(format!("axisymmetric element with radius {r:.3e} in the error quadrature"));
            }
            w *= 2.0 * std::f64::consts::PI * r;
        }
        out.push(ErrorPoint { n: n[..nn].to_vec(), weight: w, xi });
    }
    Ok(out)
}

impl Model {
    /// SPR nodal stresses from the Gauss-point stresses (`gauss_stresses` output). Patches span every element around a node
    /// whatever its material: use [`recover_spr_by_material`](Self::recover_spr_by_material) for a mesh with several materials.
    pub fn recover_spr(&self, gauss: &[Vec<GaussStress>]) -> Result<Vec<[f64; 6]>, String> {
        self.recover_spr_diag(gauss).map(|r| r.0)
    }

    /// [`recover_spr`](Self::recover_spr) with a count of how each corner node was resolved.
    pub fn recover_spr_diag(&self, gauss: &[Vec<GaussStress>]) -> Result<(Vec<[f64; 6]>, SprDiagnostics), String> {
        let d = self.mesh.dim();
        let n = self.mesh.nodes.len();
        // Elements around each node: (block, element).
        let mut around: Vec<Vec<(usize, usize)>> = vec![Vec::new(); n];
        let mut corner = vec![false; n];
        let mut degree = vec![0usize; n];
        for (bi, blk) in self.mesh.blocks.iter().enumerate() {
            let (nc, p) = (blk.kind.n_corners(), poly_degree(blk.kind));
            for e in 0..blk.n_elems() {
                for (a, &nd) in blk.elem(e).iter().enumerate() {
                    around[nd].push((bi, e));
                    if a < nc {
                        corner[nd] = true;
                        degree[nd] = degree[nd].max(p);
                    }
                }
            }
        }
        let samples_of = |nd: usize| -> Vec<&GaussStress> {
            let mut v = Vec::new();
            for &(bi, e) in &around[nd] {
                let ngp = self.mesh.blocks[bi].kind.table().ngp;
                v.extend(&gauss[bi][e * ngp..(e + 1) * ngp]);
            }
            v
        };
        let fits: Vec<Option<Fit>> = per_node(n, |nd| if corner[nd] { fit(d, degree[nd], self.mesh.nodes[nd], &samples_of(nd)) } else { None });
        // Corner nodes without a well-posed patch: borrow from an edge-adjacent fitted node
        // (iterate outward), else fall back to lower degrees on the node's own patch.
        let mut neigh: Vec<Vec<usize>> = vec![Vec::new(); n];
        for blk in &self.mesh.blocks {
            let nc = blk.kind.n_corners();
            for e in 0..blk.n_elems() {
                let c = &blk.elem(e)[..nc];
                for &a in c {
                    for &b in c {
                        if a != b && !neigh[a].contains(&b) {
                            neigh[a].push(b);
                        }
                    }
                }
            }
        }
        // Boundary nodes: the patch around them is one-sided, so (as in the original SPR) they take the
        // polynomial of the nearest interior edge-neighbour with a well-posed fit when there is one.
        let mut on_boundary = vec![false; n];
        for f in self.mesh.boundary_faces() {
            for &nd in &f.nodes {
                on_boundary[nd] = true;
            }
        }
        let mut borrowed: Vec<Option<usize>> = vec![None; n];
        for nd in 0..n {
            if corner[nd] && on_boundary[nd] {
                let x = self.mesh.nodes[nd];
                let dist = |m: usize| (0..d).map(|i| (self.mesh.nodes[m][i] - x[i]).powi(2)).sum::<f64>();
                borrowed[nd] = neigh[nd].iter().copied().filter(|&m| !on_boundary[m] && fits[m].is_some()).min_by(|&a, &b| dist(a).total_cmp(&dist(b)));
            }
        }
        for _ in 0..4 {
            let mut changed = false;
            for nd in 0..n {
                if corner[nd] && fits[nd].is_none() && borrowed[nd].is_none() {
                    if let Some(&m) = neigh[nd].iter().find(|&&m| fits[m].is_some()) {
                        borrowed[nd] = Some(m);
                        changed = true;
                    }
                }
            }
            if !changed {
                break;
            }
        }
        // Corner nodes: the node's own polynomial, a borrowed one, or a lower-degree fit.
        // Outcome tags: 0 own fit, 1 borrowed polynomial, 2 lower-degree fit, 3 plain average (no polynomial determined).
        let at_corner: Vec<Option<([f64; 6], u8)>> = per_node(n, |nd| {
            if !corner[nd] {
                return None;
            }
            let x = self.mesh.nodes[nd];
            Some(if let (true, Some(m)) = (on_boundary[nd], borrowed[nd]) {
                (fits[m].as_ref().unwrap().eval(d, x), 1)
            } else if let Some(f) = &fits[nd] {
                (f.eval(d, x), 0)
            } else if let Some(m) = borrowed[nd] {
                (fits[m].as_ref().unwrap().eval(d, x), 1)
            } else {
                // Lower-degree fit on the node's own patch; the plain average as the last resort.
                let s = samples_of(nd);
                let low = (0..degree[nd]).rev().find_map(|p| fit(d, p, x, &s));
                match low {
                    Some(f) => (f.eval(d, x), 2),
                    None => {
                        let mut avg = [0.0; 6];
                        for g in &s {
                            for c in 0..6 {
                                avg[c] += g.1[c] / s.len() as f64;
                            }
                        }
                        (avg, 3)
                    }
                }
            })
        });
        let mut diag = SprDiagnostics::default();
        for t in at_corner.iter().flatten() {
            diag.corner_nodes += 1;
            match t.1 {
                0 => diag.fitted += 1,
                1 => diag.borrowed += 1,
                2 => diag.lower_degree += 1,
                _ => diag.averaged += 1,
            }
        }
        // Midside / face / interior nodes: average of the owning elements' corner polynomials at the node.
        let corner_value = |cn: usize, x: [f64; 3]| -> Option<[f64; 6]> {
            match (on_boundary[cn], borrowed[cn], &fits[cn]) {
                (true, Some(m), _) | (_, Some(m), None) => Some(fits[m].as_ref().unwrap().eval(d, x)),
                (_, _, Some(f)) => Some(f.eval(d, x)),
                _ => None,
            }
        };
        let out: Vec<[f64; 6]> = per_node(n, |nd| {
            if let Some(v) = at_corner[nd] {
                return v.0;
            }
            let x = self.mesh.nodes[nd];
            let (mut sum, mut cnt) = ([0.0f64; 6], 0.0);
            for &(bi, e) in &around[nd] {
                let blk = &self.mesh.blocks[bi];
                for &cn in &blk.elem(e)[..blk.kind.n_corners()] {
                    if let Some(v) = corner_value(cn, x) {
                        for c in 0..6 {
                            sum[c] += v[c];
                        }
                        cnt += 1.0;
                    }
                }
            }
            let mut v = [0.0f64; 6];
            if cnt > 0.0 {
                for c in 0..6 {
                    v[c] = sum[c] / cnt;
                }
            }
            v
        });
        Ok((out, diag))
    }

    /// SPR+ recovery: patches never span two materials. The elements of each material (blocks of equal `Elastic`) are
    /// recovered as a model of their own, so at an interface each side is a one-sided boundary patch (the boundary rule of
    /// the original method) instead of an average over the jump.
    pub fn recover_spr_by_material(&self, gauss: &[Vec<GaussStress>]) -> Result<SprRecovery, String> {
        let nb = self.mesh.blocks.len();
        let mut group_of_block = vec![0usize; nb];
        let mut reps: Vec<usize> = Vec::new(); // first block of each material
        for b in 0..nb {
            match reps.iter().position(|&r| self.mesh.blocks[r].material == self.mesh.blocks[b].material) {
                Some(g) => group_of_block[b] = g,
                None => {
                    group_of_block[b] = reps.len();
                    reps.push(b);
                }
            }
        }
        if reps.len() <= 1 {
            let (rec, diag) = self.recover_spr_diag(gauss)?;
            return Ok(SprRecovery { group_of_block, groups: vec![rec], interface_nodes: 0, diagnostics: diag });
        }
        let mut groups = Vec::with_capacity(reps.len());
        let mut diagnostics = SprDiagnostics::default();
        for g in 0..reps.len() {
            let mut mesh = self.mesh.clone();
            mesh.blocks = (0..nb).filter(|&b| group_of_block[b] == g).map(|b| self.mesh.blocks[b].clone()).collect();
            let sub_gauss: Vec<Vec<GaussStress>> = (0..nb).filter(|&b| group_of_block[b] == g).map(|b| gauss[b].clone()).collect();
            let sub = Model { mesh, pattern: self.pattern.clone() };
            let (rec, diag) = sub.recover_spr_diag(&sub_gauss)?;
            diagnostics.add(&diag);
            groups.push(rec);
        }
        let mut first_group = vec![usize::MAX; self.mesh.nodes.len()];
        let mut interface = vec![false; self.mesh.nodes.len()];
        for (b, blk) in self.mesh.blocks.iter().enumerate() {
            for &nd in &blk.conn {
                match first_group[nd] {
                    usize::MAX => first_group[nd] = group_of_block[b],
                    g if g != group_of_block[b] => interface[nd] = true,
                    _ => {}
                }
            }
        }
        Ok(SprRecovery { group_of_block, groups, interface_nodes: interface.iter().filter(|&&i| i).count(), diagnostics })
    }

    /// ZZ error estimate from the displacement field `u`: per-element energy norm of the difference
    /// between the SPR-recovered and the finite-element stress.
    pub fn zz_error(&self, u: &[f64], delta_t: f64) -> Result<ZzEstimate, String> {
        self.zz_error_field(u, delta_t, None)
    }

    /// As [`zz_error`](Self::zz_error) with a nodal temperature field (`Loads::temperature`) on top of `delta_t`.
    pub fn zz_error_field(&self, u: &[f64], delta_t: f64, field: Option<&[f64]>) -> Result<ZzEstimate, String> {
        let gauss = self.gauss_stresses_field(u, delta_t, field)?;
        let rec = self.recover_spr_by_material(&gauss)?;
        let (mut eta, mut norm2) = (Vec::with_capacity(self.mesh.n_elems()), 0.0f64);
        for (bi, blk) in self.mesh.blocks.iter().enumerate() {
            let nn = blk.kind.n_nodes();
            let d = self.mesh.dim();
            // Per element: the error energy and the recovered-stress energy (summed afterwards, in element order), integrated with
            // the error rule at points where both the recovered and the finite-element stress are evaluated.
            let one = |(xyz, ue): &mut (Vec<[f64; 3]>, Vec<f64>), e: usize| -> Result<(f64, f64), String> {
                let conn = blk.elem(e);
                for (a, &nd) in conn.iter().enumerate() {
                    xyz[a] = self.mesh.nodes[nd];
                    for i in 0..d {
                        ue[a * d + i] = u[nd * d + i];
                    }
                }
                let (mut e2, mut n2) = (0.0, 0.0);
                for pt in error_points(blk.kind, self.mesh.physics, xyz)? {
                    let mut star = [0.0; 6];
                    for (a, &nd) in conn.iter().enumerate() {
                        let r = rec.at(bi, nd);
                        for c in 0..6 {
                            star[c] += pt.n[a] * r[c];
                        }
                    }
                    let dt = delta_t + field.map_or(0.0, |tf| conn.iter().enumerate().map(|(a, &nd)| pt.n[a] * tf[nd]).sum::<f64>());
                    let h = crate::analysis::stress_at_xi(blk.kind, self.mesh.physics, &blk.material, xyz, ue, pt.xi, dt)?;
                    let diff: [f64; 6] = std::array::from_fn(|c| star[c] - h[c]);
                    e2 += pt.weight * complementary_energy(&blk.material, &diff) * 2.0;
                    n2 += pt.weight * complementary_energy(&blk.material, &star) * 2.0;
                }
                Ok((e2, n2))
            };
            let init = || (vec![[0.0; 3]; nn], vec![0.0; nn * d]);
            let per_elem: Vec<(f64, f64)> = if blk.n_elems() >= PARALLEL_NODES {
                (0..blk.n_elems()).into_par_iter().map_init(init, |st, e| one(st, e)).collect::<Result<_, _>>()?
            } else {
                let mut st = init();
                (0..blk.n_elems()).map(|e| one(&mut st, e)).collect::<Result<_, _>>()?
            };
            for (e2, n2) in per_elem {
                eta.push(e2.sqrt());
                norm2 += n2;
            }
        }
        let total = eta.iter().map(|x| x * x).sum::<f64>().sqrt();
        Ok(ZzEstimate { eta, total, norm: norm2.sqrt() })
    }

    /// Energy norm `sqrt(int (sigma_exact - sigma_h) : C^-1 : (sigma_exact - sigma_h))` of the true
    /// stress error against a known stress field (verification and convergence studies).
    pub fn stress_error_energy(&self, u: &[f64], delta_t: f64, exact: impl Fn(&[f64; 3]) -> [f64; 6]) -> Result<f64, String> {
        let d = self.mesh.dim();
        let mut sum = 0.0;
        for blk in &self.mesh.blocks {
            let nn = blk.kind.n_nodes();
            let (mut xyz, mut ue) = (vec![[0.0; 3]; nn], vec![0.0; nn * d]);
            for e in 0..blk.n_elems() {
                let conn = blk.elem(e);
                for (a, &nd) in conn.iter().enumerate() {
                    xyz[a] = self.mesh.nodes[nd];
                    for i in 0..d {
                        ue[a * d + i] = u[nd * d + i];
                    }
                }
                for pt in error_points(blk.kind, self.mesh.physics, &xyz)? {
                    let x: [f64; 3] = std::array::from_fn(|k| (0..nn).map(|a| pt.n[a] * xyz[a][k]).sum());
                    let h = crate::analysis::stress_at_xi(blk.kind, self.mesh.physics, &blk.material, &xyz, &ue, pt.xi, delta_t)?;
                    let ex = exact(&x);
                    let diff: [f64; 6] = std::array::from_fn(|c| ex[c] - h[c]);
                    sum += pt.weight * complementary_energy(&blk.material, &diff) * 2.0;
                }
            }
        }
        Ok(sum.sqrt())
    }
}

#[cfg(test)]
mod rule_tests {
    use super::*;
    use crate::element::ALL_KINDS;

    #[test]
    fn the_error_rules_integrate_polynomials_over_every_reference_element() {
        for kind in ALL_KINDS {
            let rule = error_rule(kind);
            let measure: f64 = rule.iter().map(|r| r.1).sum();
            let want = match (kind.is_simplex(), kind.dim()) {
                (true, 2) => 0.5,
                (true, _) => 1.0 / 6.0,
                (false, 2) => 4.0,
                (false, _) => 8.0,
            };
            assert!((measure - want).abs() < 1e-13, "{kind:?}: measure {measure} vs {want}");
            // the squared difference of two fields of the element's order: degree 2 (linear kinds) or 4 (quadratic kinds)
            let quadratic = kind.n_nodes() > kind.n_corners();
            let p = if quadratic { 2 } else { 1 };
            let got: f64 = rule.iter().map(|(x, w)| w * x[0].powi(p) * x[1].powi(p)).sum();
            let exact = match (kind.is_simplex(), kind.dim(), quadratic) {
                (true, 2, false) => 1.0 / 24.0,           // int x y over the unit triangle = 1!1!/4!
                (true, 2, true) => 4.0 / 720.0,           // 2!2!/6!
                (true, _, false) => 1.0 / 120.0,          // 1!1!/5!
                (true, _, true) => 4.0 / 5040.0,          // 2!2!/7!
                (false, 2, false) => 0.0,                 // odd in x
                (false, 2, true) => 4.0 / 9.0,            // (2/3)^2
                (false, _, false) => 0.0,
                (false, _, true) => 8.0 / 9.0,            // (2/3)(2/3)(2)
            };
            assert!((got - exact).abs() < 1e-12, "{kind:?}: {got} vs {exact}");
        }
    }
}
