//! Tolerance-stack resolution, ported line-for-line from
//! engineering.toolbox's `src/lib/core/bushing/solveMath.ts`'s
//! `resolveTolerance`/`makeRange`/`buildOdTolerance`/
//! `containmentViolations`/`enforceBoreBandForTarget`.

pub const EPS: f64 = 1e-9;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ToleranceMode {
    NominalTol,
    Limits,
}

/// A resolved tolerance band - always stores both the limit and
/// nominal/plus-minus views, regardless of which one the caller supplied
/// (`TypeScript`'s `ToleranceRange`, same fields).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ToleranceRange {
    pub mode: ToleranceMode,
    pub lower: f64,
    pub upper: f64,
    pub nominal: f64,
    pub tol_plus: f64,
    pub tol_minus: f64,
}

pub fn clamp(value: f64, lower: f64, upper: f64) -> f64 {
    value.min(upper).max(lower)
}

fn round_tol(value: f64) -> f64 {
    if value.is_finite() {
        value
    } else {
        0.0
    }
}

pub fn make_range(mode: ToleranceMode, lower: f64, upper: f64, nominal: Option<f64>) -> ToleranceRange {
    let lo = lower.min(upper);
    let hi = lower.max(upper);
    let nom = clamp(nominal.filter(|n| n.is_finite()).unwrap_or((lo + hi) / 2.0), lo, hi);
    ToleranceRange {
        mode,
        lower: round_tol(lo),
        upper: round_tol(hi),
        nominal: round_tol(nom),
        tol_plus: round_tol(hi - nom),
        tol_minus: round_tol(nom - lo),
    }
}

/// Input for `resolve_tolerance` - mirrors the TS source's loosely-typed
/// options object, but every field the straight-bushing path actually
/// reads is a real (non-optional) parameter here instead.
pub struct ResolveToleranceInput {
    pub mode: ToleranceMode,
    pub nominal: f64,
    pub plus: f64,
    pub minus: f64,
    pub lower: Option<f64>,
    pub upper: Option<f64>,
    pub min_floor: Option<f64>,
}

