//! Geometry-aware classification of a planar region into structured and unstructured cells.
//!
//! The domain's bounding box is cut into a tensor grid of coarse cells by (a) the coordinates of the rectilinear outer
//! boundary's vertices and (b) the padded boxes of every geometric feature that makes a structured block unattractive: holes
//! (any shape), reflex corners of the outer boundary, and caller-supplied boxes (load and restraint locations, contact
//! patches). A cell outside the outer boundary is void; a cell overlapping a padded feature box is unstructured; every other
//! cell is a regular rectangle and structured. Long intervals are subdivided so a structured block follows the size function
//! instead of being one uniform-size cell.
//!
//! Only a rectilinear (axis-aligned straight) outer boundary is supported: for any other outer boundary the cells cannot
//! tile the region exactly and [`classify_2d`] returns `Unsupported` (the caller falls back to an unstructured mesh).

use super::MeshError;
use crate::geometry::{Curve, Loop, Region};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellState {
    /// Outside the outer boundary.
    Void,
    Structured,
    Unstructured,
}

/// Controls of the classification.
#[derive(Debug, Clone)]
pub struct PartitionOptions2d {
    /// Padding around a feature, in local element sizes (the buffer in which the unstructured mesh grades from the feature
    /// size to the structured block's).
    pub margin: f64,
    /// A coarse interval longer than `span` local element sizes is split.
    pub span: f64,
    /// Extra regions that must stay unstructured (`(lo, hi)` boxes, padded like a feature).
    pub extra_boxes: Vec<([f64; 2], [f64; 2])>,
}

impl Default for PartitionOptions2d {
    fn default() -> Self {
        Self { margin: 2.5, span: 6.0, extra_boxes: Vec::new() }
    }
}

/// Tensor grid of coarse cells with a state each (`cj * nx + ci`).
#[derive(Debug, Clone)]
pub struct CellGrid {
    pub xs: Vec<f64>,
    pub ys: Vec<f64>,
    pub state: Vec<CellState>,
}

impl CellGrid {
    pub fn nx(&self) -> usize {
        self.xs.len() - 1
    }
    pub fn ny(&self) -> usize {
        self.ys.len() - 1
    }
    pub fn at(&self, ci: usize, cj: usize) -> CellState {
        self.state[cj * self.nx() + ci]
    }
    /// State of a cell, `Void` outside the grid.
    pub fn at_signed(&self, ci: i64, cj: i64) -> CellState {
        if ci < 0 || cj < 0 || ci >= self.nx() as i64 || cj >= self.ny() as i64 {
            CellState::Void
        } else {
            self.at(ci as usize, cj as usize)
        }
    }
    pub fn cell_box(&self, ci: usize, cj: usize) -> ([f64; 2], [f64; 2]) {
        ([self.xs[ci], self.ys[cj]], [self.xs[ci + 1], self.ys[cj + 1]])
    }
    fn area_of(&self, s: CellState) -> f64 {
        let mut a = 0.0;
        for cj in 0..self.ny() {
            for ci in 0..self.nx() {
                if self.at(ci, cj) == s {
                    a += (self.xs[ci + 1] - self.xs[ci]) * (self.ys[cj + 1] - self.ys[cj]);
                }
            }
        }
        a
    }
    pub fn structured_area(&self) -> f64 {
        self.area_of(CellState::Structured)
    }
    pub fn unstructured_area(&self) -> f64 {
        self.area_of(CellState::Unstructured)
    }
    pub fn count(&self, s: CellState) -> usize {
        self.state.iter().filter(|c| **c == s).count()
    }
}

fn loop_bounds(l: &Loop) -> ([f64; 2], [f64; 2]) {
    let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
    for s in &l.segments {
        for k in 0..=64 {
            let p = s.curve.point(k as f64 / 64.0);
            for i in 0..2 {
                lo[i] = lo[i].min(p[i]);
                hi[i] = hi[i].max(p[i]);
            }
        }
    }
    (lo, hi)
}

