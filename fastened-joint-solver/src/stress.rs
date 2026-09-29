//! Fastener stress state - axial, torsional, combined von Mises, and full
//! plane-stress principal-stress transformation (spec sections 28-31).
//! Never collapses to a single generic "bolt stress" - callers evaluate
//! this at each relevant section (shank, tensile-stress area, root) with
//! that section's own diameter/area.

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StressState {
    pub axial: f64,
    pub torsional_shear: f64,
}

/// `sigma = F/A`.
pub fn axial_stress(force: f64, area: f64) -> f64 {
    if area <= 0.0 {
        return 0.0;
    }
    force / area
}

/// `tau = T*c/J` for a circular section (`c` the local radius) - spec
/// section 28's `tau(r) = T*r/J` evaluated at the outer fiber `r=c`.
pub fn torsional_shear_stress(torque: f64, radius: f64, polar_moment: f64) -> f64 {
    if polar_moment <= 0.0 {
        return 0.0;
    }
    torque * radius / polar_moment
}

/// `sigma_vm = sqrt(sigma^2 + 3*tau^2)` (spec section 30).
pub fn von_mises_stress(state: StressState) -> f64 {
    (state.axial * state.axial + 3.0 * state.torsional_shear * state.torsional_shear).sqrt()
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PrincipalStresses {
    pub sigma1: f64,
    pub sigma2: f64,
    /// Maximum in-plane shear stress, `(sigma1-sigma2)/2` - a natural
    /// byproduct of the same transformation, useful alongside the two
    /// principal values for a Tresca-style check.
    pub max_shear: f64,
}

/// Full plane-stress principal-stress transformation for
/// `sigma_x = axial`, `tau_xy = torsional_shear`, `sigma_y = 0` (spec
/// section 31): `sigma_1,2 = sigma_x/2 +- sqrt((sigma_x/2)^2 + tau_xy^2)`.
pub fn principal_stresses(state: StressState) -> PrincipalStresses {
    let half = state.axial / 2.0;
    let radius = (half * half + state.torsional_shear * state.torsional_shear).sqrt();
    PrincipalStresses { sigma1: half + radius, sigma2: half - radius, max_shear: radius }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn axial_stress_is_force_over_area() {
        assert!((axial_stress(10000.0, 50.0) - 200.0).abs() < 1e-9);
    }

    #[test]
    fn axial_stress_on_degenerate_area_is_zero_not_infinite() {
        assert_eq!(axial_stress(10000.0, 0.0), 0.0);
    }

    #[test]
    fn von_mises_with_zero_shear_equals_the_axial_stress() {
        let state = StressState { axial: 250.0, torsional_shear: 0.0 };
        assert!((von_mises_stress(state) - 250.0).abs() < 1e-9);
    }

    #[test]
    fn von_mises_with_pure_shear_is_sqrt3_times_shear() {
        let state = StressState { axial: 0.0, torsional_shear: 100.0 };
        assert!((von_mises_stress(state) - 100.0 * 3.0_f64.sqrt()).abs() < 1e-9);
    }

    #[test]
    fn principal_stresses_with_zero_shear_reduce_to_axial_and_zero() {
        let state = StressState { axial: 300.0, torsional_shear: 0.0 };
        let p = principal_stresses(state);
        assert!((p.sigma1 - 300.0).abs() < 1e-9);
        assert!((p.sigma2 - 0.0).abs() < 1e-9);
    }

    #[test]
    fn principal_stresses_satisfy_the_invariant_sum_equals_sigma_x() {
        let state = StressState { axial: 250.0, torsional_shear: 80.0 };
        let p = principal_stresses(state);
        assert!((p.sigma1 + p.sigma2 - state.axial).abs() < 1e-9);
    }

    #[test]
    fn max_shear_matches_half_the_principal_stress_difference() {
        let state = StressState { axial: 250.0, torsional_shear: 80.0 };
        let p = principal_stresses(state);
        assert!((p.max_shear - (p.sigma1 - p.sigma2) / 2.0).abs() < 1e-9);
    }
}
