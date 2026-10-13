//! Element quality metrics for any element mix and the report the mesher, the optimiser and the adaptation controller judge a
//! mesh by.
//!
//! - **Scaled Jacobian** in `[-1, 1]` (`1` = ideal shape, `<= 0` = inverted or degenerate): the determinant of the corner
//!   Jacobian with unit-length columns, taken at every corner and normalised so the regular triangle / tetrahedron reads `1`.
//!   Simplices use the straight corner edges (the metric judges the corner element); tensor-product elements use the
//!   isoparametric Jacobian, so curved quadratic elements are judged on their real map.
//! - **Edge ratio** longest / shortest corner edge.
//! - **Minimum / maximum angle** and **equiangle skew** (2D elements).
//! - **Radius ratio** `3 r_in / R_circ` for tetrahedra (`1` for the regular one).

#![allow(clippy::needless_range_loop)] // index-parallel numeric kernels read clearer as loops

use crate::element::ElementKind;
use crate::mesh::Mesh;
use rayon::prelude::*;

/// Quality of one element.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ElementQuality {
    pub scaled_jacobian: f64,
    pub edge_ratio: f64,
    /// Smallest / largest corner angle in degrees (2D elements; `NAN` in 3D).
    pub min_angle: f64,
    pub max_angle: f64,
    /// Equiangle skew in `[0, 1]` (`0` ideal; 2D elements; `NAN` in 3D).
    pub skew: f64,
    /// Tetrahedron radius ratio; `NAN` for other kinds.
    pub radius_ratio: f64,
    /// Mean corner edge length.
    pub size: f64,
}

/// Quality limits for acceptance.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QualityThresholds {
    pub min_scaled_jacobian: f64,
    pub max_edge_ratio: f64,
    /// Largest accepted ratio of neighbouring element sizes (checked when the report carries the progression).
    pub max_size_jump: f64,
}

impl Default for QualityThresholds {
    fn default() -> Self {
        Self { min_scaled_jacobian: 0.1, max_edge_ratio: 50.0, max_size_jump: 6.0 }
    }
}

/// Quality of a whole mesh, element by element (global element order) and summarised.
#[derive(Debug, Clone)]
pub struct QualityReport {
    pub elements: Vec<ElementQuality>,
    /// Elements with `scaled_jacobian <= 0`.
    pub inverted: usize,
    /// First inverted element as `(block, element)`.
    pub first_inverted: Option<(usize, usize)>,
    pub min_scaled_jacobian: f64,
    pub mean_scaled_jacobian: f64,
    pub max_edge_ratio: f64,
    /// Count of elements in ten equal bins of the scaled Jacobian over `[0, 1]` (negative values count in the first bin).
    pub histogram: [usize; 10],
    /// Largest ratio of neighbouring element sizes (element-size progression); `1` for a uniform mesh. Filled by
    /// [`QualityReport::with_progression`].
    pub max_size_jump: f64,
}

impl QualityReport {
    pub fn acceptable(&self, thr: &QualityThresholds) -> bool {
        self.inverted == 0 && self.min_scaled_jacobian >= thr.min_scaled_jacobian && self.max_edge_ratio <= thr.max_edge_ratio && self.max_size_jump <= thr.max_size_jump
    }

    /// `Ok` if acceptable, else the reason as a typed error.
    pub fn require(&self, thr: &QualityThresholds) -> Result<(), super::MeshError> {
        if let Some(first) = self.first_inverted {
            return Err(super::MeshError::InvalidElements { count: self.inverted, first });
        }
        if self.acceptable(thr) {
            Ok(())
        } else {
            Err(super::MeshError::Failed(format!(
                "mesh quality below the limits: minimum scaled Jacobian {:.3} (need {:.3}), largest edge ratio {:.1} (limit {:.1}), largest size jump {:.1} (limit {:.1})",
                self.min_scaled_jacobian, thr.min_scaled_jacobian, self.max_edge_ratio, thr.max_edge_ratio, self.max_size_jump, thr.max_size_jump
            )))
        }
    }

    /// Add the element-size progression over face neighbours.
    pub fn with_progression(mut self, adj: &super::adjacency::Adjacency) -> Self {
        let mut worst: f64 = 1.0;
        for e in 0..self.elements.len() {
            for &m in adj.neighbours(e) {
                let (a, b) = (self.elements[e].size, self.elements[m as usize].size);
                if a > 0.0 && b > 0.0 {
                    worst = worst.max(a / b).max(b / a);
                }
            }
        }
        self.max_size_jump = worst;
        self
    }
}

