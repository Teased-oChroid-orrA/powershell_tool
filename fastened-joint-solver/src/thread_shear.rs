//! Thread stripping / thread shear check (spec section 35) - an optional
//! comparison of average thread shear stress against a material shear
//! strength limit, using complete thread geometry rather than a generic
//! engagement rule.
//!
//! ## Simplified form, and why
//!
//! FED-STD-H28 defines separate external-thread and internal-thread shear
//! areas from each thread's own min/max pitch/major/minor diameters:
//!
//! ```text
//! A_s (external) = pi*n*Le*Kn_max*[1/(2n) + 0.57735*(Es_min - Kn_max)]
//! A_n (internal) = pi*n*Le*Ds_min*[1/(2n) + 0.57735*(Ds_min - En_max)]
//! ```
//!
//! `n` = threads per inch, `Le` = engagement length, `Kn_max` = internal
//! thread max minor diameter, `Es_min` = external thread min pitch
//! diameter, `Ds_min` = external thread min major diameter, `En_max` =
//! internal thread max pitch diameter. This crate's [`crate::thread::ThreadGeometry`]
//! stores only nominal `d`/`d2`/`d3` - no thread-class tolerance-band
//! (min/max) dimensions exist anywhere in this crate, and adding a
//! tolerance-class dimension just for this one check was judged out of
//! scope. This module instead uses the standard's own same-material
//! simplified form, which needs only the nominal pitch diameter already
//! available:
//!
//! ```text
//! A_shear = 0.5 * pi * d2 * Le
//! ```
//!
//! This does not distinguish external (bolt) from internal (nut) thread
//! shear the way the full form does - documented here as the deliberate
//! simplification it is, matching this crate's own "identify the
//! assumption explicitly" discipline (see `compliance.rs`'s VDI 2230 doc
//! comment for the same pattern).

/// Average thread shear area for `engagement_length` of the given pitch
/// diameter, via the FED-STD-H28 same-material simplified form (see module
/// doc comment).
pub fn thread_shear_area(pitch_diameter: f64, engagement_length: f64) -> f64 {
    0.5 * std::f64::consts::PI * pitch_diameter * engagement_length
}

/// Average thread shear stress for `preload` carried across `area`.
pub fn thread_shear_stress(preload: f64, area: f64) -> f64 {
    if area > 0.0 {
        preload / area
    } else {
        f64::INFINITY
    }
}

/// Margin of safety against `shear_strength`, same convention as every
/// other margin in this crate (`stress::` yield/ultimate margins): `> 0`
/// has margin, `< 0` exceeds the limit, `f64::INFINITY` when no limit was
/// supplied.
pub fn shear_margin(shear_strength: Option<f64>, stress: f64) -> f64 {
    match shear_strength {
        Some(limit) if stress > 0.0 => limit / stress - 1.0,
        Some(_) => f64::INFINITY,
        None => f64::INFINITY,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shear_area_scales_linearly_with_engagement_length() {
        let short = thread_shear_area(0.5, 0.25);
        let long = thread_shear_area(0.5, 0.5);
        assert!((long - 2.0 * short).abs() < 1e-9);
    }

    #[test]
    fn shear_stress_is_preload_over_area() {
        let area = thread_shear_area(0.5, 0.25);
        let stress = thread_shear_stress(10_000.0, area);
        assert!((stress - 10_000.0 / area).abs() < 1e-6);
    }

    #[test]
    fn zero_area_reports_infinite_stress_not_a_divide_by_zero_panic() {
        assert_eq!(thread_shear_stress(10_000.0, 0.0), f64::INFINITY);
    }

    #[test]
    fn margin_is_infinite_without_a_supplied_strength() {
        assert_eq!(shear_margin(None, 5000.0), f64::INFINITY);
    }

    #[test]
    fn margin_is_negative_when_stress_exceeds_strength() {
        assert!(shear_margin(Some(1000.0), 2000.0) < 0.0);
    }

    #[test]
    fn margin_is_positive_when_stress_is_well_under_strength() {
        assert!(shear_margin(Some(10_000.0), 1000.0) > 0.0);
    }
}
