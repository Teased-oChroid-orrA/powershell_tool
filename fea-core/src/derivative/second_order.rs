//! Second derivatives of linear static responses (Story I): Hessian-vector products and Hessians of `J(p) = g . u(p)`,
//! `K(p) u = f`, with the second derivatives of the element stiffness taken exactly by nesting dual numbers.
//!
//! Two organisations of the same second derivative, which the tests compare:
//!
//! * **forward over reverse** ([`SecondOrderEngine::hessian_vector`]): the adjoint gradient `G_i = lambda . (f_i - K_i u)`,
//!   `K lambda = g`, differentiated along a direction `v` in parameter space. One extra tangent solve for `du_v`, one for
//!   `dlambda_v`, independent of the number of parameters: the right tool when the Hessian acts on a few vectors (Newton-CG,
//!   Lanczos, Gauss-Newton checks).
//! * **reverse over forward** ([`SecondOrderEngine::hessian_reverse_over_forward`]): the direct sensitivities `du_j`, then
//!   `H_ij = lambda . (-K_ij u - K_i du_j - K_j du_i)` for the full matrix; `n` solves, one adjoint.
//!
//! The load vector must not depend on the parameters (nodal forces): a surface traction moves with the geometry through a central
//! difference in `sensitivity.rs`, and differentiating a difference twice is not an exact second derivative.

use super::dual::Dual;
use super::element;
use super::linalg::TangentSolver;
use super::linear_sens::{LinearParam, LinearSensitivityEngine};
use super::scalar::Scalar;
use crate::analysis::Model;
use crate::linear::{Dirichlet, Ordering, Reduced};
use crate::loads::{self, Loads};
use crate::mesh::Physics;

type D1 = Dual<f64, 1>;
type D2 = Dual<D1, 1>;

pub struct SecondOrderEngine<'m> {
    model: &'m Model,
    loads: &'m Loads,
    bc: &'m Dirichlet,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SecondOrderReport {
    pub tangent_solves: usize,
}

impl<'m> SecondOrderEngine<'m> {
    pub fn new(model: &'m Model, loads: &'m Loads, bc: &'m Dirichlet) -> Self {
        Self { model, loads, bc }
    }

    fn thickness(&self) -> f64 {
        match self.model.mesh.physics {
            Physics::PlaneStress { thickness } | Physics::PlaneStrain { thickness } => thickness,
            _ => 1.0,
        }
    }

    fn check_loads(&self) -> Result<(), String> {
        let nodal_only = Loads { nodal: self.loads.nodal.clone(), ..Loads::default() };
        let (a, b) = (loads::assemble(&self.model.mesh, self.loads)?, loads::assemble(&self.model.mesh, &nodal_only)?);
        if a != b {
            return Err("exact second derivatives need nodal loads only (surface, body and thermal loads depend on the parameters)".into());
        }
        Ok(())
    }

