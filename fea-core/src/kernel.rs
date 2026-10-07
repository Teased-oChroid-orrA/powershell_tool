//! Element kernels: geometry, stiffness, equivalent nodal loads.
//!
//! Isotropic stiffness is built from node-pair blocks,
//! `K_ab = sum_g w [ lambda g_a (x) g_b + mu ( g_b (x) g_a + (g_a . g_b) I ) ]`,
//! with `g_a = grad N_a` the physical gradients: no strain-displacement matrix, no `B^T D B`
//! product, half the node pairs (the rest by symmetry), and the same code for every plane and
//! solid element. Axisymmetric needs the hoop term and uses the 4x4 constitutive form.

#![allow(clippy::needless_range_loop)] // index-parallel numeric kernels read clearer as loops

use crate::element::{ElementKind, MAX_NODES};
use crate::mesh::{Elastic, Physics};

/// Reusable per-thread buffers (no allocation per element).
pub struct Work {
    /// Physical gradients, `[(g * nn + a) * dim + i]`.
    pub grad: Vec<f64>,
    /// Integration weight per Gauss point: `w |J|` times thickness (plane) or `2 pi r` (axisym).
    pub wdet: Vec<f64>,
    /// Radius at each Gauss point (axisymmetric only).
    pub radius: Vec<f64>,
    /// Component-major transposed gradients (scratch of the fast stiffness kernel).
    pub cm_grad: Vec<f64>,
}

impl Default for Work {
    fn default() -> Self {
        Self::new()
    }
}

impl Work {
    pub fn new() -> Self {
        Self { grad: vec![0.0; 27 * MAX_NODES * 3], wdet: vec![0.0; 27], radius: vec![0.0; 27], cm_grad: vec![0.0; 27 * MAX_NODES * 3] }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ElementError {
    /// Non-positive Jacobian at a Gauss point (inverted or degenerate element).
    BadJacobian { gauss_point: usize, det: f64 },
    /// Axisymmetric element with a non-positive radius at a Gauss point.
    BadRadius { gauss_point: usize, r: f64 },
}

impl std::fmt::Display for ElementError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ElementError::BadJacobian { gauss_point, det } => write!(f, "inverted or degenerate element (Jacobian {det:.3e} at Gauss point {gauss_point})"),
            ElementError::BadRadius { gauss_point, r } => write!(f, "axisymmetric element with radius {r:.3e} at Gauss point {gauss_point}"),
        }
    }
}

/// Physical gradients and integration weights of one element.
pub fn geometry(kind: ElementKind, physics: Physics, xyz: &[[f64; 3]], work: &mut Work) -> Result<(), ElementError> {
    let t = kind.table();
    let (nn, d) = (t.nn, t.dim);
    let scale = match physics {
        Physics::PlaneStress { thickness } | Physics::PlaneStrain { thickness } => thickness,
        _ => 1.0,
    };
    for g in 0..t.ngp {
        let dn = t.dn_at(g);
        let mut jac = [[0.0f64; 3]; 3]; // jac[i][k] = d x_i / d xi_k
        for a in 0..nn {
            for i in 0..d {
                let x = xyz[a][i];
                for k in 0..d {
                    jac[i][k] += x * dn[a * d + k];
                }
            }
        }
        // Inverse transpose: grad_i = sum_k inv[k][i] dN/dxi_k.
        let (inv, det) = if d == 2 {
            let det = jac[0][0] * jac[1][1] - jac[0][1] * jac[1][0];
            let r = 1.0 / det;
            ([[jac[1][1] * r, -jac[0][1] * r, 0.0], [-jac[1][0] * r, jac[0][0] * r, 0.0], [0.0; 3]], det)
        } else {
            let c00 = jac[1][1] * jac[2][2] - jac[1][2] * jac[2][1];
            let c01 = jac[1][2] * jac[2][0] - jac[1][0] * jac[2][2];
            let c02 = jac[1][0] * jac[2][1] - jac[1][1] * jac[2][0];
            let det = jac[0][0] * c00 + jac[0][1] * c01 + jac[0][2] * c02;
            let r = 1.0 / det;
            let inv = [
                [c00 * r, (jac[0][2] * jac[2][1] - jac[0][1] * jac[2][2]) * r, (jac[0][1] * jac[1][2] - jac[0][2] * jac[1][1]) * r],
                [c01 * r, (jac[0][0] * jac[2][2] - jac[0][2] * jac[2][0]) * r, (jac[0][2] * jac[1][0] - jac[0][0] * jac[1][2]) * r],
                [c02 * r, (jac[0][1] * jac[2][0] - jac[0][0] * jac[2][1]) * r, (jac[0][0] * jac[1][1] - jac[0][1] * jac[1][0]) * r],
            ];
            (inv, det)
        };
        #[allow(clippy::neg_cmp_op_on_partial_ord)] // NaN must be rejected too
        if !(det > 0.0) || !det.is_finite() {
            return Err(ElementError::BadJacobian { gauss_point: g, det });
        }
        let out = &mut work.grad[g * nn * d..(g + 1) * nn * d];
        for a in 0..nn {
            for i in 0..d {
                let mut s = 0.0;
                for k in 0..d {
                    s += inv[k][i] * dn[a * d + k];
                }
                out[a * d + i] = s;
            }
        }
        let mut w = t.w[g] * det * scale;
        if matches!(physics, Physics::Axisymmetric) {
            let n = t.n_at(g);
            let r: f64 = (0..nn).map(|a| n[a] * xyz[a][0]).sum();
            #[allow(clippy::neg_cmp_op_on_partial_ord)] // NaN must be rejected too
            if !(r > 0.0) {
                return Err(ElementError::BadRadius { gauss_point: g, r });
            }
            work.radius[g] = r;
            w *= 2.0 * std::f64::consts::PI * r;
        }
        work.wdet[g] = w;
    }
    Ok(())
}

