//! Global distance to the complete untrimmed rational surface parameter domain.
//! Candidate UV is not a unique/regular projection or trimmed-face certificate.
pub use crate::geometry_distance::{Options, Progress};
use crate::{
    geometry_bounds::Bounds3,
    geometry_distance::{self, Geometry},
    nurbs_surface::NurbsSurface,
    step::Error,
    V3,
};
#[derive(Debug)]
pub struct DistanceResult {
    pub distance_bounds: [f64; 2],
    pub candidate_parameters: [f64; 2],
    pub candidate_point: V3,
    pub candidate_enclosure: Bounds3,
    pub enclosures: usize,
}
impl Geometry<2> for NurbsSurface {
    fn domain(&self) -> [[f64; 2]; 2] {
        self.domain()
    }
    fn cells(&self, maximum: usize) -> Result<Vec<[[f64; 2]; 2]>, Error> {
        let mut cells = Vec::new();
        for u in self.knots_u().windows(2) {
            if u[0] == u[1] {
                continue;
            }
            for v in self.knots_v().windows(2) {
                if v[0] != v[1] {
                    geometry_distance::push_cell(
                        &mut cells,
                        [[u[0], u[1]], [v[0], v[1]]],
                        maximum,
                    )?;
                }
            }
        }
        Ok(cells)
    }
    fn enclose(&self, domain: [[f64; 2]; 2]) -> Result<Bounds3, Error> {
        self.enclose(domain, 9)
    }
    fn evaluate(&self, uv: [f64; 2]) -> Result<V3, Error> {
        self.evaluate(uv)
    }
}
/// Atomic bounded result over the entire untrimmed domain; uncertainty concerns
/// the distance objective, not UV/point error. Singular/nonunique minima may
/// still yield valid distance brackets, with no regularity/uniqueness claim.
pub fn distance(
    surface: &NurbsSurface,
    query: V3,
    opt: Options,
    progress: impl FnMut(Progress) -> bool,
) -> Result<DistanceResult, Error> {
    let r = geometry_distance::distance(surface, query, opt, progress)?;
    Ok(DistanceResult {
        distance_bounds: r.bounds,
        candidate_parameters: r.parameter,
        candidate_point: r.point,
        candidate_enclosure: r.enclosure,
        enclosures: r.calls,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{nurbs_surface::Limits, step::ErrorKind};
    fn plane() -> NurbsSurface {
        NurbsSurface::new_clamped(
            [1, 1],
            [vec![0., 0., 1., 1.], vec![0., 0., 1., 1.]],
            [2, 2],
            vec![[0., 0., 0.], [1., 0., 0.], [0., 1., 0.], [1., 1., 0.]],
            vec![1.; 4],
            Limits::default(),
        )
        .unwrap()
    }
    fn cylinder() -> NurbsSurface {
        let w = std::f64::consts::FRAC_1_SQRT_2;
        NurbsSurface::new_clamped(
            [2, 1],
            [vec![0., 0., 0., 1., 1., 1.], vec![0., 0., 1., 1.]],
            [3, 2],
            vec![
                [1., 0., 0.],
                [1., 1., 0.],
                [0., 1., 0.],
                [1., 0., 3.],
                [1., 1., 3.],
                [0., 1., 3.],
            ],
            vec![1., w, 1., 1., w, 1.],
            Limits::default(),
        )
        .unwrap()
    }
    fn verify(s: &NurbsSurface, p: V3, exact: f64, tolerance: f64) -> DistanceResult {
        let mut previous = 0;
        let r = distance(
            s,
            p,
            Options {
                distance_tolerance: tolerance,
                ..Options::default()
            },
            |p| {
                assert!(p.enclosures >= previous);
                previous = p.enclosures;
                true
            },
        )
        .unwrap();
        assert!(
            r.distance_bounds[0] <= exact && exact <= r.distance_bounds[1],
            "{exact} {r:?}"
        );
        assert!(r.distance_bounds[1] - r.distance_bounds[0] <= tolerance);
        for (parameter, domain) in r.candidate_parameters.iter().zip(s.domain()) {
            assert!(*parameter >= domain[0] && *parameter <= domain[1]);
        }
        r
    }
    #[test]
    fn complete_rectangle_interior_boundary_corner_and_on_surface_distances_match_analytic_values()
    {
        let s = plane();
        verify(&s, [0.25, 0.75, 2.], 2., 1e-5);
        verify(&s, [-1., -2., 0.], 5f64.sqrt(), 1e-5);
        verify(&s, [0.5, 0.5, 0.], 0., 1e-5);
        verify(&s, [0.5, -1., 0.], 1., 1e-5);
    }
    #[test]
    fn rational_cylinder_global_distance_and_nonunique_minima_are_bounded() {
        let s = cylinder();
        verify(&s, [2., 2., 1.5], 2. * 2f64.sqrt() - 1., 0.01);
        verify(&s, [0., 0., 1.5], 1., 0.02);
    }
    #[test]
    fn all_tensor_cells_are_searched_with_explicit_units_and_singular_candidates() {
        let mut points = Vec::new();
        for j in 0..3 {
            for i in 0..3 {
                points.push([25.4 * i as f64, 25.4 * j as f64, 0.]);
            }
        }
        let s = NurbsSurface::new_clamped(
            [1, 1],
            [vec![10., 10., 20., 30., 30.], vec![-2., -2., 0., 2., 2.]],
            [3, 3],
            points,
            vec![7.; 9],
            Limits::default(),
        )
        .unwrap();
        let r = verify(&s, [40., 40., 3.], 3., 0.001);
        assert!(r.candidate_parameters[0] > 20. && r.candidate_parameters[1] > 0.);
        let s = NurbsSurface::new_clamped(
            [1, 1],
            [vec![0., 0., 1., 1.], vec![0., 0., 1., 1.]],
            [2, 2],
            vec![[1., 2., 3.]; 4],
            vec![1., 3., 7., 11.],
            Limits::default(),
        )
        .unwrap();
        verify(&s, [1., 2., 5.], 2., 1e-5);
    }
    #[test]
    fn budgets_work_limits_and_initial_and_final_cancellation_never_return_partial_results() {
        let s = plane();
        let query = [0.25, 0.75, 2.];
        for opt in [
            Options {
                max_enclosures: 0,
                ..Options::default()
            },
            Options {
                max_pending: 0,
                ..Options::default()
            },
            Options {
                max_enclosures: 5,
                ..Options::default()
            },
        ] {
            assert_eq!(
                distance(&s, query, opt, |_| true).unwrap_err().kind,
                ErrorKind::Limit
            );
        }
        assert!(distance(&s, query, Options::default(), |_| false)
            .unwrap_err()
            .detail
            .contains("cancelled"));
        assert!(
            distance(&s, query, Options::default(), |p| p.enclosures == 0
                || p.pending_intervals != 0)
            .unwrap_err()
            .detail
            .contains("cancelled")
        );
        let limited = NurbsSurface::new_clamped(
            [1, 1],
            [vec![0., 0., 1., 1.], vec![0., 0., 1., 1.]],
            [2, 2],
            s.control_points().to_vec(),
            vec![1.; 4],
            Limits {
                evaluation_blends: 3,
                ..Limits::default()
            },
        )
        .unwrap();
        assert_eq!(
            distance(&limited, query, Options::default(), |_| true)
                .unwrap_err()
                .kind,
            ErrorKind::Limit
        );
        assert!(distance(&s, [f64::INFINITY, 0., 0.], Options::default(), |_| true).is_err());
    }
}
