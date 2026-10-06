//! Element library: shape functions and quadrature, tabulated once per element type.
//!
//! Node order follows VTK (corners, then edge midsides, then face centres, then the volume
//! centre), so a mesh writes to `.vtu` without renumbering and Gmsh/Abaqus imports need one fixed
//! permutation per type. Shape functions are evaluated at the Gauss points once and kept in a
//! static table: the element kernels never evaluate a polynomial in the hot loop.

#![allow(clippy::needless_range_loop)] // index-parallel numeric kernels read clearer as loops

use std::sync::OnceLock;

/// Largest node count (Hex27).
pub const MAX_NODES: usize = 27;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ElementKind {
    Tri3,
    Tri6,
    Quad4,
    Quad8,
    Quad9,
    Tet4,
    Tet10,
    Hex8,
    Hex20,
    Hex27,
}

pub const ALL_KINDS: [ElementKind; 10] = [
    ElementKind::Tri3,
    ElementKind::Tri6,
    ElementKind::Quad4,
    ElementKind::Quad8,
    ElementKind::Quad9,
    ElementKind::Tet4,
    ElementKind::Tet10,
    ElementKind::Hex8,
    ElementKind::Hex20,
    ElementKind::Hex27,
];

impl ElementKind {
    pub fn dim(self) -> usize {
        match self {
            Self::Tri3 | Self::Tri6 | Self::Quad4 | Self::Quad8 | Self::Quad9 => 2,
            _ => 3,
        }
    }

    pub fn n_nodes(self) -> usize {
        match self {
            Self::Tri3 => 3,
            Self::Tri6 => 6,
            Self::Quad4 => 4,
            Self::Quad8 => 8,
            Self::Quad9 => 9,
            Self::Tet4 => 4,
            Self::Tet10 => 10,
            Self::Hex8 => 8,
            Self::Hex20 => 20,
            Self::Hex27 => 27,
        }
    }

    /// Simplex (triangle / tetrahedron) rather than tensor-product element.
    pub fn is_simplex(self) -> bool {
        matches!(self, Self::Tri3 | Self::Tri6 | Self::Tet4 | Self::Tet10)
    }

    /// Number of corner nodes (the first `n_corners` nodes).
    pub fn n_corners(self) -> usize {
        match self {
            Self::Tri3 | Self::Tri6 => 3,
            Self::Quad4 | Self::Quad8 | Self::Quad9 | Self::Tet4 | Self::Tet10 => 4,
            _ => 8,
        }
    }

    /// VTK cell type id.
    pub fn vtk_type(self) -> u8 {
        match self {
            Self::Tri3 => 5,
            Self::Tri6 => 22,
            Self::Quad4 => 9,
            Self::Quad8 => 23,
            Self::Quad9 => 28,
            Self::Tet4 => 10,
            Self::Tet10 => 24,
            Self::Hex8 => 12,
            Self::Hex20 => 25,
            Self::Hex27 => 29,
        }
    }

    /// Natural coordinates of the nodes (unused components are zero).
    pub fn node_coords(self) -> &'static [[f64; 3]] {
        match self {
            Self::Tri3 => &TRI6[..3],
            Self::Tri6 => &TRI6,
            Self::Quad4 => &QUAD9[..4],
            Self::Quad8 => &QUAD9[..8],
            Self::Quad9 => &QUAD9,
            Self::Tet4 => &TET10[..4],
            Self::Tet10 => &TET10,
            Self::Hex8 => &HEX27[..8],
            Self::Hex20 => &HEX27[..20],
            Self::Hex27 => &HEX27,
        }
    }

    /// Tabulated shape functions and quadrature (computed on first use).
    pub fn table(self) -> &'static ShapeTable {
        static TABLES: [OnceLock<ShapeTable>; 10] = [const { OnceLock::new() }; 10];
        let idx = ALL_KINDS.iter().position(|k| *k == self).unwrap();
        TABLES[idx].get_or_init(|| ShapeTable::build(self))
    }

    /// Shape functions `n` and natural derivatives `dn[node][axis]` at `xi`.
    pub fn shape(self, xi: [f64; 3]) -> ([f64; MAX_NODES], [[f64; 3]; MAX_NODES]) {
        let mut n = [0.0; MAX_NODES];
        let mut dn = [[0.0; 3]; MAX_NODES];
        match self {
            Self::Tri3 | Self::Tri6 => simplex_shape(self, xi, &mut n, &mut dn),
            Self::Tet4 | Self::Tet10 => simplex_shape(self, xi, &mut n, &mut dn),
            Self::Quad4 | Self::Hex8 => multilinear_shape(self, xi, &mut n, &mut dn),
            Self::Quad9 | Self::Hex27 => lagrange_shape(self, xi, &mut n, &mut dn),
            Self::Quad8 | Self::Hex20 => serendipity_shape(self, xi, &mut n, &mut dn),
        }
        (n, dn)
    }
}

