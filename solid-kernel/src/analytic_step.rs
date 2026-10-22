//! Native analytic geometry feasibility checks, not trimmed B-Rep/solid validity.
//! Supported subset: one directly represented, unplaced body; line/circle edges
//! and verified full-domain two-control degree-one splines;
//! plane/cylinder/sphere/cone/torus surfaces (the last three charted in `curved_surface`) and exactly coplanar convex
//! bilinear spline patches.
//! Trim containment uses 3D loci; associated p-curves/master-representation
//! agreement remain unverified. All coordinates normalize to millimetres.
use crate::{
    step::{Entity, Error, ErrorKind, Inventory, RawBody, Value},
    step_topology::{EdgeUse, RawBound, RawEdge},
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
                // The surface is `C(t) + h d`: `h` is the height above the curve point, not the absolute position along `d`
                // (a curve in a plane away from the origin would otherwise put the whole range at the wrong place). A point
                // `p` of the face has `h = (p - C(t)) . d`, so the range is the samples' spread widened by the curve's own.
                let heights = samples.iter().map(|p| dot(*p, d));
                let [t0, t1] = curve.domain();
                let mut base = (f64::INFINITY, f64::NEG_INFINITY);
                for k in 0..=256 {
                    let t = if k == 256 { t1 } else { t0 + (t1 - t0) * k as f64 / 256. };
                    let h = dot(curve.evaluate(t).map_err(|mut err| {
                        err.offset = e.offset;
                        err
                    })?, d);
                    base = (base.0.min(h), base.1.max(h));
                }
                let lo = heights.clone().fold(f64::INFINITY, f64::min) - base.1;
                let hi = heights.fold(f64::NEG_INFINITY, f64::max) - base.0;
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
impl Options {
    /// Options for a file written by another CAD kernel (Onshape/Parasolid, SolidWorks, ...). Those kernels model to
    /// their own tolerance and declare none usable (the ABC files state an uncertainty of 1e-17), and a measured
    /// 1.8e-7 mm up to several 1e-2 mm of edge-to-surface disagreement occurs there
    /// (`docs/issue-46-abc-corpus-2026-10-18.md`): the default 1e-7 mm refuses 54 of 300 files for it. The edge-to-surface
    /// agreement is therefore required to a tenth of the sampling chord (5e-3 mm at the default 0.05 mm). The policy: a
    /// sampled shell already deviates from the true surface by up to the chord, and a boundary that disagrees with its
    /// surface by `e` adds at most `e` to that (a gap of `e` between the face interior and its boundary), so `e` at a tenth of
    /// the chord raises the worst geometric error by at most 10 %. Measured on the corpus (all other readers' limits aside):
    /// of 59 files refused for it at chord / 50, 11 need up to 2.5e-3 mm, 18 up to 5e-3 mm (this setting), 22 up to 5e-2 mm
    /// (a whole chord, not admitted), the rest are unrelated refusals. Nothing else is relaxed, and `Options::default()`
    /// stays strict.
    pub fn foreign_cad(chord_mm: f64) -> Self {
        Self {
            chord_mm,
            geometry_tolerance_mm: chord_mm / 10.,
            ..Self::default()
        }
    }
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
/// The two bounds of a face (by index) that each wind once around a periodic coordinate, in opposite senses, when every other
/// bound does not wind at all (a hole): the loops of a band that a seam joins. `turns` are the signed turns of each bound.
fn winding_pair(turns: &[(Vec<EdgeUse>, f64)]) -> Option<(usize, usize)> {
    let once = |t: f64| (t.abs() - 1.).abs() < 1e-6;
    let winding: Vec<usize> = (0..turns.len()).filter(|&k| turns[k].1.abs() > 1e-6).collect();
    let [a, b] = winding.as_slice() else {
        return None;
    };
    (once(turns[*a].1) && once(turns[*b].1) && turns[*a].1.signum() != turns[*b].1.signum()).then_some((*a, *b))
}
/// The seam joining the two loops of a torus band (`seam_unseamed_torus_bands`): the circle arc from `ends[0]` to `ends[1]`
/// and its samples. `ring`: the ends share an angle about the torus axis (a meridian arc of the tube); otherwise they share
/// an angle about the tube (an arc of the parallel). `tangents` are the directions the loops are traversed in at the two
/// ends. Of the two arcs between the ends the face is on the left of its loops (seen from its normal), so the seam is the
/// one that leaves the first loop towards that side and arrives at the second loop from it; `None` when the ends are not
/// on one meridian / parallel, coincide, or the two loops disagree about the side.
fn torus_seam(
    frame: &Frame,
    major: f64,
    minor: f64,
    ring: bool,
    same_sense: bool,
    ends: [V3; 2],
    tangents: [V3; 2],
    chord_mm: f64,
    tolerance: f64,
) -> Option<(Curve, Vec<V3>)> {
    let axis = frame.z;
    let about_axis = |p: V3| {
        let d = sub(p, frame.origin);
        dot(d, frame.y).atan2(dot(d, frame.x))
    };
    let radial = |u: f64| add(mul(frame.x, u.cos()), mul(frame.y, u.sin()));
    let wrap = |a: f64| a - TAU * (a / TAU).round();
    let unit = |v: V3| {
        let n = norm(v);
        (n > 0.).then(|| mul(v, 1. / n))
    };
    // per end: the outward normal of the torus, the direction the seam's angle increases in, and the face's left of the loop
    // (the loop's traversal `tangents[k]`, the face's normal `n`: left = n x t)
    let side = |k: usize, increasing: V3, outward: V3| {
        let n = if same_sense { outward } else { mul(outward, -1.) };
        // positive: the angle's increasing direction points to the face's left
        dot(cross(n, tangents[k]), increasing)
    };
    let (ua, ub) = (about_axis(ends[0]), about_axis(ends[1]));
    let centre: V3;
    let radius: f64;
    let e_a: V3;
    let theta: [f64; 2];
    let to_point: Box<dyn Fn(f64) -> V3>;
    let increasing: [V3; 2];
    let outward: [V3; 2];
    if ring {
        let rho = |p: V3| {
            let d = sub(p, frame.origin);
            dot(d, frame.x).hypot(dot(d, frame.y))
        };
        if rho(ends[0]).max(rho(ends[1])) * wrap(ua - ub).abs() > tolerance {
            return None;
        }
        e_a = radial(ua);
        centre = add(frame.origin, mul(e_a, major));
        radius = minor;
        let angle = |p: V3| {
            let d = sub(p, centre);
            dot(d, axis).atan2(dot(d, e_a))
        };
        theta = [angle(ends[0]), angle(ends[1])];
        let (c, a) = (centre, e_a);
        to_point = Box::new(move |t: f64| add(c, mul(add(mul(a, t.cos()), mul(axis, t.sin())), minor)));
        let at = |k: usize| {
            let (s, c) = theta[k].sin_cos();
            (add(mul(e_a, -s), mul(axis, c)), add(mul(e_a, c), mul(axis, s)))
        };
        increasing = [at(0).0, at(1).0];
        outward = [at(0).1, at(1).1];
    } else {
        let height = |p: V3| dot(sub(p, frame.origin), axis);
        let rho = |p: V3| {
            let d = sub(p, frame.origin);
            dot(d, frame.x).hypot(dot(d, frame.y))
        };
        if (height(ends[0]) - height(ends[1])).abs().hypot(rho(ends[0]) - rho(ends[1])) > tolerance {
            return None;
        }
        e_a = radial(ua);
        centre = add(frame.origin, mul(axis, height(ends[0])));
        radius = rho(ends[0]);
        theta = [ua, ub];
        let (c, x, y) = (centre, frame.x, frame.y);
        to_point = Box::new(move |t: f64| add(c, mul(add(mul(x, t.cos()), mul(y, t.sin())), radius)));
        let at = |k: usize| {
            let e = radial(theta[k]);
            let tube = add(frame.origin, mul(e, major));
            (cross(axis, e), unit(sub(ends[k], tube)).unwrap_or(e))
        };
        increasing = [at(0).0, at(1).0];
        outward = [at(0).1, at(1).1];
    }
    if !(radius > 0.) {
        return None;
    }
    // the sign of the angle's travel along the seam: towards the face's left at the first end, away from it at the second
    let (sa, sb) = (side(0, increasing[0], outward[0]), side(1, increasing[1], outward[1]));
    if sa == 0. || sb == 0. || sa.signum() == sb.signum() {
        return None;
    }
    let direction = sa.signum();
    let delta = wrap(theta[1] - theta[0]);
    if delta == 0. {
        return None;
    }
    let delta = if delta.signum() == direction { delta } else { delta - delta.signum() * TAU };
    let count = (delta.abs() / chord_angle(radius, chord_mm)).ceil().max(2.) as usize;
    let mut samples: Vec<V3> = (0..=count).map(|i| to_point(theta[0] + delta * i as f64 / count as f64)).collect();
    *samples.first_mut()? = ends[0];
    *samples.last_mut()? = ends[1];
    // circle frame: x = from the centre towards the start, y turns the way the angle runs
    let (x, y) = if ring {
        (e_a, mul(axis, direction))
    } else {
        (e_a, mul(cross(axis, e_a), direction))
    };
    let z = cross(x, y);
    Some((Curve::Circle { frame: Frame { origin: centre, x, y, z }, radius }, samples))
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
/// Share of the chord tolerance spent by the bisection; the decimation gets the rest.
const SPLIT_SHARE: f64 = 0.25;
/// No decimated segment is longer than this many chords: the face mesher needs boundary spacing comparable to its own.
const MAX_SEGMENT_CHORDS: f64 = 30.;
fn sample_spline_adaptive(
    curve: &crate::nurbs::NurbsCurve,
    chord_mm: f64,
    max_points: usize,
) -> Result<(Vec<f64>, Vec<V3>), String> {
    sample_spline_with(curve, chord_mm, max_points, SPLIT_SHARE)
}
fn sample_spline_with(
    curve: &crate::nurbs::NurbsCurve,
    chord_mm: f64,
    max_points: usize,
    split_share: f64,
) -> Result<(Vec<f64>, Vec<V3>), String> {
    sample_spline_range(curve, curve.domain(), chord_mm, max_points, split_share)
}
fn sample_spline_range(
    curve: &crate::nurbs::NurbsCurve,
    domain: [f64; 2],
    chord_mm: f64,
    max_points: usize,
    split_share: f64,
) -> Result<(Vec<f64>, Vec<V3>), String> {
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
                if norm(sub(on_curve, chord)) > split_share * chord_mm {
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
    // The seed of 2 x degree samples per knot span (to catch wiggles) is far denser than the chord needs on a short
    // span: drop every sample whose removal keeps the curve (the dense samples and the midpoints between them) within the
    // other half of the chord tolerance of the remaining polyline. Total deviation stays within `chord_mm`.
    let tol = (1. - split_share) * chord_mm;
    let within = |a: usize, b: usize| -> Result<bool, String> {
        let (pa, pb) = (points[a], points[b]);
        let ab = sub(pb, pa);
        let len2 = dot(ab, ab);
        if len2.sqrt() > MAX_SEGMENT_CHORDS * chord_mm {
            return Ok(false);
        }
        let dist = |q: V3| {
            let t = if len2 > 0. { (dot(sub(q, pa), ab) / len2).clamp(0., 1.) } else { 0. };
            norm(sub(q, add(pa, mul(ab, t))))
        };
        for k in a + 1..b {
            if dist(points[k]) > tol {
                return Ok(false);
            }
        }
        for k in a..b {
            if dist(eval(0.5 * (params[k] + params[k + 1]))?) > tol {
                return Ok(false);
            }
        }
        Ok(true)
    };
    let n = points.len();
    let mut keep = vec![0usize];
    let mut i = 0;
    while i < n - 1 {
        let mut j = i + 1;
        while j + 1 < n && within(i, j + 1)? {
            j += 1;
        }
        keep.push(j);
        i = j;
    }
    // a spline edge keeps an interior sample even when it is straight (it is not the line path)
    if keep.len() == 2 && n > 2 {
        keep.insert(1, n / 2);
    }
    Ok((keep.iter().map(|&k| params[k]).collect(), keep.iter().map(|&k| points[k]).collect()))
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
        Self::read_context(data, opt, crate::step_body::BodyContext::read(data)?)
    }
    /// One model per `MANIFOLD_SOLID_BREP` of a multi-body document (the single-body [`Self::read`] refuses those). The
    /// models are independent: whether the solids touch or overlap is not examined.
    pub fn read_all(data: &Inventory, opt: Options) -> Result<Vec<Self>, Error> {
        crate::step_body::BodyContext::read_all(data)?
            .into_iter()
            .map(|context| Self::read_context(data, opt, context))
            .collect()
    }
    fn read_context(
        data: &Inventory,
        opt: Options,
        context: crate::step_body::BodyContext,
    ) -> Result<Self, Error> {
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
                let declaration = context.read_spline_surface_active_domain(
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
        let mut model = Self {
            document: data.document_fingerprint(),
            body,
            vertices,
            surfaces,
            edges,
            scale_mm: scale,
            chord_mm: opt.chord_mm,
            geometry_tolerance_mm: opt.geometry_tolerance_mm,
            curves,
        };
        let first_synthetic_id = data.entities().keys().next_back().map_or(1, |k| k + 1);
        model.seam_unseamed_cylinder_bands(first_synthetic_id);
        Ok(model)
    }
    /// A cylinder or cone face with two closed loops that wind once around the axis in opposite senses and no seam edge
    /// (Onshape writes every through-hole wall and chamfer this way; OpenCASCADE adds a seam) is cut along a generator from
    /// the start vertex of the first loop to the second loop at the same angle. When the second loop has no vertex there,
    /// one is inserted on its edge (an exact point of its curve) and the edge is split in every face that uses it, which keeps
    /// the shell conforming. The cut is a new straight edge used once each way in a single merged loop, exactly the shape of
    /// an OpenCASCADE cylinder, so sampling, grading and connectivity treat it like any seam. Further bounds that do not wind
    /// (holes) stay as they are. Source edge and vertex identities are kept where they are not split; new ids follow the
    /// largest entity id of the document, have no entity of their own (`RawEdge::curve` of the seam is 0) and exist only in
    /// this model. A face of any other shape keeps its bounds and is refused later by the chart code as before.
    fn seam_unseamed_cylinder_bands(&mut self, first_synthetic_id: u64) {
        let mut next = first_synthetic_id;
        let tolerance = self.geometry_tolerance_mm;
        let faces: Vec<u64> = self
            .body
            .faces
            .values()
            .filter(|f| {
                f.bounds.len() >= 2
                    && matches!(
                        self.surfaces.get(&f.id),
                        Some(Surface::Cylinder { .. } | Surface::Cone { .. })
                    )
            })
            .map(|f| f.id)
            .collect();
        for face_id in faces {
            let frame = match self.surfaces.get(&face_id) {
                Some(Surface::Cylinder { frame, .. } | Surface::Cone { frame, .. }) => frame.clone(),
                _ => continue,
            };
            let angle = |p: V3| {
                let d = sub(p, frame.origin);
                dot(d, frame.y).atan2(dot(d, frame.x))
            };
            let wrap = |a: f64| a - TAU * (a / TAU).round();
            let rho = |p: V3| {
                let d = sub(p, frame.origin);
                (dot(d, frame.x).powi(2) + dot(d, frame.y).powi(2)).sqrt()
            };
            let Some(turns) = self.bound_turns(face_id, &angle, &wrap) else {
                continue;
            };
            let Some(pair) = winding_pair(&turns) else {
                continue;
            };
            let (la, lb, tb) = (turns[pair.0].0.clone(), turns[pair.1].0.clone(), turns[pair.1].1);
            let Some(va) = la.first().and_then(|u| self.use_start(u)) else {
                continue;
            };
            let target = angle(self.vertices[&va]);
            let Some(vb) = self.vertex_of_loop_at_angle(&lb, tb.signum(), target, &angle, &wrap, &mut next) else {
                continue;
            };
            // the second loop may have been rewritten by an edge split: read it again, rotated to start at the vertex
            let Some(turns) = self.bound_turns(face_id, &angle, &wrap) else {
                continue;
            };
            let (la, mut lb) = (turns[pair.0].0.clone(), turns[pair.1].0.clone());
            let Some(at) = lb.iter().position(|u| self.use_start(u) == Some(vb)) else {
                continue;
            };
            lb.rotate_left(at);
            let (pa, pb) = (self.vertices[&va], self.vertices[&vb]);
            let axial = dot(sub(pb, pa), frame.z);
            if rho(pa).max(rho(pb)) * wrap(angle(pa) - angle(pb)).abs() > tolerance || axial.abs() <= tolerance {
                continue;
            }
            let length = norm(sub(pb, pa));
            let curve = Curve::Line {
                origin: pa,
                direction: mul(sub(pb, pa), 1. / length),
            };
            self.install_band_seam(face_id, pair, la, lb, (va, vb), curve, vec![pa, pb], &mut next);
        }
        self.seam_unseamed_torus_bands(&mut next);
        self.seam_unseamed_revolution_bands(&mut next);
    }
    /// Join the loops `la` and `lb` of the bounds `pair` of a band face into one through a new seam edge from `ends.0` (the
    /// first vertex of `la`) to `ends.1` (the first vertex of `lb`), used once each way: the shape of an OpenCASCADE seam.
    /// `samples` are the seam's points from start to end. The other bounds of the face stay.
    fn install_band_seam(
        &mut self,
        face_id: u64,
        pair: (usize, usize),
        la: Vec<EdgeUse>,
        lb: Vec<EdgeUse>,
        ends: (u64, u64),
        curve: Curve,
        samples: Vec<V3>,
        next: &mut u64,
    ) {
        let seam = *next;
        *next += 3;
        self.body.edges.insert(
            seam,
            RawEdge {
                id: seam,
                start: ends.0,
                end: ends.1,
                curve: 0,
                same_sense: true,
            },
        );
        self.curves.insert(seam, curve);
        self.edges.insert(seam, samples);
        let first = self.body.faces[&face_id].bounds[pair.0].clone();
        let mut merged = la;
        merged.push(EdgeUse {
            id: seam + 1,
            edge: seam,
            forward: true,
        });
        merged.extend(lb);
        merged.push(EdgeUse {
            id: seam + 2,
            edge: seam,
            forward: false,
        });
        let face = self.body.faces.get_mut(&face_id).unwrap();
        let mut bounds = vec![RawBound {
            id: first.id,
            loop_id: first.loop_id,
            forward: true,
            explicitly_outer: false,
            edges: merged,
        }];
        bounds.extend(
            face.bounds
                .iter()
                .enumerate()
                .filter(|(k, _)| *k != pair.0 && *k != pair.1)
                .map(|(_, b)| b.clone()),
        );
        face.bounds = bounds;
    }
    /// A torus face (major radius above minor) with two closed loops and no seam, as Onshape writes a fillet ring or a pipe
    /// bend, is cut like a cylinder band, but along a circle arc: loops that each wind once around the torus axis are joined by
    /// a meridian arc (constant angle about the axis), loops that each wind once around the tube by an arc of the parallel at
    /// the first vertex (constant angle about the tube). Two such arcs join the same vertices, one each way round, and bound
    /// two different faces (the band and the rest of the ring); the face is on the left of its loops seen from its normal (the
    /// surface normal, reversed when the face senses against it), so the arc is the one that leaves the first loop towards its
    /// left and arrives at the second loop from its left. Where the two loops disagree on that, or a loop is not a plain
    /// winding, the face keeps its bounds and the chart code refuses it as before.
    fn seam_unseamed_torus_bands(&mut self, next: &mut u64) {
        let tolerance = self.geometry_tolerance_mm;
        let faces: Vec<u64> = self
            .body
            .faces
            .values()
            .filter(|f| {
                f.bounds.len() == 2
                    && matches!(self.surfaces.get(&f.id), Some(Surface::Torus { major, minor, .. }) if major > minor)
            })
            .map(|f| f.id)
            .collect();
        for face_id in faces {
            let Some(Surface::Torus { frame, major, minor }) = self.surfaces.get(&face_id).cloned() else {
                continue;
            };
            let same_sense = self.body.faces[&face_id].same_sense;
            let wrap = |a: f64| a - TAU * (a / TAU).round();
            // angle about the torus axis, and angle about the tube from the outer equator
            let about_axis = |p: V3| {
                let d = sub(p, frame.origin);
                dot(d, frame.y).atan2(dot(d, frame.x))
            };
            let about_tube = |p: V3| {
                let d = sub(p, frame.origin);
                dot(d, frame.z).atan2(dot(d, frame.x).hypot(dot(d, frame.y)) - major)
            };
            let (Some(round_axis), Some(round_tube)) = (
                self.bound_turns(face_id, &about_axis, &wrap),
                self.bound_turns(face_id, &about_tube, &wrap),
            ) else {
                continue;
            };
            let none = |t: f64| t.abs() < 1e-6;
            // true: both loops circle the axis and not the tube (a ring); false: the other way round (a bend)
            let (ring, pair) = match (winding_pair(&round_axis), winding_pair(&round_tube)) {
                (Some(pair), None) if round_tube.iter().all(|t| none(t.1)) => (true, pair),
                (None, Some(pair)) if round_axis.iter().all(|t| none(t.1)) => (false, pair),
                _ => continue,
            };
            let turns = if ring { &round_axis } else { &round_tube };
            let (la, lb, tb) = (turns[pair.0].0.clone(), turns[pair.1].0.clone(), turns[pair.1].1);
            let key = |p: V3| if ring { about_axis(p) } else { about_tube(p) };
            let Some(va) = la.first().and_then(|u| self.use_start(u)) else {
                continue;
            };
            let pa = self.vertices[&va];
            // the loop `la` leaves `va` along its first samples
            let Some(points_a) = self.loop_points(&la) else {
                continue;
            };
            let ahead = sub(points_a[1], points_a[0]);
            let Some(vb) = self.vertex_of_loop_at_angle(&lb, tb.signum(), key(pa), &key, &wrap, next) else {
                continue;
            };
            // the second loop may have been rewritten by an edge split: read it again, rotated to start at the vertex
            let Some(turns) = self.bound_turns(face_id, &key, &wrap) else {
                continue;
            };
            let mut lb = turns[pair.1].0.clone();
            let la = turns[pair.0].0.clone();
            let Some(at) = lb.iter().position(|u| self.use_start(u) == Some(vb)) else {
                continue;
            };
            lb.rotate_left(at);
            let pb = self.vertices[&vb];
            let Some(points_b) = self.loop_points(&lb) else {
                continue;
            };
            let behind = sub(points_b[1], points_b[0]);
            let Some((curve, samples)) = torus_seam(&frame, major, minor, ring, same_sense, [pa, pb], [ahead, behind], self.chord_mm, tolerance)
            else {
                continue;
            };
            self.install_band_seam(face_id, pair, la, lb, (va, vb), curve, samples, next);
        }
    }
    /// Cut a revolution band between two winding circles along its bounded profile interval.
    fn seam_unseamed_revolution_bands(&mut self, next: &mut u64) {
        let faces: Vec<u64> = self.body.faces.values()
            .filter(|f| f.bounds.len() == 2 && matches!(self.surfaces.get(&f.id), Some(Surface::Param(s)) if s.revolution_profile().is_some()))
            .map(|f| f.id).collect();
        for face_id in faces {
            let Some(Surface::Param(surface)) = self.surfaces.get(&face_id) else {
                continue;
            };
            let Some((profile, origin, axis)) = surface.revolution_profile() else {
                continue;
            };
            let Ok(reference) = profile.evaluate(profile.domain()[0]) else {
                continue;
            };
            let reference = sub(reference, origin);
            let x = sub(reference, mul(axis, dot(reference, axis)));
            if norm(x) <= self.geometry_tolerance_mm {
                continue;
            }
            let x = mul(x, 1. / norm(x));
            let y = cross(axis, x);
            let angle = |p: V3| {
                let d = sub(p, origin);
                dot(d, y).atan2(dot(d, x))
            };
            let wrap = |a: f64| a - TAU * (a / TAU).round();
            let Some(turns) = self.bound_turns(face_id, &angle, &wrap) else {
                continue;
            };
            let Some(pair) = winding_pair(&turns) else {
                continue;
            };
            let (la, lb, tb) = (
                turns[pair.0].0.clone(),
                turns[pair.1].0.clone(),
                turns[pair.1].1,
            );
            let Some(va) = la.first().and_then(|u| self.use_start(u)) else {
                continue;
            };
            let pa = self.vertices[&va];
            let Some(vb) =
                self.vertex_of_loop_at_angle(&lb, tb.signum(), angle(pa), &angle, &wrap, next)
            else {
                continue;
            };
            let Some(turns) = self.bound_turns(face_id, &angle, &wrap) else {
                continue;
            };
            let (la, mut lb) = (turns[pair.0].0.clone(), turns[pair.1].0.clone());
            let Some(at) = lb.iter().position(|u| self.use_start(u) == Some(vb)) else {
                continue;
            };
            lb.rotate_left(at);
            let pb = self.vertices[&vb];
            let Some(Surface::Param(surface)) = self.surfaces.get(&face_id) else {
                continue;
            };
            let Some((profile, origin, axis)) = surface.revolution_profile() else {
                continue;
            };
            let (Some(ta), Some(tb)) = (
                surface.revolution_parameter(pa, self.geometry_tolerance_mm),
                surface.revolution_parameter(pb, self.geometry_tolerance_mm),
            ) else { continue };
            if (ta - tb).abs() <= 1e-10 * (profile.domain()[1] - profile.domain()[0]) { continue }
            let reverse = ta > tb;
            let Ok((params, points)) = sample_spline_range(profile, [ta.min(tb), ta.max(tb)], self.chord_mm, 100_000, SPLIT_SHARE)
            else {
                continue;
            };
            let (first, last) = (points[0], *points.last().unwrap());
            let radial = |p: V3| {
                let d = sub(p, origin);
                sub(d, mul(axis, dot(d, axis)))
            };
            let x = radial(if reverse { last } else { first });
            let y = radial(pa);
            if norm(x) <= self.geometry_tolerance_mm || norm(y) <= self.geometry_tolerance_mm {
                continue;
            }
            let theta = dot(axis, cross(x, y)).atan2(dot(x, y));
            let rotate = |p: V3| {
                let d = sub(p, origin);
                let (s, c) = theta.sin_cos();
                add(
                    origin,
                    add(
                        add(mul(d, c), mul(cross(axis, d), s)),
                        mul(axis, dot(axis, d) * (1. - c)),
                    ),
                )
            };
            let matches = if reverse {
                norm(sub(rotate(last), pa)) <= self.geometry_tolerance_mm
                    && norm(sub(rotate(first), pb)) <= self.geometry_tolerance_mm
            } else {
                norm(sub(rotate(first), pa)) <= self.geometry_tolerance_mm
                    && norm(sub(rotate(last), pb)) <= self.geometry_tolerance_mm
            };
            if !matches {
                continue;
            }
            let controls = profile
                .control_points()
                .iter()
                .copied()
                .map(rotate)
                .collect();
            let Ok(rotated) = crate::nurbs::NurbsCurve::new_active_domain(
                profile.degree(),
                profile.knots().to_vec(),
                controls,
                profile.weights().to_vec(),
                crate::nurbs::Limits::default(),
            ) else {
                continue;
            };
            let mut samples: Vec<V3> = points.into_iter().map(rotate).collect();
            let mut params = params;
            if reverse {
                samples.reverse();
                params.reverse();
            }
            samples[0] = pa;
            *samples.last_mut().unwrap() = pb;
            self.install_band_seam(
                face_id,
                pair,
                la,
                lb,
                (va, vb),
                Curve::Spline {
                    curve: std::sync::Arc::new(rotated),
                    params,
                },
                samples,
                next,
            );
        }
    }
    fn use_start(&self, u: &EdgeUse) -> Option<u64> {
        let e = self.body.edges.get(&u.edge)?;
        Some(if u.forward { e.start } else { e.end })
    }
    /// The samples of a loop in traversal order, each edge's last sample left to the next edge.
    fn loop_points(&self, uses: &[EdgeUse]) -> Option<Vec<V3>> {
        let mut points = Vec::new();
        for u in uses {
            let samples = self.edges.get(&u.edge)?;
            if u.forward {
                points.extend_from_slice(&samples[..samples.len() - 1]);
            } else {
                points.extend(samples[1..].iter().rev());
            }
        }
        Some(points)
    }
    /// Every bound of a face as a directed edge-use list (a reversed bound runs its uses backwards and flipped) with the
    /// signed number of turns it makes in `angle` (a periodic coordinate: `wrap` brings an angle difference to (-pi, pi]).
    fn bound_turns(
        &self,
        face_id: u64,
        angle: &impl Fn(V3) -> f64,
        wrap: &impl Fn(f64) -> f64,
    ) -> Option<Vec<(Vec<EdgeUse>, f64)>> {
        let face = self.body.faces.get(&face_id)?;
        let directed = |b: &RawBound| -> Vec<EdgeUse> {
            if b.forward {
                b.edges.clone()
            } else {
                b.edges
                    .iter()
                    .rev()
                    .map(|u| EdgeUse {
                        id: u.id,
                        edge: u.edge,
                        forward: !u.forward,
                    })
                    .collect()
            }
        };
        face.bounds
            .iter()
            .map(|b| {
                let uses = directed(b);
                let points = self.loop_points(&uses)?;
                let mut total = 0.;
                for i in 0..points.len() {
                    total += wrap(angle(points[(i + 1) % points.len()]) - angle(points[i]));
                }
                Some((uses, total / TAU))
            })
            .collect()
    }
    /// A vertex of the loop at the angle `target` around the axis, inserting one on the loop's circle (and splitting its
    /// edge) when no sample is there. `direction` is the loop's turn sign. `None` leaves the model valid and unsplit
    /// unless the failure came after the sample was added (an extra sample on a shared edge is harmless).
    fn vertex_of_loop_at_angle(
        &mut self,
        uses: &[EdgeUse],
        direction: f64,
        target: f64,
        angle: &impl Fn(V3) -> f64,
        wrap: &impl Fn(f64) -> f64,
        next: &mut u64,
    ) -> Option<u64> {
        for u in uses {
            let samples = self.edges.get(&u.edge)?.clone();
            let n = samples.len();
            let order: Vec<usize> = if u.forward {
                (0..n).collect()
            } else {
                (0..n).rev().collect()
            };
            for j in 0..n - 1 {
                let (a0, a1) = (angle(samples[order[j]]), angle(samples[order[j + 1]]));
                let (delta, d) = (wrap(a1 - a0), wrap(target - a0));
                if delta == 0. || delta.signum() != direction.signum() || d * delta < 0. {
                    continue;
                }
                let t = d / delta;
                if t > 1. + 1e-12 {
                    continue;
                }
                let raw = self.body.edges.get(&u.edge)?.clone();
                // a sample already at the angle, or one to insert on this segment
                let snap = if t < 1e-9 {
                    Some(order[j])
                } else if t > 1. - 1e-9 {
                    Some(order[j + 1])
                } else {
                    None
                };
                let index = match snap {
                    Some(i) => i,
                    None => {
                        let lower = order[j].min(order[j + 1]);
                        let tt = if u.forward { t } else { 1. - t };
                        self.insert_edge_splits(u.edge, &[(lower, tt)]).ok()?;
                        lower + 1
                    }
                };
                let len = self.edges.get(&u.edge)?.len();
                return if index == 0 {
                    Some(raw.start)
                } else if index == len - 1 {
                    Some(raw.end)
                } else {
                    self.split_edge_at_sample(u.edge, index, next)
                };
            }
        }
        None
    }
    /// Split `edge` at its interior sample `index` into two edges meeting at a new vertex, in every face that uses it. Only
    /// circle and line edges (whose pieces keep the whole curve and their own sample slices) are split.
    fn split_edge_at_sample(&mut self, edge: u64, index: usize, next: &mut u64) -> Option<u64> {
        let samples = self.edges.get(&edge)?.clone();
        if index == 0 || index + 1 >= samples.len() {
            return None;
        }
        let curve = self.curves.get(&edge)?.clone();
        // The pieces of a circle, ellipse or line keep the whole curve (each piece is located by its own end vertices); the
        // pieces of a spline keep the whole curve and the parameters of their own samples.
        let piece_curve = |upper: bool| match &curve {
            Curve::Spline { curve, params } => Curve::Spline {
                curve: curve.clone(),
                params: if upper { params[index..].to_vec() } else { params[..=index].to_vec() },
            },
            other => other.clone(),
        };
        if let Curve::Spline { params, .. } = &curve {
            if params.len() != samples.len() {
                return None;
            }
        }
        let raw = self.body.edges.get(&edge)?.clone();
        let (vertex, lower, upper) = (*next, *next + 1, *next + 2);
        *next += 3;
        self.vertices.insert(vertex, samples[index]);
        self.body.vertices.insert(
            vertex,
            crate::step_topology::RawVertex {
                id: vertex,
                point: 0,
                coordinates: samples[index].map(|x| x / self.scale_mm),
            },
        );
        for (id, start, end) in [(lower, raw.start, vertex), (upper, vertex, raw.end)] {
            self.body.edges.insert(
                id,
                RawEdge {
                    id,
                    start,
                    end,
                    curve: raw.curve,
                    same_sense: raw.same_sense,
                },
            );
            self.curves.insert(id, piece_curve(id == upper));
        }
        self.edges.insert(lower, samples[..=index].to_vec());
        self.edges.insert(upper, samples[index..].to_vec());
        self.body.edges.remove(&edge);
        self.edges.remove(&edge);
        self.curves.remove(&edge);
        let mut use_id = *next;
        for face in self.body.faces.values_mut() {
            for bound in &mut face.bounds {
                if !bound.edges.iter().any(|u| u.edge == edge) {
                    continue;
                }
                let mut rebuilt = Vec::with_capacity(bound.edges.len() + 1);
                for u in &bound.edges {
                    if u.edge != edge {
                        rebuilt.push(u.clone());
                        continue;
                    }
                    let pieces = if u.forward { [lower, upper] } else { [upper, lower] };
                    for piece in pieces {
                        rebuilt.push(EdgeUse {
                            id: use_id,
                            edge: piece,
                            forward: u.forward,
                        });
                        use_id += 1;
                    }
                }
                bound.edges = rebuilt;
            }
        }
        *next = use_id;
        Some(vertex)
    }
}

#[cfg(test)]
mod conversion_tests {
    use super::*;
    /// A wiggly degree-6 spline edge (a cross-bore intersection is one): bisecting to the whole chord tolerance and keeping
    /// every sample is what made such edges 10x denser than the face mesher needs. The decimating sampler stays within the
    /// chord of the curve with far fewer samples than the plain bisection (`split_share` 1 = no decimation budget).
    #[test]
    fn a_wiggly_spline_edge_is_decimated_and_stays_within_the_chord() {
        // 14 knot spans (the cross-bore edge has about as many), smooth: the old seed alone was 14 x 12 samples
        let spans = 14usize;
        let n = spans + 6;
        let controls: Vec<V3> = (0..n)
            .map(|i| {
                let x = i as f64 * 9.5 / (n - 1) as f64;
                [x, 0.8 * (x * 0.5).sin(), 0.05 * x]
            })
            .collect();
        let mut knots = vec![0.; 7];
        knots.extend((1..spans).map(|k| k as f64 / spans as f64));
        knots.extend([1.; 7]);
        let curve = crate::nurbs::NurbsCurve::new_clamped(6, knots, controls, vec![1.; n], crate::nurbs::Limits::default()).unwrap();
        let chord = 0.0376;
        let (_, plain) = sample_spline_with(&curve, chord, 100_000, 1.).unwrap();
        let (params, points) = sample_spline_adaptive(&curve, chord, 100_000).unwrap();
        eprintln!("SAMPLES plain {} decimated {}", plain.len(), points.len());
        assert!(points.len() * 2 <= plain.len(), "{} decimated samples against {} plain", points.len(), plain.len());
        assert_eq!(params.len(), points.len());
        let mut worst: f64 = 0.;
        for k in 0..=4000 {
            let q = curve.evaluate(k as f64 / 4000.).unwrap();
            let d = points
                .windows(2)
                .map(|w| {
                    let ab = sub(w[1], w[0]);
                    let t = (dot(sub(q, w[0]), ab) / dot(ab, ab)).clamp(0., 1.);
                    norm(sub(q, add(w[0], mul(ab, t))))
                })
                .fold(f64::INFINITY, f64::min);
            worst = worst.max(d);
        }
        assert!(worst <= chord, "worst deviation {worst} exceeds the chord {chord}");
        let longest = points.windows(2).map(|w| norm(sub(w[1], w[0]))).fold(0., f64::max);
        assert!(longest <= MAX_SEGMENT_CHORDS * chord * (1. + 1e-9), "segment {longest} longer than the cap");
    }
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
