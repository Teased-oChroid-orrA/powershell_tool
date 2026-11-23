//! The model-by-target matrix of minimum edge distances, with the role of each model.
//!
//! Roles are the owner's decision (2026-11-22), not a property of the numbers:
//! * the **legacy** solver check (`Fbru + 0.8 p`, in `bushing-solver`) stays *governing* - it decides PASS / REVIEW;
//! * the **contact FE** is the *best estimate* of the edge distance, shown as such;
//! * every other model is a *comparison*.
//!
//! Nothing here feeds back into a verdict: [`EdgeMatrix::build`] only reads a finished [`EdgeReport`] and the legacy
//! numbers it is handed. Promoting a stress-based number to governing stays blocked until it is correlated with
//! edge-distance test data and the plane-strain assumption is bracketed by a 3D check (`docs/edge-distance-crosscheck.md`).

use crate::runner::{EdgeMin, EdgeReport, TARGET_APPLIED, TARGET_BEARING};

/// The model that is the best estimate of the edge distance (`EdgeModel::id` of the contact FE).
pub const BEST_ESTIMATE_MODEL: &str = "contact";
/// Row id of the legacy solver check.
pub const LEGACY_ID: &str = "legacy";
/// Column labels: the nominal minimum, then the edge distance at which the failure probability falls to 10 / 5 / 1 %.
pub const LEVELS: [&str; 4] = ["Nominal", "P90", "P95", "P99"];
/// Target labels, in the order of the cells of a row (`TARGET_APPLIED`, `TARGET_BEARING`).
pub const TARGET_LABELS: [&str; 2] = ["Applied load", "Bearing limit"];

/// What the contact FE has been checked against, for the window text. The numbers are those of
/// `docs/edge-distance-crosscheck.md` (a test there keeps them equal): NACA TN 1503, a plain steel pin, no bushing.
pub const VALIDATION: &str = "Contact FE validation, NACA TN 1503 pin-bearing tests (plain pin, no bushing): 12 test points predicted 0.6 to 20 % low, mean -9.9 %, sd 6.9 %. Not validated: the interference fit and the bushing, t/D other than 0.5, e/D outside 1.5 to 2.0.";

/// How much a row's numbers count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// This tool's best estimate of the edge distance (the contact FE). Not governing.
    BestEstimate,
    /// Decides PASS / REVIEW (the legacy solver check). Unchanged by anything in this module.
    Governing,
    /// Shown for comparison only.
    Comparison,
}

impl Role {
    pub fn label(self) -> &'static str {
        match self {
            Role::BestEstimate => "best estimate",
            Role::Governing => "governing",
            Role::Comparison => "comparison",
        }
    }
}

/// The legacy solver's minimum e/D (bore diameters), as `bushing_solver::solve::compute` reports it. It has no
/// probability levels: it is a deterministic closed form.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LegacyEdge {
    /// Strength check (`ed_min_strength`): the applied-load target.
    pub applied: f64,
    /// Bearing / sequencing check (`ed_min_sequence`): the bearing-limit target.
    pub bearing: f64,
}

/// One cell: the minimum edge distance in bore diameters, and whether the housing's actual e/D is short of it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MatrixCell {
    /// Same meaning as [`EdgeMin`], but in bore diameters.
    pub min: EdgeMin,
    pub short: bool,
}

impl MatrixCell {
    fn new(min: EdgeMin, actual: f64) -> Self {
        let eps = 1e-9 * actual;
        let short = match min {
            EdgeMin::Value(v) => v > actual + eps,
            EdgeMin::Exceeds(_) => true,
            EdgeMin::AtMost(_) | EdgeMin::NotSearched => false,
        };
        Self { min, short }
    }

    /// `1.37`, `<0.75` (met at the lower end of the search) or `>8.0` (not met in the search range).
    pub fn text(&self) -> String {
        match self.min {
            EdgeMin::Value(v) => format!("{v:.2}"),
            EdgeMin::AtMost(v) => format!("<{v:.2}"),
            EdgeMin::Exceeds(v) if v.is_finite() => format!(">{v:.1}"),
            EdgeMin::Exceeds(_) => "none".to_string(),
            EdgeMin::NotSearched => "-".to_string(),
        }
    }
}

