//! Outward-rounded binary64 bounds for the native rational geometry path.
//! No fast-math/FMA contraction assumptions; bounds describe the exact real
//! arithmetic on the accepted binary64 inputs, not CAD decimal provenance.
use std::ops::{Add, Div, Mul, Sub};
#[derive(Debug, Clone, Copy)]
pub(crate) struct Interval {
    pub lo: f64,
    pub hi: f64,
}
impl Interval {
    pub fn point(x: f64) -> Self {
        Self { lo: x, hi: x }
    }
    pub fn new(lo: f64, hi: f64) -> Self {
        Self { lo, hi }
    }
    fn whole() -> Self {
        Self::new(f64::NEG_INFINITY, f64::INFINITY)
    }
    fn rounded(lo: f64, hi: f64) -> Self {
        if lo.is_nan() || hi.is_nan() {
            Self::whole()
        } else {
            Self::new(lo.next_down(), hi.next_up())
        }
    }
    // Preserve only algebraically exact binary operations. General operations
    // still round outward; these identities prevent artificial domain spill at
    // exact zero/unit knots without clipping a geometric parameter interval.
    fn sum(a: f64, b: f64) -> Self {
        let value = a + b;
        if value.is_finite() && (a == 0. || b == 0. || a == -b || a == b) {
            Self::point(value)
        } else {
            Self::rounded(value, value)
        }
    }
    fn product(a: f64, b: f64) -> Self {
        let value = a * b;
        if a.is_finite() && b.is_finite() && (a == 0. || b == 0. || a.abs() == 1. || b.abs() == 1.)
        {
            return Self::point(value);
        }
        let mut result = Self::rounded(value, value);
        if a.is_finite() && b.is_finite() {
            if (a >= 0. && b >= 0.) || (a <= 0. && b <= 0.) {
                result.lo = result.lo.max(0.);
            } else {
                result.hi = result.hi.min(0.);
            }
        }
        result
    }
    fn quotient(a: f64, b: f64) -> Self {
        let value = a / b;
        if a.is_finite()
            && b.is_finite()
            && b != 0.
            && (a == 0. || b.abs() == 1. || a == b || a == -b)
        {
            Self::point(value)
        } else {
            Self::rounded(value, value)
        }
    }
    pub fn hull(self, other: Self) -> Self {
        Self::new(self.lo.min(other.lo), self.hi.max(other.hi))
    }
    pub fn intersect(self, other: Self) -> Self {
        Self::new(self.lo.max(other.lo), self.hi.min(other.hi))
    }
    pub fn square(self) -> Self {
        let low = if self.lo <= 0. && self.hi >= 0. {
            0.
        } else {
            self.lo.abs().min(self.hi.abs())
        };
        let high = self.lo.abs().max(self.hi.abs());
        Self::new(
            Self::product(low, low).lo.max(0.),
            Self::product(high, high).hi,
        )
    }
    pub fn sqrt_nonnegative(self) -> Self {
        if self.hi < 0. {
            return Self::whole();
        }
        if self.lo == self.hi && (self.lo == 0. || self.lo == 1.) {
            return self;
        }
        Self::new(
            self.lo.max(0.).sqrt().next_down().max(0.),
            self.hi.sqrt().next_up(),
        )
    }
    /// Taylor enclosures on a bounded raw radian domain. No libm rounding or
    /// approximate pi/range-reduction assumption enters the certificate.
    pub fn sin_cos_bounded(self) -> Option<(Self, Self)> {
        if !self.finite() || self.lo < -8. || self.hi > 8. {
            return None;
        }
        let negative_square = Self::point(0.) - self.square();
        let mut sine_term = self;
        let mut cosine_term = Self::point(1.);
        let mut sine = sine_term;
        let mut cosine = cosine_term;
        for n in 1..32 {
            let k = (2 * n) as f64;
            sine_term = sine_term * negative_square / Self::point(k * (k + 1.));
            cosine_term = cosine_term * negative_square / Self::point((k - 1.) * k);
            sine = sine + sine_term;
            cosine = cosine + cosine_term;
        }
        // Cosine polynomial degree 62 is also Taylor degree 63 (odd term
        // zero); sine degree 63 is also degree 64. All real sin/cos derivatives
        // have magnitude <= 1, so Lagrange remainders are |x|^64/64! and
        // |x|^65/65!, respectively. Compute those bounds outward as well.
        let magnitude = Self::point(self.lo.abs().max(self.hi.abs()));
        let mut remainder = Self::point(1.);
        for n in 1..=64 {
            remainder = remainder * magnitude / Self::point(n as f64);
        }
        cosine = cosine + Self::new(-remainder.hi, remainder.hi);
        remainder = remainder * magnitude / Self::point(65.);
        sine = sine + Self::new(-remainder.hi, remainder.hi);
        let range = Self::new(-1., 1.);
        Some((sine.intersect(range), cosine.intersect(range)))
    }
    pub fn finite(self) -> bool {
        self.lo.is_finite() && self.hi.is_finite() && self.lo <= self.hi
    }
}
impl Add for Interval {
    type Output = Self;
    fn add(self, b: Self) -> Self {
        Self::new(Self::sum(self.lo, b.lo).lo, Self::sum(self.hi, b.hi).hi)
    }
}
impl Sub for Interval {
    type Output = Self;
    fn sub(self, b: Self) -> Self {
        Self::new(Self::sum(self.lo, -b.hi).lo, Self::sum(self.hi, -b.lo).hi)
    }
}
impl Mul for Interval {
    type Output = Self;
    fn mul(self, b: Self) -> Self {
        let p = [
            Self::product(self.lo, b.lo),
            Self::product(self.lo, b.hi),
            Self::product(self.hi, b.lo),
            Self::product(self.hi, b.hi),
        ];
        Self::new(
            p.iter().map(|x| x.lo).fold(f64::INFINITY, f64::min),
            p.iter().map(|x| x.hi).fold(f64::NEG_INFINITY, f64::max),
        )
    }
}
impl Div for Interval {
    type Output = Self;
    fn div(self, b: Self) -> Self {
        if b.lo <= 0. && b.hi >= 0. {
            return Self::whole();
        }
        // Divide endpoints directly: reciprocal-first overflows for a tiny
        // nonzero denominator even when every quotient is representable.
        let q = [
            Self::quotient(self.lo, b.lo),
            Self::quotient(self.lo, b.hi),
            Self::quotient(self.hi, b.lo),
            Self::quotient(self.hi, b.hi),
        ];
        Self::new(
            q.iter().map(|x| x.lo).fold(f64::INFINITY, f64::min),
            q.iter().map(|x| x.hi).fold(f64::NEG_INFINITY, f64::max),
        )
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_trig_encloses_identities_intervals_and_small_arguments() {
        for x in [
            0.,
            1e-310,
            -1e-310,
            0.5,
            -0.5,
            1.,
            -1.,
            6.28318530718,
            -8.,
            8.,
        ] {
            let (s, c) = Interval::point(x).sin_cos_bounded().unwrap();
            assert!(s.finite() && c.finite());
            // libm is a regression oracle only, not the enclosure algorithm.
            assert!(s.lo <= x.sin() && x.sin() <= s.hi);
            assert!(c.lo <= x.cos() && x.cos() <= c.hi);
            assert!(s.hi - s.lo < 1e-10 && c.hi - c.lo < 1e-10);
        }
        let (s, c) = Interval::point(0.).sin_cos_bounded().unwrap();
        assert!(s.lo <= 0. && s.hi >= 0. && c.lo <= 1. && c.hi >= 1.);
        let (s, c) = Interval::new(-8., 8.).sin_cos_bounded().unwrap();
        assert!(s.lo <= -1. && s.hi >= 1. && c.lo <= -1. && c.hi >= 1.);
        assert!(Interval::new(-9., 0.).sin_cos_bounded().is_none());
        assert!(Interval::new(1., 0.).sin_cos_bounded().is_none());
    }
    #[test]
    fn bounds_keep_roundoff_underflow_and_zero_denominators_explicit() {
        let zero = Interval::point(0.);
        let unit = Interval::point(1.);
        for exact in [zero * unit, zero / unit, unit - unit] {
            assert_eq!([exact.lo, exact.hi], [0., 0.]);
        }
        for exact in [
            unit / unit,
            unit.square(),
            unit.sqrt_nonnegative(),
            Interval::point(0.5) + Interval::point(0.5),
        ] {
            assert_eq!([exact.lo, exact.hi], [1., 1.]);
        }
        let one = Interval::point(1.);
        let tiny_ratio = Interval::point(1e-310) / Interval::point(1e-310);
        assert!(tiny_ratio.finite() && tiny_ratio.lo <= 1. && tiny_ratio.hi >= 1.);

        let tiny = Interval::point(f64::from_bits(1));
        let sum = one + tiny;
        assert!(sum.lo <= 1. && sum.hi > 1.);
        let product = tiny * Interval::point(0.5);
        assert!(product.lo <= 0. && product.hi >= f64::from_bits(1));
        assert!(!(one / Interval::new(-1., 1.)).finite());
        let d = Interval::point(1.) / Interval::point(10.);
        assert!(d.lo < 0.1 && d.hi > 0.1);
        let square = Interval::new(-2., 3.).square();
        assert_eq!(square.lo, 0.);
        assert!(square.hi >= 9.);
        let root = square.sqrt_nonnegative();
        assert_eq!(root.lo, 0.);
        assert!(root.hi >= 3.);
        let positive = Interval::new(2., 3.).square();
        assert!(positive.lo <= 4. && positive.hi >= 9.);
        let signed = Interval::new(-2., 3.) * Interval::new(-4., 5.);
        assert!(signed.lo <= -12. && signed.hi >= 15.);
    }
}
