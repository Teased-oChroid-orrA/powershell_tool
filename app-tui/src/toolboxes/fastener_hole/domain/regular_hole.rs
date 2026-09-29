//! Regular (plain cylindrical) fastener-hole fit analysis: the worst-case
//! fit envelope between two toleranced hole diameters, and the reverse
//! transformation that derives a companion hole preserving that entire
//! envelope against a newly chosen reference hole (spec sections 6-8).

use super::errors::GeometryError;
use super::tolerance::TolerancedValue;

/// Sign convention (spec section 6): `Fit = Hole2 - Hole1`, positive =
/// clearance, negative = interference. No opposite convention exists
/// anywhere else in this repository's engineering toolboxes (verified: the
/// only other countersink/interference-fit code, `bushing-solver`, computes
/// press-fit interference on its own bore/OD pair, not a general "hole
/// fit" convention), so this is a fresh, non-conflicting definition, not a
/// deviation from an established one.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FitClassification {
    Clearance,
    Transition,
    Interference,
}

/// The complete worst-case fit envelope between two toleranced holes -
/// never just a nominal-to-nominal difference (spec section 7).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FitEnvelope {
    pub min: f64,
    pub nominal: f64,
    pub max: f64,
    pub classification: FitClassification,
}

/// Boundary convention, stated explicitly and not left implicit (spec
/// section 7): `min > 0` is guaranteed clearance; `max < 0` is guaranteed
/// interference; everything else - including either bound landing exactly
/// on zero - is Transition.
fn classify(min: f64, max: f64) -> FitClassification {
    if min > 0.0 {
        FitClassification::Clearance
    } else if max < 0.0 {
        FitClassification::Interference
    } else {
        FitClassification::Transition
    }
}

/// `Fit = Hole2 - Hole1`. The worst-case minimum fit occurs at the largest
/// Hole1 paired with the smallest Hole2 (and vice versa for the worst-case
/// maximum) - not a nominal-to-nominal subtraction.
pub fn compute_fit(hole1: &TolerancedValue, hole2: &TolerancedValue) -> FitEnvelope {
    let min = hole2.min - hole1.max;
    let max = hole2.max - hole1.min;
    let nominal = hole2.nominal - hole1.nominal;
    FitEnvelope { min, nominal, max, classification: classify(min, max) }
}

/// Result of deriving a secondary companion hole against a new reference
/// hole, along with the independent verification that the original fit
/// envelope was actually reproduced (spec section 8: "the calculation
/// engine shall independently verify").
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FitPreservationCheck {
    pub primary: FitEnvelope,
    pub secondary: FitEnvelope,
    pub preserved: bool,
}

/// Numerical tolerance for "the secondary fit envelope reproduces the
/// primary one" - generous relative to typical dimensional precision
/// (spec's own example precision goes to 0.0001 in) but tight enough to
/// catch a real formula error.
pub const FIT_PRESERVATION_EPS: f64 = 1e-9;

/// Derives the companion hole (playing Hole2's role) that reproduces
/// `primary_fit` exactly against `reference` (playing Hole1's role), i.e.
/// solves `Companion` from `Fit = Companion - Reference` at every one of
/// the fit envelope's min/nominal/max bounds simultaneously:
///
/// ```text
/// Fit.min = Companion.min - Reference.max  =>  Companion.min = Fit.min + Reference.max
/// Fit.max = Companion.max - Reference.min  =>  Companion.max = Fit.max + Reference.min
/// Fit.nominal = Companion.nominal - Reference.nominal
/// ```
///
/// If reproducing the (possibly narrow) original fit envelope against a
/// reference hole with a wider tolerance band than the envelope itself
/// allows would require a companion hole with negative width, that is
/// reported as `ImpossibleGeometry` rather than silently returning an
/// inverted interval.
pub fn derive_companion_hole(primary_fit: &FitEnvelope, reference: &TolerancedValue) -> Result<TolerancedValue, GeometryError> {
    let min = primary_fit.min + reference.max;
    let max = primary_fit.max + reference.min;
    let nominal = primary_fit.nominal + reference.nominal;
    if min > max + 1e-12 {
        return Err(GeometryError::ImpossibleGeometry);
    }
    // `min`/`max` are already correctly ordered by construction when
    // feasible; `TolerancedValue::from_min_max` still normalizes/validates
    // finiteness rather than constructing the struct by hand.
    let companion = TolerancedValue::from_min_max(min, max)?;
    Ok(TolerancedValue { nominal, ..companion })
}

