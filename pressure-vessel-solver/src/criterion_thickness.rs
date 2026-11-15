//! Minimum wall thickness AT EACH failure criterion, separately (owner request 2026-11-15): for
//! every requirement the user enters, the thinnest wall whose margin for that one criterion reaches
//! the required value, plus the governing (thickest) one. The owner compares these against their
//! own program requirements outside the tool, so **nothing here is defaulted**: every requirement
//! row carries its own load factor and required margin, and the material allowables come from the
//! caller's `Material` (or the line-run `MaterialClass`). Public references for the kind of factors
//! one might enter (14 CFR 25.1435 proof 1.5 / ultimate 3.0 x design operating pressure, 25.303
//! 1.5 on limit load, 25.625 fitting factor 1.15) are examples in `docs/pressure-vessel-thickness.md`,
//! never values used by code.
//!
//! Margin of a requirement row: `MS = allowable / (factor * applied) - 1`, the crate convention
//! (`failure::margin_of_safety`) with the factor multiplying the demand (equal to scaling the load,
//! because every stress and the buckling pressure demand are linear in pressure). The wall is
//! acceptable when `MS >= required_ms`.
//!
//! * [`solve_per_criterion`]: cylinder with fixed bore, wall found by [`bisect_outer_radius`] - the
//!   same bisection as [`crate::thickness::solve_minimum_thickness`], once per row, on that row's
//!   own margin (the whole-set governing margin is not used). Sound because each of these margins
//!   does not decrease with wall thickness, which `tests` checks per criterion.
//! * [`solve_line_run_walls`]: supported tube with fixed outer diameter and loads, wall varied. Wall
//!   thickness also changes the tube weight and the thermal-restraint force, so monotonicity is not
//!   assumed: the wall range is scanned first and the answer is bracketed at the LAST failing
//!   sample, then bisected.

use crate::buckling::{evaluate_buckling, BucklingApplicability};
use crate::failure::{max_abs_principal_stress, tresca_stress, von_mises_stress, worst_of, MarginResult};
use crate::geometry::CylinderGeometry;
use crate::line_run::{analyze, Criterion, LineRun, LineRunError, LoadCase, MaterialClass};
use crate::pressure::PressureLoading;
use crate::stress::{stress_at_inner_surface, stress_at_outer_surface};
use crate::thickness::{bisect_outer_radius, RadiusSearch};
use mechanics_core::materials::Material;

/// A failure criterion of the cylinder with a defined minimum thickness.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThicknessCriterion {
    /// Maximum |principal| stress against yield.
    YieldMaxPrincipal,
    VonMisesYield,
    TrescaYield,
    /// Maximum |principal| stress against `Ftu`.
    UltimateMaxPrincipal,
    UltimateVonMises,
    /// External-pressure buckling (`buckling.rs`): needs external pressure and an unsupported length.
    Buckling,
}
impl ThicknessCriterion {
    pub fn label(self) -> &'static str {
        match self {
            Self::YieldMaxPrincipal => "Yield (maximum principal)",
            Self::VonMisesYield => "Von Mises yield",
            Self::TrescaYield => "Tresca (maximum shear) yield",
            Self::UltimateMaxPrincipal => "Ultimate (maximum principal)",
            Self::UltimateVonMises => "Ultimate (von Mises)",
            Self::Buckling => "Buckling (external pressure)",
        }
    }
}

/// One user requirement: `criterion` must reach `required_ms` with `factor` applied to the demand.
/// The same criterion may appear in several rows (e.g. proof and burst factors).
#[derive(Debug, Clone, PartialEq)]
pub struct CriterionRequirement {
    pub label: String,
    pub criterion: ThicknessCriterion,
    /// Multiplies the demand; finite and positive.
    pub factor: f64,
    /// Margin the wall must reach; finite (0 = exactly at the allowable).
    pub required_ms: f64,
}

