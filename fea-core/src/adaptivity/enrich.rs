//! Dual-weighted residual estimate with an h-enriched dual (2D; Tri3, Tri6, Quad4, Quad8, Quad9).
//!
//! The mesh is refined once uniformly (every element into four children; curved edges stay on their quadratic curves, because
//! every fine node is placed by the *coarse* element map). The coarse space is contained in the fine one, so `u_h` is represented
//! exactly on the fine mesh (prolongation by the coarse shape functions) and the fine residual `r = f - K u_h` is the exact
//! residual of the coarse solution tested with fine functions. The dual is solved on the fine mesh, `K_f z_f = g_f` (the goal is
//! rebuilt there by its builder), and with the coarse dual prolonged as `P z_h`,
//!
//! `J(u) - J(u_h) ~ rho(z_f - P z_h) = (z_f - P z_h) . r`      (exact minus discrete, as in [`super::dwr`]).
//!
//! The nodal products `w_i r_i` sum to the estimate; each is shared equally by the coarse elements that contain the fine node, a
//! partition that conserves the sum and keeps the sign. Cost: one assembly, one factorisation and one back-solve per goal on a
//! mesh with four times the elements. Unlike the recovery-based estimate it does not rely on superconvergent stress recovery,
//! so it is not misled by singular stress fields; its accuracy is that of the fine dual.

use super::dwr::DwrResult;
use super::estimator::{ErrorField, EstimatorContext, Indicator, Reliability};
use super::goal::LinearFunctional;
use crate::analysis::Model;
use crate::element::ElementKind;
use crate::linear::{Dirichlet, Ordering, Reduced};
use crate::loads::{self, Loads};
use crate::mesh::{Block, Mesh};
use crate::meshing::weld::NodeWelder;
use crate::meshing::MeshError;
use std::time::Instant;

/// The uniformly refined mesh with the bookkeeping to move fields and constraints between the two meshes.
pub struct Refined {
    pub mesh: Mesh,
    /// Coarse global element id of every fine element.
    pub parent_of_elem: Vec<usize>,
    /// For every fine node, a `(coarse global element, natural coordinates)` that places it.
    pub origin: Vec<(usize, [f64; 3])>,
    /// Coarse elements that contain each fine node (sorted, distinct).
    pub parents_of_node: Vec<Vec<usize>>,
    n_coarse_nodes: usize,
}

type V2 = [f64; 2];

fn mid(a: V2, b: V2) -> V2 {
    [0.5 * (a[0] + b[0]), 0.5 * (a[1] + b[1])]
}

/// Child cells in the natural coordinates of the parent: corner lists (counter-clockwise) of the four children.
fn children(kind: ElementKind) -> Vec<Vec<V2>> {
    if kind.is_simplex() {
        let (v0, v1, v2) = ([0.0, 0.0], [1.0, 0.0], [0.0, 1.0]);
        let (m01, m12, m20) = (mid(v0, v1), mid(v1, v2), mid(v2, v0));
        vec![vec![v0, m01, m20], vec![m01, v1, m12], vec![m20, m12, v2], vec![m01, m12, m20]]
    } else {
        let (v0, v1, v2, v3) = ([-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]);
        let (m01, m12, m23, m30, c) = (mid(v0, v1), mid(v1, v2), mid(v2, v3), mid(v3, v0), [0.0, 0.0]);
        vec![vec![v0, m01, c, m30], vec![m01, v1, m12, c], vec![c, m12, v2, m23], vec![m30, c, m23, v3]]
    }
}

/// All node positions of a child of kind `kind` from its corners (natural coordinates): corners, midsides (edge midpoints), centre.
fn child_nodes(kind: ElementKind, corners: &[V2]) -> Vec<V2> {
    let mut v: Vec<V2> = corners.to_vec();
    let nc = corners.len();
    if kind.n_nodes() > nc {
        for k in 0..nc {
            v.push(mid(corners[k], corners[(k + 1) % nc]));
        }
        if kind == ElementKind::Quad9 {
            let c = [corners.iter().map(|p| p[0]).sum::<f64>() / 4.0, corners.iter().map(|p| p[1]).sum::<f64>() / 4.0];
            v.push(c);
        }
    }
    v
}

