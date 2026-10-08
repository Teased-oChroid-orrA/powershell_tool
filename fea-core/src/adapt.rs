//! Error-driven size fields for adaptive remeshing: from a ZZ element error estimate to a target
//! edge length per element, smoothed to nodal sizes with a grading limit, and interpolated on the
//! previous mesh so the mesher (`delaunay.rs`) can ask for the size anywhere.
//!
//! The new size of an element follows the error equidistribution principle: with the energy-norm
//! error `eta_e ~ h^p` the element size is scaled by `(eta_target / eta_e)^(1/p)`, where
//! `eta_target = rel_error * |u| / sqrt(n_elements)`.

#![allow(clippy::needless_range_loop)] // index-parallel numeric kernels read clearer as loops

use crate::element::ElementKind;
use crate::mesh::Mesh;
use crate::recover::ZzEstimate;

#[derive(Debug, Clone, Copy)]
pub struct AdaptOptions {
    /// Target relative energy-norm error of the next mesh.
    pub target_rel_error: f64,
    /// Convergence order of the energy-norm error in `h` (2 for quadratic elements).
    pub order: f64,
    /// An element shrinks by at most `min_factor` and grows by at most `max_factor` per pass.
    pub min_factor: f64,
    pub max_factor: f64,
    /// Largest size change per unit distance (`|h(x) - h(y)| <= grading |x - y|`).
    pub grading: f64,
    pub h_min: f64,
    pub h_max: f64,
}

impl Default for AdaptOptions {
    fn default() -> Self {
        Self { target_rel_error: 0.02, order: 2.0, min_factor: 0.25, max_factor: 2.0, grading: 0.4, h_min: 0.0, h_max: f64::INFINITY }
    }
}

/// A piecewise-linear size field on a background triangulation with grid-accelerated lookup.
pub struct SizeField {
    pts: Vec<[f64; 2]>,
    tris: Vec<[usize; 3]>,
    h: Vec<f64>,
    lo: [f64; 2],
    cell: f64,
    nx: usize,
    ny: usize,
    bins: Vec<Vec<u32>>,
}

