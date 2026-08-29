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
/// vary, and the derived one is always re-solved from them via
/// `solve_countersink` - never perturbed independently of its geometric
/// coupling to the others. Ported from `solveMath.ts:208-224`.
pub fn enumerate_countersink_corners(mode: CsMode, angle_deg: f64, base_dia: f64, dia_tol: ToleranceRange, depth_tol: ToleranceRange) -> Vec<CsCorner> {
    let dia_vals = if mode == CsMode::DepthAngle { vec![dia_tol.nominal] } else { tolerance_corners(dia_tol) };
    let depth_vals = if mode == CsMode::DiaAngle { vec![depth_tol.nominal] } else { tolerance_corners(depth_tol) };
    let mut corners = Vec::with_capacity(dia_vals.len() * depth_vals.len());
    for &dia in &dia_vals {
        for &depth in &depth_vals {
            corners.push(solve_countersink(mode, dia, depth, angle_deg, base_dia));
        }
    }
    corners
}

/// Propagates a resolved base-diameter tolerance range through to the
/// countersink diameter when `mode` derives dia from depth+angle - a
/// no-op passthrough (nominal-only range) for every other mode, matching
/// `solveMath.ts:159-173`.
pub fn cs_dia_tolerance_from_base(
    mode: CsMode,
    solved_dia: f64,
    solved_depth: f64,
    solved_angle_deg: f64,
    base: ToleranceRange,
    depth_tolerance: Option<ToleranceRange>,
) -> ToleranceRange {
    if mode != CsMode::DepthAngle {
        return make_range(ToleranceMode::NominalTol, solved_dia, solved_dia, Some(solved_dia));
    }
    let angle_rad = (solved_angle_deg / 2.0) * std::f64::consts::PI / 180.0;
    let depth = depth_tolerance.unwrap_or_else(|| make_range(ToleranceMode::NominalTol, solved_depth, solved_depth, Some(solved_depth)));
    let lower = base.lower + 2.0 * depth.lower.max(0.0) * angle_rad.tan();
    let upper = base.upper + 2.0 * depth.upper.max(0.0) * angle_rad.tan();
    make_range(base.mode, lower, upper, Some(solved_dia))
}

/// Propagates a resolved base-diameter tolerance range through to the
/// countersink depth when `mode` derives depth from dia+angle (or is
/// otherwise a direct input, in which case `explicit_depth_tolerance` -
/// the field's own resolved range - is returned unchanged). Matches
/// `solveMath.ts:175-196`.
pub fn cs_depth_tolerance_from_base(
    mode: CsMode,
    solved_depth: f64,
    solved_dia: f64,
    solved_angle_deg: f64,
    base: ToleranceRange,
    explicit_depth_tolerance: Option<ToleranceRange>,
    dia_tolerance: Option<ToleranceRange>,
) -> ToleranceRange {
    if mode == CsMode::DepthAngle || mode == CsMode::DiaDepth {
        return explicit_depth_tolerance.unwrap_or_else(|| make_range(ToleranceMode::NominalTol, solved_depth, solved_depth, Some(solved_depth)));
    }
    let angle_rad = (solved_angle_deg / 2.0) * std::f64::consts::PI / 180.0;
    let tan_half_angle = angle_rad.tan();
    if !tan_half_angle.is_finite() || tan_half_angle.abs() < 1e-12 {
        return make_range(ToleranceMode::NominalTol, solved_depth, solved_depth, Some(solved_depth));
    }
    let dia = dia_tolerance.unwrap_or_else(|| make_range(ToleranceMode::NominalTol, solved_dia, solved_dia, Some(solved_dia)));
    let lower = ((dia.lower - base.upper) / (2.0 * tan_half_angle)).max(0.0);
    let upper = ((dia.upper - base.lower) / (2.0 * tan_half_angle)).max(0.0);
    make_range(base.mode, lower, upper, Some(solved_depth))
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
        // depth_angle mode: dia is derived, so it must NOT vary across corners
        // even though a (degenerate, unused) dia_tol band was passed in.
        let corners = enumerate_countersink_corners(CsMode::DepthAngle, 100.0, 0.5, dia_tol, depth_tol);
        assert_eq!(corners.len(), 1, "depth is a single point and dia is derived, not enumerated");
    }

    #[test]
    fn enumerate_countersink_corners_produces_four_corners_when_both_dia_and_depth_vary() {
        let dia_tol = make_range(ToleranceMode::Limits, 0.62, 0.63, Some(0.625));
        let depth_tol = make_range(ToleranceMode::Limits, 0.12, 0.13, Some(0.125));
        let corners = enumerate_countersink_corners(CsMode::DiaDepth, 100.0, 0.5, dia_tol, depth_tol);
        assert_eq!(corners.len(), 4);
    }

    #[test]
    fn cs_dia_tolerance_from_base_propagates_base_and_depth_tolerance_in_depth_angle_mode() {
        let base = make_range(ToleranceMode::NominalTol, 0.5, 0.0, Some(0.5)); // exact base, no tol
        let depth_tol = make_range(ToleranceMode::NominalTol, 0.125, 0.005, Some(0.005));
        let result = cs_dia_tolerance_from_base(CsMode::DepthAngle, 0.6, 0.125, 100.0, base, Some(depth_tol));
        let tan_r = (50.0_f64).to_radians().tan();
        assert!((result.lower - (base.lower + 2.0 * depth_tol.lower * tan_r)).abs() < 1e-9);
        assert!((result.upper - (base.upper + 2.0 * depth_tol.upper * tan_r)).abs() < 1e-9);
    }

    #[test]
    fn cs_dia_tolerance_from_base_is_a_nominal_passthrough_outside_depth_angle_mode() {
        let base = make_range(ToleranceMode::Limits, 0.49, 0.51, Some(0.5));
        let result = cs_dia_tolerance_from_base(CsMode::DiaDepth, 0.6, 0.125, 100.0, base, None);
        assert_eq!(result.lower, 0.6);
        assert_eq!(result.upper, 0.6);
    }

    #[test]
    fn cs_depth_tolerance_from_base_derives_from_dia_in_dia_angle_mode() {
        let base = make_range(ToleranceMode::NominalTol, 0.5, 0.0, Some(0.5));
        let dia_tol = make_range(ToleranceMode::NominalTol, 0.625, 0.005, Some(0.005));
        let result = cs_depth_tolerance_from_base(CsMode::DiaAngle, 0.0625, 0.625, 100.0, base, None, Some(dia_tol));
        let tan_half = (50.0_f64).to_radians().tan();
        assert!((result.lower - ((dia_tol.lower - base.upper) / (2.0 * tan_half)).max(0.0)).abs() < 1e-9);
        assert!((result.upper - ((dia_tol.upper - base.lower) / (2.0 * tan_half)).max(0.0)).abs() < 1e-9);
    }

    #[test]
    fn cs_depth_tolerance_from_base_returns_the_explicit_range_in_depth_angle_mode() {
        let base = make_range(ToleranceMode::NominalTol, 0.5, 0.0, Some(0.5));
        let explicit = make_range(ToleranceMode::NominalTol, 0.125, 0.01, Some(0.005));
        let result = cs_depth_tolerance_from_base(CsMode::DepthAngle, 0.125, 0.6, 100.0, base, Some(explicit), None);
        assert_eq!(result, explicit);
    }
}
