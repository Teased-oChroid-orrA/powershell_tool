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
use crate::kernel::{self, Work};
use crate::mesh::Elastic;

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
    /// `coef[term][comp]`.
    coef: Vec<[f64; 6]>,
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
    let mut a = vec![0.0; nt * nt];
    let mut rhs = vec![[0.0f64; 6]; nt];
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
    let mut l = vec![0.0; nt * nt];
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
    let mut coef = vec![[0.0f64; 6]; nt];
    for c in 0..6 {
        let mut y = vec![0.0; nt];
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
    Some(Fit { centre, scale, degree: p, coef })
}

impl Fit {
    fn eval(&self, d: usize, x: [f64; 3]) -> [f64; 6] {
        let xs: [f64; 3] = std::array::from_fn(|i| if i < d { (x[i] - self.centre[i]) / self.scale } else { 0.0 });
        let mut t = [0.0; 10];
        terms(d, self.degree, xs, &mut t);
        let mut out = [0.0; 6];
        for (k, c) in self.coef.iter().enumerate() {
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

impl Model {
    /// SPR nodal stresses from the Gauss-point stresses (`gauss_stresses` output).
    pub fn recover_spr(&self, gauss: &[Vec<GaussStress>]) -> Result<Vec<[f64; 6]>, String> {
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
        let mut fits: Vec<Option<Fit>> = (0..n).map(|_| None).collect();
        for nd in 0..n {
            if corner[nd] {
                fits[nd] = fit(d, degree[nd], self.mesh.nodes[nd], &samples_of(nd));
            }
        }
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
        let mut out = vec![[0.0f64; 6]; n];
        let mut done = vec![false; n];
        for nd in 0..n {
            if !corner[nd] {
                continue;
            }
            let x = self.mesh.nodes[nd];
            if let (true, Some(m)) = (on_boundary[nd], borrowed[nd]) {
                out[nd] = fits[m].as_ref().unwrap().eval(d, x);
            } else if let Some(f) = &fits[nd] {
                out[nd] = f.eval(d, x);
            } else if let Some(m) = borrowed[nd] {
                out[nd] = fits[m].as_ref().unwrap().eval(d, x);
            } else {
                // Lower-degree fit on the node's own patch; the plain average as the last resort.
                let s = samples_of(nd);
                let low = (0..degree[nd]).rev().find_map(|p| fit(d, p, x, &s));
                out[nd] = match low {
                    Some(f) => f.eval(d, x),
                    None => {
                        let mut avg = [0.0; 6];
                        for g in &s {
                            for c in 0..6 {
                                avg[c] += g.1[c] / s.len() as f64;
                            }
                        }
                        avg
                    }
                };
            }
            done[nd] = true;
        }
        // Midside / face / interior nodes: average of the owning elements' corner polynomials at the node.
        let corner_value = |cn: usize, x: [f64; 3]| -> Option<[f64; 6]> {
            match (on_boundary[cn], borrowed[cn], &fits[cn]) {
                (true, Some(m), _) | (_, Some(m), None) => Some(fits[m].as_ref().unwrap().eval(d, x)),
                (_, _, Some(f)) => Some(f.eval(d, x)),
                _ => None,
            }
        };
        for nd in 0..n {
            if done[nd] {
                continue;
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
            if cnt > 0.0 {
                for c in 0..6 {
                    out[nd][c] = sum[c] / cnt;
                }
            }
        }
        Ok(out)
    }

    /// ZZ error estimate from the displacement field `u`: per-element energy norm of the difference
    /// between the SPR-recovered and the finite-element stress.
    pub fn zz_error(&self, u: &[f64], delta_t: f64) -> Result<ZzEstimate, String> {
        let gauss = self.gauss_stresses(u, delta_t)?;
        let rec = self.recover_spr(&gauss)?;
        let mut work = Work::new();
        let (mut eta, mut norm2) = (Vec::with_capacity(self.mesh.n_elems()), 0.0f64);
        for (bi, blk) in self.mesh.blocks.iter().enumerate() {
            let t = blk.kind.table();
            let nn = t.nn;
            let mut xyz = vec![[0.0; 3]; nn];
            for e in 0..blk.n_elems() {
                let conn = blk.elem(e);
                for (a, &nd) in conn.iter().enumerate() {
                    xyz[a] = self.mesh.nodes[nd];
                }
                kernel::geometry(blk.kind, self.mesh.physics, &xyz, &mut work).map_err(|er| er.to_string())?;
                let mut e2 = 0.0;
                for g in 0..t.ngp {
                    let nsh = t.n_at(g);
                    let mut star = [0.0; 6];
                    for (a, &nd) in conn.iter().enumerate() {
                        for c in 0..6 {
                            star[c] += nsh[a] * rec[nd][c];
                        }
                    }
                    let h = gauss[bi][e * t.ngp + g].1;
                    let diff: [f64; 6] = std::array::from_fn(|c| star[c] - h[c]);
                    e2 += work.wdet[g] * complementary_energy(&blk.material, &diff) * 2.0;
                    norm2 += work.wdet[g] * complementary_energy(&blk.material, &star) * 2.0;
                }
                eta.push(e2.sqrt());
            }
        }
        let total = eta.iter().map(|x| x * x).sum::<f64>().sqrt();
        Ok(ZzEstimate { eta, total, norm: norm2.sqrt() })
    }

    /// Energy norm `sqrt(int (sigma_exact - sigma_h) : C^-1 : (sigma_exact - sigma_h))` of the true
    /// stress error against a known stress field (verification and convergence studies).
    pub fn stress_error_energy(&self, u: &[f64], delta_t: f64, exact: impl Fn(&[f64; 3]) -> [f64; 6]) -> Result<f64, String> {
        let gauss = self.gauss_stresses(u, delta_t)?;
        let mut work = Work::new();
        let mut sum = 0.0;
        for (bi, blk) in self.mesh.blocks.iter().enumerate() {
            let t = blk.kind.table();
            let mut xyz = vec![[0.0; 3]; t.nn];
            for e in 0..blk.n_elems() {
                for (a, &nd) in blk.elem(e).iter().enumerate() {
                    xyz[a] = self.mesh.nodes[nd];
                }
                kernel::geometry(blk.kind, self.mesh.physics, &xyz, &mut work).map_err(|er| er.to_string())?;
                for g in 0..t.ngp {
                    let (x, h) = &gauss[bi][e * t.ngp + g];
                    let ex = exact(x);
                    let diff: [f64; 6] = std::array::from_fn(|c| ex[c] - h[c]);
                    sum += work.wdet[g] * complementary_energy(&blk.material, &diff) * 2.0;
                }
            }
        }
        Ok(sum.sqrt())
    }
}