/// Whether the outer boundary of `region` is made of axis-aligned straight segments; returns its vertices.
pub fn rectilinear_vertices(region: &Region) -> Result<Vec<[f64; 2]>, MeshError> {
    let (lo, hi) = region.bounds();
    let tol = 1e-9 * (hi[0] - lo[0]).max(hi[1] - lo[1]).max(1e-300);
    let mut v = Vec::new();
    for s in &region.outer.segments {
        match s.curve {
            Curve::Line { a, b } if (a[0] - b[0]).abs() <= tol || (a[1] - b[1]).abs() <= tol => v.push(a),
            _ => return Err(MeshError::Unsupported(format!("the outer boundary segment '{}' is not an axis-aligned line; hybrid meshing needs a rectilinear outer boundary", s.name))),
        }
    }
    Ok(v)
}

/// Sorted, merged line coordinates; `mandatory` ones survive a merge.
fn merge_lines(mut lines: Vec<(f64, bool)>, eps: f64) -> Vec<f64> {
    lines.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut out: Vec<(f64, bool)> = Vec::new();
    for (x, m) in lines {
        match out.last_mut() {
            Some(last) if x - last.0 <= eps => {
                if m && !last.1 {
                    *last = (x, true);
                }
            }
            _ => out.push((x, m)),
        }
    }
    out.into_iter().map(|l| l.0).collect()
}

