//! Geometry-aware classification of a planar region into structured and unstructured cells.
//!
//! The domain's bounding box is cut into a tensor grid of coarse cells by (a) the coordinates of the outer boundary's
//! vertices and (b) the padded boxes of every geometric feature that makes a structured block unattractive: holes (any
//! shape), reflex corners of the outer boundary, and caller-supplied boxes (load and restraint locations, contact patches).
//! A cell outside the outer boundary is void; a cell overlapping a padded feature box is unstructured; every other cell is a
//! regular rectangle and structured. Long intervals are subdivided so a structured block follows the size function instead of
//! being one uniform-size cell.
//!
//! An outer boundary that is not made of axis-aligned lines (angled edges, arcs) cannot be tiled by rectangles. The cells it
//! passes through, and those within [`BOUNDARY_PAD`] local element sizes of it, are unstructured instead: the structured
//! blocks stay a buffer away from the curve and a thin unstructured layer, which follows the true boundary exactly, fills the
//! gap. Strips holding such cells are subdivided finer ([`BOUNDARY_SPAN`]) so that layer is a few elements thick, not a
//! coarse cell.

use super::MeshError;
use crate::geometry::{Curve, Loop, Region};

/// Clearance, in local element sizes, between a structured cell and an outer boundary that is not axis-aligned.
pub const BOUNDARY_PAD: f64 = 1.0;
/// Longest interval, in local element sizes, of a grid strip that holds cells next to such a boundary.
pub const BOUNDARY_SPAN: f64 = 2.5;

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
    pub fn set(&mut self, ci: usize, cj: usize, s: CellState) {
        let nx = self.nx();
        self.state[cj * nx + ci] = s;
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

    /// Make every structured cell that touches an unstructured one at a corner only (the other two cells around the grid
    /// vertex being unstructured too) unstructured: the boundary of the unstructured cells would otherwise touch itself at that
    /// vertex, and a region whose boundary does so cannot be meshed. The smaller cell of the two is given up.
    pub fn remove_pinches(&mut self) {
        let (nx, ny) = (self.nx(), self.ny());
        let area = |g: &Self, ci: usize, cj: usize| (g.xs[ci + 1] - g.xs[ci]) * (g.ys[cj + 1] - g.ys[cj]);
        loop {
            let mut changed = false;
            for j in 1..ny {
                for i in 1..nx {
                    let (a, b, c, d) = ((i - 1, j - 1), (i, j - 1), (i, j), (i - 1, j));
                    let (sa, sb, sc, sd) = (self.at(a.0, a.1), self.at(b.0, b.1), self.at(c.0, c.1), self.at(d.0, d.1));
                    let (u, s) = (CellState::Unstructured, CellState::Structured);
                    let give_up = if sa == u && sc == u && sb == s && sd == s {
                        Some([b, d])
                    } else if sb == u && sd == u && sa == s && sc == s {
                        Some([a, c])
                    } else {
                        None
                    };
                    if let Some([p, q]) = give_up {
                        let smaller = if area(self, p.0, p.1) <= area(self, q.0, q.1) { p } else { q };
                        self.state[smaller.1 * nx + smaller.0] = u;
                        changed = true;
                    }
                }
            }
            if !changed {
                break;
            }
        }
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

/// Whether `c` is a straight line along an axis (it then lies on a grid line and bounds a cell exactly).
pub fn is_axis_aligned(c: &Curve, tol: f64) -> bool {
    matches!(*c, Curve::Line { a, b } if (a[0] - b[0]).abs() <= tol || (a[1] - b[1]).abs() <= tol)
}

/// Parameters in the open interval `(0, 1)` where `c` crosses the line `x = v` (`axis` 0) or `y = v` (`axis` 1).
pub fn crossings(c: &Curve, axis: usize, v: f64) -> Vec<f64> {
    match *c {
        Curve::Line { a, b } => {
            let d = b[axis] - a[axis];
            if d == 0.0 {
                return Vec::new();
            }
            let t = (v - a[axis]) / d;
            if t > 0.0 && t < 1.0 {
                vec![t]
            } else {
                Vec::new()
            }
        }
        Curve::Arc { c: ctr, r, a0, a1 } => {
            let q = (v - ctr[axis]) / r;
            if q.abs() > 1.0 {
                return Vec::new();
            }
            let sweep = a1 - a0;
            // the two angles with this cosine (axis 0) or sine (axis 1)
            let base = if axis == 0 { q.acos() } else { q.asin() };
            let angles = if axis == 0 { [base, -base] } else { [base, std::f64::consts::PI - base] };
            let mut out = Vec::new();
            for th in angles {
                let delta = if sweep >= 0.0 { th - a0 } else { a0 - th }.rem_euclid(std::f64::consts::TAU);
                let t = delta / sweep.abs();
                if t > 0.0 && t < 1.0 {
                    out.push(t);
                }
            }
            out
        }
    }
}

/// Whether `c` has a point in the open box `(lo, hi)`; running along or touching the box boundary does not count.
pub fn enters_open_box(c: &Curve, lo: [f64; 2], hi: [f64; 2], tol: f64) -> bool {
    let strictly_inside = |p: [f64; 2]| p[0] > lo[0] + tol && p[0] < hi[0] - tol && p[1] > lo[1] + tol && p[1] < hi[1] - tol;
    match *c {
        Curve::Line { a, b } => {
            // clip to the closed box (Liang-Barsky); the part inside is a line segment, so its midpoint is inside the open box
            // exactly when the line passes through the interior
            let (mut t0, mut t1) = (0.0f64, 1.0f64);
            for i in 0..2 {
                let d = b[i] - a[i];
                for (p, q) in [(-d, a[i] - lo[i]), (d, hi[i] - a[i])] {
                    if p == 0.0 {
                        if q < 0.0 {
                            return false;
                        }
                    } else {
                        let r = q / p;
                        if p < 0.0 {
                            t0 = t0.max(r);
                        } else {
                            t1 = t1.min(r);
                        }
                    }
                }
            }
            t0 < t1 && strictly_inside(c.point(0.5 * (t0 + t1)))
        }
        Curve::Arc { .. } => {
            // split the arc where it crosses the four box lines; between two crossings it is wholly inside or outside
            let mut ts = vec![0.0, 1.0];
            for axis in 0..2 {
                for l in [lo[axis], hi[axis]] {
                    ts.extend(crossings(c, axis, l));
                }
            }
            ts.sort_by(f64::total_cmp);
            ts.windows(2).any(|w| w[1] > w[0] && strictly_inside(c.point(0.5 * (w[0] + w[1]))))
        }
    }
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
    let segs = &region.outer.segments;
    let verts: Vec<[f64; 2]> = segs.iter().map(|s| s.curve.start()).collect();
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
    let n = segs.len();
    for k in 0..n {
        // the outer loop is counter-clockwise: turning clockwise at a junction is a reflex corner
        let (t_in, t_out) = (segs[(k + n - 1) % n].curve.tangent(1.0), segs[k].curve.tangent(0.0));
        if t_in[0] * t_out[1] - t_in[1] * t_out[0] < -1e-9 {
            boxes.push((verts[k], verts[k])); // reflex corner
        }
    }
    // Pieces of the outer boundary that are not on a grid line: the cells next to them cannot be regular.
    let free: Vec<Curve> = segs.iter().map(|s| s.curve).filter(|c| !is_axis_aligned(c, scale_tol)).collect();
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
    let xs = merge_lines(xl, eps);
    let ys = merge_lines(yl, eps);

    // State of the cell `(a, b)` and whether it is next to the boundary: `Cut` cells are the ones the boundary passes through
    // or comes within `BOUNDARY_PAD` sizes of. A cell is `Void` only if no part of the domain is in it.
    let e = 1e-9 * diag;
    let cell_state = |a: [f64; 2], b: [f64; 2]| -> (CellState, bool) {
        let centre = [0.5 * (a[0] + b[0]), 0.5 * (a[1] + b[1])];
        let pad = BOUNDARY_PAD * size(centre);
        let near = free.iter().any(|c| enters_open_box(c, [a[0] - pad, a[1] - pad], [b[0] + pad, b[1] + pad], e));
        let through = near && free.iter().any(|c| enters_open_box(c, a, b, e));
        if !through && !region.outer.contains(centre) {
            (CellState::Void, false)
        } else if near || padded.iter().any(|f| a[0] < f.1[0] - e && b[0] > f.0[0] + e && a[1] < f.1[1] - e && b[1] > f.0[1] + e) {
            (CellState::Unstructured, near)
        } else {
            (CellState::Structured, false)
        }
    };
    let states_of = |xs: &[f64], ys: &[f64]| -> Vec<(CellState, bool)> {
        let mut v = Vec::with_capacity((xs.len() - 1) * (ys.len() - 1));
        for cj in 0..ys.len() - 1 {
            for ci in 0..xs.len() - 1 {
                v.push(cell_state([xs[ci], ys[cj]], [xs[ci + 1], ys[cj + 1]]));
            }
        }
        v
    };
    let (cnx, cny) = (xs.len() - 1, ys.len() - 1);
    let coarse = states_of(&xs, &ys);

    // Subdivide intervals so the size function is nearly constant across each: an interval is bisected while it is longer than
    // `span` local sizes or the size varies by more than `SIZE_RATIO` along it. A coarse cell is later divided uniformly at its
    // smallest sample, so a wide cell over a graded size field would be over-resolved. Only the cells that will carry regular
    // elements, or lie next to the boundary, count: the unstructured cells around a feature are meshed freely, and the fine size
    // there must not slice the regular blocks far from it into slivers.
    const SIZE_RATIO: f64 = 1.4;
    let size_along = |t: f64, others: &[(f64, f64)], horizontal: bool| -> f64 {
        let mut h = f64::INFINITY;
        for &(o0, o1) in others {
            for s in 0..5 {
                let o = o0 + (o1 - o0) * s as f64 / 4.0;
                h = h.min(size(if horizontal { [t, o] } else { [o, t] }));
            }
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
    // `cells(k)`: the cells of strip `k` as (extent along the other axis, next to the boundary).
    let split = |lines: &[f64], cells: &dyn Fn(usize) -> Vec<((f64, f64), bool)>, horizontal: bool| -> Vec<f64> {
        let mut out = vec![lines[0]];
        for (k, w) in lines.windows(2).enumerate() {
            let c = cells(k);
            if c.is_empty() {
                out.push(w[1]);
            } else {
                let others: Vec<(f64, f64)> = c.iter().map(|x| x.0).collect();
                let span = if c.iter().any(|x| x.1) { opt.span.min(BOUNDARY_SPAN) } else { opt.span };
                bisect(w[0], w[1], 0, &|t| size_along(t, &others, horizontal), span, &mut out);
            }
        }
        out
    };
    let carries = |st: &(CellState, bool)| st.0 == CellState::Structured || (st.0 == CellState::Unstructured && st.1);
    let columns = |ci: usize| -> Vec<((f64, f64), bool)> { (0..cny).filter(|&cj| carries(&coarse[cj * cnx + ci])).map(|cj| ((ys[cj], ys[cj + 1]), coarse[cj * cnx + ci].1)).collect() };
    let rows = |cj: usize| -> Vec<((f64, f64), bool)> { (0..cnx).filter(|&ci| carries(&coarse[cj * cnx + ci])).map(|ci| ((xs[ci], xs[ci + 1]), coarse[cj * cnx + ci].1)).collect() };
    let (new_xs, new_ys) = (split(&xs, &columns, true), split(&ys, &rows, false));
    let state = states_of(&new_xs, &new_ys).into_iter().map(|s| s.0).collect();
    let mut grid = CellGrid { xs: new_xs, ys: new_ys, state };
    grid.remove_pinches();
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
    fn structured_cells_far_from_a_fine_hole_are_not_sliced_by_its_size() {
        // The size shrinks to 1/20 of the base next to the hole. The grid lines that resolve that belong to the cells around the
        // hole; the regular end blocks keep their coarse cells (they used to be cut along the strip into slivers 20 times thinner
        // than they are wide, which the size-jump gate then rejected).
        let r = Region::new(Loop::rectangle(0.0, 0.0, 12.0, 4.0).unwrap(), vec![Loop::circle([6.0, 2.0], 0.5, "hole").unwrap()], Elastic::new(1e7, 0.3)).unwrap();
        let size = |p: [f64; 2]| {
            let d = ((p[0] - 6.0).hypot(p[1] - 2.0) - 0.5).max(0.0);
            0.5 * (0.05 + 0.95 * (d / 1.0).min(1.0))
        };
        let g = classify_2d(&r, &size, &PartitionOptions2d::default()).unwrap();
        let mut thinnest = f64::INFINITY;
        for cj in 0..g.ny() {
            for ci in 0..g.nx() {
                if g.at(ci, cj) == CellState::Structured {
                    let (a, b) = g.cell_box(ci, cj);
                    thinnest = thinnest.min((b[0] - a[0]).min(b[1] - a[1]));
                }
            }
        }
        assert!(thinnest > 0.2, "a structured cell is only {thinnest} thick");
        assert!((g.structured_area() + g.unstructured_area() - 48.0).abs() < 1e-9);
    }

    #[test]
    fn a_circular_boundary_keeps_the_regular_cells_a_margin_inside() {
        let r = Region::new(Loop::circle([0.0, 0.0], 5.0, "o").unwrap(), vec![], Elastic::new(1e7, 0.3)).unwrap();
        let g = classify_2d(&r, &|_| 0.25, &PartitionOptions2d::default()).unwrap();
        assert!(g.count(CellState::Structured) > 0 && g.count(CellState::Unstructured) > 0 && g.count(CellState::Void) > 0);
        for cj in 0..g.ny() {
            for ci in 0..g.nx() {
                let (a, b) = g.cell_box(ci, cj);
                let far = [[a[0], a[1]], [b[0], a[1]], [b[0], b[1]], [a[0], b[1]]].iter().map(|c| c[0].hypot(c[1])).fold(0.0f64, f64::max);
                let near = [[a[0], a[1]], [b[0], a[1]], [b[0], b[1]], [a[0], b[1]]].iter().map(|c| c[0].hypot(c[1])).fold(f64::INFINITY, f64::min);
                match g.at(ci, cj) {
                    // a regular cell is a full pad (one local size) inside the circle
                    CellState::Structured => assert!(far <= 5.0 - 0.25 + 1e-9, "structured cell reaching radius {far}"),
                    // a void cell has no part of the disc
                    CellState::Void => assert!(near >= 5.0 - 1e-9 || (a[0] * b[0] > 0.0 && a[1] * b[1] > 0.0 && near >= 5.0 - 1e-9), "void cell at radius {near}"),
                    CellState::Unstructured => {}
                }
            }
        }
        // the strips next to the circle are subdivided to a few local sizes, so the unstructured layer is thin
        assert!(g.xs.windows(2).all(|w| w[1] - w[0] <= 2.5 * 0.25 + 1e-9), "{:?}", g.xs);
    }

    #[test]
    fn an_angled_edge_cuts_cells_and_leaves_the_axis_aligned_edges_alone() {
        let outer = Loop::polygon(&[[0.0, 0.0], [10.0, 0.0], [8.0, 4.0], [2.0, 4.0]], "o").unwrap();
        let r = Region::new(outer, vec![], Elastic::new(1e7, 0.3)).unwrap();
        let g = classify_2d(&r, &|_| 0.4, &PartitionOptions2d::default()).unwrap();
        // the cells tile the bounding box; the corners cut off by the legs are void, the cells along the legs unstructured
        assert!(g.count(CellState::Void) > 0 && g.count(CellState::Unstructured) > 0);
        // a cell on the base, in the middle, touches the axis-aligned boundary and is regular
        let mid = g.xs.iter().position(|&x| x >= 5.0).unwrap();
        assert_eq!(g.at(mid, 0), CellState::Structured);
        // a regular cell keeps its distance from the legs: all four corners are at least one local size from both lines
        for cj in 0..g.ny() {
            for ci in 0..g.nx() {
                if g.at(ci, cj) == CellState::Structured {
                    let (a, b) = g.cell_box(ci, cj);
                    for c in [[a[0], a[1]], [b[0], a[1]], [b[0], b[1]], [a[0], b[1]]] {
                        // left leg x = y / 2, right leg x = 10 - y / 2
                        let d_left = (c[0] - c[1] / 2.0) * 2.0 / 5.0f64.sqrt();
                        let d_right = (10.0 - c[1] / 2.0 - c[0]) * 2.0 / 5.0f64.sqrt();
                        assert!(d_left >= 0.4 - 1e-9 && d_right >= 0.4 - 1e-9, "structured corner {c:?} is {d_left}/{d_right} from the legs");
                    }
                }
            }
        }
    }

    #[test]
    fn a_checkerboard_corner_gives_up_the_smaller_structured_cell() {
        // Unstructured cells at (0, 0) and (1, 1) meet the structured ones at (1, 0) and (0, 1) in a single grid vertex: the
        // unstructured boundary would touch itself there.
        use CellState::{Structured as S, Unstructured as U};
        let mut g = CellGrid { xs: vec![0.0, 1.0, 3.0], ys: vec![0.0, 2.0, 3.0], state: vec![U, S, S, U] };
        g.remove_pinches();
        // (1, 0) is 2 x 2, (0, 1) is 1 x 1: the small one goes
        assert_eq!(g.state, vec![U, S, U, U]);
        // a grid without a pinch is left alone
        let mut ok = CellGrid { xs: vec![0.0, 1.0, 2.0], ys: vec![0.0, 1.0, 2.0], state: vec![U, U, S, S] };
        ok.remove_pinches();
        assert_eq!(ok.state, vec![U, U, S, S]);
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