/// Stiffness blocks of one element: `ke[(a * nn + b) * d * d + i * d + j]`.
pub fn stiffness(kind: ElementKind, physics: Physics, mat: &Elastic, xyz: &[[f64; 3]], work: &mut Work, ke: &mut [f64]) -> Result<(), ElementError> {
    geometry(kind, physics, xyz, work)?;
    let t = kind.table();
    let (nn, d) = (t.nn, t.dim);
    let blocks = &mut ke[..nn * nn * d * d];
    blocks.fill(0.0);
    let (lam, mu) = (mat.lambda(physics), mat.mu());
    if let Some(an) = &mat.aniso {
        stiffness_aniso(t, physics, an, work, blocks);
        mirror_blocks(blocks, nn, d);
        return Ok(());
    }
    match physics {
        Physics::Axisymmetric => stiffness_axisym(t.ngp, nn, work, kind, lam, mu, blocks),
        _ if d == 2 => stiffness_iso::<2>(t.ngp, nn, work, lam, mu, blocks),
        _ => stiffness_iso::<3>(t.ngp, nn, work, lam, mu, blocks),
    }
    mirror_blocks(blocks, nn, d);
    Ok(())
}

/// Mirror the lower triangle of node pairs: `K_ba = K_ab^T`.
fn mirror_blocks(blocks: &mut [f64], nn: usize, d: usize) {
    let dd = d * d;
    for a in 0..nn {
        for b in 0..a {
            for i in 0..d {
                for j in 0..d {
                    blocks[(b * nn + a) * dd + j * d + i] = blocks[(a * nn + b) * dd + i * d + j];
                }
            }
        }
    }
}

/// Strain components carried by the rows of `B` for an analysis type, as indices into the library
/// stress/strain order `(xx, yy, zz, xy, yz, zx)`.
fn strain_rows(physics: Physics) -> &'static [usize] {
    match physics {
        Physics::Solid => &[0, 1, 2, 3, 4, 5],
        Physics::Axisymmetric => &[0, 1, 2, 3],
        _ => &[0, 1, 3],
    }
}

/// Constitutive matrix over [`strain_rows`]: the sub-block of `D`, condensed on `sigma_zz = 0` in
/// plane stress.
fn reduced_d(an: &crate::mesh::Aniso, physics: Physics) -> ([[f64; 6]; 6], usize) {
    let rows = strain_rows(physics);
    let n = rows.len();
    let mut out = [[0.0f64; 6]; 6];
    for (a, &i) in rows.iter().enumerate() {
        for (b, &j) in rows.iter().enumerate() {
            out[a][b] = an.d[i][j];
            if matches!(physics, Physics::PlaneStress { .. }) {
                out[a][b] -= an.d[i][2] * an.d[2][j] / an.d[2][2];
            }
        }
    }
    (out, n)
}

