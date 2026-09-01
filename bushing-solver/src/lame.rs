//! General, self-contained thick-wall (Lamé) pressure-vessel primitives -
//! not specific to bushings, not specific to this crate's particular
//! interference-fit problem. Every function here implements the closed-
//! form Lamé solution for a cylinder loaded by uniform pressure on its
//! inner (`p_inner`) and outer (`p_outer`) surfaces - exact at every
//! radius between them, not an approximation, not a thin-wall reduction,
//! and not an interpolation between boundary values. Any future pressure-
//! vessel calculation (a different shrink fit, a hydraulic cylinder, a
//! pipe under internal pressure) can call directly into this module
//! without touching bushing-specific code.
//!
//! `solve.rs` is the one and only caller today, and it is deliberately
//! written to *derive* every stress/compliance value it needs from these
//! functions rather than re-deriving the same algebra inline a second
//! time - `stress_hoop_housing`/`stress_hoop_bushing` come from
//! [`lame_stress_at_radius`] evaluated at the shared bore interface, and
//! `term_b`/`term_h` (the shrink-fit compliance terms) come from
//! [`diametral_interference_compliance`]. This is a correctness
//! discipline, not just tidiness: before this module existed as a single
//! source of truth, `solve.rs` and this module's own field sampler each
//! hand-rolled their own copy of the same closed-form hoop-stress
//! expression, and the only thing proving they agreed was a differential
//! test that happened to still pass - two independent implementations of
//! the same physics have no structural guarantee of staying in sync if
//! either is edited later. Ported/verified against engineering.toolbox's
//! `src/lib/core/bushing/solveEngine.ts`'s `lameStressAtRadius`/
//! `buildLameRegionField` (the per-radius stress *field* this module's
//! [`sample_lame_field`] builds - the original motivation for adding this
//! module before `solve.rs` was refactored to depend on it too).
//!
//! ## The physics
//!
//! For a cylinder with inner radius `a`, outer radius `b`, uniform
//! pressure `p_inner` at `r=a` and `p_outer` at `r=b`, the Lamé stress
//! state at any radius `r` in `[a, b]` is:
//!
//! ```text
//! A = (a²·p_inner - b²·p_outer) / (b² - a²)
//! B = a²·b²·(p_inner - p_outer) / (b² - a²)
//! σ_r(r)     = A - B/r²
//! σ_θ(r)     = A + B/r²      (hoop/tangential stress)
//! ```
//!
//! This is the general two-constant thick-wall solution from any
//! elasticity reference (e.g. Timoshenko & Goodier, *Theory of
//! Elasticity*) - `A`/`B` fall out of the two boundary conditions
//! `σ_r(a) = -p_inner`, `σ_r(b) = -p_outer` and hold for *any* radius
//! in between, which is exactly what makes this "the full form" rather
//! than a boundary-only reduction.
//!
//! For an interference fit between two such cylinders sharing an
//! interface, the classic shrink-fit result (same reference) says the
//! contact pressure is the diametral interference divided by the sum of
//! each cylinder's own compliance at the interface - see
//! [`diametral_interference_compliance`]'s own doc for that derivation.

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

/// Radial displacement at radius `r` under a plane-stress Lamé stress
/// state `(sigma_r, sigma_theta)` already evaluated at that radius via
/// [`lame_stress_at_radius`] - Hooke's law for a thin axial slice:
/// `u(r) = (r/E)·(σ_θ - ν·σ_r)`. General elasticity, not specific to any
/// particular loading - the caller decides what boundary pressures
/// produced `sigma_r`/`sigma_theta`.
pub fn radial_displacement(r: f64, sigma_r: f64, sigma_theta: f64, e: f64, nu: f64) -> f64 {
    (r / e) * (sigma_theta - nu * sigma_r)
}

