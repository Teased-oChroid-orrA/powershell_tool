//! Stress recovery. Q9 displacements are differentiated at each element's
//! nine nodes and averaged over the elements sharing a node; at the bore
//! the Cartesian tensor is rotated into hoop / radial / shear components.

use crate::fe::Condensed;
use edge_check::fem::shape_q9;

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Stress {
    pub sx: f64,
    pub sy: f64,
    pub txy: f64,
}

impl Stress {
    /// Plane-stress von Mises.
    pub fn von_mises(&self) -> f64 {
        (self.sx * self.sx - self.sx * self.sy + self.sy * self.sy + 3.0 * self.txy * self.txy).sqrt()
    }

    /// `(hoop, radial, shear)` about the origin at polar angle `theta`.
    pub fn polar(&self, theta: f64) -> (f64, f64, f64) {
        let (c, s) = (theta.cos(), theta.sin());
        let radial = self.sx * c * c + self.sy * s * s + 2.0 * self.txy * s * c;
        let hoop = self.sx * s * s + self.sy * c * c - 2.0 * self.txy * s * c;
        let shear = (self.sy - self.sx) * s * c + self.txy * (c * c - s * s);
        (hoop, radial, shear)
    }

    /// Larger principal stress.
    pub fn max_principal(&self) -> f64 {
        let m = 0.5 * (self.sx + self.sy);
        let r = (0.25 * (self.sx - self.sy).powi(2) + self.txy * self.txy).sqrt();
        m + r
    }
}

/// One Gauss point of one element: physical position and the FE stress there.
#[derive(Debug, Clone, Copy)]
pub struct GaussStress {
    pub x: f64,
    pub y: f64,
    pub stress: Stress,
}

/// 3x3 Gauss locations (also the optimal stress-sampling points of a Q9 element).
const GP: [f64; 3] = [-0.774_596_669_241_483_4, 0.0, 0.774_596_669_241_483_4];
const GW: [f64; 3] = [5.0 / 9.0, 8.0 / 9.0, 5.0 / 9.0];

