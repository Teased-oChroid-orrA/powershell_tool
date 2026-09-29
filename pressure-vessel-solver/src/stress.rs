//! Full applicable stress-state evaluation - issue #11's core mandate:
//! "the full applicable Lame stress equations as the authoritative stress
//! solution for both thin-wall and thick-wall scenarios." Every function
//! here calls straight into `mechanics_core::lame` for radial/hoop stress
//! - `crate::geometry::classify`'s thin/thick label never appears in this
//! module's control flow, by design (see `geometry.rs`'s own doc
//! comment).

use crate::geometry::CylinderGeometry;
use crate::pressure::{EndCondition, PressureLoading};
use crate::thermal::ThermalLoading;

/// The full stress state at one radius: radial, hoop (circumferential),
/// and axial (longitudinal). Sign convention matches
/// `mechanics_core::lame`'s own: tension positive, compressive pressure
/// appears as negative radial stress.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StressState {
    pub radial: f64,
    pub hoop: f64,
    pub axial: f64,
    pub radius: f64,
}

/// Evaluates the full stress state at an arbitrary radius within the
/// wall. `r` is not validated against `geometry`'s bounds here - callers
/// that need a specific, meaningful location use
/// [`stress_at_inner_surface`]/[`stress_at_outer_surface`] instead, which
/// can't get the radius wrong.
pub fn stress_at_radius(geometry: &CylinderGeometry, pressure: &PressureLoading, r: f64) -> StressState {
    let (radial, hoop) =
        mechanics_core::lame::lame_stress_at_radius(r, geometry.inner_radius, geometry.outer_radius, pressure.internal_pressure, pressure.external_pressure);
    let axial = match pressure.end_condition {
        EndCondition::Closed => {
            mechanics_core::lame::closed_end_axial_stress(geometry.inner_radius, geometry.outer_radius, pressure.internal_pressure, pressure.external_pressure)
        }
        EndCondition::Open => 0.0,
    };
    StressState { radial, hoop, axial, radius: r }
}

/// The inner surface - issue #11's "at minimum investigate and support
/// evaluation at: Inner surface, Outer surface."
pub fn stress_at_inner_surface(geometry: &CylinderGeometry, pressure: &PressureLoading) -> StressState {
    stress_at_radius(geometry, pressure, geometry.inner_radius)
}

pub fn stress_at_outer_surface(geometry: &CylinderGeometry, pressure: &PressureLoading) -> StressState {
    stress_at_radius(geometry, pressure, geometry.outer_radius)
}

/// Superposes thermal stress (if any) onto the pressure-only Lamé stress
/// at the same radius - exact linear superposition of two independent
/// load cases on the same geometry, not an approximation. Axial stress is
/// left untouched by the thermal contribution: `thermal.rs`'s own doc
/// comment states plainly that axial thermal stress is out of scope for
/// this crate (both the free-ends and constrained-ends cases need an
/// additional boundary assumption this crate does not take a position on)
/// - this function does not silently guess at one.
pub fn stress_at_radius_with_thermal(geometry: &CylinderGeometry, pressure: &PressureLoading, thermal: Option<&ThermalLoading>, r: f64) -> StressState {
    let mut state = stress_at_radius(geometry, pressure, r);
    if let Some(t) = thermal {
        let thermal_stress = t.stress_at_radius(geometry, r);
        state.radial += thermal_stress.radial;
        state.hoop += thermal_stress.hoop;
    }
    state
}

pub fn stress_at_inner_surface_with_thermal(geometry: &CylinderGeometry, pressure: &PressureLoading, thermal: Option<&ThermalLoading>) -> StressState {
    stress_at_radius_with_thermal(geometry, pressure, thermal, geometry.inner_radius)
}

