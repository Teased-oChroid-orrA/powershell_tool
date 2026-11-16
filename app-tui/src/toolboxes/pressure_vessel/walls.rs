//! "Minimum wall by requirement": the thinnest wall at which each requirement row's margin reaches its required value, the
//! margin each row has at the wall entered, and the governing row. Nothing is computed here: the numbers are
//! `pressure_vessel_solver::criterion_thickness` (`solve_per_criterion` for the cylinder, `solve_line_run_walls` for a
//! supported line run), this only maps the rows onto those inputs and the outcomes onto one display type.
use super::requirements::Requirement;
use pressure_vessel_solver::criterion_thickness::{solve_line_run_walls, LineWallOutcome, LineWallRequirement, CriterionOutcome, CriterionRequirement};
use pressure_vessel_solver::line_run::{LineRun, LoadCase, MaterialClass};

/// What the solver found for one requirement row.
#[derive(Clone, Debug, PartialEq)]
pub enum WallOutcome {
    /// The thinnest remaining wall (in) that meets the row. `note` says when that number is a limit of the search.
    Wall { wall: f64, note: &'static str },
    /// No wall up to the search limit meets it (the best margin found).
    Infeasible { best_margin: f64 },
    NotEvaluable(String),
}

/// One row's result: its minimum wall and its margin at the wall entered (`None`: not evaluable).
#[derive(Clone, Debug, PartialEq)]
pub struct WallRow {
    pub outcome: WallOutcome,
    /// `allowable / (factor * demand) - 1` at the current wall.
    pub margin_at_wall: Option<f64>,
}

/// Every row's result (parallel to the requirements) and the wall they are measured against.
#[derive(Clone, Debug, PartialEq)]
pub struct Walls {
    pub rows: Vec<WallRow>,
    /// The remaining wall the margins were taken at.
    pub current_wall: f64,
}

impl Walls {
    /// An infeasible row if there is one, else the row with the thickest minimum wall (the solver's own rule).
    pub fn governing(&self) -> Option<usize> {
        if let Some(i) = self.rows.iter().position(|r| matches!(r.outcome, WallOutcome::Infeasible { .. })) {
            return Some(i);
        }
        self.rows
            .iter()
            .enumerate()
            .filter_map(|(i, r)| match r.outcome {
                WallOutcome::Wall { wall, .. } => Some((wall, i)),
                _ => None,
            })
            .max_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, i)| i)
    }
}

/// Why a row cannot be handed to the solver, in the table's words; `None` when it can.
pub fn invalid(r: &Requirement) -> Option<&'static str> {
    if !(r.factor.is_finite() && r.factor > 0.0) {
        Some("factor must be positive")
    } else if !r.required_ms.is_finite() {
        Some("required MS must be a number")
    } else {
        None
    }
}

/// Everything `solve_line_run_walls` needs, owned so the solve can run on the worker while the window stays responsive.
pub struct LineWallJob {
    pub run: LineRun,
    pub class: MaterialClass,
    pub cases: Vec<LoadCase>,
    pub rows: Vec<Requirement>,
}

impl LineWallJob {
    /// One outcome per row. The row's factor is applied to the demand, which is the same as asking the unfactored margin to
    /// reach `factor * (1 + required MS) - 1` (the class's own safety factor is 1 here: the rows carry the factors).
    pub fn run(self) -> Result<Vec<WallOutcome>, String> {
        let mut reqs = Vec::new();
        let mut slot: Vec<Option<usize>> = Vec::new();
        for r in &self.rows {
            match (invalid(r), r.criterion.line()) {
                (None, Some(criterion)) => {
                    slot.push(Some(reqs.len()));
                    reqs.push(LineWallRequirement { label: r.label.clone(), criterion, required_ms: r.factor * (1.0 + r.required_ms) - 1.0 });
                }
                _ => slot.push(None),
            }
        }
        let solved = if reqs.is_empty() { Vec::new() } else { solve_line_run_walls(&self.run, &self.class, &self.cases, &reqs, 200, 1e-9).map_err(|e| e.to_string())?.rows };
        Ok(self
            .rows
            .iter()
            .zip(slot)
            .map(|(r, s)| match s.map(|i| &solved[i].outcome) {
                None => WallOutcome::NotEvaluable(invalid(r).map_or_else(|| format!("{} is not evaluated for a line run", r.criterion.label()), str::to_string)),
                Some(LineWallOutcome::Converged { wall, .. }) => WallOutcome::Wall { wall: *wall, note: "" },
                Some(LineWallOutcome::BelowSearchFloor { floor_wall, .. }) => WallOutcome::Wall { wall: *floor_wall, note: "below search floor" },
                Some(LineWallOutcome::Infeasible { best_margin_found, .. }) => WallOutcome::Infeasible { best_margin: *best_margin_found },
                Some(LineWallOutcome::NotEvaluable(why)) => WallOutcome::NotEvaluable((*why).to_string()),
            })
            .collect())
    }
}