/// Anisotropic stiffness blocks, `K_ab = sum_g w B_a^T D B_b` over the lower triangle of node pairs.
fn stiffness_aniso(t: &crate::element::ShapeTable, physics: Physics, an: &crate::mesh::Aniso, work: &Work, blocks: &mut [f64]) {
    let (ngp, nn, d) = (t.ngp, t.nn, t.dim);
    let (dm, nc) = reduced_d(an, physics);
    let axisym = matches!(physics, Physics::Axisymmetric);
    // B_a (nc x d) for every node at one Gauss point.
    let mut bm = vec![[[0.0f64; 3]; 6]; nn];
    for g in 0..ngp {
        let gr = &work.grad[g * nn * d..(g + 1) * nn * d];
        for a in 0..nn {
            let m = &mut bm[a];
            *m = [[0.0; 3]; 6];
            if d == 3 {
                let (gx, gy, gz) = (gr[a * 3], gr[a * 3 + 1], gr[a * 3 + 2]);
                m[0][0] = gx;
                m[1][1] = gy;
                m[2][2] = gz;
                m[3][0] = gy;
                m[3][1] = gx;
                m[4][1] = gz;
                m[4][2] = gy;
                m[5][0] = gz;
                m[5][2] = gx;
            } else if axisym {
                let (gx, gy) = (gr[a * 2], gr[a * 2 + 1]);
                m[0][0] = gx;
                m[1][1] = gy;
                m[2][0] = t.n_at(g)[a] / work.radius[g];
                m[3][0] = gy;
                m[3][1] = gx;
            } else {
                let (gx, gy) = (gr[a * 2], gr[a * 2 + 1]);
                m[0][0] = gx;
                m[1][1] = gy;
                m[2][0] = gy;
                m[2][1] = gx;
            }
        }
        let w = work.wdet[g];
        for b in 0..nn {
            // D B_b
            let mut db = [[0.0f64; 3]; 6];
            for p in 0..nc {
                for j in 0..d {
                    for q in 0..nc {
                        db[p][j] += dm[p][q] * bm[b][q][j];
                    }
                }
            }
            for a in b..nn {
                let blk = &mut blocks[(a * nn + b) * d * d..(a * nn + b + 1) * d * d];
                for i in 0..d {
                    for j in 0..d {
                        let mut s = 0.0;
                        for p in 0..nc {
                            s += bm[a][p][i] * db[p][j];
                        }
                        blk[i * d + j] += w * s;
                    }
                }
            }
        }
    }
}

/// Memory layout of an element matrix produced by [`stiffness_fast`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    /// `ke[(a * nn + b) * d * d + i * d + j]` (as [`stiffness`]).
    NodeBlock,
    /// Component blocks: for `i <= j`, `ke[(i * d + j) * nn * nn + a * nn + b]` holds
    /// `K[(a, i), (b, j)]` for every `(a, b)` (diagonal blocks `i == j`: `b <= a` only).
    ComponentMajor,
}

/// Entry `K[(a, i), (b, j)]` of an element matrix in either layout.
#[allow(clippy::too_many_arguments)] // hot accessor: every index is needed and it must inline
#[inline(always)]
pub fn entry(layout: Layout, ke: &[f64], nn: usize, d: usize, a: usize, b: usize, i: usize, j: usize) -> f64 {
    match layout {
        Layout::NodeBlock => ke[(a * nn + b) * d * d + i * d + j],
        Layout::ComponentMajor => {
            if i < j {
                ke[(i * d + j) * nn * nn + a * nn + b]
            } else if i > j {
                ke[(j * d + i) * nn * nn + b * nn + a]
            } else if b <= a {
                ke[(i * d + i) * nn * nn + a * nn + b]
            } else {
                ke[(i * d + i) * nn * nn + b * nn + a]
            }
        }
    }
}

/// Element matrix in the fastest layout for the analysis type. Isotropic plane / solid elements
/// use the component-major layout: every update is a rank-1 outer product over contiguous rows of
/// one `nn x nn` block, which the compiler vectorises (the node-pair form works on 2x2 / 3x3
/// scalar blocks and cannot).
pub fn stiffness_fast(kind: ElementKind, physics: Physics, mat: &Elastic, xyz: &[[f64; 3]], work: &mut Work, ke: &mut [f64]) -> Result<Layout, ElementError> {
    if matches!(physics, Physics::Axisymmetric) || mat.aniso.is_some() {
        stiffness(kind, physics, mat, xyz, work, ke)?;
        return Ok(Layout::NodeBlock);
    }
    geometry(kind, physics, xyz, work)?;
    let t = kind.table();
    let (lam, mu) = (mat.lambda(physics), mat.mu());
    if t.dim == 2 {
        stiffness_cm::<2>(t.ngp, t.nn, work, lam, mu, ke);
    } else {
        stiffness_cm::<3>(t.ngp, t.nn, work, lam, mu, ke);
    }
    Ok(Layout::ComponentMajor)
}