/// FE stress at the 3x3 Gauss points of every element (`[a + 3 b]`, xi along `a`).
pub fn gauss_stresses(cond: &Condensed, u: &[f64]) -> Vec<[GaussStress; 9]> {
    let mesh = &cond.mesh;
    let d_lug = cond.material.d_matrix();
    let d_bush = cond.bushing_material.map(|m| m.d_matrix()).unwrap_or(d_lug);
    mesh.elems
        .iter()
        .enumerate()
        .map(|(ei, e)| {
            let d = if mesh.elem_group[ei] == 1 { &d_bush } else { &d_lug };
            let xy: Vec<[f64; 2]> = e.iter().map(|&n| mesh.nodes[n]).collect();
            let mut out = [GaussStress { x: 0.0, y: 0.0, stress: Stress::default() }; 9];
            for b in 0..3 {
                for a in 0..3 {
                    let (n, dxi, deta) = shape_q9(GP[a], GP[b]);
                    let (mut j11, mut j12, mut j21, mut j22, mut px, mut py) = (0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
                    for k in 0..9 {
                        j11 += dxi[k] * xy[k][0];
                        j12 += dxi[k] * xy[k][1];
                        j21 += deta[k] * xy[k][0];
                        j22 += deta[k] * xy[k][1];
                        px += n[k] * xy[k][0];
                        py += n[k] * xy[k][1];
                    }
                    let det = j11 * j22 - j12 * j21;
                    let inv = [[j22 / det, -j12 / det], [-j21 / det, j11 / det]];
                    let (mut exx, mut eyy, mut gxy) = (0.0, 0.0, 0.0);
                    for k in 0..9 {
                        let dnx = inv[0][0] * dxi[k] + inv[0][1] * deta[k];
                        let dny = inv[1][0] * dxi[k] + inv[1][1] * deta[k];
                        let (ux, uy) = (u[2 * e[k]], u[2 * e[k] + 1]);
                        exx += dnx * ux;
                        eyy += dny * uy;
                        gxy += dny * ux + dnx * uy;
                    }
                    out[3 * b + a] = GaussStress {
                        x: px,
                        y: py,
                        stress: Stress {
                            sx: d[0][0] * exx + d[0][1] * eyy,
                            sy: d[1][0] * exx + d[1][1] * eyy,
                            txy: d[2][2] * gxy,
                        },
                    };
                }
            }
            out
        })
        .collect()
}

/// Nodal stresses (psi) from the full displacement vector `u` (`2 * node + comp`) by
/// superconvergent patch recovery (Zienkiewicz-Zhu): per node, a quadratic polynomial is
/// least-squares fitted to the Gauss-point stresses of the elements sharing it (same
/// material only; lug and bushing nodes are separate) and evaluated at the node. This
/// removes the element-boundary jump of plain averaging, which is what the bore peak
/// hoop stress is most sensitive to. A patch too small or degenerate for the fit falls
/// back to the average of the Gauss values.
pub fn nodal_stresses(cond: &Condensed, u: &[f64]) -> Vec<Stress> {
    let gs = gauss_stresses(cond, u);
    recover(cond, &gs)
}

/// Patch recovery from precomputed Gauss stresses (shared with the error estimator).
pub fn recover(cond: &Condensed, gs: &[[GaussStress; 9]]) -> Vec<Stress> {
    let mesh = &cond.mesh;
    let mut patch: Vec<Vec<usize>> = vec![Vec::new(); mesh.nodes.len()];
    for (ei, e) in mesh.elems.iter().enumerate() {
        for &n in e {
            patch[n].push(ei);
        }
    }
    (0..mesh.nodes.len())
        .map(|node| {
            let pts: Vec<&GaussStress> = patch[node].iter().flat_map(|&ei| gs[ei].iter()).collect();
            if pts.is_empty() {
                return Stress::default();
            }
            let c = |f: fn(&Stress) -> f64| -> f64 { pts.iter().map(|g| f(&g.stress)).sum::<f64>() / pts.len() as f64 };
            let mean = Stress { sx: c(|s| s.sx), sy: c(|s| s.sy), txy: c(|s| s.txy) };
            let [x0, y0] = mesh.nodes[node];
            // Scale the local coordinates by the patch extent so the 6x6 system is conditioned.
            let h = pts.iter().map(|g| ((g.x - x0).powi(2) + (g.y - y0).powi(2)).sqrt()).fold(0.0f64, f64::max);
            if pts.len() < 9 || h <= 0.0 {
                return mean;
            }
            let basis = |g: &GaussStress| {
                let (a, b) = ((g.x - x0) / h, (g.y - y0) / h);
                [1.0, a, b, a * a, a * b, b * b]
            };
            let mut ata = [[0.0f64; 6]; 6];
            let mut atb = [[0.0f64; 6]; 3];
            for g in &pts {
                let p = basis(g);
                let v = [g.stress.sx, g.stress.sy, g.stress.txy];
                for i in 0..6 {
                    for j in 0..6 {
                        ata[i][j] += p[i] * p[j];
                    }
                    for k in 0..3 {
                        atb[k][i] += p[i] * v[k];
                    }
                }
            }
            let mut out = [0.0; 3];
            for k in 0..3 {
                match solve6(ata, atb[k]) {
                    Some(coef) => out[k] = coef[0],
                    None => return mean,
                }
            }
            Stress { sx: out[0], sy: out[1], txy: out[2] }
        })
        .collect()
}

/// Gaussian elimination with partial pivoting; `None` when (nearly) singular.
fn solve6(mut a: [[f64; 6]; 6], mut b: [f64; 6]) -> Option<[f64; 6]> {
    let scale = (0..6).map(|i| a[i][i]).fold(0.0f64, f64::max).max(1e-300);
    for col in 0..6 {
        let piv = (col..6).max_by(|&i, &j| a[i][col].abs().total_cmp(&a[j][col].abs()))?;
        if a[piv][col].abs() < 1e-12 * scale {
            return None;
        }
        a.swap(col, piv);
        b.swap(col, piv);
        for r in col + 1..6 {
            let f = a[r][col] / a[col][col];
            let pivot_row = a[col];
            for (arc, pc) in a[r].iter_mut().zip(pivot_row).skip(col) {
                *arc -= f * pc;
            }
            b[r] -= f * b[col];
        }
    }
    let mut x = [0.0; 6];
    for i in (0..6).rev() {
        let s: f64 = (i + 1..6).map(|j| a[i][j] * x[j]).sum();
        x[i] = (b[i] - s) / a[i][i];
    }
    Some(x)
}

/// Zienkiewicz-Zhu a-posteriori error estimate.
#[derive(Debug, Clone, PartialEq)]
pub struct ErrorEstimate {
    /// Estimated relative energy-norm error `|e| / |u|` (dimensionless).
    pub relative: f64,
    /// Per-element energy-norm error (same order as `mesh.elems`), lbf-in per unit thickness^0.5.
    pub element: Vec<f64>,
    /// Index of the worst element.
    pub worst: usize,
}

/// Elements whose centroid lies beyond `skip_beyond_x` (the clamped end) are left out of the sums.
/// `|e_el|^2 = int (sigma* - sigma_h)^T D^-1 (sigma* - sigma_h) dA` with `sigma*` the recovered
/// field interpolated by the element shape functions, over the 3x3 Gauss rule.
pub fn error_estimate(cond: &Condensed, u: &[f64], skip_beyond_x: f64) -> ErrorEstimate {
    let gs = gauss_stresses(cond, u);
    let nodal = recover(cond, &gs);
    let mesh = &cond.mesh;
    let c_lug = invert3(cond.material.d_matrix());
    let c_bush = cond.bushing_material.map(|m| invert3(m.d_matrix())).unwrap_or(c_lug);
    let (mut sum_e, mut sum_u) = (0.0, 0.0);
    let mut element = Vec::with_capacity(mesh.elems.len());
    for (ei, e) in mesh.elems.iter().enumerate() {
        let c = if mesh.elem_group[ei] == 1 { &c_bush } else { &c_lug };
        let xy: Vec<[f64; 2]> = e.iter().map(|&n| mesh.nodes[n]).collect();
        // The clamped far end is a modelling boundary with a corner singularity, not part of the lug.
        if xy.iter().map(|p| p[0]).sum::<f64>() / 9.0 > skip_beyond_x {
            element.push(0.0);
            continue;
        }
        let mut err2 = 0.0;
        for b in 0..3 {
            for a in 0..3 {
                let (n, dxi, deta) = shape_q9(GP[a], GP[b]);
                let (mut j11, mut j12, mut j21, mut j22) = (0.0, 0.0, 0.0, 0.0);
                for k in 0..9 {
                    j11 += dxi[k] * xy[k][0];
                    j12 += dxi[k] * xy[k][1];
                    j21 += deta[k] * xy[k][0];
                    j22 += deta[k] * xy[k][1];
                }
                let w = GW[a] * GW[b] * (j11 * j22 - j12 * j21).abs();
                let mut star = [0.0; 3];
                for k in 0..9 {
                    star[0] += n[k] * nodal[e[k]].sx;
                    star[1] += n[k] * nodal[e[k]].sy;
                    star[2] += n[k] * nodal[e[k]].txy;
                }
                let h = gs[ei][3 * b + a].stress;
                let (d, hv) = ([star[0] - h.sx, star[1] - h.sy, star[2] - h.txy], [h.sx, h.sy, h.txy]);
                for i in 0..3 {
                    for j in 0..3 {
                        err2 += w * d[i] * c[i][j] * d[j];
                        sum_u += w * hv[i] * c[i][j] * hv[j];
                    }
                }
            }
        }
        sum_e += err2;
        element.push(err2.sqrt());
    }
    let worst = element.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).map_or(0, |(i, _)| i);
    // Relative to the energy of the recovered solution (|u|^2 + |e|^2 convention).
    ErrorEstimate { relative: (sum_e / (sum_u + sum_e).max(1e-300)).sqrt(), element, worst }
}

