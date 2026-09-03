//! Failure-mode + margin-of-safety engine - issue #11 Phases 6-7. v1
//! supports the four modes confirmed data-available in
//! `docs/issue-11-phase-3.md`: yield (maximum-stress theory), Von Mises,
//! Tresca (maximum shear), and ultimate. Buckling/collapse/fatigue/creep/
//! thermal stress are explicitly deferred (`docs/issue-11-status.md`'s
//! backlog) - no code here gestures at them.
//!
//! **Principal stresses**: for the axisymmetric Lamé stress state this
//! crate computes, radial/hoop/axial stress ARE the three principal
//! stresses directly - a real, citable fact (Timoshenko & Goodier,
//! *Theory of Elasticity*: the axisymmetric thick-cylinder solution has
//! no shear stress on the r-theta, theta-z, or r-z planes by symmetry),
//! not an assumption. No Mohr's-circle transformation is needed or
//! performed.
//!
//! **Critical location**: each mode is evaluated at both the inner and
//! outer surface (issue #11: "The implementation shall not assume that
//! every failure criterion necessarily controls at the same location")
//! and reports whichever gives the lower margin - genuinely evaluated,
//! not assumed. For the record (checked by direct computation, not by
//! hand-derivation - an earlier draft of this module's own test suite
//! got this wrong by assuming instead of computing): for a plain,
//! unflawed thick cylinder, the **inner** surface turns out to govern
//! all four v1 modes under every non-negative internal/external pressure
//! combination tried here, including combined loading - a real,
//! classical result (a pressurized cylinder's bore is its critical
//! location), not a bug or a missed case. `worst_of`'s own logic is
//! still verified against synthetic data where the outer surface
//! deliberately has the larger demand, so this isn't "the code can only
//! ever pick inner" - it's "this physics, for this vessel shape, always
//! does."

use crate::geometry::CylinderGeometry;
use crate::pressure::PressureLoading;
use crate::stress::{stress_at_inner_surface, stress_at_outer_surface, StressState};
use mechanics_core::materials::Material;

/// The three principal stresses at a point in this crate's stress state -
/// literally just `(radial, hoop, axial)`, per this module's own doc
/// comment on why no transformation is needed.
pub fn principal_stresses(s: &StressState) -> (f64, f64, f64) {
    (s.radial, s.hoop, s.axial)
}

/// `sigma_vm = sqrt(0.5 * ((s1-s2)^2 + (s2-s3)^2 + (s3-s1)^2))` - the
/// standard multiaxial von Mises equivalent stress.
pub fn von_mises_stress(s: &StressState) -> f64 {
    let (s1, s2, s3) = principal_stresses(s);
    (0.5 * ((s1 - s2).powi(2) + (s2 - s3).powi(2) + (s3 - s1).powi(2))).sqrt()
}

/// `sigma_max - sigma_min` across the three principal stresses - the
/// Tresca (maximum shear) equivalent stress, stated in terms of the
/// tensile yield strength directly (`tau_yield = sigma_y / 2`, from the
/// uniaxial-tension derivation), so no separate shear-yield material
/// property is needed - see `docs/issue-11-phase-3.md`.
pub fn tresca_stress(s: &StressState) -> f64 {
    let (s1, s2, s3) = principal_stresses(s);
    let max = s1.max(s2).max(s3);
    let min = s1.min(s2).min(s3);
    max - min
}

/// The largest-magnitude principal stress - the simple "maximum stress
/// theory" demand value used for both the basic Yield check and the
/// Ultimate check (same method, different allowable - issue #11: "The
/// tool must distinguish: Yield criterion / Ultimate criterion. These are
/// not interchangeable," satisfied by using two different `Material`
/// fields, never conflating the two allowables).
pub fn max_abs_principal_stress(s: &StressState) -> f64 {
    let (s1, s2, s3) = principal_stresses(s);
    s1.abs().max(s2.abs()).max(s3.abs())
}