    /// `(d^2 K / dp_i dp_j) x` over all dofs: the outer dual layer carries `p_i`, the inner one `p_j`.
    pub fn d2k_x(&self, pi: LinearParam, pj: LinearParam, x: &[f64]) -> Result<Vec<f64>, String> {
        let m = self.model;
        let d = m.mesh.dim();
        let mut out = vec![0.0; m.mesh.n_dofs()];
        for (bi, blk) in m.mesh.blocks.iter().enumerate() {
            let acts = |p: LinearParam| match p {
                LinearParam::ModulusScale(b) | LinearParam::PoissonRatio(b) => b == bi,
                _ => true,
            };
            if !acts(pi) || !acts(pj) {
                continue;
            }
            if blk.material.aniso.is_some() {
                return Err(format!("block {bi}: anisotropic elasticity is not supported by the exact element derivatives"));
            }
            let nn = blk.kind.n_nodes();
            let nd = nn * d;
            'elements: for e in 0..blk.n_elems() {
                let conn = blk.elem(e);
                let mut coords: Vec<[D2; 3]> = conn.iter().map(|&n| m.mesh.nodes[n].map(|c| D2::cst(c))).collect();
                let (mut ev, mut nuv, mut tv) = (D2::cst(blk.material.e), D2::cst(blk.material.nu), D2::cst(self.thickness()));
                // a quantity that both layers vary needs both seeds: value = inner var, outer derivative = 1
                let seed = |p: LinearParam, outer: bool, ev: &mut D2, nuv: &mut D2, tv: &mut D2, coords: &mut Vec<[D2; 3]>| -> bool {
                    let mk = |x0: f64, cur: D2| -> D2 {
                        if outer {
                            D2::with(cur.v, [D1::cst(1.0)])
                        } else {
                            D2::with(D1::var(x0, 0), cur.d)
                        }
                    };
                    match p {
                        LinearParam::ModulusScale(_) => *ev = mk_scale(blk.material.e, *ev, outer),
                        LinearParam::PoissonRatio(_) => *nuv = mk(blk.material.nu, *nuv),
                        LinearParam::Thickness => *tv = mk(self.thickness(), *tv),
                        LinearParam::Coord { node, axis } => match conn.iter().position(|&n| n == node) {
                            Some(l) => coords[l][axis] = mk(m.mesh.nodes[node][axis], coords[l][axis]),
                            None => return false,
                        },
                    }
                    true
                };
                if !seed(pj, false, &mut ev, &mut nuv, &mut tv, &mut coords) {
                    continue 'elements;
                }
                if !seed(pi, true, &mut ev, &mut nuv, &mut tv, &mut coords) {
                    continue 'elements;
                }
                let ke = element::stiffness(blk.kind, m.mesh.physics, &coords, ev, nuv, tv).map_err(|er| er.to_string())?;
                for a in 0..nn {
                    for i in 0..d {
                        let mut s = 0.0;
                        for b in 0..nn {
                            for j in 0..d {
                                s += ke[(a * d + i) * nd + b * d + j].d[0].d[0] * x[conn[b] * d + j];
                            }
                        }
                        out[conn[a] * d + i] += s;
                    }
                }
            }
        }
        Ok(out)
    }

    fn setup<'a>(&self, red: &'a Reduced, k: &'a crate::assembly::BlockMatrix, free: &'a [usize]) -> Result<TangentSolver<'a>, String>
    where
        'm: 'a,
    {
        TangentSolver::new(red, &self.model.pattern, k, self.model.mesh.n_dofs(), free, &[]).map_err(|e| e.to_string())
    }

    fn prepare(&self) -> Result<(crate::assembly::BlockMatrix, Reduced, Vec<usize>, Vec<f64>), String> {
        self.check_loads()?;
        self.model.check_constrained(self.bc)?;
        let k = self.model.assemble()?;
        let red = Reduced::with_ordering(&self.model.pattern, self.bc, Some(&self.model.mesh.nodes), Ordering::Auto).map_err(|e| e.to_string())?;
        let free: Vec<usize> = red.free_dofs().iter().map(|&d| d as usize).collect();
        let f = loads::assemble(&self.model.mesh, self.loads)?;
        Ok((k, red, free, f))
    }

    fn scatter(&self, free: &[usize], x: &[f64]) -> Vec<f64> {
        let mut out = vec![0.0; self.model.mesh.n_dofs()];
        for (&d, v) in free.iter().zip(x) {
            out[d] = *v;
        }
        out
    }

    /// `H v` for `J = g . u`: forward over reverse.
    pub fn hessian_vector(&self, params: &[LinearParam], v: &[f64], g: &[f64]) -> Result<(Vec<f64>, SecondOrderReport), String> {
        if v.len() != params.len() {
            return Err("the direction needs one weight per parameter".into());
        }
        let (k, red, free, f) = self.prepare()?;
        let solver = self.setup(&red, &k, &free)?;
        let lin = LinearSensitivityEngine::new(self.model, self.loads, self.bc);
        let restrict = |x: &[f64]| -> Vec<f64> { free.iter().map(|&d| x[d]).collect() };
        let n = self.model.mesh.n_dofs();
        let mut u = vec![0.0; n];
        red.scatter(&solver.solve(&red.reduced_rhs(&k, &f, self.bc).0)?, &mut u);
        let lambda = self.scatter(&free, &solver.solve(&restrict(g))?);
        // K_v u and K_v lambda with K_v = sum_j v_j dK/dp_j
        let sum = |x: &[f64]| -> Result<Vec<f64>, String> {
            let mut out = vec![0.0; n];
            for (&p, &w) in params.iter().zip(v) {
                if w != 0.0 {
                    for (o, a) in out.iter_mut().zip(lin.dk_u(p, x)?) {
                        *o += w * a;
                    }
                }
            }
            Ok(out)
        };
        let kvu = sum(&u)?;
        let kvl = sum(&lambda)?;
        let du_v = self.scatter(&free, &solver.solve(&restrict(&kvu.iter().map(|a| -a).collect::<Vec<_>>()))?);
        let dl_v = self.scatter(&free, &solver.solve(&restrict(&kvl.iter().map(|a| -a).collect::<Vec<_>>()))?);
        let mut hv = vec![0.0; params.len()];
        for (i, &pi) in params.iter().enumerate() {
            // dG_i/dv = dl_v . (-K_i u) + lambda . (-K_iv u - K_i du_v)
            let kiu = lin.dk_u(pi, &u)?;
            let kidu = lin.dk_u(pi, &du_v)?;
            let mut kiv_u = vec![0.0; n];
            for (&pj, &w) in params.iter().zip(v) {
                if w != 0.0 {
                    for (o, a) in kiv_u.iter_mut().zip(self.d2k_x(pi, pj, &u)?) {
                        *o += w * a;
                    }
                }
            }
            hv[i] = (0..n).map(|d| -dl_v[d] * kiu[d] - lambda[d] * (kiv_u[d] + kidu[d])).sum();
        }
        Ok((hv, SecondOrderReport { tangent_solves: solver.n_solves() }))
    }

    /// The full Hessian by `n` Hessian-vector products (row-major `n x n`).
    pub fn hessian_forward_over_reverse(&self, params: &[LinearParam], g: &[f64]) -> Result<(Vec<f64>, SecondOrderReport), String> {
        let n = params.len();
        let mut h = vec![0.0; n * n];
        let mut solves = 0;
        for j in 0..n {
            let mut e = vec![0.0; n];
            e[j] = 1.0;
            let (col, rep) = self.hessian_vector(params, &e, g)?;
            solves += rep.tangent_solves;
            for i in 0..n {
                h[i * n + j] = col[i];
            }
        }
        Ok((h, SecondOrderReport { tangent_solves: solves }))
    }

    /// The full Hessian from the direct first sensitivities: reverse over forward.
    pub fn hessian_reverse_over_forward(&self, params: &[LinearParam], g: &[f64]) -> Result<(Vec<f64>, SecondOrderReport), String> {
        let (k, red, free, f) = self.prepare()?;
        let solver = self.setup(&red, &k, &free)?;
        let lin = LinearSensitivityEngine::new(self.model, self.loads, self.bc);
        let n_dof = self.model.mesh.n_dofs();
        let restrict = |x: &[f64]| -> Vec<f64> { free.iter().map(|&d| x[d]).collect() };
        let mut u = vec![0.0; n_dof];
        red.scatter(&solver.solve(&red.reduced_rhs(&k, &f, self.bc).0)?, &mut u);
        let lambda = self.scatter(&free, &solver.solve(&restrict(g))?);
        let n = params.len();
        let mut du = Vec::with_capacity(n);
        for &p in params {
            let rhs: Vec<f64> = lin.dk_u(p, &u)?.iter().map(|a| -a).collect();
            du.push(self.scatter(&free, &solver.solve(&restrict(&rhs))?));
        }
        let mut h = vec![0.0; n * n];
        for i in 0..n {
            for j in 0..n {
                let kij = self.d2k_x(params[i], params[j], &u)?;
                let ki_duj = lin.dk_u(params[i], &du[j])?;
                let kj_dui = lin.dk_u(params[j], &du[i])?;
                h[i * n + j] = -(0..n_dof).map(|d| lambda[d] * (kij[d] + ki_duj[d] + kj_dui[d])).sum::<f64>();
            }
        }
        Ok((h, SecondOrderReport { tangent_solves: solver.n_solves() }))
    }
}

/// Seed for a multiplicative scale at 1 on a quantity of value `x0`: derivative `x0` per unit of scale.
fn mk_scale(x0: f64, cur: D2, outer: bool) -> D2 {
    // value x0 (1 + s): first derivative x0, second derivative 0 in the same scale
    if outer {
        D2::with(cur.v, [D1::cst(x0)])
    } else {
        D2::with(D1::with(x0, [x0]), cur.d)
    }
}