fn stiffness_cm<const D: usize>(ngp: usize, nn: usize, work: &mut Work, lam: f64, mu: f64, ke: &mut [f64]) {
    let nn2 = nn * nn;
    let nblocks = D * D; // only i <= j are used; the rest keeps the indexing simple
    let (blocks, rest) = ke.split_at_mut(nblocks * nn2);
    blocks.fill(0.0);
    let g_mat = &mut rest[..nn2];
    g_mat.fill(0.0);
    // Transpose the gradients once: ut[(i * ngp + g) * nn + a].
    let ut = &mut work.cm_grad;
    for g in 0..ngp {
        let gr = &work.grad[g * nn * D..(g + 1) * nn * D];
        for a in 0..nn {
            for i in 0..D {
                ut[(i * ngp + g) * nn + a] = gr[a * D + i];
            }
        }
    }
    for g in 0..ngp {
        let w = work.wdet[g];
        let (wl, wm) = (w * lam, w * mu);
        let u: [&[f64]; D] = std::array::from_fn(|i| &ut[(i * ngp + g) * nn..(i * ngp + g + 1) * nn]);
        for i in 0..D {
            // Diagonal block: (lam + mu) u_i u_i^T, lower triangle; and mu sum_k u_k u_k^T into G.
            let ui = u[i];
            let blk = &mut blocks[(i * D + i) * nn2..(i * D + i + 1) * nn2];
            for a in 0..nn {
                let c1 = (wl + wm) * ui[a];
                let c2 = wm * ui[a];
                let (row, grow) = (&mut blk[a * nn..a * nn + a + 1], &mut g_mat[a * nn..a * nn + a + 1]);
                for b in 0..=a {
                    row[b] += c1 * ui[b];
                    grow[b] += c2 * ui[b];
                }
            }
            // Off-diagonal blocks (i, j), j > i: lam u_i u_j^T + mu u_j u_i^T, all (a, b).
            for j in i + 1..D {
                let uj = u[j];
                let blk = &mut blocks[(i * D + j) * nn2..(i * D + j + 1) * nn2];
                for a in 0..nn {
                    let (c1, c2) = (wl * ui[a], wm * uj[a]);
                    let row = &mut blk[a * nn..(a + 1) * nn];
                    for b in 0..nn {
                        row[b] += c1 * uj[b] + c2 * ui[b];
                    }
                }
            }
        }
    }
    // G (mu sum_k u_k u_k^T) belongs to every diagonal block.
    for i in 0..D {
        let blk = &mut blocks[(i * D + i) * nn2..(i * D + i + 1) * nn2];
        for a in 0..nn {
            for b in 0..=a {
                blk[a * nn + b] += g_mat[a * nn + b];
            }
        }
    }
}

fn stiffness_iso<const D: usize>(ngp: usize, nn: usize, work: &Work, lam: f64, mu: f64, ke: &mut [f64]) {
    for g in 0..ngp {
        let w = work.wdet[g];
        let (wl, wm) = (w * lam, w * mu);
        let gr = &work.grad[g * nn * D..(g + 1) * nn * D];
        for a in 0..nn {
            let ga = &gr[a * D..a * D + D];
            for b in 0..=a {
                let gb = &gr[b * D..b * D + D];
                let mut dot = 0.0;
                for i in 0..D {
                    dot += ga[i] * gb[i];
                }
                let blk = &mut ke[(a * nn + b) * D * D..(a * nn + b) * D * D + D * D];
                for i in 0..D {
                    for j in 0..D {
                        blk[i * D + j] += wl * ga[i] * gb[j] + wm * ga[j] * gb[i];
                    }
                    blk[i * D + i] += wm * dot;
                }
            }
        }
    }
}