const TRI_EDGES: [(usize, usize); 3] = [(0, 1), (1, 2), (2, 0)];
const QUAD_EDGES: [(usize, usize); 4] = [(0, 1), (1, 2), (2, 3), (3, 0)];
const TET_EDGES: [(usize, usize); 6] = [(0, 1), (0, 2), (0, 3), (1, 2), (1, 3), (2, 3)];
const HEX_EDGES: [(usize, usize); 12] = [(0, 1), (1, 2), (2, 3), (3, 0), (4, 5), (5, 6), (6, 7), (7, 4), (0, 4), (1, 5), (2, 6), (3, 7)];
const WEDGE_EDGES: [(usize, usize); 9] = [(0, 1), (1, 2), (2, 0), (3, 4), (4, 5), (5, 3), (0, 3), (1, 4), (2, 5)];

fn edges(kind: ElementKind) -> &'static [(usize, usize)] {
    match kind {
        ElementKind::Tri3 | ElementKind::Tri6 => &TRI_EDGES,
        ElementKind::Quad4 | ElementKind::Quad8 | ElementKind::Quad9 => &QUAD_EDGES,
        ElementKind::Tet4 | ElementKind::Tet10 => &TET_EDGES,
        ElementKind::Wedge6 | ElementKind::Wedge15 => &WEDGE_EDGES,
        _ => &HEX_EDGES,
    }
}

type V3 = [f64; 3];
fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn norm(a: V3) -> f64 {
    dot(a, a).sqrt()
}

/// Determinant of the unit-column matrix of `cols` (2 or 3 vectors; 2D uses the in-plane components).
fn unit_det(cols: &[V3], dim: usize) -> f64 {
    let u: Vec<V3> = cols.iter().map(|c| {
        let n = norm(if dim == 2 { [c[0], c[1], 0.0] } else { *c });
        if n > 0.0 { [c[0] / n, c[1] / n, if dim == 2 { 0.0 } else { c[2] / n }] } else { [0.0; 3] }
    }).collect();
    if dim == 2 {
        u[0][0] * u[1][1] - u[0][1] * u[1][0]
    } else {
        dot(u[0], cross(u[1], u[2]))
    }
}

/// Signed measure sign of a straight simplex (triangle: area sign; tetrahedron: volume sign).
fn simplex_orientation(p: &[V3], dim: usize) -> f64 {
    if dim == 2 {
        (p[1][0] - p[0][0]) * (p[2][1] - p[0][1]) - (p[1][1] - p[0][1]) * (p[2][0] - p[0][0])
    } else {
        dot(sub(p[1], p[0]), cross(sub(p[2], p[0]), sub(p[3], p[0])))
    }
}

/// Angle in degrees between the edges from `a` to `b` and from `a` to `c`.
fn angle(a: V3, b: V3, c: V3, dim: usize) -> f64 {
    let (u, v) = (sub(b, a), sub(c, a));
    let (u, v) = if dim == 2 { ([u[0], u[1], 0.0], [v[0], v[1], 0.0]) } else { (u, v) };
    let cs = (dot(u, v) / (norm(u) * norm(v))).clamp(-1.0, 1.0);
    cs.acos().to_degrees()
}

