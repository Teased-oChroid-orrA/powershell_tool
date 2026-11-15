//! Point location: the stress (or any element-wise field) at an arbitrary physical point.
//!
//! A [`Locator`] bins the element bounding boxes once; [`Locator::stress_at`] finds the elements
//! whose natural coordinates of the point (Newton on the isoparametric map) lie inside, and
//! averages the element stress there, so a point on an element edge gets the mean of both sides.
//! A point that falls a hair outside every element (the bore edge of a quadratic element is a
//! parabola through three on-circle nodes, so a point exactly on the true circle can sit just
//! outside it) takes the nearest element within [`NEAREST_TOL`] of its natural extent.

use crate::analysis::{stress_at_xi, Model};
use crate::element::{ElementKind, MAX_NODES};
use std::collections::HashMap;

/// A point this far outside an element (in natural coordinates) still counts when no element
/// contains it.
pub const NEAREST_TOL: f64 = 0.02;
/// Inside tolerance in natural coordinates.
const INSIDE_TOL: f64 = 1e-7;

/// Where a point sits in the mesh.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hit {
    pub block: usize,
    pub elem: usize,
    pub xi: [f64; 3],
    /// How far outside the element's natural domain (negative inside).
    pub excess: f64,
}

struct Entry {
    block: usize,
    elem: usize,
    lo: [f64; 3],
    hi: [f64; 3],
}

pub struct Locator {
    entries: Vec<Entry>,
    cell: f64,
    origin: [f64; 3],
    bins: HashMap<[i64; 3], Vec<u32>>,
    dim: usize,
}

fn excess(kind: ElementKind, xi: [f64; 3]) -> f64 {
    let d = kind.dim();
    if kind.is_simplex() {
        let sum: f64 = xi[..d].iter().sum();
        xi[..d].iter().map(|v| -v).fold(sum - 1.0, f64::max)
    } else {
        xi[..d].iter().map(|v| v.abs()).fold(0.0, f64::max) - 1.0
    }
}

/// Newton inverse of the isoparametric map. `None` if it does not converge.
fn natural_coordinates(kind: ElementKind, xyz: &[[f64; 3]], x: [f64; 3]) -> Option<([f64; 3], f64)> {
    let d = kind.dim();
    let nn = kind.n_nodes();
    let mut xi = if kind.is_simplex() { [1.0 / (d as f64 + 1.0); 3] } else { [0.0; 3] };
    for k in d..3 {
        xi[k] = 0.0;
    }
    for _ in 0..40 {
        let (n, dn) = kind.shape(xi);
        let mut r = [0.0f64; 3];
        let mut jac = [[0.0f64; 3]; 3];
        for i in 0..d {
            r[i] = x[i] - (0..nn).map(|b| n[b] * xyz[b][i]).sum::<f64>();
            for k in 0..d {
                jac[i][k] = (0..nn).map(|b| xyz[b][i] * dn[b][k]).sum();
            }
        }
        let step = solve_small(&jac, &r, d)?;
        for k in 0..d {
            xi[k] += step[k];
        }
        if xi[..d].iter().any(|v| v.abs() > 6.0) {
            return None;
        }
        if step[..d].iter().all(|s| s.abs() < 1e-12) {
            return Some((xi, excess(kind, xi)));
        }
    }
    None
}

/// Solve `J s = r` for a 2x2 or 3x3 system by Cramer's rule.
fn solve_small(j: &[[f64; 3]; 3], r: &[f64; 3], d: usize) -> Option<[f64; 3]> {
    if d == 2 {
        let det = j[0][0] * j[1][1] - j[0][1] * j[1][0];
        if det.abs() < 1e-300 {
            return None;
        }
        return Some([(r[0] * j[1][1] - r[1] * j[0][1]) / det, (-r[0] * j[1][0] + r[1] * j[0][0]) / det, 0.0]);
    }
    let det3 = |m: [[f64; 3]; 3]| m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0]) + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
    let det = det3(*j);
    if det.abs() < 1e-300 {
        return None;
    }
    let mut out = [0.0; 3];
    for c in 0..3 {
        let mut m = *j;
        for row in 0..3 {
            m[row][c] = r[row];
        }
        out[c] = det3(m) / det;
    }
    Some(out)
}

impl Locator {
    pub fn new(model: &Model) -> Self {
        let mesh = &model.mesh;
        let dim = mesh.dim();
        let mut entries = Vec::with_capacity(mesh.n_elems());
        let (mut size_sum, mut count) = (0.0, 0usize);
        for (bi, blk) in mesh.blocks.iter().enumerate() {
            for e in 0..blk.n_elems() {
                let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
                for &n in blk.elem(e) {
                    for i in 0..3 {
                        lo[i] = lo[i].min(mesh.nodes[n][i]);
                        hi[i] = hi[i].max(mesh.nodes[n][i]);
                    }
                }
                size_sum += (0..dim).map(|i| hi[i] - lo[i]).fold(0.0, f64::max);
                count += 1;
                entries.push(Entry { block: bi, elem: e, lo, hi });
            }
        }
        let cell = if count > 0 { (size_sum / count as f64).max(1e-300) } else { 1.0 };
        let mut origin = [f64::INFINITY; 3];
        for e in &entries {
            for i in 0..3 {
                origin[i] = origin[i].min(e.lo[i]);
            }
        }
        if count == 0 {
            origin = [0.0; 3];
        }
        let mut bins: HashMap<[i64; 3], Vec<u32>> = HashMap::new();
        for (idx, e) in entries.iter().enumerate() {
            let (a, b) = (Self::bin(origin, cell, e.lo), Self::bin(origin, cell, e.hi));
            for i in a[0]..=b[0] {
                for j in a[1]..=b[1] {
                    for k in a[2]..=b[2] {
                        bins.entry([i, j, k]).or_default().push(idx as u32);
                    }
                }
            }
        }
        Self { entries, cell, origin, bins, dim }
    }

