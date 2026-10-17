//! Bounded source p-curve declarations in raw surface parameter coordinates.
//! No mm/angle conversion, 3D agreement, trimming or master-selection inference.
use crate::{
    analytic_step::scalar,
    nurbs::{Limits, NurbsCurve},
    step::{Entity, EntityReference, Error, ErrorKind, Inventory, Value},
    step_nurbs::{self, DecodedCurve, Logical},
};
#[derive(Debug)]
pub struct ParametricSpline {
    definition: DecodedCurve,
}
impl ParametricSpline {
    /// Controls embed raw [u,v] as [u,v,0]; these are not world coordinates.
    pub fn curve(&self) -> &NurbsCurve {
        &self.definition.curve
    }
    pub fn source(&self) -> EntityReference {
        self.definition.source
    }
    pub fn control_point_sources(&self) -> &[EntityReference] {
        &self.definition.control_point_sources
    }
    pub fn declared_name(&self) -> &str {
        &self.definition.declared_name
    }
    pub fn declared_form(&self) -> &str {
        &self.definition.declared_form
    }
    pub fn declared_closed(&self) -> Logical {
        self.definition.declared_closed
    }
    pub fn declared_self_intersection(&self) -> Logical {
        self.definition.declared_self_intersection
    }
    pub fn declared_knot_spec(&self) -> &str {
        &self.definition.declared_knot_spec
    }
}
#[derive(Debug)]
pub enum ParametricCurve {
    Line {
        origin: [f64; 2],
        direction: [f64; 2],
        magnitude: f64,
    },
    Circle {
        origin: [f64; 2],
        x_direction: [f64; 2],
        radius: f64,
    },
    Spline(ParametricSpline),
}
#[derive(Debug)]
pub struct PcurveDeclaration {
    pub source: EntityReference,
    pub basis_surface: EntityReference,
    pub representation: EntityReference,
    pub parameter_context: EntityReference,
    pub geometry_source: EntityReference,
    pub declared_name: String,
    pub geometry: ParametricCurve,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CurveMaster {
    Curve3d,
    Pcurve1,
    Pcurve2,
}
#[derive(Debug)]
pub struct SurfaceCurveDeclaration {
    pub source: EntityReference,
    pub declared_name: String,
    pub geometry_3d: EntityReference,
    pub pcurves: Vec<EntityReference>,
    pub declared_master: CurveMaster,
    pub seam: bool,
}
fn fail(e: &Entity, message: &str) -> Error {
    Error {
        offset: e.offset,
        kind: ErrorKind::Geometry,
        detail: message.into(),
    }
}
fn args<'a>(e: &'a Entity, name: &str) -> Result<&'a [Value], Error> {
    match e.components.as_slice() {
        [c] if c.name == name => Ok(&c.args),
        _ => Err(fail(e, &format!("unsupported {name} declaration"))),
    }
}
fn pair(e: &Entity, v: &Value) -> Result<[f64; 2], Error> {
    match v {
        Value::List(v) if v.len() == 2 => Ok([scalar(e, &v[0])?, scalar(e, &v[1])?]),
        _ => Err(fail(e, "require exactly two parameter coordinates")),
    }
}
fn point(data: &Inventory, id: u64) -> Result<[f64; 2], Error> {
    let e = &data.entities()[&id];
    match args(e, "CARTESIAN_POINT")? {
        [Value::String(_), v] => pair(e, v),
        _ => Err(fail(e, "invalid parameter point")),
    }
}
fn direction(data: &Inventory, id: u64) -> Result<[f64; 2], Error> {
    let e = &data.entities()[&id];
    let p = match args(e, "DIRECTION")? {
        [Value::String(_), v] => pair(e, v)?,
        _ => return Err(fail(e, "invalid parameter direction")),
    };
    let n = crate::native_math::normalize_direction([p[0], p[1], 0.]).map_err(|s| fail(e, s))?;
    Ok([n[0], n[1]])
}
fn context(e: &Entity) -> Result<(), Error> {
    let required = [
        "GEOMETRIC_REPRESENTATION_CONTEXT",
        "PARAMETRIC_REPRESENTATION_CONTEXT",
        "REPRESENTATION_CONTEXT",
    ];
    if e.components.len() != 3
        || required
            .iter()
            .any(|name| e.components.iter().filter(|c| c.name == *name).count() != 1)
    {
        return Err(fail(
            e,
            "unsupported/duplicate parameter representation context components",
        ));
    }
    for c in &e.components {
        let valid = match c.name.as_str() {
            "GEOMETRIC_REPRESENTATION_CONTEXT" => matches!(c.args.as_slice(), [Value::Integer(2)]),
            "PARAMETRIC_REPRESENTATION_CONTEXT" => c.args.is_empty(),
            "REPRESENTATION_CONTEXT" => {
                matches!(c.args.as_slice(), [Value::String(_), Value::String(_)])
            }
            _ => false,
        };
        if !valid {
            return Err(fail(e, "invalid two-dimensional parameter context"));
        }
    }
    Ok(())
}
fn geometry(data: &Inventory, id: u64, limits: Limits) -> Result<ParametricCurve, Error> {
    let e = &data.entities()[&id];
    if e.components
        .iter()
        .any(|c| c.name == "B_SPLINE_CURVE_WITH_KNOTS")
    {
        let definition = step_nurbs::read_definition(data, id, limits, true, |id| {
            let p = point(data, id)?;
            Ok([p[0], p[1], 0.])
        })?;
        return Ok(ParametricCurve::Spline(ParametricSpline { definition }));
    }
    match e.components.as_slice() {
        [c] if c.name == "LINE" => match c.args.as_slice() {
            [Value::String(_), Value::Ref(p), Value::Ref(v)] => {
                let vector = &data.entities()[v];
                let (d, m) = match args(vector, "VECTOR")? {
                    [Value::String(_), Value::Ref(d), m] => (*d, scalar(vector, m)?),
                    _ => return Err(fail(vector, "invalid parameter vector")),
                };
                if m <= 0. {
                    return Err(fail(vector, "parameter vector magnitude must be positive"));
                }
                Ok(ParametricCurve::Line {
                    origin: point(data, *p)?,
                    direction: direction(data, d)?,
                    magnitude: m,
                })
            }
            _ => Err(fail(e, "invalid parameter LINE fields")),
        },
        [c] if c.name == "CIRCLE" => match c.args.as_slice() {
            [Value::String(_), Value::Ref(frame), r] => {
                let frame = &data.entities()[frame];
                let (p, x) = match args(frame, "AXIS2_PLACEMENT_2D")? {
                    [Value::String(_), Value::Ref(p), Value::Omitted] => (*p, [1., 0.]),
                    [Value::String(_), Value::Ref(p), Value::Ref(d)] => (*p, direction(data, *d)?),
                    _ => return Err(fail(frame, "invalid parameter circle placement")),
                };
                let radius = scalar(e, r)?;
                if radius <= 0. {
                    return Err(fail(e, "parameter circle radius must be positive"));
                }
                Ok(ParametricCurve::Circle {
                    origin: point(data, p)?,
                    x_direction: x,
                    radius,
                })
            }
            _ => Err(fail(e, "invalid parameter CIRCLE fields")),
        },
        _ => Err(fail(e, "unsupported parameter curve geometry")),
    }
}
/// Preserve raw UV and knot values. Mapping to a plane/cylinder/spline surface
/// must apply that surface's parameter semantics separately, never mm-scale UV.
pub fn read_pcurve(
    data: &Inventory,
    reference: EntityReference,
    limits: Limits,
) -> Result<PcurveDeclaration, Error> {
    let e = data.resolve_reference(reference)?;
    let (name, surface, rep) = match args(e, "PCURVE")? {
        [Value::String(name), Value::Ref(s), Value::Ref(r)] => (name, *s, *r),
        _ => return Err(fail(e, "invalid PCURVE fields")),
    };
    let surface_entity = &data.entities()[&surface];
    if !surface_entity.components.iter().any(|c| {
        matches!(
            c.name.as_str(),
            "PLANE"
                | "CYLINDRICAL_SURFACE"
                | "CONICAL_SURFACE"
                | "SPHERICAL_SURFACE"
                | "TOROIDAL_SURFACE"
                | "B_SPLINE_SURFACE"
                | "B_SPLINE_SURFACE_WITH_KNOTS"
                | "SURFACE_OF_LINEAR_EXTRUSION"
                | "SURFACE_OF_REVOLUTION"
                | "OFFSET_SURFACE"
                | "SURFACE_REPLICA"
        )
    }) {
        return Err(fail(
            surface_entity,
            "unsupported p-curve basis surface declaration",
        ));
    }
    let representation = &data.entities()[&rep];
    let (curve, ctx) = match args(representation, "DEFINITIONAL_REPRESENTATION")? {
        [Value::String(_), Value::List(items), Value::Ref(ctx)] => match items.as_slice() {
            [Value::Ref(curve)] => (*curve, *ctx),
            _ => {
                return Err(fail(
                    representation,
                    "p-curve representation requires exactly one curve item",
                ))
            }
        },
        _ => return Err(fail(representation, "invalid p-curve representation")),
    };
    context(&data.entities()[&ctx])?;
    Ok(PcurveDeclaration {
        source: reference,
        basis_surface: data.entity_reference(surface).unwrap(),
        representation: data.entity_reference(rep).unwrap(),
        parameter_context: data.entity_reference(ctx).unwrap(),
        geometry_source: data.entity_reference(curve).unwrap(),
        declared_name: name.clone(),
        geometry: geometry(data, curve, limits)?,
    })
}
/// Read declared master choice and associations only. This does not assert the
/// 3D geometry agrees with either p-curve, even when it is the declared master.
pub fn read_surface_curve(
    data: &Inventory,
    reference: EntityReference,
) -> Result<SurfaceCurveDeclaration, Error> {
    let e = data.resolve_reference(reference)?;
    let [c] = e.components.as_slice() else {
        return Err(fail(e, "unsupported complex surface-curve declaration"));
    };
    if !matches!(c.name.as_str(), "SURFACE_CURVE" | "SEAM_CURVE") {
        return Err(fail(e, "source is not a surface/seam curve"));
    }
    let [Value::String(name), Value::Ref(geometry), Value::List(associations), Value::Enum(master)] =
        c.args.as_slice()
    else {
        return Err(fail(e, "invalid surface-curve fields"));
    };
    if associations.is_empty() || associations.len() > 2 {
        return Err(fail(e, "require one or two p-curve associations"));
    }
    let mut pcurves = Vec::new();
    for association in associations {
        let Value::Ref(id) = association else {
            return Err(fail(e, "invalid p-curve association"));
        };
        let p = &data.entities()[id];
        args(p, "PCURVE")?;
        let reference = data.entity_reference(*id).unwrap();
        if pcurves.contains(&reference) {
            return Err(fail(e, "duplicate p-curve association"));
        }
        pcurves.push(reference);
    }
    let declared_master = match master.as_str() {
        "CURVE_3D" => CurveMaster::Curve3d,
        "PCURVE_S1" => CurveMaster::Pcurve1,
        "PCURVE_S2" if pcurves.len() == 2 => CurveMaster::Pcurve2,
        _ => {
            return Err(fail(
                e,
                "invalid or unavailable declared surface-curve master",
            ))
        }
    };
    Ok(SurfaceCurveDeclaration {
        source: reference,
        declared_name: name.clone(),
        geometry_3d: data.entity_reference(*geometry).unwrap(),
        pcurves,
        declared_master,
        seam: c.name == "SEAM_CURVE",
    })
}