pub fn stress_at_outer_surface_with_thermal(geometry: &CylinderGeometry, pressure: &PressureLoading, thermal: Option<&ThermalLoading>) -> StressState {
    stress_at_radius_with_thermal(geometry, pressure, thermal, geometry.outer_radius)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(actual: f64, expected: f64, label: &str) {
        let diff = (actual - expected).abs();
        let tol = expected.abs() * 1e-9 + 1e-9;
        assert!(diff <= tol, "{label}: expected {expected}, got {actual} (diff {diff})");
    }

    /// Standard reference case (Shigley's Mechanical Engineering Design,
    /// thick-wall cylinder worked example): ID 4 in (a=2), OD 6 in (b=3),
    /// 5000 psi internal pressure, no external pressure, closed ends.
    /// Textbook values: max hoop (tangential) stress at the inner surface
    /// = 13000 psi, axial stress = 4000 psi, radial stress at the inner
    /// surface = -5000 psi (equals -p_internal exactly, the boundary
    /// condition).
    #[test]
    fn matches_the_shigley_thick_wall_reference_case_at_the_inner_surface() {
        let geometry = CylinderGeometry::new(2.0, 3.0).unwrap();
        let pressure = PressureLoading::new(5000.0, 0.0, EndCondition::Closed).unwrap();
        let s = stress_at_inner_surface(&geometry, &pressure);
        close(s.radial, -5000.0, "radial at inner surface");
        close(s.hoop, 13000.0, "hoop at inner surface");
        close(s.axial, 4000.0, "axial");
        assert_eq!(s.radius, 2.0);
    }

    #[test]
    fn radial_stress_at_each_surface_satisfies_its_own_boundary_condition() {
        let geometry = CylinderGeometry::new(1.5, 2.5).unwrap();
        let pressure = PressureLoading::new(3000.0, 800.0, EndCondition::Closed).unwrap();
        let inner = stress_at_inner_surface(&geometry, &pressure);
        let outer = stress_at_outer_surface(&geometry, &pressure);
        close(inner.radial, -3000.0, "sigma_r(a) must equal -p_internal exactly");
        close(outer.radial, -800.0, "sigma_r(b) must equal -p_external exactly");
    }

    #[test]
    fn open_end_condition_produces_zero_axial_stress_regardless_of_pressure() {
        let geometry = CylinderGeometry::new(2.0, 3.0).unwrap();
        let pressure = PressureLoading::new(9000.0, 200.0, EndCondition::Open).unwrap();
        let s = stress_at_inner_surface(&geometry, &pressure);
        assert_eq!(s.axial, 0.0);
    }

    #[test]
    fn hoop_stress_is_higher_at_the_inner_surface_than_the_outer_under_internal_pressure_only() {
        // A basic, well-known qualitative fact about thick cylinders
        // under internal pressure - hoop stress is maximum at the bore
        // and decreases outward. A regression here would mean the field
        // is inverted or flat, either a real bug.
        let geometry = CylinderGeometry::new(2.0, 3.0).unwrap();
        let pressure = PressureLoading::new(5000.0, 0.0, EndCondition::Closed).unwrap();
        let inner = stress_at_inner_surface(&geometry, &pressure);
        let outer = stress_at_outer_surface(&geometry, &pressure);
        assert!(inner.hoop > outer.hoop, "expected hoop stress to decrease from bore to OD under internal-only pressure");
    }

    #[test]
    fn with_thermal_none_reproduces_the_pressure_only_result_exactly() {
        let geometry = CylinderGeometry::new(2.0, 3.0).unwrap();
        let pressure = PressureLoading::new(5000.0, 0.0, EndCondition::Closed).unwrap();
        assert_eq!(stress_at_inner_surface_with_thermal(&geometry, &pressure, None), stress_at_inner_surface(&geometry, &pressure));
        assert_eq!(stress_at_outer_surface_with_thermal(&geometry, &pressure, None), stress_at_outer_surface(&geometry, &pressure));
    }

    #[test]
    fn with_thermal_some_adds_thermal_radial_and_hoop_but_leaves_axial_untouched() {
        let geometry = CylinderGeometry::new(2.0, 3.0).unwrap();
        let pressure = PressureLoading::new(5000.0, 0.0, EndCondition::Closed).unwrap();
        let thermal = crate::thermal::ThermalLoading { delta_t: 200.0, alpha_per_f: 12.8e-6, e_psi: 10_300_000.0, nu: 0.33 };

        let pressure_only = stress_at_inner_surface(&geometry, &pressure);
        let combined = stress_at_inner_surface_with_thermal(&geometry, &pressure, Some(&thermal));
        let thermal_only = thermal.stress_at_radius(&geometry, geometry.inner_radius);

        close(combined.radial, pressure_only.radial + thermal_only.radial, "combined radial");
        close(combined.hoop, pressure_only.hoop + thermal_only.hoop, "combined hoop");
        assert_eq!(combined.axial, pressure_only.axial, "axial must be untouched by thermal (out of scope, see thermal.rs)");
    }

    #[test]
    fn external_pressure_only_places_the_wall_in_compression() {
        let geometry = CylinderGeometry::new(2.0, 3.0).unwrap();
        let pressure = PressureLoading::new(0.0, 1000.0, EndCondition::Closed).unwrap();
        let inner = stress_at_inner_surface(&geometry, &pressure);
        assert!(inner.hoop < 0.0, "expected compressive hoop stress under external-only pressure, got {}", inner.hoop);
    }
}
