//! Radial hoop/radial stress distribution across the bushing wall and
//! the housing's effective annulus - ported from engineering.toolbox's
//! `src/lib/core/bushing/solveEngine.ts`'s `lameStressAtRadius`/
//! `buildLameRegionField` (the actual per-radius stress *field*
//! `toOutput`'s `lame.field` returns, distinct from the single boundary
//! values - `sigma_hoop_housing`/`sigma_hoop_bushing` in `solve.rs` -
//! this crate already carries). Added specifically to plot a real stress
//! *distribution*, not just annotate the two boundary numbers on top of
//! the cross-section drawing.
//!
//! Closed-form thick-wall (Lamé) solution for a cylinder loaded by
//! uniform pressure on its inner (`p_inner`) and outer (`p_outer`)
//! surfaces - exact at every radius between them, not an approximation
//! or an interpolation between the two boundary values.

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LameSample {
    pub r: f64,
    pub sigma_r: f64,
    pub sigma_theta: f64,
    /// Plane-strain-style axial stress estimate at this radius -
    /// `axial_scale * nu * (sigma_r + sigma_theta)`, matching
    /// `buildLameRegionField`'s own per-sample formula (`solveEngine.ts:146`)
    /// - NOT the same formula `solve.rs`'s boundary-level
    /// `stress_axial_housing`/`stress_axial_bushing` use
    /// (`axial_scale * nu * stress_hoop`, hoop only, no radial term) -
    /// the TS source genuinely uses two different axial estimates for
    /// two different purposes, confirmed by reading both call sites, not
    /// assumed to be the same formula reused.
    pub sigma_axial: f64,
}

/// Ported from `lameStressAtRadius` (`solveEngine.ts:108-124`).
pub fn lame_stress_at_radius(r: f64, inner_radius: f64, outer_radius: f64, p_inner: f64, p_outer: f64) -> (f64, f64) {
    let a2 = inner_radius.powi(2);
    let b2 = outer_radius.powi(2);
    let denom = (b2 - a2).max(1e-12);
    let a = (a2 * p_inner - b2 * p_outer) / denom;
    let b = (a2 * b2 * (p_inner - p_outer)) / denom;
    let rr = r.powi(2).max(1e-12);
    (a - b / rr, a + b / rr)
}