/// Elastic strain energy per unit thickness (lbf-in/in) from the FE stresses, 3x3 Gauss.
pub fn strain_energy(cond: &Condensed, u: &[f64]) -> f64 {
    let gs = gauss_stresses(cond, u);
    let mesh = &cond.mesh;
    let c_lug = invert3(cond.material.d_matrix());
    let c_bush = cond.bushing_material.map(|m| invert3(m.d_matrix())).unwrap_or(c_lug);
    let mut total = 0.0;
    for (ei, e) in mesh.elems.iter().enumerate() {
        let c = if mesh.elem_group[ei] == 1 { &c_bush } else { &c_lug };
        let xy: Vec<[f64; 2]> = e.iter().map(|&n| mesh.nodes[n]).collect();
        for b in 0..3 {
            for a in 0..3 {
                let (_, dxi, deta) = shape_q9(GP[a], GP[b]);
                let (mut j11, mut j12, mut j21, mut j22) = (0.0, 0.0, 0.0, 0.0);
                for k in 0..9 {
                    j11 += dxi[k] * xy[k][0];
                    j12 += dxi[k] * xy[k][1];
                    j21 += deta[k] * xy[k][0];
                    j22 += deta[k] * xy[k][1];
                }
                let w = GW[a] * GW[b] * (j11 * j22 - j12 * j21).abs();
                let h = gs[ei][3 * b + a].stress;
                let v = [h.sx, h.sy, h.txy];
                for i in 0..3 {
                    for j in 0..3 {
                        total += 0.5 * w * v[i] * c[i][j] * v[j];
                    }
                }
            }
        }
    }
    total
}

