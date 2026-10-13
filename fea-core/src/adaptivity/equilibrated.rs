//! H(div)-conforming equilibrated stress reconstruction and the estimator built on it (2D, triangles, linear elasticity).
//!
//! **Method** (Braess-Schoberl / Ern-Vohralik vertex patches, applied to each row of the stress tensor). For a finite-element
//! stress `sigma_h` (continuous displacements of degree `p`, so `sigma_h` is discontinuous of degree `p - 1`) and the hat
//! function `psi_a` of a corner vertex `a`, the row `i` of a patch flux `s_a` is the minimiser of
//! `|| s_a - psi_a sigma_h,i ||_{L2(patch)}` over the Raviart-Thomas fields of degree `k = p` on the patch triangles with
//!
//! - `div s_a = -psi_a f_i + grad(psi_a) . sigma_h,i` in every triangle,
//! - continuous normal trace across the edges that contain `a` (the fields are elementwise RT, the constraint is imposed on the
//!   edge moments against `P_k`),
//! - zero normal trace on the edges opposite `a`,
//! - on a boundary edge containing `a`: `s_a . n = psi_a t_i` (the applied traction component), unless the displacement
//!   component `(a, i)` is constrained, in which case the edge is free (the reaction takes the difference).
//!
//! The sum over the corner vertices of an element, `sigma* = sum_a s_a`, satisfies `-div sigma*_i = f_i` exactly (the hat
//! functions sum to one and their gradients to zero), has continuous normal traces on every interior edge and carries the
//! applied tractions: a statically admissible stress field. Row-wise reconstruction does not enforce symmetry, so the result
//! is an estimator (the symmetric part is compared to `sigma_h` in the `C^-1` norm, the skew part is reported), not a certified
//! upper bound. Local problems are solved as dense KKT systems after a Gram-Schmidt pass removes the constraint rows that are
//! linearly dependent by the divergence theorem (their inconsistency, if any, is the compatibility defect and is reported).
//!
//! Supported: Tri3 and Tri6, plane stress and plane strain, isotropic or anisotropic (plane stress) material, body force,
//! uniform `delta_t`, `Traction` and `Pressure` boundary loads. Anything else is `Unsupported`, never approximated.
//! Curved (non-affine) Tri6 edges are reconstructed on the straight triangle: the field is then no longer exactly
//! equilibrated for the curved discretisation and the estimate is `Degraded`.

#![allow(clippy::needless_range_loop)] // index-parallel numeric kernels read clearer as loops

use super::estimator::{ErrorEstimator, ErrorField, EstimatorContext, Indicator, Reliability};
use crate::analysis::stress_at_xi;
use crate::element::ElementKind;
use crate::mesh::Physics;
use crate::meshing::MeshError;
use crate::recover::complementary_energy;
use rayon::prelude::*;
use std::collections::HashMap;
use std::time::Instant;

const NQ: usize = 6; // Gauss points per direction (triangles: collapsed rule, edges: 1D rule)

/// Gauss-Legendre points and weights on `[-1, 1]` (6 points).
const GL6: [(f64, f64); 6] = [
    (-0.932_469_514_203_152, 0.171_324_492_379_170_35),
    (-0.661_209_386_466_264_5, 0.360_761_573_048_138_6),
    (-0.238_619_186_083_196_9, 0.467_913_934_572_691),
    (0.238_619_186_083_196_9, 0.467_913_934_572_691),
    (0.661_209_386_466_264_5, 0.360_761_573_048_138_6),
    (0.932_469_514_203_152, 0.171_324_492_379_170_35),
];

/// Collapsed Gauss rule on the reference triangle `(0,0), (1,0), (0,1)`: points `(xi, eta)` and weights (sum 1/2).
fn tri_rule() -> Vec<([f64; 2], f64)> {
    let mut out = Vec::with_capacity(NQ * NQ);
    for &(ui, wi) in &GL6 {
        for &(vj, wj) in &GL6 {
            let (u, v) = (0.5 * (ui + 1.0), 0.5 * (vj + 1.0));
            out.push(([u * (1.0 - v), v], 0.25 * wi * wj * (1.0 - v)));
        }
    }
    out
}

fn legendre(l: usize, t: f64) -> f64 {
    match l {
        0 => 1.0,
        1 => t,
        _ => 0.5 * (3.0 * t * t - 1.0),
    }
}

/// Monomial exponents `(a, b)` of `P_k` in the order of the RT basis construction.
fn monomials(k: usize) -> Vec<(usize, usize)> {
    let mut v = Vec::new();
    for deg in 0..=k {
        for a in 0..=deg {
            v.push((a, deg - a));
        }
    }
    v
}

