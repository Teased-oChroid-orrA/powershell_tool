//! Pass/fail evaluation of the Bushing Workbench results and *verified*
//! fix recommendations for whatever fails.
//!
//! Every recommendation is produced by trial-solving a copy of the model
//! through the real solver (`BushingModel::trial`): the proposed value is
//! found by bisecting from the current value toward a limit, then checked to
//! (a) clear the failing check and (b) introduce no check that was not
//! already failing. Nothing here does engineering math of its own - the
//! solver stays the single authority (`bushing-solver/AGENTS.md`).
//!
//! `evaluate` is also what the Results pane reads to highlight a failing
//! check's name and value.

use bushing_solver::countersink::CsMode;
use bushing_solver::geometry::{BushingType, IdType};
use bushing_solver::tolerance::ToleranceStatus;

use super::model::{BushingModel, FitType, NumberTarget};

/// Below this margin of safety a passing check is flagged as a warning.
pub const WARN_MARGIN: f64 = 0.15;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Pass,
    Warn,
    Fail,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CheckKind {
    Tolerance,
    StraightWall,
    NeckWall,
    HousingStress,
    BushingStress,
    EdgeSequencing,
    EdgeStrength,
    Enforcement,
    FitType,
}

impl CheckKind {
    pub fn label(self) -> &'static str {
        match self {
            CheckKind::Tolerance => "Tolerance",
            CheckKind::StraightWall => "Straight wall",
            CheckKind::NeckWall => "Neck wall",
            CheckKind::HousingStress => "Housing stress",
            CheckKind::BushingStress => "Bushing stress",
            CheckKind::EdgeSequencing => "Edge distance (sequencing)",
            CheckKind::EdgeStrength => "Edge distance (strength)",
            CheckKind::Enforcement => "Tolerance enforcement",
            CheckKind::FitType => "Fit type",
        }
    }

    /// The solver's own candidate name for the checks that appear in its
    /// governing-margin list (used to highlight that list's rows).
    pub fn from_candidate_name(name: &str) -> Option<CheckKind> {
        match name {
            "Edge distance (sequencing)" => Some(CheckKind::EdgeSequencing),
            "Edge distance (strength)" => Some(CheckKind::EdgeStrength),
            "Straight wall thickness" => Some(CheckKind::StraightWall),
            "Neck wall thickness" => Some(CheckKind::NeckWall),
            _ => None,
        }
    }

    /// Input rows that directly drive this check - highlighted in the
    /// Inputs list while the check is failing.
    pub fn related_inputs(self) -> &'static [NumberTarget] {
        match self {
            CheckKind::Tolerance => &[NumberTarget::BoreTolPlus, NumberTarget::BoreTolMinus, NumberTarget::InterferenceTolPlus, NumberTarget::InterferenceTolMinus],
            CheckKind::StraightWall => &[NumberTarget::MinWallStraight, NumberTarget::IdBushing],
            CheckKind::NeckWall => &[NumberTarget::MinWallNeck, NumberTarget::IdBushing],
            CheckKind::HousingStress | CheckKind::BushingStress => &[NumberTarget::Interference],
            CheckKind::EdgeSequencing => &[NumberTarget::EdgeDist],
            CheckKind::EdgeStrength => &[NumberTarget::EdgeDist, NumberTarget::Load],
            CheckKind::Enforcement => &[NumberTarget::MaxBoreNominalShift, NumberTarget::BoreCapabilityMinWidth],
            CheckKind::FitType => &[NumberTarget::Interference],
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Check {
    pub kind: CheckKind,
    pub severity: Severity,
    /// One-line human result, e.g. `0.0420 in < min 0.0500 in`.
    pub detail: String,
}

fn margin_severity(margin: f64) -> Severity {
    if !margin.is_finite() || margin >= WARN_MARGIN {
        Severity::Pass
    } else if margin < 0.0 {
        Severity::Fail
    } else {
        Severity::Warn
    }
}

/// Every check, in display order, with its current severity. Only the
/// non-passing ones are interesting to most callers (`failing`).
pub fn evaluate(model: &BushingModel) -> Vec<Check> {
    let out = &model.output;
    let mut checks = Vec::new();

    let (sev, detail) = match out.tolerance_status {
        ToleranceStatus::Ok => (Severity::Pass, "OK".to_string()),
        ToleranceStatus::Clamped => (Severity::Warn, "OD nominal clamped into the interference window".to_string()),
        ToleranceStatus::Infeasible => (Severity::Fail, "bore tolerance wider than interference tolerance".to_string()),
    };
    checks.push(Check { kind: CheckKind::Tolerance, severity: sev, detail });

    checks.push(Check {
        kind: CheckKind::StraightWall,
        severity: if out.fail_straight { Severity::Fail } else { Severity::Pass },
        detail: format!("{:.4} in vs min {:.4} in", out.wall_straight, model.min_wall_straight),
    });
    checks.push(Check {
        kind: CheckKind::NeckWall,
        severity: if out.fail_neck { Severity::Fail } else { Severity::Pass },
        detail: format!("{:.4} in vs min {:.4} in", out.wall_neck, model.min_wall_neck),
    });
    checks.push(Check { kind: CheckKind::HousingStress, severity: margin_severity(out.housing_ms), detail: format!("MS {:+.2}", out.housing_ms) });
    checks.push(Check { kind: CheckKind::BushingStress, severity: margin_severity(out.bushing_ms), detail: format!("MS {:+.2}", out.bushing_ms) });
    checks.push(Check { kind: CheckKind::EdgeSequencing, severity: margin_severity(out.sequence_margin), detail: format!("margin {:+.2}", out.sequence_margin) });
    checks.push(Check { kind: CheckKind::EdgeStrength, severity: margin_severity(out.strength_margin), detail: format!("margin {:+.2}", out.strength_margin) });

    if model.enforcement_enabled {
        checks.push(Check {
            kind: CheckKind::Enforcement,
            severity: if out.enforcement_satisfied { Severity::Pass } else { Severity::Fail },
            detail: if out.enforcement_satisfied { "satisfied".to_string() } else { "could not reach a feasible band".to_string() },
        });
    }

    let mismatch = match model.fit_type {
        FitType::Clearance | FitType::Slip => model.interference > 0.0,
        FitType::Press | FitType::Shrink => model.interference <= 0.0,
    };
    if mismatch {
        checks.push(Check {
            kind: CheckKind::FitType,
            severity: Severity::Warn,
            detail: format!("{} with Target Interference {:.4} in", super::model::label_fit_type(model.fit_type), model.interference),
        });
    }
    checks
}

pub fn severity_of(checks: &[Check], kind: CheckKind) -> Severity {
    checks.iter().find(|c| c.kind == kind).map(|c| c.severity).unwrap_or(Severity::Pass)
}

#[derive(Debug, Clone, PartialEq)]
pub struct Edit {
    pub target: NumberTarget,
    pub value: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Recommendation {
    pub fixes: CheckKind,
    /// `Bushing ID 0.3750 → 0.3200 in`
    pub summary: String,
    /// What the solver reports with the edits applied.
    pub outcome: String,
    /// Empty = advice only (nothing safe to auto-apply).
    pub edits: Vec<Edit>,
}

impl Recommendation {
    pub fn is_applicable(&self) -> bool {
        !self.edits.is_empty()
    }
}

fn failing_kinds(model: &BushingModel) -> Vec<CheckKind> {
    evaluate(model).into_iter().filter(|c| c.severity == Severity::Fail).map(|c| c.kind).collect()
}

fn step_for(target: NumberTarget) -> f64 {
    if target.is_angle() {
        0.1
    } else if matches!(target, NumberTarget::Load) {
        1.0
    } else {
        0.0001
    }
}

/// Finds a value for `target`, between its current value and `limit`, that
/// makes `kind` pass without breaking any other check. Bisects toward the
/// current value (so the proposal is the smallest change found), then rounds
/// to the input's natural resolution toward the passing side and re-verifies.
/// `None` when even `limit` does not fix it.
fn smallest_fix(model: &BushingModel, base_fails: &[CheckKind], kind: CheckKind, target: NumberTarget, limit: f64) -> Option<f64> {
    // Prefer a fix that leaves the check comfortably passing (no warning);
    // fall back to one that merely clears the failure if that is all the
    // allowed range can offer.
    bisect_fix(model, base_fails, kind, target, limit, true).or_else(|| bisect_fix(model, base_fails, kind, target, limit, false))
}

fn bisect_fix(model: &BushingModel, base_fails: &[CheckKind], kind: CheckKind, target: NumberTarget, limit: f64, strict: bool) -> Option<f64> {
    let v0 = model.number_value(target);
    if !limit.is_finite() || (limit - v0).abs() < 1e-12 {
        return None;
    }
    let good = |v: f64| {
        let trial = model.trial(&[Edit { target, value: v }]);
        let checks = evaluate(&trial);
        let own = severity_of(&checks, kind);
        let cleared = if strict { own == Severity::Pass } else { own != Severity::Fail };
        cleared && checks.iter().all(|c| c.severity != Severity::Fail || base_fails.contains(&c.kind))
    };
    if !good(limit) {
        return None;
    }
    let dir = (limit - v0).signum();
    let (mut bad, mut ok) = (v0, limit);
    for _ in 0..40 {
        let mid = (bad + ok) / 2.0;
        if good(mid) {
            ok = mid;
        } else {
            bad = mid;
        }
    }
    let step = step_for(target);
    let mut v = if dir > 0.0 { (ok / step).ceil() * step } else { (ok / step).floor() * step };
    for _ in 0..4 {
        if good(v) {
            // Round away float fuzz (1.0389000000000002 -> 1.0389) without changing the verified value.
            let clean = ((v / step).round() * step * 1e6).round() / 1e6;
            return Some(if good(clean) { clean } else { v });
        }
        v += dir * step;
    }
    None
}

fn describe(model: &BushingModel, edits: &[Edit]) -> String {
    edits
        .iter()
        .map(|e| format!("{} {} \u{2192} {}", e.target.label(), e.target.format_value(model.number_value(e.target)), e.target.format_value(e.value)))
        .collect::<Vec<_>>()
        .join(", ")
}

fn outcome_of(model: &BushingModel, edits: &[Edit], kind: CheckKind) -> String {
    let trial = model.trial(edits);
    let checks = evaluate(&trial);
    let detail = checks.iter().find(|c| c.kind == kind).map(|c| c.detail.clone()).unwrap_or_default();
    format!("{}: {detail}", kind.label())
}

/// Verified fixes for every failing check (plus fit-type mismatch warnings),
/// most important first. Cheap enough to run on every recompute: each
/// recommendation costs ~45 solver evaluations.
pub fn recommend(model: &BushingModel) -> Vec<Recommendation> {
    let checks = evaluate(model);
    let base_fails: Vec<CheckKind> = checks.iter().filter(|c| c.severity == Severity::Fail).map(|c| c.kind).collect();
    let mut recs: Vec<Recommendation> = Vec::new();

    let push_edits = |recs: &mut Vec<Recommendation>, kind: CheckKind, edits: Vec<Edit>| {
        if edits.is_empty() {
            return;
        }
        let rec = Recommendation { fixes: kind, summary: describe(model, &edits), outcome: outcome_of(model, &edits, kind), edits };
        if !recs.iter().any(|r| r.edits == rec.edits) {
            recs.push(rec);
        }
    };
    let single = |kind: CheckKind, target: NumberTarget, limit: f64| -> Vec<Edit> {
        smallest_fix(model, &base_fails, kind, target, limit).map(|value| vec![Edit { target, value }]).unwrap_or_default()
    };

    for kind in &base_fails {
        let kind = *kind;
        match kind {
            CheckKind::Tolerance => {
                // Widen the interference band to contain the bore band...
                let widen = vec![
                    Edit { target: NumberTarget::InterferenceTolPlus, value: model.interference_tol_plus.max(model.bore_tol_plus) },
                    Edit { target: NumberTarget::InterferenceTolMinus, value: model.interference_tol_minus.max(model.bore_tol_minus) },
                ];
                if verified(model, &base_fails, kind, &widen) {
                    push_edits(&mut recs, kind, widen);
                }
                // ...or tighten the bore band to fit inside the interference band.
                let tighten = vec![
                    Edit { target: NumberTarget::BoreTolPlus, value: model.bore_tol_plus.min(model.interference_tol_plus) },
                    Edit { target: NumberTarget::BoreTolMinus, value: model.bore_tol_minus.min(model.interference_tol_minus) },
                ];
                if verified(model, &base_fails, kind, &tighten) {
                    push_edits(&mut recs, kind, tighten);
                }
            }
            CheckKind::StraightWall => {
                push_edits(&mut recs, kind, single(kind, NumberTarget::IdBushing, 0.01));
                let floor = (model.output.wall_straight / 0.0001).floor() * 0.0001;
                if floor > 0.0 {
                    let relax = vec![Edit { target: NumberTarget::MinWallStraight, value: floor }];
                    if verified(model, &base_fails, kind, &relax) {
                        push_edits(&mut recs, kind, relax);
                    }
                }
            }
            CheckKind::NeckWall => {
                push_edits(&mut recs, kind, single(kind, NumberTarget::IdBushing, 0.01));
                if model.id_type == IdType::Countersink {
                    if model.cs_mode != CsMode::DepthAngle {
                        push_edits(&mut recs, kind, single(kind, NumberTarget::CsDia, 0.0));
                    }
                    if model.cs_mode != CsMode::DiaAngle {
                        push_edits(&mut recs, kind, single(kind, NumberTarget::CsDepth, 0.0));
                    }
                }
                if model.bushing_type == BushingType::Countersink {
                    if model.ext_cs_mode != CsMode::DepthAngle {
                        push_edits(&mut recs, kind, single(kind, NumberTarget::ExtCsDia, 0.0));
                    }
                    if model.ext_cs_mode != CsMode::DiaAngle {
                        push_edits(&mut recs, kind, single(kind, NumberTarget::ExtCsDepth, 0.0));
                    }
                }
                let floor = (model.output.wall_neck / 0.0001).floor() * 0.0001;
                if floor > 0.0 {
                    let relax = vec![Edit { target: NumberTarget::MinWallNeck, value: floor }];
                    if verified(model, &base_fails, kind, &relax) {
                        push_edits(&mut recs, kind, relax);
                    }
                }
            }
            CheckKind::HousingStress => {
                push_edits(&mut recs, kind, single(kind, NumberTarget::Interference, 0.0001));
                push_edits(&mut recs, kind, single(kind, NumberTarget::HousingWidth, model.housing_width * 4.0));
            }
            CheckKind::BushingStress => {
                push_edits(&mut recs, kind, single(kind, NumberTarget::Interference, 0.0001));
            }
            CheckKind::EdgeSequencing => {
                push_edits(&mut recs, kind, single(kind, NumberTarget::EdgeDist, (model.edge_dist * 5.0).max(model.bore_dia * 10.0)));
            }
            CheckKind::EdgeStrength => {
                push_edits(&mut recs, kind, single(kind, NumberTarget::EdgeDist, (model.edge_dist * 5.0).max(model.bore_dia * 10.0)));
                push_edits(&mut recs, kind, single(kind, NumberTarget::Load, 0.0));
            }
            CheckKind::Enforcement => {
                recs.push(Recommendation {
                    fixes: kind,
                    summary: "Enable Allow Bore Nominal Shift (and unlock the bore), or widen the interference tolerance".to_string(),
                    outcome: "manual change - toggles are not auto-applied".to_string(),
                    edits: Vec::new(),
                });
            }
            CheckKind::FitType => {}
        }
    }

    if checks.iter().any(|c| c.kind == CheckKind::FitType) {
        let (interference, tol_plus, tol_minus) = model.fit_type_preset();
        let edits = vec![
            Edit { target: NumberTarget::Interference, value: interference },
            Edit { target: NumberTarget::InterferenceTolPlus, value: tol_plus },
            Edit { target: NumberTarget::InterferenceTolMinus, value: tol_minus },
        ];
        recs.push(Recommendation {
            fixes: CheckKind::FitType,
            summary: describe(model, &edits),
            outcome: format!("typical values for a {}", super::model::label_fit_type(model.fit_type)),
            edits,
        });
    }
    recs
}

fn verified(model: &BushingModel, base_fails: &[CheckKind], kind: CheckKind, edits: &[Edit]) -> bool {
    if edits.iter().all(|e| (model.number_value(e.target) - e.value).abs() < 1e-12) {
        return false;
    }
    let fails = failing_kinds(&model.trial(edits));
    !fails.contains(&kind) && fails.iter().all(|k| base_fails.contains(k))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model() -> BushingModel {
        BushingModel::default()
    }

    #[test]
    fn default_model_fails_only_edge_sequencing_and_offers_a_verified_fix() {
        let m = model();
        let failing: Vec<_> = m.checks.iter().filter(|c| c.severity == Severity::Fail).map(|c| c.kind).collect();
        assert_eq!(failing, vec![CheckKind::EdgeSequencing], "{:?}", m.checks);
        let mut applied = m.clone();
        applied.apply_recommendation(0).expect("applicable");
        assert!(applied.checks.iter().all(|c| c.severity != Severity::Fail), "{:?}", applied.checks);
    }

    #[test]
    fn thin_straight_wall_is_flagged_and_the_fix_clears_it() {
        let mut m = model();
        m.commit_number(NumberTarget::MinWallStraight, 0.2);
        assert_eq!(severity_of(&m.checks, CheckKind::StraightWall), Severity::Fail);
        let rec = m.recommendations.iter().find(|r| r.fixes == CheckKind::StraightWall && r.edits[0].target == NumberTarget::IdBushing).expect("ID reduction offered");
        let mut applied = m.clone();
        for e in &rec.edits {
            applied.commit_number(e.target, e.value);
        }
        assert_eq!(severity_of(&applied.checks, CheckKind::StraightWall), Severity::Pass);
        assert!(!applied.output.fail_straight);
        assert!(applied.id_bushing < m.id_bushing, "must reduce the ID, not grow it");
    }

    #[test]
    fn infeasible_tolerance_offers_a_widen_fix_that_makes_it_feasible() {
        let mut m = model();
        m.commit_number(NumberTarget::BoreTolPlus, 0.002);
        m.commit_number(NumberTarget::BoreTolMinus, 0.001);
        assert_eq!(severity_of(&m.checks, CheckKind::Tolerance), Severity::Fail);
        let rec = m.recommendations.iter().find(|r| r.fixes == CheckKind::Tolerance).expect("tolerance recommendation");
        let mut applied = m.clone();
        for e in &rec.edits {
            applied.commit_number(e.target, e.value);
        }
        assert_ne!(severity_of(&applied.checks, CheckKind::Tolerance), Severity::Fail);
    }

    #[test]
    fn short_edge_distance_is_fixed_by_increasing_it() {
        let mut m = model();
        m.commit_number(NumberTarget::EdgeDist, 0.2);
        assert_eq!(severity_of(&m.checks, CheckKind::EdgeSequencing), Severity::Fail);
        let rec = m.recommendations.iter().find(|r| r.edits.iter().any(|e| e.target == NumberTarget::EdgeDist)).expect("edge distance fix");
        assert!(rec.edits[0].value > 0.2);
        let mut applied = m.clone();
        applied.apply_recommendation(m.recommendations.iter().position(|r| r == rec).unwrap());
        assert_ne!(severity_of(&applied.checks, CheckKind::EdgeSequencing), Severity::Fail);
    }

    #[test]
    fn recommendations_never_introduce_a_new_failure() {
        let mut m = model();
        m.commit_number(NumberTarget::MinWallStraight, 0.2);
        let base: Vec<_> = failing_kinds(&m);
        for rec in &m.recommendations {
            let trial = m.trial(&rec.edits);
            assert!(failing_kinds(&trial).iter().all(|k| base.contains(k)), "{rec:?} created a new failure");
        }
    }

    #[test]
    fn candidate_names_map_to_checks() {
        assert_eq!(CheckKind::from_candidate_name("Straight wall thickness"), Some(CheckKind::StraightWall));
        assert_eq!(CheckKind::from_candidate_name("nonsense"), None);
    }
}
