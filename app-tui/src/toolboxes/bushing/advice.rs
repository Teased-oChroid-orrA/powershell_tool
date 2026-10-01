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
    /// Side effects of applying it, from solving a copy of the model: the
    /// bore line first, then the metrics that move the most, then any other
    /// check that changes status.
    pub impact: Vec<String>,
    /// E.g. which catalog reamer/drill the value was snapped to.
    pub note: Option<String>,
    /// True when the housing bore itself changes (a different reamer).
    pub touches_bore: bool,
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

/// A proposed set of edits plus an explanatory note.
struct Fix {
    edits: Vec<Edit>,
    note: Option<String>,
}

/// Targets whose value is a drilled/reamed size: a recommendation for them is
/// snapped to a real catalog reamer/drill so no new tool has to be made.
fn is_tooled(target: NumberTarget) -> bool {
    matches!(target, NumberTarget::IdBushing | NumberTarget::BoreDia)
}

/// Edits that set a tooled dimension to catalog size `size` (a bore also
/// takes the tool's own tolerance, like picking it in the reamer picker).
fn edits_for_size(model: &BushingModel, target: NumberTarget, size: &super::model::CatalogSize) -> Vec<Edit> {
    if target == NumberTarget::BoreDia {
        // The reamer's own tolerance becomes the bore band, so the interference
        // window must cover it on each side or the new bore would make the fit
        // infeasible/clamped.
        let mut edits = vec![
            Edit { target, value: size.nominal },
            Edit { target: NumberTarget::BoreTolPlus, value: size.tol_plus },
            Edit { target: NumberTarget::BoreTolMinus, value: size.tol_minus },
        ];
        let want_minus = model.interference_tol_minus.max(size.tol_plus);
        let want_plus = model.interference_tol_plus.max(size.tol_minus);
        if want_minus > model.interference_tol_minus + 1e-12 {
            edits.push(Edit { target: NumberTarget::InterferenceTolMinus, value: want_minus });
        }
        if want_plus > model.interference_tol_plus + 1e-12 {
            edits.push(Edit { target: NumberTarget::InterferenceTolPlus, value: want_plus });
        }
        edits
    } else {
        vec![Edit { target, value: size.nominal }]
    }
}

/// Finds a value for `target`, between its current value and `limit`, that
/// makes `kind` pass without breaking any other check. Bisects toward the
/// current value (so the proposal is the smallest change found); for a
/// drilled/reamed dimension it then moves to the nearest *catalog* size on
/// the passing side, so nothing new has to be fabricated.
fn smallest_fix(model: &BushingModel, base_fails: &[CheckKind], kind: CheckKind, target: NumberTarget, limit: f64) -> Option<Fix> {
    // Prefer a fix that leaves the check comfortably passing (no warning);
    // fall back to one that merely clears the failure if that is all the
    // allowed range can offer.
    bisect_fix(model, base_fails, kind, target, limit, true).or_else(|| bisect_fix(model, base_fails, kind, target, limit, false))
}