impl SizeField {
    /// Build from vertices, triangles and a size at every vertex.
    pub fn new(pts: Vec<[f64; 2]>, tris: Vec<[usize; 3]>, h: Vec<f64>) -> Self {
        let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
        for p in &pts {
            for i in 0..2 {
                lo[i] = lo[i].min(p[i]);
                hi[i] = hi[i].max(p[i]);
            }
        }
        let n = (tris.len() as f64).sqrt().ceil().max(1.0);
        let cell = ((hi[0] - lo[0]).max(hi[1] - lo[1]) / n).max(1e-300);
        let (nx, ny) = (((hi[0] - lo[0]) / cell).floor() as usize + 1, ((hi[1] - lo[1]) / cell).floor() as usize + 1);
        let mut bins = vec![Vec::new(); nx * ny];
        for (t, tr) in tris.iter().enumerate() {
            let (mut tlo, mut thi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
            for &v in tr {
                for i in 0..2 {
                    tlo[i] = tlo[i].min(pts[v][i]);
                    thi[i] = thi[i].max(pts[v][i]);
                }
            }
            let (i0, i1) = (((tlo[0] - lo[0]) / cell).floor() as usize, ((thi[0] - lo[0]) / cell).floor() as usize);
            let (j0, j1) = (((tlo[1] - lo[1]) / cell).floor() as usize, ((thi[1] - lo[1]) / cell).floor() as usize);
            for j in j0..=j1.min(ny - 1) {
                for i in i0..=i1.min(nx - 1) {
                    bins[j * nx + i].push(t as u32);
                }
            }
        }
        Self { pts, tris, h, lo, cell, nx, ny, bins }
    }

    /// Size at `x`: linear interpolation in the containing triangle. A point just outside the
    /// background mesh (a new vertex on a curved boundary the straight-sided background cuts)
    /// uses the closest nearby triangle with its barycentric weights clamped; far outside, the
    /// nearest vertex.
    pub fn at(&self, x: [f64; 2]) -> f64 {
        let (ci, cj) = (((x[0] - self.lo[0]) / self.cell).floor() as i64, ((x[1] - self.lo[1]) / self.cell).floor() as i64);
        let mut best: Option<(f64, [f64; 3], [usize; 3])> = None;
        for dj in -1..=1i64 {
            for di in -1..=1i64 {
                let (i, j) = (ci + di, cj + dj);
                if i < 0 || j < 0 || i as usize >= self.nx || j as usize >= self.ny {
                    continue;
                }
                for &t in &self.bins[j as usize * self.nx + i as usize] {
                    let tr = self.tris[t as usize];
                    let (a, b, c) = (self.pts[tr[0]], self.pts[tr[1]], self.pts[tr[2]]);
                    let d = (b[1] - c[1]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[1] - c[1]);
                    if d == 0.0 {
                        continue;
                    }
                    let l0 = ((b[1] - c[1]) * (x[0] - c[0]) + (c[0] - b[0]) * (x[1] - c[1])) / d;
                    let l1 = ((c[1] - a[1]) * (x[0] - c[0]) + (a[0] - c[0]) * (x[1] - c[1])) / d;
                    let l = [l0, l1, 1.0 - l0 - l1];
                    let m = l.iter().cloned().fold(f64::INFINITY, f64::min);
                    if m >= -1e-9 {
                        return l[0] * self.h[tr[0]] + l[1] * self.h[tr[1]] + l[2] * self.h[tr[2]];
                    }
                    if best.as_ref().is_none_or(|b| m > b.0) {
                        best = Some((m, l, tr));
                    }
                }
            }
        }
        if let Some((m, l, tr)) = best {
            if m > -1.0 {
                let w: [f64; 3] = std::array::from_fn(|i| l[i].max(0.0));
                let sum: f64 = w.iter().sum();
                return (0..3).map(|i| w[i] / sum * self.h[tr[i]]).sum();
            }
        }
        let mut near = (f64::INFINITY, 0usize);
        for (k, p) in self.pts.iter().enumerate() {
            let d = (p[0] - x[0]).powi(2) + (p[1] - x[1]).powi(2);
            if d < near.0 {
                near = (d, k);
            }
        }
        self.h[near.1]
    }
}

/// Equilateral-equivalent edge length of an area (`A = sqrt(3)/4 h^2`).
fn edge_of_area(a: f64) -> f64 {
    (4.0 * a / 3.0f64.sqrt()).sqrt()
}

/// Build the size field of the next mesh from the ZZ estimate of the current one. `quad_split`
/// marks quadrilateral meshes produced by the three-quads-per-triangle split (each quad is a third
/// of a triangle, so its area is tripled to recover the triangle size the mesher asks for).
pub fn adapted_size_field(mesh: &Mesh, zz: &ZzEstimate, quad_split: bool, opt: &AdaptOptions) -> Result<SizeField, String> {
    if mesh.dim() != 2 {
        return Err("adaptive remeshing is for 2D meshes".into());
    }
    let n_elems = mesh.n_elems();
    if zz.eta.len() != n_elems {
        return Err(format!("{} error values for {} elements", zz.eta.len(), n_elems));
    }
    let target = opt.target_rel_error * (zz.norm * zz.norm + zz.total * zz.total).sqrt() / (n_elems as f64).sqrt();
    // Corner triangles of every element (a quad is fanned into two).
    let n_nodes = mesh.nodes.len();
    let pts: Vec<[f64; 2]> = mesh.nodes.iter().map(|x| [x[0], x[1]]).collect();
    let mut tris: Vec<[usize; 3]> = Vec::new();
    let mut h_elem: Vec<f64> = Vec::with_capacity(n_elems);
    let mut elem_tris: Vec<(usize, usize)> = Vec::new(); // range into `tris` per element
    let mut e = 0;
    for blk in &mesh.blocks {
        let nc = blk.kind.n_corners();
        for conn in blk.conn.chunks_exact(blk.kind.n_nodes()) {
            let c = &conn[..nc];
            let first = tris.len();
            tris.push([c[0], c[1], c[2]]);
            if matches!(blk.kind, ElementKind::Quad4 | ElementKind::Quad8 | ElementKind::Quad9) {
                tris.push([c[0], c[2], c[3]]);
            }
            let area: f64 = tris[first..].iter().map(|t| 0.5 * ((pts[t[1]][0] - pts[t[0]][0]) * (pts[t[2]][1] - pts[t[0]][1]) - (pts[t[1]][1] - pts[t[0]][1]) * (pts[t[2]][0] - pts[t[0]][0])).abs()).sum();
            let a_tri = if quad_split && nc == 4 { 3.0 * area } else { area };
            let h_old = edge_of_area(a_tri);
            let ratio = if zz.eta[e] > 0.0 { (target / zz.eta[e]).powf(1.0 / opt.order) } else { opt.max_factor };
            h_elem.push((h_old * ratio.clamp(opt.min_factor, opt.max_factor)).clamp(opt.h_min, opt.h_max));
            elem_tris.push((first, tris.len()));
            e += 1;
        }
    }
    // Nodal size: geometric mean of the adjacent elements' sizes.
    let (mut sum, mut cnt) = (vec![0.0f64; n_nodes], vec![0u32; n_nodes]);
    let mut e = 0;
    for blk in &mesh.blocks {
        let nc = blk.kind.n_corners();
        for conn in blk.conn.chunks_exact(blk.kind.n_nodes()) {
            for &v in &conn[..nc] {
                sum[v] += h_elem[e].ln();
                cnt[v] += 1;
            }
            e += 1;
        }
    }
    let mut h: Vec<f64> = (0..n_nodes).map(|i| if cnt[i] > 0 { (sum[i] / cnt[i] as f64).exp() } else { f64::INFINITY }).collect();
    // Grading limit along the triangle edges (a few relaxation sweeps converge it).
    for _ in 0..(n_nodes.min(200)) {
        let mut changed = false;
        for t in &tris {
            for k in 0..3 {
                let (i, j) = (t[k], t[(k + 1) % 3]);
                let d = (pts[i][0] - pts[j][0]).hypot(pts[i][1] - pts[j][1]);
                if h[i] > h[j] + opt.grading * d {
                    h[i] = h[j] + opt.grading * d;
                    changed = true;
                }
                if h[j] > h[i] + opt.grading * d {
                    h[j] = h[i] + opt.grading * d;
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }
    let _ = elem_tris;
    // Midside and interior nodes carry no size; give them the mean of their element's corners so the
    // background triangles only reference corner nodes (they are the only vertices of `tris`).
    Ok(SizeField::new(pts, tris, h.into_iter().map(|v| if v.is_finite() { v } else { opt.h_max.min(1e30) }).collect()))
}

/// One meshed and solved pass of an adaptive run: what the driver needs to judge it and to size the next mesh.
pub trait Pass {
    fn mesh(&self) -> &Mesh;
    fn zz(&self) -> &ZzEstimate;
}

/// Adaptive remeshing loop shared by every consumer (the caller owns meshing and solving). From the `first` pass it asks
/// `next` for a pass on the size field of the best pass so far, up to `passes` times, and keeps the best:
/// - above `opt.target_rel_error` a pass is kept only if its ZZ estimate is lower (near a singularity the estimate is
///   unreliable, and a finer mesh must never make the answer worse);
/// - at or below it the field coarsens where the error is low, and a pass is kept when it still meets the target with
///   fewer than 90 % of the unknowns.
/// The run stops at the first pass that is not kept. `seen` is called with every pass computed, kept or not (history).
pub fn refine<T: Pass>(first: T, passes: usize, quad_split: bool, opt: &AdaptOptions, mut next: impl FnMut(&SizeField) -> Result<T, String>, mut seen: impl FnMut(&T)) -> Result<T, String> {
    seen(&first);
    let mut best = first;
    for _ in 0..passes {
        let field = adapted_size_field(best.mesh(), best.zz(), quad_split, opt)?;
        let cand = next(&field)?;
        seen(&cand);
        let (now, then) = (best.zz().relative(), cand.zz().relative());
        let better = if now > opt.target_rel_error { then < now } else { then <= opt.target_rel_error && (cand.mesh().n_dofs() as f64) < 0.9 * best.mesh().n_dofs() as f64 };
        if !better {
            break;
        }
        best = cand;
    }
    Ok(best)
}
