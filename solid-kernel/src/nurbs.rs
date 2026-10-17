//! Bounded continuous positive-weight rational B-spline evaluation.
//! Clamped defaults; explicit finite active-domain construction for extended knots.
//! Numerical geometry foundation, not a trim/solid or production CAD certificate.
use crate::{
    step::{Error, ErrorKind},
    V3,
};
use fea_core::derivative::{dual::Dual, Scalar};
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
pub(crate) fn fail(kind: ErrorKind, detail: &str) -> Error {
    Error {
        offset: 0,
        kind,
        detail: detail.into(),
    }
}
fn supported(x: f64) -> bool {
    x.is_finite() && (x == 0. || (1e-30..=1e30).contains(&x.abs()))
}
pub(crate) fn validate_controls(
    points: &[V3],
    weights: &[f64],
    limits: Limits,
) -> Result<(), Error> {
    if points.len() > limits.control_points {
        return Err(fail(
            ErrorKind::Limit,
            "B-spline control-point budget exceeded",
        ));
    }
    if weights.len() != points.len() {
        return Err(fail(
            ErrorKind::Geometry,
            "B-spline control/weight counts differ",
        ));
    }
    if points.iter().flatten().copied().any(|x| !supported(x))
        || weights.iter().any(|w| !supported(*w) || *w <= 0.)
    {
        return Err(fail(
            ErrorKind::Geometry,
            "unsupported B-spline arithmetic domain or nonpositive rational weight",
        ));
    }
    Ok(())
}
pub(crate) fn validate_degree(degree: usize, controls: usize) -> Result<(), Error> {
    if degree == 0 || controls <= degree {
        return Err(fail(
            ErrorKind::Geometry,
            "B-spline degree must be positive and smaller than control count",
        ));
    }
    Ok(())
}
/// The recurrence needs arithmetic and a unit-interval blend only. This keeps
/// interval enclosure on the same recurrence as existing Scalar/Dual evaluation.
pub(crate) trait BlendValue:
    Copy
    + std::ops::Add<Output = Self>
    + std::ops::Sub<Output = Self>
    + std::ops::Mul<Output = Self>
    + std::ops::Div<Output = Self>
{
    fn constant(x: f64) -> Self;
    fn unit(self) -> Result<Self, Error>;
    fn blend(self, a: Self, b: Self) -> Self;
}
impl<S: Scalar> BlendValue for S {
    fn constant(x: f64) -> Self {
        S::cst(x)
    }
    fn unit(self) -> Result<Self, Error> {
        if !self.value().is_finite() || self.lt(&S::cst(0.)) || self.gt(&S::cst(1.)) {
            Err(fail(
                ErrorKind::Geometry,
                "B-spline interpolation parameter outside active span",
            ))
        } else {
            Ok(self)
        }
    }
    fn blend(self, a: Self, b: Self) -> Self {
        (S::cst(1.) - self) * a + self * b
    }
}
impl BlendValue for crate::native_interval::Interval {
    fn constant(x: f64) -> Self {
        Self::point(x)
    }
    fn unit(self) -> Result<Self, Error> {
        let clipped = self.intersect(Self::new(0., 1.));
        if clipped.finite() {
            Ok(clipped)
        } else {
            Err(fail(
                ErrorKind::Geometry,
                "invalid interval spline coefficient",
            ))
        }
    }
    fn blend(self, a: Self, b: Self) -> Self {
        // Convexity is guaranteed by the active span and unit coefficient.
        ((Self::point(1.) - self) * a + self * b).intersect(a.hull(b))
    }
}
/// Shared validated basis and homogeneous recurrence for curves and surfaces.
#[derive(Debug, Clone)]
pub(crate) struct Basis {
    degree: usize,
    knots: Vec<f64>,
    controls: usize,
    clamped: bool,
}
impl Basis {
    pub(crate) fn new(
        degree: usize,
        knots: Vec<f64>,
        n: usize,
        limits: Limits,
    ) -> Result<Self, Error> {
        Self::build(degree, knots, n, limits, true)
    }
    fn build(
        degree: usize,
        knots: Vec<f64>,
        n: usize,
        limits: Limits,
        require_clamped: bool,
    ) -> Result<Self, Error> {
        if n > limits.control_points || degree > limits.degree {
            return Err(fail(
                ErrorKind::Limit,
                "B-spline control-point/degree budget exceeded",
            ));
        }
        validate_degree(degree, n)?;
        let count = n
            .checked_add(degree)
            .and_then(|n| n.checked_add(1))
            .ok_or_else(|| fail(ErrorKind::Limit, "B-spline knot count overflow"))?;
        if knots.len() != count {
            return Err(fail(
                ErrorKind::Geometry,
                "invalid B-spline degree/control/knot counts",
            ));
        }
        if knots.iter().copied().any(|x| !supported(x)) {
            return Err(fail(
                ErrorKind::Geometry,
                "unsupported B-spline knot arithmetic domain",
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
        let clamped =
            knots[..=degree].iter().all(|k| *k == lo) && knots[n..].iter().all(|k| *k == hi);
        if lo >= hi || (require_clamped && !clamped) {
            return Err(fail(
                ErrorKind::Geometry,
                if require_clamped {
                    "only nonempty clamped B-spline domains are supported"
                } else {
                    "nonempty active B-spline domain required"
                },
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
            controls: n,
            clamped,
        })
    }
    pub(crate) fn degree(&self) -> usize {
        self.degree
    }
    pub(crate) fn knots(&self) -> &[f64] {
        &self.knots
    }
    pub(crate) fn domain(&self) -> [f64; 2] {
        [self.knots[self.degree], self.knots[self.controls]]
    }
    pub(crate) fn span(&self, t: f64) -> Result<usize, Error> {
        let [lo, hi] = self.domain();
        if !t.is_finite() || t < lo || t > hi {
            return Err(fail(
                ErrorKind::Geometry,
                "B-spline parameter outside its finite active domain",
            ));
        }
        Ok(if t == hi {
            self.controls - 1
        } else {
            self.knots.partition_point(|k| *k <= t) - 1
        })
    }
    /// Active spans meeting a closed subdomain, found without scanning every
    /// unrelated knot. Endpoint-only neighbours remain included for continuity.
    pub(crate) fn span_range(
        &self,
        domain: [f64; 2],
    ) -> Result<std::ops::RangeInclusive<usize>, Error> {
        self.span(domain[0])?;
        let last = self.span(domain[1])?;
        if domain[0] > domain[1] {
            return Err(fail(
                ErrorKind::Geometry,
                "reversed spline enclosure domain",
            ));
        }
        let first = self
            .knots
            .partition_point(|k| *k < domain[0])
            .saturating_sub(1)
            .max(self.degree);
        Ok(first..=last)
    }
    pub(crate) fn differential_span(&self, t: f64) -> Result<(usize, ParameterSide), Error> {
        let span = self.span(t)?;
        let [lo, hi] = self.domain();
        if t == lo {
            return Ok((span, ParameterSide::LowerEndpoint));
        }
        if t == hi {
            return Ok((span, ParameterSide::UpperEndpoint));
        }
        let first = self.knots.partition_point(|k| *k < t);
        if span + 1 - first >= self.degree {
            return Err(fail(
                ErrorKind::Geometry,
                "first derivative is not certified at an interior C0 knot",
            ));
        }
        Ok((span, ParameterSide::Interior))
    }
    pub(crate) fn homogeneous<S: BlendValue>(
        &self,
        span: usize,
        t: S,
        mut control: impl FnMut(usize) -> Result<[S; 4], Error>,
    ) -> Result<[S; 4], Error> {
        let p = self.degree;
        let mut d = (0..=p)
            .map(|j| control(span - p + j))
            .collect::<Result<Vec<_>, _>>()?;
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
                let denominator =
                    S::constant(self.knots[i + p - r + 1]) - S::constant(self.knots[i]);
                let alpha = ((t - S::constant(self.knots[i])) / denominator).unit()?;
                for axis in 0..4 {
                    d[j][axis] = alpha.blend(d[j - 1][axis], d[j][axis]);
                }
            }
        }
        Ok(d[p])
    }
}
pub(crate) fn homogeneous_control(p: V3, w: f64) -> [f64; 4] {
    [p[0] * w, p[1] * w, p[2] * w, w]
}
pub(crate) fn project<S: Scalar>(h: [S; 4]) -> Result<[S; 3], Error> {
    if !h[3].value().is_finite() || h[3].le(&S::cst(0.)) {
        return Err(fail(
            ErrorKind::Geometry,
            "invalid evaluated rational denominator",
        ));
    }
    let point = [h[0] / h[3], h[1] / h[3], h[2] / h[3]];
    if point
        .iter()
        .any(|x| !x.value().is_finite() || x.gt(&S::cst(1e30)) || x.lt(&S::cst(-1e30)))
    {
        return Err(fail(
            ErrorKind::Geometry,
            "evaluated B-spline exceeds geometry arithmetic domain",
        ));
    }
    Ok(point)
}
/// Endpoint derivatives are one-sided from inside the active domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParameterSide {
    Interior,
    LowerEndpoint,
    UpperEndpoint,
}
#[derive(Debug, Clone, Copy)]
pub struct CurveDifferential {
    pub point: V3,
    pub tangent: V3,
    pub side: ParameterSide,
}
/// Compatibility name for the shared three-dimensional geometry bounds.
pub use crate::geometry_bounds::Bounds3 as CurveBounds;
#[derive(Debug, Clone)]
pub struct NurbsCurve {
    basis: Basis,
    control_points: Vec<V3>,
    weights: Vec<f64>,
}
impl NurbsCurve {
    /// Full expanded clamped knot vector, finite positive weights, explicit caps.
    pub fn new_clamped(
        degree: usize,
        knots: Vec<f64>,
        control_points: Vec<V3>,
        weights: Vec<f64>,
        limits: Limits,
    ) -> Result<Self, Error> {
        let basis = Basis::new(degree, knots, control_points.len(), limits)?;
        validate_controls(&control_points, &weights, limits)?;
        Ok(Self {
            basis,
            control_points,
            weights,
        })
    }
    /// Explicitly supplied extended knot vector, evaluated only on
    /// [knots[degree], knots[control_count]]. No periodic wrapping or implicit
    /// control extension is inferred. Default STEP/body readers stay clamped.
    pub fn new_active_domain(
        degree: usize,
        knots: Vec<f64>,
        control_points: Vec<V3>,
        weights: Vec<f64>,
        limits: Limits,
    ) -> Result<Self, Error> {
        let basis = Basis::build(degree, knots, control_points.len(), limits, false)?;
        validate_controls(&control_points, &weights, limits)?;
        Ok(Self {
            basis,
            control_points,
            weights,
        })
    }
    pub fn degree(&self) -> usize {
        self.basis.degree()
    }
    pub fn domain(&self) -> [f64; 2] {
        self.basis.domain()
    }
    pub fn control_points(&self) -> &[V3] {
        &self.control_points
    }
    pub fn weights(&self) -> &[f64] {
        &self.weights
    }
    pub fn knots(&self) -> &[f64] {
        self.basis.knots()
    }
    /// Bound every parameter in a closed subdomain using outward interval de
    /// Boor arithmetic. Each nonempty knot-span intersection costs one unit;
    /// insufficient budgets refuse without returning a partial enclosure.
    pub fn enclose(&self, parameters: [f64; 2], max_spans: usize) -> Result<CurveBounds, Error> {
        use crate::native_interval::Interval;
        let [lo, hi] = parameters;
        let spans = self.basis.span_range(parameters)?;
        let mut bounds: Option<[Interval; 3]> = None;
        let mut count = 0usize;
        for span in spans {
            let a = lo.max(self.basis.knots[span]);
            let b = hi.min(self.basis.knots[span + 1]);
            if self.basis.knots[span] == self.basis.knots[span + 1] || a > b {
                continue;
            }
            if count >= max_spans {
                return Err(fail(
                    ErrorKind::Limit,
                    "spline enclosure knot-span budget exceeded",
                ));
            }
            count += 1;
            let h = self.basis.homogeneous(span, Interval::new(a, b), |i| {
                let w = Interval::point(self.weights[i]);
                let p = self.control_points[i].map(Interval::point);
                Ok([p[0] * w, p[1] * w, p[2] * w, w])
            })?;
            if h[3].lo <= 0. {
                return Err(fail(
                    ErrorKind::Geometry,
                    "rational enclosure denominator not positive",
                ));
            }
            let projected = [h[0] / h[3], h[1] / h[3], h[2] / h[3]];
            // Positive rational basis functions form a partition of unity.
            // Intersect with the active Cartesian control hull to retain this
            // exact correlation (notably for constant curves/nonuniform weights).
            let xyz: [Interval; 3] = std::array::from_fn(|axis| {
                let active = &self.control_points[span - self.basis.degree..=span];
                let lo = active.iter().map(|p| p[axis]).fold(f64::INFINITY, f64::min);
                let hi = active
                    .iter()
                    .map(|p| p[axis])
                    .fold(f64::NEG_INFINITY, f64::max);
                projected[axis].intersect(Interval::new(lo, hi))
            });
            if xyz.iter().any(|x| !x.finite()) {
                return Err(fail(ErrorKind::Geometry, "nonfinite spline enclosure"));
            }
            bounds = Some(match bounds {
                None => xyz,
                Some(old) => std::array::from_fn(|i| old[i].hull(xyz[i])),
            });
        }
        let bounds = bounds.ok_or_else(|| fail(ErrorKind::Geometry, "empty spline enclosure"))?;
        Ok(CurveBounds {
            min: bounds.map(|x| x.lo),
            max: bounds.map(|x| x.hi),
        })
    }
    /// Local parameter tangent from existing fea-core dual arithmetic. This is
    /// not a derivative through CAD edits, meshing or a global FEA response.
    /// Interior C0 knots refuse; domain endpoints report one-sided semantics.
    pub fn differential(&self, t: f64) -> Result<CurveDifferential, Error> {
        let (span, side) = self.basis.differential_span(t)?;
        let parameter = Dual::<f64, 1>::var(t, 0);
        let values = project(self.basis.homogeneous(span, parameter, |i| {
            Ok(homogeneous_control(self.control_points[i], self.weights[i])
                .map(Dual::<f64, 1>::cst))
        })?)?;
        let mut point = values.map(|x| x.v);
        let tangent = values.map(|x| x.d[0]);
        if tangent.iter().any(|x| !x.is_finite()) {
            return Err(fail(
                ErrorKind::Geometry,
                "nonfinite spline parameter tangent",
            ));
        }
        if self.basis.clamped && side == ParameterSide::LowerEndpoint {
            point = self.control_points[0];
        } else if self.basis.clamped && side == ParameterSide::UpperEndpoint {
            point = *self.control_points.last().unwrap();
        }
        Ok(CurveDifferential {
            point,
            tangent,
            side,
        })
    }
    /// O(degree²) homogeneous de Boor evaluation. Authoritative endpoints remain
    /// exact; invalid/outside parameters refuse rather than clamping.
    pub fn evaluate(&self, t: f64) -> Result<V3, Error> {
        let span = self.basis.span(t)?;
        let [lo, hi] = self.domain();
        if self.basis.clamped && t == lo {
            return Ok(self.control_points[0]);
        }
        if self.basis.clamped && t == hi {
            return Ok(*self.control_points.last().unwrap());
        }
        project(self.basis.homogeneous(span, t, |i| {
            Ok(homogeneous_control(self.control_points[i], self.weights[i]))
        })?)
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
    #[test]
    fn rational_parameter_tangents_match_independent_analytic_formulas_and_endpoint_sides() {
        let c = quarter();
        let w = std::f64::consts::FRAC_1_SQRT_2;
        for t in [0., 0.1, 0.25, 0.5, 0.9, 1.] {
            let d = 1. + 2. * (w - 1.) * t + 2. * (1. - w) * t * t;
            let dd = 2. * (w - 1.) + 4. * (1. - w) * t;
            let nx = 1. + 2. * (w - 1.) * t + (1. - 2. * w) * t * t;
            let ny = 2. * w * t + (1. - 2. * w) * t * t;
            let dx = ((2. * (w - 1.) + 2. * (1. - 2. * w) * t) * d - nx * dd) / (d * d);
            let dy = ((2. * w + 2. * (1. - 2. * w) * t) * d - ny * dd) / (d * d);
            let differential = c.differential(t).unwrap();
            assert!((differential.tangent[0] - dx).abs() < 1e-12);
            assert!((differential.tangent[1] - dy).abs() < 1e-12);
            assert_eq!(differential.tangent[2], 0.);
            assert!(
                (differential.point[0] * differential.tangent[0]
                    + differential.point[1] * differential.tangent[1])
                    .abs()
                    < 1e-12
            );
            assert_eq!(
                differential.side,
                if t == 0. {
                    ParameterSide::LowerEndpoint
                } else if t == 1. {
                    ParameterSide::UpperEndpoint
                } else {
                    ParameterSide::Interior
                }
            );
        }
    }
    #[test]
    fn parameter_and_length_scaling_and_taylor_remainders_verify_local_curve_differentials() {
        let original = quarter();
        let scaled = NurbsCurve::new_clamped(
            2,
            original.knots().iter().map(|u| 10. + 20. * u).collect(),
            original
                .control_points()
                .iter()
                .map(|p| p.map(|x| 25.4 * x))
                .collect(),
            original.weights().iter().map(|w| 7. * w).collect(),
            Limits::default(),
        )
        .unwrap();
        for t in [0., 0.125, 0.5, 0.875, 1.] {
            let a = original.differential(t).unwrap();
            let b = scaled.differential(10. + 20. * t).unwrap();
            for axis in 0..3 {
                assert!((b.point[axis] - 25.4 * a.point[axis]).abs() < 1e-12);
                assert!((b.tangent[axis] - 25.4 / 20. * a.tangent[axis]).abs() < 1e-12);
            }
        }
        let report = fea_core::derivative::verify::taylor_test(
            &|x| original.evaluate(x[0]).unwrap().to_vec(),
            &|x, v| {
                original
                    .differential(x[0])
                    .unwrap()
                    .tangent
                    .map(|d| d * v[0])
                    .to_vec()
            },
            &[0.3],
            &[0.7],
            &[0.02, 0.01, 0.005, 0.0025],
            1e-14,
        );
        assert!(report.observed_order > 1.9, "{report:?}");
    }
    #[test]
    fn c0_knots_refuse_classical_tangents_and_rational_endpoint_values_remain_authoritative() {
        let c = NurbsCurve::new_clamped(
            1,
            vec![0., 0., 0.5, 1., 1.],
            vec![[0., 0., 0.], [2., 0., 0.], [2., 4., 0.]],
            vec![1.; 3],
            Limits::default(),
        )
        .unwrap();
        assert_eq!(c.differential(0.25).unwrap().tangent, [4., 0., 0.]);
        assert_eq!(c.differential(0.75).unwrap().tangent, [0., 8., 0.]);
        assert_eq!(c.evaluate(0.5).unwrap(), [2., 0., 0.]);
        assert!(c.differential(0.5).unwrap_err().detail.contains("C0"));
        let points = vec![[1.2345, -2.345, 3.456], [4.5678, 6.789, -1.234]];
        let c = NurbsCurve::new_clamped(
            1,
            vec![0., 0., 1., 1.],
            points.clone(),
            vec![3., 7.],
            Limits::default(),
        )
        .unwrap();
        for (t, index, ratio) in [(0., 0, 7. / 3.), (1., 1, 3. / 7.)] {
            let differential = c.differential(t).unwrap();
            assert_eq!(differential.point, points[index]);
            for axis in 0..3 {
                assert!(
                    (differential.tangent[axis] - ratio * (points[1][axis] - points[0][axis]))
                        .abs()
                        < 1e-12
                );
            }
        }
    }
    #[test]
    fn interval_enclosures_contain_analytic_rational_curve_and_shrink_locally() {
        let c = quarter();
        let w = std::f64::consts::FRAC_1_SQRT_2;
        for domain in [[0., 1.], [0.1, 0.9], [0.3, 0.31], [0., 0.], [1., 1.]] {
            let b = c.enclose(domain, 1).unwrap();
            for i in 0..=100 {
                let t = domain[0] + (domain[1] - domain[0]) * (i as f64 / 100.);
                let p = c.evaluate(t).unwrap();
                let d = (1. - t) * (1. - t) + 2. * w * t * (1. - t) + t * t;
                let analytic = [
                    ((1. - t) * (1. - t) + 2. * w * t * (1. - t)) / d,
                    (2. * w * t * (1. - t) + t * t) / d,
                    0.,
                ];
                for axis in 0..3 {
                    assert!(
                        b.min[axis] <= p[axis] && p[axis] <= b.max[axis],
                        "{domain:?} {p:?} {b:?}"
                    );
                    assert!(b.min[axis] <= analytic[axis] && analytic[axis] <= b.max[axis]);
                }
            }
        }
        let broad = c.enclose([0., 1.], 1).unwrap();
        let narrow = c.enclose([0.3, 0.3001], 1).unwrap();
        assert!(narrow.max[0] - narrow.min[0] < (broad.max[0] - broad.min[0]) * 0.001);
    }
    #[test]
    fn whole_interval_checks_retain_excursions_hidden_from_midpoint_tests() {
        let c = NurbsCurve::new_clamped(
            3,
            vec![0., 0., 0., 0., 1., 1., 1., 1.],
            vec![[0., 0., 0.], [0., 10., 0.], [1., -10., 0.], [1., 0., 0.]],
            vec![1.; 4],
            Limits::default(),
        )
        .unwrap();
        assert_eq!(c.evaluate(0.5).unwrap()[1], 0.);
        let b = c.enclose([0., 1.], 1).unwrap();
        assert!(b.min[1] < -2. && b.max[1] > 2.);
        for i in 0..=1000 {
            let t = i as f64 / 1000.;
            let y = 30. * t * (1. - t) * (1. - 2. * t);
            assert!(b.min[1] <= y && y <= b.max[1]);
        }
    }
    #[test]
    fn enclosure_spans_budget_and_domain_refusals_are_explicit() {
        let c = NurbsCurve::new_clamped(
            1,
            vec![0., 0., 0.5, 1., 1.],
            vec![[0., 0., 0.], [2., 0., 0.], [2., 4., 0.]],
            vec![1.; 3],
            Limits::default(),
        )
        .unwrap();
        let b = c.enclose([0., 1.], 2).unwrap();
        assert!(b.min[0] <= 0. && b.max[0] >= 2. && b.min[1] <= 0. && b.max[1] >= 4.);
        assert_eq!(c.enclose([0., 1.], 1).unwrap_err().kind, ErrorKind::Limit);
        for d in [[-0.1, 1.], [0., 1.1], [0.7, 0.3], [f64::NAN, 0.5]] {
            assert!(c.enclose(d, 100).is_err());
        }
        assert_eq!(c.enclose([0.1, 0.2], 0).unwrap_err().kind, ErrorKind::Limit);
    }
    fn active_quadratic() -> NurbsCurve {
        NurbsCurve::new_active_domain(
            2,
            vec![0., 1., 2., 3., 4., 5.],
            vec![[0., 0., 0.], [2., 4., 0.], [4., 0., 0.]],
            vec![1.; 3],
            Limits::default(),
        )
        .unwrap()
    }
    #[test]
    fn extended_knots_use_actual_basis_endpoints_and_independent_polynomial_differentials() {
        let c = active_quadratic();
        assert_eq!(c.domain(), [2., 3.]);
        assert_eq!(c.evaluate(2.).unwrap(), [1., 2., 0.]);
        assert_eq!(c.evaluate(3.).unwrap(), [3., 2., 0.]);
        assert_ne!(c.evaluate(2.).unwrap(), c.control_points()[0]);
        for s in [0., 0.125, 0.25, 0.5, 0.75, 1.] {
            let t = 2. + s;
            let p = c.evaluate(t).unwrap();
            let d = c.differential(t).unwrap();
            let exact = [1. + 2. * s, 2. + 4. * s - 4. * s * s, 0.];
            let tangent = [2., 4. - 8. * s, 0.];
            for i in 0..3 {
                assert!((p[i] - exact[i]).abs() < 1e-12);
                assert!((d.tangent[i] - tangent[i]).abs() < 1e-12);
            }
            assert_eq!(d.point, p);
        }
        for t in [0., 1., 4., 5., f64::NAN] {
            assert!(c.evaluate(t).is_err());
        }
        assert!(NurbsCurve::new_clamped(
            2,
            c.knots().to_vec(),
            c.control_points().to_vec(),
            c.weights().to_vec(),
            Limits::default()
        )
        .is_err());
    }
    #[test]
    fn active_domain_sampling_distance_and_enclosures_ignore_extended_knot_tails() {
        let c = active_quadratic();
        let b = c.enclose([2., 3.], 1).unwrap();
        assert!(b.min[0] <= 1. && b.max[0] >= 3. && b.max[1] >= 3.);
        let samples = crate::nurbs_sample::sample(
            &c,
            crate::nurbs_sample::Options {
                tolerance: 0.01,
                ..Default::default()
            },
            |_| true,
        )
        .unwrap();
        assert_eq!(samples.parameters()[0], 2.);
        assert_eq!(*samples.parameters().last().unwrap(), 3.);
        assert_eq!(samples.points()[0], [1., 2., 0.]);
        assert_eq!(*samples.points().last().unwrap(), [3., 2., 0.]);
        assert!(samples.parameters().iter().all(|t| *t >= 2. && *t <= 3.));
        let result = crate::nurbs_distance::distance(
            &c,
            [2., 3., 1.],
            crate::nurbs_distance::Options::default(),
            |_| true,
        )
        .unwrap();
        assert!(result.distance_bounds[0] <= 1. && result.distance_bounds[1] >= 1.);
        assert!(result.distance_bounds[1] - result.distance_bounds[0] <= 0.001);
        assert!(NurbsCurve::new_active_domain(
            1,
            vec![0., 0., 0.5, 0.5, 1., 1.],
            vec![[0.; 3]; 4],
            vec![1.; 4],
            Limits::default()
        )
        .is_err());
    }
    #[test]
    fn actual_bracket_extended_closed_parameter_curve_evaluates_without_inferring_periodic_wrap() {
        use crate::step::Value;
        let data = crate::step::inventory(
            include_str!("../../nema17-motor-bracket.step"),
            crate::step::Limits::default(),
        )
        .unwrap();
        let e = &data.entities()[&237];
        let base = e
            .components
            .iter()
            .find(|c| c.name == "B_SPLINE_CURVE")
            .unwrap();
        let degree = crate::step_nurbs::integer(e, &base.args[0]).unwrap();
        let Value::List(refs) = &base.args[1] else {
            panic!("fixture");
        };
        let points = refs
            .iter()
            .map(|r| {
                let Value::Ref(id) = r else {
                    panic!("fixture");
                };
                let point = &data.entities()[id];
                let Value::List(v) = &point.components[0].args[1] else {
                    panic!("fixture");
                };
                assert_eq!(v.len(), 2);
                [
                    crate::analytic_step::scalar(point, &v[0]).unwrap(),
                    crate::analytic_step::scalar(point, &v[1]).unwrap(),
                    0.,
                ]
            })
            .collect::<Vec<_>>();
        let weights = e
            .components
            .iter()
            .find(|c| c.name == "RATIONAL_B_SPLINE_CURVE")
            .unwrap();
        let Value::List(weights) = &weights.args[0] else {
            panic!("fixture");
        };
        let weights = weights
            .iter()
            .map(|v| crate::analytic_step::scalar(e, v).unwrap())
            .collect::<Vec<_>>();
        let knots = e
            .components
            .iter()
            .find(|c| c.name == "B_SPLINE_CURVE_WITH_KNOTS")
            .unwrap();
        let (Value::List(m), Value::List(k)) = (&knots.args[0], &knots.args[1]) else {
            panic!("fixture");
        };
        let knots = crate::step_nurbs::expanded_knots(e, m, k, points.len(), degree).unwrap();
        assert!(NurbsCurve::new_clamped(
            degree,
            knots.clone(),
            points.clone(),
            weights.clone(),
            Limits::default()
        )
        .is_err());
        let c = NurbsCurve::new_active_domain(degree, knots, points, weights, Limits::default())
            .unwrap();
        let [lo, hi] = c.domain();
        assert!(c.knots()[0] < lo && *c.knots().last().unwrap() > hi);
        let a = c.evaluate(lo).unwrap();
        let b = c.evaluate(hi).unwrap();
        for i in 0..3 {
            assert!((a[i] - b[i]).abs() < 1e-9);
        }
        for t in [lo, (lo + hi) * 0.5, hi] {
            assert!(c.evaluate(t).unwrap().iter().all(|x| x.is_finite()));
        }
        assert!(c.evaluate(lo - 1.).is_err());
        assert!(c.evaluate(hi + 1.).is_err());
    }
}
