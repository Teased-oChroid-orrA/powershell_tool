//! Strict planar bilinear-patch geometry for the native analytic facade.
//! No general curved patch, inverse-UV or p-curve interpretation.
use crate::{
    native_interval::Interval,
    nurbs_surface::NurbsSurface,
    step::{Entity, Error, ErrorKind},
    V3,
};
use std::cmp::Ordering;
fn fail(e: &Entity, message: &str) -> Error {
    Error {
        offset: e.offset,
        kind: ErrorKind::Geometry,
        detail: message.into(),
    }
}
fn orient(a: [f64; 2], b: [f64; 2], c: [f64; 2], e: &Entity) -> Result<Ordering, Error> {
    // Reuse the audited exact-sign 3D predicate on z=0, against z=1.
    fea_core::meshing::tet_topology::orientation(
        [a[0], a[1], 0.],
        [b[0], b[1], 0.],
        [c[0], c[1], 0.],
        [0., 0., 1.],
    )
    .map_err(|error| fail(e, &format!("planar spline predicate: {error}")))
}
#[derive(Debug)]
pub(crate) struct PlanarPatch {
    controls: [V3; 4],
    axes: [usize; 2],
    polygon: [[f64; 2]; 4],
    normal: [Interval; 3],
    normal_length: Interval,
}
impl PlanarPatch {
    pub fn new(surface: &NurbsSurface, e: &Entity) -> Result<Self, Error> {
        if surface.degrees() != [1, 1] || surface.dimensions() != [2, 2] {
            return Err(fail(
                e,
                "native spline faces require a two-by-two degree-one planar convex patch",
            ));
        }
        let p: [V3; 4] = surface.control_points().try_into().unwrap();
        if fea_core::meshing::tet_topology::orientation(p[0], p[1], p[2], p[3])
            .map_err(|error| fail(e, &error.to_string()))?
            != Ordering::Equal
        {
            return Err(fail(
                e,
                "bilinear spline controls are not exactly coplanar; no implicit flattening",
            ));
        }
        let a: V3 = std::array::from_fn(|i| p[1][i] - p[0][i]);
        let b: V3 = std::array::from_fn(|i| p[2][i] - p[0][i]);
        let du: [Interval; 3] =
            std::array::from_fn(|i| Interval::point(p[1][i]) - Interval::point(p[0][i]));
        let dv: [Interval; 3] =
            std::array::from_fn(|i| Interval::point(p[2][i]) - Interval::point(p[0][i]));
        let exact_normal: [Interval; 3] = std::array::from_fn(|i| {
            du[(i + 1) % 3] * dv[(i + 2) % 3] - du[(i + 2) % 3] * dv[(i + 1) % 3]
        });
        let normal_length =
            (exact_normal[0].square() + exact_normal[1].square() + exact_normal[2].square())
                .sqrt_nonnegative();
        if !normal_length.finite() || normal_length.lo <= 0. {
            return Err(fail(e, "planar spline normal interval is unresolved"));
        }
        let normal = crate::cross(a, b);
        let drop = (0..3)
            .max_by(|&i, &j| normal[i].abs().total_cmp(&normal[j].abs()))
            .unwrap();
        let axes = match drop {
            0 => [1, 2],
            1 => [0, 2],
            _ => [0, 1],
        };
        let mut polygon = [p[0], p[1], p[3], p[2]].map(|p| [p[axes[0]], p[axes[1]]]);
        let sign = orient(polygon[0], polygon[1], polygon[2], e)?;
        if sign == Ordering::Equal {
            return Err(fail(e, "degenerate planar spline control quadrilateral"));
        }
        for i in 1..4 {
            if orient(polygon[i], polygon[(i + 1) % 4], polygon[(i + 2) % 4], e)? != sign {
                return Err(fail(
                    e,
                    "folded, concave or unresolved planar spline control quadrilateral",
                ));
            }
        }
        if sign == Ordering::Less {
            polygon.reverse();
        }
        // A positive-weight rational bilinear patch has a bilinear homogeneous
        // Jacobian determinant. Its four corner signs are positive weight
        // products times the four adjacent control-triangle signs. Strict
        // convexity makes every sign agree; the injective boundary plus the
        // nonzero interior Jacobian gives the whole convex quadrilateral image.
        Ok(Self {
            controls: p,
            axes,
            polygon,
            normal: exact_normal,
            normal_length,
        })
    }
    fn dot_normal(&self, point: [Interval; 3]) -> Interval {
        self.normal[0] * point[0] + self.normal[1] * point[1] + self.normal[2] * point[2]
    }
    pub fn verify_chart_normal(&self, normal: V3, e: &Entity) -> Result<(), Error> {
        if self.dot_normal(normal.map(Interval::point)).lo <= 0. {
            return Err(fail(e, "planar spline chart orientation not certified"));
        }
        Ok(())
    }
    pub fn verify_point_plane(&self, point: V3, tolerance: f64, e: &Entity) -> Result<(), Error> {
        let delta = std::array::from_fn(|i| {
            Interval::point(point[i]) - Interval::point(self.controls[0][i])
        });
        self.verify_distance(self.dot_normal(delta) / self.normal_length, tolerance, e)
    }
    fn verify_distance(&self, distance: Interval, tolerance: f64, e: &Entity) -> Result<(), Error> {
        if !distance.finite() || distance.lo < -tolerance || distance.hi > tolerance {
            return Err(fail(
                e,
                "spline edge/actual control-plane locus agreement not certified",
            ));
        }
        Ok(())
    }
    pub fn verify_circle_plane(
        &self,
        center: V3,
        basis: [V3; 2],
        radius: f64,
        tolerance: f64,
        e: &Entity,
    ) -> Result<(), Error> {
        let delta = std::array::from_fn(|i| {
            Interval::point(center[i]) - Interval::point(self.controls[0][i])
        });
        let center = self.dot_normal(delta);
        let coefficients = basis.map(|v| self.dot_normal(v.map(Interval::point)));
        let amplitude = Interval::point(radius)
            * (coefficients[0].square() + coefficients[1].square()).sqrt_nonnegative();
        self.verify_distance(
            (center + Interval::new(-amplitude.hi, amplitude.hi)) / self.normal_length,
            tolerance,
            e,
        )
    }
    pub fn controls(&self) -> [V3; 4] {
        self.controls
    }
    pub fn contains_point(&self, p: V3, e: &Entity) -> Result<(), Error> {
        let p = [p[self.axes[0]], p[self.axes[1]]];
        for i in 0..4 {
            if orient(self.polygon[i], self.polygon[(i + 1) % 4], p, e)? == Ordering::Less {
                return Err(fail(
                    e,
                    "edge trim lies outside the finite spline patch domain",
                ));
            }
        }
        Ok(())
    }
    /// Conservative whole-circle containment. A trimmed arc whose full circle
    /// leaves the patch refuses until a bounded arc-domain certificate exists.
    pub fn contains_circle(
        &self,
        center: V3,
        basis: [V3; 2],
        radius: f64,
        e: &Entity,
    ) -> Result<(), Error> {
        let [x, y] = self.axes;
        for i in 0..4 {
            let a = self.polygon[i].map(Interval::point);
            let b = self.polygon[(i + 1) % 4].map(Interval::point);
            let dx = b[0] - a[0];
            let dy = b[1] - a[1];
            let side =
                dx * (Interval::point(center[y]) - a[1]) - dy * (Interval::point(center[x]) - a[0]);
            let coefficients =
                basis.map(|v| dx * Interval::point(v[y]) - dy * Interval::point(v[x]));
            let squared = coefficients[0] * coefficients[0] + coefficients[1] * coefficients[1];
            let amplitude =
                Interval::point(radius) * Interval::new(0., squared.hi.max(0.).sqrt().next_up());
            if !side.finite() || !amplitude.finite() || side.lo < amplitude.hi {
                return Err(fail(e,"whole-circle containment in finite spline patch not certified; arc-domain trimming remains unsupported"));
            }
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::nurbs_surface::Limits;
    fn patch(points: Vec<V3>) -> NurbsSurface {
        NurbsSurface::new_clamped(
            [1, 1],
            [vec![0., 0., 1., 1.], vec![0., 0., 1., 1.]],
            [2, 2],
            points,
            vec![1., 3., 7., 11.],
            Limits::default(),
        )
        .unwrap()
    }
    fn entity() -> Entity {
        Entity {
            id: 1,
            offset: 17,
            components: vec![],
        }
    }
    #[test]
    fn exact_convex_planar_patches_keep_boundary_and_refuse_outside_controls_or_implicit_healing() {
        let e = entity();
        let s = patch(vec![
            [-2., -2., 0.],
            [2., -2., 0.],
            [-2., 2., 0.],
            [2., 2., 0.],
        ]);
        let p = PlanarPatch::new(&s, &e).unwrap();
        p.verify_chart_normal([0., 0., 1.], &e).unwrap();
        assert!(p.verify_chart_normal([0., 0., -1.], &e).is_err());
        p.verify_point_plane([0.; 3], 1e-7, &e).unwrap();
        assert!(p.verify_point_plane([0., 0., 1e-3], 1e-7, &e).is_err());
        p.verify_circle_plane([0.; 3], [[1., 0., 0.], [0., 1., 0.]], 1., 1e-7, &e)
            .unwrap();
        assert!(p
            .verify_circle_plane([0.; 3], [[1., 0., 0.], [0., 0., 1.]], 1., 1e-7, &e)
            .is_err());

        for point in [[0., 0., 0.], [-2., 0., 0.], [2., 2., 0.]] {
            p.contains_point(point, &e).unwrap();
        }
        assert!(p.contains_point([2.01, 0., 0.], &e).is_err());
        p.contains_circle([0.; 3], [[1., 0., 0.], [0., 1., 0.]], 1., &e)
            .unwrap();
        assert!(p
            .contains_circle([0.; 3], [[1., 0., 0.], [0., 1., 0.]], 3., &e)
            .is_err());
        for points in [
            vec![
                [-2., -2., 0.],
                [2., -2., 0.],
                [-2., 2., 0.],
                [2., 2., 1e-10],
            ],
            vec![[-2., -2., 0.], [2., 2., 0.], [-2., 2., 0.], [2., -2., 0.]],
            vec![[0.; 3]; 4],
        ] {
            assert!(PlanarPatch::new(&patch(points), &e).is_err());
        }
    }
    #[test]
    fn tilted_and_reversed_convex_nets_keep_exact_predicate_orientation_and_regular_parameter_maps()
    {
        let e = entity();
        let points = vec![[0., 0., 0.], [2., 0., -2.], [0., 3., -3.], [2., 3., -5.]];
        let surface = patch(points.clone());
        let p = PlanarPatch::new(&surface, &e).unwrap();
        p.contains_point([1., 1., -2.], &e).unwrap();
        assert!(p.contains_point([3., 1., -4.], &e).is_err());
        for u in [0.1, 0.5, 0.9] {
            for v in [0.1, 0.5, 0.9] {
                let d = surface.differential([u, v]).unwrap();
                let normal = crate::cross(d.du, d.dv);
                assert!(normal.iter().all(|x| *x > 0.));
            }
        }
        let reversed = patch(vec![points[0], points[2], points[1], points[3]]);
        PlanarPatch::new(&reversed, &e)
            .unwrap()
            .contains_point([1., 1., -2.], &e)
            .unwrap();
    }
}