#[derive(Debug, Clone)]
pub struct PerCriterionInputs {
    pub inner_radius: f64,
    pub pressure: PressureLoading,
    pub material: Material,
    /// Unsupported length for [`ThicknessCriterion::Buckling`].
    pub unsupported_length: Option<f64>,
    pub requirements: Vec<CriterionRequirement>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CriterionOutcome {
    Converged {
        outer_radius: f64,
        wall_thickness: f64,
        /// Factored margin at `wall_thickness` (`>= required_ms`); `None` when the buckling formulas no
        /// longer apply at this thickness (see `validity_limited`).
        margin: Option<f64>,
        /// Buckling only: the requirement is met by reaching the thick-wall limit where the thin-shell
        /// formulas stop applying (`buckling.rs`), so `wall_thickness` is that limit, not a buckling margin.
        validity_limited: bool,
        iterations: u32,
    },
    /// No wall up to the search limit meets the requirement (explained, not an unexplained number).
    Infeasible { largest_radius_tried: f64, best_margin_found: f64 },
    /// The criterion does not apply to these inputs (reason given).
    NotEvaluable(&'static str),
}

#[derive(Debug, Clone, PartialEq)]
pub struct CriterionThickness {
    pub requirement: CriterionRequirement,
    pub outcome: CriterionOutcome,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PerCriterionSolution {
    pub rows: Vec<CriterionThickness>,
}
impl PerCriterionSolution {
    /// The governing row: an `Infeasible` row if any (no wall satisfies it, so nothing is thick enough),
    /// otherwise the `Converged` row with the largest wall. `NotEvaluable` rows never govern.
    pub fn governing(&self) -> Option<&CriterionThickness> {
        if let Some(r) = self.rows.iter().find(|r| matches!(r.outcome, CriterionOutcome::Infeasible { .. })) {
            return Some(r);
        }
        self.rows
            .iter()
            .filter_map(|r| match r.outcome {
                CriterionOutcome::Converged { wall_thickness, .. } => Some((wall_thickness, r)),
                _ => None,
            })
            .max_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, r)| r)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum RequirementError {
    InvalidInnerRadius,
    NoRequirements,
    /// Row index with a non-positive / non-finite factor or non-finite required margin.
    InvalidRequirement(usize),
}

enum Probe {
    /// Unfactored margin (`allowable / applied - 1`).
    Margin(f64),
    /// Buckling only: thicker than the formulas' validity range.
    BeyondValidity,
    Unavailable(&'static str),
}

fn probe(criterion: ThicknessCriterion, geometry: &CylinderGeometry, inputs: &PerCriterionInputs) -> Probe {
    let sy = inputs.material.sy_ksi * 1000.0;
    let ftu = inputs.material.ftu_ksi * 1000.0;
    let inner = stress_at_inner_surface(geometry, &inputs.pressure);
    let outer = stress_at_outer_surface(geometry, &inputs.pressure);
    let m: MarginResult = match criterion {
        ThicknessCriterion::YieldMaxPrincipal => worst_of("", max_abs_principal_stress, sy, &inner, &outer),
        ThicknessCriterion::VonMisesYield => worst_of("", von_mises_stress, sy, &inner, &outer),
        ThicknessCriterion::TrescaYield => worst_of("", tresca_stress, sy, &inner, &outer),
        ThicknessCriterion::UltimateMaxPrincipal => worst_of("", max_abs_principal_stress, ftu, &inner, &outer),
        ThicknessCriterion::UltimateVonMises => worst_of("", von_mises_stress, ftu, &inner, &outer),
        ThicknessCriterion::Buckling => {
            return match evaluate_buckling(geometry, &inputs.pressure, &inputs.material, inputs.unsupported_length) {
                BucklingApplicability::Evaluated(r) => Probe::Margin(r.margin),
                BucklingApplicability::OutsideValidityRange => Probe::BeyondValidity,
                BucklingApplicability::NotApplicable => Probe::Unavailable("no external pressure"),
                BucklingApplicability::InsufficientData => Probe::Unavailable("unsupported length not given"),
            };
        }
    };
    Probe::Margin(m.margin)
}

/// `allowable / (factor * applied) - 1` from the unfactored margin; an unbounded margin stays unbounded.
fn factored(unfactored_margin: f64, factor: f64) -> f64 {
    if unfactored_margin.is_finite() {
        (1.0 + unfactored_margin) / factor - 1.0
    } else {
        unfactored_margin
    }
}

fn solve_row(inputs: &PerCriterionInputs, req: &CriterionRequirement, max_iterations: u32, radius_tolerance: f64) -> CriterionOutcome {
    let a = inputs.inner_radius;
    // Applicability is decided on a thin real wall (the formulas' validity range only cuts off thick walls).
    if let Some(Probe::Unavailable(why)) = CylinderGeometry::new(a, a * 1.01).ok().map(|g| probe(req.criterion, &g, inputs)) {
        return CriterionOutcome::NotEvaluable(why);
    }
    let margin_at = |ro: f64| {
        let g = CylinderGeometry::new(a, ro).ok()?;
        match probe(req.criterion, &g, inputs) {
            Probe::Margin(m) => Some(factored(m, req.factor)),
            Probe::BeyondValidity => Some(f64::INFINITY),
            Probe::Unavailable(_) => None,
        }
    };
    match bisect_outer_radius(a, req.required_ms, max_iterations, radius_tolerance, margin_at) {
        RadiusSearch::Infeasible { largest_radius_tried, best_margin_found } => CriterionOutcome::Infeasible { largest_radius_tried, best_margin_found },
        RadiusSearch::Feasible { upper, iterations } => {
            let g = CylinderGeometry::new(a, upper).expect("upper bound was validated by the search");
            let (margin, validity_limited) = match probe(req.criterion, &g, inputs) {
                Probe::Margin(m) => (Some(factored(m, req.factor)), false),
                _ => (None, true),
            };
            CriterionOutcome::Converged { outer_radius: upper, wall_thickness: upper - a, margin, validity_limited, iterations }
        }
    }
}

/// Minimum wall thickness at each requirement row, separately. `max_iterations` / `radius_tolerance`
/// are the bisection controls of [`crate::thickness::solve_minimum_thickness`] (never loosened to pass).
pub fn solve_per_criterion(inputs: &PerCriterionInputs, max_iterations: u32, radius_tolerance: f64) -> Result<PerCriterionSolution, RequirementError> {
    if !(inputs.inner_radius.is_finite() && inputs.inner_radius > 0.0) {
        return Err(RequirementError::InvalidInnerRadius);
    }
    if inputs.requirements.is_empty() {
        return Err(RequirementError::NoRequirements);
    }
    for (i, r) in inputs.requirements.iter().enumerate() {
        if !(r.factor.is_finite() && r.factor > 0.0 && r.required_ms.is_finite()) {
            return Err(RequirementError::InvalidRequirement(i));
        }
    }
    let rows = inputs
        .requirements
        .iter()
        .map(|req| CriterionThickness { requirement: req.clone(), outcome: solve_row(inputs, req, max_iterations, radius_tolerance) })
        .collect();
    Ok(PerCriterionSolution { rows })
}

// ---- supported line run ------------------------------------------------------------------------

/// A line-run requirement: `criterion`'s margin (the material class's own factor of safety already
/// included, see `LineMargin`) over all load cases must reach `required_ms`.
#[derive(Debug, Clone, PartialEq)]
pub struct LineWallRequirement {
    pub label: String,
    pub criterion: Criterion,
    pub required_ms: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LineWallOutcome {
    Converged { wall: f64, margin: f64, iterations: u32 },
    /// Met even at the thinnest wall searched (`1e-4` of the outer radius); the true minimum is below it.
    BelowSearchFloor { floor_wall: f64, margin: f64 },
    /// Not met at any wall up to `0.95` of the outer radius.
    Infeasible { largest_wall_tried: f64, best_margin_found: f64 },
    NotEvaluable(&'static str),
}

#[derive(Debug, Clone, PartialEq)]
pub struct LineWallRow {
    pub requirement: LineWallRequirement,
    pub outcome: LineWallOutcome,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LineWallSolution {
    pub rows: Vec<LineWallRow>,
}
impl LineWallSolution {
    /// Same rule as [`PerCriterionSolution::governing`]; `BelowSearchFloor` counts as its floor wall.
    pub fn governing(&self) -> Option<&LineWallRow> {
        if let Some(r) = self.rows.iter().find(|r| matches!(r.outcome, LineWallOutcome::Infeasible { .. })) {
            return Some(r);
        }
        self.rows
            .iter()
            .filter_map(|r| match r.outcome {
                LineWallOutcome::Converged { wall, .. } => Some((wall, r)),
                LineWallOutcome::BelowSearchFloor { floor_wall, .. } => Some((floor_wall, r)),
                _ => None,
            })
            .max_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, r)| r)
    }
}

const WALL_SCAN_SAMPLES: usize = 64;
const WALL_FLOOR_FRACTION: f64 = 1e-4;
const WALL_CEILING_FRACTION: f64 = 0.95;

/// Lowest margin of `criterion` over all cases, `None` when the material class does not evaluate it.
fn line_margin(run: &LineRun, class: &MaterialClass, cases: &[LoadCase], criterion: Criterion, wall: f64) -> Result<Option<f64>, LineRunError> {
    let mut r = run.clone();
    r.tube.wall = wall;
    let result = analyze(&r, class, cases)?;
    Ok(result.cases.iter().flat_map(|c| c.margins.iter()).filter(|m| m.criterion == criterion).map(|m| m.margin).min_by(f64::total_cmp))
}

/// Minimum wall of a supported tube (outer diameter, supports and loads fixed, wall varied) at each
/// requirement row. Errors (unsupported class, bad loads) are those of [`analyze`].
pub fn solve_line_run_walls(
    run: &LineRun,
    class: &MaterialClass,
    cases: &[LoadCase],
    requirements: &[LineWallRequirement],
    max_iterations: u32,
    wall_tolerance: f64,
) -> Result<LineWallSolution, LineRunError> {
    if requirements.is_empty() {
        return Err(LineRunError::InvalidInput("no wall requirement".into()));
    }
    if let Some(r) = requirements.iter().find(|r| !r.required_ms.is_finite()) {
        return Err(LineRunError::InvalidInput(format!("required margin of '{}' must be finite", r.label)));
    }
    let ro = run.tube.outer_diameter / 2.0;
    let (floor, ceiling) = (ro * WALL_FLOOR_FRACTION, ro * WALL_CEILING_FRACTION);
    let ratio = (ceiling / floor).powf(1.0 / (WALL_SCAN_SAMPLES as f64 - 1.0));
    let walls: Vec<f64> = (0..WALL_SCAN_SAMPLES).map(|i| if i + 1 == WALL_SCAN_SAMPLES { ceiling } else { floor * ratio.powi(i as i32) }).collect();

    let mut rows = Vec::new();
    for req in requirements {
        let margin_at = |w: f64| line_margin(run, class, cases, req.criterion, w);
        // Scan: margin per sample; `None` (criterion not evaluated for this class) ends the row.
        let mut scan = Vec::with_capacity(walls.len());
        let mut applicable = true;
        for &w in &walls {
            match margin_at(w)? {
                Some(m) => scan.push(m),
                None => {
                    applicable = false;
                    break;
                }
            }
        }
        let outcome = if !applicable {
            LineWallOutcome::NotEvaluable("criterion is not evaluated for this material class")
        } else {
            let best = scan.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
            let ok: Vec<bool> = scan.iter().map(|m| *m >= req.required_ms).collect();
            match ok.iter().rposition(|passed| !passed) {
                None => LineWallOutcome::BelowSearchFloor { floor_wall: floor, margin: scan[0] },
                Some(k) if k + 1 == walls.len() => LineWallOutcome::Infeasible { largest_wall_tried: ceiling, best_margin_found: best },
                Some(k) => {
                    // Bracket (walls[k] fails, walls[k+1] passes); bisect for the crossing.
                    let (mut lo, mut hi) = (walls[k], walls[k + 1]);
                    let mut hi_margin = scan[k + 1];
                    let mut iterations = 0;
                    while hi - lo > wall_tolerance && iterations < max_iterations {
                        let mid = 0.5 * (lo + hi);
                        let m = margin_at(mid)?.expect("criterion evaluated at the scan samples");
                        if m >= req.required_ms {
                            hi = mid;
                            hi_margin = m;
                        } else {
                            lo = mid;
                        }
                        iterations += 1;
                    }
                    LineWallOutcome::Converged { wall: hi, margin: hi_margin, iterations }
                }
            }
        };
        rows.push(LineWallRow { requirement: req.clone(), outcome });
    }
    Ok(LineWallSolution { rows })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::failure::evaluate_failure_modes;
    use crate::pressure::EndCondition;
    use crate::thickness::{solve_minimum_thickness, ThicknessSolverInputs, ThicknessSolverOutcome};

    const TOL: f64 = 1e-10;

    fn al7075() -> Material {
        *mechanics_core::materials::get_material("al7075")
    }
    fn req(c: ThicknessCriterion, factor: f64, ms: f64) -> CriterionRequirement {
        CriterionRequirement { label: format!("{c:?}"), criterion: c, factor, required_ms: ms }
    }
    fn inputs(end: EndCondition, pi: f64, po: f64, reqs: Vec<CriterionRequirement>) -> PerCriterionInputs {
        PerCriterionInputs { inner_radius: 2.0, pressure: PressureLoading::new(pi, po, end).unwrap(), material: al7075(), unsupported_length: None, requirements: reqs }
    }
    fn wall(o: &CriterionOutcome) -> f64 {
        match o {
            CriterionOutcome::Converged { wall_thickness, .. } => *wall_thickness,
            other => panic!("expected Converged, got {other:?}"),
        }
    }
    fn margin_at(inp: &PerCriterionInputs, r: &CriterionRequirement, ro: f64) -> f64 {
        let g = CylinderGeometry::new(inp.inner_radius, ro).unwrap();
        match probe(r.criterion, &g, inp) {
            Probe::Margin(m) => factored(m, r.factor),
            _ => panic!("no margin"),
        }
    }

    /// Open-end Lame: Tresca stress at the bore is `2 p b^2/(b^2-a^2)`; setting it to
    /// `S = Sy/(f (1+ms))` gives `t = a (sqrt(S/(S-2p)) - 1)`.
    #[test]
    fn tresca_open_end_matches_the_closed_form() {
        let (p, f, ms) = (5000.0, 1.5, 0.1);
        let inp = inputs(EndCondition::Open, p, 0.0, vec![req(ThicknessCriterion::TrescaYield, f, ms)]);
        let s = inp.material.sy_ksi * 1000.0 / (f * (1.0 + ms));
        let expected = 2.0 * ((s / (s - 2.0 * p)).sqrt() - 1.0);
        let sol = solve_per_criterion(&inp, 200, TOL).unwrap();
        assert!((wall(&sol.rows[0].outcome) - expected).abs() < 1e-8, "{} vs {expected}", wall(&sol.rows[0].outcome));
    }

    /// Hoop at the bore is the largest principal for both end conditions: `b/a = sqrt((S+p)/(S-p))`.
    #[test]
    fn max_principal_matches_the_closed_form_for_both_end_conditions() {
        for end in [EndCondition::Open, EndCondition::Closed] {
            for (c, allow) in [(ThicknessCriterion::YieldMaxPrincipal, al7075().sy_ksi), (ThicknessCriterion::UltimateMaxPrincipal, al7075().ftu_ksi)] {
                let (p, f, ms) = (4000.0, 3.0, 0.0);
                let inp = inputs(end, p, 0.0, vec![req(c, f, ms)]);
                let s = allow * 1000.0 / (f * (1.0 + ms));
                let expected = 2.0 * (((s + p) / (s - p)).sqrt() - 1.0);
                let sol = solve_per_criterion(&inp, 200, TOL).unwrap();
                assert!((wall(&sol.rows[0].outcome) - expected).abs() < 1e-8, "{end:?} {c:?}");
            }
        }
    }

    /// Thin-wall limit `t = p a / S` (PD/2t on the bore radius); exact form differs by `x/2`, `x = p/S`.
    #[test]
    fn thin_wall_limit_is_pd_over_2t() {
        let p = 50.0;
        let inp = inputs(EndCondition::Open, p, 0.0, vec![req(ThicknessCriterion::YieldMaxPrincipal, 1.0, 0.0)]);
        let s = inp.material.sy_ksi * 1000.0;
        let t = wall(&solve_per_criterion(&inp, 200, 1e-12).unwrap().rows[0].outcome);
        assert!((t / (p * 2.0 / s) - 1.0).abs() < 1e-3);
    }

    /// Every row re-evaluates to margin = required at its thickness, and a hair thinner fails.
    #[test]
    fn each_thickness_reproduces_its_required_margin_and_is_minimal() {
        use ThicknessCriterion::*;
        let reqs = vec![
            req(YieldMaxPrincipal, 1.5, 0.0),
            req(VonMisesYield, 1.5, 0.05),
            req(TrescaYield, 1.0, 0.2),
            req(UltimateMaxPrincipal, 3.0, 0.0),
            req(UltimateVonMises, 3.0, 0.0),
            req(VonMisesYield, 1.0, 0.0),
        ];
        for end in [EndCondition::Closed, EndCondition::Open] {
            let inp = inputs(end, 3000.0, 100.0, reqs.clone());
            let sol = solve_per_criterion(&inp, 200, TOL).unwrap();
            for row in &sol.rows {
                let CriterionOutcome::Converged { outer_radius, margin, .. } = row.outcome else { panic!("{row:?}") };
                let r = &row.requirement;
                let m = margin_at(&inp, r, outer_radius);
                assert!(m >= r.required_ms, "{}: {m}", r.label);
                assert!((m - margin.unwrap()).abs() < 1e-12);
                assert!(m - r.required_ms < 1e-6, "{}: not tight, {m}", r.label);
                assert!(margin_at(&inp, r, outer_radius - 1e-6) < r.required_ms, "{}: thinner wall passes", r.label);
            }
            // Governing is the maximum of the parts.
            let max = sol.rows.iter().map(|r| wall(&r.outcome)).fold(0.0, f64::max);
            assert_eq!(wall(&sol.governing().unwrap().outcome), max);
        }
    }

    /// With the four existing modes at one common requirement, the thickest per-criterion wall equals
    /// `solve_minimum_thickness` (which stays as it was), and the criterion values match `failure.rs`.
    #[test]
    fn governing_equals_the_existing_whole_set_solver_and_failure_rs_values() {
        use ThicknessCriterion::*;
        let ms = 0.25;
        let pressure = PressureLoading::new(5000.0, 0.0, EndCondition::Closed).unwrap();
        let reqs = vec![req(YieldMaxPrincipal, 1.0, ms), req(VonMisesYield, 1.0, ms), req(TrescaYield, 1.0, ms), req(UltimateMaxPrincipal, 1.0, ms)];
        let inp = PerCriterionInputs { inner_radius: 2.0, pressure, material: al7075(), unsupported_length: None, requirements: reqs };
        let per = solve_per_criterion(&inp, 100, 1e-6).unwrap();
        let whole = solve_minimum_thickness(&ThicknessSolverInputs { inner_radius: 2.0, pressure, material: al7075(), required_minimum_ms: ms }, 100, 1e-6);
        let ThicknessSolverOutcome::Converged(w) = whole else { panic!() };
        assert_eq!(wall(&per.governing().unwrap().outcome), w.wall_thickness);
        let g = CylinderGeometry::new(2.0, 3.0).unwrap();
        let from_failure = evaluate_failure_modes(&g, &pressure, &al7075());
        for (r, f) in reqs_with_failure_order().iter().zip(&from_failure) {
            assert_eq!(margin_at(&inp, &req(*r, 1.0, 0.0), 3.0), f.margin, "{}", f.name);
        }
    }
    fn reqs_with_failure_order() -> [ThicknessCriterion; 4] {
        use ThicknessCriterion::*;
        [YieldMaxPrincipal, VonMisesYield, TrescaYield, UltimateMaxPrincipal]
    }

    /// The monotonicity the bisection relies on, per criterion.
    #[test]
    fn every_criterion_margin_is_non_decreasing_with_wall_thickness() {
        use ThicknessCriterion::*;
        let mut inp = inputs(EndCondition::Closed, 3000.0, 20.0, vec![]);
        inp.inner_radius = 10.0;
        inp.unsupported_length = Some(40.0);
        for c in [YieldMaxPrincipal, VonMisesYield, TrescaYield, UltimateMaxPrincipal, UltimateVonMises, Buckling] {
            let mut last = f64::NEG_INFINITY;
            for i in 1..=400 {
                let g = CylinderGeometry::new(10.0, 10.0 + 0.005 * i as f64).unwrap();
                let m = match probe(c, &g, &inp) {
                    Probe::Margin(m) => m,
                    _ => break, // buckling beyond its validity range
                };
                assert!(m >= last - 1e-12, "{c:?} decreased at step {i}: {last} -> {m}");
                last = m;
            }
        }
    }

    #[test]
    fn buckling_row_reproduces_margin_and_reports_what_is_missing() {
        let mut inp = inputs(EndCondition::Open, 0.0, 15.0, vec![req(ThicknessCriterion::Buckling, 1.5, 0.0)]);
        inp.inner_radius = 10.0;
        // no length given
        assert_eq!(solve_per_criterion(&inp, 200, TOL).unwrap().rows[0].outcome, CriterionOutcome::NotEvaluable("unsupported length not given"));
        inp.unsupported_length = Some(40.0);
        let sol = solve_per_criterion(&inp, 200, TOL).unwrap();
        let CriterionOutcome::Converged { outer_radius, margin, validity_limited, .. } = sol.rows[0].outcome else { panic!() };
        assert!(!validity_limited);
        let m = margin_at(&inp, &inp.requirements[0], outer_radius);
        assert!(m >= 0.0 && m < 1e-6 && (m - margin.unwrap()).abs() < 1e-12);
        assert!(margin_at(&inp, &inp.requirements[0], outer_radius - 1e-6) < 0.0);
        // no external pressure
        inp.pressure = PressureLoading::new(100.0, 0.0, EndCondition::Open).unwrap();
        assert_eq!(solve_per_criterion(&inp, 200, TOL).unwrap().rows[0].outcome, CriterionOutcome::NotEvaluable("no external pressure"));
    }

    #[test]
    fn unreachable_requirement_is_infeasible_and_governs() {
        use ThicknessCriterion::*;
        let inp = inputs(EndCondition::Closed, 5000.0, 0.0, vec![req(VonMisesYield, 1.0, 0.0), req(TrescaYield, 1.0, 1.0e9)]);
        let sol = solve_per_criterion(&inp, 100, 1e-6).unwrap();
        assert!(matches!(sol.rows[1].outcome, CriterionOutcome::Infeasible { .. }));
        assert_eq!(sol.governing().unwrap().requirement.criterion, TrescaYield);
    }

    #[test]
    fn invalid_requirements_are_rejected_not_defaulted() {
        use ThicknessCriterion::*;
        assert_eq!(solve_per_criterion(&inputs(EndCondition::Open, 1.0, 0.0, vec![]), 10, 1e-6), Err(RequirementError::NoRequirements));
        for bad in [req(VonMisesYield, 0.0, 0.0), req(VonMisesYield, -1.0, 0.0), req(VonMisesYield, f64::NAN, 0.0), req(VonMisesYield, 1.0, f64::NAN)] {
            assert_eq!(solve_per_criterion(&inputs(EndCondition::Open, 1.0, 0.0, vec![bad]), 10, 1e-6), Err(RequirementError::InvalidRequirement(0)));
        }
    }

    // ---- line run -----------------------------------------------------------------------------

    use crate::line_run::{Support, Tube};

    fn line() -> LineRun {
        let tube = Tube { outer_diameter: 1.0, wall: 0.05, e: 10.0e6, alpha: 12.8e-6, weight_density: 0.1 };
        let s = |x| Support { x, axial_anchor: false, rotation_fixed: false };
        LineRun { tube, x_start: 0.0, x_end: 100.0, supports: vec![s(0.0), s(100.0)], contents_weight_per_length: 0.05, insulation_weight_per_length: 0.0, point_loads: vec![], subdivisions: 4 }
    }
    fn class() -> MaterialClass {
        MaterialClass::Ductile { yield_strength: 40000.0, ultimate_strength: 60000.0, yield_factor_of_safety: 1.0, ultimate_factor_of_safety: 1.5 }
    }
    fn pressure_case() -> LoadCase {
        LoadCase { name: "p".into(), internal_pressure: 3000.0, external_pressure: 0.0, pressure_end_load: true, delta_t: 0.0, load_factors: [0.0, 0.0, -1.0], point_loads: vec![] }
    }
    fn lreq(c: Criterion, ms: f64) -> LineWallRequirement {
        LineWallRequirement { label: c.label().into(), criterion: c, required_ms: ms }
    }

    #[test]
    fn line_run_walls_reproduce_required_margin_and_are_minimal() {
        let (run, cls, cases) = (line(), class(), [pressure_case()]);
        let reqs = [lreq(Criterion::VonMisesYield, 0.0), lreq(Criterion::TrescaYield, 0.2), lreq(Criterion::UltimateMaxPrincipal, 0.0), lreq(Criterion::UltimateVonMises, 0.0)];
        let tol = 1e-9;
        let sol = solve_line_run_walls(&run, &cls, &cases, &reqs, 200, tol).unwrap();
        for row in &sol.rows {
            let LineWallOutcome::Converged { wall, margin, .. } = row.outcome else { panic!("{row:?}") };
            let c = row.requirement.criterion;
            let m = line_margin(&run, &cls, &cases, c, wall).unwrap().unwrap();
            assert!(m >= row.requirement.required_ms && (m - margin).abs() < 1e-12, "{c:?}: {m}");
            assert!(line_margin(&run, &cls, &cases, c, wall - 1e-6).unwrap().unwrap() < row.requirement.required_ms, "{c:?}: thinner passes");
        }
        let max = sol.rows.iter().map(|r| match r.outcome { LineWallOutcome::Converged { wall, .. } => wall, _ => 0.0 }).fold(0.0, f64::max);
        let LineWallOutcome::Converged { wall, .. } = sol.governing().unwrap().outcome else { panic!() };
        assert_eq!(wall, max);
    }

    /// Pressure-only run with loads fixed: Lame hoop with PD/4t axial, von Mises at the bore against
    /// yield (open weights negligible is not assumed: closed form is checked through `analyze` itself
    /// above); here the outer-diameter convention is checked on the thin limit `t = p r_o / S`.
    #[test]
    fn line_run_thin_limit_is_pd_over_2t_for_hoop_dominated_ultimate() {
        let mut run = line();
        run.tube.weight_density = 0.0;
        run.contents_weight_per_length = 0.0;
        let mut c = pressure_case();
        c.internal_pressure = 50.0;
        c.load_factors = [0.0; 3];
        // Ultimate max principal = hoop; demand = FS * p r_m / t... compare with the exact Lame closed form.
        let cls = class();
        let sol = solve_line_run_walls(&run, &cls, &[c], &[lreq(Criterion::UltimateMaxPrincipal, 0.0)], 200, 1e-12).unwrap();
        let LineWallOutcome::Converged { wall, .. } = sol.rows[0].outcome else { panic!() };
        // FS * p_i * (ro^2 + ri^2)/(ro^2 - ri^2) = Ftu with ri = ro - t; thin: t ~ FS p ro / Ftu.
        let thin = 1.5 * 50.0 * 0.5 / 60000.0;
        assert!((wall / thin - 1.0).abs() < 0.01, "{wall} vs {thin}");
    }

    #[test]
    fn line_run_refuses_unsupported_class_and_marks_missing_criteria() {
        let (run, cases) = (line(), [pressure_case()]);
        assert!(solve_line_run_walls(&run, &MaterialClass::Composite, &cases, &[lreq(Criterion::VonMisesYield, 0.0)], 50, 1e-6).is_err());
        let sol = solve_line_run_walls(&run, &class(), &cases, &[lreq(Criterion::Rankine, 0.0)], 50, 1e-6).unwrap();
        assert!(matches!(sol.rows[0].outcome, LineWallOutcome::NotEvaluable(_)));
        assert!(solve_line_run_walls(&run, &class(), &cases, &[], 50, 1e-6).is_err());
    }
}