/// Derives the companion hole and independently recomputes both fit
/// envelopes to prove the secondary reproduces the primary - never a
/// self-fulfilling check that just copies the primary's numbers.
pub fn derive_and_verify(primary_fit: &FitEnvelope, reference: &TolerancedValue) -> Result<(TolerancedValue, FitPreservationCheck), GeometryError> {
    let companion = derive_companion_hole(primary_fit, reference)?;
    let secondary = compute_fit(reference, &companion);
    let preserved = (secondary.min - primary_fit.min).abs() <= FIT_PRESERVATION_EPS
        && (secondary.nominal - primary_fit.nominal).abs() <= FIT_PRESERVATION_EPS
        && (secondary.max - primary_fit.max).abs() <= FIT_PRESERVATION_EPS;
    Ok((companion, FitPreservationCheck { primary: *primary_fit, secondary, preserved }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tv(nominal: f64, tol_minus: f64, tol_plus: f64) -> TolerancedValue {
        TolerancedValue::from_nominal_tol(nominal, tol_minus, tol_plus).unwrap()
    }

    #[test]
    fn guaranteed_clearance_when_smallest_hole2_still_exceeds_largest_hole1() {
        let hole1 = tv(0.250, 0.001, 0.000); // 0.249..0.250
        let hole2 = tv(0.260, 0.000, 0.001); // 0.260..0.261
        let fit = compute_fit(&hole1, &hole2);
        assert_eq!(fit.classification, FitClassification::Clearance);
        assert!(fit.min > 0.0);
    }

    #[test]
    fn guaranteed_interference_when_largest_hole2_is_still_smaller_than_smallest_hole1() {
        let hole1 = tv(0.260, 0.000, 0.001); // 0.260..0.261
        let hole2 = tv(0.250, 0.001, 0.000); // 0.249..0.250
        let fit = compute_fit(&hole1, &hole2);
        assert_eq!(fit.classification, FitClassification::Interference);
        assert!(fit.max < 0.0);
    }

    #[test]
    fn transition_when_the_bands_overlap() {
        let hole1 = tv(0.250, 0.001, 0.001); // 0.249..0.251
        let hole2 = tv(0.250, 0.001, 0.001); // 0.249..0.251
        let fit = compute_fit(&hole1, &hole2);
        assert_eq!(fit.classification, FitClassification::Transition);
    }

    #[test]
    fn exact_zero_minimum_boundary_is_transition_not_clearance() {
        let hole1 = tv(0.250, 0.0, 0.0);
        let hole2 = tv(0.250, 0.0, 0.001); // fit min = 0.250-0.250 = 0.0 exactly
        let fit = compute_fit(&hole1, &hole2);
        assert_eq!(fit.min, 0.0);
        assert_eq!(fit.classification, FitClassification::Transition);
    }

    #[test]
    fn exact_zero_maximum_boundary_is_transition_not_interference() {
        let hole1 = tv(0.250, 0.0, 0.0);
        let hole2 = tv(0.250, 0.001, 0.0); // fit max = hole2.max - hole1.min = 0.250-0.250 = 0.0 exactly
        let fit = compute_fit(&hole1, &hole2);
        assert_eq!(fit.max, 0.0);
        assert_eq!(fit.classification, FitClassification::Transition);
    }

    #[test]
    fn explicit_min_max_input_produces_the_same_fit_as_equivalent_nominal_tol_input() {
        let hole1_a = TolerancedValue::from_min_max(0.249, 0.251).unwrap();
        let hole1_b = tv(0.250, 0.001, 0.001);
        let hole2 = tv(0.300, 0.0005, 0.0005);
        assert_eq!(compute_fit(&hole1_a, &hole2), compute_fit(&hole1_b, &hole2));
    }

    #[test]
    fn secondary_reproduces_the_primary_fit_envelope_exactly() {
        let hole1 = tv(0.2500, 0.0010, 0.0005);
        let hole2 = tv(0.2510, 0.0005, 0.0010);
        let primary_fit = compute_fit(&hole1, &hole2);

        let reference = tv(0.3000, 0.0008, 0.0012);
        let (_companion, check) = derive_and_verify(&primary_fit, &reference).unwrap();
        assert!(check.preserved, "expected secondary envelope to reproduce primary: {check:?}");
        assert!((check.secondary.min - primary_fit.min).abs() < FIT_PRESERVATION_EPS);
        assert!((check.secondary.nominal - primary_fit.nominal).abs() < FIT_PRESERVATION_EPS);
        assert!((check.secondary.max - primary_fit.max).abs() < FIT_PRESERVATION_EPS);
    }

    #[test]
    fn asymmetric_tolerance_still_preserves_the_envelope() {
        let hole1 = tv(0.500, 0.0020, 0.0005);
        let hole2 = tv(0.510, 0.0003, 0.0025);
        let primary_fit = compute_fit(&hole1, &hole2);
        let reference = tv(0.750, 0.0015, 0.0010);
        let (_companion, check) = derive_and_verify(&primary_fit, &reference).unwrap();
        assert!(check.preserved);
    }

    #[test]
    fn a_reference_band_wider_than_the_fit_envelope_is_reported_as_impossible() {
        // A near-zero-width primary fit envelope cannot be reproduced
        // against a reference hole with a much wider tolerance band - the
        // companion hole would need negative width.
        let hole1 = tv(0.2500, 0.00001, 0.00001);
        let hole2 = tv(0.2510, 0.00001, 0.00001);
        let primary_fit = compute_fit(&hole1, &hole2);
        let reference = tv(0.500, 0.05, 0.05);
        assert_eq!(derive_and_verify(&primary_fit, &reference), Err(GeometryError::ImpossibleGeometry));
    }

    #[test]
    fn invalid_tolerance_intervals_are_rejected_at_construction() {
        // from_nominal_tol/from_min_max both reject non-finite input before
        // a FitEnvelope is ever built from it.
        assert!(TolerancedValue::from_nominal_tol(f64::NAN, 0.0, 0.0).is_err());
    }
}