/// Quality of the element `conn` of `kind` (nodes `xyz`, the full node list in element order).
pub fn element_quality(kind: ElementKind, xyz: &[V3]) -> ElementQuality {
    let dim = kind.dim();
    let nc = kind.n_corners();
    let corners = &xyz[..nc];
    let (mut lmin, mut lmax, mut lsum) = (f64::INFINITY, 0.0f64, 0.0);
    for &(a, b) in edges(kind) {
        let l = norm(sub(corners[b], corners[a]));
        lmin = lmin.min(l);
        lmax = lmax.max(l);
        lsum += l;
    }
    let n_edges = edges(kind).len() as f64;
    let edge_ratio = if lmin > 0.0 { lmax / lmin } else { f64::INFINITY };

    let scaled_jacobian = if kind.is_simplex() {
        let orient = simplex_orientation(corners, dim);
        let ideal = if dim == 2 { 3f64.sqrt() / 2.0 } else { 0.5f64.sqrt() };
        let mut worst = f64::INFINITY;
        for i in 0..nc {
            let others: Vec<V3> = (0..nc).filter(|&j| j != i).map(|j| sub(corners[j], corners[i])).collect();
            let det = unit_det(&others, dim).abs();
            worst = worst.min(det);
        }
        let v = worst / ideal;
        if orient > 0.0 { v } else { -v }
    } else {
        let mut worst = f64::INFINITY;
        for c in 0..nc {
            let xi = kind.node_coords()[c];
            let (_, dn) = kind.shape(xi);
            let mut cols = [[0.0; 3]; 3];
            for a in 0..kind.n_nodes() {
                for k in 0..dim {
                    for r in 0..3 {
                        cols[k][r] += dn[a][k] * xyz[a][r];
                    }
                }
            }
            worst = worst.min(unit_det(&cols[..dim], dim));
        }
        // a right prism on an equilateral triangle has the sine of the triangle's corner as its best value
        if matches!(kind, ElementKind::Wedge6 | ElementKind::Wedge15) { worst / (3f64.sqrt() / 2.0) } else { worst }
    };

    let (mut min_angle, mut max_angle, mut skew) = (f64::NAN, f64::NAN, f64::NAN);
    if dim == 2 {
        let mut angles = Vec::with_capacity(nc);
        for i in 0..nc {
            angles.push(angle(corners[i], corners[(i + 1) % nc], corners[(i + nc - 1) % nc], 2));
        }
        min_angle = angles.iter().cloned().fold(f64::INFINITY, f64::min);
        max_angle = angles.iter().cloned().fold(0.0, f64::max);
        let ideal = if nc == 3 { 60.0 } else { 90.0 };
        skew = ((max_angle - ideal) / (180.0 - ideal)).max((ideal - min_angle) / ideal).max(0.0);
    }

    let mut radius_ratio = f64::NAN;
    if matches!(kind, ElementKind::Tet4 | ElementKind::Tet10) {
        let vol = simplex_orientation(corners, 3).abs() / 6.0;
        let area = |a: usize, b: usize, c: usize| 0.5 * norm(cross(sub(corners[b], corners[a]), sub(corners[c], corners[a])));
        let surface = area(0, 1, 2) + area(0, 1, 3) + area(0, 2, 3) + area(1, 2, 3);
        let r_in = if surface > 0.0 { 3.0 * vol / surface } else { 0.0 };
        // circumradius from the opposite-edge products: 24 V R = sqrt(s (s-a)(s-b)(s-c)) * 4 for products a=|01||23| etc.
        let l = |a: usize, b: usize| norm(sub(corners[a], corners[b]));
        let (pa, pb, pc) = (l(0, 1) * l(2, 3), l(0, 2) * l(1, 3), l(0, 3) * l(1, 2));
        let s = 0.5 * (pa + pb + pc);
        let heron = (s * (s - pa) * (s - pb) * (s - pc)).max(0.0).sqrt();
        let r_circ = if vol > 0.0 { heron / (6.0 * vol) } else { f64::INFINITY };
        radius_ratio = if r_circ.is_finite() { 3.0 * r_in / r_circ } else { 0.0 };
    }

    ElementQuality { scaled_jacobian, edge_ratio, min_angle, max_angle, skew, radius_ratio, size: lsum / n_edges }
}

