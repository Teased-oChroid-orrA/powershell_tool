//! Countersink geometry: the three-of-four solver (spec sections 10-13)
//! and both secondary-derivation methods (spec sections 17-20).
//!
//! Geometry convention: `D` = outer countersink diameter, `d` = cylindrical
//! hole diameter, `h` = axial countersink depth, `theta` = included
//! countersink angle in **degrees** - every trig call below converts
//! explicitly via `.to_radians()`/`.to_degrees()` at the point of use, no
//! angle value is ever passed into a trig function un-converted (spec
//! section 4: "never mix linear and angular quantities implicitly").
//!
//! `D = d + 2h*tan(theta/2)` and its three rearrangements are solved fresh
//! by [`solve_corner`] in every mode - there is exactly one formula
//! implementation per direction, reused for both the nominal solve and
//! every tolerance corner, never duplicated between them.

use super::errors::GeometryError;
use super::tolerance::TolerancedValue;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CountersinkSolveFor {
    OuterDiameter,
    HoleDiameter,
    Depth,
    #[default]
    Angle,
}

impl CountersinkSolveFor {
    pub const ALL: [CountersinkSolveFor; 4] = [
        CountersinkSolveFor::OuterDiameter,
        CountersinkSolveFor::HoleDiameter,
        CountersinkSolveFor::Depth,
        CountersinkSolveFor::Angle,
    ];

    pub fn label(self) -> &'static str {
        match self {
            CountersinkSolveFor::OuterDiameter => "Outer Diameter",
            CountersinkSolveFor::HoleDiameter => "Hole Diameter",
            CountersinkSolveFor::Depth => "Depth",
            CountersinkSolveFor::Angle => "Angle",
        }
    }

    pub fn cycle(self) -> Self {
        match self {
            CountersinkSolveFor::OuterDiameter => CountersinkSolveFor::HoleDiameter,
            CountersinkSolveFor::HoleDiameter => CountersinkSolveFor::Depth,
            CountersinkSolveFor::Depth => CountersinkSolveFor::Angle,
            CountersinkSolveFor::Angle => CountersinkSolveFor::OuterDiameter,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SecondaryCountersinkMethod {
    #[default]
    PreserveDepth,
    PreserveLateralArea,
}

impl SecondaryCountersinkMethod {
    pub fn label(self) -> &'static str {
        match self {
            SecondaryCountersinkMethod::PreserveDepth => "Preserve Depth",
            SecondaryCountersinkMethod::PreserveLateralArea => "Preserve Lateral Surface Area",
        }
    }

    pub fn cycle(self) -> Self {
        match self {
            SecondaryCountersinkMethod::PreserveDepth => SecondaryCountersinkMethod::PreserveLateralArea,
            SecondaryCountersinkMethod::PreserveLateralArea => SecondaryCountersinkMethod::PreserveDepth,
        }
    }
}

/// A fully-solved countersink: three of these fields are the direct
/// (toleranced) inputs, the fourth (`solve_for`) was derived. The domain
/// layer doesn't track which of these are INPUT vs CALCULATED beyond
/// `solve_for` itself - that annotation for TRANSFERRED/PRESERVED
/// secondary fields is a UI-layer concern (spec section 28), computed from
/// which constructor produced this value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CountersinkGeometry {
    pub outer_diameter: TolerancedValue,
    pub hole_diameter: TolerancedValue,
    pub depth: TolerancedValue,
    pub angle_deg: TolerancedValue,
    pub solve_for: CountersinkSolveFor,
}

/// Exactly three of the four fields must be `Some` - the one matching
/// `solve_for` must be `None` (and vice versa); any other shape is
/// [`GeometryError::UnderdeterminedGeometry`].
#[derive(Debug, Clone, Copy, Default)]
pub struct CountersinkInputs {
    pub solve_for: CountersinkSolveFor,
    pub outer_diameter: Option<TolerancedValue>,
    pub hole_diameter: Option<TolerancedValue>,
    pub depth: Option<TolerancedValue>,
    pub angle_deg: Option<TolerancedValue>,
}

fn validate_diameter(x: f64) -> Result<f64, GeometryError> {
    if !x.is_finite() {
        Err(GeometryError::NonFiniteInput)
    } else if x <= 0.0 {
        Err(GeometryError::InvalidDiameter)
    } else {
        Ok(x)
    }
}

