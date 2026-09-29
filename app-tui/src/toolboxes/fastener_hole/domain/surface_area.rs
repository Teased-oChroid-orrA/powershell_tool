//! Countersink lateral (conical-frustum) surface area - spec section 14.
//! Always calculated, always non-editable; the domain layer only produces
//! the number, the UI layer enforces the read-only presentation.

use super::errors::GeometryError;
use super::tolerance::TolerancedValue;

fn validate_diameter(x: f64) -> Result<f64, GeometryError> {
    if !x.is_finite() {
        Err(GeometryError::NonFiniteInput)
    } else if x <= 0.0 {
        Err(GeometryError::InvalidDiameter)
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

/// `A = pi*(D^2 - d^2) / (4*sin(theta/2))` - the reduced form that needs
/// no separately-supplied depth (spec section 14). This is the canonical
/// implementation every caller in this crate uses.
pub fn lateral_surface_area(outer_diameter: f64, hole_diameter: f64, angle_deg: f64) -> Result<f64, GeometryError> {
    let outer = validate_diameter(outer_diameter)?;
    let hole = validate_diameter(hole_diameter)?;
    if outer <= hole {
        return Err(GeometryError::OuterDiameterNotGreaterThanHole);
    }
    let angle = validate_angle(angle_deg)?;
    let half_angle_sin = (angle.to_radians() / 2.0).sin();
    if !half_angle_sin.is_finite() || half_angle_sin.abs() < 1e-12 {
        return Err(GeometryError::ImpossibleGeometry);
    }
    let area = std::f64::consts::PI * (outer * outer - hole * hole) / (4.0 * half_angle_sin);
    if !area.is_finite() || area <= 0.0 {
        return Err(GeometryError::ImpossibleGeometry);
    }
    Ok(area)
}

/// `A = pi*(R+r)*s`, `s = sqrt((R-r)^2 + h^2)` - the slant-height form used
/// only as an independent cross-check against [`lateral_surface_area`]
/// (spec section 39: "verify consistency between" the two forms), since it
/// additionally requires a self-consistent depth rather than just D/d/theta.
pub fn lateral_surface_area_from_slant(outer_diameter: f64, hole_diameter: f64, depth: f64) -> Result<f64, GeometryError> {
    let outer = validate_diameter(outer_diameter)?;
    let hole = validate_diameter(hole_diameter)?;
    if outer <= hole {
        return Err(GeometryError::OuterDiameterNotGreaterThanHole);
    }
    if !depth.is_finite() || depth <= 0.0 {
        return Err(GeometryError::InvalidDepth);
    }
    let r_outer = outer / 2.0;
    let r_hole = hole / 2.0;
    let slant = ((r_outer - r_hole).powi(2) + depth * depth).sqrt();
    let area = std::f64::consts::PI * (r_outer + r_hole) * slant;
    if !area.is_finite() || area <= 0.0 {
        return Err(GeometryError::ImpossibleGeometry);
    }
    Ok(area)
}

/// Toleranced lateral area for a fully-solved countersink: nominal from
/// the geometry's own nominal D/d/theta, band from the `2^3 = 8` corners
/// of the geometry's three toleranced dimensions - never a bare copy of
/// one representative value (spec section 14/39).
pub fn lateral_surface_area_toleranced(
    outer_diameter: &TolerancedValue,
    hole_diameter: &TolerancedValue,
    angle_deg: &TolerancedValue,
) -> Result<TolerancedValue, GeometryError> {
    let nominal = lateral_surface_area(outer_diameter.nominal, hole_diameter.nominal, angle_deg.nominal)?;

    let mut min = f64::INFINITY;
    let mut max = f64::NEG_INFINITY;
    let mut any_valid = false;
    for &outer in &[outer_diameter.min, outer_diameter.max] {
        for &hole in &[hole_diameter.min, hole_diameter.max] {
            for &angle in &[angle_deg.min, angle_deg.max] {
                if let Ok(a) = lateral_surface_area(outer, hole, angle) {
                    any_valid = true;
                    min = min.min(a);
                    max = max.max(a);
                }
            }
        }
    }
    if !any_valid {
        return Err(GeometryError::ImpossibleGeometry);
    }
    TolerancedValue::from_bounds_with_nominal(nominal, min, max)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reduced_form_matches_the_slant_height_form_for_a_self_consistent_countersink() {
        // D=0.625, d=0.25, theta=100deg -> h = (D-d)/(2 tan(theta/2)).
        let outer = 0.625;
        let hole = 0.25;
        let angle = 100.0;
        let half_tan = (angle / 2.0_f64).to_radians().tan();
        let depth = (outer - hole) / (2.0 * half_tan);

        let a_reduced = lateral_surface_area(outer, hole, angle).unwrap();
        let a_slant = lateral_surface_area_from_slant(outer, hole, depth).unwrap();
        assert!((a_reduced - a_slant).abs() < 1e-9, "reduced={a_reduced} slant={a_slant}");
    }

    #[test]
    fn area_is_positive_and_finite_for_valid_geometry() {
        let area = lateral_surface_area(0.5, 0.25, 90.0).unwrap();
        assert!(area.is_finite() && area > 0.0);
    }

    #[test]
    fn invalid_geometry_is_rejected() {
        assert_eq!(lateral_surface_area(0.25, 0.5, 90.0), Err(GeometryError::OuterDiameterNotGreaterThanHole));
        assert_eq!(lateral_surface_area(0.5, 0.25, 0.0), Err(GeometryError::InvalidAngle));
        assert_eq!(lateral_surface_area(0.5, 0.25, 180.0), Err(GeometryError::InvalidAngle));
        assert_eq!(lateral_surface_area(-1.0, 0.25, 90.0), Err(GeometryError::InvalidDiameter));
    }

    #[test]
    fn toleranced_area_has_a_band_bracketing_the_nominal() {
        let outer = TolerancedValue::from_nominal_tol(0.5, 0.005, 0.005).unwrap();
        let hole = TolerancedValue::from_nominal_tol(0.25, 0.002, 0.002).unwrap();
        let angle = TolerancedValue::from_nominal_tol(90.0, 2.0, 2.0).unwrap();
        let area = lateral_surface_area_toleranced(&outer, &hole, &angle).unwrap();
        assert!(area.min <= area.nominal && area.nominal <= area.max);
        assert!(area.min < area.max, "a toleranced input must produce a nonzero-width area band");
    }
}
