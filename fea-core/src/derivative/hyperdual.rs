//! Hyper-dual numbers: exact first derivatives and the mixed second derivative of a smooth function in two seed directions,
//! with no subtractive cancellation and no step size. `f(x + e1 a + e2 b + e1e2 c)` carries `f`, `f_a`, `f_b` and `f_ab`.
//!
//! Valid for functions that are twice differentiable at the point. Kernels containing a kink (`abs`, `max`, branch changes) are
//! first-order-only there; [`super::verify::check_smooth`] rejects them for second-order use instead of returning a number.

use super::scalar::{powi_by_mul, Scalar};
use std::ops::{Add, Div, Mul, Neg, Sub};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HyperDual {
    pub f0: f64,
    pub f1: f64,
    pub f2: f64,
    pub f12: f64,
}

impl HyperDual {
    pub fn new(f0: f64, f1: f64, f2: f64, f12: f64) -> Self {
        Self { f0, f1, f2, f12 }
    }

    /// A variable seeded in direction 1 and/or 2.
    pub fn seeded(v: f64, in1: bool, in2: bool) -> Self {
        Self { f0: v, f1: if in1 { 1.0 } else { 0.0 }, f2: if in2 { 1.0 } else { 0.0 }, f12: 0.0 }
    }

    /// `g(f)` from `g`, `g'`, `g''` at the value.
    fn chain(self, g: f64, g1: f64, g2: f64) -> Self {
        Self { f0: g, f1: g1 * self.f1, f2: g1 * self.f2, f12: g1 * self.f12 + g2 * self.f1 * self.f2 }
    }
}

impl Add for HyperDual {
    type Output = Self;
    fn add(self, o: Self) -> Self {
        Self::new(self.f0 + o.f0, self.f1 + o.f1, self.f2 + o.f2, self.f12 + o.f12)
    }
}

impl Sub for HyperDual {
    type Output = Self;
    fn sub(self, o: Self) -> Self {
        Self::new(self.f0 - o.f0, self.f1 - o.f1, self.f2 - o.f2, self.f12 - o.f12)
    }
}

impl Mul for HyperDual {
    type Output = Self;
    fn mul(self, o: Self) -> Self {
        Self::new(self.f0 * o.f0, self.f0 * o.f1 + self.f1 * o.f0, self.f0 * o.f2 + self.f2 * o.f0, self.f0 * o.f12 + self.f1 * o.f2 + self.f2 * o.f1 + self.f12 * o.f0)
    }
}

impl Div for HyperDual {
    type Output = Self;
    fn div(self, o: Self) -> Self {
        self * o.chain(1.0 / o.f0, -1.0 / (o.f0 * o.f0), 2.0 / (o.f0 * o.f0 * o.f0))
    }
}

impl Neg for HyperDual {
    type Output = Self;
    fn neg(self) -> Self {
        Self::new(-self.f0, -self.f1, -self.f2, -self.f12)
    }
}

impl Scalar for HyperDual {
    fn cst(v: f64) -> Self {
        Self::new(v, 0.0, 0.0, 0.0)
    }
    fn value(&self) -> f64 {
        self.f0
    }
    fn sqrt(self) -> Self {
        let s = self.f0.sqrt();
        self.chain(s, 0.5 / s, -0.25 / (s * self.f0))
    }
    fn exp(self) -> Self {
        let e = self.f0.exp();
        self.chain(e, e, e)
    }
    fn ln(self) -> Self {
        self.chain(self.f0.ln(), 1.0 / self.f0, -1.0 / (self.f0 * self.f0))
    }
    fn sin(self) -> Self {
        self.chain(self.f0.sin(), self.f0.cos(), -self.f0.sin())
    }
    fn cos(self) -> Self {
        self.chain(self.f0.cos(), -self.f0.sin(), -self.f0.cos())
    }
    fn atan(self) -> Self {
        let q = 1.0 + self.f0 * self.f0;
        self.chain(self.f0.atan(), 1.0 / q, -2.0 * self.f0 / (q * q))
    }
    fn tanh(self) -> Self {
        let t = self.f0.tanh();
        self.chain(t, 1.0 - t * t, -2.0 * t * (1.0 - t * t))
    }
    fn powi(self, n: i32) -> Self {
        if n < 0 {
            return powi_by_mul(self, n);
        }
        if n == 0 {
            return Self::cst(1.0);
        }
        // repeated multiplication keeps the derivatives finite at a zero base
        powi_by_mul(self, n)
    }
    fn powf(self, c: f64) -> Self {
        self.chain(self.f0.powf(c), c * self.f0.powf(c - 1.0), c * (c - 1.0) * self.f0.powf(c - 2.0))
    }
}
