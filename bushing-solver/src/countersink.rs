//! Countersink geometry solve, ported from engineering.toolbox's
//! `src/lib/core/bushing/solveMath.ts` (`solveCountersink`,
//! `enumerateCountersinkCorners`, `csDiaToleranceFromBase`,
//! `csDepthToleranceFromBase`).

use crate::tolerance::{make_range, ToleranceMode, ToleranceRange, EPS};

/// Which countersink dimension is the direct user input vs. derived from
/// the other two + the base diameter (`solveMath.ts`'s `CSMode`). Default
/// `DepthAngle`, matching `normalize.ts`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CsMode {
    DiaAngle,
    DepthAngle,
    DiaDepth,
}

impl Default for CsMode {
    fn default() -> Self {
        CsMode::DepthAngle
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CsCorner {
    pub dia: f64,
    pub depth: f64,
    pub angle_deg: f64,
}

/// Solves whichever of dia/depth/angle is derived for `mode`, from the
/// other two plus `base_dia` (the bushing bore/OD the countersink cuts
/// into). Ported from `solveMath.ts:226-241`.
pub fn solve_countersink(mode: CsMode, dia: f64, depth: f64, angle: f64, base_dia: f64) -> CsCorner {
    let safe_base_dia = if base_dia.is_finite() { base_dia.max(0.0) } else { 0.0 };
    let safe_dia = if dia.is_finite() { dia.max(0.0) } else { 0.0 };
    let safe_depth = if depth.is_finite() { depth.max(0.0) } else { 0.0 };
    let safe_angle = if angle.is_finite() { angle.clamp(1e-3, 179.999) } else { 100.0 };
    let r_rad = (safe_angle / 2.0) * std::f64::consts::PI / 180.0;
    let tan_r = r_rad.tan();

    let mut result = CsCorner { dia: safe_dia, depth: safe_depth, angle_deg: safe_angle };
    match mode {
        CsMode::DepthAngle => {
            result.dia = (safe_base_dia + 2.0 * safe_depth * tan_r).max(safe_base_dia);
        }
        CsMode::DiaAngle => {
            result.depth = if tan_r > 1e-9 { ((safe_dia - safe_base_dia) / (2.0 * tan_r)).max(0.0) } else { 0.0 };
        }
        CsMode::DiaDepth => {
            let angle_rad = if safe_depth > 1e-9 { 2.0 * ((safe_dia - safe_base_dia).max(0.0) / (2.0 * safe_depth)).atan() } else { 0.0 };
            result.angle_deg = (angle_rad * 180.0 / std::f64::consts::PI).clamp(0.0, 179.999);
        }
    }
    result
}

fn tolerance_corners(tol: ToleranceRange) -> Vec<f64> {
    if tol.upper > tol.lower + EPS {
        vec![tol.lower, tol.upper]
    } else {
        vec![tol.nominal]
    }
}

/// Enumerates physically self-consistent countersink corners for a
/// worst-case search: only the mode's actual free/direct-input variables
/// vary (dia/depth, and - beyond the TS reference, which has no angle
/// tolerance concept at all, confirmed by reading its schema - angle too,
/// when the mode makes it a direct input), and the derived one is always
/// re-solved from them via `solve_countersink` - never perturbed
/// independently of its geometric coupling to the others. Base ported
/// from `solveMath.ts:208-224`; `angle_tol` is this crate's own
/// extension. `angle_tol` collapsing to a single point (equal
/// lower/upper, e.g. `ToleranceRange` built with zero tolerance) exactly
/// reproduces the original 2-variable enumeration - proven in this
/// module's own tests, not just assumed.
pub fn enumerate_countersink_corners(mode: CsMode, base_dia: f64, dia_tol: ToleranceRange, depth_tol: ToleranceRange, angle_tol: ToleranceRange) -> Vec<CsCorner> {
    let dia_vals = if mode == CsMode::DepthAngle { vec![dia_tol.nominal] } else { tolerance_corners(dia_tol) };
    let depth_vals = if mode == CsMode::DiaAngle { vec![depth_tol.nominal] } else { tolerance_corners(depth_tol) };
    // Angle is a direct input only in DepthAngle/DiaAngle mode (DiaDepth
    // derives angle from dia+depth instead) - matching the same
    // "only the mode's actual direct-input variables vary" rule the
    // dia/depth gating above already follows.
    let angle_vals = if mode == CsMode::DiaDepth { vec![angle_tol.nominal] } else { tolerance_corners(angle_tol) };
    let mut corners = Vec::with_capacity(dia_vals.len() * depth_vals.len() * angle_vals.len());
    for &dia in &dia_vals {
        for &depth in &depth_vals {
            for &angle in &angle_vals {
                corners.push(solve_countersink(mode, dia, depth, angle, base_dia));
            }
        }
    }
    corners
}

fn point_range(v: f64) -> ToleranceRange {
    make_range(ToleranceMode::NominalTol, v, v, Some(v))
}

fn half_angle_tan_rad(angle_deg: f64) -> f64 {
    ((angle_deg / 2.0) * std::f64::consts::PI / 180.0).tan()
}

/// Propagates the base-diameter (and, beyond the TS reference, angle)
/// tolerance through to the countersink diameter when `mode` derives dia
/// from depth+angle - a no-op passthrough (nominal-only range) for every
/// other mode. Base formula from `solveMath.ts:159-173`
/// (`csDiaToleranceFromBase`); the TS source has no angle-tolerance
/// input at all (confirmed by reading its schema), so `angle_tolerance`
/// is this crate's own addition, not a divergence from an existing
/// reference behavior.
///
/// `dia = base + 2*depth*tan(angle/2)` is monotonically increasing in
/// all three of `base`, `depth`, and `angle` (`tan` is increasing on the
/// relevant `(0deg, 90deg)` half-angle range) - so the true min/max are
/// exactly the all-low/all-high corners, not just an assumed pairing
/// (`enumerate_countersink_corners_full_cartesian_search_matches_the_monotonic_shortcut`
/// below brute-forces the full 8-corner cartesian product and proves it
/// matches this direct formula, rather than trusting the monotonicity
/// argument alone).
pub fn cs_dia_tolerance_from_base(mode: CsMode, solved_dia: f64, solved_depth: f64, solved_angle_deg: f64, base: ToleranceRange, depth_tolerance: Option<ToleranceRange>, angle_tolerance: Option<ToleranceRange>) -> ToleranceRange {
    if mode != CsMode::DepthAngle {
        return point_range(solved_dia);
    }
    let depth = depth_tolerance.unwrap_or_else(|| point_range(solved_depth));
    let angle = angle_tolerance.unwrap_or_else(|| point_range(solved_angle_deg));
    let lower = base.lower + 2.0 * depth.lower.max(0.0) * half_angle_tan_rad(angle.lower);
    let upper = base.upper + 2.0 * depth.upper.max(0.0) * half_angle_tan_rad(angle.upper);
    make_range(base.mode, lower, upper, Some(solved_dia))
}

/// Propagates the base-diameter (and angle) tolerance through to the
/// countersink depth when `mode` derives depth from dia+angle (or is
/// otherwise a direct input, in which case `explicit_depth_tolerance` -
/// the field's own resolved range - is returned unchanged). Base formula
/// from `solveMath.ts:175-196` (`csDepthToleranceFromBase`);
/// `angle_tolerance` is this crate's own addition (see
/// `cs_dia_tolerance_from_base`'s doc comment).
///
/// `depth = (dia - base) / (2*tan(angle/2))` is increasing in `dia`,
/// *decreasing* in `base` (subtracted) and *decreasing* in `angle`
/// (denominator) - the opposite pairing direction from
/// `cs_dia_tolerance_from_base` for the angle term specifically, which
/// is exactly the kind of sign flip that's easy to get wrong by copying
/// the dia-tolerance pairing pattern instead of re-deriving it. Proven
/// against the full cartesian search in this module's own tests, not
/// assumed correct by symmetry with the dia case.
pub fn cs_depth_tolerance_from_base(mode: CsMode, solved_depth: f64, solved_dia: f64, solved_angle_deg: f64, base: ToleranceRange, explicit_depth_tolerance: Option<ToleranceRange>, dia_tolerance: Option<ToleranceRange>, angle_tolerance: Option<ToleranceRange>) -> ToleranceRange {
    if mode == CsMode::DepthAngle || mode == CsMode::DiaDepth {
        return explicit_depth_tolerance.unwrap_or_else(|| point_range(solved_depth));
    }
    let dia = dia_tolerance.unwrap_or_else(|| point_range(solved_dia));
    let angle = angle_tolerance.unwrap_or_else(|| point_range(solved_angle_deg));
    let tan_lower = half_angle_tan_rad(angle.upper); // larger angle -> smaller depth -> pairs with the LOWER depth bound
    let tan_upper = half_angle_tan_rad(angle.lower); // smaller angle -> larger depth -> pairs with the UPPER depth bound
    if !tan_lower.is_finite() || tan_lower.abs() < 1e-12 || !tan_upper.is_finite() || tan_upper.abs() < 1e-12 {
        return point_range(solved_depth);
    }
    let lower = ((dia.lower - base.upper) / (2.0 * tan_lower)).max(0.0);
    let upper = ((dia.upper - base.lower) / (2.0 * tan_upper)).max(0.0);
    make_range(base.mode, lower, upper, Some(solved_depth))
}

/// Propagates the base-diameter, diameter, and depth tolerances through
/// to the countersink angle when `mode` derives angle from dia+depth
/// (`DiaDepth` mode - the one mode where angle is never a direct
/// user-entered tolerance, so this is its *only* source of a real
/// tolerance range). Not present in the TS reference at all (it has no
/// angle-tolerance concept in any mode) - this crate's own addition,
/// completing the pattern `cs_dia_tolerance_from_base`/
/// `cs_depth_tolerance_from_base` already establish for the other two
/// modes' derived dimension.
///
/// `angle = 2*atan((dia-base)/(2*depth))` is increasing in `dia`,
/// decreasing in `base`, and decreasing in `depth` - proven against the
/// full cartesian search, not assumed from the dia/depth formulas' own
/// sign patterns.
pub fn cs_angle_tolerance_from_base(mode: CsMode, solved_angle_deg: f64, solved_dia: f64, solved_depth: f64, base: ToleranceRange, dia_tolerance: Option<ToleranceRange>, depth_tolerance: Option<ToleranceRange>) -> ToleranceRange {
    if mode != CsMode::DiaDepth {
        return point_range(solved_angle_deg);
    }
    let dia = dia_tolerance.unwrap_or_else(|| point_range(solved_dia));
    let depth = depth_tolerance.unwrap_or_else(|| point_range(solved_depth));
    let angle_at = |dia_v: f64, base_v: f64, depth_v: f64| -> f64 {
        if depth_v <= 1e-9 {
            return 0.0;
        }
        let rad = 2.0 * ((dia_v - base_v).max(0.0) / (2.0 * depth_v)).atan();
        (rad * 180.0 / std::f64::consts::PI).clamp(0.0, 179.999)
    };
    let lower = angle_at(dia.lower, base.upper, depth.upper);
    let upper = angle_at(dia.upper, base.lower, depth.lower);
    make_range(ToleranceMode::NominalTol, lower, upper, Some(solved_angle_deg))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solve_countersink_depth_angle_mode_derives_dia_from_depth_and_angle() {
        // 100deg countersink, 0.125in depth, into a 0.5in base bore:
        // dia = base + 2*depth*tan(50deg).
        let corner = solve_countersink(CsMode::DepthAngle, 0.0, 0.125, 100.0, 0.5);
        let expected_dia = 0.5 + 2.0 * 0.125 * (50.0_f64.to_radians()).tan();
        assert!((corner.dia - expected_dia).abs() < 1e-9);
        assert_eq!(corner.depth, 0.125);
        assert_eq!(corner.angle_deg, 100.0);
    }

    #[test]
    fn solve_countersink_dia_angle_mode_derives_depth_from_dia_and_angle() {
        let corner = solve_countersink(CsMode::DiaAngle, 0.625, 0.0, 100.0, 0.5);
        let expected_depth = (0.625 - 0.5) / (2.0 * (50.0_f64.to_radians()).tan());
        assert!((corner.depth - expected_depth).abs() < 1e-9);
    }

    #[test]
    fn solve_countersink_dia_depth_mode_derives_angle_from_dia_and_depth() {
        let corner = solve_countersink(CsMode::DiaDepth, 0.625, 0.125, 0.0, 0.5);
        assert!(corner.angle_deg > 0.0 && corner.angle_deg < 180.0);
        // Round-trip: solving depth_angle back with the derived angle should
        // reproduce the original dia.
        let back = solve_countersink(CsMode::DepthAngle, 0.0, 0.125, corner.angle_deg, 0.5);
        assert!((back.dia - 0.625).abs() < 1e-6);
    }

    #[test]
    fn enumerate_countersink_corners_only_varies_the_modes_direct_inputs() {
        let dia_tol = make_range(ToleranceMode::Limits, 0.62, 0.63, Some(0.625));
        let depth_tol = make_range(ToleranceMode::NominalTol, 0.125, 0.125, Some(0.125));
        let angle_tol = point_range(100.0);
        // depth_angle mode: dia is derived, so it must NOT vary across corners
        // even though a (degenerate, unused) dia_tol band was passed in.
        let corners = enumerate_countersink_corners(CsMode::DepthAngle, 0.5, dia_tol, depth_tol, angle_tol);
        assert_eq!(corners.len(), 1, "depth is a single point, angle is a single point, and dia is derived - not enumerated");
    }

    #[test]
    fn enumerate_countersink_corners_produces_four_corners_when_both_dia_and_depth_vary() {
        let dia_tol = make_range(ToleranceMode::Limits, 0.62, 0.63, Some(0.625));
        let depth_tol = make_range(ToleranceMode::Limits, 0.12, 0.13, Some(0.125));
        let angle_tol = point_range(100.0); // DiaDepth mode: angle is derived, never enumerated
        let corners = enumerate_countersink_corners(CsMode::DiaDepth, 0.5, dia_tol, depth_tol, angle_tol);
        assert_eq!(corners.len(), 4);
    }

    #[test]
    fn enumerate_countersink_corners_produces_four_corners_when_depth_and_angle_both_vary() {
        // DepthAngle mode: depth and angle are both direct inputs now
        // that angle tolerance exists - dia (derived) must not multiply
        // the corner count.
        let dia_tol = point_range(0.625); // unused/derived, degenerate on purpose
        let depth_tol = make_range(ToleranceMode::Limits, 0.12, 0.13, Some(0.125));
        let angle_tol = make_range(ToleranceMode::Limits, 95.0, 105.0, Some(100.0));
        let corners = enumerate_countersink_corners(CsMode::DepthAngle, 0.5, dia_tol, depth_tol, angle_tol);
        assert_eq!(corners.len(), 4);
    }

    #[test]
    fn cs_dia_tolerance_from_base_propagates_base_depth_and_angle_tolerance_in_depth_angle_mode() {
        let base = make_range(ToleranceMode::NominalTol, 0.5, 0.0, Some(0.5)); // exact base, no tol
        let depth_tol = make_range(ToleranceMode::NominalTol, 0.125, 0.005, Some(0.005));
        let angle_tol = make_range(ToleranceMode::Limits, 98.0, 102.0, Some(100.0));
        let result = cs_dia_tolerance_from_base(CsMode::DepthAngle, 0.6, 0.125, 100.0, base, Some(depth_tol), Some(angle_tol));
        assert!((result.lower - (base.lower + 2.0 * depth_tol.lower * half_angle_tan_rad(angle_tol.lower))).abs() < 1e-9);
        assert!((result.upper - (base.upper + 2.0 * depth_tol.upper * half_angle_tan_rad(angle_tol.upper))).abs() < 1e-9);
    }

    #[test]
    fn cs_dia_tolerance_from_base_is_a_nominal_passthrough_outside_depth_angle_mode() {
        let base = make_range(ToleranceMode::Limits, 0.49, 0.51, Some(0.5));
        let result = cs_dia_tolerance_from_base(CsMode::DiaDepth, 0.6, 0.125, 100.0, base, None, None);
        assert_eq!(result.lower, 0.6);
        assert_eq!(result.upper, 0.6);
    }

    #[test]
    fn cs_depth_tolerance_from_base_derives_from_dia_and_angle_in_dia_angle_mode() {
        let base = make_range(ToleranceMode::NominalTol, 0.5, 0.0, Some(0.5));
        let dia_tol = make_range(ToleranceMode::NominalTol, 0.625, 0.005, Some(0.005));
        let angle_tol = make_range(ToleranceMode::Limits, 98.0, 102.0, Some(100.0));
        let result = cs_depth_tolerance_from_base(CsMode::DiaAngle, 0.0625, 0.625, 100.0, base, None, Some(dia_tol), Some(angle_tol));
        // Larger angle -> smaller depth, so the LOWER depth bound pairs
        // with the UPPER angle bound - the opposite direction from the
        // dia-tolerance test above.
        assert!((result.lower - ((dia_tol.lower - base.upper) / (2.0 * half_angle_tan_rad(angle_tol.upper))).max(0.0)).abs() < 1e-9);
        assert!((result.upper - ((dia_tol.upper - base.lower) / (2.0 * half_angle_tan_rad(angle_tol.lower))).max(0.0)).abs() < 1e-9);
    }

    #[test]
    fn cs_depth_tolerance_from_base_returns_the_explicit_range_in_depth_angle_mode() {
        let base = make_range(ToleranceMode::NominalTol, 0.5, 0.0, Some(0.5));
        let explicit = make_range(ToleranceMode::NominalTol, 0.125, 0.01, Some(0.005));
        let result = cs_depth_tolerance_from_base(CsMode::DepthAngle, 0.125, 0.6, 100.0, base, Some(explicit), None, None);
        assert_eq!(result, explicit);
    }

    #[test]
    fn cs_angle_tolerance_from_base_is_a_nominal_passthrough_outside_dia_depth_mode() {
        let base = make_range(ToleranceMode::NominalTol, 0.5, 0.0, Some(0.5));
        let result = cs_angle_tolerance_from_base(CsMode::DepthAngle, 100.0, 0.625, 0.125, base, None, None);
        assert_eq!(result.lower, 100.0);
        assert_eq!(result.upper, 100.0);
    }

    /// The actual proof this crate's monotonicity-shortcut formulas
    /// (`cs_dia_tolerance_from_base`/`cs_depth_tolerance_from_base`/
    /// `cs_angle_tolerance_from_base`) are correct: brute-force every
    /// corner of the full base x dia x depth x angle cartesian product
    /// via `solve_countersink` directly, take the actual min/max of the
    /// derived dimension across all of them, and confirm it matches the
    /// closed-form formula exactly - not merely "looks plausible," and
    /// not assumed correct just because each formula's monotonicity
    /// argument sounds right in isolation.
    #[test]
    fn all_three_derived_tolerance_formulas_match_a_full_brute_force_cartesian_search() {
        let base = make_range(ToleranceMode::Limits, 0.499, 0.501, Some(0.5));
        let dia_tol = make_range(ToleranceMode::Limits, 0.615, 0.635, Some(0.625));
        let depth_tol = make_range(ToleranceMode::Limits, 0.115, 0.135, Some(0.125));
        let angle_tol = make_range(ToleranceMode::Limits, 92.0, 108.0, Some(100.0));

        let bases = [base.lower, base.upper];
        let dias = [dia_tol.lower, dia_tol.upper];
        let depths = [depth_tol.lower, depth_tol.upper];
        let angles = [angle_tol.lower, angle_tol.upper];

        // DepthAngle mode: brute-force every (base, depth, angle) corner,
        // solving dia fresh each time - compare against
        // cs_dia_tolerance_from_base's direct formula.
        let mut dia_min = f64::INFINITY;
        let mut dia_max = f64::NEG_INFINITY;
        for &b in &bases {
            for &d in &depths {
                for &a in &angles {
                    let solved = solve_countersink(CsMode::DepthAngle, 0.0, d, a, b);
                    dia_min = dia_min.min(solved.dia);
                    dia_max = dia_max.max(solved.dia);
                }
            }
        }
        let solved_nominal = solve_countersink(CsMode::DepthAngle, 0.0, depth_tol.nominal, angle_tol.nominal, base.nominal);
        let dia_result = cs_dia_tolerance_from_base(CsMode::DepthAngle, solved_nominal.dia, depth_tol.nominal, angle_tol.nominal, base, Some(depth_tol), Some(angle_tol));
        assert!((dia_result.lower - dia_min).abs() < 1e-9, "dia lower: formula {} vs brute force {}", dia_result.lower, dia_min);
        assert!((dia_result.upper - dia_max).abs() < 1e-9, "dia upper: formula {} vs brute force {}", dia_result.upper, dia_max);

        // DiaAngle mode: brute-force every (base, dia, angle) corner,
        // solving depth fresh each time.
        let mut depth_min = f64::INFINITY;
        let mut depth_max = f64::NEG_INFINITY;
        for &b in &bases {
            for &d in &dias {
                for &a in &angles {
                    let solved = solve_countersink(CsMode::DiaAngle, d, 0.0, a, b);
                    depth_min = depth_min.min(solved.depth);
                    depth_max = depth_max.max(solved.depth);
                }
            }
        }
        let solved_nominal = solve_countersink(CsMode::DiaAngle, dia_tol.nominal, 0.0, angle_tol.nominal, base.nominal);
        let depth_result = cs_depth_tolerance_from_base(CsMode::DiaAngle, solved_nominal.depth, dia_tol.nominal, angle_tol.nominal, base, None, Some(dia_tol), Some(angle_tol));
        assert!((depth_result.lower - depth_min).abs() < 1e-9, "depth lower: formula {} vs brute force {}", depth_result.lower, depth_min);
        assert!((depth_result.upper - depth_max).abs() < 1e-9, "depth upper: formula {} vs brute force {}", depth_result.upper, depth_max);

        // DiaDepth mode: brute-force every (base, dia, depth) corner,
        // solving angle fresh each time.
        let mut angle_min = f64::INFINITY;
        let mut angle_max = f64::NEG_INFINITY;
        for &b in &bases {
            for &d in &dias {
                for &dp in &depths {
                    let solved = solve_countersink(CsMode::DiaDepth, d, dp, 0.0, b);
                    angle_min = angle_min.min(solved.angle_deg);
                    angle_max = angle_max.max(solved.angle_deg);
                }
            }
        }
        let solved_nominal = solve_countersink(CsMode::DiaDepth, dia_tol.nominal, depth_tol.nominal, 0.0, base.nominal);
        let angle_result = cs_angle_tolerance_from_base(CsMode::DiaDepth, solved_nominal.angle_deg, dia_tol.nominal, depth_tol.nominal, base, Some(dia_tol), Some(depth_tol));
        assert!((angle_result.lower - angle_min).abs() < 1e-6, "angle lower: formula {} vs brute force {}", angle_result.lower, angle_min);
        assert!((angle_result.upper - angle_max).abs() < 1e-6, "angle upper: formula {} vs brute force {}", angle_result.upper, angle_max);
    }
}
