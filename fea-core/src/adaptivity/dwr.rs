//! Dual-weighted residual (goal-oriented) error estimation for linear static problems.
//!
//! **Convention.** The primal problem is `a(u, v) = f(v)` for all admissible `v` (`K u = f` on the free dofs, constrained dofs
//! eliminated), the goal is the linear functional `J(u) = g . u + c`, and the *dual* (adjoint) problem is `a(v, z) = g . v`,
//! i.e. `K z = g` on the free dofs with `z = 0` on constrained dofs (`K` is symmetric, so the primal factorisation is reused
//! and the tangent-transpose is `K` itself). The error is signed as *exact minus discrete*:
//!
//! `J(u) - J(u_h) = a(u - u_h, z) = a(u - u_h, z - z_h)`   (Galerkin orthogonality).
//!
//! **Localisation.** With `a(v, w) = int sigma(v) : C^-1 : sigma(w)` the right-hand side is the element sum of
//! `eta_e = int_e (sigma*(u) - sigma_h(u)) : C^-1 : (sigma*(z) - sigma_h(z)) dV`, where `sigma*` is the SPR-recovered stress
//! standing in for the exact stress of the primal and of the dual problem. The signed `eta_e` sum to the estimated error of the
//! goal (`J(u) ~ J(u_h) + sum eta_e`); the non-negative marking indicator is `|eta_e|`. No enriched space is needed, so the
//! estimate covers every element family of the kernel and inherits improvements of the stress recovery. Its quality is the
//! quality of `sigma*`: it is trustworthy for regularised goals on smooth solutions and not for point goals (singular dual).
//!
//! Linear elastic blocks only: a plastic block, or a nodal temperature field, is reported `Unsupported`.

use super::estimator::{ErrorField, EstimatorContext, Indicator, Reliability};
use super::goal::LinearFunctional;
use crate::analysis::{GaussStress, Model};
use crate::kernel::{self, Work};
use crate::linear::{Ordering, Reduced};
use crate::meshing::MeshError;
use crate::recover::{complementary_energy, SprRecovery};
use rayon::prelude::*;
use std::time::Instant;

/// Elements per block from which the element loop runs in parallel (each element is independent; the sum is taken in element
/// order afterwards, so the result is identical for any thread count).
const PARALLEL_ELEMS: usize = 2000;

/// `a : C^-1 : b` for stress vectors (polarisation of the complementary energy `1/2 s : C^-1 : s`).
fn bilinear(mat: &crate::mesh::Elastic, a: &[f64; 6], b: &[f64; 6]) -> f64 {
    let s: [f64; 6] = std::array::from_fn(|k| a[k] + b[k]);
    complementary_energy(mat, &s) - complementary_energy(mat, a) - complementary_energy(mat, b)
}

/// Result of the dual solve of one goal.
pub struct Dual {
    /// The adjoint field over all dofs.
    pub z: Vec<f64>,
    /// `|K z - g| / |g|` over the free dofs.
    pub residual: f64,
}