/// One model's row.
#[derive(Debug, Clone, PartialEq)]
pub struct MatrixRow {
    pub id: &'static str,
    pub label: &'static str,
    pub role: Role,
    /// The model could not be evaluated; the cells are empty.
    pub error: Option<String>,
    /// `cells[target][level]`: `None` where the model has no number (the legacy check has no probability levels).
    pub cells: [[Option<MatrixCell>; 4]; 2],
    /// First-yield e/D at the applied load, nominal (informational, not a column).
    pub first_yield: Option<MatrixCell>,
    /// Probability of failing the bearing-limit target at the actual edge distance (Monte Carlo).
    pub p_fail_bearing: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EdgeMatrix {
    /// The housing's actual e/D.
    pub actual: f64,
    /// Best estimate first, then the governing legacy check, then the comparisons in report order.
    pub rows: Vec<MatrixRow>,
}

impl EdgeMatrix {
    /// The matrix of a finished run. `legacy` is the solver's own check (omit it and there is no governing row).
    pub fn build(report: &EdgeReport, legacy: Option<LegacyEdge>) -> Self {
        let d = report.bore_diameter;
        let actual = report.edge / d;
        let in_d = |m: EdgeMin| match m {
            EdgeMin::Value(v) => EdgeMin::Value(v / d),
            EdgeMin::AtMost(v) => EdgeMin::AtMost(v / d),
            EdgeMin::Exceeds(v) => EdgeMin::Exceeds(v / d),
            EdgeMin::NotSearched => EdgeMin::NotSearched,
        };
        let cell = |m: EdgeMin| (m != EdgeMin::NotSearched).then(|| MatrixCell::new(in_d(m), actual));
        let mut rows: Vec<MatrixRow> = report
            .models
            .iter()
            .map(|m| {
                let mut cells = [[None; 4]; 2];
                if m.error.is_none() {
                    for (ti, row) in cells.iter_mut().enumerate() {
                        if let Some(t) = m.targets.get(ti).and_then(|t| t.as_ref()) {
                            row[0] = cell(t.e_min);
                            for l in 0..3 {
                                row[l + 1] = cell(t.e_levels[l]);
                            }
                        }
                    }
                }
                let at = |name: &str| m.targets.iter().flatten().find(|t| t.target.label.starts_with(name));
                MatrixRow {
                    id: m.id,
                    label: m.label,
                    role: if m.id == BEST_ESTIMATE_MODEL { Role::BestEstimate } else { Role::Comparison },
                    error: m.error.clone(),
                    cells,
                    first_yield: m.error.is_none().then(|| at("First yield").and_then(|t| cell(t.e_min))).flatten(),
                    p_fail_bearing: m.targets.get(TARGET_BEARING).and_then(|t| t.as_ref()).and_then(|t| t.mc.as_ref()).map(|mc| mc.p_fail),
                }
            })
            .collect();
        if let Some(l) = legacy {
            let c = |v: f64| Some(MatrixCell::new(if v.is_finite() { EdgeMin::Value(v) } else { EdgeMin::Exceeds(f64::INFINITY) }, actual));
            let mut cells = [[None; 4]; 2];
            cells[TARGET_APPLIED][0] = c(l.applied);
            cells[TARGET_BEARING][0] = c(l.bearing);
            rows.push(MatrixRow { id: LEGACY_ID, label: "Legacy solver", role: Role::Governing, error: None, cells, first_yield: None, p_fail_bearing: None });
        }
        // Stable: best estimate, governing, then the rest in report order.
        let rank = |r: &MatrixRow| match r.role {
            Role::BestEstimate => 0,
            Role::Governing => 1,
            Role::Comparison => 2,
        };
        rows.sort_by_key(rank);
        Self { actual, rows }
    }

    /// The best-estimate row, if the contact FE was run (it is the deep check) and evaluated.
    pub fn best_estimate(&self) -> Option<&MatrixRow> {
        self.rows.iter().find(|r| r.role == Role::BestEstimate && r.error.is_none())
    }

    /// No governing row: the workbench has no legacy check of its own, so every number here is advisory.
    pub fn is_advisory(&self) -> bool {
        self.governing().is_none()
    }

    /// The governing row (the legacy check).
    pub fn governing(&self) -> Option<&MatrixRow> {
        self.rows.iter().find(|r| r.role == Role::Governing)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_validation_text_quotes_the_numbers_of_the_validation_doc() {
        let doc = include_str!("../../docs/edge-distance-crosscheck.md");
        for n in ["0.6-20 %", "mean -9.9 %", "sd 6.9 %"] {
            assert!(doc.contains(n), "the doc no longer says {n:?}");
        }
        for n in ["0.6 to 20 %", "mean -9.9 %", "sd 6.9 %", "12 test points", "t/D other than 0.5", "e/D outside 1.5 to 2.0"] {
            assert!(VALIDATION.contains(n), "the window text lost {n:?}");
        }
        assert!(doc.contains("**Not validated**: the interference fit and the bushing"));
    }

    #[test]
    fn a_cell_is_short_only_when_the_actual_edge_distance_is_below_it() {
        assert!(MatrixCell::new(EdgeMin::Value(1.7), 1.5).short);
        assert!(!MatrixCell::new(EdgeMin::Value(1.3), 1.5).short);
        assert!(!MatrixCell::new(EdgeMin::Value(1.5), 1.5).short, "exactly enough is enough");
        assert!(MatrixCell::new(EdgeMin::Exceeds(8.0), 1.5).short);
        assert!(!MatrixCell::new(EdgeMin::AtMost(0.75), 1.5).short);
        assert_eq!(MatrixCell::new(EdgeMin::Value(1.374), 1.5).text(), "1.37");
        assert_eq!(MatrixCell::new(EdgeMin::AtMost(0.75), 1.5).text(), "<0.75");
        assert_eq!(MatrixCell::new(EdgeMin::Exceeds(8.0), 1.5).text(), ">8.0");
        assert_eq!(MatrixCell::new(EdgeMin::Exceeds(f64::INFINITY), 1.5).text(), "none");
    }
}