pub(crate) fn associated_edge_pcurve<'a>(
    context: &'a crate::step_body::BodyContext,
    data: &Inventory,
    face_reference: EntityReference,
    surface: EntityReference,
    edge: EntityReference,
    pcurve: EntityReference,
) -> Result<
    (
        &'a crate::step_topology::RawEdge,
        crate::step_pcurve::PcurveDeclaration,
        crate::step_pcurve::SurfaceCurveDeclaration,
    ),
    Error,
> {
    let face = data.resolve_reference(face_reference)?;
    data.resolve_reference(context.length_context())?;
    let entity = data.resolve_reference(edge)?;
    let f = context
        .body()
        .faces
        .get(&face_reference.entity)
        .ok_or_else(|| fail(face, "map face not in this source body"))?;
    if !f
        .bounds
        .iter()
        .flat_map(|b| &b.edges)
        .any(|u| u.edge == edge.entity)
    {
        return Err(fail(entity, "edge is not a bound of the mapped face"));
    }
    let topology = context
        .body()
        .edges
        .get(&edge.entity)
        .ok_or_else(|| fail(entity, "source is not a body edge"))?;
    let declaration =
        crate::step_pcurve::read_pcurve(data, pcurve, crate::nurbs::Limits::default())?;
    if declaration.basis_surface != surface {
        return Err(fail(
            entity,
            "p-curve basis surface does not match the mapped face",
        ));
    }
    let mut current = topology.curve;
    let mut found = None;
    let mut seen = std::collections::BTreeSet::new();
    while seen.len() < 32 && seen.insert(current) {
        let record = &data.entities()[&current];
        if !record
            .components
            .iter()
            .any(|c| matches!(c.name.as_str(), "SURFACE_CURVE" | "SEAM_CURVE"))
        {
            break;
        }
        let wrapper =
            crate::step_pcurve::read_surface_curve(data, data.entity_reference(current).unwrap())?;
        current = wrapper.geometry_3d.entity;
        if wrapper.pcurves.contains(&pcurve) {
            found = Some(wrapper);
            break;
        }
    }
    let wrapper = found.ok_or_else(|| {
        fail(
            entity,
            "p-curve is not associated with this source edge wrapper",
        )
    })?;
    Ok((topology, declaration, wrapper))
}

/// Original source direction ratios, before rounded display normalization.
pub(crate) fn line_direction_ratios(
    data: &Inventory,
    source: EntityReference,
) -> Result<[f64; 2], Error> {
    let line = data.resolve_reference(source)?;
    let vector = match args(line, "LINE")? {
        [Value::String(_), Value::Ref(_), Value::Ref(v)] => &data.entities()[v],
        _ => return Err(fail(line, "invalid parameter LINE")),
    };
    let d = match args(vector, "VECTOR")? {
        [Value::String(_), Value::Ref(d), m] if scalar(vector, m)? > 0. => &data.entities()[d],
        _ => return Err(fail(vector, "invalid parameter vector")),
    };
    let raw = match args(d, "DIRECTION")? {
        [Value::String(_), v] => pair(d, v)?,
        _ => return Err(fail(d, "invalid parameter direction")),
    };
    crate::native_math::normalize_direction([raw[0], raw[1], 0.]).map_err(|s| fail(d, s))?;
    Ok(raw)
}
