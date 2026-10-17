//! Shared finite source-line proof for plane and cylinder p-curve carriers.
use crate::{
    analytic_step::{call, vector3},
    native_interval::Interval,
    step::{Entity, EntityReference, Error, ErrorKind, Inventory, Value},
    step_body::BodyContext,
    step_frame::{direction, dot, V},
};
fn fail(e: &Entity, message: &str) -> Error {
    Error {
        offset: e.offset,
        kind: ErrorKind::Geometry,
        detail: message.into(),
    }
}
#[derive(Debug)]
pub struct LineLocus {
    pub face: EntityReference,
    pub edge: EntityReference,
    pub pcurve: EntityReference,
    pub wrapper: EntityReference,
    pub declared_master: crate::step_pcurve::CurveMaster,
    pub master_reference: EntityReference,
    /// Source-identity comparison only; not complete master validation.
    pub checked_pcurve_is_master: bool,
    pub source_to_3d_error_mm: [f64; 2],
    pub source_to_pcurve_error_mm: [f64; 2],
    pub curve_to_pcurve_error_mm: [f64; 2],
    /// Geometric direction agreement only; not parameter speed/offset agreement.
    pub natural_direction_dot: [f64; 2],
}
fn world_point(data: &Inventory, id: u64, scale: f64) -> Result<V, Error> {
    let e = &data.entities()[&id];
    let p = match call(e, "CARTESIAN_POINT")? {
        [Value::String(_), v] => vector3(e, v)?,
        _ => return Err(fail(e, "invalid line/source vertex point")),
    };
    Ok(p.map(|x| Interval::point(x) * Interval::point(scale)))
}
pub(crate) fn distance_upper(a: V, b: V, e: &Entity) -> Result<f64, Error> {
    let d: V = std::array::from_fn(|i| a[i] - b[i]);
    let r = (d[0].square() + d[1].square() + d[2].square()).sqrt_nonnegative();
    if !r.finite() {
        return Err(fail(e, "curve agreement distance not bounded"));
    }
    Ok(r.hi)
}
fn projection(p: V, origin: V, direction: V) -> V {
    let delta = std::array::from_fn(|i| p[i] - origin[i]);
    let t = dot(delta, direction);
    std::array::from_fn(|i| origin[i] + direction[i] * t)
}
pub(crate) fn verify_line(
    context: &BodyContext,
    data: &Inventory,
    face: EntityReference,
    edge: EntityReference,
    pcurve: EntityReference,
    wrapper: &crate::step_pcurve::SurfaceCurveDeclaration,
    pc_origin: V,
    pc_direction: V,
    tolerance_mm: f64,
) -> Result<LineLocus, Error> {
    let entity = data.resolve_reference(edge)?;
    let topology = &context.body().edges[&edge.entity];
    let scale_mm = context.millimetres_per_source_unit();
    let geometry = crate::analytic_step::curve_geometry(data, topology.curve)?;
    let (origin, vector) = match call(geometry, "LINE")? {
        [Value::String(_), Value::Ref(p), Value::Ref(v)] => (*p, &data.entities()[v]),
        _ => return Err(fail(geometry, "only 3D LINE loci are certified")),
    };
    let direction = match call(vector, "VECTOR")? {
        [Value::String(_), Value::Ref(d), m] if crate::analytic_step::scalar(vector, m)? > 0. => {
            direction(data, &Value::Ref(*d), [0.; 3], vector)?
        }
        _ => return Err(fail(vector, "invalid 3D line vector")),
    };
    let origin = world_point(data, origin, scale_mm)?;
    let mut source_to_3d = [0.; 2];
    let mut source_to_pc = [0.; 2];
    let mut curve_to_pc = [0.; 2];
    let mut projected = [[Interval::point(0.); 3]; 2];
    for (i, id) in [topology.start, topology.end].into_iter().enumerate() {
        let vertex = &context.body().vertices[&id];
        let point = world_point(data, vertex.point, scale_mm)?;
        let q = projection(point, origin, direction);
        let r = projection(point, pc_origin, pc_direction);
        let s = projection(q, pc_origin, pc_direction);
        source_to_3d[i] = distance_upper(point, q, entity)?;
        source_to_pc[i] = distance_upper(point, r, entity)?;
        curve_to_pc[i] = distance_upper(q, s, entity)?;
        projected[i] = q;
        if [source_to_3d[i], source_to_pc[i], curve_to_pc[i]]
            .iter()
            .any(|x| *x > tolerance_mm)
        {
            return Err(fail(
                entity,
                "finite 3D line/p-curve/source vertex locus agreement not certified",
            ));
        }
    }
    let span: V = std::array::from_fn(|i| projected[1][i] - projected[0][i]);
    let sense = dot(span, direction);
    if (topology.same_sense && sense.lo <= 0.) || (!topology.same_sense && sense.hi >= 0.) {
        return Err(fail(entity, "source line extent/sense not certified"));
    }
    let orientation = dot(direction, pc_direction);
    if !orientation.finite() {
        return Err(fail(entity, "line natural orientation not bounded"));
    }
    let master = match wrapper.declared_master {
        crate::step_pcurve::CurveMaster::Curve3d => wrapper.geometry_3d,
        crate::step_pcurve::CurveMaster::Pcurve1 => wrapper.pcurves[0],
        crate::step_pcurve::CurveMaster::Pcurve2 => wrapper.pcurves[1],
    };
    Ok(LineLocus {
        face,
        edge,
        pcurve,
        wrapper: wrapper.source,
        declared_master: wrapper.declared_master,
        master_reference: master,
        checked_pcurve_is_master: master == pcurve,
        source_to_3d_error_mm: source_to_3d,
        source_to_pcurve_error_mm: source_to_pc,
        curve_to_pcurve_error_mm: curve_to_pc,
        natural_direction_dot: [orientation.lo, orientation.hi],
    })
}