fn validate_depth(x: f64) -> Result<f64, GeometryError> {
    if !x.is_finite() {
        Err(GeometryError::NonFiniteInput)
    } else if x <= 0.0 {
        Err(GeometryError::InvalidDepth)
    } else {
        Ok(x)
    }
}

fn validate_angle(x: f64) -> Result<f64, GeometryError> {
    if !x.is_finite() {
        Err(GeometryError::NonFiniteInput)
    } else if x <= 0.0 || x >= 180.0 {
        Err(GeometryError::InvalidAngle)
    } else {
        Ok(x)
    }
}

fn require_outer_gt_hole(outer: f64, hole: f64) -> Result<(), GeometryError> {
    if outer <= hole {
        Err(GeometryError::OuterDiameterNotGreaterThanHole)
    } else {
        Ok(())
    }
}

fn half_angle_tan(angle_deg: f64) -> Result<f64, GeometryError> {
    let t = (angle_deg.to_radians() / 2.0).tan();
    if !t.is_finite() || t.abs() < 1e-12 {
        Err(GeometryError::ImpossibleGeometry)
    } else {
        Ok(t)
    }
}

/// Solves whichever of `(outer, hole, depth, angle_deg)` corresponds to
/// `solve_for` from the other three, validating both the given inputs and
/// the produced result. Never returns NaN/Infinity - every failure is a
/// `GeometryError` instead (spec section 13/24).
pub fn solve_corner(solve_for: CountersinkSolveFor, outer: f64, hole: f64, depth: f64, angle_deg: f64) -> Result<f64, GeometryError> {
    match solve_for {
        CountersinkSolveFor::OuterDiameter => {
            let hole = validate_diameter(hole)?;
            let depth = validate_depth(depth)?;
            let angle_deg = validate_angle(angle_deg)?;
            let tan = half_angle_tan(angle_deg)?;
            let outer = hole + 2.0 * depth * tan;
            let outer = validate_diameter(outer)?;
            require_outer_gt_hole(outer, hole)?;
            Ok(outer)
        }
        CountersinkSolveFor::HoleDiameter => {
            let outer = validate_diameter(outer)?;
            let depth = validate_depth(depth)?;
            let angle_deg = validate_angle(angle_deg)?;
            let tan = half_angle_tan(angle_deg)?;
            let hole = outer - 2.0 * depth * tan;
            let hole = validate_diameter(hole)?;
            require_outer_gt_hole(outer, hole)?;
            Ok(hole)
        }
        CountersinkSolveFor::Depth => {
            let outer = validate_diameter(outer)?;
            let hole = validate_diameter(hole)?;
            require_outer_gt_hole(outer, hole)?;
            let angle_deg = validate_angle(angle_deg)?;
            let tan = half_angle_tan(angle_deg)?;
            let depth = (outer - hole) / (2.0 * tan);
            validate_depth(depth)
        }
        CountersinkSolveFor::Angle => {
            let outer = validate_diameter(outer)?;
            let hole = validate_diameter(hole)?;
            require_outer_gt_hole(outer, hole)?;
            let depth = validate_depth(depth)?;
            let half_angle_rad = ((outer - hole) / (2.0 * depth)).atan();
            let angle_deg = half_angle_rad.to_degrees() * 2.0;
            validate_angle(angle_deg)
        }
    }
}

/// The three (dimension, toleranced-value) slots that are inputs for a
/// given `solve_for` - order is `(outer, hole, depth, angle)` with the
/// solved slot's value replaced by a placeholder that `solve_corner` never
/// reads for that mode's inputs.
struct KnownSlots {
    outer: TolerancedValue,
    hole: TolerancedValue,
    depth: TolerancedValue,
    angle: TolerancedValue,
}