fn bisect_fix(model: &BushingModel, base_fails: &[CheckKind], kind: CheckKind, target: NumberTarget, limit: f64, strict: bool) -> Option<Fix> {
    let v0 = model.number_value(target);
    if !limit.is_finite() || (limit - v0).abs() < 1e-12 {
        return None;
    }
    let good = |edits: &[Edit]| {
        let trial = model.trial(edits);
        let checks = evaluate(&trial);
        let own = severity_of(&checks, kind);
        let cleared = if strict { own == Severity::Pass } else { own != Severity::Fail };
        cleared && checks.iter().all(|c| c.severity != Severity::Fail || base_fails.contains(&c.kind))
    };
    let one = |v: f64| [Edit { target, value: v }];
    // The far limit may overshoot (e.g. a bore so small the wall vanishes):
    // pull it back toward the current value until something passes. The
    // snap-to-catalog step below keeps the search inside [v0, limit].
    let far = limit;
    let limit = [1.0, 0.75, 0.5, 0.35, 0.2, 0.1, 0.05].iter().map(|f| v0 + (far - v0) * f).find(|v| good(&one(*v)))?;
    let dir = (limit - v0).signum();
    let (mut bad, mut ok) = (v0, limit);
    for _ in 0..40 {
        let mid = (bad + ok) / 2.0;
        if good(&one(mid)) {
            ok = mid;
        } else {
            bad = mid;
        }
    }

    if is_tooled(target) {
        let (lo, hi) = (v0.min(limit), v0.max(limit));
        let mut sizes: Vec<super::model::CatalogSize> = model
            .catalog_sizes()
            .into_iter()
            .filter(|c| c.nominal > lo + 1e-9 && c.nominal <= hi + 1e-9)
            // A bore is reamed, never drilled: drills are for finished IDs only.
            .filter(|c| !(target == NumberTarget::BoreDia && c.source == "drill"))
            // On the passing side of the boundary found above.
            .filter(|c| if dir > 0.0 { c.nominal >= ok - 1e-9 } else { c.nominal <= ok + 1e-9 })
            .collect();
        // Closest to the boundary first = the smallest change that is a real tool.
        sizes.sort_by(|a, b| (a.nominal - ok).abs().partial_cmp(&(b.nominal - ok).abs()).unwrap_or(std::cmp::Ordering::Equal));
        for size in sizes.into_iter().take(8) {
            let edits = edits_for_size(model, target, &size);
            if good(&edits) {
                return Some(Fix { edits, note: Some(format!(
                        "snapped to {}{} size {} ({:.4} in) - no new tooling needed",
                        if size.common { "common " } else { "" },
                        size.source,
                        size.label,
                        size.nominal
                    )) });
            }
        }
    }

    let step = step_for(target);
    let mut v = if dir > 0.0 { (ok / step).ceil() * step } else { (ok / step).floor() * step };
    for _ in 0..4 {
        if good(&one(v)) {
            // Round away float fuzz (1.0389000000000002 -> 1.0389) without changing the verified value.
            let clean = ((v / step).round() * step * 1e6).round() / 1e6;
            let value = if good(&one(clean)) { clean } else { v };
            let note = is_tooled(target).then(|| "no catalog reamer/drill size passes - this needs a custom-size tool".to_string());
            return Some(Fix { edits: vec![Edit { target, value }], note });
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

/// What applying `edits` does besides fixing the check, from solving a copy:
/// bore change, the results that move the most, and checks that change status.
fn impact_of(model: &BushingModel, edits: &[Edit], fixed: CheckKind) -> Vec<String> {
    let after = model.trial(edits);
    let (a, b) = (&model.output, &after.output);
    let mut lines = Vec::new();

    if let Some(bore) = edits.iter().find(|e| e.target == NumberTarget::BoreDia) {
        lines.push(format!("Bore {:.4} \u{2192} {:.4} in - needs a different reamer ({})", model.bore_dia, bore.value, if bore.value < model.bore_dia { "smaller, never enlarged" } else { "LARGER" }));
    } else {
        lines.push("Bore unchanged - no re-reaming".to_string());
    }

    // (label, before, after, decimals, unit, is a margin - compared by difference)
    let metrics: [(&str, f64, f64, usize, &str, bool); 10] = [
        ("Contact pressure", a.pressure, b.pressure, 0, "psi", false),
        ("Install force", a.install_force, b.install_force, 1, "lbf", false),
        ("Retained force", a.retained_install_force, b.retained_install_force, 1, "lbf", false),
        ("OD installed", a.od_installed, b.od_installed, 4, "in", false),
        ("Straight wall", a.wall_straight, b.wall_straight, 4, "in", false),
        ("Neck wall", a.wall_neck, b.wall_neck, 4, "in", false),
        ("Housing stress MS", a.housing_ms, b.housing_ms, 2, "", true),
        ("Bushing stress MS", a.bushing_ms, b.bushing_ms, 2, "", true),
        ("Edge sequencing margin", a.sequence_margin, b.sequence_margin, 2, "", true),
        ("Edge strength margin", a.strength_margin, b.strength_margin, 2, "", true),
    ];
    let mut moved: Vec<(f64, String)> = Vec::new();
    for (label, before, now, dp, unit, is_margin) in metrics {
        if !before.is_finite() || !now.is_finite() {
            continue;
        }
        let (score, text) = if is_margin {
            let d = now - before;
            if d.abs() < 0.01 {
                continue;
            }
            (d.abs(), format!("{label} {before:+.dp$} \u{2192} {now:+.dp$} ({d:+.2})"))
        } else {
            let rel = if before.abs() > 1e-12 { (now - before) / before.abs() } else { 0.0 };
            if rel.abs() < 0.005 {
                continue;
            }
            (rel.abs(), format!("{label} {before:.dp$} \u{2192} {now:.dp$} {unit} ({:+.0}%)", rel * 100.0))
        };
        moved.push((score, text));
    }
    moved.sort_by(|x, y| y.0.partial_cmp(&x.0).unwrap_or(std::cmp::Ordering::Equal));
    lines.extend(moved.into_iter().take(5).map(|(_, t)| t));

    let (before_checks, after_checks) = (evaluate(model), evaluate(&after));
    for c in &after_checks {
        let was = severity_of(&before_checks, c.kind);
        if c.kind != fixed && c.severity > was {
            lines.push(format!("Warning: {} goes {:?} \u{2192} {:?}", c.kind.label(), was, c.severity));
        }
    }
    lines
}

/// Verified fixes for every failing check (plus fit-type mismatch warnings),
/// most important first. Several alternatives are offered per failure -
/// including ones that leave the housing bore alone, since a bore usually
/// cannot be enlarged - each with its impact. The bore is only ever
/// proposed smaller, and only to a real catalog reamer. Cheap enough to run
/// on every recompute: each recommendation costs ~45 solver evaluations.
pub fn recommend(model: &BushingModel) -> Vec<Recommendation> {
    let checks = evaluate(model);
    let base_fails: Vec<CheckKind> = checks.iter().filter(|c| c.severity == Severity::Fail).map(|c| c.kind).collect();
    let mut recs: Vec<Recommendation> = Vec::new();

    let push_fix = |recs: &mut Vec<Recommendation>, kind: CheckKind, fix: Option<Fix>| {
        let Some(Fix { edits, note }) = fix else { return };
        if edits.is_empty() || recs.iter().any(|r| r.edits == edits) {
            return;
        }
        recs.push(Recommendation {
            fixes: kind,
            summary: describe(model, &edits),
            outcome: outcome_of(model, &edits, kind),
            impact: impact_of(model, &edits, kind),
            touches_bore: edits.iter().any(|e| e.target == NumberTarget::BoreDia),
            note,
            edits,
        });
    };
    let direct = |edits: Vec<Edit>| Some(Fix { edits, note: None });
    let fix_by = |kind: CheckKind, target: NumberTarget, limit: f64| smallest_fix(model, &base_fails, kind, target, limit);
    // More interference = a bigger OD (thicker wall) without touching the bore.
    let more_interference = model.interference + (model.interference.abs() * 3.0).max(0.004);
    // A smaller bore needs a smaller reamer; never proposed larger.
    let smaller_bore = model.bore_dia * 0.5;

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
                    push_fix(&mut recs, kind, direct(widen));
                }
                // ...or tighten the bore band to fit inside the interference band.
                let tighten = vec![
                    Edit { target: NumberTarget::BoreTolPlus, value: model.bore_tol_plus.min(model.interference_tol_plus) },
                    Edit { target: NumberTarget::BoreTolMinus, value: model.bore_tol_minus.min(model.interference_tol_minus) },
                ];
                if verified(model, &base_fails, kind, &tighten) {
                    push_fix(&mut recs, kind, direct(tighten));
                }
            }
            CheckKind::StraightWall | CheckKind::NeckWall => {
                let (wall, min_target) = if kind == CheckKind::StraightWall { (model.output.wall_straight, NumberTarget::MinWallStraight) } else { (model.output.wall_neck, NumberTarget::MinWallNeck) };
                push_fix(&mut recs, kind, fix_by(kind, NumberTarget::IdBushing, 0.01));
                push_fix(&mut recs, kind, fix_by(kind, NumberTarget::Interference, more_interference));
                if kind == CheckKind::NeckWall {
                    if model.id_type == IdType::Countersink {
                        if model.cs_mode != CsMode::DepthAngle {
                            push_fix(&mut recs, kind, fix_by(kind, NumberTarget::CsDia, 0.0));
                        }
                        if model.cs_mode != CsMode::DiaAngle {
                            push_fix(&mut recs, kind, fix_by(kind, NumberTarget::CsDepth, 0.0));
                        }
                    }
                    if model.bushing_type == BushingType::Countersink {
                        if model.ext_cs_mode != CsMode::DepthAngle {
                            push_fix(&mut recs, kind, fix_by(kind, NumberTarget::ExtCsDia, 0.0));
                        }
                        if model.ext_cs_mode != CsMode::DiaAngle {
                            push_fix(&mut recs, kind, fix_by(kind, NumberTarget::ExtCsDepth, 0.0));
                        }
                    }
                }
                let floor = (wall / 0.0001).floor() * 0.0001;
                if floor > 0.0 {
                    let relax = vec![Edit { target: min_target, value: floor }];
                    if verified(model, &base_fails, kind, &relax) {
                        push_fix(&mut recs, kind, direct(relax));
                    }
                }
            }
            CheckKind::HousingStress => {
                push_fix(&mut recs, kind, fix_by(kind, NumberTarget::Interference, 0.0001));
                push_fix(&mut recs, kind, fix_by(kind, NumberTarget::HousingWidth, model.housing_width * 4.0));
            }
            CheckKind::BushingStress => {
                push_fix(&mut recs, kind, fix_by(kind, NumberTarget::Interference, 0.0001));
                push_fix(&mut recs, kind, fix_by(kind, NumberTarget::IdBushing, 0.01));
            }
            CheckKind::EdgeSequencing | CheckKind::EdgeStrength => {
                let far = (model.edge_dist * 5.0).max(model.bore_dia * 10.0);
                push_fix(&mut recs, kind, fix_by(kind, NumberTarget::EdgeDist, far));
                if kind == CheckKind::EdgeStrength {
                    push_fix(&mut recs, kind, fix_by(kind, NumberTarget::Load, 0.0));
                }
                push_fix(&mut recs, kind, fix_by(kind, NumberTarget::BoreDia, smaller_bore));
            }
            CheckKind::Enforcement => {
                recs.push(Recommendation {
                    fixes: kind,
                    summary: "Enable Allow Bore Nominal Shift (and unlock the bore), or widen the interference tolerance".to_string(),
                    outcome: "manual change - toggles are not auto-applied".to_string(),
                    edits: Vec::new(),
                    impact: Vec::new(),
                    note: None,
                    touches_bore: false,
                });
            }
            CheckKind::FitType => {}
        }
    }

    // OD clamped (a warning, not a failure): make each side of the interference
    // window cover the matching side of the bore band so the OD can be centred.
    if severity_of(&checks, CheckKind::Tolerance) == Severity::Warn {
        let edits = vec![
            Edit { target: NumberTarget::InterferenceTolPlus, value: model.interference_tol_plus.max(model.bore_tol_minus) },
            Edit { target: NumberTarget::InterferenceTolMinus, value: model.interference_tol_minus.max(model.bore_tol_plus) },
        ];
        let changes = edits.iter().any(|e| (model.number_value(e.target) - e.value).abs() > 1e-12);
        if changes && model.trial(&edits).output.tolerance_status == ToleranceStatus::Ok {
            push_fix(&mut recs, CheckKind::Tolerance, direct(edits));
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
            impact: impact_of(model, &edits, CheckKind::FitType),
            note: None,
            touches_bore: false,
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

/// Plain-language explanation of why the OD tolerance came out Clamped (or
/// Infeasible), built from the actual numbers in the current model - the
/// same quantities `bushing_solver::tolerance::build_od_tolerance` compares.
/// `None` when the tolerance status is Ok. The result alternates heading,
/// body, heading, body... (the Fixes window relies on that pairing).
///
/// The rule it explains: the OD must keep the *achieved* interference inside
/// the interference window at every bore-tolerance extreme, so the OD
/// nominal may only lie in `[bore max + interference min, bore min +
/// interference max]`. Wanting `bore nominal + target interference` outside
/// that range (an asymmetric bore band vs. interference band) forces the OD
/// nominal to be pulled to the nearest edge: Clamped. A bore band wider than
/// the whole interference band leaves no range at all: Infeasible.
pub fn explain_tolerance(model: &BushingModel) -> Option<Vec<String>> {
    let out = &model.output;
    if out.tolerance_status == ToleranceStatus::Ok {
        return None;
    }
    let (bore, int, od) = (&out.bore_tol, &out.interference_tol, &out.od_tol);
    let f = |v: f64| format!("{v:.4}");
    let desired = bore.nominal + int.nominal;
    let req_lo = bore.upper + int.lower;
    let req_hi = bore.lower + int.upper;
    let bore_plus = bore.upper - bore.nominal;
    let bore_minus = bore.nominal - bore.lower;
    let int_plus = int.upper - int.nominal;
    let int_minus = int.nominal - int.lower;
    let bore_width = bore.upper - bore.lower;
    let int_width = int.upper - int.lower;
    let mut p = Vec::new();

    p.push("How the OD is chosen".to_string());
    p.push(format!(
        "The bushing OD is not simply Bore + Target Interference. It has to keep the achieved interference inside your interference window ({} to {} in) for EVERY bore size the bore tolerance allows ({} to {} in). That limits the OD to a range: from (largest bore + smallest interference) = {} in up to (smallest bore + largest interference) = {} in.",
        f(int.lower), f(int.upper), f(bore.lower), f(bore.upper), f(req_lo), f(req_hi)
    ));

    if out.tolerance_status == ToleranceStatus::Infeasible {
        p.push("Why it is INFEASIBLE".to_string());
        p.push(format!(
            "The bore band is {} in wide but the interference band is only {} in wide. Whatever OD is chosen, the bore's own variation uses up more than the whole allowed interference spread, so no OD can keep every part inside the window (the lower end of the OD range, {} in, is above the upper end, {} in).",
            f(bore_width), f(int_width), f(req_lo), f(req_hi)
        ));
        p.push("Inputs that cause it".to_string());
        p.push(format!(
            "Bore Tol + ({}) + Bore Tol - ({}) is larger than Interference Tol + ({}) + Interference Tol - ({}). Widen the interference tolerances or tighten the bore tolerances until the bore band is no wider than the interference band.",
            f(bore_plus), f(bore_minus), f(int_plus), f(int_minus)
        ));
    } else {
        let shift = od.nominal - desired;
        p.push("Why it is CLAMPED".to_string());
        p.push(format!(
            "The OD you would get from the nominals is Bore nominal {} + Target Interference {} = {} in. That value falls outside the allowed OD range ({} to {} in), so the OD nominal was moved to {} in ({:+.4} in).",
            f(bore.nominal), f(int.nominal), f(desired), f(req_lo), f(req_hi), f(od.nominal), shift
        ));
        p.push("Inputs that cause it".to_string());
        if desired < req_lo {
            p.push(format!(
                "The bore tolerance is lopsided toward the large side: Bore Tol + is {} in but Interference Tol - is only {} in. At the largest bore the OD would have to be {} in bigger than the nominal fit gives just to keep the minimum interference. Total widths still fit ({} in bore vs {} in interference), so a valid OD exists - it just cannot be centred.",
                f(bore_plus), f(int_minus), f(bore_plus - int_minus), f(bore_width), f(int_width)
            ));
        } else {
            p.push(format!(
                "The bore tolerance is lopsided toward the small side: Bore Tol - is {} in but Interference Tol + is only {} in. At the smallest bore the OD would have to be {} in smaller than the nominal fit gives to stay under the maximum interference. Total widths still fit ({} in bore vs {} in interference), so a valid OD exists - it just cannot be centred.",
                f(bore_minus), f(int_plus), f(bore_minus - int_plus), f(bore_width), f(int_width)
            ));
        }
        p.push("What it means for the result".to_string());
        p.push(format!(
            "The nominal interference actually achieved is {} in instead of the {} in you entered; across the tolerance band it ranges {} to {} in. Every part still stays inside your window - only the centre moved. This is a warning, not a failure.",
            f(od.nominal - bore.nominal), f(int.nominal), f(out.achieved_interference_tol.lower), f(out.achieved_interference_tol.upper)
        ));
    }
    p.push("How to clear it".to_string());
    p.push("Make the bore tolerance sit inside the interference window on each side separately (Bore Tol + <= Interference Tol -, Bore Tol - <= Interference Tol +), or move the Target Interference so the window re-centres on the bore band. The Fixes list offers verified edits where one exists.".to_string());
    if model.enforcement_enabled {
        if let Some(last) = p.last_mut() {
            last.push_str(" Note: Strict Interference Enforcement is on - the solver may already have tightened the bore band to reach this state; the bore tolerance shown above is the tightened one.");
        }
    }
    Some(p)
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

    #[test]
    fn clamped_tolerance_is_explained_with_the_inputs_that_cause_it() {
        let mut m = model();
        // Bore band lopsided to the large side but total width still fits the interference band.
        m.commit_number(NumberTarget::BoreTolPlus, 0.002);
        m.commit_number(NumberTarget::InterferenceTolPlus, 0.001);
        m.commit_number(NumberTarget::InterferenceTolMinus, 0.001);
        assert_eq!(m.output.tolerance_status, ToleranceStatus::Clamped);
        let text = explain_tolerance(&m).expect("explanation").join("\n");
        assert!(text.contains("CLAMPED"));
        assert!(text.contains("Bore Tol + is 0.0020 in but Interference Tol - is only 0.0010 in"), "{text}");
        assert!(text.contains("0.0010 in bigger"));
    }

    #[test]
    fn infeasible_tolerance_is_explained_by_the_band_widths() {
        let mut m = model();
        m.commit_number(NumberTarget::BoreTolPlus, 0.003);
        let text = explain_tolerance(&m).expect("explanation").join("\n");
        assert!(text.contains("INFEASIBLE") && text.contains("0.0030 in wide"), "{text}");
    }

    #[test]
    fn no_explanation_when_the_tolerance_is_ok() {
        assert!(explain_tolerance(&model()).is_none());
    }

    #[test]
    fn a_clamped_od_gets_a_verified_fix_that_centres_it() {
        let mut m = model();
        m.commit_number(NumberTarget::BoreTolPlus, 0.002);
        m.commit_number(NumberTarget::InterferenceTolPlus, 0.001);
        m.commit_number(NumberTarget::InterferenceTolMinus, 0.001);
        let idx = m.recommendations.iter().position(|r| r.fixes == CheckKind::Tolerance).expect("fix for clamped OD");
        m.apply_recommendation(idx);
        assert_eq!(m.output.tolerance_status, ToleranceStatus::Ok);
    }

    #[test]
    fn id_recommendations_snap_to_a_real_catalog_size_on_the_passing_side() {
        let mut m = model();
        m.commit_number(NumberTarget::MinWallStraight, 0.07);
        let rec = m.recommendations.iter().find(|r| r.edits.len() == 1 && r.edits[0].target == NumberTarget::IdBushing).expect("ID fix");
        let id = rec.edits[0].value;
        assert!(id < m.id_bushing, "ID only ever shrinks to fix a thin wall");
        assert!(m.catalog_sizes().iter().any(|c| (c.nominal - id).abs() < 1e-9), "{id} must be a catalog reamer/drill size; note={:?}", rec.note);
        assert!(rec.note.as_deref().unwrap().contains("no new tooling"), "{:?}", rec.note);
        let mut applied = m.clone();
        applied.apply_recommendation(m.recommendations.iter().position(|r| r == rec).unwrap());
        assert!(!applied.output.fail_straight);
    }

    #[test]
    fn a_thin_wall_offers_alternatives_that_leave_the_bore_alone_and_never_enlarge_it() {
        let mut m = model();
        m.commit_number(NumberTarget::MinWallStraight, 0.065);
        let wall: Vec<_> = m.recommendations.iter().filter(|r| r.fixes == CheckKind::StraightWall).collect();
        assert!(wall.len() >= 2, "more than one way to fix it: {wall:?}");
        assert!(wall.iter().any(|r| r.edits.iter().any(|e| e.target == NumberTarget::IdBushing)), "smaller ID alternative");
        assert!(wall.iter().any(|r| r.edits.iter().any(|e| e.target == NumberTarget::MinWallStraight)), "accept-thinner-wall alternative");
        for r in &m.recommendations {
            for e in &r.edits {
                if e.target == NumberTarget::BoreDia {
                    assert!(e.value < m.bore_dia, "bore may only be reduced: {r:?}");
                }
            }
            assert!(r.impact.first().map(|l| l.starts_with("Bore")).unwrap_or(true), "every applicable fix states its bore impact: {r:?}");
        }
        assert!(wall.iter().filter(|r| !r.touches_bore).count() >= 2);
    }

    #[test]
    fn edge_distance_offers_a_smaller_catalog_reamer_alternative_with_its_impact() {
        let mut m = model(); // default fails edge sequencing; a thinner bushing leaves room for a smaller bore
        m.commit_number(NumberTarget::IdBushing, 0.15);
        let rec = m.recommendations.iter().find(|r| r.touches_bore).expect("smaller-bore alternative");
        let bore = rec.edits.iter().find(|e| e.target == NumberTarget::BoreDia).unwrap().value;
        assert!(bore < m.bore_dia);
        assert!(m.catalog_sizes().iter().any(|c| (c.nominal - bore).abs() < 1e-9));
        assert!(rec.edits.iter().any(|e| e.target == NumberTarget::BoreTolPlus), "takes the reamer's own tolerance");
        assert!(rec.impact[0].contains("smaller, never enlarged"), "{:?}", rec.impact);
        assert!(rec.impact.len() > 1, "metrics that move are listed: {:?}", rec.impact);
    }

    #[test]
    fn impact_lists_the_metrics_that_move() {
        let mut m = model();
        m.commit_number(NumberTarget::MinWallStraight, 0.065);
        let rec = m.recommendations.iter().find(|r| r.edits.iter().any(|e| e.target == NumberTarget::IdBushing)).unwrap();
        let text = rec.impact.join("\n");
        assert!(text.contains("Contact pressure") || text.contains("Install force"), "{text}");
    }

    #[test]
    fn the_snap_catalog_includes_standard_drills_but_a_bore_never_snaps_to_one() {
        let m = model();
        let sizes = m.catalog_sizes();
        assert!(sizes.iter().any(|c| c.source == "drill" && c.label == "Q" && (c.nominal - 0.332).abs() < 1e-9), "letter drill present");
        assert!(sizes.iter().any(|c| c.source == "drill" && c.common), "common drills flagged");
        assert!(sizes.iter().any(|c| c.source == "reamer"));
        // Reamer wins a tie on the same nominal.
        for r in bushing_solver::reamers::all_reamers() {
            let at: Vec<_> = sizes.iter().filter(|c| (c.nominal - r.nominal_in).abs() < 1e-6).collect();
            assert_eq!(at.len(), 1);
            assert_eq!(at[0].source, "reamer");
        }
        assert!(sizes.windows(2).all(|w| w[0].nominal <= w[1].nominal));

        // Smaller-bore fixes only ever use reamers.
        let mut m = model();
        m.commit_number(NumberTarget::IdBushing, 0.15);
        for r in &m.recommendations {
            if let Some(b) = r.edits.iter().find(|e| e.target == NumberTarget::BoreDia) {
                let c = sizes.iter().find(|c| (c.nominal - b.value).abs() < 1e-9).expect("catalog bore");
                assert_ne!(c.source, "drill", "{r:?}");
            }
        }
    }

    #[test]
    fn id_fixes_can_now_land_on_a_drill_size_that_no_reamer_covers() {
        let mut m = model();
        m.commit_number(NumberTarget::MinWallStraight, 0.07);
        let rec = m.recommendations.iter().find(|r| r.edits.len() == 1 && r.edits[0].target == NumberTarget::IdBushing).unwrap();
        let id = rec.edits[0].value;
        let size = m.catalog_sizes().into_iter().find(|c| (c.nominal - id).abs() < 1e-9).expect("a catalog size");
        assert!(["reamer", "drill", "saved"].contains(&size.source));
        assert!(rec.note.as_deref().unwrap().contains(size.source));
    }
}