const TRI6: [[f64; 3]; 6] = [[0., 0., 0.], [1., 0., 0.], [0., 1., 0.], [0.5, 0., 0.], [0.5, 0.5, 0.], [0., 0.5, 0.]];

const QUAD9: [[f64; 3]; 9] = [
    [-1., -1., 0.],
    [1., -1., 0.],
    [1., 1., 0.],
    [-1., 1., 0.],
    [0., -1., 0.],
    [1., 0., 0.],
    [0., 1., 0.],
    [-1., 0., 0.],
    [0., 0., 0.],
];

const TET10: [[f64; 3]; 10] = [
    [0., 0., 0.],
    [1., 0., 0.],
    [0., 1., 0.],
    [0., 0., 1.],
    [0.5, 0., 0.],
    [0.5, 0.5, 0.],
    [0., 0.5, 0.],
    [0., 0., 0.5],
    [0.5, 0., 0.5],
    [0., 0.5, 0.5],
];

const HEX27: [[f64; 3]; 27] = [
    [-1., -1., -1.],
    [1., -1., -1.],
    [1., 1., -1.],
    [-1., 1., -1.],
    [-1., -1., 1.],
    [1., -1., 1.],
    [1., 1., 1.],
    [-1., 1., 1.],
    [0., -1., -1.],
    [1., 0., -1.],
    [0., 1., -1.],
    [-1., 0., -1.],
    [0., -1., 1.],
    [1., 0., 1.],
    [0., 1., 1.],
    [-1., 0., 1.],
    [-1., -1., 0.],
    [1., -1., 0.],
    [1., 1., 0.],
    [-1., 1., 0.],
    [-1., 0., 0.],
    [1., 0., 0.],
    [0., -1., 0.],
    [0., 1., 0.],
    [0., 0., -1.],
    [0., 0., 1.],
    [0., 0., 0.],
];

/// Edge (corner pair) of every midside node, Tri6 then Tet10 (nodes after the corners).
const TRI6_EDGES: [[usize; 2]; 3] = [[0, 1], [1, 2], [2, 0]];
const TET10_EDGES: [[usize; 2]; 6] = [[0, 1], [1, 2], [2, 0], [0, 3], [1, 3], [2, 3]];

/// Barycentric coordinates and their natural gradients: `L[0] = 1 - sum xi`, `L[a] = xi[a-1]`.
fn simplex_shape(kind: ElementKind, xi: [f64; 3], n: &mut [f64; MAX_NODES], dn: &mut [[f64; 3]; MAX_NODES]) {
    let d = kind.dim();
    let nc = d + 1;
    let mut l = [0.0; 4];
    let mut dl = [[0.0; 3]; 4];
    l[0] = 1.0;
    for a in 0..d {
        l[0] -= xi[a];
        l[a + 1] = xi[a];
        dl[0][a] = -1.0;
        dl[a + 1][a] = 1.0;
    }
    match kind {
        ElementKind::Tri3 | ElementKind::Tet4 => {
            n[..nc].copy_from_slice(&l[..nc]);
            dn[..nc].copy_from_slice(&dl[..nc]);
        }
        _ => {
            // Corner: L (2L - 1); midside: 4 La Lb.
            for a in 0..nc {
                n[a] = l[a] * (2.0 * l[a] - 1.0);
                for k in 0..d {
                    dn[a][k] = (4.0 * l[a] - 1.0) * dl[a][k];
                }
            }
            let edges: &[[usize; 2]] = if d == 2 { &TRI6_EDGES } else { &TET10_EDGES };
            for (e, &[p, q]) in edges.iter().enumerate() {
                let m = nc + e;
                n[m] = 4.0 * l[p] * l[q];
                for k in 0..d {
                    dn[m][k] = 4.0 * (l[p] * dl[q][k] + l[q] * dl[p][k]);
                }
            }
        }
    }
}