fn mono(a: usize, b: usize, x: f64, y: f64) -> f64 {
    x.powi(a as i32) * y.powi(b as i32)
}

/// Raviart-Thomas basis of degree `k` in scaled local coordinates: `P_k^2` (monomial times a unit vector) and `x * P~_k`.
/// Returns `(values, divergences)`; values are `[nb][2]`, divergences `[nb]` (with respect to the scaled coordinates).
fn rt_basis(k: usize, x: f64, y: f64) -> (Vec<[f64; 2]>, Vec<f64>) {
    let mut val = Vec::new();
    let mut div = Vec::new();
    for (a, b) in monomials(k) {
        let m = mono(a, b, x, y);
        let dx = if a > 0 { a as f64 * mono(a - 1, b, x, y) } else { 0.0 };
        let dy = if b > 0 { b as f64 * mono(a, b - 1, x, y) } else { 0.0 };
        val.push([m, 0.0]);
        div.push(dx);
        val.push([0.0, m]);
        div.push(dy);
    }
    for a in 0..=k {
        let m = mono(a, k - a, x, y);
        val.push([m * x, m * y]);
        div.push((2 + k) as f64 * m);
    }
    (val, div)
}

fn n_rt(k: usize) -> usize {
    (k + 1) * (k + 3)
}

/// Dense solve with partial pivoting; `None` for a singular matrix.
fn solve_dense(a: &mut [f64], n: usize, b: &mut [f64]) -> Option<()> {
    for col in 0..n {
        let piv = (col..n).max_by(|&i, &j| a[i * n + col].abs().total_cmp(&a[j * n + col].abs()))?;
        if a[piv * n + col].abs() < 1e-300 {
            return None;
        }
        if piv != col {
            for c in 0..n {
                a.swap(piv * n + c, col * n + c);
            }
            b.swap(piv, col);
        }
        let d = a[col * n + col];
        for r in col + 1..n {
            let f = a[r * n + col] / d;
            if f != 0.0 {
                for c in col..n {
                    a[r * n + c] -= f * a[col * n + c];
                }
                b[r] -= f * b[col];
            }
        }
    }
    for col in (0..n).rev() {
        let mut s = b[col];
        for c in col + 1..n {
            s -= a[col * n + c] * b[c];
        }
        b[col] = s / a[col * n + col];
    }
    Some(())
}

struct ElemData {
    block: usize,
    corners: [usize; 3],
    p: [[f64; 2]; 3],
    c: [f64; 2],
    s: f64,
    /// gradient of the barycentric coordinate of corner a
    grad: [[f64; 2]; 3],
    /// quadrature points: physical position, weight (physical), RT basis values / divergences (physical), P_k test values,
    /// sigma_h = [xx, yy, xy], barycentric coordinates
    q_x: Vec<[f64; 2]>,
    q_w: Vec<f64>,
    q_v: Vec<Vec<[f64; 2]>>,
    q_div: Vec<Vec<f64>>,
    q_test: Vec<Vec<f64>>,
    q_sigma: Vec<[f64; 3]>,
    q_lambda: Vec<[f64; 3]>,
    /// mass matrix of the RT basis
    mass: Vec<f64>,
    body: [f64; 2],
}

struct EdgeRec {
    a: usize,
    b: usize,
    pa: [f64; 2],
    pb: [f64; 2],
    len: f64,
    owners: Vec<(usize, usize)>, // (element, local edge)
}

impl EdgeRec {
    fn point(&self, t: f64) -> [f64; 2] {
        let s = 0.5 * (t + 1.0);
        [self.pa[0] + s * (self.pb[0] - self.pa[0]), self.pa[1] + s * (self.pb[1] - self.pa[1])]
    }
}

/// Outward normal of local edge `k` of `e` (edge from corner k to k+1, counter-clockwise element).
fn outward_normal(e: &ElemData, k: usize) -> [f64; 2] {
    let (p, q) = (e.p[k], e.p[(k + 1) % 3]);
    let l = (q[0] - p[0]).hypot(q[1] - p[1]);
    [(q[1] - p[1]) / l, -(q[0] - p[0]) / l]
}

/// The reconstruction: per element and stress row, the coefficients of the RT basis.
pub struct Reconstruction {
    k: usize,
    elems: Vec<ElemData>,
    edges: Vec<EdgeRec>,
    coef: Vec<[Vec<f64>; 2]>,
    pub report: EquilibrationReport,
}

