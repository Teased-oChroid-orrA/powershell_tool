//! Quantities of interest. A goal functional `J(u) = g . u + c` is linear in the displacements (every initial QoI of the epic
//! is: a displacement, a difference of displacements, a region average of a displacement or a stress, a reaction, the
//! compliance); the discrete adjoint problem is then `K z = g` on the free dofs with the same factorisation as the primal.
//!
//! Point functionals (a displacement at one node) have a dual solution that is singular at the point, so a recovery-based
//! dual estimate is unreliable there: they are flagged `regularised == false` and the DWR estimator reports `Degraded`. Use a
//! region average for a trustworthy estimate. Unregularised peak stresses are deliberately not offered: they do not admit a
//! reliable goal-oriented estimate.

use crate::analysis::Model;
use crate::kernel::{self, Work};
use crate::loads::{self, Loads};
use crate::meshing::MeshError;

/// A linear functional `J(u) = gradient . u + offset`.
#[derive(Debug, Clone)]
pub struct LinearFunctional {
    pub name: String,
    /// `dJ/du` over all dofs.
    pub gradient: Vec<f64>,
    pub offset: f64,
    /// False for a point functional (singular dual).
    pub regularised: bool,
}

impl LinearFunctional {
    /// `J(u)`.
    pub fn value(&self, u: &[f64]) -> f64 {
        self.gradient.iter().zip(u).map(|(g, v)| g * v).sum::<f64>() + self.offset
    }

    fn check_node(model: &Model, node: usize, comp: usize) -> Result<(), MeshError> {
        if node >= model.mesh.nodes.len() || comp >= model.mesh.dim() {
            return Err(MeshError::Failed(format!("node {node} component {comp} is outside the mesh")));
        }
        Ok(())
    }

    /// One displacement component of one node (a point functional).
    pub fn nodal_displacement(model: &Model, node: usize, comp: usize) -> Result<Self, MeshError> {
        Self::check_node(model, node, comp)?;
        let d = model.mesh.dim();
        let mut g = vec![0.0; model.mesh.n_dofs()];
        g[node * d + comp] = 1.0;
        Ok(Self { name: format!("u[{node}].{comp}"), gradient: g, offset: 0.0, regularised: false })
    }

    /// Difference of one component between two nodes (a relative displacement; a point functional).
    pub fn relative_displacement(model: &Model, a: usize, b: usize, comp: usize) -> Result<Self, MeshError> {
        Self::check_node(model, a, comp)?;
        Self::check_node(model, b, comp)?;
        let d = model.mesh.dim();
        let mut g = vec![0.0; model.mesh.n_dofs()];
        g[a * d + comp] += 1.0;
        g[b * d + comp] -= 1.0;
        Ok(Self { name: format!("u[{a}].{comp}-u[{b}].{comp}"), gradient: g, offset: 0.0, regularised: false })
    }

    /// Mean of one displacement component over a node set (regularised when the set has several nodes).
    pub fn mean_displacement(model: &Model, nodes: &[usize], comp: usize) -> Result<Self, MeshError> {
        if nodes.is_empty() {
            return Err(MeshError::Failed("an empty node set".into()));
        }
        let d = model.mesh.dim();
        let mut g = vec![0.0; model.mesh.n_dofs()];
        for &n in nodes {
            Self::check_node(model, n, comp)?;
            g[n * d + comp] += 1.0 / nodes.len() as f64;
        }
        Ok(Self { name: format!("mean u.{comp} over {} nodes", nodes.len()), gradient: g, offset: 0.0, regularised: nodes.len() > 1 })
    }

    /// Compliance `f . u` (twice the strain energy for loads independent of the displacements).
    pub fn compliance(model: &Model, loads: &Loads) -> Result<Self, MeshError> {
        let f = loads::assemble(&model.mesh, loads).map_err(MeshError::Failed)?;
        Ok(Self { name: "compliance".into(), gradient: f, offset: 0.0, regularised: true })
    }

