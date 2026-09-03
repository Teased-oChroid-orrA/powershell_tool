//! External-pressure shell buckling/instability - v1 addition, pulled in
//! from the explicit backlog (`docs/issue-11-status.md`) by direct user
//! request, with an unsupported-length input ("how far apart the member
//! is supported") as specifically asked for.
//!
//! # What's derived here from first principles, and what isn't - stated
//! plainly, not glossed over
//!
//! The long-tube (very long unsupported span) limit is **fully derived**
//! below from the classical ring-buckling energy/bifurcation analysis
//! (Bryan, 1888; the same derivation given in Timoshenko & Gere, *Theory
//! of Elastic Stability*, Ch. 7 "Buckling of Rings and Tubes") - see
//! [`ring_buckling_pressure`]'s own doc comment for every step.
//!
//! The **finite-length** correction (where the unsupported-length input
//! actually matters) is a harder case to safely re-derive here. The
//! general theory is a real 2D eigenvalue problem (Donnell/Von Mises
//! shell buckling, minimizing over both axial half-wave count `m` and
//! circumferential lobe count `n`) whose closed-form expression varies in
//! presented notation across sources, was not independently verifiable
//! against a text-readable primary source in this session (the most
//! promising primary source found, a NASA technical report, turned out to
//! be a scanned image with no extractable text/equations), and - notably
//! - **Donnell's own simplified shell theory is documented to diverge
//! from the well-established solution specifically in the long-cylinder
//! limit** (Brush & Almroth, 1975, cited via a 2026 literature search)-
//! meaning a hand-reconstructed Donnell derivation would be *less*
//! trustworthy than the ring-buckling result above in exactly the regime
//! this module needs to get right at the boundary. Rather than present a
//! from-scratch finite-length derivation this session could not fully
//! verify, [`windenburg_trilling_critical_pressure`] uses the published,
//! ASME-code-adjacent **Windenburg & Trilling (1934)** closed form
//! directly - itself a validated distillation of the same underlying
//! theory (calibrated against Von Mises' equation and real experimental
//! collapse data, not an arbitrary curve fit) - cited, not re-derived.
//! This is the one piece of this module that is a cited shorthand rather
//! than a from-scratch derivation, and it is used only where the fully-
//! derived ring result does not apply (finite `L`) - see
//! [`critical_external_pressure`] for exactly how the two combine.
//!
//! # 1. Fully derived: the long-tube (`n=2` ring) limit
//!
//! See [`ring_buckling_pressure`].
//!
//! # 2. Cited shorthand, used only for the finite-length regime
//!
//! **Windenburg & Trilling (1934)**, "Collapse by Instability of Thin
//! Cylindrical Shells Under External Pressure," *Trans. ASME* 56(8),
//! 819-825 - the short-to-intermediate unsupported-length formula that
//! underlies the diagonal curves in ASME Section VIII Division 1 (via
//! Section II-D, Fig. G):
//!
//! ```text
//! P_cr = 2.42 * E * (t/D)^2.5 / [(L/D - 0.45*sqrt(t/D)) * sqrt(1-nu^2)]
//! ```
//!
//! where `D` is the mean shell diameter, `t` the wall thickness, `L` the
//! unsupported length between supports/stiffening rings.
//!
//! # Combining the two
//!
//! **Neither formula alone is correct across the whole length range**:
//! the Windenburg-Trilling denominator implies `P_cr -> 0` as
//! `L -> infinity`, which is not physical - a real long span's critical
//! pressure converges to the fully-derived ring value, not zero, because
//! the governing buckling mode switches to the long-tube `n=2` ring mode
//! once `L` is large enough. [`critical_external_pressure`] takes the
//! larger of the two - **this combination is this module's own reasoned
//! synthesis, not itself a named formula from either source** - stated
//! explicitly rather than presented as if it were. It matches how the
//! real ASME design charts these formulas underlie actually behave
//! (short-`L` curves flatten into long-tube-like behavior at high `L/D`).
//!
//! **Applicability / validity range**: both results assume a thin shell;
//! the commonly cited validity range is outer-diameter-to-thickness ratio
//! > ~40. [`evaluate_buckling`] reports
//! [`BucklingApplicability::OutsideValidityRange`] rather than silently
//! computing a number outside that proven domain.

use crate::failure::{margin_of_safety, CriticalLocation, MarginResult};
use crate::geometry::CylinderGeometry;
use crate::pressure::PressureLoading;
use mechanics_core::materials::Material;

