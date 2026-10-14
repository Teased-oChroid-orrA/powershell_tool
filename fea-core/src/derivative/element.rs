//! Element kernels written against [`Scalar`]: geometry (Jacobian, determinant, physical gradients, integration weights), isotropic
//! stiffness and consistent mass. With a dual-number scalar the nodal coordinates, the material constants and the thickness become
//! differentiable inputs, so the derivative of an element matrix with respect to a node coordinate or a material parameter is exact
//! (to rounding), not a central difference. Coordinates are the interface to the hybrid mesher: geometry and shape sensitivities of
//! any element it produces (Quad, Tri, Hex, Wedge, linear or quadratic) come from here.
//!
//! The formulas are those of `kernel.rs` (the f64 instance reproduces its element matrices to rounding, which the tests assert).

use super::scalar::Scalar;
use crate::element::ElementKind;
use crate::mesh::Physics;

/// Geometry of one Gauss point.
#[derive(Debug, Clone)]
pub struct GaussGeometry<S: Scalar> {
    /// `det J`.
    pub det: S,
    /// Integration weight: `w |J|` times the thickness (plane) or `2 pi r` (axisymmetric).
    pub weight: S,
    /// Physical gradients `grad N_a`, `[a * dim + i]`.
    pub grad: Vec<S>,
    /// Radius (axisymmetric; otherwise zero).
    pub radius: S,
}

/// Why an element could not be evaluated.
#[derive(Debug, Clone, PartialEq)]
pub enum ElementGeometryError {
    NonPositiveJacobian { gauss_point: usize, det: f64 },
    NonPositiveRadius { gauss_point: usize, r: f64 },
}

impl std::fmt::Display for ElementGeometryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NonPositiveJacobian { gauss_point, det } => write!(f, "inverted or degenerate element (Jacobian {det:.3e} at Gauss point {gauss_point})"),
            Self::NonPositiveRadius { gauss_point, r } => write!(f, "axisymmetric element with radius {r:.3e} at Gauss point {gauss_point}"),
        }
    }
}

impl std::error::Error for ElementGeometryError {}

fn jacobian_inverse<S: Scalar>(jac: &[[S; 3]; 3], d: usize) -> ([[S; 3]; 3], S) {
    let zero = S::cst(0.0);
    if d == 2 {
        let det = jac[0][0] * jac[1][1] - jac[0][1] * jac[1][0];
        let r = det.recip();
        ([[jac[1][1] * r, -(jac[0][1] * r), zero], [-(jac[1][0] * r), jac[0][0] * r, zero], [zero; 3]], det)
    } else {
        let c00 = jac[1][1] * jac[2][2] - jac[1][2] * jac[2][1];
        let c01 = jac[1][2] * jac[2][0] - jac[1][0] * jac[2][2];
        let c02 = jac[1][0] * jac[2][1] - jac[1][1] * jac[2][0];
        let det = jac[0][0] * c00 + jac[0][1] * c01 + jac[0][2] * c02;
        let r = det.recip();
        let inv = [
            [c00 * r, (jac[0][2] * jac[2][1] - jac[0][1] * jac[2][2]) * r, (jac[0][1] * jac[1][2] - jac[0][2] * jac[1][1]) * r],
            [c01 * r, (jac[0][0] * jac[2][2] - jac[0][2] * jac[2][0]) * r, (jac[0][2] * jac[1][0] - jac[0][0] * jac[1][2]) * r],
            [c02 * r, (jac[0][1] * jac[2][0] - jac[0][0] * jac[2][1]) * r, (jac[0][0] * jac[1][1] - jac[0][1] * jac[1][0]) * r],
        ];
        (inv, det)
    }
}

/// Geometry of every Gauss point of an element on the rule `table` (stiffness or mass).
pub fn geometry<S: Scalar>(table: &crate::element::ShapeTable, physics: Physics, coords: &[[S; 3]], thickness: S) -> Result<Vec<GaussGeometry<S>>, ElementGeometryError> {
    let (nn, d) = (table.nn, table.dim);
    let scale = match physics {
        Physics::PlaneStress { .. } | Physics::PlaneStrain { .. } => thickness,
        _ => S::cst(1.0),
    };
    let mut out = Vec::with_capacity(table.ngp);
    for g in 0..table.ngp {
        let dn = table.dn_at(g);
        let mut jac = [[S::cst(0.0); 3]; 3];
        for a in 0..nn {
            for i in 0..d {
                for k in 0..d {
                    jac[i][k] = jac[i][k] + coords[a][i] * S::cst(dn[a * d + k]);
                }
            }
        }
        let (inv, det) = jacobian_inverse(&jac, d);
        if !(det.value() > 0.0) || !det.value().is_finite() {
            return Err(ElementGeometryError::NonPositiveJacobian { gauss_point: g, det: det.value() });
        }
        let mut grad = vec![S::cst(0.0); nn * d];
        for a in 0..nn {
            for i in 0..d {
                let mut s = S::cst(0.0);
                for k in 0..d {
                    s = s + inv[k][i] * S::cst(dn[a * d + k]);
                }
                grad[a * d + i] = s;
            }
        }
        let mut weight = S::cst(table.w[g]) * det * scale;
        let mut radius = S::cst(0.0);
        if matches!(physics, Physics::Axisymmetric) {
            let n = table.n_at(g);
            let mut r = S::cst(0.0);
            for a in 0..nn {
                r = r + coords[a][0] * S::cst(n[a]);
            }
            if !(r.value() > 0.0) {
                return Err(ElementGeometryError::NonPositiveRadius { gauss_point: g, r: r.value() });
            }
            weight = weight * r * S::cst(2.0 * std::f64::consts::PI);
            radius = r;
        }
        out.push(GaussGeometry { det, weight, grad, radius });
    }
    Ok(out)
}

