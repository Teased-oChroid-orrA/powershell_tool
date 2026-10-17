//! Bounded extraction of supported STEP B-spline curve declarations. Not B-Rep
//! import, placement/unit-context binding or certification of declared flags.
use crate::{
    analytic_step::{point, scalar},
    nurbs::{Limits, NurbsCurve},
    step::{Entity, EntityReference, Error, ErrorKind, Inventory, Value},
};
use std::collections::BTreeMap;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Logical {
    True,
    False,
    Unknown,
}
#[derive(Debug)]
pub struct StepSplineDeclaration {
    pub curve: NurbsCurve,
    pub source: EntityReference,
    /// Cartesian control declarations, not topological VERTEX_POINT identities.
    pub control_point_sources: Vec<EntityReference>,
    pub declared_name: String,
    pub declared_form: String,
    pub declared_closed: Logical,
    pub declared_self_intersection: Logical,
    pub declared_knot_spec: String,
    /// Explicit caller-provided conversion. This reader does not establish a
    /// representation context or infer units from entity names/header labels.
    pub millimetres_per_source_unit: f64,
}
pub(crate) fn fail(e: &Entity, detail: impl Into<String>) -> Error {
    Error {
        offset: e.offset,
        kind: ErrorKind::Geometry,
        detail: detail.into(),
    }
}
pub(crate) fn logical(e: &Entity, v: &Value) -> Result<Logical, Error> {
    match v {
        Value::Enum(s) if s == "T" => Ok(Logical::True),
        Value::Enum(s) if s == "F" => Ok(Logical::False),
        Value::Enum(s) if s == "U" => Ok(Logical::Unknown),
        _ => Err(fail(e, "invalid STEP spline logical declaration")),
    }
}
pub(crate) fn integer(e: &Entity, v: &Value) -> Result<usize, Error> {
    match v {
        Value::Integer(n) => {
            usize::try_from(*n).map_err(|_| fail(e, "negative/out-of-range spline count"))
        }
        _ => Err(fail(
            e,
            "spline degree/multiplicity must be an exact integer",
        )),
    }
}
pub(crate) fn enumeration(e: &Entity, v: &Value, allowed: &[&str]) -> Result<String, Error> {
    match v {
        Value::Enum(s) if allowed.contains(&s.as_str()) => Ok(s.clone()),
        _ => Err(fail(e, "unsupported spline form/knot specification")),
    }
}
pub(crate) const KNOT_SPECS: &[&str] = &[
    "UNIFORM_KNOTS",
    "QUASI_UNIFORM_KNOTS",
    "PIECEWISE_BEZIER_KNOTS",
    "UNSPECIFIED",
];
pub(crate) enum SplineKind {
    Curve,
    Surface,
}
pub(crate) struct Parts<'a> {
    pub name: &'a str,
    pub base: &'a [Value],
    pub knots: &'a [Value],
    pub weights: Option<&'a Value>,
}
pub(crate) fn complex_parts(e: &Entity, kind: SplineKind) -> Result<Parts<'_>, Error> {
    if !(6..=7).contains(&e.components.len()) {
        return Err(fail(e, "unsupported complex spline declaration shape"));
    }
    let (bounded, base, knots, category, rational) = match kind {
        SplineKind::Curve => (
            "BOUNDED_CURVE",
            "B_SPLINE_CURVE",
            "B_SPLINE_CURVE_WITH_KNOTS",
            "CURVE",
            "RATIONAL_B_SPLINE_CURVE",
        ),
        SplineKind::Surface => (
            "BOUNDED_SURFACE",
            "B_SPLINE_SURFACE",
            "B_SPLINE_SURFACE_WITH_KNOTS",
            "SURFACE",
            "RATIONAL_B_SPLINE_SURFACE",
        ),
    };
    let allowed = [
        bounded,
        base,
        knots,
        category,
        rational,
        "GEOMETRIC_REPRESENTATION_ITEM",
        "REPRESENTATION_ITEM",
    ];
    let mut components = BTreeMap::new();
    for call in &e.components {
        if !allowed.contains(&call.name.as_str())
            || components
                .insert(call.name.as_str(), call.args.as_slice())
                .is_some()
        {
            return Err(fail(e, "unknown or duplicate complex spline component"));
        }
    }
    let get = |name| {
        components
            .get(name)
            .copied()
            .ok_or_else(|| fail(e, format!("missing complex spline component {name}")))
    };
    for wrapper in [bounded, category, "GEOMETRIC_REPRESENTATION_ITEM"] {
        if !get(wrapper)?.is_empty() {
            return Err(fail(e, "unexpected spline wrapper arguments"));
        }
    }
    let [Value::String(name)] = get("REPRESENTATION_ITEM")? else {
        return Err(fail(e, "invalid spline representation item"));
    };
    let weights = match components.get(rational) {
        None => None,
        Some(args) => match args {
            [value] => Some(value),
            _ => return Err(fail(e, "invalid rational spline component")),
        },
    };
    Ok(Parts {
        name,
        base: get(base)?,
        knots: get(knots)?,
        weights,
    })
}
pub(crate) fn expanded_knots(
    e: &Entity,
    multiplicities: &[Value],
    values: &[Value],
    controls: usize,
    degree: usize,
) -> Result<Vec<f64>, Error> {
    crate::nurbs::validate_degree(degree, controls).map_err(|mut error| {
        error.offset = e.offset;
        error
    })?;
    let expected = controls
        .checked_add(degree)
        .and_then(|n| n.checked_add(1))
        .ok_or_else(|| Error {
            offset: e.offset,
            kind: ErrorKind::Limit,
            detail: "spline knot expansion count overflow".into(),
        })?;
    if values.len() != multiplicities.len() || values.len() < 2 || values.len() > expected {
        return Err(fail(e, "invalid spline distinct-knot/multiplicity counts"));
    }
    let mut knots = Vec::with_capacity(expected);
    let mut previous = None;
    for (value, count) in values.iter().zip(multiplicities) {
        let value = scalar(e, value)?;
        if previous.is_some_and(|before| value <= before) {
            return Err(fail(e, "distinct spline knots must increase strictly"));
        }
        previous = Some(value);
        let count = integer(e, count)?;
        if count == 0 || count > expected.saturating_sub(knots.len()) {
            return Err(fail(
                e,
                "spline knot multiplicity exceeds expected expansion",
            ));
        }
        knots.extend(std::iter::repeat_n(value, count));
    }
    if knots.len() != expected {
        return Err(fail(
            e,
            "spline knot expansion does not match degree/control count",
        ));
    }
    Ok(knots)
}
/// Read one actual spline declaration. Supported: simple polynomial
/// B_SPLINE_CURVE_WITH_KNOTS and complete complex polynomial/rational variants.
/// Metadata is retained as declared, never used as a geometric certificate.
pub fn read_curve(
    data: &Inventory,
    id: u64,
    scale_mm: f64,
    limits: Limits,
) -> Result<StepSplineDeclaration, Error> {
    let e = data.entities().get(&id).ok_or_else(|| Error {
        offset: 0,
        kind: ErrorKind::DanglingReference,
        detail: format!("missing spline entity #{id}"),
    })?;
    if !scale_mm.is_finite() || scale_mm <= 0. {
        return Err(fail(e, "spline source scale must be finite and positive"));
    }
    let d = read_definition(data, id, limits, false, |id| point(data, id, scale_mm))?;
    Ok(StepSplineDeclaration {
        curve: d.curve,
        source: d.source,
        control_point_sources: d.control_point_sources,
        declared_name: d.declared_name,
        declared_form: d.declared_form,
        declared_closed: d.declared_closed,
        declared_self_intersection: d.declared_self_intersection,
        declared_knot_spec: d.declared_knot_spec,
        millimetres_per_source_unit: scale_mm,
    })
}
/// Coordinate/units decoding is deliberately supplied separately. Metadata is
/// shared without attaching a false physical length scale to parameter curves.
#[derive(Debug)]
pub(crate) struct DecodedCurve {
    pub curve: NurbsCurve,
    pub source: EntityReference,
    pub control_point_sources: Vec<EntityReference>,
    pub declared_name: String,
    pub declared_form: String,
    pub declared_closed: Logical,
    pub declared_self_intersection: Logical,
    pub declared_knot_spec: String,
}
pub(crate) fn read_definition(
    data: &Inventory,
    id: u64,
    limits: Limits,
    active_domain: bool,
    mut read_point: impl FnMut(u64) -> Result<crate::V3, Error>,
) -> Result<DecodedCurve, Error> {
    let e = data.entities().get(&id).ok_or_else(|| Error {
        offset: 0,
        kind: ErrorKind::DanglingReference,
        detail: format!("missing spline entity #{id}"),
    })?;
    let parts = match e.components.as_slice() {
        [call] if call.name == "B_SPLINE_CURVE_WITH_KNOTS" && call.args.len() == 9 => {
            let Value::String(name) = &call.args[0] else {
                return Err(fail(e, "invalid spline representation name"));
            };
            Parts {
                name,
                base: &call.args[1..6],
                knots: &call.args[6..9],
                weights: None,
            }
        }
        _ => complex_parts(e, SplineKind::Curve)?,
    };
    let Parts {
        name,
        base,
        knots: knot_data,
        weights: weight_data,
    } = parts;
    let [degree, Value::List(controls), form, closed, self_intersection] = base else {
        return Err(fail(e, "invalid B_SPLINE_CURVE fields"));
    };
    let degree = integer(e, degree)?;
    if degree > limits.degree || controls.len() > limits.control_points {
        return Err(Error {
            offset: e.offset,
            kind: ErrorKind::Limit,
            detail: "spline degree/control-point budget exceeded".into(),
        });
    }
    let [Value::List(multiplicities), Value::List(values), spec] = knot_data else {
        return Err(fail(e, "invalid B_SPLINE_CURVE_WITH_KNOTS fields"));
    };
    let knots = expanded_knots(e, multiplicities, values, controls.len(), degree)?;
    let weights = match weight_data {
        None => vec![1.; controls.len()],
        Some(Value::List(weights)) if weights.len() == controls.len() => weights
            .iter()
            .map(|v| scalar(e, v))
            .collect::<Result<Vec<_>, _>>()?,
        _ => return Err(fail(e, "rational spline weight count/type mismatch")),
    };
    let mut points = Vec::with_capacity(controls.len());
    let mut control_point_sources = Vec::with_capacity(controls.len());
    for control in controls {
        let Value::Ref(id) = control else {
            return Err(fail(e, "spline control point must be an entity reference"));
        };
        points.push(read_point(*id)?);
        control_point_sources.push(data.entity_reference(*id).unwrap());
    }
    let construct = if active_domain {
        NurbsCurve::new_active_domain
    } else {
        NurbsCurve::new_clamped
    };
    let curve = construct(degree, knots, points, weights, limits).map_err(|mut error| {
        error.offset = e.offset;
        error
    })?;
    Ok(DecodedCurve {
        curve,
        source: data.entity_reference(id).unwrap(),
        control_point_sources,
        declared_name: name.into(),
        declared_form: enumeration(
            e,
            form,
            &[
                "POLYLINE_FORM",
                "CIRCULAR_ARC",
                "ELLIPTIC_ARC",
                "PARABOLIC_ARC",
                "HYPERBOLIC_ARC",
                "UNSPECIFIED",
            ],
        )?,
        declared_closed: logical(e, closed)?,
        declared_self_intersection: logical(e, self_intersection)?,
        declared_knot_spec: enumeration(e, spec, KNOT_SPECS)?,
    })
}
