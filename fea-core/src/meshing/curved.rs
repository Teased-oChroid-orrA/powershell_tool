//! Validity of curved (quadratic) simplex elements: is the isoparametric map
//! injective, i.e. is the Jacobian determinant positive everywhere in the element
//! rather than only at the corners?
//!
//! Lifting midside nodes onto a curved boundary does not by itself make a valid
//! element. The determinant of a Tet10 map is a cubic (a Tri6 map: quadratic)
//! polynomial in the barycentric coordinates, so its Bernstein-Bezier coefficients
//! bound it from below: all positive proves the element valid, while a
//! non-positive polynomial value at a lattice point proves it invalid. Otherwise
//! the element is subdivided and the test repeated, down to a depth limit
//! (`Undetermined` if still unresolved). Dividing by the determinant of the straight
//! corner element makes the value a ratio: 1 for a straight element.
use super::MeshError;
use crate::element::ElementKind;
use crate::mesh::Mesh;
type V3 = [f64; 3];
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// The Jacobian determinant is positive at every point (proved by the Bernstein bound).
    Valid,
    /// A point with a non-positive determinant was found.
    Invalid,
    /// Neither proved nor disproved within the subdivision depth.
    Undetermined,
}
#[derive(Debug, Clone, PartialEq)]
pub struct Validity {
    pub verdict: Verdict,
    /// Rigorous lower bound of `det J / det J_straight` over the element (it may be
    /// negative or zero when the verdict is not `Valid`).
    pub min_ratio_bound: f64,
    /// Smallest ratio actually evaluated.
    pub min_ratio_sampled: f64,
    pub depth: usize,
}
/// Multi-indices of the degree-`d` lattice of an `n`-simplex (`n + 1` barycentric entries).
fn lattice(n: usize, d: usize) -> Vec<Vec<usize>> {
    fn go(n: usize, left: usize, prefix: &mut Vec<usize>, out: &mut Vec<Vec<usize>>) {
        if prefix.len() == n {
            prefix.push(left);
            out.push(prefix.clone());
            prefix.pop();
            return;
        }
        for k in 0..=left {
            prefix.push(k);
            go(n, left - k, prefix, out);
            prefix.pop();
        }
    }
    let mut out = Vec::new();
    go(n, d, &mut Vec::new(), &mut out);
    out
}
fn factorial(k: usize) -> f64 {
    (1..=k).fold(1., |a, b| a * b as f64)
}
/// Inverse of the Bernstein collocation matrix of the lattice (values -> coefficients).
fn inverse_collocation(n: usize, d: usize) -> (Vec<Vec<usize>>, Vec<Vec<f64>>) {
    let idx = lattice(n, d);
    let m = idx.len();
    let basis = |alpha: &[usize], lam: &[f64]| -> f64 {
        let coef = factorial(d) / alpha.iter().map(|a| factorial(*a)).product::<f64>();
        coef * alpha
            .iter()
            .zip(lam)
            .map(|(a, l)| l.powi(*a as i32))
            .product::<f64>()
    };
    let mut a = vec![vec![0.; 2 * m]; m];
    for (r, beta) in idx.iter().enumerate() {
        let lam: Vec<f64> = beta.iter().map(|b| *b as f64 / d as f64).collect();
        for (c, alpha) in idx.iter().enumerate() {
            a[r][c] = basis(alpha, &lam);
        }
        a[r][m + r] = 1.;
    }
    for col in 0..m {
        let pivot = (col..m)
            .max_by(|&x, &y| a[x][col].abs().total_cmp(&a[y][col].abs()))
            .unwrap();
        a.swap(col, pivot);
        let p = a[col][col];
        for v in a[col].iter_mut() {
            *v /= p;
        }
        for r in 0..m {
            if r != col {
                let f = a[r][col];
                if f != 0. {
                    for c in 0..2 * m {
                        let sub = f * a[col][c];
                        a[r][c] -= sub;
                    }
                }
            }
        }
    }
    (idx, a.into_iter().map(|row| row[m..].to_vec()).collect())
}
fn det3(c: [V3; 3]) -> f64 {
    c[0][0] * (c[1][1] * c[2][2] - c[1][2] * c[2][1])
        - c[1][0] * (c[0][1] * c[2][2] - c[0][2] * c[2][1])
        + c[2][0] * (c[0][1] * c[1][2] - c[0][2] * c[1][1])
}
struct Element<'a> {
    kind: ElementKind,
    xyz: &'a [V3],
    straight: f64,
}
impl Element<'_> {
    /// `det J / det J_straight` at the barycentric point `lam` (entries `lam[1..]` are `xi`).
    fn ratio(&self, lam: &[f64]) -> f64 {
        let dim = self.kind.dim();
        let mut xi = [0.; 3];
        xi[..dim].copy_from_slice(&lam[1..=dim]);
        let (_, dn) = self.kind.shape(xi);
        let mut cols = [[0.; 3]; 3];
        for (a, p) in self.xyz.iter().enumerate().take(self.kind.n_nodes()) {
            for k in 0..dim {
                for r in 0..3 {
                    cols[k][r] += dn[a][k] * p[r];
                }
            }
        }
        let det = if dim == 3 {
            det3(cols)
        } else {
            cols[0][0] * cols[1][1] - cols[0][1] * cols[1][0]
        };
        det / self.straight
    }
}
/// Check one Tet10 or Tri6 element (Tri6: the first two coordinates, a planar element).
pub fn simplex_validity(
    kind: ElementKind,
    xyz: &[[f64; 3]],
    max_depth: usize,
) -> Result<Validity, MeshError> {
    let (dim, degree) = match kind {
        ElementKind::Tet10 => (3, 3),
        ElementKind::Tri6 => (2, 2),
        other => {
            return Err(MeshError::Unsupported(format!(
                "curved validity is defined for Tet10 and Tri6, not {other:?}"
            )))
        }
    };
    if xyz.len() < kind.n_nodes() || xyz.iter().flatten().any(|x| !x.is_finite()) {
        return Err(MeshError::InvalidElements {
            count: 1,
            first: (0, 0),
        });
    }
    let straight = if dim == 3 {
        det3([
            std::array::from_fn(|r| xyz[1][r] - xyz[0][r]),
            std::array::from_fn(|r| xyz[2][r] - xyz[0][r]),
            std::array::from_fn(|r| xyz[3][r] - xyz[0][r]),
        ])
    } else {
        (xyz[1][0] - xyz[0][0]) * (xyz[2][1] - xyz[0][1])
            - (xyz[1][1] - xyz[0][1]) * (xyz[2][0] - xyz[0][0])
    };
    if !(straight.is_finite() && straight > 0.) {
        // Corners inverted or flat: nothing to refine.
        return Ok(Validity {
            verdict: Verdict::Invalid,
            min_ratio_bound: f64::NEG_INFINITY,
            min_ratio_sampled: f64::NEG_INFINITY,
            depth: 0,
        });
    }
    let element = Element {
        kind,
        xyz,
        straight,
    };
    let (idx, inv) = inverse_collocation(dim, degree);
    let mut corners: Vec<Vec<Vec<f64>>> = vec![(0..=dim)
        .map(|i| (0..=dim).map(|j| (i == j) as u8 as f64).collect())
        .collect()];
    let mut next = Vec::new();
    let mut depth = 0;
    let mut min_bound = f64::INFINITY;
    let mut min_sampled = f64::INFINITY;
    let mut undecided = false;
    while !corners.is_empty() {
        for cell in &corners {
            let values: Vec<f64> = idx
                .iter()
                .map(|beta| {
                    let lam: Vec<f64> = (0..=dim)
                        .map(|k| {
                            (0..=dim)
                                .map(|i| beta[i] as f64 / degree as f64 * cell[i][k])
                                .sum()
                        })
                        .collect();
                    element.ratio(&lam)
                })
                .collect();
            let sampled = values.iter().copied().fold(f64::INFINITY, f64::min);
            min_sampled = min_sampled.min(sampled);
            if sampled <= 0. {
                return Ok(Validity {
                    verdict: Verdict::Invalid,
                    min_ratio_bound: min_bound.min(sampled),
                    min_ratio_sampled: min_sampled,
                    depth,
                });
            }
            let bound = inv
                .iter()
                .map(|row| row.iter().zip(&values).map(|(a, b)| a * b).sum::<f64>())
                .fold(f64::INFINITY, f64::min);
            if bound > 0. {
                min_bound = min_bound.min(bound);
            } else if depth < max_depth {
                next.extend(subdivide(cell, dim));
            } else {
                undecided = true;
                min_bound = min_bound.min(bound);
            }
        }
        corners = std::mem::take(&mut next);
        if !corners.is_empty() {
            depth += 1;
        }
    }
    Ok(Validity {
        verdict: if undecided {
            Verdict::Undetermined
        } else {
            Verdict::Valid
        },
        min_ratio_bound: min_bound,
        min_ratio_sampled: min_sampled,
        depth,
    })
}
/// Midpoint subdivision of a simplex (corner barycentrics `cell[i]`) into 2^dim simplices.
fn subdivide(cell: &[Vec<f64>], dim: usize) -> Vec<Vec<Vec<f64>>> {
    let mid = |a: usize, b: usize| -> Vec<f64> {
        cell[a]
            .iter()
            .zip(&cell[b])
            .map(|(x, y)| 0.5 * (x + y))
            .collect()
    };
    let v = |i: usize| cell[i].clone();
    if dim == 2 {
        let (m01, m12, m02) = (mid(0, 1), mid(1, 2), mid(0, 2));
        vec![
            vec![v(0), m01.clone(), m02.clone()],
            vec![v(1), m01.clone(), m12.clone()],
            vec![v(2), m02.clone(), m12.clone()],
            vec![m01, m12, m02],
        ]
    } else {
        let (m01, m02, m03, m12, m13, m23) = (
            mid(0, 1),
            mid(0, 2),
            mid(0, 3),
            mid(1, 2),
            mid(1, 3),
            mid(2, 3),
        );
        vec![
            vec![v(0), m01.clone(), m02.clone(), m03.clone()],
            vec![v(1), m01.clone(), m12.clone(), m13.clone()],
            vec![v(2), m02.clone(), m12.clone(), m23.clone()],
            vec![v(3), m03.clone(), m13.clone(), m23.clone()],
            // The inner octahedron, split along the m02 - m13 axis.
            vec![m02.clone(), m13.clone(), m01.clone(), m12.clone()],
            vec![m02.clone(), m13.clone(), m12, m23.clone()],
            vec![m02.clone(), m13.clone(), m23, m03.clone()],
            vec![m02, m13, m03, m01],
        ]
    }
}
/// Result of checking every curved element of a mesh.
#[derive(Debug, Clone, PartialEq)]
pub struct CurvedReport {
    pub checked: usize,
    pub valid: usize,
    pub invalid: usize,
    pub undetermined: usize,
    /// First invalid element as `(block, element)`.
    pub first_invalid: Option<(usize, usize)>,
    /// Smallest proven lower bound of `det J / det J_straight` over the valid elements.
    pub min_ratio_bound: f64,
}
impl CurvedReport {
    /// Every curved element is proven valid.
    pub fn all_valid(&self) -> bool {
        self.invalid == 0 && self.undetermined == 0
    }
}
/// Check every Tet10 and Tri6 element; other kinds are not curved and are skipped.
pub fn check_mesh(mesh: &Mesh, max_depth: usize) -> Result<CurvedReport, MeshError> {
    let mut report = CurvedReport {
        checked: 0,
        valid: 0,
        invalid: 0,
        undetermined: 0,
        first_invalid: None,
        min_ratio_bound: f64::INFINITY,
    };
    for (b, block) in mesh.blocks.iter().enumerate() {
        if !matches!(block.kind, ElementKind::Tet10 | ElementKind::Tri6) {
            continue;
        }
        let n = block.kind.n_nodes();
        for (e, conn) in block.conn.chunks(n).enumerate() {
            let xyz: Vec<V3> = conn.iter().map(|&i| mesh.nodes[i]).collect();
            let v = simplex_validity(block.kind, &xyz, max_depth)?;
            report.checked += 1;
            match v.verdict {
                Verdict::Valid => {
                    report.valid += 1;
                    report.min_ratio_bound = report.min_ratio_bound.min(v.min_ratio_bound);
                }
                Verdict::Invalid => {
                    report.invalid += 1;
                    report.first_invalid.get_or_insert((b, e));
                }
                Verdict::Undetermined => report.undetermined += 1,
            }
        }
    }
    Ok(report)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn tet10(corners: [V3; 4]) -> Vec<V3> {
        let mid = |a: usize, b: usize| -> V3 {
            std::array::from_fn(|i| 0.5 * (corners[a][i] + corners[b][i]))
        };
        let mut p = corners.to_vec();
        for (a, b) in [(0, 1), (1, 2), (2, 0), (0, 3), (1, 3), (2, 3)] {
            p.push(mid(a, b));
        }
        p
    }
    const REF: [V3; 4] = [[0., 0., 0.], [1., 0., 0.], [0., 1., 0.], [0., 0., 1.]];
    fn close(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol * (1. + a.abs().max(b.abs()))
    }
    /// Dense lattice sampling of the ratio, the independent oracle.
    fn brute_min(xyz: &[V3], kind: ElementKind) -> f64 {
        let (dim, n) = if kind == ElementKind::Tet10 {
            (3, 24)
        } else {
            (2, 40)
        };
        let straight = if dim == 3 {
            det3([
                std::array::from_fn(|r| xyz[1][r] - xyz[0][r]),
                std::array::from_fn(|r| xyz[2][r] - xyz[0][r]),
                std::array::from_fn(|r| xyz[3][r] - xyz[0][r]),
            ])
        } else {
            (xyz[1][0] - xyz[0][0]) * (xyz[2][1] - xyz[0][1])
                - (xyz[1][1] - xyz[0][1]) * (xyz[2][0] - xyz[0][0])
        };
        let el = Element {
            kind,
            xyz,
            straight,
        };
        lattice(dim, n)
            .iter()
            .map(|b| {
                let lam: Vec<f64> = b.iter().map(|x| *x as f64 / n as f64).collect();
                el.ratio(&lam)
            })
            .fold(f64::INFINITY, f64::min)
    }
    #[test]
    fn straight_elements_have_a_constant_unit_ratio_under_any_affine_map() {
        let v = simplex_validity(ElementKind::Tet10, &tet10(REF), 4).unwrap();
        assert_eq!(v.verdict, Verdict::Valid);
        assert!(close(v.min_ratio_bound, 1., 1e-12) && v.depth == 0);
        // Sheared, rotated and scaled: still a constant ratio of one.
        let a = [[2., 0.3, 0.1], [-0.2, 1.5, 0.4], [0.1, 0.2, 0.7]];
        let corners =
            REF.map(|p| std::array::from_fn(|i| 5. + (0..3).map(|j| a[i][j] * p[j]).sum::<f64>()));
        let v = simplex_validity(ElementKind::Tet10, &tet10(corners), 4).unwrap();
        assert!(v.verdict == Verdict::Valid && close(v.min_ratio_bound, 1., 1e-10));
        let tri: Vec<V3> = vec![
            [0., 0., 0.],
            [2., 0., 0.],
            [0., 3., 0.],
            [1., 0., 0.],
            [1., 1.5, 0.],
            [0., 1.5, 0.],
        ];
        let v = simplex_validity(ElementKind::Tri6, &tri, 4).unwrap();
        assert!(v.verdict == Verdict::Valid && close(v.min_ratio_bound, 1., 1e-12));
    }
    #[test]
    fn bounded_curvature_is_proved_valid_and_extreme_curvature_is_found_invalid() {
        // Midside node of edge 0-1 pushed 0.2 sideways: curved but valid.
        let mut p = tet10(REF);
        p[4][1] -= 0.2;
        let v = simplex_validity(ElementKind::Tet10, &p, 6).unwrap();
        assert_eq!(v.verdict, Verdict::Valid, "{v:?}");
        let brute = brute_min(&p, ElementKind::Tet10);
        assert!(
            v.min_ratio_bound > 0. && v.min_ratio_bound <= brute + 1e-12,
            "{v:?} {brute}"
        );
        assert!(brute < 1., "the element is really curved: {brute}");
        // The same node pushed 0.9 sideways folds the map over itself.
        let mut bad = tet10(REF);
        bad[4][1] -= 1.6;
        bad[6][0] += 1.6;
        let v = simplex_validity(ElementKind::Tet10, &bad, 6).unwrap();
        assert_eq!(v.verdict, Verdict::Invalid, "{v:?}");
        assert!(brute_min(&bad, ElementKind::Tet10) <= 0.);
        // Inverted corners are invalid at once; non-finite input and wrong kinds are errors.
        let flipped: Vec<V3> = tet10([REF[1], REF[0], REF[2], REF[3]]);
        assert_eq!(
            simplex_validity(ElementKind::Tet10, &flipped, 3)
                .unwrap()
                .verdict,
            Verdict::Invalid
        );
        let mut nan = tet10(REF);
        nan[9][2] = f64::NAN;
        assert!(simplex_validity(ElementKind::Tet10, &nan, 3).is_err());
        assert!(simplex_validity(ElementKind::Tet4, &tet10(REF), 3).is_err());
    }
    #[test]
    fn the_verdict_agrees_with_dense_sampling_on_random_curved_elements() {
        let mut s = 7u64;
        let mut next = || {
            s = s
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((s >> 33) as f64) / (1u64 << 30) as f64 - 1. // [-1, 1)
        };
        let (mut valid, mut invalid) = (0, 0);
        for trial in 0..200 {
            let amp = 0.05 + 0.9 * (trial as f64 / 200.);
            let mut p = tet10(REF);
            for node in p.iter_mut().skip(4) {
                for x in node.iter_mut() {
                    *x += amp * 0.5 * next();
                }
            }
            let v = simplex_validity(ElementKind::Tet10, &p, 8).unwrap();
            let brute = brute_min(&p, ElementKind::Tet10);
            match v.verdict {
                Verdict::Valid => {
                    valid += 1;
                    assert!(
                        brute > 0. && v.min_ratio_bound <= brute + 1e-12,
                        "trial {trial}: {v:?} {brute}"
                    );
                }
                Verdict::Invalid => {
                    invalid += 1;
                    assert!(
                        brute <= 1e-9 || v.min_ratio_sampled <= 0.,
                        "trial {trial}: {v:?} {brute}"
                    );
                    assert!(v.min_ratio_sampled <= 0.);
                }
                Verdict::Undetermined => {
                    assert!(brute.abs() < 0.05, "trial {trial}: {v:?} {brute}")
                }
            }
        }
        assert!(
            valid > 20 && invalid > 20,
            "valid {valid} invalid {invalid}"
        );
    }
    #[test]
    fn a_curved_tri6_is_checked_the_same_way() {
        let mut tri: Vec<V3> = vec![
            [0., 0., 0.],
            [1., 0., 0.],
            [0., 1., 0.],
            [0.5, 0., 0.],
            [0.5, 0.5, 0.],
            [0., 0.5, 0.],
        ];
        // An inward bend of 0.15: det J = 1 - 4 * 0.15 * xi, so the minimum is 0.4 at the far corner.
        tri[3][1] += 0.15;
        let v = simplex_validity(ElementKind::Tri6, &tri, 6).unwrap();
        assert_eq!(v.verdict, Verdict::Valid);
        assert!(
            close(v.min_ratio_bound, 0.4, 1e-9) && close(v.min_ratio_sampled, 0.4, 1e-9),
            "{v:?}"
        );
        // An outward bend only thickens the element: the minimum stays at 1.
        tri[3][1] = -0.15;
        let v = simplex_validity(ElementKind::Tri6, &tri, 6).unwrap();
        assert!(
            v.verdict == Verdict::Valid && close(v.min_ratio_bound, 1., 1e-9),
            "{v:?}"
        );
        tri[3][1] = 0.15;
        tri[3][1] = 1.2; // the mid-edge node far inside the element
        let v = simplex_validity(ElementKind::Tri6, &tri, 6).unwrap();
        assert_eq!(v.verdict, Verdict::Invalid);
    }
}