/// Windenburg & Trilling (1934) critical external pressure - `mean_diameter`,
/// `thickness`, `unsupported_length` all in the same length unit.
/// `f64::INFINITY` when supports are close enough together that
/// `L/D - 0.45*sqrt(t/D)` is non-positive (the formula's own denominator
/// floor - physically, supports that close make this failure mode
/// effectively irrelevant, not a division-by-zero bug).
pub fn windenburg_trilling_critical_pressure(mean_diameter: f64, thickness: f64, unsupported_length: f64, e: f64, nu: f64) -> f64 {
    let t_over_d = thickness / mean_diameter;
    let l_over_d = unsupported_length / mean_diameter;
    let denom = (l_over_d - 0.45 * t_over_d.sqrt()) * (1.0 - nu * nu).sqrt();
    if denom <= 0.0 {
        return f64::INFINITY;
    }
    2.42 * e * t_over_d.powf(2.5) / denom
}

/// Plane-strain flexural rigidity per unit length of a thin shell wall -
/// the same `E/(1-nu^2)` "plate modulus" substitution that appears
/// throughout thin-shell theory (a slice of a long tube is restrained
/// against axial curvature change by its neighboring slices, unlike an
/// isolated flat plate/beam of the same cross-section, so the effective
/// bending stiffness is higher than the bare `E*I` a free-standing ring
/// would use): `D = E*t^3 / (12*(1-nu^2))`.
fn plane_strain_flexural_rigidity(thickness: f64, e: f64, nu: f64) -> f64 {
    e * thickness.powi(3) / (12.0 * (1.0 - nu * nu))
}

/// The critical pressure for a thin ring (or a unit-length axial slice of
/// a very long tube, under the plane-strain assumption above) to buckle
/// into `n` circumferential lobes - **fully derived**, not cited as a
/// black-box result:
///
/// 1. **Pre-buckled state.** A thin circular ring of mean radius `r`
///    under uniform external pressure `p` (force per unit length of
///    circumference) deforms without bending in its fundamental
///    equilibrium state - pure uniform radial compression, with
///    compressive thrust `N_0 = p*r` per unit length and zero bending
///    moment anywhere around the ring.
/// 2. **Buckled trial shape.** At the critical load, an infinitesimally
///    close alternative equilibrium shape exists. Following Bryan (1888),
///    assume a small radial perturbation `w(theta) = w_n * cos(n*theta)`
///    for integer `n` - `n=0` is just more uniform radial expansion (not
///    a real buckle) and `n=1` is a rigid-body translation of the ring's
///    center (not a real deformation either), so the smallest physically
///    real buckling mode is `n=2` (an oval/elliptical shape).
/// 3. **Bending moment under the perturbed shape.** For this inextensional
///    perturbation, the standard thin-ring curvature-change relation
///    gives a bending moment `M(theta) = (D/r^2) * (n^2 - 1) * w(theta)`
///    at the critical (bifurcation) pressure - `D` is the plane-strain
///    flexural rigidity from [`plane_strain_flexural_rigidity`].
/// 4. **Eigenvalue condition.** Combining this with the ring's moment
///    equilibrium under the pre-buckling thrust `N_0 = p*r` acting through
///    the perturbed curvature yields the classical bifurcation condition
///    (Bryan 1888; reproduced in Timoshenko & Gere and every standard
///    shell-stability reference since):
///
///    ```text
///    p_cr(n) = D * (n^2 - 1) / r^3
///    ```
///
/// 5. **Minimize over the physically allowed modes.** `p_cr(n)` is
///    strictly increasing in `n` for `n >= 2` (since `n^2 - 1` is), so the
///    governing (first-to-occur, lowest-pressure) buckling mode is always
///    `n=2` - verified by this module's own test, not assumed:
///
///    ```text
///    p_cr = p_cr(2) = 3*D/r^3 = 3*E*t^3 / (12*(1-nu^2)*r^3)
///         = E/(4*(1-nu^2)) * (t/r)^3
///    ```
///
/// Cross-checked against an equivalent, independently-stated industry
/// form written in terms of outside diameter,
/// `P_cr = [2E/(1-nu^2)] / [(D/t)(D/t-1)^2]`, which reduces algebraically
/// to this same expression in the thin-wall limit `D/t >> 1` - a second,
/// independent confirmation the coefficient is right, not just one
/// source taken on faith (see this module's own tests).
pub fn ring_buckling_pressure(mean_radius: f64, thickness: f64, e: f64, nu: f64) -> f64 {
    ring_buckling_pressure_for_mode(2, mean_radius, thickness, e, nu)
}

