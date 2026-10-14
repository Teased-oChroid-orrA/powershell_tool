//! Forward-mode automatic differentiation: dual numbers with `N` fixed derivative directions stored inline (no heap allocation).
//!
//! `Dual<f64, N>` carries a value and `N` directional derivatives; `Dual<Dual<f64, 1>, 1>` is second order (a Hessian-vector
//! product by forward-over-forward); `Dual<Rev<..>, N>` is reverse-over-forward. Each seed direction costs one extra multiply-add
//! per operation, so the cost of a Jacobian is `ceil(n / N)` evaluations of the kernel (chunked forward mode).

use super::scalar::{powi_by_mul, Scalar};
use std::ops::{Add, Div, Mul, Neg, Sub};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Dual<T: Scalar, const N: usize> {
    pub v: T,
    pub d: [T; N],
}

impl<T: Scalar, const N: usize> Dual<T, N> {
    /// A variable: value `v`, derivative one in direction `i`.
    pub fn var(v: T, i: usize) -> Self {
        let mut d = [T::cst(0.0); N];
        d[i] = T::cst(1.0);
        Self { v, d }
    }

    /// A value with the given derivative vector (a directional seed when `N = 1`).
    pub fn with(v: T, d: [T; N]) -> Self {
        Self { v, d }
    }

    fn lift(self, f: T, df: T) -> Self {
        // chain rule: value f(v), derivative df(v) * d
        let mut d = self.d;
        for x in d.iter_mut() {
            *x = *x * df;
        }
        Self { v: f, d }
    }
}

impl<T: Scalar, const N: usize> Add for Dual<T, N> {
    type Output = Self;
    fn add(self, o: Self) -> Self {
        let mut d = self.d;
        for (x, y) in d.iter_mut().zip(&o.d) {
            *x = *x + *y;
        }
        Self { v: self.v + o.v, d }
    }
}

impl<T: Scalar, const N: usize> Sub for Dual<T, N> {
    type Output = Self;
    fn sub(self, o: Self) -> Self {
        let mut d = self.d;
        for (x, y) in d.iter_mut().zip(&o.d) {
            *x = *x - *y;
        }
        Self { v: self.v - o.v, d }
    }
}

impl<T: Scalar, const N: usize> Mul for Dual<T, N> {
    type Output = Self;
    fn mul(self, o: Self) -> Self {
        let mut d = [T::cst(0.0); N];
        for i in 0..N {
            d[i] = self.v * o.d[i] + self.d[i] * o.v;
        }
        Self { v: self.v * o.v, d }
    }
}

impl<T: Scalar, const N: usize> Div for Dual<T, N> {
    type Output = Self;
    fn div(self, o: Self) -> Self {
        let q = self.v / o.v;
        let mut d = [T::cst(0.0); N];
        for i in 0..N {
            d[i] = (self.d[i] - q * o.d[i]) / o.v;
        }
        Self { v: q, d }
    }
}

impl<T: Scalar, const N: usize> Neg for Dual<T, N> {
    type Output = Self;
    fn neg(self) -> Self {
        let mut d = self.d;
        for x in d.iter_mut() {
            *x = -*x;
        }
        Self { v: -self.v, d }
    }
}

impl<T: Scalar, const N: usize> Scalar for Dual<T, N> {
    fn cst(v: f64) -> Self {
        Self { v: T::cst(v), d: [T::cst(0.0); N] }
    }
    fn value(&self) -> f64 {
        self.v.value()
    }
    fn sqrt(self) -> Self {
        let s = self.v.sqrt();
        self.lift(s, T::cst(0.5) / s)
    }
    fn exp(self) -> Self {
        let e = self.v.exp();
        self.lift(e, e)
    }
    fn ln(self) -> Self {
        self.lift(self.v.ln(), self.v.recip())
    }
    fn sin(self) -> Self {
        self.lift(self.v.sin(), self.v.cos())
    }
    fn cos(self) -> Self {
        self.lift(self.v.cos(), -self.v.sin())
    }
    fn atan(self) -> Self {
        self.lift(self.v.atan(), (T::cst(1.0) + self.v * self.v).recip())
    }
    fn tanh(self) -> Self {
        let t = self.v.tanh();
        self.lift(t, T::cst(1.0) - t * t)
    }
    fn powi(self, n: i32) -> Self {
        if n == 0 {
            return Self::cst(1.0);
        }
        // d/dx x^n = n x^(n-1): the lower power is built by multiplication so a zero base stays finite for n >= 1
        let low = powi_by_mul(self.v, n - 1);
        self.lift(low * self.v, T::cst(n as f64) * low)
    }
    fn powf(self, c: f64) -> Self {
        let p = self.v.powf(c);
        self.lift(p, T::cst(c) * self.v.powf(c - 1.0))
    }
}