/// Uniform refinement of a 2D mesh of triangles and quadrilaterals.
pub fn refine_uniform_2d(mesh: &Mesh) -> Result<Refined, MeshError> {
    if mesh.dim() != 2 {
        return Err(MeshError::Unsupported("uniform refinement for the enriched dual is implemented for 2D meshes".into()));
    }
    let (lo, hi) = mesh.nodes.iter().fold(([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]), |(l, h), x| ([l[0].min(x[0]), l[1].min(x[1])], [h[0].max(x[0]), h[1].max(x[1])]));
    let diag = (hi[0] - lo[0]).hypot(hi[1] - lo[1]).max(1e-300);
    // the welder over the node list, coarse nodes first (they keep their indices)
    let mut welder = NodeWelder::new(1e-10 * diag);
    let mut all: Vec<[f64; 3]> = Vec::new();
    for x in &mesh.nodes {
        let before = all.len();
        let id = welder.node(&mut all, *x);
        if id != before {
            return Err(MeshError::NonConforming("two coarse nodes coincide".into()));
        }
    }
    let n_coarse = all.len();
    let mut origin: Vec<(usize, [f64; 3])> = vec![(usize::MAX, [0.0; 3]); n_coarse];
    let mut parents_of_node: Vec<Vec<usize>> = vec![Vec::new(); n_coarse];
    let mut blocks: Vec<Block> = Vec::new();
    let mut parent_of_elem: Vec<usize> = Vec::new();
    let mut global = 0usize;
    for blk in &mesh.blocks {
        if !matches!(blk.kind, ElementKind::Tri3 | ElementKind::Tri6 | ElementKind::Quad4 | ElementKind::Quad8 | ElementKind::Quad9) {
            return Err(MeshError::Unsupported(format!("{:?} elements cannot be refined for the enriched dual", blk.kind)));
        }
        let kids = children(blk.kind);
        let mut conn: Vec<usize> = Vec::with_capacity(blk.n_elems() * 4 * blk.kind.n_nodes());
        for e in 0..blk.n_elems() {
            let pc = blk.elem(e);
            let xyz: Vec<[f64; 3]> = pc.iter().map(|&n| mesh.nodes[n]).collect();
            for kid in &kids {
                for xi in child_nodes(blk.kind, kid) {
                    let (n, _) = blk.kind.shape([xi[0], xi[1], 0.0]);
                    let mut x = [0.0; 3];
                    for (b, p) in xyz.iter().enumerate() {
                        for (c, v) in x.iter_mut().enumerate() {
                            *v += n[b] * p[c];
                        }
                    }
                    let id = welder.node(&mut all, x);
                    if id >= origin.len() {
                        origin.resize(id + 1, (usize::MAX, [0.0; 3]));
                        parents_of_node.resize(id + 1, Vec::new());
                    }
                    if origin[id].0 == usize::MAX {
                        origin[id] = (global, [xi[0], xi[1], 0.0]);
                    }
                    if !parents_of_node[id].contains(&(global)) {
                        parents_of_node[id].push(global);
                    }
                    conn.push(id);
                }
                parent_of_elem.push(global);
            }
            global += 1;
        }
        blocks.push(Block { kind: blk.kind, conn, material: blk.material, name: blk.name.clone(), plasticity: blk.plasticity, density: blk.density, thermal: blk.thermal.clone() });
    }
    // coarse nodes: place them by their own parent (the first element that contains them)
    let mut g = 0usize;
    for blk in &mesh.blocks {
        for e in 0..blk.n_elems() {
            for (a, &n) in blk.elem(e).iter().enumerate() {
                if origin[n].0 == usize::MAX {
                    let xi = blk.kind.node_coords()[a];
                    origin[n] = (g, xi);
                }
                if !parents_of_node[n].contains(&g) {
                    parents_of_node[n].push(g);
                }
            }
            g += 1;
        }
    }
    for p in parents_of_node.iter_mut() {
        p.sort_unstable();
        p.dedup();
    }
    let mut fine = Mesh::new(mesh.physics);
    fine.nodes = all;
    fine.blocks = blocks;
    Ok(Refined { mesh: fine, parent_of_elem, origin, parents_of_node, n_coarse_nodes: n_coarse })
}