pub fn resolve_tolerance(input: ResolveToleranceInput) -> ToleranceRange {
    // The TS source's `Math.max(0, Number(input.minFloor))` only clamps a
    // *provided* floor to be nonnegative - a genuinely absent floor stays
    // `-Infinity` (no floor at all), not 0.
    let min_floor = match input.min_floor {
        Some(f) if f.is_finite() => f.max(0.0),
        _ => f64::NEG_INFINITY,
    };

    if input.mode == ToleranceMode::Limits {
        let lower = input.lower.unwrap_or(input.nominal);
        let upper = input.upper.unwrap_or(input.nominal.max(lower));
        let lo = min_floor.max(lower.min(upper));
        let hi = lo.max(lower.max(upper));
        return make_range(ToleranceMode::Limits, lo, hi, Some(input.nominal));
    }

    let nominal = input.nominal;
    let plus = input.plus.max(0.0);
    let minus = input.minus.max(0.0);
    let lo = min_floor.max(nominal - minus);
    let hi = lo.max(nominal + plus);
    make_range(ToleranceMode::NominalTol, lo, hi, Some(nominal))
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ToleranceStatus {
    Ok,
    /// The desired OD nominal fell outside the feasible `[requiredLower,
    /// requiredUpper]` band and was clamped into it - reachable once a
    /// caller uses `enforce_bore_band_for_target` to tighten an
    /// originally-infeasible bore band (`solveMath.ts`'s own
    /// `'ok' | 'clamped' | 'infeasible'` tri-state - `buildOdTolerance`
    /// line 74).
    Clamped,
    Infeasible,
}

pub struct OdToleranceResult {
    pub status: ToleranceStatus,
    pub notes: Vec<String>,
    pub od: ToleranceRange,
    pub achieved_interference: ToleranceRange,
}

/// `bore + targetInterference -> requiredOD`, ported from `buildOdTolerance`.
pub fn build_od_tolerance(bore: ToleranceRange, target_interference: ToleranceRange) -> OdToleranceResult {
    let required_lower = bore.upper + target_interference.lower;
    let required_upper = bore.lower + target_interference.upper;
    let desired_nominal = bore.nominal + target_interference.nominal;

    if required_lower <= required_upper + EPS {
        let nominal = clamp(desired_nominal, required_lower, required_upper);
        let od = make_range(ToleranceMode::Limits, required_lower, required_upper, Some(nominal));
        let achieved = make_range(ToleranceMode::Limits, od.lower - bore.upper, od.upper - bore.lower, Some(od.nominal - bore.nominal));
        let status = if (nominal - desired_nominal).abs() > EPS { ToleranceStatus::Clamped } else { ToleranceStatus::Ok };
        let notes = if status == ToleranceStatus::Clamped {
            vec!["OD nominal was clamped to keep fit inside the requested interference tolerance window.".to_string()]
        } else {
            Vec::new()
        };
        return OdToleranceResult { status, notes, od, achieved_interference: achieved };
    }

    let od_collapsed = make_range(ToleranceMode::Limits, desired_nominal, desired_nominal, Some(desired_nominal));
    let achieved_collapsed = make_range(
        ToleranceMode::Limits,
        od_collapsed.nominal - bore.upper,
        od_collapsed.nominal - bore.lower,
        Some(od_collapsed.nominal - bore.nominal),
    );
    OdToleranceResult {
        status: ToleranceStatus::Infeasible,
        notes: vec!["Bore tolerance width exceeds interference tolerance width; full-range containment is infeasible.".to_string()],
        od: od_collapsed,
        achieved_interference: achieved_collapsed,
    }
}

/// Process-capability floor on the bore's achievable tolerance width - a
/// tightening attempt below this is physically impossible, not just
/// undesirable (`solveMath.ts`'s `capability?.minAchievableTolWidth`).
#[derive(Debug, Clone, Copy, Default)]
pub struct BoreCapability {
    pub min_achievable_tol_width: Option<f64>,
}

/// Governs whether/how an infeasible bore tolerance band gets
/// auto-tightened to fit inside the target interference tolerance width
/// (`solveMath.ts`'s `policy` parameter). `lock_bore`/`preserve_bore_nominal`
/// default `true` in the TS source (a reamer-fixed bore can't be
/// auto-adjusted at all; when it can, the entered minimum clean-up size
/// is preserved by default).
#[derive(Debug, Clone, Copy)]
pub struct EnforcementPolicy {
    pub enabled: bool,
    pub lock_bore: bool,
    pub preserve_bore_nominal: bool,
    pub allow_bore_nominal_shift: bool,
    pub max_bore_nominal_shift: f64,
}

impl Default for EnforcementPolicy {
    fn default() -> Self {
        EnforcementPolicy { enabled: false, lock_bore: true, preserve_bore_nominal: true, allow_bore_nominal_shift: false, max_bore_nominal_shift: 0.0 }
    }
}

pub struct EnforceBoreBandResult {
    pub adjusted: ToleranceRange,
    pub changed: bool,
    pub note: Option<String>,
}

/// Ported from `solveMath.ts:102-147`. Tightens `bore`'s tolerance band
/// (from the upper side, never moving the entered minimum clean-up size
/// downward) until its width matches `target_interference`'s width, so a
/// subsequent `build_od_tolerance` call can achieve full-range
/// containment. A no-op (`changed: false`) when the bore band is already
/// narrow enough, or when `capability`'s floor makes the target width
/// unreachable.
pub fn enforce_bore_band_for_target(
    bore: ToleranceRange,
    target_interference: ToleranceRange,
    capability: Option<&BoreCapability>,
    policy: &EnforcementPolicy,
) -> EnforceBoreBandResult {
    let bore_width = (bore.upper - bore.lower).max(0.0);
    let target_width = (target_interference.upper - target_interference.lower).max(0.0);
    if bore_width <= target_width + EPS {
        return EnforceBoreBandResult { adjusted: bore, changed: false, note: None };
    }
    let min_achievable = capability.and_then(|c| c.min_achievable_tol_width).unwrap_or(0.0);
    if min_achievable.is_finite() && min_achievable > target_width + EPS {
        return EnforceBoreBandResult {
            adjusted: bore,
            changed: false,
            note: Some(format!(
                "Strict interference enforcement is blocked by bore process capability floor ({min_achievable:.4} > target width {target_width:.4})."
            )),
        };
    }

    let mut lower = (bore.upper - target_width).max(1e-6);
    let mut upper = lower.max(bore.upper);
    if policy.allow_bore_nominal_shift && !policy.preserve_bore_nominal {
        let max_shift = policy.max_bore_nominal_shift.max(0.0);
        if max_shift > EPS {
            let desired_nominal = bore.nominal.max(lower);
            let current_nominal = clamp(bore.nominal, lower, upper);
            let shift = clamp(desired_nominal - current_nominal, 0.0, max_shift);
            lower += shift;
            upper += shift;
        }
    }
    let note = if policy.allow_bore_nominal_shift && !policy.preserve_bore_nominal {
        "Bore tolerance was tightened while preserving the entered minimum bore, then shifted upward within the allowed limit."
    } else {
        "Bore tolerance was tightened from the upper side while preserving the entered minimum bore."
    };
    EnforceBoreBandResult {
        adjusted: make_range(ToleranceMode::Limits, lower, upper, Some(clamp(bore.nominal, lower, upper))),
        changed: true,
        note: Some(note.to_string()),
    }
}

pub struct ContainmentViolations {
    pub lower_violation: f64,
    pub upper_violation: f64,
}

pub fn containment_violations(achieved: ToleranceRange, target: ToleranceRange) -> ContainmentViolations {
    ContainmentViolations {
        lower_violation: (target.lower - achieved.lower).max(0.0),
        upper_violation: (achieved.upper - target.upper).max(0.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_tolerance_nominal_tol_mode_builds_symmetric_and_asymmetric_bands() {
        let r = resolve_tolerance(ResolveToleranceInput { mode: ToleranceMode::NominalTol, nominal: 0.5, plus: 0.002, minus: 0.001, lower: None, upper: None, min_floor: None });
        assert!((r.lower - 0.499).abs() < 1e-9);
        assert!((r.upper - 0.502).abs() < 1e-9);
        assert!((r.nominal - 0.5).abs() < 1e-9);
    }

    #[test]
    fn resolve_tolerance_limits_mode_uses_lower_upper_directly() {
        let r = resolve_tolerance(ResolveToleranceInput { mode: ToleranceMode::Limits, nominal: 0.5, plus: 0.0, minus: 0.0, lower: Some(0.498), upper: Some(0.503), min_floor: None });
        assert!((r.lower - 0.498).abs() < 1e-9);
        assert!((r.upper - 0.503).abs() < 1e-9);
    }

    #[test]
    fn build_od_tolerance_is_ok_for_a_normal_bore_and_interference_band() {
        let bore = make_range(ToleranceMode::NominalTol, 0.5, 0.5, Some(0.5));
        let interference = make_range(ToleranceMode::NominalTol, 0.0015, 0.0015, Some(0.0015));
        let result = build_od_tolerance(bore, interference);
        assert_eq!(result.status, ToleranceStatus::Ok);
        assert!((result.od.nominal - 0.5015).abs() < 1e-9);
        assert!((result.achieved_interference.nominal - 0.0015).abs() < 1e-9);
    }

    #[test]
    fn build_od_tolerance_is_infeasible_when_bore_band_is_wider_than_interference_band() {
        let bore = make_range(ToleranceMode::NominalTol, 0.5, 0.0, Some(0.0)); // wide 0..0.5 band
        let interference = make_range(ToleranceMode::NominalTol, 0.0001, 0.0, Some(0.0));
        let result = build_od_tolerance(bore, interference);
        assert_eq!(result.status, ToleranceStatus::Infeasible);
        assert!(!result.notes.is_empty());
    }

    #[test]
    fn enforce_bore_band_is_a_noop_when_bore_band_already_fits_target_width() {
        let bore = make_range(ToleranceMode::Limits, 0.499, 0.501, Some(0.5)); // width 0.002
        let target = make_range(ToleranceMode::Limits, 0.0, 0.003, Some(0.0015)); // width 0.003
        let result = enforce_bore_band_for_target(bore, target, None, &EnforcementPolicy { enabled: true, ..Default::default() });
        assert!(!result.changed);
        assert_eq!(result.adjusted, bore);
    }

    #[test]
    fn enforce_bore_band_tightens_from_the_upper_side_never_moving_the_lower_limit_down() {
        // Bore band width 0.005 (0.4975..0.5025), target width 0.001 - too wide to
        // achieve full containment as entered.
        let bore = make_range(ToleranceMode::Limits, 0.4975, 0.5025, Some(0.5));
        let target = make_range(ToleranceMode::Limits, 0.0005, 0.0015, Some(0.001)); // width 0.001
        let result = enforce_bore_band_for_target(bore, target, None, &EnforcementPolicy { enabled: true, ..Default::default() });
        assert!(result.changed);
        // Tightens toward bore.upper - target_width, which - since bore_width >
        // target_width - is always >= the original lower, never below it.
        assert!(result.adjusted.lower >= bore.lower);
        assert!((result.adjusted.lower - (bore.upper - 0.001)).abs() < 1e-9);
        assert!((result.adjusted.upper - bore.upper).abs() < 1e-9, "upper limit stays put with no nominal shift allowed");
        assert!(result.note.is_some());
    }

    #[test]
    fn enforce_bore_band_refuses_to_go_below_the_process_capability_floor() {
        let bore = make_range(ToleranceMode::Limits, 0.4975, 0.5025, Some(0.5)); // width 0.005
        let target = make_range(ToleranceMode::Limits, 0.0, 0.001, Some(0.0005)); // width 0.001
        let capability = BoreCapability { min_achievable_tol_width: Some(0.002) }; // floor > target width
        let result = enforce_bore_band_for_target(bore, target, Some(&capability), &EnforcementPolicy { enabled: true, ..Default::default() });
        assert!(!result.changed);
        assert!(result.note.unwrap().contains("capability floor"));
    }

    #[test]
    fn build_od_tolerance_reports_clamped_when_an_enforced_bore_band_forces_the_od_nominal_off_center() {
        let bore = make_range(ToleranceMode::Limits, 0.4995, 0.5005, Some(0.5)); // tightened band, width 0.001
        // Target interference is centered well above what a symmetric OD nominal
        // clamp allows relative to this bore band - forces the Clamped path.
        let interference = make_range(ToleranceMode::Limits, 0.001, 0.002, Some(0.0025));
        let result = build_od_tolerance(bore, interference);
        assert_eq!(result.status, ToleranceStatus::Clamped);
        assert!(!result.notes.is_empty());
    }
}
