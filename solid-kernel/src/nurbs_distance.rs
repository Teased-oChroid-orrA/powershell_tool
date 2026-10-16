//! Global point-to-curve distance brackets, not unique/regular CAD projection.
use crate::{
    native_interval::Interval,
    nurbs::{fail, CurveBounds, NurbsCurve},
    step::{Error, ErrorKind},
    V3,
};
use std::{cmp::Ordering, collections::BinaryHeap};
#[derive(Debug, Clone, Copy)]
pub struct Options {
    pub distance_tolerance: f64,
    pub max_enclosures: usize,
    pub max_pending: usize,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            distance_tolerance: 0.001,
            max_enclosures: 100_000,
            max_pending: 100_000,
        }
    }
}
#[derive(Debug, Clone, Copy)]
pub struct Progress {
    pub enclosures: usize,
    pub pending_intervals: usize,
    pub distance_bounds: [f64; 2],
}
#[derive(Debug)]
pub struct DistanceResult {
    /// Distance to the complete curve lies in this closed bracket.
    pub distance_bounds: [f64; 2],
    /// Evaluated representative. The mathematical curve point at this parameter
    /// is within the requested distance gap of the global minimum. This does
    /// not bound parameter error or prove a unique minimum.
    pub candidate_parameter: f64,
    pub candidate_point: V3,
    /// Contains the exact real curve point at candidate_parameter, including
    /// rounding error in candidate_point (which is a binary64 evaluation).
    pub candidate_enclosure: CurveBounds,
    pub enclosures: usize,
}
#[derive(Debug, Clone, Copy)]
struct Node {
    domain: [f64; 2],
    lower: f64,
}
impl PartialEq for Node {
    fn eq(&self, b: &Self) -> bool {
        self.lower == b.lower && self.domain == b.domain
    }
}
impl Eq for Node {}
impl PartialOrd for Node {
    fn partial_cmp(&self, b: &Self) -> Option<Ordering> {
        Some(self.cmp(b))
    }
}
impl Ord for Node {
    fn cmp(&self, b: &Self) -> Ordering {
        b.lower
            .total_cmp(&self.lower)
            .then_with(|| b.domain[0].total_cmp(&self.domain[0]))
            .then_with(|| b.domain[1].total_cmp(&self.domain[1]))
    }
}
fn distance_bounds(b: CurveBounds, point: V3) -> Result<[f64; 2], Error> {
    let mut sum = Interval::point(0.);
    for i in 0..3 {
        let d = Interval::new(b.min[i], b.max[i]) - Interval::point(point[i]);
        let lo = if d.lo > 0. {
            d.lo
        } else if d.hi < 0. {
            -d.hi
        } else {
            0.
        };
        let range = Interval::new(lo, d.lo.abs().max(d.hi.abs()));
        sum = sum + range * range;
    }
    let bounds = [
        sum.lo.max(0.).sqrt().next_down().max(0.),
        sum.hi.sqrt().next_up(),
    ];
    if bounds.iter().any(|x| !x.is_finite()) {
        return Err(fail(
            ErrorKind::Geometry,
            "nonfinite curve distance enclosure",
        ));
    }
    Ok(bounds)
}
struct Search<'a> {
    curve: &'a NurbsCurve,
    query: V3,
    opt: Options,
    calls: usize,
    best: Option<(f64, f64, CurveBounds)>,
}
impl Search<'_> {
    fn enclose(&mut self, domain: [f64; 2]) -> Result<CurveBounds, Error> {
        if self.calls >= self.opt.max_enclosures {
            return Err(fail(
                ErrorKind::Limit,
                "curve distance enclosure budget exceeded",
            ));
        }
        self.calls += 1;
        self.curve.enclose(domain, 3)
    }
    fn candidate(&mut self, t: f64) -> Result<(), Error> {
        let enclosure = self.enclose([t, t])?;
        let upper = distance_bounds(enclosure, self.query)?[1];
        if self.best.as_ref().is_none_or(|best| upper < best.0) {
            self.best = Some((upper, t, enclosure));
        }
        Ok(())
    }
    fn node(&mut self, domain: [f64; 2]) -> Result<Node, Error> {
        let bounds = self.enclose(domain)?;
        Ok(Node {
            domain,
            lower: distance_bounds(bounds, self.query)?[0],
        })
    }
}
/// Deterministic branch-and-bound over every nonempty knot span. Termination
/// requires an outward-rounded global distance gap <= tolerance. No unconverged
/// result escapes on cancellation, work/queue exhaustion or parameter precision.
pub fn distance(
    curve: &NurbsCurve,
    query: V3,
    opt: Options,
    mut progress: impl FnMut(Progress) -> bool,
) -> Result<DistanceResult, Error> {
    if !opt.distance_tolerance.is_finite()
        || opt.distance_tolerance <= 0.
        || query.iter().any(|x| !x.is_finite() || x.abs() > 1e30)
    {
        return Err(fail(
            ErrorKind::Geometry,
            "invalid curve distance query/tolerance",
        ));
    }
    let cancelled = || fail(ErrorKind::Geometry, "curve distance query cancelled");
    if !progress(Progress {
        enclosures: 0,
        pending_intervals: 0,
        distance_bounds: [0., f64::INFINITY],
    }) {
        return Err(cancelled());
    }
    let mut search = Search {
        curve,
        query,
        opt,
        calls: 0,
        best: None,
    };
    let mut queue = BinaryHeap::new();
    search.candidate(curve.domain()[0])?;
    for k in curve.knots().windows(2) {
        if k[0] == k[1] {
            continue;
        }
        if queue.len() >= opt.max_pending {
            return Err(fail(
                ErrorKind::Limit,
                "curve distance pending-interval budget exceeded",
            ));
        }
        let domain = [k[0], k[1]];
        queue.push(search.node(domain)?);
        search.candidate(k[1])?;
        search.candidate(k[0] + (k[1] - k[0]) * 0.5)?;
        let upper = search.best.unwrap().0;
        if !progress(Progress {
            enclosures: search.calls,
            pending_intervals: queue.len(),
            distance_bounds: [0., upper],
        }) {
            return Err(cancelled());
        }
    }
    loop {
        let (upper, t, enclosure) = search.best.unwrap();
        let lower = queue.peek().map_or(upper, |node| node.lower.min(upper));
        let bounds = [lower, upper];
        let state = Progress {
            enclosures: search.calls,
            pending_intervals: queue.len(),
            distance_bounds: bounds,
        };
        if !progress(state) {
            return Err(cancelled());
        }
        if (Interval::point(upper) - Interval::point(lower)).hi <= opt.distance_tolerance {
            let result = DistanceResult {
                distance_bounds: bounds,
                candidate_parameter: t,
                candidate_point: curve.evaluate(t)?,
                candidate_enclosure: enclosure,
                enclosures: search.calls,
            };
            // Explicit final cancellation boundary, even for immediate matches.
            if !progress(Progress {
                pending_intervals: 0,
                ..state
            }) {
                return Err(cancelled());
            }
            return Ok(result);
        }
        let node = queue.pop().unwrap();
        if node.lower >= upper {
            continue;
        }
        let middle = node.domain[0] + (node.domain[1] - node.domain[0]) * 0.5;
        if middle <= node.domain[0] || middle >= node.domain[1] {
            return Err(fail(
                ErrorKind::Geometry,
                "curve distance parameter precision exhausted",
            ));
        }
        search.candidate(middle)?;
        for domain in [[node.domain[0], middle], [middle, node.domain[1]]] {
            let child = search.node(domain)?;
            if child.lower < search.best.unwrap().0 {
                if queue.len() >= opt.max_pending {
                    return Err(fail(
                        ErrorKind::Limit,
                        "curve distance pending-interval budget exceeded",
                    ));
                }
                queue.push(child);
            }
        }
    }
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
    fn verify(c: &NurbsCurve, p: V3, exact: f64, tolerance: f64) -> DistanceResult {
        let mut previous = 0;
        let r = distance(
            c,
            p,
            Options {
                distance_tolerance: tolerance,
                ..Options::default()
            },
            |s| {
                assert!(s.enclosures >= previous);
                previous = s.enclosures;
                true
            },
        )
        .unwrap();
        assert!(
            r.distance_bounds[0] <= exact && exact <= r.distance_bounds[1],
            "exact={exact}, {r:?}"
        );
        assert!(r.distance_bounds[1] - r.distance_bounds[0] <= tolerance);
        r
    }
    #[test]
    fn analytic_segment_interior_endpoint_and_degenerate_curve_distances_are_bracketed() {
        let c = NurbsCurve::new_clamped(
            1,
            vec![0., 0., 1., 1.],
            vec![[0., 0., 0.], [1., 0., 0.]],
            vec![1., 7.],
            Limits::default(),
        )
        .unwrap();
        verify(&c, [0.25, 1., 0.], 1., 1e-6);
        verify(&c, [-2., 0., 0.], 2., 1e-6);
        let c = NurbsCurve::new_clamped(
            1,
            vec![0., 0., 1., 1.],
            vec![[1., 2., 3.]; 2],
            vec![1., 7.],
            Limits::default(),
        )
        .unwrap();
        verify(&c, [1., 2., 5.], 2., 1e-6);
    }
    #[test]
    fn analytic_arc_global_distances_include_nonunique_and_endpoint_minima() {
        let c = quarter();
        verify(&c, [2., 2., 0.], 2. * 2f64.sqrt() - 1., 0.001);
        let r = verify(&c, [0., 0., 0.], 1., 0.01);
        assert!(r.candidate_parameter >= 0. && r.candidate_parameter <= 1.);
        verify(&c, [2., -1., 0.], 2f64.sqrt(), 0.001);
    }
    #[test]
    fn all_knot_spans_are_searched_and_units_and_parameter_domains_remain_explicit() {
        let c = NurbsCurve::new_clamped(
            1,
            vec![0., 0., 0.5, 1., 1.],
            vec![[0., 0., 0.], [10., 0., 0.], [10., 10., 0.]],
            vec![1.; 3],
            Limits::default(),
        )
        .unwrap();
        let r = verify(&c, [9., 7., 0.], 1., 1e-5);
        assert!(r.candidate_parameter > 0.5);
        let original = quarter();
        let scaled = NurbsCurve::new_clamped(
            2,
            original.knots().iter().map(|x| 10. + 20. * x).collect(),
            original
                .control_points()
                .iter()
                .map(|p| p.map(|x| x * 25.4))
                .collect(),
            original.weights().iter().map(|w| w * 7.).collect(),
            Limits::default(),
        )
        .unwrap();
        let r = verify(
            &scaled,
            [50.8, 50.8, 0.],
            (2. * 2f64.sqrt() - 1.) * 25.4,
            0.01,
        );
        assert!(r.candidate_parameter >= 10. && r.candidate_parameter <= 30.);
    }
    #[test]
    fn work_queue_input_and_cancellation_failures_never_return_unconverged_results() {
        let c = quarter();
        let query = [2., 2., 0.];
        for opt in [
            Options {
                max_enclosures: 0,
                ..Options::default()
            },
            Options {
                max_enclosures: 3,
                ..Options::default()
            },
            Options {
                max_pending: 0,
                ..Options::default()
            },
            Options {
                max_pending: 1,
                distance_tolerance: 1e-12,
                ..Options::default()
            },
        ] {
            assert_eq!(
                distance(&c, query, opt, |_| true).unwrap_err().kind,
                ErrorKind::Limit
            );
        }
        assert!(distance(&c, query, Options::default(), |_| false)
            .unwrap_err()
            .detail
            .contains("cancelled"));
        assert!(
            distance(&c, query, Options::default(), |p| p.enclosures == 0
                || p.pending_intervals != 0)
            .unwrap_err()
            .detail
            .contains("cancelled")
        );
        for tolerance in [0., -1., f64::NAN, f64::INFINITY] {
            assert!(distance(
                &c,
                query,
                Options {
                    distance_tolerance: tolerance,
                    ..Options::default()
                },
                |_| true
            )
            .is_err());
        }
        assert!(distance(&c, [f64::NAN, 0., 0.], Options::default(), |_| true).is_err());
    }
}
