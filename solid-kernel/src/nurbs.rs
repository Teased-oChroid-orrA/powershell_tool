//! Bounded numerical evaluation of clamped, continuous rational B-spline curves.
//! Geometry foundation only: no STEP integration, trim/solid certificate or new
//! production CAD capability. Periodic/unclamped/discontinuous curves refuse.
use crate::{
    step::{Error, ErrorKind},
    V3,
};
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub control_points: usize,
    pub degree: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            control_points: 4096,
            degree: 16,
        }
    }
}
#[derive(Debug, Clone)]
pub struct NurbsCurve {
    degree: usize,
    knots: Vec<f64>,
    control_points: Vec<V3>,
    weights: Vec<f64>,
}
fn fail(kind: ErrorKind, detail: &str) -> Error {
    Error {
        offset: 0,
        kind,
        detail: detail.into(),
    }
}
fn supported(x: f64) -> bool {
    x.is_finite() && (x == 0. || (1e-30..=1e30).contains(&x.abs()))
}
impl NurbsCurve {
    /// Full expanded knot vector, positive finite weights. Arithmetic input
    /// envelope matches the native geometry domain; budgets are admission caps.
    pub fn new_clamped(
        degree: usize,
        knots: Vec<f64>,
        control_points: Vec<V3>,
        weights: Vec<f64>,
        limits: Limits,
    ) -> Result<Self, Error> {
        let n = control_points.len();
        if n > limits.control_points || degree > limits.degree {
            return Err(fail(
                ErrorKind::Limit,
                "B-spline control-point/degree budget exceeded",
            ));
        }
        let count = n
            .checked_add(degree)
            .and_then(|n| n.checked_add(1))
            .ok_or_else(|| fail(ErrorKind::Limit, "B-spline knot count overflow"))?;
        if degree == 0 || n <= degree || knots.len() != count || weights.len() != n {
            return Err(fail(
                ErrorKind::Geometry,
                "invalid B-spline degree/control/weight/knot counts",
            ));
        }
        if knots.iter().copied().any(|x| !supported(x))
            || control_points
                .iter()
                .flatten()
                .copied()
                .any(|x| !supported(x))
            || weights.iter().any(|w| !supported(*w) || *w <= 0.)
        {
            return Err(fail(
                ErrorKind::Geometry,
                "unsupported B-spline arithmetic domain or nonpositive rational weight",
            ));
        }
        if knots.windows(2).any(|k| k[0] > k[1]) {
            return Err(fail(
                ErrorKind::Geometry,
                "B-spline knots must be nondecreasing",
            ));
        }
        let lo = knots[degree];
        let hi = knots[n];
        if lo >= hi
            || knots[..=degree].iter().any(|k| *k != lo)
            || knots[n..].iter().any(|k| *k != hi)
        {
            return Err(fail(
                ErrorKind::Geometry,
                "only nonempty clamped B-spline domains are supported",
            ));
        }
        let mut i = degree + 1;
        while i < n {
            let mut end = i + 1;
            while end < n && knots[end] == knots[i] {
                end += 1;
            }
            if knots[i] <= lo || knots[i] >= hi || end - i > degree {
                return Err(fail(
                    ErrorKind::Geometry,
                    "discontinuous or overclamped B-spline knot multiplicity",
                ));
            }
            i = end;
        }
        Ok(Self {
            degree,
            knots,
            control_points,
            weights,
        })
    }
    pub fn degree(&self) -> usize {
        self.degree
    }
    pub fn domain(&self) -> [f64; 2] {
        [
            self.knots[self.degree],
            self.knots[self.control_points.len()],
        ]
    }
    pub fn control_points(&self) -> &[V3] {
        &self.control_points
    }
    pub fn weights(&self) -> &[f64] {
        &self.weights
    }
    pub fn knots(&self) -> &[f64] {
        &self.knots
    }
    /// Homogeneous de Boor evaluation, O(degree²) work/O(degree) scratch.
    /// Exact endpoint requests retain authoritative source control coordinates;
    /// out-of-domain parameters are errors, never silently clamped.
    pub fn evaluate(&self, t: f64) -> Result<V3, Error> {
        let [lo, hi] = self.domain();
        if !t.is_finite() || t < lo || t > hi {
            return Err(fail(
                ErrorKind::Geometry,
                "B-spline parameter outside its finite active domain",
            ));
        }
        if t == lo {
            return Ok(self.control_points[0]);
        }
        if t == hi {
            return Ok(*self.control_points.last().unwrap());
        }
        let span = self.knots.partition_point(|k| *k <= t) - 1;
        let p = self.degree;
        let mut d: Vec<[f64; 4]> = (0..=p)
            .map(|j| {
                let i = span - p + j;
                let w = self.weights[i];
                let c = self.control_points[i];
                [c[0] * w, c[1] * w, c[2] * w, w]
            })
            .collect();
        for r in 1..=p {
            for j in (r..=p).rev() {
                let i = span - p + j;
                let denominator = self.knots[i + p - r + 1] - self.knots[i];
                if denominator <= 0. || !denominator.is_finite() {
                    return Err(fail(
                        ErrorKind::Geometry,
                        "invalid active B-spline knot span",
                    ));
                }
                let alpha = (t - self.knots[i]) / denominator;
                if !(0. ..=1.).contains(&alpha) {
                    return Err(fail(
                        ErrorKind::Geometry,
                        "B-spline interpolation parameter outside active span",
                    ));
                }
                for axis in 0..4 {
                    d[j][axis] = (1. - alpha) * d[j - 1][axis] + alpha * d[j][axis];
                }
            }
        }
        let h = d[p];
        if !h[3].is_finite() || h[3] <= 0. {
            return Err(fail(
                ErrorKind::Geometry,
                "invalid evaluated rational denominator",
            ));
        }
        let point = [h[0] / h[3], h[1] / h[3], h[2] / h[3]];
        if point.iter().any(|x| !x.is_finite() || x.abs() > 1e30) {
            return Err(fail(
                ErrorKind::Geometry,
                "evaluated B-spline exceeds geometry arithmetic domain",
            ));
        }
        Ok(point)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn quarter() -> NurbsCurve {
        NurbsCurve::new_clamped(
            2,
            vec![0., 0., 0., 1., 1., 1.],
            vec![[1., 0., 0.], [1., 1., 0.], [0., 1., 0.]],
            vec![1., std::f64::consts::FRAC_1_SQRT_2, 1.],
            Limits::default(),
        )
        .unwrap()
    }
    #[test]
    fn rational_quarter_circle_matches_analytic_radius_and_endpoints() {
        let c = quarter();
        assert_eq!(c.evaluate(0.).unwrap(), [1., 0., 0.]);
        assert_eq!(c.evaluate(1.).unwrap(), [0., 1., 0.]);
        for t in [0., 0.1, 0.25, 0.5, 0.9, 1.] {
            let p = c.evaluate(t).unwrap();
            assert!((p[0] * p[0] + p[1] * p[1] - 1.).abs() < 1e-12);
            assert_eq!(p[2], 0.);
        }
        let mid = c.evaluate(0.5).unwrap();
        assert!((mid[0] - std::f64::consts::FRAC_1_SQRT_2).abs() < 1e-12);
        assert!((mid[1] - mid[0]).abs() < 1e-12);
    }
    #[test]
    fn polynomial_bezier_and_piecewise_linear_active_spans_are_exact() {
        let c = NurbsCurve::new_clamped(
            2,
            vec![0., 0., 0., 1., 1., 1.],
            vec![[0., 0., 0.], [1., 2., 0.], [2., 0., 0.]],
            vec![1.; 3],
            Limits::default(),
        )
        .unwrap();
        assert_eq!(c.evaluate(0.5).unwrap(), [1., 1., 0.]);
        let c = NurbsCurve::new_clamped(
            1,
            vec![0., 0., 0.5, 1., 1.],
            vec![[0., 0., 0.], [2., 0., 0.], [2., 4., 0.]],
            vec![1.; 3],
            Limits::default(),
        )
        .unwrap();
        assert_eq!(c.evaluate(0.25).unwrap(), [1., 0., 0.]);
        assert_eq!(c.evaluate(0.5).unwrap(), [2., 0., 0.]);
        assert_eq!(c.evaluate(0.75).unwrap(), [2., 2., 0.]);
    }
    #[test]
    fn affine_parameter_and_common_weight_scaling_preserve_the_curve() {
        let original = quarter();
        let scaled = NurbsCurve::new_clamped(
            2,
            original.knots().iter().map(|u| 10. + 20. * u).collect(),
            original.control_points().to_vec(),
            original.weights().iter().map(|w| 7. * w).collect(),
            Limits::default(),
        )
        .unwrap();
        for u in [0., 0.125, 0.5, 0.875, 1.] {
            let a = original.evaluate(u).unwrap();
            let b = scaled.evaluate(10. + 20. * u).unwrap();
            for axis in 0..3 {
                assert!((a[axis] - b[axis]).abs() < 1e-12);
            }
        }
    }
    #[test]
    fn rational_degree_elevation_preserves_the_analytic_quarter_circle() {
        let quadratic = quarter();
        let w = std::f64::consts::FRAC_1_SQRT_2;
        let elevated_weight = (1. + 2. * w) / 3.;
        let a = 2. * w / (1. + 2. * w);
        let cubic = NurbsCurve::new_clamped(
            3,
            vec![0., 0., 0., 0., 1., 1., 1., 1.],
            vec![[1., 0., 0.], [1., a, 0.], [a, 1., 0.], [0., 1., 0.]],
            vec![1., elevated_weight, elevated_weight, 1.],
            Limits::default(),
        )
        .unwrap();
        for t in [0., 0.1, 0.3, 0.5, 0.9, 1.] {
            let expected = quadratic.evaluate(t).unwrap();
            let actual = cubic.evaluate(t).unwrap();
            for axis in 0..3 {
                assert!((expected[axis] - actual[axis]).abs() < 1e-12);
            }
            assert!((actual[0] * actual[0] + actual[1] * actual[1] - 1.).abs() < 1e-12);
        }
        let joined = NurbsCurve::new_clamped(
            2,
            vec![0., 0., 0., 0.5, 0.5, 1., 1., 1.],
            vec![
                [0., 0., 0.],
                [1., 1., 0.],
                [2., 0., 0.],
                [3., -1., 0.],
                [4., 0., 0.],
            ],
            vec![1.; 5],
            Limits::default(),
        )
        .unwrap();
        assert_eq!(joined.evaluate(0.25).unwrap(), [1., 0.5, 0.]);
        assert_eq!(joined.evaluate(0.5).unwrap(), [2., 0., 0.]);
        assert_eq!(joined.evaluate(0.75).unwrap(), [3., -0.5, 0.]);
    }
    #[test]
    fn malformed_unsupported_and_over_budget_curves_refuse_without_parameter_clamping() {
        let c = quarter();
        for t in [-1e-12, 1. + 1e-12, f64::NAN, f64::INFINITY] {
            assert!(c.evaluate(t).is_err());
        }
        let create = |degree, knots, points, weights, limits| {
            NurbsCurve::new_clamped(degree, knots, points, weights, limits)
        };
        assert!(create(
            0,
            vec![0., 0., 0., 1., 1., 1.],
            c.control_points().to_vec(),
            c.weights().to_vec(),
            Limits::default()
        )
        .is_err());
        for weights in [
            vec![1., 0., 1.],
            vec![1., -1., 1.],
            vec![1., f64::NAN, 1.],
            vec![1.; 2],
        ] {
            assert!(create(
                2,
                c.knots().to_vec(),
                c.control_points().to_vec(),
                weights,
                Limits::default()
            )
            .is_err());
        }
        for knots in [
            vec![0., 0., 0., -1., 1., 1.],
            vec![0., 0., 0.1, 1., 1., 1.],
            vec![0.; 6],
            vec![0.; 5],
        ] {
            assert!(create(
                2,
                knots,
                c.control_points().to_vec(),
                c.weights().to_vec(),
                Limits::default()
            )
            .is_err());
        }
        assert!(create(
            1,
            vec![0., 0., 0.5, 0.5, 1., 1.],
            vec![[0.; 3]; 4],
            vec![1.; 4],
            Limits::default()
        )
        .is_err());
        let mut invalid = c.control_points().to_vec();
        invalid[1][2] = f64::NAN;
        assert!(create(
            2,
            c.knots().to_vec(),
            invalid,
            c.weights().to_vec(),
            Limits::default()
        )
        .is_err());
        assert_eq!(
            create(
                2,
                c.knots().to_vec(),
                c.control_points().to_vec(),
                c.weights().to_vec(),
                Limits {
                    control_points: 2,
                    ..Limits::default()
                }
            )
            .unwrap_err()
            .kind,
            ErrorKind::Limit
        );
        assert_eq!(
            create(
                2,
                c.knots().to_vec(),
                c.control_points().to_vec(),
                c.weights().to_vec(),
                Limits {
                    degree: 1,
                    ..Limits::default()
                }
            )
            .unwrap_err()
            .kind,
            ErrorKind::Limit
        );
    }
}