    /// Volume average of stress component `comp` (library order xx, yy, zz, xy, yz, zx) over the elements whose centroid
    /// satisfies `inside`. Thermal strain is not part of the functional (it is the same constant for every mesh).
    pub fn average_stress(model: &Model, comp: usize, inside: &dyn Fn(&[f64; 3]) -> bool) -> Result<Self, MeshError> {
        if comp >= 6 {
            return Err(MeshError::Failed("stress component must be 0..6".into()));
        }
        let d = model.mesh.dim();
        let mut g = vec![0.0; model.mesh.n_dofs()];
        let mut work = Work::new();
        let mut volume = 0.0;
        let mut n_sel = 0usize;
        for blk in &model.mesh.blocks {
            let t = blk.kind.table();
            let (nn, ngp) = (t.nn, t.ngp);
            let mut ue = vec![0.0; nn * d];
            for e in 0..blk.n_elems() {
                let conn = blk.elem(e);
                let xyz: Vec<[f64; 3]> = conn.iter().map(|&n| model.mesh.nodes[n]).collect();
                let centroid: [f64; 3] = std::array::from_fn(|k| xyz.iter().map(|x| x[k]).sum::<f64>() / nn as f64);
                if !inside(&centroid) {
                    continue;
                }
                n_sel += 1;
                kernel::geometry(blk.kind, model.mesh.physics, &xyz, &mut work).map_err(|er| MeshError::Failed(er.to_string()))?;
                for gp in 0..ngp {
                    volume += work.wdet[gp];
                    for a in 0..nn {
                        for i in 0..d {
                            ue.iter_mut().for_each(|v| *v = 0.0);
                            ue[a * d + i] = 1.0;
                            let strain = kernel::strain_at(blk.kind, model.mesh.physics, &work, gp, &ue);
                            let s = kernel::stress_from_strain(model.mesh.physics, &blk.material, strain, 0.0);
                            g[conn[a] * d + i] += work.wdet[gp] * s[comp];
                        }
                    }
                }
            }
        }
        if n_sel == 0 || volume <= 0.0 {
            return Err(MeshError::Failed("the averaging region contains no element".into()));
        }
        g.iter_mut().for_each(|v| *v /= volume);
        Ok(Self { name: format!("mean stress[{comp}] over {n_sel} elements"), gradient: g, offset: 0.0, regularised: n_sel > 1 })
    }

    /// `int w sigma[comp] dV / int w dV` with a smooth non-negative weight `w` (a bump around the point of interest). Unlike
    /// the element-selecting [`average_stress`](Self::average_stress), its limit under refinement does not depend on which
    /// elements the region boundary happens to cut, so a convergence study of it is meaningful.
    pub fn weighted_stress(model: &Model, comp: usize, w: &dyn Fn(&[f64; 3]) -> f64) -> Result<Self, MeshError> {
        if comp >= 6 {
            return Err(MeshError::Failed("stress component must be 0..6".into()));
        }
        let d = model.mesh.dim();
        let mut g = vec![0.0; model.mesh.n_dofs()];
        let mut work = Work::new();
        let mut wsum = 0.0;
        for blk in &model.mesh.blocks {
            let t = blk.kind.table();
            let (nn, ngp) = (t.nn, t.ngp);
            let mut ue = vec![0.0; nn * d];
            for e in 0..blk.n_elems() {
                let conn = blk.elem(e);
                let xyz: Vec<[f64; 3]> = conn.iter().map(|&n| model.mesh.nodes[n]).collect();
                kernel::geometry(blk.kind, model.mesh.physics, &xyz, &mut work).map_err(|er| MeshError::Failed(er.to_string()))?;
                for gp in 0..ngp {
                    let sh = t.n_at(gp);
                    let pos: [f64; 3] = std::array::from_fn(|k| (0..nn).map(|a| sh[a] * xyz[a][k]).sum());
                    let wt = w(&pos);
                    if !(wt.is_finite() && wt >= 0.0) {
                        return Err(MeshError::Failed(format!("the weight is {wt} at ({}, {}, {})", pos[0], pos[1], pos[2])));
                    }
                    if wt == 0.0 {
                        continue;
                    }
                    wsum += work.wdet[gp] * wt;
                    for a in 0..nn {
                        for i in 0..d {
                            ue.iter_mut().for_each(|v| *v = 0.0);
                            ue[a * d + i] = 1.0;
                            let strain = kernel::strain_at(blk.kind, model.mesh.physics, &work, gp, &ue);
                            let s = kernel::stress_from_strain(model.mesh.physics, &blk.material, strain, 0.0);
                            g[conn[a] * d + i] += work.wdet[gp] * wt * s[comp];
                        }
                    }
                }
            }
        }
        if wsum <= 0.0 {
            return Err(MeshError::Failed("the weight vanishes on the whole mesh".into()));
        }
        g.iter_mut().for_each(|v| *v /= wsum);
        Ok(Self { name: format!("weighted stress[{comp}]"), gradient: g, offset: 0.0, regularised: true })
    }