/// Checks of the reconstructed field, all relative to the stress scale (so they are dimensionless).
#[derive(Debug, Clone, Default)]
pub struct EquilibrationReport {
    pub degree: usize,
    pub patches: usize,
    /// Largest `|div sigma*_i + f_i|` at quadrature points.
    pub max_div_residual: f64,
    /// Largest jump of the normal traction of a row across an interior edge.
    pub max_normal_jump: f64,
    /// Largest deviation of the boundary traction from the applied one on Neumann edges (free edges are not checked).
    pub max_neumann_residual: f64,
    /// Largest compatibility defect found by a local problem (the divergence theorem failing by this much, relative).
    pub max_compat_defect: f64,
    /// `||skew(sigma*)|| / ||sigma*||` (zero for a symmetric reconstruction).
    pub skew_fraction: f64,
    /// All elements are straight-sided.
    pub affine: bool,
    pub stress_scale: f64,
}

impl EquilibrationReport {
    /// The field is equilibrated to round-off (a sound basis for the estimate).
    pub fn equilibrated(&self) -> bool {
        self.max_div_residual < 1e-8 && self.max_normal_jump < 1e-8 && self.max_neumann_residual < 1e-8
    }
}

struct PatchSolution {
    parts: Vec<(usize, usize, Vec<f64>)>, // (element, row, coefficients)
    defect: f64,
}

fn unsupported(why: impl Into<String>) -> MeshError {
    MeshError::Unsupported(why.into())
}