/// Bilinear / trilinear: `prod (1 + xi x_n) / 2^d`.
fn multilinear_shape(kind: ElementKind, xi: [f64; 3], n: &mut [f64; MAX_NODES], dn: &mut [[f64; 3]; MAX_NODES]) {
    let d = kind.dim();
    let scale = 0.5f64.powi(d as i32);
    for (a, x) in kind.node_coords().iter().enumerate() {
        let f: [f64; 3] = std::array::from_fn(|k| if k < d { 1.0 + xi[k] * x[k] } else { 1.0 });
        n[a] = scale * f[..d].iter().product::<f64>();
        for k in 0..d {
            let mut p = scale * x[k];
            for j in 0..d {
                if j != k {
                    p *= f[j];
                }
            }
            dn[a][k] = p;
        }
    }
}

/// 1D quadratic Lagrange function on the nodes `-1, 0, 1` for a node at `xn`; value and slope.
fn lag1(xn: f64, x: f64) -> (f64, f64) {
    if xn < -0.5 {
        (0.5 * x * (x - 1.0), x - 0.5)
    } else if xn > 0.5 {
        (0.5 * x * (x + 1.0), x + 0.5)
    } else {
        (1.0 - x * x, -2.0 * x)
    }
}

/// Biquadratic / triquadratic Lagrange: tensor product of the 1D functions.
fn lagrange_shape(kind: ElementKind, xi: [f64; 3], n: &mut [f64; MAX_NODES], dn: &mut [[f64; 3]; MAX_NODES]) {
    let d = kind.dim();
    for (a, x) in kind.node_coords().iter().enumerate() {
        let mut v = [(1.0, 0.0); 3];
        for k in 0..d {
            v[k] = lag1(x[k], xi[k]);
        }
        n[a] = v[..d].iter().map(|p| p.0).product();
        for k in 0..d {
            let mut p = v[k].1;
            for j in 0..d {
                if j != k {
                    p *= v[j].0;
                }
            }
            dn[a][k] = p;
        }
    }
}

/// Eight- and twenty-node serendipity elements.
fn serendipity_shape(kind: ElementKind, xi: [f64; 3], n: &mut [f64; MAX_NODES], dn: &mut [[f64; 3]; MAX_NODES]) {
    let d = kind.dim();
    for (a, x) in kind.node_coords().iter().enumerate() {
        let corner = x[..d].iter().all(|c| c.abs() > 0.5);
        // f_k = 1 + xi_k x_k (corner or the axis the node lies on the edge of), s = sum xi_k x_k
        let f: [f64; 3] = std::array::from_fn(|k| if k < d { 1.0 + xi[k] * x[k] } else { 1.0 });
        let s: f64 = (0..d).map(|k| xi[k] * x[k]).sum();
        let norm = if d == 2 { 0.25 } else { 0.125 };
        if corner {
            // norm * prod f * (s - (d - 1))
            let prod: f64 = f[..d].iter().product();
            n[a] = norm * prod * (s - (d as f64 - 1.0));
            for k in 0..d {
                let mut pk = x[k];
                for j in 0..d {
                    if j != k {
                        pk *= f[j];
                    }
                }
                dn[a][k] = norm * (pk * (s - (d as f64 - 1.0)) + prod * x[k]);
            }
        } else {
            // Midside node: the zero coordinate `m` carries (1 - xi_m^2), the others (1 + xi x).
            let m = (0..d).find(|&k| x[k].abs() < 0.5).unwrap();
            let norm = if d == 2 { 0.5 } else { 0.25 };
            let mut val = norm * (1.0 - xi[m] * xi[m]);
            for j in 0..d {
                if j != m {
                    val *= f[j];
                }
            }
            n[a] = val;
            for k in 0..d {
                let mut p = if k == m { norm * (-2.0 * xi[m]) } else { norm * (1.0 - xi[m] * xi[m]) * x[k] };
                for j in 0..d {
                    if j != m && j != k {
                        p *= f[j];
                    }
                }
                // For k == m the remaining factors are all f_j (j != m); for k != m the factor
                // f_k was replaced by x_k above, the rest are f_j.
                dn[a][k] = p;
            }
        }
    }
}

/// Quadrature rule and tabulated shape functions.
#[derive(Debug)]
pub struct ShapeTable {
    pub kind: ElementKind,
    pub dim: usize,
    pub nn: usize,
    pub ngp: usize,
    /// Gauss points (natural coordinates) and weights.
    pub xi: Vec<[f64; 3]>,
    pub w: Vec<f64>,
    /// `n[g * nn + a]`.
    pub n: Vec<f64>,
    /// `dn[(g * nn + a) * dim + k]`.
    pub dn: Vec<f64>,
}

