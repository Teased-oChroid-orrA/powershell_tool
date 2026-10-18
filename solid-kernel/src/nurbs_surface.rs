//! Bounded tensor-product rational surface values, differentials and enclosures.
//! Standalone numerical geometry, not unique projection, exact trims or B-Rep validity.
use crate::{
    nurbs::{self, Basis, Limits as CurveLimits},
    step::{Error, ErrorKind},
    V3,
};
use fea_core::derivative::{dual::Dual, Scalar};
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
    work_limit: usize,
}
#[derive(Debug, Clone, Copy)]
pub struct SurfaceDifferential {
    pub point: V3,
    pub du: V3,
    pub dv: V3,
    pub sides: [nurbs::ParameterSide; 2],
}
impl SurfaceDifferential {
    /// Computed tangent-cross direction, not a CAD regularity certificate.
    /// Scaling directions first avoids overflowing/underflowing their cross.
    pub fn computed_unit_normal(&self) -> Result<V3, Error> {
        if self.point.iter().any(|x| !x.is_finite()) {
            return Err(nurbs::fail(
                ErrorKind::Geometry,
                "nonfinite surface differential point",
            ));
        }
        let normalized = |v| {
            crate::native_math::normalize_direction(v).map_err(|_| {
                nurbs::fail(
                    ErrorKind::Geometry,
                    "singular or invalid surface differential",
                )
            })
        };
        let u = normalized(self.du)?;
        let v = normalized(self.dv)?;
        normalized(crate::cross(u, v))
    }
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
        Self::build(degrees, knots, dimensions, control_points, weights, limits, true)
    }
    /// Like [`Self::new_clamped`], but unclamped knot vectors are accepted and the surface is evaluated on the active domain
    /// `[knots[degree], knots[count]]` of each parameter only (how OpenCASCADE writes a periodic surface: the control net is
    /// already extended and nothing is wrapped). Only point evaluation and the domain are meant to be used on such a surface.
    pub fn new_active_domain(
        degrees: [usize; 2],
        knots: [Vec<f64>; 2],
        dimensions: [usize; 2],
        control_points: Vec<V3>,
        weights: Vec<f64>,
        limits: Limits,
    ) -> Result<Self, Error> {
        Self::build(degrees, knots, dimensions, control_points, weights, limits, false)
    }
    fn build(
        degrees: [usize; 2],
        knots: [Vec<f64>; 2],
        dimensions: [usize; 2],
        control_points: Vec<V3>,
        weights: Vec<f64>,
        limits: Limits,
        clamped: bool,
    ) -> Result<Self, Error> {
        let basis = |degree, knots, n, axis: &str| {
            let b = if clamped {
                Basis::new(degree, knots, n, CurveLimits { control_points: limits.control_points, degree: limits.degree })
            } else {
                Basis::new_active(degree, knots, n, CurveLimits { control_points: limits.control_points, degree: limits.degree })
            };
            b.map_err(|mut e| {
                e.detail = format!("surface {axis} basis: {}", e.detail);
                e
            })
        };
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
        let u = basis(degrees[0], uk, dimensions[0], "u")?;
        let v = basis(degrees[1], vk, dimensions[1], "v")?;
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
            work_limit: limits.evaluation_blends,
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
    /// Enclose the exact real tensor surface over a closed U/V rectangle.
    /// Uses shared outward-rounded de Boor arithmetic without row projection.
    /// Patch and two-lane blend budgets refuse before returning partial bounds.
    /// This is not a regularity, injectivity, trim or unique-projection proof.
    pub fn enclose(
        &self,
        domain: [[f64; 2]; 2],
        max_patches: usize,
    ) -> Result<crate::geometry_bounds::Bounds3, Error> {
        use crate::native_interval::Interval;
        let urange = self.u.span_range(domain[0])?;
        let vrange = self.v.span_range(domain[1])?;
        let work = self
            .blends
            .checked_mul(2)
            .ok_or_else(|| nurbs::fail(ErrorKind::Limit, "surface interval work overflow"))?;
        let mut used = 0usize;
        let mut patches = 0usize;
        let mut result: Option<[Interval; 3]> = None;
        for us in urange {
            if self.u.knots()[us] == self.u.knots()[us + 1] {
                continue;
            }
            let up = Interval::new(
                domain[0][0].max(self.u.knots()[us]),
                domain[0][1].min(self.u.knots()[us + 1]),
            );
            for vs in vrange.clone() {
                if self.v.knots()[vs] == self.v.knots()[vs + 1] {
                    continue;
                }
                if patches >= max_patches || self.work_limit.saturating_sub(used) < work {
                    return Err(nurbs::fail(
                        ErrorKind::Limit,
                        "surface interval patch/blend budget exceeded",
                    ));
                }
                patches += 1;
                used += work;
                let vp = Interval::new(
                    domain[1][0].max(self.v.knots()[vs]),
                    domain[1][1].min(self.v.knots()[vs + 1]),
                );
                let h = self.v.homogeneous(vs, vp, |j| {
                    self.u.homogeneous(us, up, |i| {
                        let k = j * self.dimensions[0] + i;
                        let w = Interval::point(self.weights[k]);
                        let p = self.control_points[k].map(Interval::point);
                        Ok([p[0] * w, p[1] * w, p[2] * w, w])
                    })
                })?;
                if h[3].lo <= 0. {
                    return Err(nurbs::fail(
                        ErrorKind::Geometry,
                        "surface interval denominator not positive",
                    ));
                }
                let projected = [h[0] / h[3], h[1] / h[3], h[2] / h[3]];
                let bounds: [Interval; 3] = std::array::from_fn(|axis| {
                    let mut hull = Interval::new(f64::INFINITY, f64::NEG_INFINITY);
                    for j in vs - self.v.degree()..=vs {
                        for i in us - self.u.degree()..=us {
                            hull = hull.hull(Interval::point(
                                self.control_points[j * self.dimensions[0] + i][axis],
                            ));
                        }
                    }
                    projected[axis].intersect(hull)
                });
                if bounds.iter().any(|b| !b.finite()) {
                    return Err(nurbs::fail(
                        ErrorKind::Geometry,
                        "nonfinite surface interval",
                    ));
                }
                result = Some(match result {
                    None => bounds,
                    Some(old) => std::array::from_fn(|i| old[i].hull(bounds[i])),
                });
            }
        }
        let result =
            result.ok_or_else(|| nurbs::fail(ErrorKind::Geometry, "empty surface interval"))?;
        Ok(crate::geometry_bounds::Bounds3 {
            min: result.map(|x| x.lo),
            max: result.map(|x| x.hi),
        })
    }
    /// First parameter partials using the existing dual-number implementation.
    /// Three inline scalar lanes count against the same constructor work cap.
    /// C0 knots refuse and endpoints are explicitly one-sided. No global CAD/
    /// topology/solver sensitivity or geometric regularity certificate implied.
    pub fn differential(&self, parameters: [f64; 2]) -> Result<SurfaceDifferential, Error> {
        let work = self
            .blends
            .checked_mul(3)
            .ok_or_else(|| nurbs::fail(ErrorKind::Limit, "surface differential work overflow"))?;
        if work > self.work_limit {
            return Err(nurbs::fail(
                ErrorKind::Limit,
                "surface differential blend budget exceeded",
            ));
        }
        let [u, v] = parameters;
        let (us, u_side) = self.u.differential_span(u)?;
        let (vs, v_side) = self.v.differential_span(v)?;
        let up = Dual::<f64, 2>::var(u, 0);
        let vp = Dual::<f64, 2>::var(v, 1);
        let h = self.v.homogeneous(vs, vp, |j| {
            self.u.homogeneous(us, up, |i| {
                let k = j * self.dimensions[0] + i;
                Ok(
                    nurbs::homogeneous_control(self.control_points[k], self.weights[k])
                        .map(Dual::<f64, 2>::cst),
                )
            })
        })?;
        let values = nurbs::project(h)?;
        let mut point = values.map(|x| x.v);
        let du = values.map(|x| x.d[0]);
        let dv = values.map(|x| x.d[1]);
        if du.iter().chain(&dv).any(|x| !x.is_finite()) {
            return Err(nurbs::fail(
                ErrorKind::Geometry,
                "nonfinite surface parameter partial",
            ));
        }
        if u_side != nurbs::ParameterSide::Interior && v_side != nurbs::ParameterSide::Interior {
            let i = if u_side == nurbs::ParameterSide::LowerEndpoint {
                0
            } else {
                self.dimensions[0] - 1
            };
            let j = if v_side == nurbs::ParameterSide::LowerEndpoint {
                0
            } else {
                self.dimensions[1] - 1
            };
            point = self.control_points[j * self.dimensions[0] + i];
        }
        Ok(SurfaceDifferential {
            point,
            du,
            dv,
            sides: [u_side, v_side],
        })
    }
    /// Retains homogeneous weights through both basis recurrences; projecting
    /// individual rows first would change the rational surface. No clamping.
    pub fn evaluate(&self, parameters: [f64; 2]) -> Result<V3, Error> {
        let [u, v] = parameters;
        let us = self.u.span(u)?;
        let vs = self.v.span(v)?;
        // a corner of a clamped surface is its corner control point; an unclamped one has none
        let endpoint = |t: f64, domain: [f64; 2], n: usize, clamped: bool| {
            if !clamped {
                None
            } else if t == domain[0] {
                Some(0)
            } else if t == domain[1] {
                Some(n - 1)
            } else {
                None
            }
        };
        if let (Some(i), Some(j)) = (
            endpoint(u, self.u.domain(), self.dimensions[0], self.u.is_clamped()),
            endpoint(v, self.v.domain(), self.dimensions[1], self.v.is_clamped()),
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
    fn quarter_sphere() -> NurbsSurface {
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
        s
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
        let s = quarter_sphere();
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
    #[test]
    fn plane_and_cylinder_parameter_partials_and_normals_match_analytic_geometry() {
        let p = plane(Limits::default()).unwrap();
        let d = p.differential([0.25, 0.75]).unwrap();
        assert_eq!(d.du, [2., 0., 4.]);
        assert_eq!(d.dv, [0., 3., 9.]);
        let n = d.computed_unit_normal().unwrap();
        let norm = 14f64.sqrt();
        for axis in 0..3 {
            assert!((n[axis] - [-2., -3., 1.][axis] / norm).abs() < 1e-12);
        }
        let corner = p.differential([0., 0.]).unwrap();
        assert_eq!(corner.point, p.control_points()[0]);
        assert_eq!(corner.sides, [nurbs::ParameterSide::LowerEndpoint; 2]);
        let s = quarter_cylinder();
        for u in [0., 0.25, 0.5, 0.75, 1.] {
            for v in [0., 0.25, 0.75, 1.] {
                let d = s.differential([u, v]).unwrap();
                let n = d.computed_unit_normal().unwrap();
                assert!((d.point[0] * d.du[0] + d.point[1] * d.du[1]).abs() < 1e-12);
                assert!(d.du[2].abs() < 1e-12);
                for axis in 0..3 {
                    assert!((d.dv[axis] - [0., 0., 3.][axis]).abs() < 1e-12);
                    assert!((n[axis] - [d.point[0], d.point[1], 0.][axis]).abs() < 1e-12);
                }
            }
        }
    }
    #[test]
    fn regular_spherical_normals_match_radial_directions_and_singular_poles_refuse() {
        let s = quarter_sphere();
        for u in [0., 0.25, 0.5, 0.75, 1.] {
            for v in [0., 0.25, 0.5, 0.75] {
                let d = s.differential([u, v]).unwrap();
                let n = d.computed_unit_normal().unwrap();
                for axis in 0..3 {
                    assert!((n[axis] - d.point[axis]).abs() < 1e-12);
                }
            }
        }
        let pole = s.differential([0.3, 1.]).unwrap();
        assert_eq!(pole.point, [0., 0., 1.]);
        assert_eq!(pole.du, [0.; 3]);
        assert!(pole.computed_unit_normal().is_err());
        assert_eq!(pole.sides[1], nurbs::ParameterSide::UpperEndpoint);
    }
    #[test]
    fn differential_work_caps_and_c0_surface_knots_are_explicit_refusals() {
        let s = plane(Limits {
            evaluation_blends: 3,
            ..Limits::default()
        })
        .unwrap();
        assert!(s.evaluate([0.25, 0.75]).is_ok());
        assert_eq!(
            s.differential([0.25, 0.75]).unwrap_err().kind,
            ErrorKind::Limit
        );
        let s = NurbsSurface::new_clamped(
            [1, 1],
            [vec![0., 0., 0.5, 1., 1.], vec![0., 0., 1., 1.]],
            [3, 2],
            vec![
                [0., 0., 0.],
                [1., 0., 0.],
                [1., 1., 0.],
                [0., 0., 1.],
                [1., 0., 1.],
                [1., 1., 1.],
            ],
            vec![1.; 6],
            Limits::default(),
        )
        .unwrap();
        assert_eq!(s.evaluate([0.5, 0.5]).unwrap(), [1., 0., 0.5]);
        assert!(s
            .differential([0.5, 0.5])
            .unwrap_err()
            .detail
            .contains("C0"));
    }
    #[test]
    fn parameter_length_scaling_and_existing_taylor_verifier_check_surface_differentials() {
        let original = quarter_cylinder();
        let scaled = NurbsSurface::new_clamped(
            original.degrees(),
            [
                original.knots_u().iter().map(|u| 10. + 20. * u).collect(),
                original.knots_v().iter().map(|v| -5. + 10. * v).collect(),
            ],
            original.dimensions(),
            original
                .control_points()
                .iter()
                .map(|p| p.map(|x| 25.4 * x))
                .collect(),
            original.weights().iter().map(|w| 7. * w).collect(),
            Limits::default(),
        )
        .unwrap();
        let a = original.differential([0.3, 0.4]).unwrap();
        let b = scaled.differential([16., -1.]).unwrap();
        for axis in 0..3 {
            assert!((b.point[axis] - 25.4 * a.point[axis]).abs() < 1e-12);
            assert!((b.du[axis] - 25.4 / 20. * a.du[axis]).abs() < 1e-12);
            assert!((b.dv[axis] - 25.4 / 10. * a.dv[axis]).abs() < 1e-12);
        }
        let report = fea_core::derivative::verify::taylor_test(
            &|x| original.evaluate([x[0], x[1]]).unwrap().to_vec(),
            &|x, v| {
                let d = original.differential([x[0], x[1]]).unwrap();
                (0..3).map(|i| d.du[i] * v[0] + d.dv[i] * v[1]).collect()
            },
            &[0.3, 0.4],
            &[0.4, -0.3],
            &[0.02, 0.01, 0.005, 0.0025],
            1e-14,
        );
        assert!(report.observed_order > 1.9, "{report:?}");
    }
    #[test]
    fn computed_normals_handle_extreme_direction_scales_and_refuse_zero_or_parallel_partials() {
        for scale in [1e-300, 1e300] {
            let d = SurfaceDifferential {
                point: [0.; 3],
                du: [scale, 0., 0.],
                dv: [0., scale, 0.],
                sides: [nurbs::ParameterSide::Interior; 2],
            };
            assert_eq!(d.computed_unit_normal().unwrap(), [0., 0., 1.]);
        }
        for (du, dv) in [([0.; 3], [0., 1., 0.]), ([1., 0., 0.], [2., 0., 0.])] {
            let d = SurfaceDifferential {
                point: [0.; 3],
                du,
                dv,
                sides: [nurbs::ParameterSide::Interior; 2],
            };
            assert!(d.computed_unit_normal().is_err());
        }
    }
    #[test]
    fn patch_enclosures_contain_independent_plane_cylinder_and_sphere_geometry() {
        let p = plane(Limits::default()).unwrap();
        let b = p.enclose([[0.2, 0.8], [0.1, 0.9]], 1).unwrap();
        for u in [0.2, 0.3, 0.7, 0.8] {
            for v in [0.1, 0.4, 0.8, 0.9] {
                let exact = [2. * u, 3. * v, 1. + 4. * u + 9. * v];
                for i in 0..3 {
                    assert!(b.min[i] <= exact[i] && exact[i] <= b.max[i]);
                }
            }
        }
        for (kind, surface) in [quarter_cylinder(), quarter_sphere()]
            .into_iter()
            .enumerate()
        {
            for domain in [
                [[0., 1.], [0., 1.]],
                [[0.3, 0.31], [0.6, 0.61]],
                [[1., 1.], [1., 1.]],
            ] {
                let b = surface.enclose(domain, 1).unwrap();
                for i in 0..=10 {
                    for j in 0..=10 {
                        let uv = [
                            domain[0][0] + (domain[0][1] - domain[0][0]) * (i as f64 / 10.),
                            domain[1][0] + (domain[1][1] - domain[1][0]) * (j as f64 / 10.),
                        ];
                        // Independent analytic reference, not a rounded evaluator
                        // point which can sit one ulp outside the exact CP hull.
                        let arc = |t: f64| {
                            let w = std::f64::consts::FRAC_1_SQRT_2;
                            let a = (1. - t) * (1. - t);
                            let b = 2. * w * t * (1. - t);
                            let c = t * t;
                            [(a + b) / (a + b + c), (b + c) / (a + b + c)]
                        };
                        let a = arc(uv[0]);
                        let v = arc(uv[1]);
                        let point = if kind == 0 {
                            [a[0], a[1], 3. * uv[1]]
                        } else {
                            [a[0] * v[0], a[1] * v[0], v[1]]
                        };
                        for k in 0..3 {
                            assert!(
                                b.min[k] <= point[k] && point[k] <= b.max[k],
                                "{point:?} {b:?}"
                            );
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn patch_enclosure_budgets_and_domains_refuse_and_cartesian_hulls_keep_constants_exact() {
        let plane = plane(Limits {
            evaluation_blends: 3,
            ..Limits::default()
        })
        .unwrap();
        assert!(plane.evaluate([0.5, 0.5]).is_ok());
        assert_eq!(
            plane.enclose([[0., 1.], [0., 1.]], 1).unwrap_err().kind,
            ErrorKind::Limit
        );
        let s = quarter_cylinder();
        assert_eq!(
            s.enclose([[0., 1.], [0., 1.]], 0).unwrap_err().kind,
            ErrorKind::Limit
        );
        for d in [
            [[-0.1, 1.], [0., 1.]],
            [[0.8, 0.2], [0., 1.]],
            [[0., 1.], [0., f64::NAN]],
        ] {
            assert!(s.enclose(d, 10).is_err());
        }
        let s = NurbsSurface::new_clamped(
            [1, 1],
            [vec![0., 0., 1., 1.], vec![0., 0., 1., 1.]],
            [2, 2],
            vec![[1., 2., 3.]; 4],
            vec![1., 3., 7., 11.],
            Limits::default(),
        )
        .unwrap();
        let b = s.enclose([[0., 1.], [0., 1.]], 1).unwrap();
        assert_eq!(b.min, [1., 2., 3.]);
        assert_eq!(b.max, b.min);
    }
    #[test]
    fn every_intersected_tensor_patch_is_counted_including_c0_endpoint_neighbours() {
        let mut points = Vec::new();
        for j in 0..3 {
            for i in 0..3 {
                points.push([i as f64, j as f64, (2 * i + 3 * j) as f64]);
            }
        }
        let s = NurbsSurface::new_clamped(
            [1, 1],
            [vec![0., 0., 0.5, 1., 1.], vec![0., 0., 0.5, 1., 1.]],
            [3, 3],
            points,
            vec![1.; 9],
            Limits::default(),
        )
        .unwrap();
        let d = [[0.25, 0.75], [0.25, 0.75]];
        assert_eq!(s.enclose(d, 3).unwrap_err().kind, ErrorKind::Limit);
        let b = s.enclose(d, 4).unwrap();
        for u in [0.25, 0.5, 0.75] {
            for v in [0.25, 0.5, 0.75] {
                let p = [2. * u, 2. * v, 4. * u + 6. * v];
                for k in 0..3 {
                    assert!(b.min[k] <= p[k] && p[k] <= b.max[k]);
                }
            }
        }
        assert_eq!(
            s.enclose([[0.5; 2]; 2], 3).unwrap_err().kind,
            ErrorKind::Limit
        );
        assert!(s.enclose([[0.5; 2]; 2], 4).is_ok());
    }
}
