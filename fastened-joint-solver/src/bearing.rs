//! Bearing (under-head/nut/washer) friction torque - spec sections 8-11.
//! Computed from the actual annular contact geometry (inner/outer radius),
//! never assumed to be a fixed mean diameter. Two closed-form contact-
//! pressure models are provided (uniform pressure, uniform wear), each
//! derived from the same general integral
//! `T = integral[ri, ro] mu * p(r) * r * 2*pi*r dr`
//! rather than looked up as a pre-reduced formula - [`bearing_torque_quadrature`]
//! evaluates that same integral numerically for either model, and this
//! module's own tests prove the closed forms match it (spec section 11:
//! "architect the solver so nonuniform pressure can later be represented
//! numerically").

use crate::quadrature::gauss_legendre_5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BearingPressureModel {
    /// `p(r) = F / (pi*(ro^2 - ri^2))` - constant over the annulus.
    UniformPressure,
    /// `p(r) = k/r` (constant wear rate, proportional to sliding
    /// velocity*pressure being constant - the other classical assumption
    /// alongside uniform pressure, spec section 10).
    UniformWear,
}

/// `p(r)` for `model`, normalized so `integral[ri,ro] p(r) 2*pi*r dr = F`
/// exactly - the same normalization both closed forms and
/// [`bearing_torque_quadrature`] use, so switching models never silently
/// changes the total clamping force represented.
fn pressure_at(model: BearingPressureModel, f: f64, r_i: f64, r_o: f64, r: f64) -> f64 {
    match model {
        BearingPressureModel::UniformPressure => f / (std::f64::consts::PI * (r_o * r_o - r_i * r_i)),
        BearingPressureModel::UniformWear => {
            // integral[ri,ro] (k/r) 2*pi*r dr = 2*pi*k*(ro-ri) = F -> k = F/(2*pi*(ro-ri))
            let k = f / (2.0 * std::f64::consts::PI * (r_o - r_i));
            k / r
        }
    }
}

/// Closed-form effective bearing radius for `model` - the radius at which
/// a point friction force `mu*F` would produce the same torque as the
/// actual distributed pressure. Uniform pressure: `r_eff = (2/3)(ro^3-ri^3)/(ro^2-ri^2)`
/// (spec section 9). Uniform wear: `r_eff = (ri+ro)/2`, derived the same
/// way (substitute `p(r)=k/r` into the general integral and simplify) -
/// this module's own brute-force quadrature test proves both, not just
/// the uniform-pressure case the spec worked out explicitly.
pub fn effective_radius(model: BearingPressureModel, r_i: f64, r_o: f64) -> f64 {
    match model {
        BearingPressureModel::UniformPressure => (2.0 / 3.0) * (r_o.powi(3) - r_i.powi(3)) / (r_o.powi(2) - r_i.powi(2)),
        BearingPressureModel::UniformWear => (r_i + r_o) / 2.0,
    }
}

/// `T_bearing = mu_bearing * F * r_eff` (spec sections 9/10) - the closed
/// form, used as the fast path by the main solver's root-finding inner
/// loop (evaluated many times per Brent iteration).
pub fn bearing_torque(preload: f64, mu_bearing: f64, r_i: f64, r_o: f64, model: BearingPressureModel) -> f64 {
    if r_o <= r_i || r_i < 0.0 {
        return 0.0;
    }
    mu_bearing * preload * effective_radius(model, r_i, r_o)
}

/// The same physical integral, evaluated numerically rather than via the
/// closed-form effective radius - proof the closed forms above are correct
/// derivations rather than looked-up formulas, and the extensibility point
/// spec section 11 asks for ("architect the solver so nonuniform pressure
/// can later be represented numerically"). A future non-classical `p(r)`
/// model can reuse this function directly by extending
/// [`BearingPressureModel`] and [`pressure_at`].
pub fn bearing_torque_quadrature(preload: f64, mu_bearing: f64, r_i: f64, r_o: f64, model: BearingPressureModel) -> f64 {
    if r_o <= r_i || r_i < 0.0 {
        return 0.0;
    }
    let integrand = |r: f64| mu_bearing * pressure_at(model, preload, r_i, r_o, r) * r * r;
    2.0 * std::f64::consts::PI * gauss_legendre_5(r_i, r_o, integrand)
}

/// `p_average = F / (pi*(ro^2-ri^2))` (spec section 33) - the same uniform-
/// pressure formula [`pressure_at`] uses, exposed directly for a UI/report
/// "average bearing contact pressure" readout independent of which torque
/// model is selected (contact pressure reporting is always the simple
/// average; only the *friction torque* derivation differs by model).
pub fn average_bearing_pressure(preload: f64, r_i: f64, r_o: f64) -> f64 {
    if r_o <= r_i {
        return 0.0;
    }
    preload / (std::f64::consts::PI * (r_o * r_o - r_i * r_i))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uniform_pressure_closed_form_matches_the_quadrature_integral() {
        let (f, mu, ri, ro) = (10000.0, 0.15, 0.0055, 0.0095);
        let closed = bearing_torque(f, mu, ri, ro, BearingPressureModel::UniformPressure);
        let numeric = bearing_torque_quadrature(f, mu, ri, ro, BearingPressureModel::UniformPressure);
        assert!((closed - numeric).abs() / closed < 1e-9, "closed {closed} vs numeric {numeric}");
    }

    #[test]
    fn uniform_wear_closed_form_matches_the_quadrature_integral() {
        let (f, mu, ri, ro) = (10000.0, 0.15, 0.0055, 0.0095);
        let closed = bearing_torque(f, mu, ri, ro, BearingPressureModel::UniformWear);
        let numeric = bearing_torque_quadrature(f, mu, ri, ro, BearingPressureModel::UniformWear);
        assert!((closed - numeric).abs() / closed < 1e-6, "closed {closed} vs numeric {numeric}");
    }

    #[test]
    fn zero_bearing_friction_yields_zero_bearing_torque() {
        // Spec section 71: test mu_bearing = 0 and verify the physically
        // expected limit.
        let t = bearing_torque(10000.0, 0.0, 0.0055, 0.0095, BearingPressureModel::UniformPressure);
        assert_eq!(t, 0.0);
    }

    #[test]
    fn uniform_wear_effective_radius_is_the_simple_mean_radius() {
        let r = effective_radius(BearingPressureModel::UniformWear, 4.0, 6.0);
        assert!((r - 5.0).abs() < 1e-12);
    }

    #[test]
    fn degenerate_annulus_yields_zero_not_nan_or_panic() {
        assert_eq!(bearing_torque(1000.0, 0.15, 0.01, 0.01, BearingPressureModel::UniformPressure), 0.0);
        assert_eq!(bearing_torque(1000.0, 0.15, 0.02, 0.01, BearingPressureModel::UniformPressure), 0.0);
        assert_eq!(average_bearing_pressure(1000.0, 0.02, 0.01), 0.0);
    }

    #[test]
    fn average_bearing_pressure_integrates_to_the_total_force() {
        let (f, ri, ro) = (10000.0, 0.0055, 0.0095);
        let p = average_bearing_pressure(f, ri, ro);
        let area = std::f64::consts::PI * (ro * ro - ri * ri);
        assert!((p * area - f).abs() / f < 1e-9);
    }
}