/// Axisymmetric: strains `(rr, zz, tt, rz)`, `B_a = [[g_r, 0], [0, g_z], [N/r, 0], [g_z, g_r]]`.
fn stiffness_axisym(ngp: usize, nn: usize, work: &Work, kind: ElementKind, lam: f64, mu: f64, ke: &mut [f64]) {
    let t = kind.table();
    let dmat = [[lam + 2.0 * mu, lam, lam, 0.0], [lam, lam + 2.0 * mu, lam, 0.0], [lam, lam, lam + 2.0 * mu, 0.0], [0.0, 0.0, 0.0, mu]];
    for g in 0..ngp {
        let w = work.wdet[g];
        let r = work.radius[g];
        let n = t.n_at(g);
        let gr = &work.grad[g * nn * 2..(g + 1) * nn * 2];
        // Strain-displacement matrix of every node: bm[a][row][col].
        let mut bm = [[[0.0f64; 2]; 4]; MAX_NODES];
        for a in 0..nn {
            let (gx, gz) = (gr[2 * a], gr[2 * a + 1]);
            bm[a] = [[gx, 0.0], [0.0, gz], [n[a] / r, 0.0], [gz, gx]];
        }
        for a in 0..nn {
            for b in 0..=a {
                // DB = D B_b (4 x 2)
                let mut db = [[0.0f64; 2]; 4];
                for i in 0..4 {
                    for j in 0..2 {
                        for k in 0..4 {
                            db[i][j] += dmat[i][k] * bm[b][k][j];
                        }
                    }
                }
                let blk = &mut ke[(a * nn + b) * 4..(a * nn + b) * 4 + 4];
                for i in 0..2 {
                    for j in 0..2 {
                        let mut s = 0.0;
                        for k in 0..4 {
                            s += bm[a][k][i] * db[k][j];
                        }
                        blk[i * 2 + j] += w * s;
                    }
                }
            }
        }
    }
}

/// Equivalent nodal force of a uniform temperature change `d_t` of a fully free element
/// (`f[a * d + i]`); the stress it would cause when constrained is `-c d_t I`.
pub fn thermal_load(kind: ElementKind, physics: Physics, mat: &Elastic, d_t: f64, xyz: &[[f64; 3]], work: &mut Work, f: &mut [f64]) -> Result<(), ElementError> {
    geometry(kind, physics, xyz, work)?;
    let t = kind.table();
    let (nn, d) = (t.nn, t.dim);
    f[..nn * d].fill(0.0);
    if let Some(an) = &mat.aniso {
        // f_a = -int B_a^T sigma_th with sigma_th = -D alpha dT, i.e. the stress of zero strain.
        let s0 = stress_aniso(an, physics, [0.0; 6], mat.thermal_strain(d_t));
        for g in 0..t.ngp {
            let w = work.wdet[g];
            let gr = &work.grad[g * nn * d..(g + 1) * nn * d];
            let n = t.n_at(g);
            for a in 0..nn {
                let mut fa = [0.0f64; 3];
                if d == 3 {
                    let (gx, gy, gz) = (gr[a * 3], gr[a * 3 + 1], gr[a * 3 + 2]);
                    fa[0] = gx * s0[0] + gy * s0[3] + gz * s0[5];
                    fa[1] = gy * s0[1] + gx * s0[3] + gz * s0[4];
                    fa[2] = gz * s0[2] + gy * s0[4] + gx * s0[5];
                } else {
                    let (gx, gy) = (gr[a * 2], gr[a * 2 + 1]);
                    fa[0] = gx * s0[0] + gy * s0[3];
                    fa[1] = gy * s0[1] + gx * s0[3];
                    if matches!(physics, Physics::Axisymmetric) {
                        fa[0] += n[a] / work.radius[g] * s0[2];
                    }
                }
                for i in 0..d {
                    f[a * d + i] -= w * fa[i];
                }
            }
        }
        return Ok(());
    }
    let c = mat.thermal_modulus(physics) * d_t;
    for g in 0..t.ngp {
        let w = work.wdet[g] * c;
        let gr = &work.grad[g * nn * d..(g + 1) * nn * d];
        let n = t.n_at(g);
        for a in 0..nn {
            for i in 0..d {
                f[a * d + i] += w * gr[a * d + i];
            }
            if matches!(physics, Physics::Axisymmetric) {
                f[a * d] += w * n[a] / work.radius[g];
            }
        }
    }
    Ok(())
}

/// Equivalent nodal force of a body force per unit volume (per unit area x thickness in plane
/// problems).
pub fn body_load(kind: ElementKind, physics: Physics, body: [f64; 3], xyz: &[[f64; 3]], work: &mut Work, f: &mut [f64]) -> Result<(), ElementError> {
    geometry(kind, physics, xyz, work)?;
    let t = kind.table();
    let (nn, d) = (t.nn, t.dim);
    f[..nn * d].fill(0.0);
    for g in 0..t.ngp {
        let w = work.wdet[g];
        let n = t.n_at(g);
        for a in 0..nn {
            for i in 0..d {
                f[a * d + i] += w * n[a] * body[i];
            }
        }
    }
    Ok(())
}