impl Refined {
    /// A coarse nodal vector (`d` values per node) on the fine mesh.
    pub fn prolong(&self, coarse: &Mesh, v: &[f64]) -> Vec<f64> {
        let d = coarse.dim();
        let mut starts = Vec::with_capacity(coarse.blocks.len());
        let mut acc = 0;
        for b in &coarse.blocks {
            starts.push(acc);
            acc += b.n_elems();
        }
        let mut out = vec![0.0; self.mesh.nodes.len() * d];
        for (i, &(g, xi)) in self.origin.iter().enumerate() {
            let bi = starts.partition_point(|&s| s <= g) - 1;
            let blk = &coarse.blocks[bi];
            let conn = blk.elem(g - starts[bi]);
            let (n, _) = blk.kind.shape(xi);
            for (b, &nd) in conn.iter().enumerate() {
                for c in 0..d {
                    out[i * d + c] += n[b] * v[nd * d + c];
                }
            }
        }
        out
    }

    pub fn n_coarse_nodes(&self) -> usize {
        self.n_coarse_nodes
    }
}

/// Constraints and loads of the coarse problem on the refined mesh. A fine node is fixed in a component when it is a fixed
/// coarse node, or when it lies on a coarse element edge whose both ends are fixed in that component.
fn refined_problem(ctx: &EstimatorContext, rf: &Refined) -> Result<(Model, Loads, Dirichlet), MeshError> {
    let coarse = &ctx.model.mesh;
    let d = coarse.dim();
    let model = Model::new(rf.mesh.clone()).map_err(MeshError::Failed)?;
    let mut bc = model.dirichlet();
    let mut starts = Vec::new();
    let mut acc = 0;
    for b in &coarse.blocks {
        starts.push(acc);
        acc += b.n_elems();
    }
    for (i, &(g, xi)) in rf.origin.iter().enumerate() {
        if i < rf.n_coarse_nodes {
            for c in 0..d {
                if ctx.bc.fixed[i * d + c] {
                    bc.fix(i, c, ctx.bc.value[i * d + c]);
                }
            }
            continue;
        }
        let bi = starts.partition_point(|&s| s <= g) - 1;
        let blk = &coarse.blocks[bi];
        let conn = blk.elem(g - starts[bi]);
        let nc = blk.kind.n_corners();
        let nat = blk.kind.node_coords();
        for k in 0..nc {
            let (a, b) = (nat[k], nat[(k + 1) % nc]);
            // on the segment a-b?
            let (ab, ax) = ([b[0] - a[0], b[1] - a[1]], [xi[0] - a[0], xi[1] - a[1]]);
            let cross = ab[0] * ax[1] - ab[1] * ax[0];
            let dotp = ab[0] * ax[0] + ab[1] * ax[1];
            let l2 = ab[0] * ab[0] + ab[1] * ab[1];
            if cross.abs() < 1e-9 && dotp > -1e-9 && dotp < l2 + 1e-9 {
                for c in 0..d {
                    if ctx.bc.fixed[conn[k] * d + c] && ctx.bc.fixed[conn[(k + 1) % nc] * d + c] {
                        // the prescribed value along the edge is the coarse interpolation of the end values
                        let v = ctx.bc.value[conn[k] * d + c] + (ctx.bc.value[conn[(k + 1) % nc] * d + c] - ctx.bc.value[conn[k] * d + c]) * (dotp / l2).clamp(0.0, 1.0);
                        bc.fix(i, c, v);
                    }
                }
            }
        }
    }
    // loads: a face splits into two; the nodes are found by position on the coarse face curve
    let find = {
        let mut w = NodeWelder::new(1e-10 * rf.mesh.nodes.iter().fold(0.0f64, |m, x| m.max(x[0].abs()).max(x[1].abs())).max(1.0));
        let mut scratch: Vec<[f64; 3]> = Vec::new();
        for x in &rf.mesh.nodes {
            w.node(&mut scratch, *x);
        }
        move |x: [f64; 3]| -> Option<usize> { w.find(&scratch, x) }
    };
    let face_halves = |face: &[usize]| -> Result<[Vec<usize>; 2], MeshError> {
        let pos = |s: f64| -> [f64; 3] {
            if face.len() == 2 {
                let (a, b) = (coarse.nodes[face[0]], coarse.nodes[face[1]]);
                [a[0] + s * (b[0] - a[0]), a[1] + s * (b[1] - a[1]), 0.0]
            } else {
                // quadratic edge through the end nodes and the midside node (VTK order: ends, then the middle)
                let (a, b, m) = (coarse.nodes[face[0]], coarse.nodes[face[1]], coarse.nodes[face[2]]);
                let t = 2.0 * s - 1.0;
                let (na, nb, nm) = (0.5 * t * (t - 1.0), 0.5 * t * (t + 1.0), 1.0 - t * t);
                [na * a[0] + nb * b[0] + nm * m[0], na * a[1] + nb * b[1] + nm * m[1], 0.0]
            }
        };
        let id = |s: f64| find(pos(s)).ok_or_else(|| MeshError::Failed("a loaded face has no counterpart on the refined mesh".into()));
        if face.len() == 2 {
            let m = id(0.5)?;
            Ok([vec![face[0], m], vec![m, face[1]]])
        } else {
            let (q1, m, q2) = (id(0.25)?, id(0.5)?, id(0.75)?);
            Ok([vec![face[0], m, q1], vec![m, face[1], q2]])
        }
    };
    let mut fine_loads = Loads { body: ctx.loads.body, block_body: ctx.loads.block_body.clone(), delta_t: ctx.loads.delta_t, nodal: ctx.loads.nodal.clone(), ..Default::default() };
    for (face, load) in &ctx.loads.faces {
        for half in face_halves(face)? {
            fine_loads.faces.push((half, *load));
        }
    }
    for (face, p) in &ctx.loads.followers {
        for half in face_halves(face)? {
            fine_loads.followers.push((half, *p));
        }
    }
    if !ctx.loads.field_faces.is_empty() {
        return Err(MeshError::Unsupported("position-dependent boundary force fields".into()));
    }
    Ok((model, fine_loads, bc))
}