/// Ported from `buildLameRegionField` (`solveEngine.ts:126-158`) - just
/// the sample array itself; the max-abs-hoop/axial tracking that
/// function also does has no caller yet in this port, so it's left out
/// per this project's no-speculative-surface rule. `axial_scale` is
/// `axial_constraint_factor * axial_length_factor` (`solve.rs`'s own
/// end-constraint/length-ratio product) and `nu` is the region's own
/// material Poisson's ratio - both required to get `sigma_axial` right
/// per sample, not just the pure hoop/radial field.
pub fn sample_lame_field(inner_radius: f64, outer_radius: f64, p_inner: f64, p_outer: f64, axial_scale: f64, nu: f64, sample_count: usize) -> Vec<LameSample> {
    let n = sample_count.max(3);
    (0..n)
        .map(|i| {
            let t = i as f64 / (n as f64 - 1.0);
            let r = inner_radius + (outer_radius - inner_radius) * t;
            let (sigma_r, sigma_theta) = lame_stress_at_radius(r, inner_radius, outer_radius, p_inner, p_outer);
            let sigma_axial = axial_scale * nu * (sigma_r + sigma_theta);
            LameSample { r, sigma_r, sigma_theta, sigma_axial }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(actual: f64, expected: f64, label: &str) {
        let diff = (actual - expected).abs();
        let tol = expected.abs() * 1e-6 + 1e-6;
        assert!(diff <= tol, "{label}: expected {expected}, got {actual} (diff {diff})");
    }

    /// Golden values captured from the real TS `lameStressAtRadius`
    /// (2026-08-29) for the project's own base fixture (pressure
    /// 8794.147762602435, id_bushing 0.375, bore 0.5, effective housing
    /// OD 1.692568750643269 - the same fixture `differential.rs` uses).
    #[test]
    fn matches_real_ts_lame_stress_at_radius_for_bushing_and_housing_regions() {
        let pressure = 8794.147762602435;
        let id_bushing = 0.375;
        let bore = 0.5;
        let eff_od_housing = 1.692568750643269;

        let (r, sigma) = lame_stress_at_radius(id_bushing / 2.0, id_bushing / 2.0, bore / 2.0, 0.0, pressure);
        close(r, 0.0, "bushing@inner sigma_r");
        close(sigma, -40201.818343325416, "bushing@inner sigma_theta");

        let mid_bushing = (id_bushing / 2.0 + bore / 2.0) / 2.0;
        let (r, sigma) = lame_stress_at_radius(mid_bushing, id_bushing / 2.0, bore / 2.0, 0.0, pressure);
        close(r, -5332.894270032964, "bushing@mid sigma_r");
        close(sigma, -34868.92407329245, "bushing@mid sigma_theta");

        let (r, sigma) = lame_stress_at_radius(bore / 2.0, id_bushing / 2.0, bore / 2.0, 0.0, pressure);
        close(r, -8794.147762602435, "bushing@outer sigma_r");
        close(sigma, -31407.67058072298, "bushing@outer sigma_theta");

        let (r, sigma) = lame_stress_at_radius(bore / 2.0, bore / 2.0, eff_od_housing / 2.0, pressure, 0.0);
        close(r, -8794.147762602435, "housing@inner sigma_r");
        close(sigma, 10475.764872909105, "housing@inner sigma_theta");

        let mid_housing = (bore / 2.0 + eff_od_housing / 2.0) / 2.0;
        let (r, sigma) = lame_stress_at_radius(mid_housing, bore / 2.0, eff_od_housing / 2.0, pressure, 0.0);
        close(r, -1163.4018378137357, "housing@mid sigma_r");
        close(sigma, 2845.0189481204075, "housing@mid sigma_theta");

        let (r, sigma) = lame_stress_at_radius(eff_od_housing / 2.0, bore / 2.0, eff_od_housing / 2.0, pressure, 0.0);
        close(r, 0.0, "housing@outer sigma_r");
        close(sigma, 1681.6171103066717, "housing@outer sigma_theta");
    }

    #[test]
    fn sample_lame_field_endpoints_match_boundary_evaluation() {
        let samples = sample_lame_field(0.25, 0.6, 100.0, 0.0, 1.0, 0.33, 21);
        assert_eq!(samples.len(), 21);
        let (r0, theta0) = lame_stress_at_radius(0.25, 0.25, 0.6, 100.0, 0.0);
        let (r1, theta1) = lame_stress_at_radius(0.6, 0.25, 0.6, 100.0, 0.0);
        assert!((samples.first().unwrap().r - 0.25).abs() < 1e-9);
        assert!((samples.first().unwrap().sigma_theta - theta0).abs() < 1e-9);
        assert!((samples.first().unwrap().sigma_r - r0).abs() < 1e-9);
        assert!((samples.first().unwrap().sigma_axial - 0.33 * (r0 + theta0)).abs() < 1e-9);
        assert!((samples.last().unwrap().r - 0.6).abs() < 1e-9);
        assert!((samples.last().unwrap().sigma_theta - theta1).abs() < 1e-9);
        assert!((samples.last().unwrap().sigma_r - r1).abs() < 1e-9);
    }

    #[test]
    fn sample_count_below_three_is_floored_to_three() {
        let samples = sample_lame_field(0.25, 0.6, 100.0, 0.0, 1.0, 0.33, 1);
        assert_eq!(samples.len(), 3);
    }

    #[test]
    fn zero_axial_scale_zeroes_sigma_axial_regardless_of_hoop_and_radial_stress() {
        // axial_scale = axial_constraint_factor * axial_length_factor - 0
        // for EndConstraint::Free (solve.rs's own end_constraint_factor),
        // matching the TS source's "no axial constraint -> no axial
        // stress estimate" behavior.
        let samples = sample_lame_field(0.25, 0.6, 100.0, 0.0, 0.0, 0.33, 5);
        assert!(samples.iter().all(|s| s.sigma_axial == 0.0));
    }

    /// Golden values captured from the real TS engine (2026-08-29) with
    /// `endConstraint: 'both_ends'` (axialConstraintFactor=1,
    /// axialLengthFactor=1 for this fixture, so axial_scale=1) - the
    /// project's own base fixture again, this time exercising the
    /// axial-stress formula this crate didn't have until now.
    #[test]
    fn matches_real_ts_sigma_axial_for_the_base_fixture_with_both_ends_constrained() {
        let pressure = 8794.147762602435;
        let id_bushing = 0.375;
        let bore = 0.5;
        let eff_od_housing = 1.692568750643269;
        let nu_bushing = 0.34; // bronze
        let nu_housing = 0.33; // al7075

        let bushing_samples = sample_lame_field(id_bushing / 2.0, bore / 2.0, 0.0, pressure, 1.0, nu_bushing, 41);
        close(bushing_samples[10].r, 0.203125, "bushing sample[10] r");
        close(bushing_samples[10].sigma_r, -2973.5072739146017, "bushing sample[10] sigma_r");
        close(bushing_samples[10].sigma_theta, -37228.31106941082, "bushing sample[10] sigma_theta");
        close(bushing_samples[10].sigma_axial, -13668.618236730643, "bushing sample[10] sigma_axial");

        let housing_samples = sample_lame_field(bore / 2.0, eff_od_housing / 2.0, pressure, 0.0, 1.0, nu_housing, 41);
        close(housing_samples[10].r, 0.3990710938304086, "housing sample[10] r");
        close(housing_samples[10].sigma_r, -2940.3877476639423, "housing sample[10] sigma_r");
        close(housing_samples[10].sigma_theta, 4622.0048579706145, "housing sample[10] sigma_theta");
        close(housing_samples[10].sigma_axial, 554.9336464012018, "housing sample[10] sigma_axial");
    }
}
