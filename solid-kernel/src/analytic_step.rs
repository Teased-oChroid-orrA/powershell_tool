//! Native analytic geometry feasibility checks, not trimmed B-Rep/solid validity.
//! Supported subset: one directly represented, unplaced body; line/circle edges
//! and verified full-domain two-control degree-one splines;
//! plane/cylinder/sphere/cone/torus surfaces (the last three charted in `curved_surface`) and exactly coplanar convex
//! bilinear spline patches.
//! Trim containment uses 3D loci; associated p-curves/master-representation
//! agreement remain unverified. All coordinates normalize to millimetres.
use crate::{
    step::{Entity, Error, ErrorKind, Inventory, RawBody, Value},
    V3,
};
use std::collections::{BTreeMap, BTreeSet};
use std::f64::consts::{FRAC_PI_2, TAU};
fn add(a: V3, b: V3) -> V3 {
    std::array::from_fn(|i| a[i] + b[i])
}
fn sub(a: V3, b: V3) -> V3 {
    std::array::from_fn(|i| a[i] - b[i])
}
fn mul(a: V3, s: f64) -> V3 {
    a.map(|v| v * s)
}
fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: V3, b: V3) -> V3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn norm(a: V3) -> f64 {
    a[0].hypot(a[1]).hypot(a[2])
}
fn finite(p: V3) -> bool {
    p.iter().all(|x| x.is_finite())
}
fn coordinate_domain(p: V3) -> bool {
    finite(p)
        && p.iter()
            .all(|x| *x == 0. || (1e-30..=1e30).contains(&x.abs()))
}
fn error(e: &Entity, s: impl Into<String>) -> Error {
    Error {
        offset: e.offset,
        kind: ErrorKind::Geometry,
        detail: s.into(),
    }
}
pub(crate) fn scalar(e: &Entity, v: &Value) -> Result<f64, Error> {
    match v {
        Value::Real(x) if x.is_finite() => Ok(*x),
        Value::Integer(x) if x.unsigned_abs() <= 1u64 << 53 => Ok(*x as f64),
        _ => Err(error(e, "unsupported or non-exact geometric scalar")),
    }
}
pub(crate) fn call<'a>(e: &'a Entity, name: &str) -> Result<&'a [Value], Error> {
    match e.components.as_slice() {
        [c] if c.name == name => Ok(&c.args),
        _ => Err(error(e, format!("unsupported {name} declaration"))),
    }
}
fn unit(e: &Entity, v: V3) -> Result<V3, Error> {
    crate::native_math::normalize_direction(v).map_err(|detail| error(e, detail))
}
pub(crate) fn vector3(e: &Entity, v: &Value) -> Result<V3, Error> {
    match v {
        Value::List(v) if v.len() == 3 => {
            Ok([scalar(e, &v[0])?, scalar(e, &v[1])?, scalar(e, &v[2])?])
        }
        _ => Err(error(e, "expected three geometric components")),
    }
}
fn scale_coordinates(e: &Entity, source: V3, scale: f64) -> Result<V3, Error> {
    if !scale.is_finite() || scale <= 0. || !finite(source) {
        return Err(error(e, "invalid coordinate length normalization"));
    }
    let result = mul(source, scale);
    if source
        .iter()
        .zip(result)
        .any(|(before, after)| *before != 0. && after == 0.)
    {
        return Err(error(
            e,
            "coordinate length normalization underflows a nonzero source component",
        ));
    }
    // CAD exporters leave rounding noise around exact zeros (a cone's apex at 3.5e-32 mm, a meridian through a pole): a
    // magnitude below 1e-25 mm is 1e-19 of an atom and is read as an exact zero. Everything else outside the exact
    // predicates' envelope is still refused.
    let result = result.map(|x| if x.abs() < 1e-25 { 0. } else { x });
    if !coordinate_domain(result) {
        return Err(error(
            e,
            "coordinate is outside the supported arithmetic range",
        ));
    }
    Ok(result)
}
pub(crate) fn point(data: &Inventory, id: u64, scale: f64) -> Result<V3, Error> {
    let e = &data.entities()[&id];
    let source = match call(e, "CARTESIAN_POINT")? {
        [Value::String(_), v] => vector3(e, v)?,
        _ => return Err(error(e, "unsupported 3D point")),
    };
    scale_coordinates(e, source, scale)
}
fn direction(data: &Inventory, id: u64) -> Result<V3, Error> {
    let e = &data.entities()[&id];
    match call(e, "DIRECTION")? {
        [Value::String(_), v] => unit(e, vector3(e, v)?),
        _ => Err(error(e, "unsupported direction")),
    }
}
#[derive(Debug, Clone)]
pub struct Frame {
    pub(crate) origin: V3,
    pub(crate) x: V3,
    pub(crate) y: V3,
    pub(crate) z: V3,
}
impl Frame {
    pub(crate) fn point(&self, p: V3) -> V3 {
        add(
            self.origin,
            add(mul(self.x, p[0]), add(mul(self.y, p[1]), mul(self.z, p[2]))),
        )
    }
}
fn frame(data: &Inventory, id: u64, scale: f64) -> Result<Frame, Error> {
    let e = &data.entities()[&id];
    let (origin, z, x) = match call(e, "AXIS2_PLACEMENT_3D")? {
        [Value::String(_), Value::Ref(p), axis, reference] => {
            let z = match axis {
                Value::Omitted => [0., 0., 1.],
                Value::Ref(id) => direction(data, *id)?,
                _ => return Err(error(e, "unsupported axis")),
            };
            let x = match reference {
                Value::Omitted => [1., 0., 0.],
                Value::Ref(id) => direction(data, *id)?,
                _ => return Err(error(e, "unsupported reference direction")),
            };
            (point(data, *p, scale)?, z, x)
        }
        _ => return Err(error(e, "unsupported placement")),
    };
    let y = cross(z, x);
    if norm(y) < 1e-12 {
        return Err(error(e, "parallel or unresolved placement directions"));
    }
    let y = unit(e, y)?;
    let x = unit(e, cross(y, z))?;
    Ok(Frame { origin, x, y, z })
}
#[derive(Debug, Clone)]
pub enum Surface {
    Plane(Frame),
    Cylinder {
        frame: Frame,
        radius: f64,
    },
    /// `frame` is the *chart* frame of one face: origin at the centre, `z` toward the face's mean direction, which is the
    /// centre of the stereographic chart (see `curved_surface`). The poles of the source parameterization play no role.
    Sphere {
        frame: Frame,
        radius: f64,
    },
    /// Radius `radius` at the frame's origin plane, opening toward +z with half-angle `semi_angle` (radians). `centre` is the
    /// chart angle in the middle of the face, which resolves the branch of the polar chart (set per face).
    Cone {
        frame: Frame,
        radius: f64,
        semi_angle: f64,
        centre: f64,
    },
    Torus {
        frame: Frame,
        major: f64,
        minor: f64,
    },
    /// A parametric surface (a rational B-spline surface, or a B-spline curve revolved or extruded), charted by its parameter
    /// rectangle (see `curved_surface::SplineSurface`).
    Param(std::sync::Arc<crate::curved_surface::SplineSurface>),
}
#[derive(Debug, Clone, Copy)]
pub struct Projection {
    pub chart: [f64; 2],
    pub residual_mm: f64,
}
impl Surface {
    /// Untrimmed chart in physical millimetres. Cylinder u is circumferential
    /// arc length; its periodicity/trim selection is not resolved by projection.
    pub fn project(&self, p: V3) -> Option<Projection> {
        if !finite(p) {
            return None;
        }
        let (frame, radius) = match self {
            Self::Plane(f) => (f, None),
            Self::Cylinder { frame, radius } => (frame, Some(*radius)),
            _ => return self.project_curved(p),
        };
        let p = sub(p, frame.origin);
        let x = dot(p, frame.x);
        let y = dot(p, frame.y);
        let z = dot(p, frame.z);
        let result = if let Some(r) = radius {
            let rho = x.hypot(y);
            if rho == 0. {
                return None;
            }
            Projection {
                chart: [y.atan2(x) * r, z],
                residual_mm: (rho - r).abs(),
            }
        } else {
            Projection {
                chart: [x, y],
                residual_mm: z.abs(),
            }
        };
        if result.chart.iter().all(|x| x.is_finite()) && result.residual_mm.is_finite() {
            Some(result)
        } else {
            None
        }
    }
    pub fn point(&self, chart: [f64; 2]) -> Option<V3> {
        if !chart.iter().all(|x| x.is_finite()) {
            return None;
        }
        let p = match self {
            Self::Plane(f) => f.point([chart[0], chart[1], 0.]),
            Self::Cylinder { frame, radius } => {
                let u = chart[0] / radius;
                frame.point([radius * u.cos(), radius * u.sin(), chart[1]])
            }
            _ => return self.point_curved(chart),
        };
        finite(p).then_some(p)
    }
    /// Underlying geometric normal; this does not apply the declared face sense.
    pub fn normal(&self, chart: [f64; 2]) -> Option<V3> {
        if !chart.iter().all(|x| x.is_finite()) {
            return None;
        }
        let p = match self {
            Self::Plane(f) => f.z,
            Self::Cylinder { frame, radius } => {
                let u = chart[0] / radius;
                add(mul(frame.x, u.cos()), mul(frame.y, u.sin()))
            }
            Self::Param(_) => return self.spline_normal(chart),
            _ => return self.normal_at_world(self.point_curved(chart)?),
        };
        finite(p).then_some(p)
    }
}
#[derive(Debug, Clone)]
enum Curve {
    Line {
        origin: V3,
        direction: V3,
    },
    Circle {
        frame: Frame,
        radius: f64,
    },
    /// `P(t) = origin + major cos(t) x + minor sin(t) y`.
    Ellipse {
        frame: Frame,
        major: f64,
        minor: f64,
    },
    /// A B-spline (rational or not) edge curve covering its whole domain, with the parameter of every sample in edge order.
    Spline {
        curve: std::sync::Arc<crate::nurbs::NurbsCurve>,
        params: Vec<f64>,
    },
}
fn radius(e: &Entity, v: &Value, scale: f64) -> Result<f64, Error> {
    let r = scalar(e, v)? * scale;
    if (1e-30..=1e30).contains(&r) {
        Ok(r)
    } else {
        Err(error(e, "invalid or unsupported geometric radius"))
    }
}
/// Resolve only the bounded 3D curve wrapper chain. Associated p-curves are
/// not validated here; terminal geometry remains the caller's responsibility.
pub(crate) fn curve_geometry(data: &Inventory, mut id: u64) -> Result<&Entity, Error> {
    let mut seen = BTreeSet::new();
    loop {
        let e = &data.entities()[&id];
        if seen.len() >= 32 || !seen.insert(id) {
            return Err(error(e, "cyclic/deep curve wrapper"));
        }
        if let [c] = e.components.as_slice() {
            if let ("SURFACE_CURVE" | "SEAM_CURVE", [Value::String(_), Value::Ref(base), _, _]) =
                (c.name.as_str(), c.args.as_slice())
            {
                id = *base;
                continue;
            }
        }
        return Ok(e);
    }
}
fn curve(data: &Inventory, e: &Entity, scale: f64) -> Result<Curve, Error> {
    let [c] = e.components.as_slice() else {
        return Err(error(e, "unsupported complex 3D curve"));
    };
    match (c.name.as_str(), c.args.as_slice()) {
        ("LINE", [Value::String(_), Value::Ref(p), Value::Ref(v)]) => {
            let ve = &data.entities()[v];
            let direction = match call(ve, "VECTOR")? {
                [Value::String(_), Value::Ref(d), length] if scalar(ve, length)? > 0. => {
                    direction(data, *d)?
                }
                _ => return Err(error(ve, "unsupported line vector")),
            };
            Ok(Curve::Line {
                origin: point(data, *p, scale)?,
                direction,
            })
        }
        ("CIRCLE", [Value::String(_), Value::Ref(f), r]) => Ok(Curve::Circle {
            frame: frame(data, *f, scale)?,
            radius: radius(e, r, scale)?,
        }),
        ("ELLIPSE", [Value::String(_), Value::Ref(f), a, b]) => {
            let (major, minor) = (radius(e, a, scale)?, radius(e, b, scale)?);
            Ok(Curve::Ellipse {
                frame: frame(data, *f, scale)?,
                major,
                minor,
            })
        }
        _ => Err(error(e, "unsupported referenced 3D curve geometry")),
    }
}
/// Radians per plane-angle unit of the file: only the explicit SI radian is accepted (a degree-based file is refused rather
/// than guessed), and there must be exactly one plane-angle unit.
fn plane_angle_factor(data: &Inventory, e: &Entity) -> Result<f64, Error> {
    let mut found = None;
    for unit in data.entities().values() {
        if !unit.components.iter().any(|c| c.name == "PLANE_ANGLE_UNIT") {
            continue;
        }
        let radian = unit.components.iter().any(|c| {
            c.name == "SI_UNIT"
                && matches!(c.args.as_slice(), [Value::Omitted, Value::Enum(n)] if n == "RADIAN")
        });
        if found.replace(radian).is_some() {
            return Err(error(e, "ambiguous plane-angle units"));
        }
    }
    match found {
        Some(true) => Ok(1.),
        _ => Err(error(
            e,
            "a cone's semi-angle needs one explicit SI radian plane-angle unit",
        )),
    }
}
fn surface(data: &Inventory, id: u64, scale: f64) -> Result<Surface, Error> {
    let e = &data.entities()[&id];
    match e.components.as_slice() {
        [c] => match (c.name.as_str(), c.args.as_slice()) {
            ("PLANE", [Value::String(_), Value::Ref(f)]) => {
                Ok(Surface::Plane(frame(data, *f, scale)?))
            }
            ("CYLINDRICAL_SURFACE", [Value::String(_), Value::Ref(f), r]) => {
                Ok(Surface::Cylinder {
                    frame: frame(data, *f, scale)?,
                    radius: radius(e, r, scale)?,
                })
            }
            ("SPHERICAL_SURFACE", [Value::String(_), Value::Ref(f), r]) => Ok(Surface::Sphere {
                frame: frame(data, *f, scale)?,
                radius: radius(e, r, scale)?,
            }),
            ("CONICAL_SURFACE", [Value::String(_), Value::Ref(f), r, a]) => {
                let semi_angle = scalar(e, a)? * plane_angle_factor(data, e)?;
                if !(semi_angle > 1e-6 && semi_angle < FRAC_PI_2 - 1e-6) {
                    return Err(error(e, "cone semi-angle outside (0, 90) degrees"));
                }
                Ok(Surface::Cone {
                    frame: frame(data, *f, scale)?,
                    radius: radius(e, r, scale)?,
                    semi_angle,
                    centre: 0.,
                })
            }
            ("TOROIDAL_SURFACE", [Value::String(_), Value::Ref(f), major, minor]) => {
                let (major, minor) = (radius(e, major, scale)?, radius(e, minor, scale)?);
                if major <= minor {
                    return Err(error(
                        e,
                        "horn and spindle tori (major radius not above minor radius) are not implemented",
                    ));
                }
                Ok(Surface::Torus {
                    frame: frame(data, *f, scale)?,
                    major,
                    minor,
                })
            }
            _ => Err(error(e, "unsupported native analytic surface")),
        },
        _ => Err(error(e, "unsupported complex surface")),
    }
}
/// A `SURFACE_OF_REVOLUTION` or `SURFACE_OF_LINEAR_EXTRUSION` over a B-spline curve. An extrusion is unbounded along its
/// direction, so its height range is taken from `samples` (the face's boundary points) with a margin.
fn swept_surface(
    data: &Inventory,
    e: &Entity,
    scale: f64,
    samples: &[V3],
) -> Result<crate::curved_surface::SplineSurface, Error> {
    let [c] = e.components.as_slice() else {
        return Err(error(e, "unsupported complex surface"));
    };
    let [Value::String(_), Value::Ref(curve), Value::Ref(second)] = c.args.as_slice() else {
        return Err(error(e, "unsupported swept surface declaration"));
    };
    let curve = crate::step_nurbs::read_curve_active_domain(data, *curve, scale, crate::nurbs::Limits::default())
        .map_err(|mut err| {
            err.detail = format!("swept surface curve: {} (only B-spline curves are supported)", err.detail);
            err
        })?
        .curve;
    let second = &data.entities()[second];
    let surface = match c.name.as_str() {
        "SURFACE_OF_REVOLUTION" => match call(second, "AXIS1_PLACEMENT")? {
            [Value::String(_), Value::Ref(origin), Value::Ref(axis)] => {
                crate::curved_surface::SplineSurface::revolution(
                    curve,
                    point(data, *origin, scale)?,
                    direction(data, *axis)?,
                )
            }
            _ => return Err(error(second, "unsupported revolution axis")),
        },
        _ => match call(second, "VECTOR")? {
            [Value::String(_), Value::Ref(d), _] => {
                let d = direction(data, *d)?;
                let heights = samples.iter().map(|p| dot(*p, d));
                let lo = heights.clone().fold(f64::INFINITY, f64::min);
                let hi = heights.fold(f64::NEG_INFINITY, f64::max);
                let margin = 0.05 * (hi - lo);
                if !(margin.is_finite() && margin > 0.) {
                    return Err(error(e, "an extrusion face without an extent along its direction"));
                }
                crate::curved_surface::SplineSurface::extrusion(curve, d, [lo - margin, hi + margin])
            }
            _ => return Err(error(second, "unsupported extrusion direction")),
        },
    };
    surface.map_err(|mut err| {
        err.offset = e.offset;
        err
    })
}
#[derive(Debug, Clone, Copy)]
pub struct Options {
    pub chord_mm: f64,
    pub geometry_tolerance_mm: f64,
    pub max_edge_points: usize,
    pub max_total_points: usize,
    /// Opt-in: split every straight edge into equal pieces no longer than this (mm), so long edges do not force needle
    /// triangles on the faces they bound.
    pub max_line_mm: Option<f64>,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            chord_mm: 0.05,
            geometry_tolerance_mm: 1e-7,
            max_edge_points: 100_000,
            max_total_points: 1_000_000,
            max_line_mm: None,
        }
    }
}
#[derive(Debug, Clone)]
pub struct AnalyticModel {
    document: crate::step::DocumentFingerprint,
    body: RawBody,
    vertices: BTreeMap<u64, V3>,
    surfaces: BTreeMap<u64, Surface>,
    edges: BTreeMap<u64, Vec<V3>>,
    scale_mm: f64,
    chord_mm: f64,
    geometry_tolerance_mm: f64,
    curves: BTreeMap<u64, Curve>,
}
pub(crate) fn chord_angle(radius: f64, chord_mm: f64) -> f64 {
    (4. * (chord_mm / (2. * radius)).min(0.5).sqrt().asin()).min(FRAC_PI_2)
}
fn samples(
    e: &Entity,
    c: &Curve,
    a: V3,
    b: V3,
    closed: bool,
    forward: bool,
    opt: Options,
    remaining: usize,
) -> Result<Vec<V3>, Error> {
    let points = match c {
        Curve::Line { origin, direction } => {
            if a == b
                || dot(sub(b, a), *direction) == 0.
                || (dot(sub(b, a), *direction) > 0.) != forward
            {
                return Err(error(
                    e,
                    "unresolved line extent or inconsistent curve sense",
                ));
            }
            for p in [a, b] {
                let distance = norm(cross(sub(p, *origin), *direction));
                if !distance.is_finite() || distance > opt.geometry_tolerance_mm {
                    return Err(error(e, "edge endpoint is not on its declared line"));
                }
            }
            if remaining < 2 || opt.max_edge_points < 2 {
                return Err(Error {
                    offset: e.offset,
                    kind: ErrorKind::Limit,
                    detail: "edge sample budget exceeded".into(),
                });
            }
            let pieces = match opt.max_line_mm {
                Some(m) if m > 0. => (norm(sub(b, a)) / m).ceil().max(1.),
                _ => 1.,
            };
            if pieces > opt.max_edge_points.saturating_sub(1) as f64
                || pieces > remaining.saturating_sub(1) as f64
            {
                return Err(Error {
                    offset: e.offset,
                    kind: ErrorKind::Limit,
                    detail: "line sample budget exceeded".into(),
                });
            }
            let n = pieces as usize;
            let mut p: Vec<V3> = (0..=n)
                .map(|i| {
                    let t = i as f64 / n as f64;
                    [
                        a[0] + (b[0] - a[0]) * t,
                        a[1] + (b[1] - a[1]) * t,
                        a[2] + (b[2] - a[2]) * t,
                    ]
                })
                .collect();
            *p.first_mut().unwrap() = a;
            *p.last_mut().unwrap() = b;
            p
        }
        Curve::Circle { frame, radius } => {
            let mut angle = Vec::new();
            for p in [a, b] {
                let d = sub(p, frame.origin);
                let x = dot(d, frame.x);
                let y = dot(d, frame.y);
                let rho = x.hypot(y);
                if rho == 0.
                    || (rho - radius).abs().hypot(dot(d, frame.z)) > opt.geometry_tolerance_mm
                {
                    return Err(error(e, "edge endpoint is not on its declared circle"));
                }
                angle.push(y.atan2(x));
            }
            let span = if closed {
                if forward {
                    TAU
                } else {
                    -TAU
                }
            } else if forward {
                (angle[1] - angle[0]).rem_euclid(TAU)
            } else {
                -((angle[0] - angle[1]).rem_euclid(TAU))
            };
            if span == 0. {
                return Err(error(
                    e,
                    "distinct vertices have indistinguishable circle parameters",
                ));
            }
            let max_angle = chord_angle(*radius, opt.chord_mm);
            // Retain at least one interior sample on every curved edge. A short
            // arc and a straight edge can share both endpoint IDs; representing
            // both by one chord collapses distinct B-Rep edges into one mesh edge.
            let count = (span.abs() / max_angle).ceil().max(2.);
            if !count.is_finite()
                || count < 1.
                || count > opt.max_edge_points.saturating_sub(1) as f64
                || count > remaining.saturating_sub(1) as f64
            {
                return Err(Error {
                    offset: e.offset,
                    kind: ErrorKind::Limit,
                    detail: "circle sample budget/resolution exceeded".into(),
                });
            }
            let n = count as usize;
            let mut p = Vec::with_capacity(n + 1);
            for i in 0..=n {
                let t = angle[0] + span * (i as f64 / n as f64);
                // A coordinate below the exact predicates' envelope (1e-30) can only be rounding noise around an exact zero
                // (a meridian through a pole, a ring through the origin plane); coordinates above it are left bit-identical.
                p.push(
                    frame
                        .point([radius * t.cos(), radius * t.sin(), 0.])
                        .map(|x| if x.abs() < 1e-25 { 0. } else { x }),
                );
            }
            *p.first_mut().unwrap() = a;
            *p.last_mut().unwrap() = b;
            p
        }
        Curve::Ellipse {
            frame,
            major,
            minor,
        } => {
            // Parameter t is the eccentric anomaly: P(t) = a cos t x + b sin t y. The ends are located from their coordinates.
            let mut angle = Vec::new();
            for p in [a, b] {
                let d = sub(p, frame.origin);
                let (x, y, z) = (
                    dot(d, frame.x) / major,
                    dot(d, frame.y) / minor,
                    dot(d, frame.z),
                );
                if (x.hypot(y) - 1.).abs() * major.min(*minor)
                    > opt.geometry_tolerance_mm.max(1e-9) * 10.
                    || z.abs() > opt.geometry_tolerance_mm
                {
                    return Err(error(e, "edge endpoint is not on its declared ellipse"));
                }
                angle.push(y.atan2(x));
            }
            let span = if closed {
                if forward {
                    TAU
                } else {
                    -TAU
                }
            } else if forward {
                (angle[1] - angle[0]).rem_euclid(TAU)
            } else {
                -((angle[0] - angle[1]).rem_euclid(TAU))
            };
            if span == 0. {
                return Err(error(
                    e,
                    "distinct vertices have indistinguishable ellipse parameters",
                ));
            }
            let at = |t: f64| {
                frame
                    .point([major * t.cos(), minor * t.sin(), 0.])
                    .map(|x| if x.abs() < 1e-25 { 0. } else { x })
            };
            // Bisect every parameter interval whose midpoint is further from its chord than the chord tolerance.
            let start = ((span.abs() / (std::f64::consts::PI / 8.)).ceil().max(2.)) as usize;
            let mut ts: Vec<f64> = (0..=start)
                .map(|i| angle[0] + span * (i as f64 / start as f64))
                .collect();
            let mut pts: Vec<V3> = ts.iter().map(|t| at(*t)).collect();
            *pts.first_mut().unwrap() = a;
            *pts.last_mut().unwrap() = b;
            let limit = opt
                .max_edge_points
                .saturating_sub(1)
                .min(remaining.saturating_sub(1));
            loop {
                let mut next_t = Vec::with_capacity(ts.len());
                let mut next_p = Vec::with_capacity(pts.len());
                let mut split = false;
                for i in 0..ts.len() - 1 {
                    next_t.push(ts[i]);
                    next_p.push(pts[i]);
                    let tm = 0.5 * (ts[i] + ts[i + 1]);
                    let (mid, chord) = (at(tm), mul(add(pts[i], pts[i + 1]), 0.5));
                    if norm(sub(mid, chord)) > opt.chord_mm {
                        next_t.push(tm);
                        next_p.push(mid);
                        split = true;
                    }
                }
                next_t.push(*ts.last().unwrap());
                next_p.push(*pts.last().unwrap());
                if next_t.len() > limit + 1 {
                    return Err(Error {
                        offset: e.offset,
                        kind: ErrorKind::Limit,
                        detail: "ellipse sample budget/resolution exceeded".into(),
                    });
                }
                ts = next_t;
                pts = next_p;
                if !split {
                    break;
                }
            }
            pts
        }
        Curve::Spline { .. } => {
            return Err(error(e, "a spline edge is sampled by its own reader"));
        }
    };
    if points.iter().any(|p| !coordinate_domain(*p)) || points.windows(2).any(|p| p[0] == p[1]) {
        return Err(error(
            e,
            "sampled curve is unresolved in current coordinate precision",
        ));
    }
    Ok(points)
}
/// Sample a B-spline edge curve over its whole domain: every distinct knot is a sample, and a parameter interval is bisected
/// until the curve at its quarter points and midpoint lies within the chord tolerance of the chord. Unlike `nurbs_sample` it
/// certifies nothing (the interval bounds of that sampler need 40 000 points for a 10 mm cross-bore intersection curve at
/// 0.05 mm); the endpoints are checked against the vertices by the caller.
fn sample_spline_adaptive(
    curve: &crate::nurbs::NurbsCurve,
    chord_mm: f64,
    max_points: usize,
) -> Result<(Vec<f64>, Vec<V3>), String> {
    let domain = curve.domain();
    let mut breaks: Vec<f64> = curve
        .knots()
        .iter()
        .copied()
        .filter(|k| *k >= domain[0] && *k <= domain[1])
        .collect();
    breaks.dedup();
    if breaks.first().copied() != Some(domain[0]) {
        breaks.insert(0, domain[0]);
    }
    if breaks.last().copied() != Some(domain[1]) {
        breaks.push(domain[1]);
    }
    let eval = |t: f64| curve.evaluate(t).map_err(|e| e.detail);
    let mut params: Vec<f64> = Vec::new();
    for w in breaks.windows(2) {
        let n = 2 * curve.degree().max(1);
        for i in 0..n {
            params.push(w[0] + (w[1] - w[0]) * (i as f64 / n as f64));
        }
    }
    params.push(domain[1]);
    let mut points: Vec<V3> = params.iter().map(|t| eval(*t)).collect::<Result<_, _>>()?;
    loop {
        let mut next_t = Vec::with_capacity(params.len());
        let mut next_p = Vec::with_capacity(points.len());
        let mut split = false;
        for i in 0..params.len() - 1 {
            next_t.push(params[i]);
            next_p.push(points[i]);
            let (t0, t1) = (params[i], params[i + 1]);
            let mut worst = false;
            for f in [0.25, 0.5, 0.75] {
                let on_curve = eval(t0 + (t1 - t0) * f)?;
                let chord = add(mul(points[i], 1. - f), mul(points[i + 1], f));
                if norm(sub(on_curve, chord)) > chord_mm {
                    worst = true;
                    break;
                }
            }
            if worst {
                let tm = 0.5 * (t0 + t1);
                next_t.push(tm);
                next_p.push(eval(tm)?);
                split = true;
            }
        }
        next_t.push(*params.last().unwrap());
        next_p.push(*points.last().unwrap());
        if next_t.len() > max_points {
            return Err("spline edge sample budget exceeded".into());
        }
        params = next_t;
        points = next_p;
        if !split {
            break;
        }
    }
    Ok((params, points))
}
fn on_surface(e: &Entity, s: &Surface, c: &Curve, a: V3, b: V3, tol: f64) -> Result<(), Error> {
    for point in [a, b] {
        let projection = s
            .project(point)
            .ok_or_else(|| error(e, "unresolved edge endpoint/surface projection"))?;
        if projection.residual_mm > tol {
            return Err(error(
                e,
                format!(
                    "declared edge endpoint/surface geometry disagrees by {:e} mm",
                    projection.residual_mm
                ),
            ));
        }
    }
    // Topological vertices may be within tolerance of both entities without
    // the declared curve itself satisfying the surface tolerance. Evaluate its
    // actual finite locus between projected endpoint parameters as well.
    let (a, b) = if let Curve::Line { origin, direction } = c {
        let project = |p| add(*origin, mul(*direction, dot(sub(p, *origin), *direction)));
        let points = (project(a), project(b));
        if !finite(points.0) || !finite(points.1) {
            return Err(error(e, "nonfinite declared line locus"));
        }
        points
    } else {
        (a, b)
    };
    let deviation = match (s, c) {
        (Surface::Plane(f), Curve::Line { .. }) => dot(sub(a, f.origin), f.z)
            .abs()
            .max(dot(sub(b, f.origin), f.z).abs()),
        (Surface::Plane(f), Curve::Circle { frame, radius }) => {
            dot(sub(frame.origin, f.origin), f.z).abs()
                + radius * dot(frame.x, f.z).hypot(dot(frame.y, f.z))
        }
        (
            Surface::Cylinder { frame, radius },
            Curve::Circle {
                frame: c,
                radius: r,
            },
        ) => {
            let d = sub(c.origin, frame.origin);
            let offset = norm(sub(d, mul(frame.z, dot(d, frame.z))));
            offset
                + (r - radius)
                    .abs()
                    .max((r * dot(c.z, frame.z).abs() - radius).abs())
        }
        (Surface::Cylinder { frame, radius }, Curve::Line { .. }) => {
            let radial = |p| {
                let d = sub(p, frame.origin);
                sub(d, mul(frame.z, dot(d, frame.z)))
            };
            let x = radial(a);
            let y = radial(b);
            let step = sub(y, x);
            let length = norm(step);
            let min = if length > 0. {
                let d = mul(step, 1. / length);
                norm(add(x, mul(d, (-dot(x, d)).clamp(0., length))))
            } else {
                norm(x)
            };
            (norm(x) - radius)
                .abs()
                .max((norm(y) - radius).abs())
                .max((min - radius).abs())
        }
        // Sphere, cone and torus: the largest distance from the surface over the declared locus (the segment between the
        // endpoints, or the whole circle, which lies on these surfaces entirely or not at all).
        (surface, curve) => {
            let points: Vec<V3> = match curve {
                Curve::Line { .. } => (0..=16)
                    .map(|i| add(mul(a, 1. - i as f64 / 16.), mul(b, i as f64 / 16.)))
                    .collect(),
                Curve::Circle { frame, radius } => (0..64)
                    .map(|i| {
                        let t = TAU * i as f64 / 64.;
                        frame.point([radius * t.cos(), radius * t.sin(), 0.])
                    })
                    .collect(),
                Curve::Ellipse {
                    frame,
                    major,
                    minor,
                } => (0..128)
                    .map(|i| {
                        let t = TAU * i as f64 / 128.;
                        frame.point([major * t.cos(), minor * t.sin(), 0.])
                    })
                    .collect(),
                Curve::Spline { curve, params } => {
                    // the samples themselves and the curve at every parameter midpoint
                    let mut v = Vec::with_capacity(2 * params.len());
                    for w in params.windows(2) {
                        for t in [w[0], 0.5 * (w[0] + w[1])] {
                            if let Ok(p) = curve.evaluate(t) {
                                v.push(p);
                            }
                        }
                    }
                    if let Some(t) = params.last() {
                        if let Ok(p) = curve.evaluate(*t) {
                            v.push(p);
                        }
                    }
                    v
                }
            };
            let mut worst = 0f64;
            for p in points {
                worst = worst.max(surface.distance(p).unwrap_or(f64::INFINITY));
            }
            worst
        }
    };
    if !deviation.is_finite() || deviation > tol {
        Err(error(
            e,
            format!("declared edge/surface geometry disagrees by {deviation:e} mm"),
        ))
    } else {
        Ok(())
    }
}
impl AnalyticModel {
    /// Insert extra samples into one edge: `(segment, t)` puts a point at fraction `t` of segment `segment` (between samples
    /// `segment` and `segment + 1`), on the edge's own curve (a line is interpolated, a circle by angle).
    pub(crate) fn insert_edge_splits(
        &mut self,
        edge: u64,
        splits: &[(usize, f64)],
    ) -> Result<(), Error> {
        let fail = |s: &str| Error {
            offset: 0,
            kind: ErrorKind::Geometry,
            detail: s.into(),
        };
        let old = self.edges.get(&edge).ok_or_else(|| fail("unknown edge"))?;
        let curve = self
            .curves
            .get(&edge)
            .ok_or_else(|| fail("unknown edge curve"))?;
        let mut sorted = splits.to_vec();
        sorted.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)));
        sorted.dedup_by(|a, b| a.0 == b.0 && (a.1 - b.1).abs() < 1e-9);
        let mut out = Vec::with_capacity(old.len() + sorted.len());
        let spline_params = match curve {
            Curve::Spline { params, .. } => Some(params),
            _ => None,
        };
        let mut out_params: Vec<f64> = Vec::new();
        let mut k = 0;
        for (seg, w) in old.windows(2).enumerate() {
            out.push(w[0]);
            if let Some(params) = spline_params {
                out_params.push(params[seg]);
            }
            while k < sorted.len() && sorted[k].0 == seg {
                let t = sorted[k].1;
                k += 1;
                if !(t > 0. && t < 1.) {
                    continue;
                }
                let mut new_param = None;
                let p = match curve {
                    Curve::Line { .. } => add(mul(w[0], 1. - t), mul(w[1], t)),
                    Curve::Circle { frame, radius } => {
                        let angle = |p: V3| {
                            let d = sub(p, frame.origin);
                            dot(d, frame.y).atan2(dot(d, frame.x))
                        };
                        let (a0, a1) = (angle(w[0]), angle(w[1]));
                        let mut delta = a1 - a0;
                        if delta > std::f64::consts::PI {
                            delta -= TAU;
                        } else if delta < -std::f64::consts::PI {
                            delta += TAU;
                        }
                        let a = a0 + delta * t;
                        frame.point([radius * a.cos(), radius * a.sin(), 0.])
                    }
                    Curve::Ellipse {
                        frame,
                        major,
                        minor,
                    } => {
                        let angle = |p: V3| {
                            let d = sub(p, frame.origin);
                            (dot(d, frame.y) / minor).atan2(dot(d, frame.x) / major)
                        };
                        let (a0, a1) = (angle(w[0]), angle(w[1]));
                        let mut delta = a1 - a0;
                        if delta > std::f64::consts::PI {
                            delta -= TAU;
                        } else if delta < -std::f64::consts::PI {
                            delta += TAU;
                        }
                        let a = a0 + delta * t;
                        frame.point([major * a.cos(), minor * a.sin(), 0.])
                    }
                    Curve::Spline { curve, params } => {
                        let u = params[seg] * (1. - t) + params[seg + 1] * t;
                        let Ok(p) = curve.evaluate(u) else { continue };
                        new_param = Some(u);
                        p
                    }
                };
                // Rounding noise around an exact zero would sit below the exact predicates' arithmetic envelope.
                let scale = w[0].iter().chain(&w[1]).fold(0f64, |m, x| m.max(x.abs()));
                let p = p.map(|x| if x.abs() < 1e-13 * scale { 0. } else { x });
                if !coordinate_domain(p) || p == *out.last().unwrap() || p == w[1] {
                    continue;
                }
                out.push(p);
                if let Some(u) = new_param {
                    out_params.push(u);
                }
            }
        }
        out.push(*old.last().unwrap());
        if let Some(params) = spline_params {
            out_params.push(*params.last().unwrap());
        }
        self.edges.insert(edge, out);
        if spline_params.is_some() {
            if let Some(Curve::Spline { params, .. }) = self.curves.get_mut(&edge) {
                *params = out_params;
            }
        }
        Ok(())
    }
    pub fn geometry_tolerance_mm(&self) -> f64 {
        self.geometry_tolerance_mm
    }
    // Fixed chart coordinates of supported curves on a cylinder. Applying
    // them is explicit canonicalization, never a modification of source points.
    pub(crate) fn cylindrical_edge_constraint(
        &self,
        face: u64,
        edge: u64,
    ) -> Option<[Option<f64>; 2]> {
        let surface = self.surface(face)?;
        let Surface::Cylinder { frame, .. } = surface else {
            return None;
        };
        match &self.curves[&edge] {
            Curve::Circle { frame: c, .. } => {
                Some([None, Some(dot(sub(c.origin, frame.origin), frame.z))])
            }
            Curve::Line { .. } => Some([
                Some(
                    surface
                        .project(self.vertices[&self.body.edges[&edge].start])?
                        .chart[0],
                ),
                None,
            ]),
            // an ellipse or a spline fixes no chart coordinate
            Curve::Ellipse { .. } | Curve::Spline { .. } => Some([None, None]),
        }
    }
    pub fn chord_tolerance_mm(&self) -> f64 {
        self.chord_mm
    }
    pub fn document_fingerprint(&self) -> crate::step::DocumentFingerprint {
        self.document
    }
    pub fn body(&self) -> &RawBody {
        &self.body
    }
    pub fn vertex(&self, id: u64) -> Option<V3> {
        self.vertices.get(&id).copied()
    }
    pub fn surface(&self, id: u64) -> Option<&Surface> {
        self.surfaces.get(&id)
    }
    /// Apply the source face's same_sense flag. This still does not certify a
    /// globally consistent/outward shell orientation or a trimmed face domain.
    pub fn declared_face_normal(&self, id: u64, chart: [f64; 2]) -> Option<V3> {
        let n = self.surfaces.get(&id)?.normal(chart)?;
        Some(if self.body.faces.get(&id)?.same_sense {
            n
        } else {
            mul(n, -1.)
        })
    }
    pub fn edge_points(&self, id: u64) -> Option<&[V3]> {
        self.edges.get(&id).map(Vec::as_slice)
    }
    pub fn millimetres_per_source_unit(&self) -> f64 {
        self.scale_mm
    }
    pub fn read(data: &Inventory, opt: Options) -> Result<Self, Error> {
        let fail = |s: &str| Error {
            offset: 0,
            kind: ErrorKind::Geometry,
            detail: s.into(),
        };
        if !opt.chord_mm.is_finite()
            || !opt.geometry_tolerance_mm.is_finite()
            || opt.chord_mm <= 0.
            || opt.geometry_tolerance_mm <= 0.
            || opt.geometry_tolerance_mm > opt.chord_mm
        {
            return Err(fail(
                "require finite 0 < geometry tolerance <= chord tolerance",
            ));
        }
        let context = crate::step_body::BodyContext::read(data)?;
        let body = context.body();
        let scale = context.millimetres_per_source_unit();
        let mut vertices = BTreeMap::new();
        for v in body.vertices.values() {
            let p = scale_coordinates(&data.entities()[&v.point], v.coordinates, scale)?;
            vertices.insert(v.id, p);
        }
        let mut curves = BTreeMap::new();
        let mut edges = BTreeMap::new();
        let mut total = 0usize;
        let mut spline_segments = BTreeMap::new();
        for edge in body.edges.values() {
            let e = &data.entities()[&edge.id];
            let geometry = curve_geometry(data, edge.curve)?;
            let mut precomputed: Option<Vec<V3>> = None;
            let c = if geometry
                .components
                .iter()
                .any(|c| c.name == "B_SPLINE_CURVE_WITH_KNOTS")
            {
                let reference = data.entity_reference(edge.id).unwrap();
                let declaration =
                    context.read_spline_curve_active_domain(data, reference, crate::nurbs::Limits::default())?;
                if declaration.curve.degree() != 1 || declaration.curve.control_points().len() != 2
                {
                    // A general B-spline edge over its whole domain: endpoints must be the vertices (in the edge's sense), the
                    // interior is sampled to the chord tolerance.
                    let curve_domain = declaration.curve.domain();
                    let (first, last) = (
                        declaration
                            .curve
                            .evaluate(curve_domain[0])
                            .map_err(|e| error(geometry, e.detail))?,
                        declaration
                            .curve
                            .evaluate(curve_domain[1])
                            .map_err(|e| error(geometry, e.detail))?,
                    );
                    let (start, end) = if edge.same_sense {
                        (first, last)
                    } else {
                        (last, first)
                    };
                    for (curve_end, vertex) in
                        [(start, vertices[&edge.start]), (end, vertices[&edge.end])]
                    {
                        if norm(sub(curve_end, vertex)) > opt.geometry_tolerance_mm {
                            return Err(error(
                                geometry,
                                "a spline edge must cover its whole curve: an end of the curve is not the vertex",
                            ));
                        }
                    }
                    let (mut params, mut pts) = sample_spline_adaptive(
                        &declaration.curve,
                        opt.chord_mm,
                        opt.max_edge_points
                            .min(opt.max_total_points.saturating_sub(total)),
                    )
                    .map_err(|detail| error(geometry, detail))?;
                    if !edge.same_sense {
                        params.reverse();
                        pts.reverse();
                    }
                    // the vertices are the authoritative end points
                    *pts.first_mut().unwrap() = vertices[&edge.start];
                    *pts.last_mut().unwrap() = vertices[&edge.end];
                    if pts.iter().any(|p| !coordinate_domain(*p))
                        || pts.windows(2).any(|w| w[0] == w[1])
                    {
                        return Err(error(
                            geometry,
                            "sampled spline edge is unresolved in current coordinate precision",
                        ));
                    }
                    precomputed = Some(pts);
                    Curve::Spline {
                        curve: std::sync::Arc::new(declaration.curve.clone()),
                        params,
                    }
                } else {
                    // Geometric reduction only: a positive-weight degree-one span
                    // traces its control segment. Source declarations/IDs stay intact.
                    // Full-domain endpoint agreement is mandatory; no inferred trims.
                    let accepted = crate::step_spline_edge::sample_declaration(
                        &context,
                        data,
                        reference,
                        crate::step_spline_edge::Options {
                            geometry_tolerance_mm: opt.geometry_tolerance_mm,
                            curve_limits: crate::nurbs::Limits::default(),
                            sampling: crate::nurbs_sample::Options {
                                tolerance: opt.chord_mm,
                                max_segments: opt
                                    .max_edge_points
                                    .min(opt.max_total_points.saturating_sub(total))
                                    .saturating_sub(1),
                                ..crate::nurbs_sample::Options::default()
                            },
                        },
                        declaration,
                        |_| true,
                    )?;
                    let points = accepted.declaration().curve.control_points();
                    spline_segments.insert(edge.id, [points[0], points[1]]);
                    Curve::Line {
                        origin: points[0],
                        direction: unit(geometry, sub(points[1], points[0]))?,
                    }
                }
            } else {
                curve(data, geometry, scale)?
            };

            let p = match precomputed {
                Some(p) => p,
                None => samples(
                    e,
                    &c,
                    vertices[&edge.start],
                    vertices[&edge.end],
                    edge.start == edge.end,
                    edge.same_sense,
                    opt,
                    opt.max_total_points.saturating_sub(total),
                )?,
            };
            total += p.len();
            edges.insert(edge.id, p);
            curves.insert(edge.id, c);
        }
        let mut surfaces = BTreeMap::new();
        for face in body.faces.values() {
            let source_surface = &data.entities()[&face.surface];
            let (s, patch) = if source_surface
                .components
                .iter()
                .any(|c| c.name == "B_SPLINE_SURFACE_WITH_KNOTS")
            {
                let declaration = context.read_spline_surface(
                    data,
                    data.entity_reference(face.id).unwrap(),
                    crate::nurbs_surface::Limits::default(),
                )?;
                match crate::planar_spline::PlanarPatch::new(&declaration.surface, source_surface) {
                    Ok(patch) => {
                        let p = patch.controls();
                        let x = unit(source_surface, sub(p[1], p[0]))?;
                        let z = unit(source_surface, cross(x, sub(p[2], p[0])))?;
                        let y = unit(source_surface, cross(z, x))?;
                        patch.verify_chart_normal(z, source_surface)?;
                        (
                            Surface::Plane(Frame {
                                origin: p[0],
                                x,
                                y,
                                z,
                            }),
                            Some(patch),
                        )
                    }
                    // a bilinear 2 x 2 patch is a planar-face candidate only: a warped one keeps its refusal
                    Err(e)
                        if declaration.surface.degrees() == [1, 1]
                            && declaration.surface.dimensions() == [2, 2] =>
                    {
                        return Err(e)
                    }
                    // any other net is a general (rational) B-spline surface
                    Err(_) => (
                        Surface::Param(std::sync::Arc::new(
                            crate::curved_surface::SplineSurface::new(declaration.surface)
                                .map_err(|mut e| {
                                    e.offset = source_surface.offset;
                                    e
                                })?,
                        )),
                        None,
                    ),
                }
            } else if source_surface.components.iter().any(|c| {
                matches!(c.name.as_str(), "SURFACE_OF_REVOLUTION" | "SURFACE_OF_LINEAR_EXTRUSION")
            }) {
                let samples: Vec<V3> = face
                    .bounds
                    .iter()
                    .flat_map(|b| &b.edges)
                    .flat_map(|u| edges[&u.edge].iter().copied())
                    .collect();
                let swept = swept_surface(data, source_surface, scale, &samples)?;
                (Surface::Param(std::sync::Arc::new(swept)), None)
            } else {
                let source = surface(data, face.surface, scale)?;
                // A sphere is charted around each face's own centre, from the face's boundary loops in traversal order.
                let source = if let Surface::Sphere { .. } = source {
                    let mut loops = Vec::new();
                    for bound in &face.bounds {
                        let mut points = Vec::new();
                        for usage in &bound.edges {
                            let samples = &edges[&usage.edge];
                            if usage.forward {
                                points.extend_from_slice(&samples[..samples.len() - 1]);
                            } else {
                                points.extend(samples[1..].iter().rev());
                            }
                        }
                        if !bound.forward {
                            points.reverse();
                        }
                        loops.push(points);
                    }
                    source
                        .localized_sphere(&loops, face.same_sense)
                        .map_err(|mut err| {
                            err.offset = data.entities()[&face.surface].offset;
                            err
                        })?
                } else {
                    source
                };
                (source, None)
            };

            for bound in &face.bounds {
                for u in &bound.edges {
                    let edge = &body.edges[&u.edge];
                    if let Some(patch) = &patch {
                        for vertex in [edge.start, edge.end] {
                            patch.verify_point_plane(
                                vertices[&vertex],
                                opt.geometry_tolerance_mm,
                                source_surface,
                            )?;
                            patch.contains_point(vertices[&vertex], source_surface)?;
                        }
                        match &curves[&edge.id] {
                            Curve::Line { origin, direction } => {
                                let points =
                                    spline_segments.get(&edge.id).copied().unwrap_or_else(|| {
                                        [edge.start, edge.end].map(|id| {
                                            add(
                                                *origin,
                                                mul(
                                                    *direction,
                                                    dot(sub(vertices[&id], *origin), *direction),
                                                ),
                                            )
                                        })
                                    });
                                for p in points {
                                    patch.verify_point_plane(
                                        p,
                                        opt.geometry_tolerance_mm,
                                        source_surface,
                                    )?;
                                    patch.contains_point(p, source_surface)?;
                                }
                            }
                            Curve::Circle { frame, radius } => {
                                patch.verify_circle_plane(
                                    frame.origin,
                                    [frame.x, frame.y],
                                    *radius,
                                    opt.geometry_tolerance_mm,
                                    source_surface,
                                )?;
                                patch.contains_circle(
                                    frame.origin,
                                    [frame.x, frame.y],
                                    *radius,
                                    source_surface,
                                )?;
                            }
                            Curve::Ellipse { .. } | Curve::Spline { .. } => {
                                return Err(error(
                                    source_surface,
                                    "an ellipse or spline edge on a spline-patch face is not supported",
                                ));
                            }
                        }
                    }

                    if let Surface::Param(_) = &s {
                        // An edge of a spline-surface face lies on the surface along its samples (an arc of a circle need not
                        // lie on it elsewhere).
                        for p in &edges[&edge.id] {
                            let d = s.distance(*p).unwrap_or(f64::INFINITY);
                            if !(d <= opt.geometry_tolerance_mm) {
                                return Err(error(
                                    &data.entities()[&face.id],
                                    format!("an edge sample is {d:e} mm off the face's spline surface (edge #{}, {} samples, face #{})", edge.id, edges[&edge.id].len(), face.id),
                                ));
                            }
                        }
                        continue;
                    }
                    if let Some(points) = spline_segments.get(&edge.id) {
                        // Validate the actual spline locus as well as authoritative
                        // source-vertex chords; endpoint agreement alone is not
                        // an edge/surface certificate.
                        on_surface(
                            &data.entities()[&face.id],
                            &s,
                            &curves[&edge.id],
                            points[0],
                            points[1],
                            opt.geometry_tolerance_mm,
                        )?;
                    }
                    on_surface(
                        &data.entities()[&face.id],
                        &s,
                        &curves[&edge.id],
                        vertices[&edge.start],
                        vertices[&edge.end],
                        opt.geometry_tolerance_mm,
                    )?;
                }
            }
            surfaces.insert(face.id, s);
        }
        let (body, scale) = context.into_parts();
        Ok(Self {
            document: data.document_fingerprint(),
            body,
            vertices,
            surfaces,
            edges,
            scale_mm: scale,
            chord_mm: opt.chord_mm,
            geometry_tolerance_mm: opt.geometry_tolerance_mm,
            curves,
        })
    }
}