/// Solve `K z = g` for several goals with one factorisation.
pub fn solve_duals(ctx: &EstimatorContext, goals: &[&LinearFunctional]) -> Result<Vec<Dual>, MeshError> {
    let model = ctx.model;
    for g in goals {
        if g.gradient.len() != model.mesh.n_dofs() {
            return Err(MeshError::Failed(format!("goal '{}' has {} gradient entries for {} dofs", g.name, g.gradient.len(), model.mesh.n_dofs())));
        }
    }
    model.check_constrained(ctx.bc).map_err(MeshError::Failed)?;
    let k = model.assemble().map_err(MeshError::Failed)?;
    let red = Reduced::with_ordering(&model.pattern, ctx.bc, Some(&model.mesh.nodes), Ordering::Auto).map_err(|e| MeshError::Failed(e.to_string()))?;
    let free: Vec<usize> = red.free_dofs().iter().map(|&d| d as usize).collect();
    // the shared tangent solver of the derivative engine: the adjoint equation `K^T z = g` is solved there for every consumer
    let solver = crate::derivative::linalg::TangentSolver::new(&red, &model.pattern, &k, model.mesh.n_dofs(), &free, &[]).map_err(|e| MeshError::Failed(e.to_string()))?;
    let mut out = Vec::new();
    for g in goals {
        let rhs: Vec<f64> = free.iter().map(|&d| g.gradient[d]).collect();
        let rhs = solver.solve_transpose(&rhs).map_err(MeshError::Failed)?;
        let mut z = vec![0.0; model.mesh.n_dofs()];
        for (&d, v) in free.iter().zip(&rhs) {
            z[d] = *v;
        }
        // residual of the dual on the free dofs
        let mut kz = vec![0.0; z.len()];
        k.matvec_add(&model.pattern, &z, &mut kz);
        let (mut num, mut den) = (0.0f64, 0.0f64);
        for &d in &free {
            num += (kz[d] - g.gradient[d]).powi(2);
            den += g.gradient[d].powi(2);
        }
        out.push(Dual { z, residual: if den > 0.0 { (num / den).sqrt() } else { 0.0 } });
    }
    Ok(out)
}

/// Per-element `int (sigma*(a) - sigma_h(a)) : C^-1 : (sigma*(b) - sigma_h(b))` in global element order.
fn local_products(model: &Model, ga: &[Vec<GaussStress>], ra: &SprRecovery, gb: &[Vec<GaussStress>], rb: &SprRecovery) -> Result<Vec<f64>, MeshError> {
    let mut out = Vec::with_capacity(model.mesh.n_elems());
    for (bi, blk) in model.mesh.blocks.iter().enumerate() {
        let t = blk.kind.table();
        let nn = t.nn;
        let one = |(work, xyz): &mut (Work, Vec<[f64; 3]>), e: usize| -> Result<f64, String> {
            let conn = blk.elem(e);
            for (a, &nd) in conn.iter().enumerate() {
                xyz[a] = model.mesh.nodes[nd];
            }
            kernel::geometry(blk.kind, model.mesh.physics, xyz, work).map_err(|er| er.to_string())?;
            let mut sum = 0.0;
            for g in 0..t.ngp {
                let nsh = t.n_at(g);
                let (mut sa, mut sb) = ([0.0; 6], [0.0; 6]);
                for (a, &nd) in conn.iter().enumerate() {
                    let (va, vb) = (ra.at(bi, nd), rb.at(bi, nd));
                    for c in 0..6 {
                        sa[c] += nsh[a] * va[c];
                        sb[c] += nsh[a] * vb[c];
                    }
                }
                let (ha, hb) = (ga[bi][e * t.ngp + g].1, gb[bi][e * t.ngp + g].1);
                let da: [f64; 6] = std::array::from_fn(|c| sa[c] - ha[c]);
                let db: [f64; 6] = std::array::from_fn(|c| sb[c] - hb[c]);
                sum += work.wdet[g] * bilinear(&blk.material, &da, &db);
            }
            Ok(sum)
        };
        let init = || (Work::new(), vec![[0.0; 3]; nn]);
        let per: Vec<f64> = if blk.n_elems() >= PARALLEL_ELEMS {
            (0..blk.n_elems()).into_par_iter().map_init(init, |st, e| one(st, e)).collect::<Result<_, _>>().map_err(MeshError::Failed)?
        } else {
            let mut st = init();
            (0..blk.n_elems()).map(|e| one(&mut st, e)).collect::<Result<_, _>>().map_err(MeshError::Failed)?
        };
        out.extend(per);
    }
    Ok(out)
}

/// Result for one goal.
#[derive(Debug, Clone)]
pub struct DwrResult {
    pub goal: String,
    /// `J(u_h)`.
    pub value: f64,
    /// Estimated `J(u) - J(u_h)` (sum of the signed contributions).
    pub estimated_error: f64,
    /// `J(u_h) + estimated_error`.
    pub corrected: f64,
    /// Signed element contributions (sum = `estimated_error`); `field.marking()` is the non-negative indicator.
    pub field: ErrorField,
}