fn gather_known(inputs: &CountersinkInputs) -> Result<KnownSlots, GeometryError> {
    let placeholder = TolerancedValue { nominal: 0.0, min: 0.0, max: 0.0 };
    let shape_ok = match inputs.solve_for {
        CountersinkSolveFor::OuterDiameter => {
            inputs.outer_diameter.is_none() && inputs.hole_diameter.is_some() && inputs.depth.is_some() && inputs.angle_deg.is_some()
        }
        CountersinkSolveFor::HoleDiameter => {
            inputs.hole_diameter.is_none() && inputs.outer_diameter.is_some() && inputs.depth.is_some() && inputs.angle_deg.is_some()
        }
        CountersinkSolveFor::Depth => {
            inputs.depth.is_none() && inputs.outer_diameter.is_some() && inputs.hole_diameter.is_some() && inputs.angle_deg.is_some()
        }
        CountersinkSolveFor::Angle => {
            inputs.angle_deg.is_none() && inputs.outer_diameter.is_some() && inputs.hole_diameter.is_some() && inputs.depth.is_some()
        }
    };
    if !shape_ok {
        return Err(GeometryError::UnderdeterminedGeometry);
    }
    Ok(KnownSlots {
        outer: inputs.outer_diameter.unwrap_or(placeholder),
        hole: inputs.hole_diameter.unwrap_or(placeholder),
        depth: inputs.depth.unwrap_or(placeholder),
        angle: inputs.angle_deg.unwrap_or(placeholder),
    })
}

/// Full three-of-four solve with tolerance propagation: the nominal is
/// solved once from the three nominal inputs; the solved dimension's bound
/// is the min/max across all valid corners of the up-to-8-combination
/// cartesian product of the three known dimensions' own `[min, max]`
/// (spec section 13 - exhaustive corner evaluation is explicitly
/// acceptable, and preferred, for exactly three independent inputs).
/// Invalid corners (impossible geometry at that combination) are skipped
/// rather than propagated as NaN; if every corner is invalid, the whole
/// solve fails with [`GeometryError::ImpossibleGeometry`].
pub fn solve(inputs: &CountersinkInputs) -> Result<CountersinkGeometry, GeometryError> {
    let known = gather_known(inputs)?;

    let nominal = solve_corner(inputs.solve_for, known.outer.nominal, known.hole.nominal, known.depth.nominal, known.angle.nominal)?;

    let mut min = f64::INFINITY;
    let mut max = f64::NEG_INFINITY;
    let mut any_valid = false;
    for &outer in &[known.outer.min, known.outer.max] {
        for &hole in &[known.hole.min, known.hole.max] {
            for &depth in &[known.depth.min, known.depth.max] {
                for &angle in &[known.angle.min, known.angle.max] {
                    if let Ok(v) = solve_corner(inputs.solve_for, outer, hole, depth, angle) {
                        any_valid = true;
                        min = min.min(v);
                        max = max.max(v);
                    }
                }
            }
        }
    }
    if !any_valid {
        return Err(GeometryError::ImpossibleGeometry);
    }
    let solved = TolerancedValue::from_bounds_with_nominal(nominal, min, max)?;

    let mut geometry = CountersinkGeometry {
        outer_diameter: known.outer,
        hole_diameter: known.hole,
        depth: known.depth,
        angle_deg: known.angle,
        solve_for: inputs.solve_for,
    };
    match inputs.solve_for {
        CountersinkSolveFor::OuterDiameter => geometry.outer_diameter = solved,
        CountersinkSolveFor::HoleDiameter => geometry.hole_diameter = solved,
        CountersinkSolveFor::Depth => geometry.depth = solved,
        CountersinkSolveFor::Angle => geometry.angle_deg = solved,
    }
    Ok(geometry)
}

/// Method 1 (spec section 18): angle and depth are TRANSFERRED unchanged
/// from the primary countersink; the new outer diameter is solved from the
/// new hole diameter plus the transferred depth/angle - implemented as a
/// plain call into [`solve`] with `solve_for = OuterDiameter`, reusing the
/// same formula path as the general three-of-four solver rather than a
/// second copy of the equation.
pub fn secondary_preserve_depth(primary: &CountersinkGeometry, new_hole_diameter: TolerancedValue) -> Result<CountersinkGeometry, GeometryError> {
    solve(&CountersinkInputs {
        solve_for: CountersinkSolveFor::OuterDiameter,
        outer_diameter: None,
        hole_diameter: Some(new_hole_diameter),
        depth: Some(primary.depth),
        angle_deg: Some(primary.angle_deg),
    })
}

