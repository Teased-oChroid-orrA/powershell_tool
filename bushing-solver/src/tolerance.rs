//! Tolerance-stack resolution, ported line-for-line from
//! engineering.toolbox's `src/lib/core/bushing/solveMath.ts`'s
//! `resolveTolerance`/`makeRange`/`buildOdTolerance`/
//! `containmentViolations` - the straight-bushing-relevant subset (the
//! TS source's bore-capability/interference-policy auto-adjustment
//! machinery is not ported here - v1 always resolves the bore/
//! interference tolerance bands as entered, without attempting to
//! auto-tighten an infeasible bore band; `status` still reports
//! `Infeasible` honestly when that happens, matching the TS source's own
//! "OK/clamped/infeasible" tri-state, just without the auto-adjust step
//! that tries to resolve it).

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
    Infeasible,
}

pub struct OdToleranceResult {
    pub status: ToleranceStatus,
    pub notes: Vec<&'static str>,
    pub od: ToleranceRange,
    pub achieved_interference: ToleranceRange,
}

/// `bore + targetInterference -> requiredOD`, ported from
/// `buildOdTolerance` exactly (the `'clamped'` status the TS source also
/// has never triggers for the straight-line resolution v1 does - it only
/// happens through the bore-auto-adjustment path this crate doesn't
/// implement - so only `Ok`/`Infeasible` are modeled here).
pub fn build_od_tolerance(bore: ToleranceRange, target_interference: ToleranceRange) -> OdToleranceResult {
    let required_lower = bore.upper + target_interference.lower;
    let required_upper = bore.lower + target_interference.upper;
    let desired_nominal = bore.nominal + target_interference.nominal;

    if required_lower <= required_upper + EPS {
        let nominal = clamp(desired_nominal, required_lower, required_upper);
        let od = make_range(ToleranceMode::Limits, required_lower, required_upper, Some(nominal));
        let achieved = make_range(ToleranceMode::Limits, od.lower - bore.upper, od.upper - bore.lower, Some(od.nominal - bore.nominal));
        return OdToleranceResult { status: ToleranceStatus::Ok, notes: Vec::new(), od, achieved_interference: achieved };
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
        notes: vec!["Bore tolerance width exceeds interference tolerance width; full-range containment is infeasible."],
        od: od_collapsed,
        achieved_interference: achieved_collapsed,
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
}