/// Goal-oriented estimator for linear elasticity.
pub struct DwrEstimator;

impl DwrEstimator {
    /// Estimate the error of every goal (one factorisation, one primal recovery).
    pub fn estimate_goals(&self, ctx: &EstimatorContext, goals: &[&LinearFunctional]) -> Result<Vec<DwrResult>, MeshError> {
        let t0 = Instant::now();
        let model = ctx.model;
        let name = |g: &LinearFunctional| format!("dwr:{}", g.name);
        if model.mesh.blocks.iter().any(|b| b.plasticity.is_some()) {
            return Ok(goals.iter().map(|g| unsupported(g, &name(g), "a plastic block needs the tangent-transpose adjoint of the nonlinear problem", ctx.n_elems(), ctx.u)).collect());
        }
        if ctx.loads.temperature.is_some() {
            return Ok(goals.iter().map(|g| unsupported(g, &name(g), "nodal temperature fields are not supported by the dual-weighted estimate", ctx.n_elems(), ctx.u)).collect());
        }
        let duals = solve_duals(ctx, goals)?;
        let gauss_u = model.gauss_stresses(ctx.u, ctx.loads.delta_t).map_err(MeshError::Failed)?;
        let rec_u = model.recover_spr_by_material(&gauss_u).map_err(MeshError::Failed)?;
        let mut results = Vec::new();
        for (goal, dual) in goals.iter().zip(&duals) {
            let gauss_z = model.gauss_stresses(&dual.z, 0.0).map_err(MeshError::Failed)?;
            let rec_z = model.recover_spr_by_material(&gauss_z).map_err(MeshError::Failed)?;
            let eta = local_products(model, &gauss_u, &rec_u, &gauss_z, &rec_z)?;
            let estimated: f64 = eta.iter().sum();
            let value = goal.value(ctx.u);
            let mut reliability = Reliability::Reliable;
            let mut notes = vec![format!("dual residual {:.2e}", dual.residual)];
            if dual.residual > 1e-6 {
                reliability = Reliability::Failed(format!("the dual solve left a relative residual of {:.2e}", dual.residual));
            } else if !goal.regularised {
                reliability = Reliability::Degraded("point functional: its dual solution is singular at the point, the recovered estimate is unreliable".into());
            } else if rec_u.diagnostics.averaged_fraction() > 0.05 || rec_z.diagnostics.averaged_fraction() > 0.05 {
                reliability = Reliability::Degraded(format!(
                    "{:.0} % (primal) / {:.0} % (dual) of the recovery patches are undetermined and use plain averages",
                    100.0 * rec_u.diagnostics.averaged_fraction(),
                    100.0 * rec_z.diagnostics.averaged_fraction()
                ));
            }
            notes.push(format!("recovery: {} corner nodes, {} own fits, {} borrowed, {} lower-degree, {} averaged; {} interface nodes", rec_u.diagnostics.corner_nodes, rec_u.diagnostics.fitted, rec_u.diagnostics.borrowed, rec_u.diagnostics.lower_degree, rec_u.diagnostics.averaged, rec_u.interface_nodes));
            let mut field = ErrorField::new(&name(goal), Indicator::Signed, eta, estimated, reliability, t0.elapsed().as_secs_f64());
            field.notes.append(&mut notes);
            results.push(DwrResult { goal: goal.name.clone(), value, estimated_error: estimated, corrected: value + estimated, field });
        }
        Ok(results)
    }
}

fn unsupported(goal: &LinearFunctional, name: &str, why: &str, n: usize, u: &[f64]) -> DwrResult {
    let value = goal.value(u);
    DwrResult { goal: goal.name.clone(), value, estimated_error: 0.0, corrected: value, field: ErrorField::unsupported(name, why, n) }
}

/// Single-goal estimator for use through the common interface.
pub struct DwrGoalEstimator {
    pub goal: LinearFunctional,
    name: String,
}