#[cfg(test)]
mod conversion_tests {
    use super::*;
    #[test]
    fn coordinate_length_conversion_preserves_units_zeros_and_rejects_invalid_scales() {
        let e = Entity {
            id: 1,
            offset: 17,
            components: vec![],
        };
        assert_eq!(
            scale_coordinates(&e, [1., -2., 0.], 25.4).unwrap(),
            [25.4, -50.8, 0.]
        );
        assert_eq!(scale_coordinates(&e, [0.; 3], 1e-21).unwrap(), [0.; 3]);
        for scale in [0., -1., f64::NAN, f64::INFINITY] {
            assert!(scale_coordinates(&e, [0.; 3], scale).is_err());
        }
        assert!(scale_coordinates(&e, [1e30, 0., 0.], 1e30).is_err());
        let error = scale_coordinates(&e, [1e-310, 1e21, 0.], 1e-21).unwrap_err();
        assert_eq!(error.offset, 17);
        assert!(error.detail.contains("underflow"));
    }
    #[test]
    fn coordinate_scaling_underflow_is_not_silently_converted_to_zero() {
        let source = "ISO-10303-21; HEADER; FILE_DESCRIPTION(('unit-test'),'2;1'); FILE_NAME('fixture','',(''),(''),'','',''); FILE_SCHEMA(('TEST')); ENDSEC; DATA; #1=CARTESIAN_POINT('',(1.E-310,0.,0.)); ENDSEC; END-ISO-10303-21;";
        let data = crate::step::inventory(source, crate::step::Limits::default()).unwrap();
        assert_eq!(1e-310 * 1e-21, 0.);
        let error = point(&data, 1, 1e-21).unwrap_err();
        assert_eq!(error.kind, ErrorKind::Geometry);
        assert!(error.detail.contains("underflow"));
    }
    #[test]
    fn surface_locus_uses_declared_line_not_only_nearby_topological_vertices() {
        let e = Entity {
            id: 1,
            offset: 17,
            components: vec![],
        };
        let surface = Surface::Plane(Frame {
            origin: [0.; 3],
            x: [1., 0., 0.],
            y: [0., 1., 0.],
            z: [0., 0., 1.],
        });
        let line = Curve::Line {
            origin: [0., 0., 1.5e-7],
            direction: [1., 0., 0.],
        };
        let a = [0., 0., 0.75e-7];
        let b = [1., 0., 0.75e-7];
        let opt = Options::default();
        assert!(samples(&e, &line, a, b, false, true, opt, 10).is_ok());
        assert!(on_surface(&e, &surface, &line, a, b, opt.geometry_tolerance_mm).is_err());
        let near = Curve::Line {
            origin: [0., 0., 0.5e-7],
            direction: [1., 0., 0.],
        };
        let a = [0., 0., 1.25e-7];
        let b = [1., 0., 1.25e-7];
        assert!(samples(&e, &near, a, b, false, true, opt, 10).is_ok());
        assert!(on_surface(&e, &surface, &near, a, b, opt.geometry_tolerance_mm).is_err());

        let cylinder = Surface::Cylinder {
            frame: Frame {
                origin: [0.; 3],
                x: [1., 0., 0.],
                y: [0., 1., 0.],
                z: [0., 0., 1.],
            },
            radius: 1.,
        };
        let generator = Curve::Line {
            origin: [1. + 1.5e-7, 0., 0.],
            direction: [0., 0., 1.],
        };
        let a = [1. + 0.75e-7, 0., 0.];
        let b = [1. + 0.75e-7, 0., 1.];
        assert!(samples(&e, &generator, a, b, false, true, opt, 10).is_ok());
        assert!(on_surface(&e, &cylinder, &generator, a, b, opt.geometry_tolerance_mm).is_err());
    }
}
