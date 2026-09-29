//! Brent's method scalar root solver - per spec section 12/53: "retain a
//! numerical root-solving architecture" for the torque-preload equilibrium,
//! "never depend solely on Newton iteration for user-defined engineering
//! inputs." Brent combines bisection's guaranteed convergence (given a
//! valid bracket) with the speed of secant/inverse-quadratic
//! interpolation when the function is well-behaved - the standard robust
//! choice for a bracketed 1-D root with no reliable derivative.

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RootError {
    /// `f(a)` and `f(b)` have the same sign - no sign change guaranteed in
    /// the bracket, so a root cannot be bracketed at all.
    NotBracketed,
    /// Iteration limit reached without meeting the tolerance - returned
    /// with the best estimate found, never silently accepted as converged.
    MaxIterationsExceeded { best_estimate: f64, best_residual: f64 },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RootSolution {
    pub x: f64,
    pub residual: f64,
    pub iterations: u32,
}

/// Brent's method for a scalar root of `f` bracketed by `[a, b]`. `xtol`
/// is an absolute tolerance on the bracket width; `max_iter` bounds
/// iteration count. Never returns a result without evaluating `f` at it
/// for the caller-visible residual - solver output is never presented as
/// converged without a residual check (spec section 55/77).
pub fn brent(mut a: f64, mut b: f64, f: impl Fn(f64) -> f64, xtol: f64, max_iter: u32) -> Result<RootSolution, RootError> {
    let mut fa = f(a);
    let mut fb = f(b);
    if fa == 0.0 {
        return Ok(RootSolution { x: a, residual: 0.0, iterations: 0 });
    }
    if fb == 0.0 {
        return Ok(RootSolution { x: b, residual: 0.0, iterations: 0 });
    }
    if fa.signum() == fb.signum() {
        return Err(RootError::NotBracketed);
    }
    if fa.abs() < fb.abs() {
        std::mem::swap(&mut a, &mut b);
        std::mem::swap(&mut fa, &mut fb);
    }
    let mut c = a;
    let mut fc = fa;
    let mut mflag = true;
    let mut d = a; // only meaningful once mflag is false; initialized to a harmless value
    let mut s;
    let mut fs;

    for iter in 0..max_iter {
        if fb == 0.0 || (b - a).abs() < xtol {
            return Ok(RootSolution { x: b, residual: fb, iterations: iter });
        }
        s = if fa != fc && fb != fc {
            // Inverse quadratic interpolation.
            a * fb * fc / ((fa - fb) * (fa - fc)) + b * fa * fc / ((fb - fa) * (fb - fc)) + c * fa * fb / ((fc - fa) * (fc - fb))
        } else {
            // Secant.
            b - fb * (b - a) / (fb - fa)
        };

        let bisection_midpoint = (3.0 * a + b) / 4.0;
        let cond1 = !((s > bisection_midpoint && s < b) || (s < bisection_midpoint && s > b));
        let cond2 = mflag && (s - b).abs() >= (b - c).abs() / 2.0;
        let cond3 = !mflag && (s - b).abs() >= (c - d).abs() / 2.0;
        let cond4 = mflag && (b - c).abs() < xtol;
        let cond5 = !mflag && (c - d).abs() < xtol;

        if cond1 || cond2 || cond3 || cond4 || cond5 {
            s = (a + b) / 2.0;
            mflag = true;
        } else {
            mflag = false;
        }

        fs = f(s);
        d = c;
        c = b;
        fc = fb;

        if fa.signum() != fs.signum() {
            b = s;
            fb = fs;
        } else {
            a = s;
            fa = fs;
        }

        if fa.abs() < fb.abs() {
            std::mem::swap(&mut a, &mut b);
            std::mem::swap(&mut fa, &mut fb);
        }
    }
    Err(RootError::MaxIterationsExceeded { best_estimate: b, best_residual: fb })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_root_of_a_simple_linear_function() {
        // f(x) = x - 3, root at x=3.
        let result = brent(0.0, 10.0, |x| x - 3.0, 1e-10, 100).unwrap();
        assert!((result.x - 3.0).abs() < 1e-8);
        assert!(result.residual.abs() < 1e-6);
    }

    #[test]
    fn finds_the_root_of_a_nonlinear_function() {
        // f(x) = x^2 - 2, root at sqrt(2).
        let result = brent(0.0, 2.0, |x| x * x - 2.0, 1e-12, 100).unwrap();
        assert!((result.x - std::f64::consts::SQRT_2).abs() < 1e-8);
    }

    #[test]
    fn returns_not_bracketed_when_both_ends_share_a_sign() {
        let result = brent(1.0, 2.0, |x| x * x + 1.0, 1e-8, 50);
        assert_eq!(result, Err(RootError::NotBracketed));
    }

    #[test]
    fn exact_root_at_an_endpoint_returns_immediately() {
        let result = brent(3.0, 10.0, |x| x - 3.0, 1e-8, 50).unwrap();
        assert_eq!(result.x, 3.0);
        assert_eq!(result.iterations, 0);
    }
}