/// The cylinder's outcomes mapped to the display type (`rows` parallel to the requirements; `solved` is parallel to the rows
/// that were valid and evaluable for the cylinder, `slot` says which).
pub fn cylinder_outcome(rows: &[Requirement], slot: &[Option<usize>], solved: &[(CriterionRequirement, CriterionOutcome)]) -> Vec<WallOutcome> {
    rows.iter()
        .zip(slot)
        .map(|(r, s)| match s.map(|i| &solved[i].1) {
            None => WallOutcome::NotEvaluable(invalid(r).map_or_else(|| format!("{} is not evaluated for a cylinder", r.criterion.label()), str::to_string)),
            Some(CriterionOutcome::Converged { wall_thickness, validity_limited, .. }) => WallOutcome::Wall { wall: *wall_thickness, note: if *validity_limited { "thin-shell limit" } else { "" } },
            Some(CriterionOutcome::Infeasible { best_margin_found, .. }) => WallOutcome::Infeasible { best_margin: *best_margin_found },
            Some(CriterionOutcome::NotEvaluable(why)) => WallOutcome::NotEvaluable((*why).to_string()),
        })
        .collect()
}

/// Factor a margin the way a row does: `(1 + margin) / factor - 1`; an unbounded margin stays unbounded.
pub fn factored(margin: f64, factor: f64) -> f64 {
    if margin.is_finite() { (1.0 + margin) / factor - 1.0 } else { margin }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(outcome: WallOutcome) -> WallRow {
        WallRow { outcome, margin_at_wall: None }
    }

    #[test]
    fn governing_is_an_infeasible_row_else_the_thickest_wall() {
        let w = Walls { rows: vec![row(WallOutcome::Wall { wall: 0.2, note: "" }), row(WallOutcome::NotEvaluable("x".into())), row(WallOutcome::Wall { wall: 0.3, note: "" })], current_wall: 1.0 };
        assert_eq!(w.governing(), Some(2));
        let mut w = w;
        w.rows.push(row(WallOutcome::Infeasible { best_margin: -0.5 }));
        assert_eq!(w.governing(), Some(3));
        assert_eq!(Walls { rows: vec![row(WallOutcome::NotEvaluable("x".into()))], current_wall: 1.0 }.governing(), None);
    }

    #[test]
    fn a_factored_margin_matches_the_solvers_definition() {
        // allowable 100, demand 40, factor 1.5: 100 / 60 - 1.
        assert!((factored(100.0 / 40.0 - 1.0, 1.5) - (100.0 / 60.0 - 1.0)).abs() < 1e-15);
        assert_eq!(factored(f64::INFINITY, 2.0), f64::INFINITY);
    }

    #[test]
    fn invalid_rows_are_named_not_defaulted() {
        let mut r = Requirement::neutral("x", super::super::requirements::Req::VonMisesYield);
        assert_eq!(invalid(&r), None);
        r.factor = 0.0;
        assert!(invalid(&r).is_some());
        r.factor = 1.0;
        r.required_ms = f64::NAN;
        assert!(invalid(&r).is_some());
    }
}