/// Unsigned whole-circle carrier comparison. It does not select an arc,
/// establish edge sense, or certify complete master/trim/body agreement.
#[derive(Debug)]
pub struct CircleCarrier {
    pub face: EntityReference,
    pub edge: EntityReference,
    pub pcurve: EntityReference,
    pub wrapper: EntityReference,
    pub declared_master: crate::step_pcurve::CurveMaster,
    pub master_reference: EntityReference,
    pub centre_error_mm: f64,
    pub radius_error_mm: f64,
    pub normal_error: f64,
    pub hausdorff_upper_mm: f64,
    pub natural_normal_dot: [f64; 2],
}

pub(crate) fn verify_circle(
    context: &BodyContext,
    data: &Inventory,
    face: EntityReference,
    edge: EntityReference,
    pcurve: EntityReference,
    wrapper: &crate::step_pcurve::SurfaceCurveDeclaration,
    target_origin: V,
    target_normal: V,
    target_radius: Interval,
    tolerance_mm: f64,
) -> Result<CircleCarrier, Error> {
    use crate::step_frame::SourceFrame;
    let entity = data.resolve_reference(edge)?;
    let topology = &context.body().edges[&edge.entity];
    let scale_mm = context.millimetres_per_source_unit();
    let geometry = crate::analytic_step::curve_geometry(data, topology.curve)?;
    let (placement, source_radius) = match call(geometry, "CIRCLE")? {
        [Value::String(_), Value::Ref(frame), r] => (
            &data.entities()[frame],
            crate::analytic_step::scalar(geometry, r)?,
        ),
        _ => return Err(fail(geometry, "requires a source 3D CIRCLE carrier")),
    };
    let source_radius = Interval::point(source_radius) * Interval::point(scale_mm);
    if !source_radius.finite()
        || source_radius.lo <= 0.
        || source_radius.hi > 1e30
        || !target_radius.finite()
        || target_radius.lo <= 0.
        || target_radius.hi > 1e30
    {
        return Err(fail(geometry, "circle radii must be positively bounded"));
    }
    let frame = SourceFrame::read(data, placement, scale_mm)?;
    let centre_error = distance_upper(frame.origin, target_origin, entity)?;
    let natural = dot(frame.z, target_normal);
    // Carrier orientation is unsigned; choose a normal sign only when its
    // alignment is certified. Arc direction and parameter offset stay open.
    let aligned = if natural.lo > 0. {
        target_normal
    } else if natural.hi < 0. {
        target_normal.map(|x| Interval::point(0.) - x)
    } else {
        return Err(fail(
            entity,
            "circle carrier normal alignment not certified",
        ));
    };
    let normal_error = distance_upper(frame.z, aligned, entity)?;
    let difference = source_radius - target_radius;
    let radius_error = difference.lo.abs().max(difference.hi.abs());
    let largest = source_radius.hi.max(target_radius.hi);
    // For aligned unit normals, the minimal plane rotation has operator
    // norm ||n-m|| = 2 sin(theta/2). Pairing equally rotated points bounds
    // both directed distances by centre + radius + Rmax*normal error.
    let total = Interval::point(centre_error)
        + Interval::point(radius_error)
        + Interval::point(largest) * Interval::point(normal_error);
    if !total.finite() || total.hi > tolerance_mm {
        return Err(fail(entity, "whole-circle carrier agreement not certified"));
    }
    let master_reference = match wrapper.declared_master {
        crate::step_pcurve::CurveMaster::Curve3d => wrapper.geometry_3d,
        crate::step_pcurve::CurveMaster::Pcurve1 => wrapper.pcurves[0],
        crate::step_pcurve::CurveMaster::Pcurve2 => wrapper.pcurves[1],
    };
    Ok(CircleCarrier {
        face,
        edge,
        pcurve,
        wrapper: wrapper.source,
        declared_master: wrapper.declared_master,
        master_reference,
        centre_error_mm: centre_error,
        radius_error_mm: radius_error,
        normal_error,
        hausdorff_upper_mm: total.hi,
        natural_normal_dot: [natural.lo, natural.hi],
    })
}
