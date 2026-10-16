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
fn fail(e: &Entity, detail: impl Into<String>) -> Error {
    Error {
        offset: e.offset,
        kind: ErrorKind::Geometry,
        detail: detail.into(),
    }
}
fn logical(e: &Entity, v: &Value) -> Result<Logical, Error> {
    match v {
        Value::Enum(s) if s == "T" => Ok(Logical::True),
        Value::Enum(s) if s == "F" => Ok(Logical::False),
        Value::Enum(s) if s == "U" => Ok(Logical::Unknown),
        _ => Err(fail(e, "invalid STEP spline logical declaration")),
    }
}
fn integer(e: &Entity, v: &Value) -> Result<usize, Error> {
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
fn enumeration(e: &Entity, v: &Value, allowed: &[&str]) -> Result<String, Error> {
    match v {
        Value::Enum(s) if allowed.contains(&s.as_str()) => Ok(s.clone()),
        _ => Err(fail(e, "unsupported spline form/knot specification")),
    }
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
    let (name, base, knot_data, weight_data) = match e.components.as_slice() {
        [call] if call.name == "B_SPLINE_CURVE_WITH_KNOTS" && call.args.len() == 9 => {
            let Value::String(name) = &call.args[0] else {
                return Err(fail(e, "invalid spline representation name"));
            };
            (name.as_str(), &call.args[1..6], &call.args[6..9], None)
        }
        calls if (6..=7).contains(&calls.len()) => {
            let mut components = BTreeMap::new();
            for call in calls {
                if !matches!(
                    call.name.as_str(),
                    "BOUNDED_CURVE"
                        | "B_SPLINE_CURVE"
                        | "B_SPLINE_CURVE_WITH_KNOTS"
                        | "CURVE"
                        | "GEOMETRIC_REPRESENTATION_ITEM"
                        | "RATIONAL_B_SPLINE_CURVE"
                        | "REPRESENTATION_ITEM"
                ) || components
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
            for wrapper in ["BOUNDED_CURVE", "CURVE", "GEOMETRIC_REPRESENTATION_ITEM"] {
                if !get(wrapper)?.is_empty() {
                    return Err(fail(e, "unexpected spline wrapper arguments"));
                }
            }
            let [Value::String(name)] = get("REPRESENTATION_ITEM")? else {
                return Err(fail(e, "invalid spline representation item"));
            };
            let weights = match components.get("RATIONAL_B_SPLINE_CURVE") {
                None => None,
                Some(args) => match args {
                    [value] => Some(value),
                    _ => return Err(fail(e, "invalid rational spline component")),
                },
            };
            (
                name.as_str(),
                get("B_SPLINE_CURVE")?,
                get("B_SPLINE_CURVE_WITH_KNOTS")?,
                weights,
            )
        }
        _ => return Err(fail(e, "unsupported STEP spline declaration shape")),
    };
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
    let expected = controls
        .len()
        .checked_add(degree)
        .and_then(|n| n.checked_add(1))
        .ok_or_else(|| fail(e, "spline knot expansion count overflow"))?;
    let [Value::List(multiplicities), Value::List(values), spec] = knot_data else {
        return Err(fail(e, "invalid B_SPLINE_CURVE_WITH_KNOTS fields"));
    };
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
        points.push(point(data, *id, scale_mm)?);
        control_point_sources.push(data.entity_reference(*id).unwrap());
    }
    let curve =
        NurbsCurve::new_clamped(degree, knots, points, weights, limits).map_err(|mut error| {
            error.offset = e.offset;
            error
        })?;
    Ok(StepSplineDeclaration {
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
        declared_knot_spec: enumeration(
            e,
            spec,
            &[
                "UNIFORM_KNOTS",
                "QUASI_UNIFORM_KNOTS",
                "PIECEWISE_BEZIER_KNOTS",
                "UNSPECIFIED",
            ],
        )?,
        millimetres_per_source_unit: scale_mm,
    })
}