impl ShapeTable {
    fn build(kind: ElementKind) -> Self {
        let (xi, w) = quadrature(kind);
        let (dim, nn) = (kind.dim(), kind.n_nodes());
        let mut n = Vec::with_capacity(xi.len() * nn);
        let mut dn = Vec::with_capacity(xi.len() * nn * dim);
        for p in &xi {
            let (sn, sdn) = kind.shape(*p);
            for a in 0..nn {
                n.push(sn[a]);
                for k in 0..dim {
                    dn.push(sdn[a][k]);
                }
            }
        }
        Self { kind, dim, nn, ngp: xi.len(), xi, w, n, dn }
    }

    #[inline]
    pub fn n_at(&self, g: usize) -> &[f64] {
        &self.n[g * self.nn..(g + 1) * self.nn]
    }

    /// Natural derivatives at Gauss point `g`: `[a * dim + k]`.
    #[inline]
    pub fn dn_at(&self, g: usize) -> &[f64] {
        let s = self.nn * self.dim;
        &self.dn[g * s..(g + 1) * s]
    }
}

const GL2: [(f64, f64); 2] = [(-0.577_350_269_189_625_8, 1.0), (0.577_350_269_189_625_8, 1.0)];
const GL3: [(f64, f64); 3] = [(-0.774_596_669_241_483_4, 5.0 / 9.0), (0.0, 8.0 / 9.0), (0.774_596_669_241_483_4, 5.0 / 9.0)];

fn tensor(rule: &[(f64, f64)], dim: usize) -> (Vec<[f64; 3]>, Vec<f64>) {
    let mut xi = Vec::new();
    let mut w = Vec::new();
    let nz = if dim == 3 { rule.len() } else { 1 };
    for k in 0..nz {
        for j in 0..rule.len() {
            for i in 0..rule.len() {
                xi.push([rule[i].0, rule[j].0, if dim == 3 { rule[k].0 } else { 0.0 }]);
                w.push(rule[i].1 * rule[j].1 * if dim == 3 { rule[k].1 } else { 1.0 });
            }
        }
    }
    (xi, w)
}