impl DwrGoalEstimator {
    pub fn new(goal: LinearFunctional) -> Self {
        let name = format!("dwr:{}", goal.name);
        Self { goal, name }
    }
}

impl super::estimator::ErrorEstimator for DwrGoalEstimator {
    fn name(&self) -> &str {
        &self.name
    }

    fn estimate(&self, ctx: &EstimatorContext) -> Result<ErrorField, MeshError> {
        let mut r = DwrEstimator.estimate_goals(ctx, &[&self.goal])?;
        Ok(r.remove(0).field)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adaptivity::estimator::tests::solved_strip;
    use crate::meshing::adjacency::Adjacency;

    #[test]
    fn the_dual_satisfies_reciprocity_and_has_a_small_residual() {
        let (model, loads, bc, u) = solved_strip(3);
        let adj = Adjacency::new(&model.mesh);
        let ctx = EstimatorContext::new(&model, &loads, &bc, &u, &adj).unwrap();
        let tip = LinearFunctional::mean_displacement(&model, model.mesh.node_set("u1").unwrap(), 1).unwrap();
        let comp = LinearFunctional::compliance(&model, &loads).unwrap();
        let duals = solve_duals(&ctx, &[&tip, &comp]).unwrap();
        let f = crate::loads::assemble(&model.mesh, &loads).unwrap();
        for (goal, dual) in [&tip, &comp].into_iter().zip(&duals) {
            assert!(dual.residual < 1e-10, "{} residual {:e}", goal.name, dual.residual);
            // reciprocity: f . z = g . u = J(u_h)
            let fz: f64 = f.iter().zip(&dual.z).map(|(a, b)| a * b).sum();
            let ju = goal.value(&u);
            assert!((fz - ju).abs() < 1e-9 * ju.abs().max(1e-12), "{}: f.z = {fz}, J(u) = {ju}", goal.name);
            // the dual vanishes where the primal is constrained
            for &n in model.mesh.node_set("u0").unwrap() {
                assert_eq!(dual.z[2 * n], 0.0);
                assert_eq!(dual.z[2 * n + 1], 0.0);
            }
        }
        // the dual of the compliance is the primal solution itself
        let diff = duals[1].z.iter().zip(&u).map(|(a, b)| (a - b).abs()).fold(0.0, f64::max);
        assert!(diff < 1e-9 * u.iter().fold(0.0f64, |m, v| m.max(v.abs())), "{diff:e}");
    }

    #[test]
    fn a_goal_of_the_wrong_size_is_refused() {
        let (model, loads, bc, u) = solved_strip(2);
        let adj = Adjacency::new(&model.mesh);
        let ctx = EstimatorContext::new(&model, &loads, &bc, &u, &adj).unwrap();
        let bad = LinearFunctional { name: "bad".into(), gradient: vec![1.0; 3], offset: 0.0, regularised: true };
        assert!(solve_duals(&ctx, &[&bad]).is_err());
    }

    #[test]
    fn plastic_blocks_and_temperature_fields_are_unsupported() {
        let (mut model, mut loads, bc, u) = solved_strip(2);
        let adj = Adjacency::new(&model.mesh);
        let comp = LinearFunctional::compliance(&model, &loads).unwrap();
        model.mesh.blocks[0].plasticity = Some(crate::J2::elastic_finite());
        {
            let ctx = EstimatorContext::new(&model, &loads, &bc, &u, &adj).unwrap();
            let r = DwrEstimator.estimate_goals(&ctx, &[&comp]).unwrap();
            assert!(matches!(r[0].field.reliability, Reliability::Unsupported(_)));
        }
        model.mesh.blocks[0].plasticity = None;
        loads.temperature = Some(vec![0.0; model.mesh.nodes.len()]);
        let ctx = EstimatorContext::new(&model, &loads, &bc, &u, &adj).unwrap();
        let r = DwrEstimator.estimate_goals(&ctx, &[&comp]).unwrap();
        assert!(matches!(r[0].field.reliability, Reliability::Unsupported(_)));
    }
}
