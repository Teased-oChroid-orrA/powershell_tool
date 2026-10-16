//! Global point-to-curve distance brackets, not unique/regular CAD projection.
pub use crate::geometry_distance::{Options, Progress};
use crate::{
    geometry_distance::{self, Geometry},
    nurbs::{CurveBounds, NurbsCurve},
    step::Error,
    V3,
};
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

impl Geometry<1> for NurbsCurve {
    fn domain(&self) -> [[f64; 2]; 1] {
        [self.domain()]
    }
    fn cells(&self, maximum: usize) -> Result<Vec<[[f64; 2]; 1]>, Error> {
        let mut cells = Vec::new();
        for k in self.knots().windows(2) {
            if k[0] != k[1] {
                geometry_distance::push_cell(&mut cells, [[k[0], k[1]]], maximum)?;
            }
        }
        Ok(cells)
    }
    fn enclose(&self, domain: [[f64; 2]; 1]) -> Result<CurveBounds, Error> {
        self.enclose(domain[0], 3)
    }
    fn evaluate(&self, t: [f64; 1]) -> Result<V3, Error> {
        self.evaluate(t[0])
    }
}
/// Globally bounded distance gap; no unique nearest-parameter/position claim.
pub fn distance(
    curve: &NurbsCurve,
    query: V3,
    opt: Options,
    progress: impl FnMut(Progress) -> bool,
) -> Result<DistanceResult, Error> {
    let r = geometry_distance::distance(curve, query, opt, progress)?;
    Ok(DistanceResult {
        distance_bounds: r.bounds,
        candidate_parameter: r.parameter[0],
        candidate_point: r.point,
        candidate_enclosure: r.enclosure,
        enclosures: r.calls,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::nurbs::Limits;
    use crate::step::ErrorKind;
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
        assert_eq!(
            distance(&c, query, Options::default(), |_| false)
                .unwrap_err()
                .kind,
            ErrorKind::Cancelled
        );
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