fn quadrature(kind: ElementKind) -> (Vec<[f64; 3]>, Vec<f64>) {
    match kind {
        ElementKind::Tri3 => (vec![[1.0 / 3.0, 1.0 / 3.0, 0.0]], vec![0.5]),
        // Degree 2 (exact for straight-sided Tri6 stiffness).
        ElementKind::Tri6 => (vec![[1.0 / 6.0, 1.0 / 6.0, 0.0], [2.0 / 3.0, 1.0 / 6.0, 0.0], [1.0 / 6.0, 2.0 / 3.0, 0.0]], vec![1.0 / 6.0; 3]),
        ElementKind::Tet4 => (vec![[0.25, 0.25, 0.25]], vec![1.0 / 6.0]),
        ElementKind::Tet10 => {
            let (a, b) = (0.585_410_196_624_968_5, 0.138_196_601_125_010_5);
            (vec![[b, b, b], [a, b, b], [b, a, b], [b, b, a]], vec![1.0 / 24.0; 4])
        }
        ElementKind::Quad4 => tensor(&GL2, 2),
        ElementKind::Quad8 | ElementKind::Quad9 => tensor(&GL3, 2),
        ElementKind::Hex8 => tensor(&GL2, 3),
        ElementKind::Hex20 | ElementKind::Hex27 => tensor(&GL3, 3),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fd_dn(kind: ElementKind, xi: [f64; 3], a: usize, k: usize) -> f64 {
        let h = 1e-6;
        let (mut p, mut m) = (xi, xi);
        p[k] += h;
        m[k] -= h;
        (kind.shape(p).0[a] - kind.shape(m).0[a]) / (2.0 * h)
    }

    /// An interior point of the reference element for every type.
    fn probe(kind: ElementKind) -> [f64; 3] {
        if kind.is_simplex() {
            [0.21, 0.17, if kind.dim() == 3 { 0.13 } else { 0.0 }]
        } else {
            [0.31, -0.42, if kind.dim() == 3 { 0.27 } else { 0.0 }]
        }
    }

    #[test]
    fn node_counts_and_tables_are_consistent() {
        for k in ALL_KINDS {
            assert_eq!(k.node_coords().len(), k.n_nodes(), "{k:?}");
            let t = k.table();
            assert_eq!((t.nn, t.dim, t.n.len(), t.dn.len()), (k.n_nodes(), k.dim(), t.ngp * k.n_nodes(), t.ngp * k.n_nodes() * k.dim()));
        }
    }

    #[test]
    fn shape_functions_are_kronecker_at_the_nodes() {
        for k in ALL_KINDS {
            for (b, x) in k.node_coords().iter().enumerate() {
                let (n, _) = k.shape(*x);
                for a in 0..k.n_nodes() {
                    let want = if a == b { 1.0 } else { 0.0 };
                    assert!((n[a] - want).abs() < 1e-12, "{k:?} N{a} at node {b} = {}", n[a]);
                }
            }
        }
    }

    #[test]
    fn partition_of_unity_and_linear_completeness() {
        for k in ALL_KINDS {
            let xi = probe(k);
            let (n, dn) = k.shape(xi);
            let d = k.dim();
            let s: f64 = n[..k.n_nodes()].iter().sum();
            assert!((s - 1.0).abs() < 1e-13, "{k:?} sum N {s}");
            for ax in 0..d {
                let ds: f64 = (0..k.n_nodes()).map(|a| dn[a][ax]).sum();
                assert!(ds.abs() < 1e-12, "{k:?} sum dN {ds}");
                // sum N x == xi (the isoparametric map is the identity on the reference element)
                let x: f64 = (0..k.n_nodes()).map(|a| n[a] * k.node_coords()[a][ax]).sum();
                assert!((x - xi[ax]).abs() < 1e-13, "{k:?} axis {ax}: {x} vs {}", xi[ax]);
                let dx: f64 = (0..k.n_nodes()).map(|a| dn[a][ax] * k.node_coords()[a][ax]).sum();
                assert!((dx - 1.0).abs() < 1e-12, "{k:?} d x / d xi {dx}");
            }
        }
    }

    #[test]
    fn analytic_derivatives_match_finite_differences() {
        for k in ALL_KINDS {
            let xi = probe(k);
            let (_, dn) = k.shape(xi);
            for a in 0..k.n_nodes() {
                for ax in 0..k.dim() {
                    let fd = fd_dn(k, xi, a, ax);
                    assert!((dn[a][ax] - fd).abs() < 1e-7, "{k:?} dN{a}/d{ax}: {} vs {fd}", dn[a][ax]);
                }
            }
        }
    }

    #[test]
    fn quadrature_weights_sum_to_the_reference_measure() {
        for k in ALL_KINDS {
            let want = match (k.is_simplex(), k.dim()) {
                (true, 2) => 0.5,
                (true, _) => 1.0 / 6.0,
                (false, 2) => 4.0,
                (false, _) => 8.0,
            };
            let s: f64 = k.table().w.iter().sum();
            assert!((s - want).abs() < 1e-13, "{k:?} {s}");
        }
    }

    #[test]
    fn quadrature_is_exact_for_the_element_stiffness_degree() {
        // Integrate xi^p over the reference element where the rule must be exact (degree 2 for
        // the quadratic simplices, 3 per axis for 3-point Gauss, 1 for 2-point Gauss).
        // Tri6 / Tet10 / Tri3 / Tet4 : linear and quadratic monomials.
        let fact = |n: u32| (1..=n).map(|v| v as f64).product::<f64>();
        for k in [ElementKind::Tri6, ElementKind::Tet10] {
            let d = k.dim();
            let t = k.table();
            for a in 0..=2u32 {
                for b in 0..=(2 - a) {
                    let c_range = if d == 3 { 0..=(2 - a - b) } else { 0..=0 };
                    for c in c_range {
                        let got: f64 = (0..t.ngp).map(|g| t.w[g] * t.xi[g][0].powi(a as i32) * t.xi[g][1].powi(b as i32) * t.xi[g][2].powi(c as i32)).sum();
                        let exact = fact(a) * fact(b) * fact(c) / fact(a + b + c + d as u32);
                        assert!((got - exact).abs() < 1e-14, "{k:?} x^{a} y^{b} z^{c}: {got} vs {exact}");
                    }
                }
            }
        }
        // Tensor rules: x^p over [-1, 1] is 2/(p+1) for even p, else 0.
        for (k, maxp) in [(ElementKind::Quad4, 3), (ElementKind::Hex8, 3), (ElementKind::Quad9, 5), (ElementKind::Hex27, 5)] {
            let t = k.table();
            for p in 0..=maxp {
                let got: f64 = (0..t.ngp).map(|g| t.w[g] * t.xi[g][0].powi(p)).sum();
                let one_d = if p % 2 == 0 { 2.0 / (p as f64 + 1.0) } else { 0.0 };
                let want = one_d * 2.0f64.powi(k.dim() as i32 - 1);
                assert!((got - want).abs() < 1e-13, "{k:?} x^{p}: {got} vs {want}");
            }
        }
    }
}
