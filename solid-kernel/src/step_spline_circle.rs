//! Complete-span deviation of piecewise rational quadratic planar p-curves
//! from source circles, with optional closed monotone angular coverage proof.
//! Optional single-winding proof; no finite arc, trim/body or complete master certificate.
use crate::{
    analytic_step::{call, scalar},
    native_interval::Interval as I,
    step::{Entity, EntityReference, Error, ErrorKind, Inventory, Value},
    step_body::BodyContext,
    step_frame::{dot, SourceFrame, V},
    step_pcurve::{self, CurveMaster, ParametricCurve},
    step_plane_map::PlaneParameterMap,
};
#[derive(Debug, Clone, Copy)]
pub struct Options {
    pub tolerance_mm: f64,
    pub max_spans: usize,
    /// Also require exact closure and a consistently nonzero angular derivative.
    /// This proves full circle coverage, not a single winding or simple trim.
    pub require_circle_coverage: bool,
    /// Require coverage plus <=4 half-plane-contained spans, proving one turn.
    /// A longer supported spline can still have coverage without this proof.
    pub require_single_winding: bool,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            tolerance_mm: 1e-7,
            max_spans: 1024,
            require_circle_coverage: false,
            require_single_winding: false,
        }
    }
}
#[derive(Debug, Clone, Copy)]
pub struct Progress {
    pub spans: usize,
    pub total_spans: usize,
}
#[derive(Debug)]
pub struct SplineCircleDeviation {
    pub face: EntityReference,
    pub edge: EntityReference,
    pub pcurve: EntityReference,
    pub declared_master: CurveMaster,
    pub master_reference: EntityReference,
    pub spans: usize,
    pub curve_to_circle_upper_mm: f64,
    /// When true, the same error bound is symmetric (whole-circle Hausdorff).
    pub full_circle_coverage: bool,
    pub single_winding: bool,
    /// Sign relative to the source circle frame, when coverage was required.
    /// No winding count, finite arc selection or full master proof is implied.
    pub angular_orientation: Option<i8>,
}
fn fail(e: &Entity, kind: ErrorKind, message: &str) -> Error {
    Error {
        offset: e.offset,
        kind,
        detail: message.into(),
    }
}
fn absolute_upper(x: I) -> f64 {
    x.lo.abs().max(x.hi.abs())
}
impl PlaneParameterMap {
    /// Bound every point of the complete active spline domain against the
    /// associated source circle. Degree two and Bezier-separated active spans
    /// are required. Optional exact closure and strict consistent angular speed
    /// prove full circle coverage: the positive angular change of a closed,
    /// nonzero projected curve is an integer multiple of a full turn. Every
    /// circle angle then has a curve point within the same deviation bound.
    /// Coverage alone permits multiple windings. The optional <=4-span
    /// half-plane proof bounds the total change below two turns, establishing
    /// one winding. Finite trims and full master semantics remain open.
    pub fn verify_spline_circle_deviation(
        &self,
        context: &BodyContext,
        data: &Inventory,
        edge: EntityReference,
        pcurve: EntityReference,
        options: Options,
        mut progress: impl FnMut(Progress) -> bool,
    ) -> Result<SplineCircleDeviation, Error> {
        let entity = data.resolve_reference(pcurve)?;
        if !options.tolerance_mm.is_finite() || options.tolerance_mm <= 0. {
            return Err(fail(
                entity,
                ErrorKind::Geometry,
                "spline-circle tolerance must be finite and positive",
            ));
        }
        if options.require_single_winding && !options.require_circle_coverage {
            return Err(fail(
                entity,
                ErrorKind::Geometry,
                "single winding requires full circle coverage",
            ));
        }
        if !progress(Progress {
            spans: 0,
            total_spans: 0,
        }) {
            return Err(fail(
                entity,
                ErrorKind::Cancelled,
                "spline-circle verification cancelled",
            ));
        }
        let (topology, declaration, wrapper) = step_pcurve::associated_edge_pcurve(
            context,
            data,
            self.face_reference(),
            self.surface_reference(),
            edge,
            pcurve,
        )?;
        let ParametricCurve::Spline(spline) = declaration.geometry else {
            return Err(fail(
                entity,
                ErrorKind::Geometry,
                "requires planar spline p-curve",
            ));
        };
        let curve = spline.curve();
        if curve.degree() != 2 {
            return Err(fail(
                entity,
                ErrorKind::Geometry,
                "circle deviation supports rational quadratic spans only",
            ));
        }
        if options.require_circle_coverage
            && curve.control_points().first() != curve.control_points().last()
        {
            return Err(fail(
                entity,
                ErrorKind::Geometry,
                "circle coverage requires exactly closed endpoint controls",
            ));
        }
        let knots = curve.knots();
        let mut spans = Vec::new();
        for i in 2..curve.control_points().len() {
            if knots[i] == knots[i + 1] {
                continue;
            }
            if spans.len() >= options.max_spans {
                return Err(fail(
                    entity,
                    ErrorKind::Limit,
                    "spline-circle span budget exceeded",
                ));
            }
            if knots[i - 1] != knots[i] || knots[i + 1] != knots[i + 2] {
                return Err(fail(
                    entity,
                    ErrorKind::Geometry,
                    "active quadratic span is not Bezier-separated",
                ));
            }
            spans.push(i);
        }
        let geometry = crate::analytic_step::curve_geometry(data, topology.curve)?;
        let (placement, radius) = match call(geometry, "CIRCLE")? {
            [Value::String(_), Value::Ref(frame), r] => {
                (&data.entities()[frame], scalar(geometry, r)?)
            }
            _ => {
                return Err(fail(
                    geometry,
                    ErrorKind::Geometry,
                    "requires associated source 3D CIRCLE",
                ))
            }
        };
        let scale = context.millimetres_per_source_unit();
        let radius = I::point(radius) * I::point(scale);
        if !radius.finite() || radius.lo <= 0. || radius.hi > 1e30 {
            return Err(fail(
                geometry,
                ErrorKind::Geometry,
                "source circle radius is not positively bounded",
            ));
        }
        let frame = SourceFrame::read(data, placement, scale)?;
        let mut upper: f64 = 0.;
        let mut angular_orientation = None;
        let total = spans.len();
        if options.require_single_winding && total > 4 {
            return Err(fail(
                entity,
                ErrorKind::Geometry,
                "single-winding certificate supports at most four Bezier spans",
            ));
        }
        if !progress(Progress {
            spans: 0,
            total_spans: total,
        }) {
            return Err(fail(
                entity,
                ErrorKind::Cancelled,
                "spline-circle verification cancelled",
            ));
        }
        for (index, span) in spans.into_iter().enumerate() {
            let mut x = [I::point(0.); 3];
            let mut y = x;
            let mut z = x;
            let mut w = x;
            for j in 0..3 {
                let i = span - 2 + j;
                let p = curve.control_points()[i];
                let b = self.point_bounds([p[0], p[1]])?;
                let delta: V = std::array::from_fn(|axis| {
                    I::new(b.min[axis], b.max[axis]) - frame.origin[axis]
                });
                w[j] = I::point(curve.weights()[i]);
                x[j] = dot(delta, frame.x) * w[j];
                y[j] = dot(delta, frame.y) * w[j];
                z[j] = dot(delta, frame.z) * w[j];
            }
            // Product of two Bernstein degree-2 polynomials has coefficient
            // C(2,i) C(2,j) / C(4,i+j). Bound the implicit homogeneous circle
            // residual X^2+Y^2-R^2 W^2 by its degree-4 coefficient hull.
            let choose2 = [1., 2., 1.];
            let choose4 = [1., 4., 6., 4., 1.];
            let mut residual: f64 = 0.;
            for k in 0..5 {
                let mut coefficient = I::point(0.);
                for i in 0..3 {
                    if k < i || k - i >= 3 {
                        continue;
                    }
                    let j = k - i;
                    let factor = I::point(choose2[i] * choose2[j]) / I::point(choose4[k]);
                    coefficient = coefficient
                        + factor * (x[i] * x[j] + y[i] * y[j] - radius.square() * w[i] * w[j]);
                }
                if !coefficient.finite() {
                    return Err(fail(
                        entity,
                        ErrorKind::Geometry,
                        "circle residual not bounded",
                    ));
                }
                residual = residual.max(absolute_upper(coefficient));
            }
            let minimum = curve.weights()[span - 2..=span]
                .iter()
                .copied()
                .fold(f64::INFINITY, f64::min);
            let minimum = I::point(minimum);
            let denominator = minimum.square() * radius;
            if !denominator.finite() || denominator.lo <= 0. {
                return Err(fail(
                    entity,
                    ErrorKind::Geometry,
                    "circle residual denominator not positively bounded",
                ));
            }
            // |rho-R| = |rho^2-R^2|/(rho+R), with rho+R >= R.
            let radial = I::point(residual) / denominator;
            let planar = I::point(z.into_iter().map(absolute_upper).fold(0., f64::max)) / minimum;
            let error = (radial.square() + planar.square()).sqrt_nonnegative();
            if !error.finite() || error.hi > options.tolerance_mm {
                return Err(fail(
                    entity,
                    ErrorKind::Geometry,
                    "whole spline-to-circle deviation not certified",
                ));
            }
            if options.require_circle_coverage {
                if error.hi >= radius.lo {
                    return Err(fail(
                        entity,
                        ErrorKind::Geometry,
                        "circle coverage requires nonzero projected radius",
                    ));
                }
                if options.require_single_winding {
                    // An open half-plane bounds the angular change of this
                    // monotone span strictly below pi. At most four such
                    // spans give a total strictly below 4pi; closed, nonzero
                    // monotone coverage then has exactly one full turn.
                    let bx = x[0] / w[0] + x[2] / w[2];
                    let by = y[0] / w[0] + y[2] / w[2];
                    for j in 0..3 {
                        let side = x[j] * bx + y[j] * by;
                        if !side.finite() || side.lo <= 0. {
                            return Err(fail(
                                entity,
                                ErrorKind::Geometry,
                                "single-winding span half-plane not certified",
                            ));
                        }
                    }
                }
                // X Y' - Y X' has degree 3 in Bernstein form. Rational W
                // cancels from angular speed; local-to-knot scaling is positive.
                let dx = [(x[1] - x[0]) * I::point(2.), (x[2] - x[1]) * I::point(2.)];
                let dy = [(y[1] - y[0]) * I::point(2.), (y[2] - y[1]) * I::point(2.)];
                let choose3 = [1., 3., 3., 1.];
                let mut positive = true;
                let mut negative = true;
                for k in 0..4 {
                    let mut coefficient = I::point(0.);
                    for i in 0..3 {
                        if k < i || k - i >= 2 {
                            continue;
                        }
                        let j = k - i;
                        coefficient = coefficient
                            + (I::point(choose2[i]) / I::point(choose3[k]))
                                * (x[i] * dy[j] - y[i] * dx[j]);
                    }
                    positive &= coefficient.finite() && coefficient.lo > 0.;
                    negative &= coefficient.finite() && coefficient.hi < 0.;
                }
                let sign = if positive {
                    1
                } else if negative {
                    -1
                } else {
                    return Err(fail(
                        entity,
                        ErrorKind::Geometry,
                        "circle angular monotonicity not certified",
                    ));
                };
                if angular_orientation.is_some_and(|s| s != sign) {
                    return Err(fail(
                        entity,
                        ErrorKind::Geometry,
                        "spline angular direction changes across spans",
                    ));
                }
                angular_orientation = Some(sign);
            }
            upper = upper.max(error.hi);
            if !progress(Progress {
                spans: index + 1,
                total_spans: total,
            }) {
                return Err(fail(
                    entity,
                    ErrorKind::Cancelled,
                    "spline-circle verification cancelled",
                ));
            }
        }
        let master_reference = match wrapper.declared_master {
            CurveMaster::Curve3d => wrapper.geometry_3d,
            CurveMaster::Pcurve1 => wrapper.pcurves[0],
            CurveMaster::Pcurve2 => wrapper.pcurves[1],
        };
        Ok(SplineCircleDeviation {
            face: self.face_reference(),
            edge,
            pcurve,
            declared_master: wrapper.declared_master,
            master_reference,
            spans: total,
            curve_to_circle_upper_mm: upper,
            full_circle_coverage: options.require_circle_coverage,
            single_winding: options.require_single_winding,
            angular_orientation,
        })
    }
}