/// Strain (engineering shear) at Gauss point `g` from the element's nodal displacements `u`
/// (`u[a * d + i]`). Plane: `(xx, yy, zz, xy)`; axisymmetric: `(rr, zz, tt, rz)`; solid:
/// `(xx, yy, zz, xy, yz, zx)`. `zz` is zero in plane strain and not reported for plane stress.
pub fn strain_at(kind: ElementKind, physics: Physics, work: &Work, g: usize, u: &[f64]) -> [f64; 6] {
    let t = kind.table();
    let (nn, d) = (t.nn, t.dim);
    let gr = &work.grad[g * nn * d..(g + 1) * nn * d];
    let mut h = [[0.0f64; 3]; 3]; // h[i][k] = d u_i / d x_k
    for a in 0..nn {
        for i in 0..d {
            for k in 0..d {
                h[i][k] += u[a * d + i] * gr[a * d + k];
            }
        }
    }
    match physics {
        Physics::Solid => [h[0][0], h[1][1], h[2][2], h[0][1] + h[1][0], h[1][2] + h[2][1], h[2][0] + h[0][2]],
        Physics::Axisymmetric => {
            let n = t.n_at(g);
            let ur: f64 = (0..nn).map(|a| n[a] * u[a * 2]).sum();
            [h[0][0], h[1][1], ur / work.radius[g], h[0][1] + h[1][0], 0.0, 0.0]
        }
        _ => [h[0][0], h[1][1], 0.0, h[0][1] + h[1][0], 0.0, 0.0],
    }
}

/// Cauchy stress from a strain (same component order as [`strain_at`]); thermal strain
/// `alpha d_t` removed. Plane stress returns `zz = 0`; plane strain computes `zz`.
pub fn stress_from_strain(physics: Physics, mat: &Elastic, strain: [f64; 6], d_t: f64) -> [f64; 6] {
    if let Some(an) = &mat.aniso {
        return stress_aniso(an, physics, strain, mat.thermal_strain(d_t));
    }
    let th = mat.alpha * d_t;
    let (lam, mu) = (mat.lambda(physics), mat.mu());
    match physics {
        Physics::PlaneStress { .. } => {
            let tr = strain[0] + strain[1] - 2.0 * th;
            [lam * tr + 2.0 * mu * (strain[0] - th), lam * tr + 2.0 * mu * (strain[1] - th), 0.0, mu * strain[3], 0.0, 0.0]
        }
        Physics::PlaneStrain { .. } => {
            let tr = strain[0] + strain[1] - 3.0 * th;
            [lam * tr + 2.0 * mu * (strain[0] - th), lam * tr + 2.0 * mu * (strain[1] - th), lam * tr - 2.0 * mu * th, mu * strain[3], 0.0, 0.0]
        }
        Physics::Axisymmetric => {
            let tr = strain[0] + strain[1] + strain[2] - 3.0 * th;
            [lam * tr + 2.0 * mu * (strain[0] - th), lam * tr + 2.0 * mu * (strain[1] - th), lam * tr + 2.0 * mu * (strain[2] - th), mu * strain[3], 0.0, 0.0]
        }
        Physics::Solid => {
            let tr = strain[0] + strain[1] + strain[2] - 3.0 * th;
            [
                lam * tr + 2.0 * mu * (strain[0] - th),
                lam * tr + 2.0 * mu * (strain[1] - th),
                lam * tr + 2.0 * mu * (strain[2] - th),
                mu * strain[3],
                mu * strain[4],
                mu * strain[5],
            ]
        }
    }
}

/// `sigma = D (strain - thermal)` for an anisotropic material (`th` = thermal strain vector, engineering shear).
fn stress_aniso(an: &crate::mesh::Aniso, physics: Physics, strain: [f64; 6], th: [f64; 6]) -> [f64; 6] {
    let mut out = [0.0f64; 6];
    match physics {
        Physics::Solid => {
            let e: [f64; 6] = std::array::from_fn(|i| strain[i] - th[i]);
            for i in 0..6 {
                out[i] = (0..6).map(|j| an.d[i][j] * e[j]).sum();
            }
        }
        Physics::PlaneStress { .. } => {
            let (dm, _) = reduced_d(an, physics);
            let e = [strain[0] - th[0], strain[1] - th[1], strain[3] - th[3]];
            let s: [f64; 3] = std::array::from_fn(|i| (0..3).map(|j| dm[i][j] * e[j]).sum());
            out[0] = s[0];
            out[1] = s[1];
            out[3] = s[2];
        }
        Physics::PlaneStrain { .. } | Physics::Axisymmetric => {
            // Strain over (xx, yy, zz, xy): zz is zero in plane strain, the hoop strain in axisymmetry.
            let ezz = if matches!(physics, Physics::PlaneStrain { .. }) { 0.0 } else { strain[2] };
            let e = [strain[0] - th[0], strain[1] - th[1], ezz - th[2], strain[3] - th[3]];
            for i in 0..4 {
                out[i] = (0..4).map(|j| an.d[i][j] * e[j]).sum();
            }
        }
    }
    out
}