/// Classify `region` into cells (see the module docs). `size` is the target element size at a point.
pub fn classify_2d(region: &Region, size: &dyn Fn([f64; 2]) -> f64, opt: &PartitionOptions2d) -> Result<CellGrid, MeshError> {
    let verts = rectilinear_vertices(region)?;
    let (lo, hi) = region.bounds();
    let diag = (hi[0] - lo[0]).hypot(hi[1] - lo[1]);
    let scale_tol = 1e-9 * diag;
    // reference (smallest) size over a sampling of the domain: sets the merge tolerance of nearby lines
    let mut h_ref = f64::INFINITY;
    for i in 0..9 {
        for j in 0..9 {
            let p = [lo[0] + (hi[0] - lo[0]) * i as f64 / 8.0, lo[1] + (hi[1] - lo[1]) * j as f64 / 8.0];
            let h = size(p);
            if !(h.is_finite() && h > 0.0) {
                return Err(MeshError::Failed(format!("the size function returned {h} at ({}, {})", p[0], p[1])));
            }
            h_ref = h_ref.min(h);
        }
    }
    // padded feature boxes
    let mut boxes: Vec<([f64; 2], [f64; 2])> = Vec::new();
    let pad_of = |b: &([f64; 2], [f64; 2])| opt.margin * size([0.5 * (b.0[0] + b.1[0]), 0.5 * (b.0[1] + b.1[1])]);
    for h in &region.holes {
        boxes.push(loop_bounds(h));
    }
    boxes.extend(opt.extra_boxes.iter().copied());
    let n = verts.len();
    let outer_area_sign = region.outer.signed_area().signum();
    for k in 0..n {
        let (p, c, nx) = (verts[(k + n - 1) % n], verts[k], verts[(k + 1) % n]);
        let cross = (c[0] - p[0]) * (nx[1] - c[1]) - (c[1] - p[1]) * (nx[0] - c[0]);
        if cross * outer_area_sign < -scale_tol * scale_tol {
            boxes.push((c, c)); // reflex corner
        }
    }
    // The padding must cover the transition: a block starts where the element size has grown to about `1/margin` of the
    // distance to the feature, so the pad is the fixed point of `pad = margin * (largest size on the padded boundary)`. Where
    // the size grows faster than `1/margin` per unit length the fixed point does not exist and the box grows over the whole
    // domain (no structured cells: the unstructured mesh is the right answer there).
    let pad_box = |b: &([f64; 2], [f64; 2]), pad: f64| ([(b.0[0] - pad).max(lo[0]), (b.0[1] - pad).max(lo[1])], [(b.1[0] + pad).min(hi[0]), (b.1[1] + pad).min(hi[1])]);
    let padded: Vec<([f64; 2], [f64; 2])> = boxes
        .iter()
        .map(|b| {
            let mut pad = pad_of(b);
            for _ in 0..12 {
                let q = pad_box(b, pad);
                // sizes along the padded boundary (corners and edge midpoints)
                let mut h_edge = 0.0f64;
                for (fx, fy) in [(0.0, 0.0), (0.5, 0.0), (1.0, 0.0), (0.0, 0.5), (1.0, 0.5), (0.0, 1.0), (0.5, 1.0), (1.0, 1.0)] {
                    h_edge = h_edge.max(size([q.0[0] + fx * (q.1[0] - q.0[0]), q.0[1] + fy * (q.1[1] - q.0[1])]));
                }
                let next = opt.margin * h_edge;
                if (next - pad).abs() <= 1e-3 * pad || pad > diag {
                    pad = next.max(pad);
                    break;
                }
                pad = next;
            }
            pad_box(b, pad)
        })
        .collect();

    let eps = (0.25 * h_ref).min(0.25 * diag);
    let mut xl: Vec<(f64, bool)> = verts.iter().map(|v| (v[0], true)).collect();
    let mut yl: Vec<(f64, bool)> = verts.iter().map(|v| (v[1], true)).collect();
    xl.push((lo[0], true));
    xl.push((hi[0], true));
    yl.push((lo[1], true));
    yl.push((hi[1], true));
    for b in &padded {
        xl.push((b.0[0], false));
        xl.push((b.1[0], false));
        yl.push((b.0[1], false));
        yl.push((b.1[1], false));
    }
    let mut xs = merge_lines(xl, eps);
    let mut ys = merge_lines(yl, eps);

    // Subdivide intervals so the size function is nearly constant across each: an interval is bisected while it is longer than
    // `span` local sizes or the (across-the-domain minimum of the) size varies by more than `SIZE_RATIO` along it. A coarse
    // cell is later divided uniformly at its smallest sample, so a wide cell over a graded size field would be over-resolved.
    const SIZE_RATIO: f64 = 1.4;
    let size_along = |t: f64, other: (f64, f64), horizontal: bool| -> f64 {
        let mut h = f64::INFINITY;
        for s in 0..5 {
            let o = other.0 + (other.1 - other.0) * s as f64 / 4.0;
            h = h.min(size(if horizontal { [t, o] } else { [o, t] }));
        }
        h
    };
    fn bisect(a: f64, b: f64, depth: usize, size_at: &dyn Fn(f64) -> f64, span: f64, out: &mut Vec<f64>) {
        let (ha, hm, hb) = (size_at(a), size_at(0.5 * (a + b)), size_at(b));
        let (lo, hi) = (ha.min(hm).min(hb), ha.max(hm).max(hb));
        if depth < 12 && ((b - a) > span * lo || hi > SIZE_RATIO * lo) {
            let m = 0.5 * (a + b);
            bisect(a, m, depth + 1, size_at, span, out);
            bisect(m, b, depth + 1, size_at, span, out);
        } else {
            out.push(b);
        }
    }
    let split = |lines: &[f64], other: (f64, f64), horizontal: bool| -> Vec<f64> {
        let mut out = vec![lines[0]];
        for w in lines.windows(2) {
            bisect(w[0], w[1], 0, &|t| size_along(t, other, horizontal), opt.span, &mut out);
        }
        out
    };
    xs = split(&xs, (lo[1], hi[1]), true);
    ys = split(&ys, (lo[0], hi[0]), false);

    let mut grid = CellGrid { xs, ys, state: Vec::new() };
    let e = 1e-9 * diag;
    for cj in 0..grid.ny() {
        for ci in 0..grid.nx() {
            let (a, b) = grid.cell_box(ci, cj);
            let centre = [0.5 * (a[0] + b[0]), 0.5 * (a[1] + b[1])];
            let st = if !region.outer.contains(centre) {
                CellState::Void
            } else if padded.iter().any(|f| a[0] < f.1[0] - e && b[0] > f.0[0] + e && a[1] < f.1[1] - e && b[1] > f.0[1] + e) {
                CellState::Unstructured
            } else {
                CellState::Structured
            };
            grid.state.push(st);
        }
    }
    Ok(grid)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::Elastic;

    fn plate_with_hole() -> Region {
        Region::new(Loop::rectangle(0.0, 0.0, 10.0, 4.0).unwrap(), vec![Loop::circle([5.0, 2.0], 0.5, "hole").unwrap()], Elastic::new(1e7, 0.3)).unwrap()
    }

    #[test]
    fn the_cells_around_a_hole_are_unstructured_and_the_ends_structured() {
        let g = classify_2d(&plate_with_hole(), &|_| 0.25, &PartitionOptions2d::default()).unwrap();
        assert!(g.count(CellState::Structured) > 0 && g.count(CellState::Unstructured) > 0);
        assert_eq!(g.count(CellState::Void), 0);
        // the hole is at x in [4.5, 5.5]: with 2.5 * 0.25 = 0.625 padding every cell touching [3.875, 6.125] x [0.875, 3.125] is unstructured
        for cj in 0..g.ny() {
            for ci in 0..g.nx() {
                let (a, b) = g.cell_box(ci, cj);
                let overlaps = a[0] < 6.125 - 1e-9 && b[0] > 3.875 + 1e-9 && a[1] < 3.125 - 1e-9 && b[1] > 0.875 + 1e-9;
                if overlaps {
                    assert_eq!(g.at(ci, cj), CellState::Unstructured);
                }
            }
        }
        // the far left end is structured, and cells tile the plate exactly
        assert_eq!(g.at(0, 0), CellState::Structured);
        let total = g.structured_area() + g.unstructured_area();
        assert!((total - 40.0).abs() < 1e-9, "{total}");
        // long intervals were subdivided to follow the 0.25 size: no cell is longer than 6 * 0.25
        assert!(g.xs.windows(2).all(|w| w[1] - w[0] <= 1.5 + 1e-9));
    }

    #[test]
    fn an_l_shaped_plate_has_void_cells_and_a_reflex_corner_box() {
        let outer = Loop::polygon(&[[0.0, 0.0], [6.0, 0.0], [6.0, 2.0], [2.0, 2.0], [2.0, 6.0], [0.0, 6.0]], "o").unwrap();
        let r = Region::new(outer, vec![], Elastic::new(1e7, 0.3)).unwrap();
        let g = classify_2d(&r, &|_| 0.5, &PartitionOptions2d::default()).unwrap();
        assert!(g.count(CellState::Void) > 0);
        assert!((g.structured_area() + g.unstructured_area() - 20.0).abs() < 1e-9);
        // the reflex corner (2, 2) is padded by 2.5 * 0.5 = 1.25: the cell right at the corner is unstructured
        let corner_cell = (0..g.nx()).flat_map(|ci| (0..g.ny()).map(move |cj| (ci, cj))).find(|&(ci, cj)| {
            let (a, b) = g.cell_box(ci, cj);
            a[0] < 2.0 && b[0] > 1.5 && a[1] < 2.0 && b[1] > 1.5 && g.at(ci, cj) != CellState::Void
        });
        assert_eq!(g.at(corner_cell.unwrap().0, corner_cell.unwrap().1), CellState::Unstructured);
        // the arm ends are structured
        assert!(g.count(CellState::Structured) > 0);
    }

    #[test]
    fn a_curved_outer_boundary_is_unsupported() {
        let outer = Loop::circle([0.0, 0.0], 1.0, "o").unwrap();
        let r = Region::new(outer, vec![], Elastic::new(1e7, 0.3)).unwrap();
        assert!(matches!(classify_2d(&r, &|_| 0.1, &PartitionOptions2d::default()), Err(MeshError::Unsupported(_))));
    }

    #[test]
    fn extra_boxes_force_unstructured_cells() {
        let r = Region::new(Loop::rectangle(0.0, 0.0, 10.0, 2.0).unwrap(), vec![], Elastic::new(1e7, 0.3)).unwrap();
        let none = classify_2d(&r, &|_| 0.5, &PartitionOptions2d::default()).unwrap();
        assert_eq!(none.count(CellState::Unstructured), 0);
        let opt = PartitionOptions2d { extra_boxes: vec![([9.5, 0.0], [10.0, 2.0])], ..Default::default() };
        let g = classify_2d(&r, &|_| 0.5, &opt).unwrap();
        assert!(g.count(CellState::Unstructured) > 0 && g.at(0, 0) == CellState::Structured);
    }

    #[test]
    fn a_bad_size_function_is_reported() {
        let r = Region::new(Loop::rectangle(0.0, 0.0, 1.0, 1.0).unwrap(), vec![], Elastic::new(1e7, 0.3)).unwrap();
        assert!(classify_2d(&r, &|_| 0.0, &PartitionOptions2d::default()).is_err());
        assert!(classify_2d(&r, &|_| f64::NAN, &PartitionOptions2d::default()).is_err());
    }
}
