//! Normalizes the two supported tolerance-entry representations (Nominal
//! +/-Tolerance, and explicit Minimum/Maximum) into one bounded internal
//! form - every downstream calculation operates on [`TolerancedValue`]'s
//! `min`/`max`/`nominal` fields only, never on "which representation did
//! the user type this in as", so there is exactly one calculation path
//! regardless of entry style (spec: "do not implement duplicate
//! calculation paths for the two input styles").
//!
//! This is a fresh, toolbox-local implementation rather than a reuse of
//! `bushing_solver::tolerance::ToleranceRange` - that type lives in a
//! sibling solver crate scoped to press-fit bushing calculations
//! (`RangedValue`/`EnforcementPolicy` machinery this toolbox has no use
//! for), and pulling in `bushing-solver` as a dependency here would create
//! exactly the kind of arbitrary cross-domain coupling `engineering-math`/
//! `mechanics-core` were extracted to avoid. The normalized-interval shape
//! is small enough (min/max/nominal) that re-deriving it locally is the
//! smaller-total-complexity choice.

use super::errors::GeometryError;

/// A closed bound `[min, max]` on a single dimension - the shape every
/// calculation in this domain actually consumes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DimensionInterval {
    pub min: f64,
    pub max: f64,
}

impl DimensionInterval {
    pub fn width(&self) -> f64 {
        self.max - self.min
    }
}

/// A single dimension's toleranced value: a nominal plus the normalized
/// bound it produces. Always constructed through [`TolerancedValue::from_nominal_tol`]
/// or [`TolerancedValue::from_min_max`] so `min <= nominal <= max` and every
/// field is finite - there is no public way to build an invalid one.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TolerancedValue {
    pub nominal: f64,
    pub min: f64,
    pub max: f64,
}

fn require_finite(v: f64) -> Result<f64, GeometryError> {
    if v.is_finite() {
        Ok(v)
    } else {
        Err(GeometryError::NonFiniteInput)
    }
}

impl TolerancedValue {
    /// Nominal +/-Tolerance representation (section 3.1). `tol_minus`/
    /// `tol_plus` are stored as their absolute magnitude - entering a
    /// negative "-Tol" or "+Tol" is a sign the user already means "below"/
    /// "above" nominal, not a request to invert which side of nominal the
    /// band sits on.
    pub fn from_nominal_tol(nominal: f64, tol_minus: f64, tol_plus: f64) -> Result<Self, GeometryError> {
        let nominal = require_finite(nominal)?;
        let tol_minus = require_finite(tol_minus)?.abs();
        let tol_plus = require_finite(tol_plus)?.abs();
        let min = nominal - tol_minus;
        let max = nominal + tol_plus;
        if min > max {
            return Err(GeometryError::InvalidToleranceRange);
        }
        Ok(Self { nominal, min, max })
    }

    /// Explicit Minimum/Maximum representation (section 3.2). Nominal is
    /// taken as the midpoint, matching this codebase's existing
    /// `bushing_solver::tolerance::make_range` convention for the same
    /// input shape.
    pub fn from_min_max(min: f64, max: f64) -> Result<Self, GeometryError> {
        let a = require_finite(min)?;
        let b = require_finite(max)?;
        let lo = a.min(b);
        let hi = a.max(b);
        Ok(Self { nominal: (lo + hi) / 2.0, min: lo, max: hi })
    }

    /// A degenerate (zero-width) toleranced value at exactly `v` - used to
    /// lift a plain calculated/transferred nominal into the same type as a
    /// user-entered toleranced field.
    pub fn exact(v: f64) -> Result<Self, GeometryError> {
        let v = require_finite(v)?;
        Ok(Self { nominal: v, min: v, max: v })
    }

    pub fn tol_minus(&self) -> f64 {
        self.nominal - self.min
    }

    pub fn tol_plus(&self) -> f64 {
        self.max - self.nominal
    }

    pub fn interval(&self) -> DimensionInterval {
        DimensionInterval { min: self.min, max: self.max }
    }

    pub fn is_exact(&self) -> bool {
        (self.max - self.min).abs() < 1e-12
    }

    /// Builds a toleranced value from an independently-computed nominal
    /// plus a separately-derived `[min, max]` bound (used by the
    /// countersink corner-evaluation solver, where the nominal is solved
    /// once from the three nominal inputs and the bound is solved
    /// separately from worst-case tolerance corners - spec: "nominal
    /// calculations shall be performed independently from nominal
    /// inputs"). `nominal` is clamped into `[min, max]` to preserve this
    /// type's invariant in case of floating-point noise at the boundary;
    /// for a correctly monotonic solve the nominal already falls inside.
    pub fn from_bounds_with_nominal(nominal: f64, min: f64, max: f64) -> Result<Self, GeometryError> {
        let nominal = require_finite(nominal)?;
        let min = require_finite(min)?;
        let max = require_finite(max)?;
        if min > max + 1e-9 {
            return Err(GeometryError::InvalidToleranceRange);
        }
        let (lo, hi) = (min.min(max), min.max(max));
        Ok(Self { nominal: nominal.clamp(lo, hi), min: lo, max: hi })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nominal_tol_normalizes_to_min_max() {
        let v = TolerancedValue::from_nominal_tol(0.25, 0.001, 0.002).unwrap();
        assert!((v.min - 0.249).abs() < 1e-9);
        assert!((v.max - 0.252).abs() < 1e-9);
        assert_eq!(v.nominal, 0.25);
    }

    #[test]
    fn min_max_normalizes_regardless_of_input_order() {
        let v = TolerancedValue::from_min_max(0.252, 0.249).unwrap();
        assert!((v.min - 0.249).abs() < 1e-9);
        assert!((v.max - 0.252).abs() < 1e-9);
        assert!((v.nominal - 0.2505).abs() < 1e-9);
    }

    #[test]
    fn negative_entered_tolerances_are_treated_as_magnitudes() {
        let v = TolerancedValue::from_nominal_tol(0.25, -0.001, -0.002).unwrap();
        assert!((v.min - 0.249).abs() < 1e-9);
        assert!((v.max - 0.252).abs() < 1e-9);
    }

    #[test]
    fn non_finite_input_is_rejected_not_propagated() {
        assert_eq!(TolerancedValue::from_nominal_tol(f64::NAN, 0.0, 0.0), Err(GeometryError::NonFiniteInput));
        assert_eq!(TolerancedValue::from_min_max(f64::INFINITY, 1.0), Err(GeometryError::NonFiniteInput));
    }

    #[test]
    fn tol_minus_and_tol_plus_round_trip_through_nominal_tol() {
        let v = TolerancedValue::from_nominal_tol(1.0, 0.01, 0.02).unwrap();
        assert!((v.tol_minus() - 0.01).abs() < 1e-9);
        assert!((v.tol_plus() - 0.02).abs() < 1e-9);
    }

    #[test]
    fn exact_produces_a_zero_width_interval() {
        let v = TolerancedValue::exact(0.5).unwrap();
        assert!(v.is_exact());
        assert_eq!(v.min, 0.5);
        assert_eq!(v.max, 0.5);
    }
}