/// Von Mises equivalent of a stress in the order of [`strain_at`] (plane: `zz` as stored).
pub fn von_mises(s: &[f64; 6]) -> f64 {
    let d = (s[0] - s[1]).powi(2) + (s[1] - s[2]).powi(2) + (s[2] - s[0]).powi(2);
    (0.5 * d + 3.0 * (s[3] * s[3] + s[4] * s[4] + s[5] * s[5])).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element::ALL_KINDS;

    /// A mildly distorted physical element: the reference nodes pushed through a smooth map.
    fn distorted(kind: ElementKind) -> Vec<[f64; 3]> {
        kind.node_coords()
            .iter()
            .map(|x| {
                let (a, b, c) = (x[0], x[1], x[2]);
                if kind.dim() == 2 {
                    [2.0 + 0.8 * a + 0.1 * b + 0.05 * a * b, 1.0 + 0.1 * a + 0.7 * b - 0.04 * a * a, 0.0]
                } else {
                    [2.0 + 0.8 * a + 0.1 * b + 0.05 * c + 0.03 * a * b, 1.0 + 0.1 * a + 0.7 * b + 0.04 * c, 3.0 + 0.05 * a + 0.1 * b + 0.9 * c + 0.02 * b * c]
                }
            })
            .collect()
    }

    fn physics_for(kind: ElementKind, plane: Physics) -> Physics {
        if kind.dim() == 3 {
            Physics::Solid
        } else {
            plane
        }
    }

    #[test]
    fn rigid_body_modes_carry_no_force_and_the_matrix_is_symmetric() {
        let mat = Elastic::new(10.0e6, 0.33);
        let mut work = Work::new();
        for kind in ALL_KINDS {
            for plane in [Physics::PlaneStress { thickness: 0.25 }, Physics::PlaneStrain { thickness: 1.0 }] {
                let ph = physics_for(kind, plane);
                let (nn, d) = (kind.n_nodes(), kind.dim());
                let xyz = distorted(kind);
                let mut ke = vec![0.0; nn * nn * d * d];
                stiffness(kind, ph, &mat, &xyz, &mut work, &mut ke).unwrap();
                // Assemble into the dense nn*d square.
                let n = nn * d;
                let mut k = vec![0.0; n * n];
                for a in 0..nn {
                    for b in 0..nn {
                        for i in 0..d {
                            for j in 0..d {
                                k[(a * d + i) * n + b * d + j] = ke[(a * nn + b) * d * d + i * d + j];
                            }
                        }
                    }
                }
                let scale = k.iter().fold(0.0f64, |m, v| m.max(v.abs()));
                for i in 0..n {
                    for j in 0..i {
                        assert!((k[i * n + j] - k[j * n + i]).abs() < 1e-10 * scale, "{kind:?} asymmetric");
                    }
                }
                // Rigid translations and (infinitesimal) rotations.
                let mut modes: Vec<Vec<f64>> = Vec::new();
                for c in 0..d {
                    modes.push((0..n).map(|q| if q % d == c { 1.0 } else { 0.0 }).collect());
                }
                if d == 2 {
                    modes.push((0..nn).flat_map(|a| [-xyz[a][1], xyz[a][0]]).collect());
                } else {
                    modes.push((0..nn).flat_map(|a| [-xyz[a][1], xyz[a][0], 0.0]).collect());
                    modes.push((0..nn).flat_map(|a| [0.0, -xyz[a][2], xyz[a][1]]).collect());
                    modes.push((0..nn).flat_map(|a| [xyz[a][2], 0.0, -xyz[a][0]]).collect());
                }
                for (m, u) in modes.iter().enumerate() {
                    for i in 0..n {
                        let f: f64 = (0..n).map(|j| k[i * n + j] * u[j]).sum();
                        assert!(f.abs() < 1e-9 * scale * 4.0, "{kind:?} mode {m} force {f}");
                    }
                }
            }
        }
    }

    #[test]
    fn constant_strain_field_recovers_exact_stress_and_energy() {
        // u = H x + c exactly representable by every element: strain exact at every Gauss point.
        let mat = Elastic::new(10.0e6, 0.3);
        let mut work = Work::new();
        for kind in ALL_KINDS {
            let ph = physics_for(kind, Physics::PlaneStress { thickness: 1.0 });
            let (nn, d) = (kind.n_nodes(), kind.dim());
            let xyz = distorted(kind);
            let h = [[1e-3, 2e-4, -3e-4], [5e-4, -2e-3, 1e-4], [-1e-4, 3e-4, 7e-4]];
            let mut u = vec![0.0; nn * d];
            for a in 0..nn {
                for i in 0..d {
                    u[a * d + i] = (0..d).map(|k| h[i][k] * xyz[a][k]).sum::<f64>() + 1e-3 * (i as f64 + 1.0);
                }
            }
            geometry(kind, ph, &xyz, &mut work).unwrap();
            for g in 0..kind.table().ngp {
                let e = strain_at(kind, ph, &work, g, &u);
                assert!((e[0] - h[0][0]).abs() < 1e-12 && (e[1] - h[1][1]).abs() < 1e-12, "{kind:?}");
                assert!((e[3] - (h[0][1] + h[1][0])).abs() < 1e-12, "{kind:?}");
            }
            // Strain energy U = 1/2 u^T K u equals the integral of 1/2 sigma:eps.
            let mut ke = vec![0.0; nn * nn * d * d];
            stiffness(kind, ph, &mat, &xyz, &mut work, &mut ke).unwrap();
            let mut ku = 0.0;
            for a in 0..nn {
                for b in 0..nn {
                    for i in 0..d {
                        for j in 0..d {
                            ku += u[a * d + i] * ke[(a * nn + b) * d * d + i * d + j] * u[b * d + j];
                        }
                    }
                }
            }
            geometry(kind, ph, &xyz, &mut work).unwrap();
            let mut energy = 0.0;
            for g in 0..kind.table().ngp {
                let e = strain_at(kind, ph, &work, g, &u);
                let s = stress_from_strain(ph, &mat, e, 0.0);
                let dot = if d == 2 { s[0] * e[0] + s[1] * e[1] + s[3] * e[3] } else { s[0] * e[0] + s[1] * e[1] + s[2] * e[2] + s[3] * e[3] + s[4] * e[4] + s[5] * e[5] };
                energy += work.wdet[g] * dot;
            }
            assert!((ku - energy).abs() < 1e-9 * energy.abs(), "{kind:?}: {ku} vs {energy}");
        }
    }

    #[test]
    fn fast_component_major_kernel_equals_the_node_pair_kernel() {
        let mat = Elastic::new(10.0e6, 0.33);
        let mut work = Work::new();
        for kind in ALL_KINDS {
            for plane in [Physics::PlaneStress { thickness: 0.25 }, Physics::PlaneStrain { thickness: 1.0 }] {
                let ph = physics_for(kind, plane);
                let (nn, d) = (kind.n_nodes(), kind.dim());
                let xyz = distorted(kind);
                let mut want = vec![0.0; nn * nn * d * d];
                stiffness(kind, ph, &mat, &xyz, &mut work, &mut want).unwrap();
                let mut got = vec![0.0; MAX_NODES * MAX_NODES * 10];
                let layout = stiffness_fast(kind, ph, &mat, &xyz, &mut work, &mut got).unwrap();
                assert_eq!(layout, Layout::ComponentMajor);
                let scale = want.iter().fold(0.0f64, |m, v| m.max(v.abs()));
                for a in 0..nn {
                    for b in 0..nn {
                        for i in 0..d {
                            for j in 0..d {
                                let (w, g) = (entry(Layout::NodeBlock, &want, nn, d, a, b, i, j), entry(layout, &got, nn, d, a, b, i, j));
                                assert!((w - g).abs() < 1e-12 * scale, "{kind:?} ({a},{b}) ({i},{j}): {w} vs {g}");
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn inverted_element_is_rejected() {
        let mut work = Work::new();
        let mut xyz: Vec<[f64; 3]> = ElementKind::Quad4.node_coords().to_vec();
        xyz.swap(1, 3); // mirrored
        let r = geometry(ElementKind::Quad4, Physics::PlaneStrain { thickness: 1.0 }, &xyz, &mut work);
        assert!(matches!(r, Err(ElementError::BadJacobian { .. })), "{r:?}");
    }
}