    fn bin(origin: [f64; 3], cell: f64, x: [f64; 3]) -> [i64; 3] {
        std::array::from_fn(|i| ((x[i] - origin[i]) / cell).floor() as i64)
    }

    /// Every element that contains `x`, else the nearest one within [`NEAREST_TOL`].
    pub fn locate(&self, model: &Model, x: [f64; 3]) -> Vec<Hit> {
        let mesh = &model.mesh;
        let pad = NEAREST_TOL * self.cell * 1.5;
        let (a, b) = (Self::bin(self.origin, self.cell, std::array::from_fn(|i| x[i] - pad)), Self::bin(self.origin, self.cell, std::array::from_fn(|i| x[i] + pad)));
        let mut seen: Vec<u32> = Vec::new();
        for i in a[0]..=b[0] {
            for j in a[1]..=b[1] {
                for k in a[2]..=b[2] {
                    if let Some(v) = self.bins.get(&[i, j, k]) {
                        seen.extend_from_slice(v);
                    }
                }
            }
        }
        seen.sort_unstable();
        seen.dedup();
        let mut inside = Vec::new();
        let mut nearest: Option<Hit> = None;
        for idx in seen {
            let e = &self.entries[idx as usize];
            let tol = 1e-9 * (1.0 + x.iter().map(|v| v.abs()).sum::<f64>()) + NEAREST_TOL * (0..self.dim).map(|i| e.hi[i] - e.lo[i]).fold(0.0, f64::max);
            if (0..self.dim).any(|i| x[i] < e.lo[i] - tol || x[i] > e.hi[i] + tol) {
                continue;
            }
            let blk = &mesh.blocks[e.block];
            let mut xyz = [[0.0f64; 3]; MAX_NODES];
            for (a, &n) in blk.elem(e.elem).iter().enumerate() {
                xyz[a] = mesh.nodes[n];
            }
            let Some((xi, ex)) = natural_coordinates(blk.kind, &xyz[..blk.kind.n_nodes()], x) else { continue };
            let hit = Hit { block: e.block, elem: e.elem, xi, excess: ex };
            if ex <= INSIDE_TOL {
                inside.push(hit);
            } else if ex <= NEAREST_TOL && nearest.is_none_or(|n| ex < n.excess) {
                nearest = Some(hit);
            }
        }
        if inside.is_empty() {
            inside.extend(nearest);
        }
        inside
    }

    /// Stress `[xx, yy, zz, xy, yz, zx]` at `x` for displacements `u`, averaged over the elements
    /// containing the point. `None` outside the mesh.
    pub fn stress_at(&self, model: &Model, u: &[f64], x: [f64; 3], delta_t: f64) -> Option<[f64; 6]> {
        let hits = self.locate(model, x);
        if hits.is_empty() {
            return None;
        }
        let mesh = &model.mesh;
        let d = mesh.dim();
        let mut acc = [0.0f64; 6];
        for h in &hits {
            let blk = &mesh.blocks[h.block];
            let nn = blk.kind.n_nodes();
            let mut xyz = vec![[0.0f64; 3]; nn];
            let mut ue = vec![0.0f64; nn * d];
            for (a, &n) in blk.elem(h.elem).iter().enumerate() {
                xyz[a] = mesh.nodes[n];
                for i in 0..d {
                    ue[a * d + i] = u[n * d + i];
                }
            }
            // A point a hair outside is evaluated at the clamped natural coordinates (the element's own edge).
            let xi = clamp_natural(blk.kind, h.xi);
            let s = stress_at_xi(blk.kind, mesh.physics, &blk.material, &xyz, &ue, xi, delta_t).ok()?;
            for c in 0..6 {
                acc[c] += s[c];
            }
        }
        for v in &mut acc {
            *v /= hits.len() as f64;
        }
        Some(acc)
    }
}

fn clamp_natural(kind: ElementKind, mut xi: [f64; 3]) -> [f64; 3] {
    let d = kind.dim();
    if kind.is_simplex() {
        for v in xi.iter_mut().take(d) {
            *v = v.max(0.0);
        }
        let sum: f64 = xi[..d].iter().sum();
        if sum > 1.0 {
            for v in xi.iter_mut().take(d) {
                *v /= sum;
            }
        }
    } else {
        for v in xi.iter_mut().take(d) {
            *v = v.clamp(-1.0, 1.0);
        }
    }
    xi
}

impl Model {
    /// A point locator for this mesh (build once, query many times).
    pub fn locator(&self) -> Locator {
        Locator::new(self)
    }
}