impl Mesh {
    /// Quality of every element (global element order) with the summary. Parallel over elements; each element is independent,
    /// so the result does not depend on the thread count.
    pub fn quality(&self) -> QualityReport {
        let mut index: Vec<(usize, usize)> = Vec::with_capacity(self.n_elems());
        for (b, blk) in self.blocks.iter().enumerate() {
            index.extend((0..blk.n_elems()).map(|e| (b, e)));
        }
        let one = |&(b, e): &(usize, usize)| {
            let blk = &self.blocks[b];
            let xyz: Vec<V3> = blk.elem(e).iter().map(|&n| self.nodes[n]).collect();
            element_quality(blk.kind, &xyz)
        };
        let elements: Vec<ElementQuality> = if index.len() >= 2000 { index.par_iter().map(one).collect() } else { index.iter().map(one).collect() };
        let (mut inverted, mut first, mut minj, mut sumj, mut maxr) = (0usize, None, f64::INFINITY, 0.0, 1.0f64);
        let mut histogram = [0usize; 10];
        for (q, &(b, e)) in elements.iter().zip(&index) {
            if q.scaled_jacobian.is_nan() || q.scaled_jacobian <= 0.0 {
                inverted += 1;
                first.get_or_insert((b, e));
            }
            minj = minj.min(q.scaled_jacobian);
            sumj += q.scaled_jacobian;
            maxr = maxr.max(q.edge_ratio);
            let bin = ((q.scaled_jacobian.clamp(0.0, 1.0) * 10.0) as usize).min(9);
            histogram[bin] += 1;
        }
        let n = elements.len().max(1) as f64;
        QualityReport { elements, inverted, first_inverted: first, min_scaled_jacobian: minj, mean_scaled_jacobian: sumj / n, max_edge_ratio: maxr, histogram, max_size_jump: 1.0 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generate::grid;
    use crate::mesh::{Elastic, Physics};

    const S3: f64 = 1.732_050_807_568_877_2;

    #[test]
    fn ideal_shapes_score_one() {
        let tri = element_quality(ElementKind::Tri3, &[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.5, S3 / 2.0, 0.0]]);
        assert!((tri.scaled_jacobian - 1.0).abs() < 1e-12 && (tri.edge_ratio - 1.0).abs() < 1e-12 && tri.skew.abs() < 1e-12);
        assert!((tri.min_angle - 60.0).abs() < 1e-9);
        let quad = element_quality(ElementKind::Quad4, &[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]]);
        assert!((quad.scaled_jacobian - 1.0).abs() < 1e-12 && quad.skew.abs() < 1e-12);
        let tet = element_quality(ElementKind::Tet4, &[[1.0, 1.0, 1.0], [1.0, -1.0, -1.0], [-1.0, 1.0, -1.0], [-1.0, -1.0, 1.0]]);
        assert!((tet.radius_ratio - 1.0).abs() < 1e-12, "{}", tet.radius_ratio);
        assert!((tet.scaled_jacobian.abs() - 1.0).abs() < 1e-12);
        let prism = element_quality(ElementKind::Wedge6, &[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.5, 3f64.sqrt() / 2.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 1.0], [0.5, 3f64.sqrt() / 2.0, 1.0]]);
        assert!((prism.scaled_jacobian - 1.0).abs() < 1e-12 && (prism.edge_ratio - 1.0).abs() < 1e-12, "{prism:?}");
        let hex = grid(Physics::Solid, ElementKind::Hex8, Elastic::new(1.0, 0.3), [1, 1, 1], &|p| p).unwrap().quality();
        assert!((hex.min_scaled_jacobian - 1.0).abs() < 1e-12);
    }

    #[test]
    fn degenerate_and_inverted_elements_are_flagged() {
        let sliver = element_quality(ElementKind::Tri3, &[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.5, 1e-4, 0.0]]);
        assert!(sliver.scaled_jacobian > 0.0 && sliver.scaled_jacobian < 1e-3 && sliver.skew > 0.99);
        let flipped = element_quality(ElementKind::Tri3, &[[0.0, 0.0, 0.0], [0.0, 1.0, 0.0], [1.0, 0.0, 0.0]]);
        assert!(flipped.scaled_jacobian < 0.0);
        let bowtie = element_quality(ElementKind::Quad4, &[[0.0, 0.0, 0.0], [1.0, 1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]);
        assert!(bowtie.scaled_jacobian <= 0.0);
        let flat_tet = element_quality(ElementKind::Tet4, &[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [1.0, 1.0, 0.0]]);
        assert!(flat_tet.scaled_jacobian.abs() < 1e-12 && flat_tet.radius_ratio.abs() < 1e-12);
    }

    #[test]
    fn a_report_counts_inverted_elements_and_fills_the_histogram() {
        let mut m = grid(Physics::PlaneStress { thickness: 1.0 }, ElementKind::Quad4, Elastic::new(1.0, 0.3), [3, 3, 1], &|p| p).unwrap();
        let ok = m.quality();
        assert_eq!(ok.inverted, 0);
        assert!(ok.acceptable(&QualityThresholds::default()));
        assert_eq!(ok.histogram[9], 9);
        assert!((ok.max_size_jump - 1.0).abs() < 1e-12);
        // pull the first interior node across its neighbours
        m.nodes[5] = [0.9, 0.9, 0.0];
        let bad = m.quality();
        assert!(bad.inverted > 0 && bad.first_inverted.is_some());
        assert!(matches!(bad.require(&QualityThresholds::default()), Err(crate::meshing::MeshError::InvalidElements { .. })));
    }

    #[test]
    fn curved_quadratic_elements_are_judged_on_their_real_map() {
        // a quarter annulus of Quad8: well shaped, positive everywhere
        let m = grid(Physics::PlaneStress { thickness: 1.0 }, ElementKind::Quad8, Elastic::new(1.0, 0.3), [4, 2, 1], &|p| {
            let (r, th) = (1.0 + p[0], p[1] * std::f64::consts::FRAC_PI_2);
            [r * th.cos(), r * th.sin(), 0.0]
        })
        .unwrap();
        let q = m.quality();
        assert_eq!(q.inverted, 0);
        assert!(q.min_scaled_jacobian > 0.7, "{}", q.min_scaled_jacobian);
    }

    #[test]
    fn size_progression_sees_a_graded_mesh() {
        let m = grid(Physics::PlaneStress { thickness: 1.0 }, ElementKind::Quad4, Elastic::new(1.0, 0.3), [4, 1, 1], &|p| [p[0] * p[0], p[1] * 0.01, 0.0]).unwrap();
        let q = m.quality().with_progression(&m.adjacency());
        assert!(q.max_size_jump > 1.5, "{}", q.max_size_jump);
    }
}
