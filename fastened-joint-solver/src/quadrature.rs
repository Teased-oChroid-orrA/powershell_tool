//! Numerical integration primitives - a 5-point Gauss-Legendre rule for
//! smooth, well-behaved integrands (thread/bearing torque, axial/torsional
//! compliance profiles) and a composite Simpson's rule for the
//! member-compression pressure-cone integral, whose integrand
//! (`1/(E*A(z))`) can have a mild kink at a member interface where the
//! cone's growth rate changes. Per spec section 11/54: "for smooth
//! axisymmetric contact, numerical quadrature is inexpensive relative to
//! UI/event overhead" - these are deliberately simple, not an
//! adaptive/error-controlled scheme, because every integrand this crate
//! feeds them is smooth-per-segment and the segment count is always small
//! (a handful of members/regions, never thousands of slices).

/// 5-point Gauss-Legendre nodes/weights on `[-1, 1]` - exact for
/// polynomials up to degree 9, more than sufficient for the smooth
/// rational integrands (`1/(E*A(z))`, `p(r)*r^2`) this crate integrates.
const GL5_NODES: [f64; 5] = [-0.906_179_845_938_664, -0.538_469_310_105_683, 0.0, 0.538_469_310_105_683, 0.906_179_845_938_664];
const GL5_WEIGHTS: [f64; 5] = [0.236_926_885_056_189, 0.478_628_670_499_366, 0.568_888_888_888_889, 0.478_628_670_499_366, 0.236_926_885_056_189];

/// Integrates `f` over `[a, b]` via 5-point Gauss-Legendre - exact for the
/// smooth, low-order-polynomial-like integrands this crate uses it for
/// (bearing torque's `p(r) r^2`, a single axial/torsional compliance
/// segment). For a integrand with real curvature over a wide interval,
/// prefer [`composite_simpson`] with enough subdivisions instead.
pub fn gauss_legendre_5(a: f64, b: f64, f: impl Fn(f64) -> f64) -> f64 {
    if !(a.is_finite() && b.is_finite()) || b <= a {
        return 0.0;
    }
    let half_width = (b - a) / 2.0;
    let mid = (a + b) / 2.0;
    let mut sum = 0.0;
    for i in 0..5 {
        let x = mid + half_width * GL5_NODES[i];
        sum += GL5_WEIGHTS[i] * f(x);
    }
    sum * half_width
}

/// Composite Simpson's rule with `n` subintervals (`n` rounded up to even)
/// - used for the member pressure-cone compliance integral, which is
/// smooth within a member but can have a slope discontinuity in
/// `A(z)`'s derivative at a member interface (the cone half-angle is
/// constant, but the truncating outer-diameter/mid-grip bound can switch
/// which constraint governs) that a single low-order Gauss rule across the
/// whole span could under-resolve.
pub fn composite_simpson(a: f64, b: f64, n: usize, f: impl Fn(f64) -> f64) -> f64 {
    if !(a.is_finite() && b.is_finite()) || b <= a {
        return 0.0;
    }
    let n = if n % 2 == 1 { n + 1 } else { n.max(2) };
    let h = (b - a) / n as f64;
    let mut sum = f(a) + f(b);
    for i in 1..n {
        let x = a + i as f64 * h;
        sum += if i % 2 == 0 { 2.0 * f(x) } else { 4.0 * f(x) };
    }
    sum * h / 3.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gauss_legendre_5_integrates_a_cubic_exactly() {
        // ∫[0,1] x^3 dx = 1/4
        let result = gauss_legendre_5(0.0, 1.0, |x| x.powi(3));
        assert!((result - 0.25).abs() < 1e-12);
    }

    #[test]
    fn gauss_legendre_5_integrates_1_over_r_reasonably() {
        // ∫[1,2] 1/r dr = ln(2)
        let result = gauss_legendre_5(1.0, 2.0, |r| 1.0 / r);
        assert!((result - 2.0_f64.ln()).abs() < 1e-6);
    }

    #[test]
    fn composite_simpson_integrates_a_quartic_exactly() {
        // ∫[0,1] x^4 dx = 1/5
        let result = composite_simpson(0.0, 1.0, 100, |x| x.powi(4));
        assert!((result - 0.2).abs() < 1e-8);
    }

    #[test]
    fn degenerate_bounds_return_zero_not_nan() {
        assert_eq!(gauss_legendre_5(1.0, 1.0, |x| x), 0.0);
        assert_eq!(gauss_legendre_5(2.0, 1.0, |x| x), 0.0);
        assert_eq!(composite_simpson(1.0, 1.0, 10, |x| x), 0.0);
    }
}
