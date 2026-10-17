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
        let rounded = Self::rounded(low * low, high * high);
        Self::new(rounded.lo.max(0.), rounded.hi)
    }
    pub fn sqrt_nonnegative(self) -> Self {
        if self.hi < 0. {
            return Self::whole();
        }
        Self::new(
            self.lo.max(0.).sqrt().next_down().max(0.),
            self.hi.sqrt().next_up(),
        )
    }
    pub fn finite(self) -> bool {
        self.lo.is_finite() && self.hi.is_finite() && self.lo <= self.hi
    }
}
impl Add for Interval {
    type Output = Self;
    fn add(self, b: Self) -> Self {
        Self::rounded(self.lo + b.lo, self.hi + b.hi)
    }
}
impl Sub for Interval {
    type Output = Self;
    fn sub(self, b: Self) -> Self {
        Self::rounded(self.lo - b.hi, self.hi - b.lo)
    }
}
impl Mul for Interval {
    type Output = Self;
    fn mul(self, b: Self) -> Self {
        let p = [
            self.lo * b.lo,
            self.lo * b.hi,
            self.hi * b.lo,
            self.hi * b.hi,
        ];
        if p.iter().any(|x| x.is_nan()) {
            return Self::whole();
        }
        Self::rounded(
            p.into_iter().fold(f64::INFINITY, f64::min),
            p.into_iter().fold(f64::NEG_INFINITY, f64::max),
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
            self.lo / b.lo,
            self.lo / b.hi,
            self.hi / b.lo,
            self.hi / b.hi,
        ];
        if q.iter().any(|x| x.is_nan()) {
            return Self::whole();
        }
        Self::rounded(
            q.into_iter().fold(f64::INFINITY, f64::min),
            q.into_iter().fold(f64::NEG_INFINITY, f64::max),
        )
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounds_keep_roundoff_underflow_and_zero_denominators_explicit() {
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