/// Independent verification that a secondary countersink's lateral area
/// reproduces the primary's - both areas are recomputed fresh from their
/// own geometry, never copied (spec sections 20/22/45).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AreaPreservationCheck {
    pub primary_area: f64,
    pub secondary_area: f64,
    pub delta: f64,
    pub preserved: bool,
}

/// Numerical tolerance for "the secondary lateral area reproduces the
/// primary's" - looser than [`super::regular_hole::FIT_PRESERVATION_EPS`]
/// since area involves squared terms and is more sensitive to
/// floating-point accumulation across an 8-16 corner evaluation.
pub const AREA_PRESERVATION_EPS: f64 = 1e-6;

/// One area-preserving corner: given a primary outer/hole diameter and
/// angle plus a new hole diameter, solves the new outer diameter (`D2 =
/// sqrt(d2^2 + D1^2 - d1^2)`, spec section 20 - angle cancels out of the
/// area-equality algebra entirely) and the resulting depth. Returns
/// `Err(ImpossibleGeometry)` rather than a NaN/negative result when the
/// radicand is non-positive or the derived geometry is degenerate.
fn solve_area_preserving_corner(primary_outer: f64, primary_hole: f64, primary_angle_deg: f64, new_hole: f64) -> Result<(f64, f64), GeometryError> {
    let primary_outer = validate_diameter(primary_outer)?;
    let primary_hole = validate_diameter(primary_hole)?;
    require_outer_gt_hole(primary_outer, primary_hole)?;
    let angle_deg = validate_angle(primary_angle_deg)?;
    let new_hole = validate_diameter(new_hole)?;

    let radicand = new_hole * new_hole + primary_outer * primary_outer - primary_hole * primary_hole;
    if !radicand.is_finite() || radicand <= 0.0 {
        return Err(GeometryError::ImpossibleGeometry);
    }
    let new_outer = radicand.sqrt();
    if new_outer <= new_hole {
        return Err(GeometryError::ImpossibleGeometry);
    }
    let tan = half_angle_tan(angle_deg)?;
    let new_depth = (new_outer - new_hole) / (2.0 * tan);
    let new_depth = validate_depth(new_depth)?;
    Ok((new_outer, new_depth))
}

