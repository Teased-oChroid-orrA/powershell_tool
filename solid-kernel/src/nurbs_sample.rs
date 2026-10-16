//! Bounded, cancellable rational-curve sampling with whole-interval bounds.
//! This does not certify a simple curve, edge/surface agreement or B-Rep trims.
use crate::{
    native_interval::Interval,
    nurbs::{fail, CurveBounds, NurbsCurve},
    step::{Error, ErrorKind},
    V3,
};
#[derive(Debug, Clone, Copy)]
pub struct Options {
    pub tolerance: f64,
    pub max_segments: usize,
    pub max_depth: usize,
    pub max_enclosures: usize,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            tolerance: 0.001,
            max_segments: 100_000,
            max_depth: 48,
            max_enclosures: 1_000_000,
        }
    }
}
#[derive(Debug, Clone, Copy)]
pub struct Progress {
    pub enclosures: usize,
    pub accepted_segments: usize,
    pub pending_intervals: usize,
}
#[derive(Debug)]
pub struct SampledCurve {
    parameters: Vec<f64>,
    points: Vec<V3>,
    error_bounds: Vec<f64>,
}
impl SampledCurve {
    pub fn parameters(&self) -> &[f64] {
        &self.parameters
    }
    pub fn points(&self) -> &[V3] {
        &self.points
    }
    /// Per-segment symmetric Hausdorff upper bounds in control-coordinate units.
    pub fn error_bounds(&self) -> &[f64] {
        &self.error_bounds
    }
}
// Distance to a convex set is convex, so its maximum over a box is bounded
// by the largest corner distance. Each chosen q is an exact real convex blend
// of the segment endpoints; it need not be the exact nearest point.
fn box_distance(b: CurveBounds, a: V3, z: V3) -> Result<f64, Error> {
    let d: V3 = std::array::from_fn(|i| z[i] - a[i]);
    let length2 = d.iter().map(|x| x * x).sum::<f64>();
    let mut bound = 0f64;
    for corner in 0..8 {
        let p: V3 = std::array::from_fn(|i| {
            if corner & (1 << i) == 0 {
                b.min[i]
            } else {
                b.max[i]
            }
        });
        let raw = std::array::from_fn::<_, 3, _>(|i| (p[i] - a[i]) * d[i])
            .iter()
            .sum::<f64>()
            / length2;
        let s = if raw.is_finite() {
            raw.clamp(0., 1.)
        } else {
            0.
        };
        let t = Interval::point(s);
        let mut squared = Interval::point(0.);
        for i in 0..3 {
            let q = (Interval::point(1.) - t) * Interval::point(a[i]) + t * Interval::point(z[i]);
            let delta = Interval::point(p[i]) - q;
            let magnitude = Interval::point(delta.lo.abs().max(delta.hi.abs()));
            squared = squared + magnitude * magnitude;
        }
        let upper = squared.hi.sqrt().next_up();
        if !upper.is_finite() {
            return Err(fail(
                ErrorKind::Geometry,
                "nonfinite spline sampling distance bound",
            ));
        }
        bound = bound.max(upper);
    }
    Ok(bound)
}
#[derive(Clone, Copy)]
struct Task {
    domain: [f64; 2],
    points: [V3; 2],
    depth: usize,
}
/// Atomic result: cancellation/resource exhaustion never returns partial samples.
/// Every distinct knot is retained; returned endpoints use the evaluator's
/// authoritative endpoint policy. Callback counters describe adaptive work,
/// not a predetermined percentage. Returning false cancels, including at commit.
pub fn sample(
    curve: &NurbsCurve,
    opt: Options,
    mut progress: impl FnMut(Progress) -> bool,
) -> Result<SampledCurve, Error> {
    if !opt.tolerance.is_finite() || opt.tolerance <= 0. {
        return Err(fail(
            ErrorKind::Geometry,
            "require finite positive spline sampling tolerance",
        ));
    }
    let cancelled = || fail(ErrorKind::Geometry, "spline sampling cancelled");
    if !progress(Progress {
        enclosures: 0,
        accepted_segments: 0,
        pending_intervals: 0,
    }) {
        return Err(cancelled());
    }
    let mut spans = Vec::new();
    for k in curve.knots().windows(2) {
        if k[0] == k[1] {
            continue;
        }
        if spans.len() >= opt.max_segments {
            return Err(fail(
                ErrorKind::Limit,
                "spline sampling segment budget exceeded",
            ));
        }
        spans.push([k[0], k[1]]);
    }
    let mut stack = Vec::new();
    for domain in spans.into_iter().rev() {
        stack.push(Task {
            domain,
            points: [curve.evaluate(domain[0])?, curve.evaluate(domain[1])?],
            depth: 0,
        });
    }
    let mut result = SampledCurve {
        parameters: vec![curve.domain()[0]],
        points: vec![curve.evaluate(curve.domain()[0])?],
        error_bounds: Vec::new(),
    };
    let mut enclosures = 0;
    while let Some(task) = stack.pop() {
        if !progress(Progress {
            enclosures,
            accepted_segments: result.error_bounds.len(),
            pending_intervals: stack.len() + 1,
        }) {
            return Err(cancelled());
        }
        if opt.max_enclosures.saturating_sub(enclosures) < 3 {
            return Err(fail(
                ErrorKind::Limit,
                "spline sampling enclosure budget exceeded",
            ));
        }
        enclosures += 3;
        // At most three spans meet a span-contained closed interval: its
        // interior and the two adjacent endpoint-only intersections.
        let b = curve.enclose(task.domain, 3)?;
        let start = curve.enclose([task.domain[0]; 2], 2)?;
        let end = curve.enclose([task.domain[1]; 2], 2)?;
        let endpoint_error = box_distance(start, task.points[0], task.points[0])?
            .max(box_distance(end, task.points[1], task.points[1])?);
        // A positive-weight degree-one span traces exactly the straight segment
        // between its mathematical endpoints. Endpoint rounding is still bounded.
        let error = if curve.degree() == 1 {
            endpoint_error
        } else {
            box_distance(b, task.points[0], task.points[1])?.max(endpoint_error)
        };
        // A continuous curve in a segment's epsilon capsule, with endpoints
        // within epsilon of the segment endpoints, has symmetric Hausdorff
        // distance <= epsilon (intermediate value of axial projection).
        if error <= opt.tolerance {
            result.parameters.push(task.domain[1]);
            result.points.push(task.points[1]);
            result.error_bounds.push(error);
        } else {
            if task.depth >= opt.max_depth {
                return Err(fail(
                    ErrorKind::Limit,
                    "spline sampling depth budget exceeded",
                ));
            }
            if result.error_bounds.len() + stack.len() + 2 > opt.max_segments {
                return Err(fail(
                    ErrorKind::Limit,
                    "spline sampling segment budget exceeded",
                ));
            }
            let middle = task.domain[0] + (task.domain[1] - task.domain[0]) * 0.5;
            if middle <= task.domain[0] || middle >= task.domain[1] {
                return Err(fail(
                    ErrorKind::Geometry,
                    "spline sampling cannot subdivide parameter precision",
                ));
            }
            let point = curve.evaluate(middle)?;
            stack.push(Task {
                domain: [middle, task.domain[1]],
                points: [point, task.points[1]],
                depth: task.depth + 1,
            });
            stack.push(Task {
                domain: [task.domain[0], middle],
                points: [task.points[0], point],
                depth: task.depth + 1,
            });
        }
    }
    if !progress(Progress {
        enclosures,
        accepted_segments: result.error_bounds.len(),
        pending_intervals: 0,
    }) {
        return Err(cancelled());
    }
    Ok(result)
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::nurbs::Limits;
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
    fn point_segment(p: V3, a: V3, b: V3) -> f64 {
        let d: V3 = std::array::from_fn(|i| b[i] - a[i]);
        let t = (0..3).map(|i| (p[i] - a[i]) * d[i]).sum::<f64>()
            / d.iter().map(|x| x * x).sum::<f64>();
        let t = t.clamp(0., 1.);
        (0..3)
            .map(|i| (p[i] - a[i] - t * d[i]).powi(2))
            .sum::<f64>()
            .sqrt()
    }
    #[test]
    fn circle_sampling_bounds_analytic_arc_and_chords_with_monotonic_progress() {
        let c = quarter();
        let mut previous = 0;
        let samples = sample(&c, Options::default(), |p| {
            assert!(p.enclosures >= previous);
            previous = p.enclosures;
            true
        })
        .unwrap();
        assert_eq!(samples.points()[0], [1., 0., 0.]);
        assert_eq!(*samples.points().last().unwrap(), [0., 1., 0.]);
        assert!(samples.parameters().windows(2).all(|x| x[0] < x[1]));
        assert!(samples.error_bounds().iter().all(|e| *e <= 0.001));
        for i in 0..samples.error_bounds().len() {
            let [a, b] = [samples.points()[i], samples.points()[i + 1]];
            for j in 0..=8 {
                let t = samples.parameters()[i]
                    + (samples.parameters()[i + 1] - samples.parameters()[i]) * (j as f64 / 8.);
                let p = c.evaluate(t).unwrap();
                assert!(point_segment(p, a, b) <= samples.error_bounds()[i]);
                let q: V3 =
                    std::array::from_fn(|axis| a[axis] + (b[axis] - a[axis]) * (j as f64 / 8.));
                // Independent analytic distance from the chord to the unit arc.
                assert!((1. - q[0].hypot(q[1])).abs() <= samples.error_bounds()[i]);
            }
        }
        eprintln!(
            "quarter-circle: {} segments; {} enclosure calls",
            samples.error_bounds().len(),
            previous
        );
    }
    #[test]
    fn straight_span_and_c0_knot_are_retained_without_uniform_oversampling() {
        let c = NurbsCurve::new_clamped(
            1,
            vec![0., 0., 0.5, 1., 1.],
            vec![[0., 0., 0.], [2., 3., 1.], [2., 4., 5.]],
            vec![1.; 3],
            Limits::default(),
        )
        .unwrap();
        let s = sample(&c, Options::default(), |_| true).unwrap();
        assert_eq!(s.parameters(), &[0., 0.5, 1.]);
        assert_eq!(s.points(), c.control_points());
    }
    #[test]
    fn hidden_midpoint_excursion_is_refined_and_independently_checked() {
        let c = NurbsCurve::new_clamped(
            3,
            vec![0., 0., 0., 0., 1., 1., 1., 1.],
            vec![[0., 0., 0.], [0., 10., 0.], [1., -10., 0.], [1., 0., 0.]],
            vec![1.; 4],
            Limits::default(),
        )
        .unwrap();
        assert_eq!(c.evaluate(0.5).unwrap()[1], 0.);
        let s = sample(
            &c,
            Options {
                tolerance: 0.01,
                ..Options::default()
            },
            |_| true,
        )
        .unwrap();
        assert!(s.points().iter().any(|p| p[1] > 2.));
        assert!(s.points().iter().any(|p| p[1] < -2.));
        for i in 0..s.error_bounds().len() {
            for j in 0..=4 {
                let t = s.parameters()[i]
                    + (s.parameters()[i + 1] - s.parameters()[i]) * (j as f64 / 4.);
                let p = [
                    3. * t * t - 2. * t * t * t,
                    30. * t * (1. - t) * (1. - 2. * t),
                    0.,
                ];
                assert!(point_segment(p, s.points()[i], s.points()[i + 1]) <= s.error_bounds()[i]);
            }
        }
    }
    #[test]
    fn cancellation_and_all_budgets_refuse_without_a_partial_result() {
        let c = quarter();
        assert!(sample(&c, Options::default(), |_| false)
            .unwrap_err()
            .detail
            .contains("cancelled"));
        assert!(sample(&c, Options::default(), |p| p.enclosures == 0
            || p.pending_intervals != 0)
        .unwrap_err()
        .detail
        .contains("cancelled"));
        for opt in [
            Options {
                max_segments: 0,
                ..Options::default()
            },
            Options {
                max_segments: 1,
                ..Options::default()
            },
            Options {
                max_depth: 0,
                ..Options::default()
            },
            Options {
                max_enclosures: 2,
                ..Options::default()
            },
        ] {
            assert_eq!(
                sample(&c, opt, |_| true).unwrap_err().kind,
                ErrorKind::Limit
            );
        }
        for tolerance in [0., -1., f64::NAN, f64::INFINITY] {
            assert!(sample(
                &c,
                Options {
                    tolerance,
                    ..Options::default()
                },
                |_| true
            )
            .is_err());
        }
    }
}