    /// Sum of the support reactions of `comp` over the dofs `nodes` (a reaction force). `J(u) = sum_i (K u - f)_i`, so
    /// `dJ/du = K e` and `offset = -sum f_i`.
    pub fn reaction(model: &Model, loads: &Loads, nodes: &[usize], comp: usize) -> Result<Self, MeshError> {
        if nodes.is_empty() {
            return Err(MeshError::Failed("an empty node set".into()));
        }
        let d = model.mesh.dim();
        for &n in nodes {
            Self::check_node(model, n, comp)?;
        }
        let k = model.assemble().map_err(MeshError::Failed)?;
        let mut e = vec![0.0; model.mesh.n_dofs()];
        for &n in nodes {
            e[n * d + comp] = 1.0;
        }
        let mut g = vec![0.0; e.len()];
        k.matvec_add(&model.pattern, &e, &mut g);
        let f = loads::assemble(&model.mesh, loads).map_err(MeshError::Failed)?;
        let offset = -nodes.iter().map(|&n| f[n * d + comp]).sum::<f64>();
        Ok(Self { name: format!("reaction.{comp} over {} nodes", nodes.len()), gradient: g, offset, regularised: nodes.len() > 1 })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adaptivity::estimator::tests::solved_strip;

    #[test]
    fn nodal_functionals_read_the_solution() {
        let (model, _loads, _bc, u) = solved_strip(2);
        let n = model.mesh.nodes.len() - 1;
        let j = LinearFunctional::nodal_displacement(&model, n, 1).unwrap();
        assert_eq!(j.value(&u), u[2 * n + 1]);
        assert!(!j.regularised);
        let r = LinearFunctional::relative_displacement(&model, n, 0, 1).unwrap();
        assert_eq!(r.value(&u), u[2 * n + 1] - u[1]);
        assert!(LinearFunctional::nodal_displacement(&model, n + 5, 0).is_err());
        assert!(LinearFunctional::mean_displacement(&model, &[], 0).is_err());
    }

    #[test]
    fn compliance_is_twice_the_strain_energy() {
        let (model, loads, _bc, u) = solved_strip(2);
        let j = LinearFunctional::compliance(&model, &loads).unwrap();
        let energy = model.strain_energy(&u).unwrap();
        assert!((j.value(&u) - 2.0 * energy).abs() < 1e-9 * j.value(&u).abs(), "{} {}", j.value(&u), 2.0 * energy);
        assert!(j.regularised);
    }

    #[test]
    fn the_average_stress_functional_matches_a_direct_average_of_the_gauss_stresses() {
        let (model, _loads, _bc, u) = solved_strip(3);
        let inside = |c: &[f64; 3]| c[0] > 1.0 && c[0] < 3.0;
        let j = LinearFunctional::average_stress(&model, 0, &inside).unwrap();
        // direct: volume-weighted mean of sigma_xx over the same elements
        let gauss = model.gauss_stresses(&u, 0.0).unwrap();
        let mut work = Work::new();
        let (mut num, mut den) = (0.0, 0.0);
        for (bi, blk) in model.mesh.blocks.iter().enumerate() {
            let t = blk.kind.table();
            for e in 0..blk.n_elems() {
                let xyz: Vec<[f64; 3]> = blk.elem(e).iter().map(|&n| model.mesh.nodes[n]).collect();
                let c: [f64; 3] = std::array::from_fn(|k| xyz.iter().map(|x| x[k]).sum::<f64>() / xyz.len() as f64);
                if !inside(&c) {
                    continue;
                }
                kernel::geometry(blk.kind, model.mesh.physics, &xyz, &mut work).unwrap();
                for gp in 0..t.ngp {
                    num += work.wdet[gp] * gauss[bi][e * t.ngp + gp].1[0];
                    den += work.wdet[gp];
                }
            }
        }
        assert!((j.value(&u) - num / den).abs() < 1e-9 * (num / den).abs().max(1.0), "{} {}", j.value(&u), num / den);
        assert!(j.regularised);
        assert!(LinearFunctional::average_stress(&model, 0, &|_| false).is_err());
    }

    #[test]
    fn the_reaction_functional_equals_the_support_reaction() {
        let (model, loads, bc, u) = solved_strip(3);
        let sol = model.solve_static(&loads, &bc).unwrap();
        let nodes: Vec<usize> = model.mesh.node_set("u0").unwrap().to_vec();
        let j = LinearFunctional::reaction(&model, &loads, &nodes, 1).unwrap();
        let direct: f64 = nodes.iter().map(|&n| sol.reactions[2 * n + 1]).sum();
        assert!((j.value(&u) - direct).abs() < 1e-8 * direct.abs().max(1.0), "{} {direct}", j.value(&u));
        // the cantilever tip load of 100 per unit height over 1 unit: reaction balances it
        assert!((direct - 100.0).abs() < 1e-6, "{direct}");
    }
}
