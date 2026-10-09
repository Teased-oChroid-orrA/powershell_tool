//! Compliance minimisation over block stiffness scales at a fixed material budget (sizing optimisation) by the optimality-
//! criteria update on the adjoint sensitivities of `sensitivity.rs`.
//!
//! Variables `s_b` multiply the modulus of block `b`; the budget is `sum_b w_b s_b = budget` with `w_b` the block volumes (the
//! amount of material a unit scale costs). Each iteration: sensitivities `dC/ds_b <= 0`, then `s_b <- clamp(s_b sqrt(-dC/ds_b /
//! (mu w_b)))` within a move limit, `mu` found by bisection so that the budget holds. The result is a stationary point of the
//! constrained problem (checked by the equal-sensitivity-per-weight condition on the free variables), not a global optimum claim.

use crate::analysis::Model;
use crate::kernel::{self, Work};
use crate::linear::Dirichlet;
use crate::loads::Loads;
use crate::sensitivity::Param;

#[derive(Debug, Clone)]
pub struct OptOptions {
    pub s_min: f64,
    pub s_max: f64,
    /// Largest relative change of a variable per iteration.
    pub move_limit: f64,
    pub max_iter: usize,
    /// Stop when the largest relative change of a variable falls below this.
    pub tol: f64,
}

impl Default for OptOptions {
    fn default() -> Self {
        Self { s_min: 1e-3, s_max: 1e3, move_limit: 0.3, max_iter: 200, tol: 1e-8 }
    }
}

#[derive(Debug, Clone)]
pub struct OptResult {
    pub scales: Vec<f64>,
    pub compliance: Vec<f64>,
    pub iterations: usize,
    pub converged: bool,
    /// `-dC/ds_b / w_b` of the variables strictly inside their bounds: equal at a stationary point.
    pub free_sensitivity_spread: f64,
}

impl Model {
    /// Volume (area x thickness in plane problems) of every block.
    pub fn block_volumes(&self) -> Result<Vec<f64>, String> {
        let mut work = Work::new();
        let mut out = Vec::new();
        for blk in &self.mesh.blocks {
            let nn = blk.kind.n_nodes();
            let mut me = vec![0.0; nn * nn];
            let mut total = 0.0;
            for conn in blk.conn.chunks_exact(nn) {
                let xyz: Vec<[f64; 3]> = conn.iter().map(|&n| self.mesh.nodes[n]).collect();
                kernel::mass(blk.kind, self.mesh.physics, 1.0, &xyz, &mut work, &mut me).map_err(|e| e.to_string())?;
                total += me[..nn * nn].iter().sum::<f64>();
            }
            out.push(total);
        }
        Ok(out)
    }

    /// Minimise the compliance over the block stiffness scales at the budget `sum w_b s_b = budget`, starting from `s = 1`
    /// (the model's current moduli). `blocks` selects the design blocks (others stay fixed).
    pub fn optimize_compliance(&self, loads: &Loads, bc: &Dirichlet, blocks: &[usize], budget: f64, opt: &OptOptions) -> Result<OptResult, String> {
        let w_all = self.block_volumes()?;
        let w: Vec<f64> = blocks.iter().map(|&b| w_all[b]).collect();
        let fixed_part: f64 = (0..self.mesh.blocks.len()).filter(|b| !blocks.contains(b)).map(|b| w_all[b]).sum();
        let target = budget - fixed_part;
        let (lo, hi) = (opt.s_min, opt.s_max);
        if !(target > lo * w.iter().sum::<f64>() && target < hi * w.iter().sum::<f64>()) {
            return Err(format!("the budget {budget} cannot be met within the scale bounds [{lo}, {hi}] of the design blocks"));
        }
        let e0: Vec<f64> = blocks.iter().map(|&b| self.mesh.blocks[b].material.e).collect();
        // Start from the uniform scale that meets the budget.
        let mut s = vec![target / w.iter().sum::<f64>(); blocks.len()];
        let apply = |s: &[f64]| -> Model {
            let mut mesh = self.mesh.clone();
            for (k, &b) in blocks.iter().enumerate() {
                mesh.blocks[b].material.e = e0[k] * s[k];
            }
            Model { mesh, pattern: self.pattern.clone() }
        };
        let (mut history, mut converged) = (Vec::new(), false);
        let mut iterations = 0;
        let mut last_dc = vec![0.0; blocks.len()];
        for it in 0..opt.max_iter {
            iterations = it + 1;
            let m = apply(&s);
            let params: Vec<Param> = blocks.iter().map(|&b| Param::ModulusScale(b)).collect();
            let dc = m.compliance_sensitivities(loads, bc, &params)?; // with respect to a multiplier of the CURRENT modulus
            // d C / d s_b (s relative to the original modulus) = dC/d(multiplier) / s_b.
            let g: Vec<f64> = dc.iter().zip(&s).map(|(d, sb)| -d / sb).collect();
            last_dc = g.clone();
            let f = crate::loads::assemble(&m.mesh, loads)?;
            let u = m.solve_static(loads, bc)?.u;
            history.push(f.iter().zip(&u).map(|(a, b)| a * b).sum());
            // Optimality-criteria update with the multiplier found by bisection on the budget.
            let update = |mu: f64| -> Vec<f64> {
                (0..s.len())
                    .map(|k| {
                        let target_s = s[k] * (g[k].max(0.0) / (mu * w[k])).sqrt();
                        target_s.clamp(s[k] * (1.0 - opt.move_limit), s[k] * (1.0 + opt.move_limit)).clamp(lo, hi)
                    })
                    .collect()
            };
            let used = |x: &[f64]| x.iter().zip(&w).map(|(a, b)| a * b).sum::<f64>();
            let (mut a, mut b) = (1e-30f64, 1e30f64);
            for _ in 0..200 {
                let mu = (a * b).sqrt();
                if used(&update(mu)) > target {
                    a = mu;
                } else {
                    b = mu;
                }
            }
            let new = update((a * b).sqrt());
            let change = new.iter().zip(&s).map(|(n, o)| (n - o).abs() / o).fold(0.0, f64::max);
            s = new;
            if change < opt.tol {
                converged = true;
                break;
            }
        }
        let free: Vec<f64> = (0..s.len()).filter(|&k| s[k] > lo * (1.0 + 1e-9) && s[k] < hi * (1.0 - 1e-9)).map(|k| last_dc[k] / w[k]).collect();
        let spread = if free.is_empty() { 0.0 } else { (free.iter().cloned().fold(f64::NEG_INFINITY, f64::max) - free.iter().cloned().fold(f64::INFINITY, f64::min)) / free.iter().cloned().fold(0.0f64, f64::max).max(1e-300) };
        Ok(OptResult { scales: s, compliance: history, iterations, converged, free_sensitivity_spread: spread })
    }
}
