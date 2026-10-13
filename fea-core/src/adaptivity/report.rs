//! Structured (JSON) reports of adaptive runs: the stop reason, one record per cycle with its sizes, estimates, reliability notes
//! and stage timings, so runs can be archived and compared (`docs/hybrid-meshing-amr-evidence.md` is made from them).

use super::controller::{AmrResult, Stop};
use super::nonlinear::{NlResult, NlStop};
use crate::report::{json_str, num};
use std::fmt::Write;

impl AmrResult {
    /// The run as a JSON object.
    pub fn report_json(&self) -> String {
        let stop = match &self.stop {
            Stop::Converged => "converged".to_string(),
            Stop::MaxCycles => "max_cycles".to_string(),
            Stop::BudgetExhausted => "budget_exhausted".to_string(),
            Stop::NothingToRefine => "nothing_to_refine".to_string(),
            Stop::Stagnated(w) => format!("stagnated: {w}"),
            Stop::RolledBack(w) => format!("rolled_back: {w}"),
            Stop::EstimatorUnavailable(w) => format!("estimator_unavailable: {w}"),
        };
        let mut o = String::new();
        let _ = write!(o, "{{\"stop\":{},\"cycles\":[", json_str(&stop));
        for (i, c) in self.history.iter().enumerate() {
            if i > 0 {
                o.push(',');
            }
            let _ = write!(o, "{{\"cycle\":{},\"elements\":{},\"dofs\":{},\"factor_nnz\":{},\"factor_bytes\":{},\"physics_error\":{},\"physics_estimator\":{},\"goal_estimator\":{},\"qoi_converged\":{},\"physics_converged\":{},\"marked\":{},\"mandatory\":{},\"capped\":{},\"min_scaled_jacobian\":{},\"structured_fraction\":{},\"transition_elements\":{},\"solution_change\":{},", c.cycle, c.elements, c.dofs, c.factor_nnz, c.factor_bytes, num(c.physics_error), json_str(&c.physics_estimator), json_str(&c.goal_estimator), c.qoi_converged, c.physics_converged, c.marked, c.mandatory, c.capped, num(c.min_scaled_jacobian), num(c.structured_fraction), c.transition_elements, c.solution_change.map_or("null".into(), num));
            o.push_str("\"goals\":[");
            for (j, g) in c.goals.iter().enumerate() {
                if j > 0 {
                    o.push(',');
                }
                let _ = write!(o, "{{\"name\":{},\"value\":{},\"estimated_error\":{},\"corrected\":{},\"tolerance\":{},\"reliability\":{}}}", json_str(&g.name), num(g.value), num(g.estimated_error), num(g.corrected), g.tolerance.map_or("null".into(), num), json_str(&format!("{:?}", g.reliability)));
            }
            let s = &c.seconds;
            let _ = write!(o, "],\"seconds\":{{\"solve\":{},\"goals\":{},\"physics\":{},\"fusion\":{},\"marking_and_sizing\":{},\"meshing\":{},\"checks\":{}}},\"notes\":[", num(s.solve), num(s.goals), num(s.physics), num(s.fusion), num(s.marking_and_sizing), num(s.meshing), num(s.checks));
            for (j, n) in c.reliability.iter().enumerate() {
                if j > 0 {
                    o.push(',');
                }
                o.push_str(&json_str(n));
            }
            o.push_str("]}");
        }
        o.push_str("]}");
        o
    }
}

impl NlResult {
    /// The nonlinear run as a JSON object.
    pub fn report_json(&self) -> String {
        let stop = match &self.stop {
            NlStop::Converged => "converged".to_string(),
            NlStop::MaxCycles => "max_cycles".to_string(),
            NlStop::BudgetExhausted => "budget_exhausted".to_string(),
            NlStop::NothingToRefine => "nothing_to_refine".to_string(),
            NlStop::SolveFailed(w) => format!("solve_failed: {w}"),
            NlStop::RolledBack(w) => format!("rolled_back: {w}"),
        };
        let mut o = String::new();
        let _ = write!(o, "{{\"stop\":{},\"cycles\":[", json_str(&stop));
        for (i, c) in self.history.iter().enumerate() {
            if i > 0 {
                o.push(',');
            }
            let _ = write!(o, "{{\"cycle\":{},\"elements\":{},\"dofs\":{},\"physics_error\":{},\"max_ep\":{},\"plastic_fraction\":{},\"contact_active\":{},\"contact_halfwidth\":{},\"contact_balance\":{},\"load_steps\":{},\"newton_iterations\":{},\"factorisations\":{},\"marked\":{},\"mandatory\":{},\"contact_protected\":{},\"solve_seconds\":{},\"warm_started\":{},\"goals\":[", c.cycle, c.elements, c.dofs, num(c.physics_error), num(c.max_ep), num(c.plastic_fraction), c.contact_active, num(c.contact_halfwidth), num(c.contact_balance), c.load_steps, c.newton_iterations, c.factorisations, c.marked, c.mandatory, c.contact_protected, num(c.solve_seconds), c.warm_started);
            for (j, ((name, v), ch)) in c.goals.iter().zip(&c.goal_change).enumerate() {
                if j > 0 {
                    o.push(',');
                }
                let _ = write!(o, "{{\"name\":{},\"value\":{},\"change\":{}}}", json_str(name), num(*v), num(*ch));
            }
            o.push_str("]}");
        }
        o.push_str("]}");
        o
    }
}