/// `allowable / applied - 1` - this crate's margin convention, matching
/// `bushing-solver`'s own existing convention (checked before choosing
/// this, per issue #11's "The project shall inspect existing MS
/// conventions before implementation. The chosen convention must be
/// consistent across the tool"). `applied` of exactly zero (no stress at
/// all under this mode) reports `f64::INFINITY` - an unbounded margin,
/// not a division-by-zero panic or a fabricated number.
/// `pub(crate)`, not private - `buckling.rs` reuses this same convention
/// for its own margin (checked before choosing it: issue #11's own "the
/// chosen convention must be consistent across the tool" applies within
/// this crate too, not just against `bushing-solver`).
pub(crate) fn margin_of_safety(allowable: f64, applied: f64) -> f64 {
    if applied != 0.0 {
        allowable / applied - 1.0
    } else {
        f64::INFINITY
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CriticalLocation {
    InnerSurface,
    OuterSurface,
    /// Buckling is a global-instability phenomenon, not a through-wall
    /// stress location - neither `InnerSurface` nor `OuterSurface` means
    /// anything for it, so it gets its own variant rather than being
    /// forced into a label that would misrepresent what actually governs
    /// (the unsupported span between supports, not a wall surface).
    UnsupportedSpan,
}

impl CriticalLocation {
    pub fn label(self) -> &'static str {
        match self {
            CriticalLocation::InnerSurface => "inner surface",
            CriticalLocation::OuterSurface => "outer surface",
            CriticalLocation::UnsupportedSpan => "unsupported span",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MarginResult {
    pub name: &'static str,
    pub margin: f64,
    pub critical_location: CriticalLocation,
    /// The demand value (applied stress or equivalent stress) at the
    /// critical location - kept alongside the margin so a caller/UI can
    /// show "3200 psi vs. allowable 4000 psi", not just the ratio.
    pub applied: f64,
    pub allowable: f64,
}

fn worst_of(
    name: &'static str,
    demand_fn: impl Fn(&StressState) -> f64,
    allowable: f64,
    inner: &StressState,
    outer: &StressState,
) -> MarginResult {
    let applied_inner = demand_fn(inner).abs();
    let applied_outer = demand_fn(outer).abs();
    let margin_inner = margin_of_safety(allowable, applied_inner);
    let margin_outer = margin_of_safety(allowable, applied_outer);
    if margin_inner <= margin_outer {
        MarginResult { name, margin: margin_inner, critical_location: CriticalLocation::InnerSurface, applied: applied_inner, allowable }
    } else {
        MarginResult { name, margin: margin_outer, critical_location: CriticalLocation::OuterSurface, applied: applied_outer, allowable }
    }
}

/// Evaluates all four v1 failure modes at both surfaces and returns one
/// [`MarginResult`] per mode, each already resolved to its own worse
/// (critical) location - never a single assumed location applied
/// uniformly to every mode.
pub fn evaluate_failure_modes(geometry: &CylinderGeometry, pressure: &PressureLoading, material: &Material) -> Vec<MarginResult> {
    let inner = stress_at_inner_surface(geometry, pressure);
    let outer = stress_at_outer_surface(geometry, pressure);
    let sy_psi = material.sy_ksi * 1000.0;
    let ftu_psi = material.ftu_ksi * 1000.0;

    vec![
        worst_of("Yield (maximum stress)", max_abs_principal_stress, sy_psi, &inner, &outer),
        worst_of("Von Mises yield", von_mises_stress, sy_psi, &inner, &outer),
        worst_of("Tresca (maximum shear)", tresca_stress, sy_psi, &inner, &outer),
        worst_of("Ultimate", max_abs_principal_stress, ftu_psi, &inner, &outer),
    ]
}

/// The controlling (minimum) margin among a set of results - issue #11:
/// "The controlling failure mode is: Minimum applicable Margin of
/// Safety." Panics if `results` is empty - callers always pass
/// `evaluate_failure_modes`'s own (never-empty) output.
pub fn governing(results: &[MarginResult]) -> &MarginResult {
    results.iter().min_by(|a, b| a.margin.partial_cmp(&b.margin).expect("margins are never NaN for finite, valid stress inputs")).expect("evaluate_failure_modes never returns an empty vec")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pressure::EndCondition;

    fn uniaxial(sigma: f64) -> StressState {
        StressState { radial: 0.0, hoop: sigma, axial: 0.0, radius: 1.0 }
    }

    #[test]
    fn von_mises_reduces_to_the_uniaxial_stress_for_a_single_nonzero_principal_stress() {
        // The classic sanity check for any von Mises implementation: in
        // pure uniaxial tension, sigma_vm must equal the applied stress
        // exactly (this is how the criterion is calibrated to the
        // uniaxial yield test in the first place).
        let s = uniaxial(30000.0);
        assert!((von_mises_stress(&s) - 30000.0).abs() < 1e-9);
    }

    #[test]
    fn tresca_reduces_to_the_uniaxial_stress_for_a_single_nonzero_principal_stress() {
        let s = uniaxial(30000.0);
        assert!((tresca_stress(&s) - 30000.0).abs() < 1e-9);
    }

    #[test]
    fn von_mises_of_equal_triaxial_stress_is_zero() {
        // Pure hydrostatic stress state - no shape distortion, no von
        // Mises equivalent stress at all (a well-known, real physical
        // property of the criterion, not just an arithmetic coincidence).
        let s = StressState { radial: -5000.0, hoop: -5000.0, axial: -5000.0, radius: 1.0 };
        assert!(von_mises_stress(&s).abs() < 1e-9);
    }

    #[test]
    fn margin_of_safety_of_zero_applied_stress_is_infinite_not_a_panic() {
        assert_eq!(margin_of_safety(50000.0, 0.0), f64::INFINITY);
    }

    #[test]
    fn worst_of_picks_the_lower_margin_location() {
        let inner = uniaxial(30000.0);
        let outer = uniaxial(10000.0);
        let r = worst_of("test", |s| s.hoop, 40000.0, &inner, &outer);
        assert_eq!(r.critical_location, CriticalLocation::InnerSurface);
        assert!((r.applied - 30000.0).abs() < 1e-9);
    }

    #[test]
    fn evaluate_failure_modes_uses_yield_for_yield_modes_and_ultimate_for_ultimate() {
        let geometry = CylinderGeometry::new(2.0, 3.0).unwrap();
        let pressure = PressureLoading::new(5000.0, 0.0, EndCondition::Closed).unwrap();
        let material = *mechanics_core::materials::get_material("al7075");
        let results = evaluate_failure_modes(&geometry, &pressure, &material);
        let yield_mode = results.iter().find(|r| r.name == "Yield (maximum stress)").unwrap();
        let ultimate_mode = results.iter().find(|r| r.name == "Ultimate").unwrap();
        assert_eq!(yield_mode.allowable, material.sy_ksi * 1000.0);
        assert_eq!(ultimate_mode.allowable, material.ftu_ksi * 1000.0);
        assert_ne!(yield_mode.allowable, ultimate_mode.allowable, "al7075's sy and ftu are genuinely different values");
    }

    #[test]
    fn governing_is_the_minimum_margin_among_results() {
        let results = vec![
            MarginResult { name: "a", margin: 0.5, critical_location: CriticalLocation::InnerSurface, applied: 1.0, allowable: 1.5 },
            MarginResult { name: "b", margin: -0.1, critical_location: CriticalLocation::OuterSurface, applied: 1.0, allowable: 0.9 },
            MarginResult { name: "c", margin: 2.0, critical_location: CriticalLocation::InnerSurface, applied: 1.0, allowable: 3.0 },
        ];
        assert_eq!(governing(&results).name, "b");
    }

    #[test]
    fn external_pressure_only_still_governs_at_the_inner_surface_a_real_verified_elasticity_fact() {
        // Checked against the real code, not assumed: for a thick
        // cylinder under external pressure only, the hoop stress
        // magnitude is actually LARGEST at the *inner* surface (a real,
        // if slightly non-obvious, elasticity result - Timoshenko &
        // Goodier confirm it directly), not the outer surface where the
        // pressure is physically applied. All four v1 modes are hoop-
        // stress-dominated here, so all four are governed by the inner
        // surface in this case - this test exists specifically because
        // an earlier draft of this test suite wrongly assumed the
        // opposite and was caught by actually running it, not by
        // inspection.
        let geometry = CylinderGeometry::new(2.0, 3.0).unwrap();
        let pressure = PressureLoading::new(0.0, 5000.0, EndCondition::Closed).unwrap();
        let material = *mechanics_core::materials::get_material("al7075");
        let results = evaluate_failure_modes(&geometry, &pressure, &material);
        assert!(
            results.iter().all(|r| r.critical_location == CriticalLocation::InnerSurface),
            "expected every mode to be governed by the inner surface under external-only pressure for this geometry"
        );
    }

    #[test]
    fn worst_of_is_not_hardcoded_to_either_surface() {
        // For THIS problem (a plain, unflawed thick cylinder), the inner
        // surface turns out to govern all four v1 modes under every
        // non-negative internal/external pressure combination checked
        // (including combined loading, verified by direct computation,
        // not assumed - see this module's own top-level doc comment).
        // That is a property of the physics, not of `worst_of`'s logic -
        // proven here with synthetic, hand-controlled `StressState`
        // values where the outer surface's demand is deliberately made
        // larger, independent of any real geometry/pressure case.
        let inner = uniaxial(5000.0);
        let outer = uniaxial(9000.0);
        let r = worst_of("test", |s| s.hoop, 40000.0, &inner, &outer);
        assert_eq!(r.critical_location, CriticalLocation::OuterSurface);
        assert!((r.applied - 9000.0).abs() < 1e-9);
    }
}