/// `1/2 int u,i sigma_ij u,j dA`: the energy of the geometric stiffness for the stress state
/// `sigma` (`[3 b + a]` per element, as `from_mesh_geo`) and displacement field `u`.
pub fn geometric_energy(cond: &Condensed, u: &[f64], sigma: &[[[f64; 3]; 9]]) -> f64 {
    let mesh = &cond.mesh;
    let mut total = 0.0;
    for (ei, e) in mesh.elems.iter().enumerate() {
        let xy: Vec<[f64; 2]> = e.iter().map(|&n| mesh.nodes[n]).collect();
        for b in 0..3 {
            for a in 0..3 {
                let (_, dxi, deta) = shape_q9(GP[a], GP[b]);
                let (mut j11, mut j12, mut j21, mut j22) = (0.0, 0.0, 0.0, 0.0);
                for k in 0..9 {
                    j11 += dxi[k] * xy[k][0];
                    j12 += dxi[k] * xy[k][1];
                    j21 += deta[k] * xy[k][0];
                    j22 += deta[k] * xy[k][1];
                }
                let det = j11 * j22 - j12 * j21;
                let inv = [[j22 / det, -j12 / det], [-j21 / det, j11 / det]];
                let (mut gx, mut gy) = ([0.0; 2], [0.0; 2]); // d(ux, uy)/dx and /dy
                for k in 0..9 {
                    let dnx = inv[0][0] * dxi[k] + inv[0][1] * deta[k];
                    let dny = inv[1][0] * dxi[k] + inv[1][1] * deta[k];
                    for c in 0..2 {
                        gx[c] += dnx * u[2 * e[k] + c];
                        gy[c] += dny * u[2 * e[k] + c];
                    }
                }
                let s = sigma[ei][3 * b + a];
                let w = GW[a] * GW[b] * det.abs();
                for c in 0..2 {
                    total += 0.5 * w * (s[0] * gx[c] * gx[c] + 2.0 * s[2] * gx[c] * gy[c] + s[1] * gy[c] * gy[c]);
                }
            }
        }
    }
    total
}

fn invert3(m: [[f64; 3]; 3]) -> [[f64; 3]; 3] {
    let det = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0]) + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
    std::array::from_fn(|j| {
        std::array::from_fn(|i| {
            let (i1, i2, j1, j2) = ((i + 1) % 3, (i + 2) % 3, (j + 1) % 3, (j + 2) % 3);
            (m[i1][j1] * m[i2][j2] - m[i1][j2] * m[i2][j1]) / det
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn polar_components_of_a_uniaxial_stress() {
        let s = Stress { sx: 100.0, sy: 0.0, txy: 0.0 };
        let (h, r, t) = s.polar(0.0);
        assert!((h - 0.0).abs() < 1e-12 && (r - 100.0).abs() < 1e-12 && t.abs() < 1e-12);
        let (h, r, _) = s.polar(std::f64::consts::FRAC_PI_2);
        assert!((h - 100.0).abs() < 1e-9 && r.abs() < 1e-9);
    }

    #[test]
    fn von_mises_and_principal_of_pure_shear() {
        let s = Stress { sx: 0.0, sy: 0.0, txy: 50.0 };
        assert!((s.von_mises() - 50.0 * 3f64.sqrt()).abs() < 1e-9);
        assert!((s.max_principal() - 50.0).abs() < 1e-9);
    }
}