/// Isotropic element stiffness, full `(nn d) x (nn d)` row-major: entry `[(a d + i) (nn d) + b d + j]` is `K[(a, i), (b, j)]`.
pub fn stiffness<S: Scalar>(kind: ElementKind, physics: Physics, coords: &[[S; 3]], e: S, nu: S, thickness: S) -> Result<Vec<S>, ElementGeometryError> {
    let t = kind.table();
    let (nn, d) = (t.nn, t.dim);
    let nd = nn * d;
    let geo = geometry(t, physics, coords, thickness)?;
    let one = S::cst(1.0);
    let mu = e / (S::cst(2.0) * (one + nu));
    let lam = match physics {
        Physics::PlaneStress { .. } => e * nu / (one - nu * nu),
        _ => e * nu / ((one + nu) * (one - nu.scale(2.0))),
    };
    let mut ke = vec![S::cst(0.0); nd * nd];
    for (g, gg) in geo.iter().enumerate() {
        let w = gg.weight;
        if matches!(physics, Physics::Axisymmetric) {
            // strain rows (rr, zz, theta theta, rz), constitutive 4 x 4
            let n = t.n_at(g);
            let dmat = [[lam + mu.scale(2.0), lam, lam, S::cst(0.0)], [lam, lam + mu.scale(2.0), lam, S::cst(0.0)], [lam, lam, lam + mu.scale(2.0), S::cst(0.0)], [S::cst(0.0), S::cst(0.0), S::cst(0.0), mu]];
            let mut bm = vec![[[S::cst(0.0); 2]; 4]; nn];
            for a in 0..nn {
                let (gx, gz) = (gg.grad[2 * a], gg.grad[2 * a + 1]);
                bm[a] = [[gx, S::cst(0.0)], [S::cst(0.0), gz], [S::cst(n[a]) / gg.radius, S::cst(0.0)], [gz, gx]];
            }
            for a in 0..nn {
                for b in 0..nn {
                    for i in 0..2 {
                        for j in 0..2 {
                            let mut s = S::cst(0.0);
                            for p in 0..4 {
                                for q in 0..4 {
                                    s = s + bm[a][p][i] * dmat[p][q] * bm[b][q][j];
                                }
                            }
                            ke[(a * d + i) * nd + b * d + j] = ke[(a * d + i) * nd + b * d + j] + w * s;
                        }
                    }
                }
            }
        } else {
            for a in 0..nn {
                for b in 0..nn {
                    let mut dot = S::cst(0.0);
                    for i in 0..d {
                        dot = dot + gg.grad[a * d + i] * gg.grad[b * d + i];
                    }
                    for i in 0..d {
                        for j in 0..d {
                            let mut v = lam * gg.grad[a * d + i] * gg.grad[b * d + j] + mu * gg.grad[b * d + i] * gg.grad[a * d + j];
                            if i == j {
                                v = v + mu * dot;
                            }
                            ke[(a * d + i) * nd + b * d + j] = ke[(a * d + i) * nd + b * d + j] + w * v;
                        }
                    }
                }
            }
        }
    }
    Ok(ke)
}

/// Consistent mass `rho * integral N_a N_b` as `nn x nn` scalars; the element mass matrix is this times the identity on the components.
pub fn mass<S: Scalar>(kind: ElementKind, physics: Physics, coords: &[[S; 3]], rho: S, thickness: S) -> Result<Vec<S>, ElementGeometryError> {
    let t = kind.mass_table();
    let nn = t.nn;
    let geo = geometry(t, physics, coords, thickness)?;
    let mut me = vec![S::cst(0.0); nn * nn];
    for (g, gg) in geo.iter().enumerate() {
        let w = rho * gg.weight;
        let n = t.n_at(g);
        for a in 0..nn {
            for b in 0..nn {
                me[a * nn + b] = me[a * nn + b] + w * S::cst(n[a] * n[b]);
            }
        }
    }
    Ok(me)
}