/// `p_cr(n) = D*(n^2-1)/r^3` for an arbitrary lobe count `n` - step 4 of
/// [`ring_buckling_pressure`]'s own derivation, exposed separately so its
/// claim ("`n=2` always minimizes this for `n>=2`") is a real, checkable
/// property in this module's tests rather than an assertion taken on
/// faith.
fn ring_buckling_pressure_for_mode(n: u32, mean_radius: f64, thickness: f64, e: f64, nu: f64) -> f64 {
    let d = plane_strain_flexural_rigidity(thickness, e, nu);
    let n = n as f64;
    d * (n * n - 1.0) / mean_radius.powi(3)
}

/// The governing (larger, physically accurate) critical external pressure
/// across the whole unsupported-length range - see this module's own doc
/// comment for why `max`, not either formula alone.
pub fn critical_external_pressure(geometry: &CylinderGeometry, unsupported_length: f64, e: f64, nu: f64) -> f64 {
    let mean_radius = geometry.mean_radius();
    let mean_diameter = 2.0 * mean_radius;
    let thickness = geometry.wall_thickness();
    let wt = windenburg_trilling_critical_pressure(mean_diameter, thickness, unsupported_length, e, nu);
    let long = ring_buckling_pressure(mean_radius, thickness, e, nu);
    wt.max(long)
}

/// Below this outer-diameter-to-thickness ratio, both formulas above are
/// outside their commonly cited thin-shell validity range - see this
/// module's own doc comment.
const MIN_VALID_DIAMETER_TO_THICKNESS: f64 = 40.0;

#[derive(Debug, Clone, PartialEq)]
pub enum BucklingApplicability {
    /// No external pressure - nothing driving this failure mode.
    NotApplicable,
    /// External pressure is present but no unsupported length was
    /// supplied - the epic's own "insufficient data" state, not silently
    /// skipped or defaulted to an arbitrary length.
    InsufficientData,
    /// The shell is too thick relative to its diameter for either cited
    /// formula's validity range.
    OutsideValidityRange,
    Evaluated(MarginResult),
}