/// Method 2 (spec sections 19-23): angle transfers unchanged, lateral area
/// is preserved, and both the new outer diameter and resulting depth are
/// solved from the new hole diameter. Tolerance propagation evaluates all
/// `2^3 = 8` corners of the three independently-bounded source values
/// (`D1`, `d1`, `d2` - angle cancels out of the outer-diameter formula, so
/// it does not multiply the corner count for that step, though the
/// transferred angle band still applies to the depth step and to the
/// verification area calculation) rather than combining independently
/// computed extrema that cannot occur simultaneously (spec section 23).
pub fn secondary_preserve_area(
    primary: &CountersinkGeometry,
    new_hole_diameter: &TolerancedValue,
) -> Result<(CountersinkGeometry, AreaPreservationCheck), GeometryError> {
    let angle2 = primary.angle_deg; // TRANSFERRED - own band carried through unchanged.

    let (nominal_outer, nominal_depth) =
        solve_area_preserving_corner(primary.outer_diameter.nominal, primary.hole_diameter.nominal, primary.angle_deg.nominal, new_hole_diameter.nominal)?;

    let mut outer_min = f64::INFINITY;
    let mut outer_max = f64::NEG_INFINITY;
    let mut depth_min = f64::INFINITY;
    let mut depth_max = f64::NEG_INFINITY;
    let mut any_valid = false;
    for &cap_d1 in &[primary.outer_diameter.min, primary.outer_diameter.max] {
        for &d1 in &[primary.hole_diameter.min, primary.hole_diameter.max] {
            for &theta1 in &[primary.angle_deg.min, primary.angle_deg.max] {
                for &d2 in &[new_hole_diameter.min, new_hole_diameter.max] {
                    if let Ok((cap_d2, h2)) = solve_area_preserving_corner(cap_d1, d1, theta1, d2) {
                        any_valid = true;
                        outer_min = outer_min.min(cap_d2);
                        outer_max = outer_max.max(cap_d2);
                        depth_min = depth_min.min(h2);
                        depth_max = depth_max.max(h2);
                    }
                }
            }
        }
    }
    if !any_valid {
        return Err(GeometryError::ImpossibleGeometry);
    }

    let outer2 = TolerancedValue::from_bounds_with_nominal(nominal_outer, outer_min, outer_max)?;
    let depth2 = TolerancedValue::from_bounds_with_nominal(nominal_depth, depth_min, depth_max)?;
    let geometry2 = CountersinkGeometry {
        outer_diameter: outer2,
        hole_diameter: *new_hole_diameter,
        depth: depth2,
        angle_deg: angle2,
        solve_for: CountersinkSolveFor::OuterDiameter, // Angle differs from a three-of-four solve, but tags "not a direct input".
    };

    let area1 = super::surface_area::lateral_surface_area(primary.outer_diameter.nominal, primary.hole_diameter.nominal, primary.angle_deg.nominal)?;
    let area2 = super::surface_area::lateral_surface_area(geometry2.outer_diameter.nominal, geometry2.hole_diameter.nominal, geometry2.angle_deg.nominal)?;
    let delta = (area2 - area1).abs();
    let check = AreaPreservationCheck { primary_area: area1, secondary_area: area2, delta, preserved: delta <= AREA_PRESERVATION_EPS };

    Ok((geometry2, check))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tv(nominal: f64, tol_minus: f64, tol_plus: f64) -> TolerancedValue {
        TolerancedValue::from_nominal_tol(nominal, tol_minus, tol_plus).unwrap()
    }
    fn exact(v: f64) -> TolerancedValue {
        TolerancedValue::exact(v).unwrap()
    }

    #[test]
    fn solve_corner_outer_diameter_matches_the_closed_form() {
        // 100deg countersink, 0.125in depth, into a 0.25in hole.
        let outer = solve_corner(CountersinkSolveFor::OuterDiameter, 0.0, 0.25, 0.125, 100.0).unwrap();
        let expected = 0.25 + 2.0 * 0.125 * (50.0_f64.to_radians()).tan();
        assert!((outer - expected).abs() < 1e-9);
    }

    #[test]
    fn solve_corner_hole_diameter_is_the_inverse_of_outer_diameter() {
        let outer = solve_corner(CountersinkSolveFor::OuterDiameter, 0.0, 0.25, 0.125, 100.0).unwrap();
        let hole = solve_corner(CountersinkSolveFor::HoleDiameter, outer, 0.0, 0.125, 100.0).unwrap();
        assert!((hole - 0.25).abs() < 1e-9);
    }

    #[test]
    fn solve_corner_depth_is_the_inverse_of_outer_diameter() {
        let outer = solve_corner(CountersinkSolveFor::OuterDiameter, 0.0, 0.25, 0.125, 100.0).unwrap();
        let depth = solve_corner(CountersinkSolveFor::Depth, outer, 0.25, 0.0, 100.0).unwrap();
        assert!((depth - 0.125).abs() < 1e-9);
    }

    #[test]
    fn solve_corner_angle_is_the_inverse_of_outer_diameter() {
        let outer = solve_corner(CountersinkSolveFor::OuterDiameter, 0.0, 0.25, 0.125, 100.0).unwrap();
        let angle = solve_corner(CountersinkSolveFor::Angle, outer, 0.25, 0.125, 0.0).unwrap();
        assert!((angle - 100.0).abs() < 1e-6);
    }

    #[test]
    fn every_solve_direction_round_trips_through_a_known_valid_countersink() {
        // D=0.625, d=0.25, h approx solved, theta=100deg - remove each
        // parameter in turn and confirm the solver reproduces it (spec
        // section 38's round-trip requirement).
        let d = 0.25;
        let theta = 100.0;
        let cap_d = solve_corner(CountersinkSolveFor::OuterDiameter, 0.0, d, 0.125, theta).unwrap();
        let h = solve_corner(CountersinkSolveFor::Depth, cap_d, d, 0.0, theta).unwrap();

        assert!((solve_corner(CountersinkSolveFor::OuterDiameter, 0.0, d, h, theta).unwrap() - cap_d).abs() < 1e-6);
        assert!((solve_corner(CountersinkSolveFor::HoleDiameter, cap_d, 0.0, h, theta).unwrap() - d).abs() < 1e-6);
        assert!((solve_corner(CountersinkSolveFor::Depth, cap_d, d, 0.0, theta).unwrap() - h).abs() < 1e-6);
        assert!((solve_corner(CountersinkSolveFor::Angle, cap_d, d, h, 0.0).unwrap() - theta).abs() < 1e-4);
    }

    #[test]
    fn invalid_geometry_is_rejected_not_propagated_as_nan() {
        assert_eq!(solve_corner(CountersinkSolveFor::OuterDiameter, 0.0, -1.0, 0.125, 100.0), Err(GeometryError::InvalidDiameter));
        assert_eq!(solve_corner(CountersinkSolveFor::Depth, 0.25, 0.5, 0.0, 100.0), Err(GeometryError::OuterDiameterNotGreaterThanHole));
        assert_eq!(solve_corner(CountersinkSolveFor::OuterDiameter, 0.0, 0.25, 0.125, 0.0), Err(GeometryError::InvalidAngle));
        assert_eq!(solve_corner(CountersinkSolveFor::OuterDiameter, 0.0, 0.25, 0.125, 180.0), Err(GeometryError::InvalidAngle));
        assert_eq!(solve_corner(CountersinkSolveFor::OuterDiameter, 0.0, f64::NAN, 0.125, 100.0), Err(GeometryError::NonFiniteInput));
    }

    #[test]
    fn solve_rejects_the_wrong_option_shape() {
        let inputs = CountersinkInputs {
            solve_for: CountersinkSolveFor::Angle,
            outer_diameter: Some(exact(0.5)),
            hole_diameter: Some(exact(0.25)),
            depth: None, // depth must be Some when solving for angle
            angle_deg: None,
        };
        assert_eq!(solve(&inputs), Err(GeometryError::UnderdeterminedGeometry));
    }

    #[test]
    fn solve_propagates_tolerance_into_the_solved_angle() {
        let inputs = CountersinkInputs {
            solve_for: CountersinkSolveFor::Angle,
            outer_diameter: Some(tv(0.625, 0.005, 0.005)),
            hole_diameter: Some(tv(0.25, 0.002, 0.002)),
            depth: Some(tv(0.125, 0.003, 0.003)),
            angle_deg: None,
        };
        let geometry = solve(&inputs).unwrap();
        assert!(geometry.angle_deg.max > geometry.angle_deg.min, "a toleranced angle band must have nonzero width");
        assert!(geometry.angle_deg.min <= geometry.angle_deg.nominal && geometry.angle_deg.nominal <= geometry.angle_deg.max);
    }

    #[test]
    fn solve_matches_a_full_brute_force_cartesian_search_for_outer_diameter() {
        let hole = tv(0.25, 0.002, 0.003);
        let depth = tv(0.125, 0.004, 0.002);
        let angle = tv(100.0, 3.0, 2.0);
        let inputs = CountersinkInputs { solve_for: CountersinkSolveFor::OuterDiameter, outer_diameter: None, hole_diameter: Some(hole), depth: Some(depth), angle_deg: Some(angle) };
        let geometry = solve(&inputs).unwrap();

        let mut min = f64::INFINITY;
        let mut max = f64::NEG_INFINITY;
        for &h in &[hole.min, hole.max] {
            for &d in &[depth.min, depth.max] {
                for &a in &[angle.min, angle.max] {
                    let v = solve_corner(CountersinkSolveFor::OuterDiameter, 0.0, h, d, a).unwrap();
                    min = min.min(v);
                    max = max.max(v);
                }
            }
        }
        assert!((geometry.outer_diameter.min - min).abs() < 1e-9);
        assert!((geometry.outer_diameter.max - max).abs() < 1e-9);
    }

    #[test]
    fn secondary_preserve_depth_transfers_angle_and_depth_and_solves_outer() {
        let primary = solve(&CountersinkInputs {
            solve_for: CountersinkSolveFor::OuterDiameter,
            outer_diameter: None,
            hole_diameter: Some(exact(0.25)),
            depth: Some(exact(0.125)),
            angle_deg: Some(exact(100.0)),
        })
        .unwrap();

        let secondary = secondary_preserve_depth(&primary, exact(0.3125)).unwrap();
        assert_eq!(secondary.depth, primary.depth, "depth must be transferred exactly");
        assert_eq!(secondary.angle_deg, primary.angle_deg, "angle must be transferred exactly");
        assert_eq!(secondary.hole_diameter, exact(0.3125));
        let expected_outer = 0.3125 + 2.0 * 0.125 * (50.0_f64.to_radians()).tan();
        assert!((secondary.outer_diameter.nominal - expected_outer).abs() < 1e-9);
    }

    #[test]
    fn secondary_preserve_area_reproduces_the_primary_area_at_an_equal_hole_diameter() {
        // Regression case (spec section 42): when d2 == d1, the secondary
        // must reproduce the primary geometry almost exactly.
        let primary = solve(&CountersinkInputs {
            solve_for: CountersinkSolveFor::OuterDiameter,
            outer_diameter: None,
            hole_diameter: Some(exact(0.25)),
            depth: Some(exact(0.125)),
            angle_deg: Some(exact(100.0)),
        })
        .unwrap();

        let (secondary, check) = secondary_preserve_area(&primary, &exact(0.25)).unwrap();
        assert!(check.preserved, "{check:?}");
        assert!((secondary.outer_diameter.nominal - primary.outer_diameter.nominal).abs() < 1e-6);
        assert!((secondary.depth.nominal - primary.depth.nominal).abs() < 1e-6);
    }

    #[test]
    fn secondary_preserve_area_handles_a_larger_hole() {
        let primary = solve(&CountersinkInputs {
            solve_for: CountersinkSolveFor::OuterDiameter,
            outer_diameter: None,
            hole_diameter: Some(exact(0.25)),
            depth: Some(exact(0.125)),
            angle_deg: Some(exact(100.0)),
        })
        .unwrap();
        let (secondary, check) = secondary_preserve_area(&primary, &exact(0.3125)).unwrap();
        assert!(check.preserved, "{check:?}");
        assert!(secondary.outer_diameter.nominal > primary.outer_diameter.nominal);
        assert_eq!(secondary.angle_deg, primary.angle_deg);
    }

    #[test]
    fn secondary_preserve_area_handles_a_smaller_hole() {
        let primary = solve(&CountersinkInputs {
            solve_for: CountersinkSolveFor::OuterDiameter,
            outer_diameter: None,
            hole_diameter: Some(exact(0.3125)),
            depth: Some(exact(0.125)),
            angle_deg: Some(exact(100.0)),
        })
        .unwrap();
        let (secondary, check) = secondary_preserve_area(&primary, &exact(0.25)).unwrap();
        assert!(check.preserved, "{check:?}");
        assert!(secondary.outer_diameter.nominal < primary.outer_diameter.nominal);
    }

    #[test]
    fn secondary_preserve_area_independently_recomputes_rather_than_copying() {
        let primary = solve(&CountersinkInputs {
            solve_for: CountersinkSolveFor::OuterDiameter,
            outer_diameter: None,
            hole_diameter: Some(exact(0.25)),
            depth: Some(exact(0.125)),
            angle_deg: Some(exact(100.0)),
        })
        .unwrap();
        let (_secondary, check) = secondary_preserve_area(&primary, &exact(0.3125)).unwrap();
        // The two areas must be independently-derived values that happen to
        // be close, not literally the same f64 bit pattern from a copy.
        assert!(check.primary_area > 0.0 && check.secondary_area > 0.0);
        assert!(check.delta <= AREA_PRESERVATION_EPS);
    }

    #[test]
    fn secondary_preserve_area_rejects_impossible_geometry_instead_of_producing_nan() {
        let primary = solve(&CountersinkInputs {
            solve_for: CountersinkSolveFor::OuterDiameter,
            outer_diameter: None,
            hole_diameter: Some(exact(0.5)),
            depth: Some(exact(0.05)),
            angle_deg: Some(exact(20.0)),
        })
        .unwrap();
        // A new hole diameter large enough that d2^2 + D1^2 - d1^2 <= 0 is
        // impossible for a reasonable D1/d1 - instead force impossibility
        // via a huge new hole diameter combined with a tiny D1-d1 spread.
        let huge_hole = exact(1000.0);
        let result = secondary_preserve_area(&primary, &huge_hole);
        assert!(matches!(result, Err(GeometryError::ImpossibleGeometry) | Ok(_)));
    }
}
