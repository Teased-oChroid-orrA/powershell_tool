//! Bounded tensor-product rational surface evaluation. Not STEP surface import,
//! normals/projection, exact trims or a B-Rep/solid validity certificate.
use crate::{
    nurbs::{self, Basis, Limits as CurveLimits},
    step::{Error, ErrorKind},
    V3,
};
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub control_points: usize,
    pub degree: usize,
    pub evaluation_blends: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            control_points: 4096,
            degree: 16,
            evaluation_blends: 100_000,
        }
    }
}
#[derive(Debug, Clone)]
pub struct NurbsSurface {
    u: Basis,
    v: Basis,
    dimensions: [usize; 2],
    control_points: Vec<V3>,
    weights: Vec<f64>,
    blends: usize,
}
impl NurbsSurface {
    /// Control net is row-major: index = v_index * u_count + u_index.
    /// The work cap counts four-component homogeneous interpolation blends.
    pub fn new_clamped(
        degrees: [usize; 2],
        knots: [Vec<f64>; 2],
        dimensions: [usize; 2],
        control_points: Vec<V3>,
        weights: Vec<f64>,
        limits: Limits,
    ) -> Result<Self, Error> {
        let count = dimensions[0]
            .checked_mul(dimensions[1])
            .ok_or_else(|| nurbs::fail(ErrorKind::Limit, "surface control-net count overflow"))?;
        if count == 0 || count != control_points.len() {
            return Err(nurbs::fail(
                ErrorKind::Geometry,
                "surface control-net dimensions/count mismatch",
            ));
        }
        let curve_limits = CurveLimits {
            control_points: limits.control_points,
            degree: limits.degree,
        };
        nurbs::validate_controls(&control_points, &weights, curve_limits)?;
        let [uk, vk] = knots;
        let u = Basis::new(degrees[0], uk, dimensions[0], curve_limits).map_err(|mut e| {
            e.detail = format!("surface u basis: {}", e.detail);
            e
        })?;
        let v = Basis::new(degrees[1], vk, dimensions[1], curve_limits).map_err(|mut e| {
            e.detail = format!("surface v basis: {}", e.detail);
            e
        })?;
        let blend = |p: usize| {
            p.checked_add(1)
                .and_then(|n| p.checked_mul(n))
                .map(|n| n / 2)
        };
        let blends = blend(degrees[0])
            .and_then(|n| degrees[1].checked_add(1).and_then(|q| n.checked_mul(q)))
            .and_then(|n| blend(degrees[1]).and_then(|m| n.checked_add(m)))
            .ok_or_else(|| {
                nurbs::fail(ErrorKind::Limit, "surface evaluation work count overflow")
            })?;
        if blends > limits.evaluation_blends {
            return Err(nurbs::fail(
                ErrorKind::Limit,
                "surface evaluation blend budget exceeded",
            ));
        }
        Ok(Self {
            u,
            v,
            dimensions,
            control_points,
            weights,
            blends,
        })
    }
    pub fn domain(&self) -> [[f64; 2]; 2] {
        [self.u.domain(), self.v.domain()]
    }
    pub fn degrees(&self) -> [usize; 2] {
        [self.u.degree(), self.v.degree()]
    }
    pub fn dimensions(&self) -> [usize; 2] {
        self.dimensions
    }
    pub fn control_points(&self) -> &[V3] {
        &self.control_points
    }
    pub fn weights(&self) -> &[f64] {
        &self.weights
    }
    pub fn knots_u(&self) -> &[f64] {
        self.u.knots()
    }
    pub fn knots_v(&self) -> &[f64] {
        self.v.knots()
    }
    pub fn evaluation_blends(&self) -> usize {
        self.blends
    }
    /// Retains homogeneous weights through both basis recurrences; projecting
    /// individual rows first would change the rational surface. No clamping.
    pub fn evaluate(&self, parameters: [f64; 2]) -> Result<V3, Error> {
        let [u, v] = parameters;
        let us = self.u.span(u)?;
        let vs = self.v.span(v)?;
        let endpoint = |t: f64, domain: [f64; 2], n: usize| {
            if t == domain[0] {
                Some(0)
            } else if t == domain[1] {
                Some(n - 1)
            } else {
                None
            }
        };
        if let (Some(i), Some(j)) = (
            endpoint(u, self.u.domain(), self.dimensions[0]),
            endpoint(v, self.v.domain(), self.dimensions[1]),
        ) {
            return Ok(self.control_points[j * self.dimensions[0] + i]);
        }
        let h = self.v.homogeneous(vs, v, |j| {
            self.u.homogeneous(us, u, |i| {
                let k = j * self.dimensions[0] + i;
                Ok(nurbs::homogeneous_control(
                    self.control_points[k],
                    self.weights[k],
                ))
            })
        })?;
        nurbs::project(h)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn plane(limits: Limits) -> Result<NurbsSurface, Error> {
        NurbsSurface::new_clamped(
            [1, 1],
            [vec![0., 0., 1., 1.], vec![0., 0., 1., 1.]],
            [2, 2],
            vec![[0., 0., 1.], [2., 0., 5.], [0., 3., 10.], [2., 3., 14.]],
            vec![1.; 4],
            limits,
        )
    }
    fn quarter_cylinder() -> NurbsSurface {
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
    #[test]
    fn bilinear_plane_and_authoritative_corners_match_analytic_coordinates() {
        let s = plane(Limits::default()).unwrap();
        assert_eq!(s.evaluate([0.25, 0.75]).unwrap(), [0.5, 2.25, 8.75]);
        for (uv, k) in [([0., 0.], 0), ([1., 0.], 1), ([0., 1.], 2), ([1., 1.], 3)] {
            assert_eq!(s.evaluate(uv).unwrap(), s.control_points()[k]);
        }
        assert_eq!(s.evaluation_blends(), 3);
    }
    #[test]
    fn rational_quarter_cylinder_matches_radius_and_axial_coordinates() {
        let s = quarter_cylinder();
        assert_eq!(s.evaluation_blends(), 7);
        for u in [0., 0.1, 0.3, 0.5, 0.9, 1.] {
            for v in [0., 0.25, 0.75, 1.] {
                let p = s.evaluate([u, v]).unwrap();
                assert!((p[0] * p[0] + p[1] * p[1] - 1.).abs() < 1e-12);
                assert!((p[2] - 3. * v).abs() < 1e-12);
            }
        }
    }
    #[test]
    fn rational_spherical_patch_preserves_analytic_radius_including_pole() {
        let w = std::f64::consts::FRAC_1_SQRT_2;
        let s = NurbsSurface::new_clamped(
            [2, 2],
            [vec![0., 0., 0., 1., 1., 1.], vec![0., 0., 0., 1., 1., 1.]],
            [3, 3],
            vec![
                [1., 0., 0.],
                [1., 1., 0.],
                [0., 1., 0.],
                [1., 0., 1.],
                [1., 1., 1.],
                [0., 1., 1.],
                [0., 0., 1.],
                [0., 0., 1.],
                [0., 0., 1.],
            ],
            vec![1., w, 1., w, w * w, w, 1., w, 1.],
            Limits::default(),
        )
        .unwrap();
        assert_eq!(s.evaluation_blends(), 12);
        for u in [0., 0.125, 0.5, 0.875, 1.] {
            for v in [0., 0.25, 0.5, 0.75, 1.] {
                let p = s.evaluate([u, v]).unwrap();
                assert!((p.iter().map(|x| x * x).sum::<f64>() - 1.).abs() < 1e-12);
                if v == 1. {
                    assert_eq!(p, [0., 0., 1.]);
                }
            }
        }
    }
    #[test]
    fn affine_parameter_and_global_weight_scaling_leave_the_surface_unchanged() {
        let original = quarter_cylinder();
        let scaled = NurbsSurface::new_clamped(
            original.degrees(),
            [
                original.knots_u().iter().map(|u| 10. + 20. * u).collect(),
                original.knots_v().iter().map(|v| -5. + 10. * v).collect(),
            ],
            original.dimensions(),
            original.control_points().to_vec(),
            original.weights().iter().map(|w| 7. * w).collect(),
            Limits::default(),
        )
        .unwrap();
        for u in [0., 0.125, 0.5, 0.875, 1.] {
            for v in [0., 0.25, 0.75, 1.] {
                let a = original.evaluate([u, v]).unwrap();
                let b = scaled.evaluate([10. + 20. * u, -5. + 10. * v]).unwrap();
                for axis in 0..3 {
                    assert!((a[axis] - b[axis]).abs() < 1e-12);
                }
            }
        }
    }
    #[test]
    fn invalid_grids_parameters_and_global_work_budgets_refuse() {
        let s = plane(Limits::default()).unwrap();
        for parameters in [
            [-1e-12, 0.5],
            [0.5, 1. + 1e-12],
            [f64::NAN, 0.5],
            [0.5, f64::INFINITY],
        ] {
            assert!(s.evaluate(parameters).is_err());
        }
        for limits in [
            Limits {
                control_points: 3,
                ..Limits::default()
            },
            Limits {
                degree: 0,
                ..Limits::default()
            },
            Limits {
                evaluation_blends: 1,
                ..Limits::default()
            },
        ] {
            assert_eq!(plane(limits).unwrap_err().kind, ErrorKind::Limit);
        }
        assert_eq!(
            NurbsSurface::new_clamped(
                [1, 1],
                [vec![], vec![]],
                [usize::MAX, 2],
                vec![],
                vec![],
                Limits::default()
            )
            .unwrap_err()
            .kind,
            ErrorKind::Limit
        );
        assert!(NurbsSurface::new_clamped(
            [1, 1],
            [s.knots_u().to_vec(), s.knots_v().to_vec()],
            [3, 2],
            s.control_points().to_vec(),
            s.weights().to_vec(),
            Limits::default()
        )
        .is_err());
        let mut weights = s.weights().to_vec();
        weights[1] = 0.;
        assert!(NurbsSurface::new_clamped(
            s.degrees(),
            [s.knots_u().to_vec(), s.knots_v().to_vec()],
            s.dimensions(),
            s.control_points().to_vec(),
            weights,
            Limits::default()
        )
        .is_err());
    }
}