/// Reconstruct the equilibrated stress of a solved model. Errors with `Unsupported` outside the supported class.
pub fn reconstruct(ctx: &EstimatorContext) -> Result<Reconstruction, MeshError> {
    let model = ctx.model;
    let mesh = &model.mesh;
    if !matches!(mesh.physics, Physics::PlaneStress { .. } | Physics::PlaneStrain { .. }) {
        return Err(unsupported("the equilibrated reconstruction is for plane stress and plane strain"));
    }
    if ctx.loads.temperature.is_some() {
        return Err(unsupported("nodal temperature fields"));
    }
    if !ctx.loads.nodal.is_empty() {
        return Err(unsupported("nodal point loads (a point force is not in H(div))"));
    }
    if !ctx.loads.field_faces.is_empty() {
        return Err(unsupported("position-dependent boundary force fields"));
    }
    if mesh.blocks.iter().any(|b| b.plasticity.is_some()) {
        return Err(unsupported("plastic blocks"));
    }
    let mut p_deg = 1usize;
    for b in &mesh.blocks {
        match b.kind {
            ElementKind::Tri3 => {}
            ElementKind::Tri6 => p_deg = 2,
            other => return Err(unsupported(format!("{other:?} elements (triangles only)"))),
        }
    }
    let k = p_deg;
    let nb = n_rt(k);
    let tests = monomials(k);
    let rule = tri_rule();
    let plane_strain = matches!(mesh.physics, Physics::PlaneStrain { .. });

    // body force of every block
    let mut body_of_block = vec![[0.0f64; 2]; mesh.blocks.len()];
    for (b, f) in body_of_block.iter_mut().enumerate() {
        if let Some(g) = ctx.loads.body {
            f[0] += g[0];
            f[1] += g[1];
        }
        for &(bb, g) in &ctx.loads.block_body {
            if bb == b {
                f[0] += g[0];
                f[1] += g[1];
            }
        }
    }

    // elements
    let mut elems: Vec<ElemData> = Vec::with_capacity(mesh.n_elems());
    let mut affine = true;
    let mut stress_scale = 0.0f64;
    for (bi, blk) in mesh.blocks.iter().enumerate() {
        let nn = blk.kind.n_nodes();
        for e in 0..blk.n_elems() {
            let conn = blk.elem(e);
            let corners = [conn[0], conn[1], conn[2]];
            let p: [[f64; 2]; 3] = std::array::from_fn(|a| [mesh.nodes[corners[a]][0], mesh.nodes[corners[a]][1]]);
            let area = 0.5 * ((p[1][0] - p[0][0]) * (p[2][1] - p[0][1]) - (p[1][1] - p[0][1]) * (p[2][0] - p[0][0]));
            if area <= 0.0 {
                return Err(MeshError::InvalidElements { count: 1, first: (bi, e) });
            }
            let h = (p[1][0] - p[0][0]).hypot(p[1][1] - p[0][1]);
            if nn == 6 {
                for m in 0..3 {
                    let (q0, q1) = (p[m], p[(m + 1) % 3]);
                    let mid = mesh.nodes[conn[3 + m]];
                    if (mid[0] - 0.5 * (q0[0] + q1[0])).hypot(mid[1] - 0.5 * (q0[1] + q1[1])) > 1e-9 * h {
                        affine = false;
                    }
                }
            }
            let c = [(p[0][0] + p[1][0] + p[2][0]) / 3.0, (p[0][1] + p[1][1] + p[2][1]) / 3.0];
            let s = (2.0 * area).sqrt();
            let grad: [[f64; 2]; 3] = std::array::from_fn(|a| {
                let (b, cc) = (p[(a + 1) % 3], p[(a + 2) % 3]);
                [(b[1] - cc[1]) / (2.0 * area), (cc[0] - b[0]) / (2.0 * area)]
            });
            let xyz: Vec<[f64; 3]> = conn.iter().map(|&n| mesh.nodes[n]).collect();
            let ue: Vec<f64> = conn.iter().flat_map(|&n| [ctx.u[2 * n], ctx.u[2 * n + 1]]).collect();
            let (mut q_x, mut q_w, mut q_v, mut q_div, mut q_test, mut q_sigma, mut q_lambda) = (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
            let mut mass = vec![0.0; nb * nb];
            for &(r, wref) in &rule {
                let x = [p[0][0] + (p[1][0] - p[0][0]) * r[0] + (p[2][0] - p[0][0]) * r[1], p[0][1] + (p[1][1] - p[0][1]) * r[0] + (p[2][1] - p[0][1]) * r[1]];
                let w = 2.0 * area * wref;
                let (xl, yl) = ((x[0] - c[0]) / s, (x[1] - c[1]) / s);
                let (val, dv) = rt_basis(k, xl, yl);
                let divp: Vec<f64> = dv.iter().map(|d| d / s).collect();
                let test: Vec<f64> = tests.iter().map(|&(a, b)| mono(a, b, xl, yl)).collect();
                let st = stress_at_xi(blk.kind, mesh.physics, &blk.material, &xyz, &ue, [r[0], r[1], 0.0], ctx.loads.delta_t).map_err(MeshError::Failed)?;
                stress_scale = stress_scale.max(st[0].abs()).max(st[1].abs()).max(st[3].abs());
                for i in 0..nb {
                    for j in 0..nb {
                        mass[i * nb + j] += w * (val[i][0] * val[j][0] + val[i][1] * val[j][1]);
                    }
                }
                q_x.push(x);
                q_w.push(w);
                q_v.push(val);
                q_div.push(divp);
                q_test.push(test);
                q_sigma.push([st[0], st[1], st[3]]);
                q_lambda.push([1.0 - r[0] - r[1], r[0], r[1]]);
            }
            elems.push(ElemData { block: bi, corners, p, c, s, grad, q_x, q_w, q_v, q_div, q_test, q_sigma, q_lambda, mass, body: body_of_block[bi] });
        }
    }
    if stress_scale <= 0.0 {
        stress_scale = 1.0;
    }

    // edges
    let mut edge_ids: HashMap<(usize, usize), usize> = HashMap::new();
    let mut edges: Vec<EdgeRec> = Vec::new();
    let mut elem_edges: Vec<[usize; 3]> = Vec::with_capacity(elems.len());
    for (ei, el) in elems.iter().enumerate() {
        let mut ids = [0usize; 3];
        for kk in 0..3 {
            let (n0, n1) = (el.corners[kk], el.corners[(kk + 1) % 3]);
            let key = (n0.min(n1), n0.max(n1));
            let id = *edge_ids.entry(key).or_insert_with(|| {
                let (pa, pb) = if n0 < n1 { (el.p[kk], el.p[(kk + 1) % 3]) } else { (el.p[(kk + 1) % 3], el.p[kk]) };
                let len = (pb[0] - pa[0]).hypot(pb[1] - pa[1]);
                edges.push(EdgeRec { a: key.0, b: key.1, pa, pb, len, owners: Vec::new() });
                edges.len() - 1
            });
            edges[id].owners.push((ei, kk));
            ids[kk] = id;
        }
        elem_edges.push(ids);
    }
    if edges.iter().any(|e| e.owners.len() > 2) {
        return Err(MeshError::NonConforming("an edge is shared by more than two elements".into()));
    }

    // applied boundary tractions per edge (stress units): Traction is constant, Pressure follows the outward normal
    let mut traction: HashMap<usize, [f64; 2]> = HashMap::new();
    let mut add_load = |nodes: &[usize], load: crate::loads::SurfaceLoad| -> Result<(), MeshError> {
        let key = (nodes[0].min(nodes[1]), nodes[0].max(nodes[1]));
        let id = *edge_ids.get(&key).ok_or_else(|| MeshError::Failed("a loaded face is not an edge of the mesh".into()))?;
        let t = match load {
            crate::loads::SurfaceLoad::Traction(t) => [t[0], t[1]],
            crate::loads::SurfaceLoad::Pressure(p) => {
                let (ei, kk) = edges[id].owners[0];
                let n = outward_normal(&elems[ei], kk);
                [-p * n[0], -p * n[1]]
            }
        };
        let e = traction.entry(id).or_insert([0.0, 0.0]);
        e[0] += t[0];
        e[1] += t[1];
        Ok(())
    };
    for (nodes, load) in &ctx.loads.faces {
        add_load(nodes, *load)?;
    }
    for (nodes, p) in &ctx.loads.followers {
        add_load(nodes, crate::loads::SurfaceLoad::Pressure(*p))?;
    }

    // vertex patches
    let n_nodes = mesh.nodes.len();
    let mut around: Vec<Vec<usize>> = vec![Vec::new(); n_nodes];
    for (ei, el) in elems.iter().enumerate() {
        for &c in &el.corners {
            around[c].push(ei);
        }
    }
    let vertices: Vec<usize> = (0..n_nodes).filter(|&v| !around[v].is_empty()).collect();
    let solve_patch = |&a: &usize| -> Result<PatchSolution, MeshError> {
        let members = &around[a];
        let n_el = members.len();
        let n_a = n_el * nb;
        let mut parts = Vec::new();
        let mut defect = 0.0f64;
        for row in 0..2usize {
            let constrained = ctx.bc.fixed[2 * a + row];
            // constraints as dense rows over the n_a unknowns with their right-hand sides
            let mut rows: Vec<(Vec<f64>, f64)> = Vec::new();
            let mut r = vec![0.0; n_a];
            for (m, &ei) in members.iter().enumerate() {
                let el = &elems[ei];
                let la = el.corners.iter().position(|&c| c == a).expect("the vertex belongs to its patch element");
                let mut gq = vec![0.0; tests.len()];
                for (q, &w) in el.q_w.iter().enumerate() {
                    let psi = el.q_lambda[q][la];
                    let sig_row = if row == 0 { [el.q_sigma[q][0], el.q_sigma[q][2]] } else { [el.q_sigma[q][2], el.q_sigma[q][1]] };
                    let g = -psi * el.body[row] + el.grad[la][0] * sig_row[0] + el.grad[la][1] * sig_row[1];
                    for j in 0..nb {
                        r[m * nb + j] += w * psi * (el.q_v[q][j][0] * sig_row[0] + el.q_v[q][j][1] * sig_row[1]);
                    }
                    for (t, gv) in gq.iter_mut().enumerate() {
                        *gv += w * g * el.q_test[q][t];
                    }
                }
                for t in 0..tests.len() {
                    let mut c = vec![0.0; n_a];
                    for q in 0..el.q_w.len() {
                        for j in 0..nb {
                            c[m * nb + j] += el.q_w[q] * el.q_div[q][j] * el.q_test[q][t];
                        }
                    }
                    rows.push((c, gq[t]));
                }
            }
            // edge moments: dense row of  int (v_j . n) P_l ds  for element index m (local position) on edge `eid`
            let edge_row = |m: usize, kk: usize, l: usize, sign: f64, c: &mut [f64]| {
                let ei = members[m];
                let el = &elems[ei];
                let ed = &edges[elem_edges[ei][kk]];
                // physical normal used for the moment: the element's outward normal times `sign`
                let n = outward_normal(el, kk);
                for &(t, w) in &GL6 {
                    let x = ed.point(t);
                    let (xl, yl) = ((x[0] - el.c[0]) / el.s, (x[1] - el.c[1]) / el.s);
                    let (val, _) = rt_basis(k, xl, yl);
                    let pl = legendre(l, t);
                    for j in 0..nb {
                        c[m * nb + j] += sign * w * 0.5 * ed.len * pl * (val[j][0] * n[0] + val[j][1] * n[1]);
                    }
                }
            };
            let pos_of = |ei: usize| members.iter().position(|&x| x == ei);
            let mut handled = std::collections::HashSet::new();
            for &ei in members.iter() {
                for kk in 0..3 {
                    let eid = elem_edges[ei][kk];
                    if !handled.insert(eid) {
                        continue;
                    }
                    let ed = &edges[eid];
                    let incident = ed.a == a || ed.b == a;
                    if ed.owners.len() == 2 {
                        let (e1, k1) = ed.owners[0];
                        let (e2, k2) = ed.owners[1];
                        match (pos_of(e1), pos_of(e2)) {
                            (Some(m1), Some(m2)) => {
                                // continuity of the normal component: sigma_1 . n1 = -sigma_2 . n2, i.e. the moments of
                                // sigma . (outward normal) of the two owners add up to zero
                                for l in 0..=k {
                                    let mut c = vec![0.0; n_a];
                                    edge_row(m1, k1, l, 1.0, &mut c);
                                    edge_row(m2, k2, l, 1.0, &mut c);
                                    rows.push((c, 0.0));
                                }
                            }
                            (Some(mm), None) | (None, Some(mm)) => {
                                let kloc = if pos_of(e1).is_some() { k1 } else { k2 };
                                // opposite edge (or an edge of the patch rim): zero flux
                                for l in 0..=k {
                                    let mut c = vec![0.0; n_a];
                                    edge_row(mm, kloc, l, 1.0, &mut c);
                                    rows.push((c, 0.0));
                                }
                            }
                            (None, None) => {}
                        }
                    } else {
                        // a boundary edge
                        let (e1, k1) = ed.owners[0];
                        let mm = pos_of(e1).expect("an edge of a patch element is owned by it");
                        if !incident {
                            for l in 0..=k {
                                let mut c = vec![0.0; n_a];
                                edge_row(mm, k1, l, 1.0, &mut c);
                                rows.push((c, 0.0));
                            }
                        } else if !constrained {
                            let t = traction.get(&eid).copied().unwrap_or([0.0, 0.0]);
                            for l in 0..=k {
                                let mut c = vec![0.0; n_a];
                                edge_row(mm, k1, l, 1.0, &mut c);
                                // rhs: int psi_a t_row P_l ds with psi_a = (1 -/+ t)/2 along the edge
                                let mut d = 0.0;
                                for &(tt, w) in &GL6 {
                                    let psi = if ed.a == a { 0.5 * (1.0 - tt) } else { 0.5 * (1.0 + tt) };
                                    d += w * 0.5 * ed.len * psi * legendre(l, tt) * t[row];
                                }
                                rows.push((c, d));
                            }
                        }
                    }
                }
            }
            // independent constraint rows (Gram-Schmidt); a dependent row's leftover rhs is the compatibility defect
            let mut basis: Vec<(Vec<f64>, f64)> = Vec::new(); // orthonormal rows with transformed rhs
            let mut kept: Vec<usize> = Vec::new();
            let scale_rhs = rows.iter().map(|(_, d)| d.abs()).fold(0.0f64, f64::max).max(stress_scale * el_scale(&elems, members));
            for (idx, (c, d)) in rows.iter().enumerate() {
                let norm0 = c.iter().map(|v| v * v).sum::<f64>().sqrt();
                if norm0 == 0.0 {
                    defect = defect.max(d.abs() / scale_rhs);
                    continue;
                }
                let (mut cc, mut dd) = (c.clone(), *d);
                for _ in 0..2 {
                    for (q, dq) in &basis {
                        let proj: f64 = cc.iter().zip(q).map(|(a, b)| a * b).sum();
                        for (x, qv) in cc.iter_mut().zip(q) {
                            *x -= proj * qv;
                        }
                        dd -= proj * dq;
                    }
                }
                let n1 = cc.iter().map(|v| v * v).sum::<f64>().sqrt();
                if n1 > 1e-9 * norm0 {
                    for x in cc.iter_mut() {
                        *x /= n1;
                    }
                    basis.push((cc, dd / n1));
                    kept.push(idx);
                } else {
                    defect = defect.max(dd.abs() / scale_rhs);
                }
            }
            // KKT system
            let nc = kept.len();
            let n = n_a + nc;
            let mut kkt = vec![0.0; n * n];
            let mut rhs = vec![0.0; n];
            for (m, &ei) in members.iter().enumerate() {
                let el = &elems[ei];
                for i in 0..nb {
                    for j in 0..nb {
                        kkt[(m * nb + i) * n + m * nb + j] = el.mass[i * nb + j];
                    }
                }
            }
            rhs[..n_a].copy_from_slice(&r[..n_a]);
            for (ci, &ri) in kept.iter().enumerate() {
                let (c, d) = &rows[ri];
                for j in 0..n_a {
                    kkt[(n_a + ci) * n + j] = c[j];
                    kkt[j * n + n_a + ci] = c[j];
                }
                rhs[n_a + ci] = *d;
            }
            solve_dense(&mut kkt, n, &mut rhs).ok_or_else(|| MeshError::Failed(format!("the local reconstruction problem at node {a} is singular")))?;
            for (m, &ei) in members.iter().enumerate() {
                parts.push((ei, row, rhs[m * nb..(m + 1) * nb].to_vec()));
            }
        }
        Ok(PatchSolution { parts, defect })
    };
    let solutions: Vec<PatchSolution> = if vertices.len() >= 200 {
        vertices.par_iter().map(solve_patch).collect::<Result<_, _>>()?
    } else {
        vertices.iter().map(solve_patch).collect::<Result<_, _>>()?
    };
    let mut coef: Vec<[Vec<f64>; 2]> = (0..elems.len()).map(|_| [vec![0.0; nb], vec![0.0; nb]]).collect();
    let mut max_defect = 0.0f64;
    for sol in &solutions {
        max_defect = max_defect.max(sol.defect);
        for (ei, row, c) in &sol.parts {
            for (x, v) in coef[*ei][*row].iter_mut().zip(c) {
                *x += v;
            }
        }
    }
    let mut rec = Reconstruction { k, elems, edges, coef, report: EquilibrationReport { degree: k, patches: vertices.len(), max_compat_defect: max_defect, affine, stress_scale, ..Default::default() } };
    rec.verify(ctx, &traction, plane_strain);
    Ok(rec)
}

fn el_scale(elems: &[ElemData], members: &[usize]) -> f64 {
    members.iter().map(|&m| elems[m].s).fold(0.0f64, f64::max).max(1e-300)
}

impl Reconstruction {
    /// `sigma*` rows at physical point `x` of element `ei`: `[[sxx, sxy], [syx, syy]]`.
    pub fn stress_at(&self, ei: usize, x: [f64; 2]) -> [[f64; 2]; 2] {
        let el = &self.elems[ei];
        let (val, _) = rt_basis(self.k, (x[0] - el.c[0]) / el.s, (x[1] - el.c[1]) / el.s);
        let mut out = [[0.0; 2]; 2];
        for row in 0..2 {
            for (j, v) in val.iter().enumerate() {
                out[row][0] += self.coef[ei][row][j] * v[0];
                out[row][1] += self.coef[ei][row][j] * v[1];
            }
        }
        out
    }

    fn div_at(&self, ei: usize, x: [f64; 2]) -> [f64; 2] {
        let el = &self.elems[ei];
        let (_, dv) = rt_basis(self.k, (x[0] - el.c[0]) / el.s, (x[1] - el.c[1]) / el.s);
        let mut out = [0.0; 2];
        for row in 0..2 {
            for (j, d) in dv.iter().enumerate() {
                out[row] += self.coef[ei][row][j] * d / el.s;
            }
        }
        out
    }

    fn verify(&mut self, ctx: &EstimatorContext, traction: &HashMap<usize, [f64; 2]>, _plane_strain: bool) {
        let scale = self.report.stress_scale;
        let (mut div_res, mut jump, mut neu) = (0.0f64, 0.0f64, 0.0f64);
        for (ei, el) in self.elems.iter().enumerate() {
            let hs = el.s;
            for &x in &el.q_x {
                let d = self.div_at(ei, x);
                for row in 0..2 {
                    div_res = div_res.max((d[row] + el.body[row]).abs() * hs / scale);
                }
            }
        }
        for (eid, ed) in self.edges.iter().enumerate() {
            for &(t, _) in &GL6 {
                let x = ed.point(t);
                if ed.owners.len() == 2 {
                    let (e1, k1) = ed.owners[0];
                    let (e2, k2) = ed.owners[1];
                    let (n1, n2) = (outward_normal(&self.elems[e1], k1), outward_normal(&self.elems[e2], k2));
                    let (s1, s2) = (self.stress_at(e1, x), self.stress_at(e2, x));
                    for row in 0..2 {
                        let f1 = s1[row][0] * n1[0] + s1[row][1] * n1[1];
                        let f2 = s2[row][0] * n2[0] + s2[row][1] * n2[1];
                        jump = jump.max((f1 + f2).abs() / scale);
                    }
                } else {
                    let (e1, k1) = ed.owners[0];
                    let n = outward_normal(&self.elems[e1], k1);
                    let s = self.stress_at(e1, x);
                    let tr = traction.get(&eid).copied().unwrap_or([0.0, 0.0]);
                    for row in 0..2 {
                        // a row is checked only where both end nodes are free in that component
                        if ctx.bc.fixed[2 * ed.a + row] || ctx.bc.fixed[2 * ed.b + row] {
                            continue;
                        }
                        let f = s[row][0] * n[0] + s[row][1] * n[1];
                        neu = neu.max((f - tr[row]).abs() / scale);
                    }
                }
            }
        }
        let (mut skew2, mut tot2) = (0.0f64, 0.0f64);
        for (ei, el) in self.elems.iter().enumerate() {
            for (q, &x) in el.q_x.iter().enumerate() {
                let s = self.stress_at(ei, x);
                let sk = 0.5 * (s[0][1] - s[1][0]);
                skew2 += el.q_w[q] * sk * sk;
                tot2 += el.q_w[q] * (s[0][0] * s[0][0] + s[1][1] * s[1][1] + 0.5 * (s[0][1] + s[1][0]).powi(2) + sk * sk);
            }
        }
        self.report.max_div_residual = div_res;
        self.report.max_normal_jump = jump;
        self.report.max_neumann_residual = neu;
        self.report.skew_fraction = if tot2 > 0.0 { (skew2 / tot2).sqrt() } else { 0.0 };
    }

    /// Element error indicators `eta_K = || sym(sigma*) - sigma_h ||_{C^-1, K}` (global element order).
    pub fn indicators(&self, ctx: &EstimatorContext) -> Vec<f64> {
        let mesh = &ctx.model.mesh;
        let plane_strain = matches!(mesh.physics, Physics::PlaneStrain { .. });
        self.elems
            .iter()
            .enumerate()
            .map(|(ei, el)| {
                let mat = &mesh.blocks[el.block].material;
                let nu = mat.nu;
                let mut sum = 0.0;
                for (q, &x) in el.q_x.iter().enumerate() {
                    let s = self.stress_at(ei, x);
                    let sh = el.q_sigma[q];
                    let (dxx, dyy, dxy) = (s[0][0] - sh[0], s[1][1] - sh[1], 0.5 * (s[0][1] + s[1][0]) - sh[2]);
                    let dzz = if plane_strain && mat.aniso.is_none() { nu * (dxx + dyy) } else { 0.0 };
                    sum += el.q_w[q] * 2.0 * complementary_energy(mat, &[dxx, dyy, dzz, dxy, 0.0, 0.0]);
                }
                sum.max(0.0).sqrt()
            })
            .collect()
    }
}

/// Estimator from the equilibrated stress reconstruction.
pub struct EquilibratedEstimator;

impl ErrorEstimator for EquilibratedEstimator {
    fn name(&self) -> &str {
        "equilibrated"
    }

    fn estimate(&self, ctx: &EstimatorContext) -> Result<ErrorField, MeshError> {
        let t = Instant::now();
        let rec = match reconstruct(ctx) {
            Ok(r) => r,
            Err(MeshError::Unsupported(why)) => return Ok(ErrorField::unsupported("equilibrated", &why, ctx.n_elems())),
            Err(e) => return Err(e),
        };
        let eta = rec.indicators(ctx);
        let total = eta.iter().map(|v| v * v).sum::<f64>().sqrt();
        let rep = &rec.report;
        let reliability = if !rep.equilibrated() {
            Reliability::Degraded(format!("the reconstruction is not equilibrated to round-off (div {:.1e}, jump {:.1e}, traction {:.1e}); the geometry is {}", rep.max_div_residual, rep.max_normal_jump, rep.max_neumann_residual, if rep.affine { "straight-sided" } else { "curved and treated as straight" }))
        } else if !rep.affine {
            Reliability::Degraded("curved element edges are reconstructed on straight triangles".into())
        } else if rep.skew_fraction > 0.05 {
            Reliability::Degraded(format!("the row-wise reconstruction is {:.1} % skew", 100.0 * rep.skew_fraction))
        } else {
            Reliability::Reliable
        };
        let mut f = ErrorField::new("equilibrated", Indicator::Marking, eta, total, reliability, t.elapsed().as_secs_f64());
        f.notes.push(format!(
            "RT degree {}, {} patches; div residual {:.1e}, normal jump {:.1e}, traction residual {:.1e}, compatibility defect {:.1e}, skew {:.2e}",
            rep.degree, rep.patches, rep.max_div_residual, rep.max_normal_jump, rep.max_neumann_residual, rep.max_compat_defect, rep.skew_fraction
        ));
        Ok(f)
    }
}
