//! Complex numbers for complex-step differentiation: `f'(x) ~ Im f(x + ih) / h` with no subtractive cancellation, so `h` can be
//! 1e-20 and the derivative is correct to the rounding error of `f` itself.
//!
//! The method needs `f` analytic and implemented with analytic operations only. `abs`, `max`, `min` and branches on the value are
//! analytic only away from their kinks (here `abs` continues as `sign(Re z) z`); comparisons use the real part. Contact
//! complementarity, projections and discontinuous state changes are not holomorphic: [`super::verify`] rejects them.

use super::scalar::{powi_by_mul, Scalar};
use std::ops::{Add, Div, Mul, Neg, Sub};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Complex {
    pub re: f64,
    pub im: f64,
}

impl Complex {
    pub fn new(re: f64, im: f64) -> Self {
        Self { re, im }
    }
}

impl Add for Complex {
    type Output = Self;
    fn add(self, o: Self) -> Self {
        Self::new(self.re + o.re, self.im + o.im)
    }
}

impl Sub for Complex {
    type Output = Self;
    fn sub(self, o: Self) -> Self {
        Self::new(self.re - o.re, self.im - o.im)
    }
}

impl Mul for Complex {
    type Output = Self;
    fn mul(self, o: Self) -> Self {
        Self::new(self.re * o.re - self.im * o.im, self.re * o.im + self.im * o.re)
    }
}

impl Div for Complex {
    type Output = Self;
    fn div(self, o: Self) -> Self {
        // Smith's algorithm: no overflow in the denominator
        if o.re.abs() >= o.im.abs() {
            let r = o.im / o.re;
            let den = o.re + o.im * r;
            Self::new((self.re + self.im * r) / den, (self.im - self.re * r) / den)
        } else {
            let r = o.re / o.im;
            let den = o.re * r + o.im;
            Self::new((self.re * r + self.im) / den, (self.im * r - self.re) / den)
        }
    }
}

impl Neg for Complex {
    type Output = Self;
    fn neg(self) -> Self {
        Self::new(-self.re, -self.im)
    }
}

impl Scalar for Complex {
    fn cst(v: f64) -> Self {
        Self::new(v, 0.0)
    }
    fn value(&self) -> f64 {
        self.re
    }
    fn sqrt(self) -> Self {
        // principal root; for a real positive part the imaginary part stays the first-order perturbation
        let m = self.re.hypot(self.im);
        let r = ((m + self.re) * 0.5).sqrt();
        if r == 0.0 {
            return Self::new(0.0, (-self.re).max(0.0).sqrt());
        }
        Self::new(r, self.im / (2.0 * r))
    }
    fn exp(self) -> Self {
        let e = self.re.exp();
        Self::new(e * self.im.cos(), e * self.im.sin())
    }
    fn ln(self) -> Self {
        Self::new(self.re.hypot(self.im).ln(), self.im.atan2(self.re))
    }
    fn sin(self) -> Self {
        Self::new(self.re.sin() * self.im.cosh(), self.re.cos() * self.im.sinh())
    }
    fn cos(self) -> Self {
        Self::new(self.re.cos() * self.im.cosh(), -self.re.sin() * self.im.sinh())
    }
    fn atan(self) -> Self {
        // atan(a + ib) = 1/2 atan2(2a, 1 - a^2 - b^2) + i/4 ln((a^2 + (b + 1)^2) / (a^2 + (b - 1)^2)); the logarithm is taken
        // with ln_1p so a tiny imaginary part (the complex step) is not lost
        let (a, b) = (self.re, self.im);
        let den = a * a + (b - 1.0) * (b - 1.0);
        Self::new(0.5 * (2.0 * a).atan2(1.0 - a * a - b * b), 0.25 * (4.0 * b / den).ln_1p())
    }
    fn tanh(self) -> Self {
        let (e2, one) = ((self + self).exp(), Self::cst(1.0));
        (e2 - one) / (e2 + one)
    }
    fn powi(self, n: i32) -> Self {
        powi_by_mul(self, n)
    }
    fn powf(self, c: f64) -> Self {
        if self.re == 0.0 && self.im == 0.0 {
            return Self::cst(0.0);
        }
        (self.ln().scale(c)).exp()
    }
    fn abs(self) -> Self {
        if self.re < 0.0 {
            -self
        } else {
            self
        }
    }
}