/// The shrink-fit "compliance" (or "flexibility") of one cylinder region
/// at its own loaded interface radius: how much that interface's
/// diameter changes per unit of contact pressure applied there, always
/// reported as a positive quantity regardless of whether the interface
/// physically moves inward (a bore compressed from outside) or outward
/// (a bore expanded from inside).
///
/// Derivation: apply a unit pressure at `interface_radius` (whichever of
/// `inner_radius`/`outer_radius` is the loaded boundary), read off the
/// Lamé stress state there via [`lame_stress_at_radius`], convert to
/// radial displacement via [`radial_displacement`], and double it -
/// displacement is radial (half the diameter), so the *diametral* change
/// is twice the radial one. This is exactly Timoshenko's compound-
/// cylinder interference-fit compliance term; a two-cylinder interference
/// fit's contact pressure is the diametral interference divided by the
/// sum of each region's own `diametral_interference_compliance` at the
/// shared interface (optionally with a finite-plate/geometry correction
/// factor applied to one side, as `solve.rs`'s `psi` does for a bushing
/// seated in a finite-width housing - that correction is a bushing-
/// specific geometry concern, not part of the general pressure-vessel
/// physics, so it stays out of this function and gets multiplied in by
/// the caller).
pub fn diametral_interference_compliance(inner_radius: f64, outer_radius: f64, interface_radius: f64, p_inner: f64, p_outer: f64, e: f64, nu: f64) -> f64 {
    let (sigma_r, sigma_theta) = lame_stress_at_radius(interface_radius, inner_radius, outer_radius, p_inner, p_outer);
    2.0 * radial_displacement(interface_radius, sigma_r, sigma_theta, e, nu).abs()
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

    /// Proves `diametral_interference_compliance` reproduces the
    /// textbook shrink-fit compliance formula (Timoshenko, compound
    /// cylinders) from first principles - computed independently here
    /// (not by calling `solve.rs`, so this is a real cross-check against
    /// a second, hand-derived implementation of the same physics, not a
    /// tautology) for the project's own real base fixture (bore 0.5,
    /// id_bushing 0.375, effective housing OD 1.692568750643269, Al
    /// 7075-T6 housing / Al-Bronze C630 bushing - same materials/
    /// geometry `solve.rs`'s own `base_input()` test fixture uses).
    #[test]
    fn diametral_interference_compliance_matches_the_textbook_shrink_fit_formula() {
        let bore = 0.5_f64;
        let id_bushing = 0.375_f64;
        let eff_od_housing = 1.692568750643269_f64;
        let e_bushing = 17000.0 * 1000.0; // Al-Bronze C630, ksi -> psi
        let nu_bushing = 0.34;
        let e_housing = 10300.0 * 1000.0; // Al 7075-T6, ksi -> psi
        let nu_housing = 0.33;

        let expected_term_b = (bore / e_bushing) * (((bore.powi(2) + id_bushing.powi(2)) / (bore.powi(2) - id_bushing.powi(2))) - nu_bushing);
        let expected_term_h_pre_psi = (bore / e_housing) * (((eff_od_housing.powi(2) + bore.powi(2)) / (eff_od_housing.powi(2) - bore.powi(2))) + nu_housing);

        let term_b = diametral_interference_compliance(id_bushing / 2.0, bore / 2.0, bore / 2.0, 0.0, 1.0, e_bushing, nu_bushing);
        let term_h_pre_psi = diametral_interference_compliance(bore / 2.0, eff_od_housing / 2.0, bore / 2.0, 1.0, 0.0, e_housing, nu_housing);

        close(term_b, expected_term_b, "term_b (bushing compliance)");
        close(term_h_pre_psi, expected_term_h_pre_psi, "term_h pre-psi (housing compliance)");

        // Both compliances are always positive, regardless of which
        // direction the interface physically moves (bushing OD moves
        // inward under external pressure, housing ID moves outward
        // under internal pressure) - the whole point of the abs() in
        // the implementation.
        assert!(term_b > 0.0);
        assert!(term_h_pre_psi > 0.0);
    }

    #[test]
    fn radial_displacement_direction_matches_physical_loading() {
        // A cylinder loaded on its OUTER surface (p_outer > 0, p_inner =
        // 0) is compressed - its outer boundary must move inward
        // (negative radial displacement).
        let (sigma_r, sigma_theta) = lame_stress_at_radius(0.25, 0.1875, 0.25, 0.0, 8794.0);
        let u = radial_displacement(0.25, sigma_r, sigma_theta, 17000.0 * 1000.0, 0.34);
        assert!(u < 0.0, "outer-loaded cylinder's outer boundary should move inward, got {u}");

        // A cylinder loaded on its INNER surface (p_inner > 0, p_outer =
        // 0) is expanded - its inner boundary must move outward
        // (positive radial displacement).
        let (sigma_r, sigma_theta) = lame_stress_at_radius(0.25, 0.25, 0.846, 8794.0, 0.0);
        let u = radial_displacement(0.25, sigma_r, sigma_theta, 10300.0 * 1000.0, 0.33);
        assert!(u > 0.0, "inner-loaded cylinder's inner boundary should move outward, got {u}");
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
