//! The scalar abstraction every differentiable kernel is written against. A kernel generic over [`Scalar`] runs unchanged on
//! `f64` (plain evaluation), on [`Dual`](super::dual::Dual) (forward mode, nestable for second order), on
//! [`Complex`](super::complex::Complex) (complex step), on [`HyperDual`](super::hyperdual::HyperDual) and on
//! [`Rev`](super::tape::Rev) (reverse mode on a local tape). Dispatch is static: no scalar operation goes through a vtable.
//!
//! Branching: a kernel branches on [`Scalar::value`] (the real value), so the differentiated execution path is the path the
//! plain evaluation takes, and every scalar type differentiates that branch. A derivative through a branch boundary is one of the
//! one-sided derivatives; [`super::verify::branch_signature`] records which branches a kernel took.

use std::ops::{Add, Div, Mul, Neg, Sub};

pub trait Scalar: Copy + Add<Output = Self> + Sub<Output = Self> + Mul<Output = Self> + Div<Output = Self> + Neg<Output = Self> {
    /// A constant (no derivative).
    fn cst(v: f64) -> Self;
    /// The real value, used for comparisons and branches.
    fn value(&self) -> f64;
    fn sqrt(self) -> Self;
    fn exp(self) -> Self;
    fn ln(self) -> Self;
    fn sin(self) -> Self;
    fn cos(self) -> Self;
    fn atan(self) -> Self;
    fn tanh(self) -> Self;
    /// Integer power (exact repeated multiplication for non-negative `n`, so the derivative at zero is finite).
    fn powi(self, n: i32) -> Self;
    /// Power with a constant exponent.
    fn powf(self, c: f64) -> Self;
    /// Comparisons on the real values. A kernel should branch through these (not through raw `value()` comparisons) so that
    /// [`super::verify`]'s probe can record which branch was taken.
    fn lt(&self, o: &Self) -> bool {
        self.value() < o.value()
    }
    fn gt(&self, o: &Self) -> bool {
        self.value() > o.value()
    }
    fn le(&self, o: &Self) -> bool {
        self.value() <= o.value()
    }
    fn ge(&self, o: &Self) -> bool {
        self.value() >= o.value()
    }
    /// `|x|`: the derivative of the branch the value is on (`+1` at zero). Not analytic at zero: complex-step and hyper-dual
    /// kernels containing it are rejected there by [`super::verify`].
    fn abs(self) -> Self {
        if self.lt(&Self::cst(0.0)) {
            -self
        } else {
            self
        }
    }
    fn tan(self) -> Self {
        self.sin() / self.cos()
    }
    fn recip(self) -> Self {
        Self::cst(1.0) / self
    }
    fn scale(self, c: f64) -> Self {
        self * Self::cst(c)
    }
    fn square(self) -> Self {
        self * self
    }
    /// `atan2(self, x)` (the angle of the point `(x, self)`), built on `atan` with constant quadrant shifts.
    fn atan2(self, x: Self) -> Self {
        let (xv, yv) = (x.value(), self.value());
        if xv > 0.0 {
            (self / x).atan()
        } else if xv < 0.0 {
            (self / x).atan() + Self::cst(if yv >= 0.0 { std::f64::consts::PI } else { -std::f64::consts::PI })
        } else {
            Self::cst(if yv >= 0.0 { std::f64::consts::FRAC_PI_2 } else { -std::f64::consts::FRAC_PI_2 }) - (x / self).atan()
        }
    }
    /// The larger of two values (the branch of the larger real value).
    fn max(self, other: Self) -> Self {
        if self.ge(&other) {
            self
        } else {
            other
        }
    }
    fn min(self, other: Self) -> Self {
        if self.le(&other) {
            self
        } else {
            other
        }
    }
}

impl Scalar for f64 {
    fn cst(v: f64) -> Self {
        v
    }
    fn value(&self) -> f64 {
        *self
    }
    fn sqrt(self) -> Self {
        f64::sqrt(self)
    }
    fn exp(self) -> Self {
        f64::exp(self)
    }
    fn ln(self) -> Self {
        f64::ln(self)
    }
    fn sin(self) -> Self {
        f64::sin(self)
    }
    fn cos(self) -> Self {
        f64::cos(self)
    }
    fn atan(self) -> Self {
        f64::atan(self)
    }
    fn tanh(self) -> Self {
        f64::tanh(self)
    }
    fn powi(self, n: i32) -> Self {
        f64::powi(self, n)
    }
    fn powf(self, c: f64) -> Self {
        f64::powf(self, c)
    }
}

/// `x^n` by repeated multiplication (square-and-multiply) for any scalar; negative `n` inverts.
pub(crate) fn powi_by_mul<S: Scalar>(x: S, n: i32) -> S {
    let mut e = n.unsigned_abs();
    let (mut base, mut acc) = (x, S::cst(1.0));
    while e > 0 {
        if e & 1 == 1 {
            acc = acc * base;
        }
        e >>= 1;
        if e > 0 {
            base = base * base;
        }
    }
    if n < 0 {
        acc.recip()
    } else {
        acc
    }
}
