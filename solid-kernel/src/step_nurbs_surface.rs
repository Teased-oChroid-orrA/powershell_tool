//! Supported STEP spline surface declarations only, not B-Rep import or unit/
//! placement/trim/normal/self-intersection certification.
use crate::{
    analytic_step::{point, scalar},
    nurbs_surface::{Limits, NurbsSurface},
    step::{Entity, EntityReference, Error, ErrorKind, Inventory, Value},
    step_nurbs::{
        complex_parts, enumeration, expanded_knots, fail, integer, logical, Logical, Parts,
        SplineKind, KNOT_SPECS,
    },
};
#[derive(Debug)]
pub struct StepSurfaceDeclaration {
    pub surface: NurbsSurface,
    pub source: EntityReference,
    /// Parallel to evaluator V-major control storage; these are Cartesian
    /// declarations, not topological vertices. STEP input itself is U-major.
    pub control_point_sources: Vec<EntityReference>,
    pub declared_name: String,
    pub declared_form: String,
    pub declared_u_closed: Logical,
    pub declared_v_closed: Logical,
    pub declared_self_intersection: Logical,
    pub declared_knot_spec: String,
    pub millimetres_per_source_unit: f64,
}
impl StepSurfaceDeclaration {
    pub fn control_point_reference(&self, uv: [usize; 2]) -> Option<EntityReference> {
        let [nu, nv] = self.surface.dimensions();
        if uv[0] >= nu || uv[1] >= nv {
            return None;
        }
        self.control_point_sources.get(uv[1] * nu + uv[0]).copied()
    }
}
fn rows<'a>(
    e: &Entity,
    values: &'a [Value],
    max_controls: usize,
) -> Result<([usize; 2], Vec<&'a [Value]>), Error> {
    let Some(Value::List(first)) = values.first() else {
        return Err(fail(
            e,
            "surface control/weight net must contain nonempty rows",
        ));
    };
    let dimensions = [values.len(), first.len()];
    let count = dimensions[0]
        .checked_mul(dimensions[1])
        .ok_or_else(|| Error {
            offset: e.offset,
            kind: ErrorKind::Limit,
            detail: "surface net count overflow".into(),
        })?;
    if count > max_controls {
        return Err(Error {
            offset: e.offset,
            kind: ErrorKind::Limit,
            detail: "surface aggregate control-net budget exceeded".into(),
        });
    }
    if count == 0 {
        return Err(fail(e, "empty surface control/weight net"));
    }
    let mut rows = Vec::with_capacity(values.len());
    for value in values {
        let Value::List(row) = value else {
            return Err(fail(e, "surface net row is not a list"));
        };
        if row.len() != dimensions[1] {
            return Err(fail(e, "nonrectangular surface control/weight net"));
        }
        rows.push(row.as_slice());
    }
    Ok((dimensions, rows))
}
/// Read one actual declaration. The caller must bind the explicit mm factor to
/// the correct representation. Declared flags/form are retained, never trusted.
pub fn read_surface(
    data: &Inventory,
    id: u64,
    scale_mm: f64,
    limits: Limits,
) -> Result<StepSurfaceDeclaration, Error> {
    let e = data.entities().get(&id).ok_or_else(|| Error {
        offset: 0,
        kind: ErrorKind::DanglingReference,
        detail: format!("missing surface spline entity #{id}"),
    })?;
    if !scale_mm.is_finite() || scale_mm <= 0. {
        return Err(fail(e, "surface source scale must be finite and positive"));
    }
    let parts = match e.components.as_slice() {
        [call] if call.name == "B_SPLINE_SURFACE_WITH_KNOTS" && call.args.len() == 13 => {
            let Value::String(name) = &call.args[0] else {
                return Err(fail(e, "invalid surface representation name"));
            };
            Parts {
                name,
                base: &call.args[1..8],
                knots: &call.args[8..13],
                weights: None,
            }
        }
        _ => complex_parts(e, SplineKind::Surface)?,
    };
    let [du, dv, Value::List(controls), form, u_closed, v_closed, self_intersection] = parts.base
    else {
        return Err(fail(e, "invalid B_SPLINE_SURFACE fields"));
    };
    let degrees = [integer(e, du)?, integer(e, dv)?];
    if degrees.iter().any(|p| *p > limits.degree) {
        return Err(Error {
            offset: e.offset,
            kind: ErrorKind::Limit,
            detail: "surface degree budget exceeded".into(),
        });
    }
    let (dimensions, controls) = rows(e, controls, limits.control_points)?;
    let [Value::List(um), Value::List(vm), Value::List(uk), Value::List(vk), spec] = parts.knots
    else {
        return Err(fail(e, "invalid B_SPLINE_SURFACE_WITH_KNOTS fields"));
    };
    let knots = [
        expanded_knots(e, um, uk, dimensions[0], degrees[0])?,
        expanded_knots(e, vm, vk, dimensions[1], degrees[1])?,
    ];
    let count = dimensions[0] * dimensions[1];
    let weights = match parts.weights {
        None => vec![1.; count],
        Some(Value::List(values)) => {
            let (shape, weights) = rows(e, values, limits.control_points)?;
            if shape != dimensions {
                return Err(fail(e, "surface weight/control net dimensions differ"));
            }
            let mut flat = Vec::with_capacity(count);
            for v in 0..dimensions[1] {
                for row in &weights {
                    flat.push(scalar(e, &row[v])?);
                }
            }
            flat
        }
        _ => return Err(fail(e, "invalid rational surface weight matrix")),
    };
    let mut points = Vec::with_capacity(count);
    let mut control_point_sources = Vec::with_capacity(count);
    // STEP has U rows and V columns. Evaluator storage is V-major.
    for v in 0..dimensions[1] {
        for row in &controls {
            let Value::Ref(id) = &row[v] else {
                return Err(fail(e, "surface control must reference a Cartesian point"));
            };
            points.push(point(data, *id, scale_mm)?);
            control_point_sources.push(data.entity_reference(*id).unwrap());
        }
    }
    let surface = NurbsSurface::new_clamped(degrees, knots, dimensions, points, weights, limits)
        .map_err(|mut error| {
            error.offset = e.offset;
            error
        })?;
    Ok(StepSurfaceDeclaration {
        surface,
        source: data.entity_reference(id).unwrap(),
        control_point_sources,
        declared_name: parts.name.into(),
        declared_form: enumeration(
            e,
            form,
            &[
                "PLANE_SURF",
                "CYLINDRICAL_SURF",
                "CONICAL_SURF",
                "SPHERICAL_SURF",
                "TOROIDAL_SURF",
                "SURF_OF_REVOLUTION",
                "RULED_SURF",
                "GENERALISED_CONE",
                "QUADRIC_SURF",
                "SURF_OF_LINEAR_EXTRUSION",
                "UNSPECIFIED",
            ],
        )?,
        declared_u_closed: logical(e, u_closed)?,
        declared_v_closed: logical(e, v_closed)?,
        declared_self_intersection: logical(e, self_intersection)?,
        declared_knot_spec: enumeration(e, spec, KNOT_SPECS)?,
        millimetres_per_source_unit: scale_mm,
    })
}