/// Rebuilds a goal on another mesh (the builder of a [`super::controller::GoalSpec`]).
pub type Builder<'a> = &'a (dyn Fn(&Model, &Loads) -> Result<LinearFunctional, MeshError> + Send + Sync);

/// Goal-oriented estimator with an h-enriched dual. `goals` are builders: the goal is rebuilt on the fine mesh.
pub struct EnrichedDwrEstimator;

impl EnrichedDwrEstimator {
    /// Estimate the error of each goal; `coarse_goals[k]` is the goal on the coarse mesh and `builders[k]` rebuilds it.
    pub fn estimate_goals(&self, ctx: &EstimatorContext, coarse_goals: &[&LinearFunctional], builders: &[Builder]) -> Result<Vec<DwrResult>, MeshError> {
        let t0 = Instant::now();
        let name = |g: &LinearFunctional| format!("edwr:{}", g.name);
        let plastic = ctx.model.mesh.blocks.iter().any(|b| b.plasticity.is_some());
        let unsupported = |why: &str| -> Vec<DwrResult> {
            coarse_goals.iter().map(|g| DwrResult { goal: g.name.clone(), value: g.value(ctx.u), estimated_error: 0.0, corrected: g.value(ctx.u), field: ErrorField::unsupported(&name(g), why, ctx.n_elems()) }).collect()
        };
        if plastic {
            return Ok(unsupported("plastic blocks need the nonlinear adjoint"));
        }
        if ctx.loads.temperature.is_some() {
            return Ok(unsupported("nodal temperature fields"));
        }
        let rf = match refine_uniform_2d(&ctx.model.mesh) {
            Ok(r) => r,
            Err(MeshError::Unsupported(why)) => return Ok(unsupported(&why)),
            Err(e) => return Err(e),
        };
        let (fmodel, floads, fbc) = match refined_problem(ctx, &rf) {
            Ok(p) => p,
            Err(MeshError::Unsupported(why)) => return Ok(unsupported(&why)),
            Err(e) => return Err(e),
        };
        let d = ctx.model.mesh.dim();
        // fine system, one factorisation for the residual and every dual
        fmodel.check_constrained(&fbc).map_err(MeshError::Failed)?;
        let k = fmodel.assemble().map_err(MeshError::Failed)?;
        let red = Reduced::with_ordering(&fmodel.pattern, &fbc, Some(&fmodel.mesh.nodes), Ordering::Auto).map_err(|e| MeshError::Failed(e.to_string()))?;
        let fac = red.factor(&k).map_err(|e| MeshError::Failed(e.to_string()))?;
        let free: Vec<usize> = red.free_dofs().iter().map(|&x| x as usize).collect();
        let f = loads::assemble(&fmodel.mesh, &floads).map_err(MeshError::Failed)?;
        // the coarse solution on the fine mesh, with the prescribed values on the constrained dofs
        let u_fine = rf.prolong(&ctx.model.mesh, ctx.u);
        let mut ku = vec![0.0; u_fine.len()];
        k.matvec_add(&fmodel.pattern, &u_fine, &mut ku);
        let resid: Vec<f64> = (0..f.len()).map(|i| f[i] - ku[i]).collect();
        let mut results = Vec::new();
        for (idx, goal) in coarse_goals.iter().enumerate() {
            let gf = builders[idx](&fmodel, &floads)?;
            let mut rhs: Vec<f64> = free.iter().map(|&i| gf.gradient[i]).collect();
            fac.solve_reduced(&mut rhs);
            let mut zf = vec![0.0; u_fine.len()];
            for (&i, v) in free.iter().zip(&rhs) {
                zf[i] = *v;
            }
            // the coarse dual, prolonged
            let coarse_dual = super::dwr::solve_duals(ctx, &[goal])?.remove(0);
            let pz = rf.prolong(&ctx.model.mesh, &coarse_dual.z);
            // nodal products, shared among the coarse elements that contain the fine node
            let mut eta = vec![0.0; ctx.n_elems()];
            let nfine = fmodel.mesh.nodes.len();
            for i in 0..nfine {
                let mut p = 0.0;
                for c in 0..d {
                    let dof = i * d + c;
                    if !fbc.fixed[dof] {
                        p += (zf[dof] - pz[dof]) * resid[dof];
                    }
                }
                let owners = &rf.parents_of_node[i];
                for &g in owners {
                    eta[g] += p / owners.len() as f64;
                }
            }
            let estimated: f64 = eta.iter().sum();
            let value = goal.value(ctx.u);
            let mut reliability = Reliability::Reliable;
            if coarse_dual.residual > 1e-6 {
                reliability = Reliability::Failed(format!("the coarse dual solve left a relative residual of {:.2e}", coarse_dual.residual));
            } else if !goal.regularised {
                reliability = Reliability::Degraded("point functional: its dual solution is singular at the point".into());
            }
            let mut field = ErrorField::new(&name(goal), Indicator::Signed, eta, estimated, reliability, t0.elapsed().as_secs_f64());
            field.notes.push(format!("fine mesh: {} nodes, {} elements", nfine, fmodel.mesh.n_elems()));
            results.push(DwrResult { goal: goal.name.clone(), value, estimated_error: estimated, corrected: value + estimated, field });
        }
        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generate::grid;
    use crate::mesh::{Elastic, Physics};

    #[test]
    fn refinement_quadruples_the_elements_keeps_curved_edges_and_conforms() {
        for kind in [ElementKind::Tri3, ElementKind::Tri6, ElementKind::Quad4, ElementKind::Quad8, ElementKind::Quad9] {
            // a quarter annulus: curved sides
            let m = grid(Physics::PlaneStress { thickness: 1.0 }, kind, Elastic::new(1.0, 0.3), [3, 2, 1], &|p| {
                let (r, th) = (1.0 + p[0], p[1] * std::f64::consts::FRAC_PI_2);
                [r * th.cos(), r * th.sin(), 0.0]
            })
            .unwrap();
            let rf = refine_uniform_2d(&m).unwrap();
            assert_eq!(rf.mesh.n_elems(), 4 * m.n_elems(), "{kind:?}");
            let c = rf.mesh.conformity();
            assert!(c.is_conforming() && c.components == 1, "{kind:?}: {}", c.summary());
            assert_eq!(rf.mesh.quality().inverted, 0, "{kind:?}");
            // the coarse nodes keep their positions and indices
            for (i, x) in m.nodes.iter().enumerate() {
                assert_eq!(rf.mesh.nodes[i], *x);
            }
            // total area is preserved (children tile the parent)
            let area = |mesh: &Mesh| -> f64 {
                let mut work = crate::kernel::Work::new();
                let mut a = 0.0;
                for blk in &mesh.blocks {
                    let t = blk.kind.table();
                    for e in 0..blk.n_elems() {
                        let xyz: Vec<[f64; 3]> = blk.elem(e).iter().map(|&n| mesh.nodes[n]).collect();
                        crate::kernel::geometry(blk.kind, mesh.physics, &xyz, &mut work).unwrap();
                        a += (0..t.ngp).map(|g| work.wdet[g]).sum::<f64>();
                    }
                }
                a
            };
            assert!((area(&m) - area(&rf.mesh)).abs() < 1e-12 * area(&m), "{kind:?}");
        }
    }

    #[test]
    fn prolongation_is_exact_for_the_coarse_space() {
        for kind in [ElementKind::Tri6, ElementKind::Quad8, ElementKind::Quad9, ElementKind::Tri3] {
            let m = grid(Physics::PlaneStress { thickness: 1.0 }, kind, Elastic::new(1.0, 0.3), [3, 3, 1], &|p| [2.0 * p[0] + 0.1 * p[1], p[1], 0.0]).unwrap();
            let rf = refine_uniform_2d(&m).unwrap();
            // a field the coarse space reproduces: quadratic for quadratic elements, affine otherwise
            let f = |x: &[f64; 3]| -> [f64; 2] {
                if matches!(kind, ElementKind::Tri6 | ElementKind::Quad8 | ElementKind::Quad9) {
                    [1.0 + x[0] * x[0] - 0.5 * x[0] * x[1], x[1] * x[1] + x[0]]
                } else {
                    [1.0 + 2.0 * x[0] - x[1], 0.3 * x[0] + 0.7 * x[1]]
                }
            };
            let v: Vec<f64> = m.nodes.iter().flat_map(&f).collect();
            let fine = rf.prolong(&m, &v);
            for (i, x) in rf.mesh.nodes.iter().enumerate() {
                let w = f(x);
                // Quad8 reproduces only the serendipity space: x^2 y terms are absent, ours has x y which is in it
                assert!((fine[2 * i] - w[0]).abs() < 1e-10 && (fine[2 * i + 1] - w[1]).abs() < 1e-10, "{kind:?} node {i}: {:?} vs {w:?}", [fine[2 * i], fine[2 * i + 1]]);
            }
        }
    }
}
