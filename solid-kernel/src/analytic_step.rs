//! Native analytic geometry feasibility checks, not trimmed B-Rep/solid validity.
//! Supported subset: one directly represented, unplaced body; line/circle edges;
//! plane/cylinder surfaces. All coordinates are normalized to millimetres.
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
fn scalar(e: &Entity, v: &Value) -> Result<f64, Error> {
    match v {
        Value::Real(x) if x.is_finite() => Ok(*x),
        Value::Integer(x) if x.unsigned_abs() <= 1u64 << 53 => Ok(*x as f64),
        _ => Err(error(e, "unsupported or non-exact geometric scalar")),
    }
}
fn call<'a>(e: &'a Entity, name: &str) -> Result<&'a [Value], Error> {
    match e.components.as_slice() {
        [c] if c.name == name => Ok(&c.args),
        _ => Err(error(e, format!("unsupported {name} declaration"))),
    }
}
fn unit(e: &Entity, v: V3) -> Result<V3, Error> {
    let m = v.iter().map(|x| x.abs()).fold(0., f64::max);
    if !finite(v) || m == 0. {
        return Err(error(e, "zero or non-finite direction"));
    }
    let v = v.map(|x| x / m);
    let n = norm(v);
    if !n.is_finite() || n == 0. {
        return Err(error(e, "unresolved direction"));
    }
    Ok(mul(v, 1. / n))
}
fn vector3(e: &Entity, v: &Value) -> Result<V3, Error> {
    match v {
        Value::List(v) if v.len() == 3 => {
            Ok([scalar(e, &v[0])?, scalar(e, &v[1])?, scalar(e, &v[2])?])
        }
        _ => Err(error(e, "expected three geometric components")),
    }
}
fn point(data: &Inventory, id: u64, scale: f64) -> Result<V3, Error> {
    let e = &data.entities()[&id];
    let p = match call(e, "CARTESIAN_POINT")? {
        [Value::String(_), v] => mul(vector3(e, v)?, scale),
        _ => return Err(error(e, "unsupported 3D point")),
    };
    if coordinate_domain(p) {
        Ok(p)
    } else {
        Err(error(
            e,
            "point is outside the supported coordinate arithmetic range",
        ))
    }
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
    origin: V3,
    x: V3,
    y: V3,
    z: V3,
}
impl Frame {
    fn point(&self, p: V3) -> V3 {
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
    Cylinder { frame: Frame, radius: f64 },
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
        };
        finite(p).then_some(p)
    }
}
#[derive(Debug, Clone)]
enum Curve {
    Line { origin: V3, direction: V3 },
    Circle { frame: Frame, radius: f64 },
}
fn radius(e: &Entity, v: &Value, scale: f64) -> Result<f64, Error> {
    let r = scalar(e, v)? * scale;
    if (1e-30..=1e30).contains(&r) {
        Ok(r)
    } else {
        Err(error(e, "invalid or unsupported geometric radius"))
    }
}
fn curve(data: &Inventory, mut id: u64, scale: f64) -> Result<Curve, Error> {
    let mut seen = BTreeSet::new();
    loop {
        let e = &data.entities()[&id];
        if seen.len() >= 32 || !seen.insert(id) {
            return Err(error(e, "cyclic/deep curve wrapper"));
        }
        let c = match e.components.as_slice() {
            [c] => c,
            _ => return Err(error(e, "unsupported complex 3D curve")),
        };
        match (c.name.as_str(), c.args.as_slice()) {
            ("SURFACE_CURVE" | "SEAM_CURVE", [Value::String(_), Value::Ref(base), _, _]) => {
                id = *base;
            }
            ("LINE", [Value::String(_), Value::Ref(p), Value::Ref(v)]) => {
                let ve = &data.entities()[v];
                let direction = match call(ve, "VECTOR")? {
                    [Value::String(_), Value::Ref(d), length] if scalar(ve, length)? > 0. => {
                        direction(data, *d)?
                    }
                    _ => return Err(error(ve, "unsupported line vector")),
                };
                return Ok(Curve::Line {
                    origin: point(data, *p, scale)?,
                    direction,
                });
            }
            ("CIRCLE", [Value::String(_), Value::Ref(f), r]) => {
                return Ok(Curve::Circle {
                    frame: frame(data, *f, scale)?,
                    radius: radius(e, r, scale)?,
                })
            }
            _ => return Err(error(e, "unsupported referenced 3D curve geometry")),
        }
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
            _ => Err(error(e, "unsupported native analytic surface")),
        },
        _ => Err(error(e, "unsupported complex surface")),
    }
}
#[derive(Debug, Clone, Copy)]
pub struct Options {
    pub chord_mm: f64,
    pub geometry_tolerance_mm: f64,
    pub max_edge_points: usize,
    pub max_total_points: usize,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            chord_mm: 0.05,
            geometry_tolerance_mm: 1e-7,
            max_edge_points: 100_000,
            max_total_points: 1_000_000,
        }
    }
}
#[derive(Debug)]
pub struct AnalyticModel {
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
            vec![a, b]
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
                p.push(frame.point([radius * t.cos(), radius * t.sin(), 0.]));
            }
            *p.first_mut().unwrap() = a;
            *p.last_mut().unwrap() = b;
            p
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
fn on_surface(e: &Entity, s: &Surface, c: &Curve, a: V3, b: V3, tol: f64) -> Result<(), Error> {
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
        }
    }
    pub fn chord_tolerance_mm(&self) -> f64 {
        self.chord_mm
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
        for e in data.entities().values() {
            if e.components.iter().any(|c| {
                matches!(
                    c.name.as_str(),
                    "MAPPED_ITEM"
                        | "NEXT_ASSEMBLY_USAGE_OCCURRENCE"
                        | "CONTEXT_DEPENDENT_SHAPE_REPRESENTATION"
                        | "REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION"
                )
            }) {
                return Err(error(e, "assembly/placement mapping is not implemented"));
            }
        }
        if data.counts().get("MANIFOLD_SOLID_BREP") != Some(&1) {
            return Err(fail("analytic scope requires exactly one supported body"));
        }
        let body = data.raw_bodies()?.remove(0);
        let mut context = None;
        for e in data.entities().values() {
            for c in &e.components {
                if matches!(
                    c.name.as_str(),
                    "ADVANCED_BREP_SHAPE_REPRESENTATION" | "SHAPE_REPRESENTATION"
                ) {
                    if let [Value::String(_), Value::List(items), Value::Ref(ctx)] =
                        c.args.as_slice()
                    {
                        if items.iter().any(|v| *v == Value::Ref(body.id)) {
                            if context.replace(*ctx).is_some() {
                                return Err(error(
                                    e,
                                    "multiple body representations require explicit mapping",
                                ));
                            }
                            for item in items {
                                if let Value::Ref(id) = item {
                                    let ie = &data.entities()[id];
                                    if *id != body.id
                                        && !ie
                                            .components
                                            .iter()
                                            .all(|c| c.name == "AXIS2_PLACEMENT_3D")
                                    {
                                        return Err(error(ie, "unsupported representation item"));
                                    }
                                } else {
                                    return Err(error(
                                        e,
                                        "unsupported representation item encoding",
                                    ));
                                }
                            }
                        }
                    }
                }
            }
        }
        let context = context.ok_or_else(|| fail("no direct body/representation association"))?;
        let contexts = data.length_contexts()?;
        let scale = contexts
            .iter()
            .find(|c| c.context == context)
            .ok_or_else(|| fail("body representation has no assigned length context"))?
            .metres
            * 1000.;
        if !scale.is_finite() || scale <= 0. {
            return Err(fail("source length normalization overflows"));
        }
        let mut vertices = BTreeMap::new();
        for v in body.vertices.values() {
            let p = mul(v.coordinates, scale);
            if !coordinate_domain(p) {
                return Err(fail(
                    "vertex is outside the supported coordinate arithmetic range",
                ));
            }
            vertices.insert(v.id, p);
        }
        let mut curves = BTreeMap::new();
        let mut edges = BTreeMap::new();
        let mut total = 0usize;
        for edge in body.edges.values() {
            let e = &data.entities()[&edge.id];
            let c = curve(data, edge.curve, scale)?;
            let p = samples(
                e,
                &c,
                vertices[&edge.start],
                vertices[&edge.end],
                edge.start == edge.end,
                edge.same_sense,
                opt,
                opt.max_total_points.saturating_sub(total),
            )?;
            total += p.len();
            edges.insert(edge.id, p);
            curves.insert(edge.id, c);
        }
        let mut surfaces = BTreeMap::new();
        for face in body.faces.values() {
            let s = surface(data, face.surface, scale)?;
            for bound in &face.bounds {
                for u in &bound.edges {
                    let edge = &body.edges[&u.edge];
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
        Ok(Self {
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
