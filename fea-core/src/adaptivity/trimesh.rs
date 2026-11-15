//! Geometry, edges and boundary data of a triangle mesh in the form the 2D estimators need: elements with their corner
//! coordinates and barycentric gradients, the edges with their owners, the applied boundary traction per edge and the body force
//! per block. One place checks that the problem is in the supported class (Tri3 / Tri6, plane stress or strain, linear elastic,
//! `Traction` / `Pressure` loads, no point loads or temperature fields) so the estimators built on it cannot disagree.

use super::estimator::EstimatorContext;
use crate::element::ElementKind;
use crate::loads::SurfaceLoad;
use crate::mesh::Physics;
use crate::meshing::MeshError;
use std::collections::HashMap;

pub(crate) struct TriElem {
    pub block: usize,
    pub local: usize,
    pub corners: [usize; 3],
    /// Corner coordinates, counter-clockwise.
    pub p: [[f64; 2]; 3],
    pub c: [f64; 2],
    /// Length scale `sqrt(2 area)`.
    pub s: f64,
    pub area: f64,
    /// Gradient of the barycentric coordinate of corner `a`.
    pub grad: [[f64; 2]; 3],
}

pub(crate) struct TriEdge {
    /// Corner nodes, `a < b`.
    pub a: usize,
    pub b: usize,
    pub pa: [f64; 2],
    pub pb: [f64; 2],
    pub len: f64,
    /// `(element, local edge)`; local edge `k` joins corners `k` and `k + 1`.
    pub owners: Vec<(usize, usize)>,
}

impl TriEdge {
    /// Point at parameter `t` in `[-1, 1]` from `a` towards `b`.
    pub fn point(&self, t: f64) -> [f64; 2] {
        let s = 0.5 * (t + 1.0);
        [self.pa[0] + s * (self.pb[0] - self.pa[0]), self.pa[1] + s * (self.pb[1] - self.pa[1])]
    }
}

pub(crate) struct TriMesh {
    pub elems: Vec<TriElem>,
    pub edges: Vec<TriEdge>,
    pub elem_edges: Vec<[usize; 3]>,
    /// Applied traction per boundary edge (force per area).
    pub traction: HashMap<usize, [f64; 2]>,
    /// Body force of every block.
    pub body: Vec<[f64; 2]>,
    /// Every element is straight-sided.
    pub affine: bool,
    /// Polynomial degree of the displacement (1 for Tri3, 2 for Tri6).
    pub degree: usize,
}

/// Outward normal of local edge `k` of a counter-clockwise triangle with corners `p`.
pub(crate) fn outward_normal(p: &[[f64; 2]; 3], k: usize) -> [f64; 2] {
    let (a, b) = (p[k], p[(k + 1) % 3]);
    let l = (b[0] - a[0]).hypot(b[1] - a[1]);
    [(b[1] - a[1]) / l, -(b[0] - a[0]) / l]
}

fn unsupported(why: impl Into<String>) -> MeshError {
    MeshError::Unsupported(why.into())
}

impl TriMesh {
    pub fn build(ctx: &EstimatorContext) -> Result<Self, MeshError> {
        let mesh = &ctx.model.mesh;
        if !matches!(mesh.physics, Physics::PlaneStress { .. } | Physics::PlaneStrain { .. }) {
            return Err(unsupported("the 2D estimators are for plane stress and plane strain"));
        }
        if ctx.loads.temperature.is_some() {
            return Err(unsupported("nodal temperature fields"));
        }
        if !ctx.loads.nodal.is_empty() {
            return Err(unsupported("nodal point loads (a point force is not in H(div) and has no pointwise residual)"));
        }
        if !ctx.loads.field_faces.is_empty() {
            return Err(unsupported("position-dependent boundary force fields"));
        }
        if mesh.blocks.iter().any(|b| b.plasticity.is_some()) {
            return Err(unsupported("plastic blocks"));
        }
        let mut degree = 1usize;
        for b in &mesh.blocks {
            match b.kind {
                ElementKind::Tri3 => {}
                ElementKind::Tri6 => degree = 2,
                other => return Err(unsupported(format!("{other:?} elements (triangles only)"))),
            }
        }
        let mut body = vec![[0.0f64; 2]; mesh.blocks.len()];
        for (b, f) in body.iter_mut().enumerate() {
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
        let mut elems = Vec::with_capacity(mesh.n_elems());
        let mut affine = true;
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
                let grad: [[f64; 2]; 3] = std::array::from_fn(|a| {
                    let (b, cc) = (p[(a + 1) % 3], p[(a + 2) % 3]);
                    [(b[1] - cc[1]) / (2.0 * area), (cc[0] - b[0]) / (2.0 * area)]
                });
                elems.push(TriElem { block: bi, local: e, corners, p, c, s: (2.0 * area).sqrt(), area, grad });
            }
        }
        let mut edge_ids: HashMap<(usize, usize), usize> = HashMap::new();
        let mut edges: Vec<TriEdge> = Vec::new();
        let mut elem_edges: Vec<[usize; 3]> = Vec::with_capacity(elems.len());
        for (ei, el) in elems.iter().enumerate() {
            let mut ids = [0usize; 3];
            for (kk, slot) in ids.iter_mut().enumerate() {
                let (n0, n1) = (el.corners[kk], el.corners[(kk + 1) % 3]);
                let key = (n0.min(n1), n0.max(n1));
                let id = *edge_ids.entry(key).or_insert_with(|| {
                    let (pa, pb) = if n0 < n1 { (el.p[kk], el.p[(kk + 1) % 3]) } else { (el.p[(kk + 1) % 3], el.p[kk]) };
                    edges.push(TriEdge { a: key.0, b: key.1, pa, pb, len: (pb[0] - pa[0]).hypot(pb[1] - pa[1]), owners: Vec::new() });
                    edges.len() - 1
                });
                edges[id].owners.push((ei, kk));
                *slot = id;
            }
            elem_edges.push(ids);
        }
        if edges.iter().any(|e| e.owners.len() > 2) {
            return Err(MeshError::NonConforming("an edge is shared by more than two elements".into()));
        }
        let mut traction: HashMap<usize, [f64; 2]> = HashMap::new();
        let mut add_load = |nodes: &[usize], load: SurfaceLoad| -> Result<(), MeshError> {
            let key = (nodes[0].min(nodes[1]), nodes[0].max(nodes[1]));
            let id = *edge_ids.get(&key).ok_or_else(|| MeshError::Failed("a loaded face is not an edge of the mesh".into()))?;
            let t = match load {
                SurfaceLoad::Traction(t) => [t[0], t[1]],
                SurfaceLoad::Pressure(p) => {
                    let (ei, kk) = edges[id].owners[0];
                    let n = outward_normal(&elems[ei].p, kk);
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
            add_load(nodes, SurfaceLoad::Pressure(*p))?;
        }
        Ok(Self { elems, edges, elem_edges, traction, body, affine, degree })
    }
}