/// Evaluates external-pressure buckling if and only if it's genuinely
/// applicable - never computed unconditionally the way the four stress-
/// based v1 modes are, per the epic's own "not every failure mode is
/// universally applicable" requirement.
pub fn evaluate_buckling(geometry: &CylinderGeometry, pressure: &PressureLoading, material: &Material, unsupported_length: Option<f64>) -> BucklingApplicability {
    if pressure.external_pressure <= 0.0 {
        return BucklingApplicability::NotApplicable;
    }
    let Some(l) = unsupported_length.filter(|l| *l > 0.0) else {
        return BucklingApplicability::InsufficientData;
    };
    let outer_diameter = 2.0 * geometry.outer_radius;
    let thickness = geometry.wall_thickness();
    if outer_diameter / thickness < MIN_VALID_DIAMETER_TO_THICKNESS {
        return BucklingApplicability::OutsideValidityRange;
    }
    let e_psi = material.e_ksi * 1000.0;
    let p_cr = critical_external_pressure(geometry, l, e_psi, material.nu);
    let margin = margin_of_safety(p_cr, pressure.external_pressure);
    BucklingApplicability::Evaluated(MarginResult {
        name: "Buckling (external pressure)",
        margin,
        critical_location: CriticalLocation::UnsupportedSpan,
        applied: pressure.external_pressure,
        allowable: p_cr,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pressure::EndCondition;

    fn al7075() -> Material {
        *mechanics_core::materials::get_material("al7075")
    }

    /// Cross-check against the equivalent industry form stated in terms
    /// of outside diameter (`P_cr = [2E/(1-nu^2)] / [(D/t)(D/t-1)^2]`) -
    /// the two forms are algebraically equivalent only in the thin-wall
    /// limit `D/t >> 1`, so this test uses a genuinely thin case
    /// (D/t = 200) and checks they agree to within the approximation's
    /// own error, not exactly.
    #[test]
    fn ring_buckling_formula_matches_the_equivalent_diameter_based_industry_form() {
        let d = 20.0;
        let t = 0.1; // D/t = 200, thin
        let r = d / 2.0;
        let e = 10_300_000.0;
        let nu = 0.33;
        let radius_form = ring_buckling_pressure(r, t, e, nu);
        let diameter_form = (2.0 * e / (1.0 - nu * nu)) / ((d / t) * (d / t - 1.0).powi(2));
        let rel_diff = (radius_form - diameter_form).abs() / radius_form;
        assert!(rel_diff < 0.02, "expected the two equivalent forms to agree within 2% for D/t=200, got radius_form={radius_form}, diameter_form={diameter_form}, rel_diff={rel_diff}");
    }

    /// Step 5 of `ring_buckling_pressure`'s own derivation ("p_cr(n) is
    /// minimized at n=2") is a real, checkable claim, not an assertion
    /// taken on faith - verify n=2 gives a strictly lower critical
    /// pressure than every other physically real mode up to a reasonable
    /// bound, i.e. that n=2 really is the first (governing) mode to
    /// buckle as pressure rises from zero.
    #[test]
    fn n_equals_2_minimizes_the_ring_buckling_pressure_among_real_modes() {
        let r = 3.0;
        let t = 0.05;
        let e = 10_300_000.0;
        let nu = 0.33;
        let p2 = ring_buckling_pressure_for_mode(2, r, t, e, nu);
        for n in 3..20 {
            let pn = ring_buckling_pressure_for_mode(n, r, t, e, nu);
            assert!(p2 < pn, "expected n=2 ({p2} psi) to be strictly lower than n={n} ({pn} psi)");
        }
    }

    #[test]
    fn windenburg_trilling_pressure_decreases_as_unsupported_length_increases() {
        let d = 6.0;
        let t = 0.1;
        let e = 10_300_000.0;
        let nu = 0.33;
        let p_short = windenburg_trilling_critical_pressure(d, t, 2.0, e, nu);
        let p_long = windenburg_trilling_critical_pressure(d, t, 20.0, e, nu);
        assert!(p_short > p_long, "expected shorter unsupported spans to be more buckling-resistant: {p_short} psi (L=2) vs {p_long} psi (L=20)");
    }

    #[test]
    fn critical_external_pressure_uses_windenburg_trilling_for_short_spans() {
        let geometry = CylinderGeometry::new(2.95, 3.0).unwrap(); // t=0.05, thin
        let e = 10_300_000.0;
        let nu = 0.33;
        let short_l = 1.0;
        let wt = windenburg_trilling_critical_pressure(2.0 * geometry.mean_radius(), geometry.wall_thickness(), short_l, e, nu);
        let long = ring_buckling_pressure(geometry.mean_radius(), geometry.wall_thickness(), e, nu);
        let governing = critical_external_pressure(&geometry, short_l, e, nu);
        assert!(wt > long, "test setup expects WT to dominate for a short span");
        assert_eq!(governing, wt);
    }

    #[test]
    fn critical_external_pressure_floors_at_the_long_tube_value_for_very_long_spans() {
        let geometry = CylinderGeometry::new(2.95, 3.0).unwrap();
        let e = 10_300_000.0;
        let nu = 0.33;
        let very_long_l = 500.0;
        let long = ring_buckling_pressure(geometry.mean_radius(), geometry.wall_thickness(), e, nu);
        let governing = critical_external_pressure(&geometry, very_long_l, e, nu);
        assert!((governing - long).abs() < 1e-6, "expected the governing pressure to floor at the long-tube asymptote for a very long span, got {governing} vs long-tube {long}");
    }

    fn base_pressure(external: f64, closed: bool) -> PressureLoading {
        PressureLoading::new(0.0, external, if closed { EndCondition::Closed } else { EndCondition::Open }).unwrap()
    }

    #[test]
    fn no_external_pressure_is_not_applicable() {
        let geometry = CylinderGeometry::new(2.95, 3.0).unwrap();
        let pressure = base_pressure(0.0, true);
        let result = evaluate_buckling(&geometry, &pressure, &al7075(), Some(10.0));
        assert_eq!(result, BucklingApplicability::NotApplicable);
    }

    #[test]
    fn external_pressure_without_an_unsupported_length_is_insufficient_data() {
        let geometry = CylinderGeometry::new(2.95, 3.0).unwrap();
        let pressure = base_pressure(50.0, true);
        let result = evaluate_buckling(&geometry, &pressure, &al7075(), None);
        assert_eq!(result, BucklingApplicability::InsufficientData);
    }

    #[test]
    fn a_thick_shell_is_outside_the_validity_range() {
        // D/t = 6/1 = 6, far below the 40 threshold.
        let geometry = CylinderGeometry::new(2.0, 3.0).unwrap();
        let pressure = base_pressure(50.0, true);
        let result = evaluate_buckling(&geometry, &pressure, &al7075(), Some(10.0));
        assert_eq!(result, BucklingApplicability::OutsideValidityRange);
    }

    #[test]
    fn a_thin_shell_with_external_pressure_and_a_length_evaluates_a_real_margin() {
        let geometry = CylinderGeometry::new(2.95, 3.0).unwrap(); // D/t = 6/0.05 = 120
        let pressure = base_pressure(50.0, true);
        let result = evaluate_buckling(&geometry, &pressure, &al7075(), Some(10.0));
        let BucklingApplicability::Evaluated(margin_result) = result else { panic!("expected Evaluated, got a non-applicable/insufficient-data/out-of-range result") };
        assert!(margin_result.margin.is_finite());
        assert_eq!(margin_result.applied, 50.0);
    }
}
