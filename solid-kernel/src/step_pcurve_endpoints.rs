//! Endpoint agreement for an explicitly selected source p-curve interval.
//! No inferred trim, winding, whole-locus/master or B-Rep validity certificate.
use crate::{
    geometry_bounds::Bounds3,
    native_interval::Interval as I,
    nurbs,
    step::{EntityReference, Error, ErrorKind, Inventory},
    step_body::BodyContext,
    step_curve_agreement::{distance_upper, world_point},
    step_parameter_map::ParameterMapRef,
    step_pcurve::{self, SurfaceCurveDeclaration},
};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParameterOrder {
    /// Increasing p-curve parameter runs from source topological start to end.
    Forward,
    Reversed,
}
#[derive(Debug, Clone, Copy)]
pub struct Request {
    pub edge: EntityReference,
    pub pcurve: EntityReference,
    pub interval: [f64; 2],
    /// Proposed correspondence only: closed endpoints cannot prove winding.
    pub order: ParameterOrder,
}
#[derive(Debug, Clone, Copy)]
pub struct Options {
    pub tolerance_mm: f64,
    pub curve_limits: nurbs::Limits,
    pub max_curve_spans_per_enclosure: usize,
    pub max_surface_patches_per_enclosure: usize,
    pub max_enclosures: usize,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            tolerance_mm: 1e-7,
            curve_limits: nurbs::Limits::default(),
            max_curve_spans_per_enclosure: 3,
            max_surface_patches_per_enclosure: 64,
            max_enclosures: 3,
        }
    }
}
#[derive(Debug, Clone, Copy)]
pub struct Progress {
    pub enclosures: usize,
    pub checked_endpoints: usize,
}
#[derive(Debug)]
pub struct Report {
    pub face: EntityReference,
    pub surface: EntityReference,
    pub request: Request,
    pub source_vertices: [EntityReference; 2],
    /// Boxes and errors are in topological start/end order, in physical mm.
    pub source_vertex_bounds_mm: [Bounds3; 2],
    pub mapped_endpoint_bounds_mm: [Bounds3; 2],
    pub endpoint_error_upper_mm: [f64; 2],
    pub interval_world_bounds_mm: Bounds3,
    /// Actual source declaration, not a semantic master verification.
    pub wrapper: SurfaceCurveDeclaration,
    pub enclosures: usize,
}
/// Verify association, whole interval domain admission and both endpoint
/// correspondences. The order is explicit and independent of EDGE_CURVE sense.
/// Endpoints alone cannot establish the selected interior arc/trace or winding.
/// Source Cartesian conversion is outward rounded before distance comparison.
pub fn verify<'a>(
    map: impl Into<ParameterMapRef<'a>>,
    context: &BodyContext,
    data: &Inventory,
    request: Request,
    options: Options,
    mut progress: impl FnMut(Progress) -> bool,
) -> Result<Report, Error> {
    let map = map.into();
    let entity = data.resolve_reference(request.pcurve)?;
    let fail = |kind, message: &str| Error {
        offset: entity.offset,
        kind,
        detail: message.into(),
    };
    let publish = |callback: &mut dyn FnMut(Progress) -> bool, state| {
        if callback(state) {
            Ok(())
        } else {
            Err(fail(
                ErrorKind::Cancelled,
                "p-curve endpoint verification cancelled",
            ))
        }
    };
    let mut state = Progress {
        enclosures: 0,
        checked_endpoints: 0,
    };
    publish(&mut progress, state)?;
    if !options.tolerance_mm.is_finite()
        || options.tolerance_mm <= 0.
        || request.interval.iter().any(|x| !x.is_finite())
        || request.interval[0] >= request.interval[1]
    {
        return Err(fail(
            ErrorKind::Geometry,
            "require positive finite endpoint tolerance and increasing finite interval",
        ));
    }
    data.resolve_reference(map.length_context())?;
    if map.length_context() != context.length_context() {
        return Err(fail(
            ErrorKind::Geometry,
            "map and source body unit context differ",
        ));
    }
    let (edge, declaration, wrapper) = step_pcurve::associated_edge_pcurve_with_limits(
        context,
        data,
        map.face_reference(),
        map.surface_reference(),
        request.edge,
        request.pcurve,
        options.curve_limits,
    )?;
    let enclose = |interval, state: &mut Progress| {
        if state.enclosures >= options.max_enclosures {
            return Err(fail(
                ErrorKind::Limit,
                "p-curve endpoint enclosure budget exceeded",
            ));
        }
        let b = map.bounds_for_declaration(
            data,
            &declaration,
            interval,
            options.max_curve_spans_per_enclosure,
            options.max_surface_patches_per_enclosure,
        )?;
        state.enclosures += 1;
        Ok(b)
    };
    let whole = enclose(request.interval, &mut state)?;
    publish(&mut progress, state)?;
    let ids = [edge.start, edge.end];
    let source_vertices = ids.map(|id| data.entity_reference(id).unwrap());
    let mut source_boxes = [whole; 2];
    let mut mapped_boxes = [whole; 2];
    let mut errors = [0.; 2];
    for i in 0..2 {
        let vertex = &context.body().vertices[&ids[i]];
        let source = world_point(data, vertex.point, context.millimetres_per_source_unit())?;
        let index = if request.order == ParameterOrder::Forward {
            i
        } else {
            1 - i
        };
        let parameter = request.interval[index];
        let b = enclose([parameter; 2], &mut state)?;
        let mapped = std::array::from_fn(|a| I::new(b.min[a], b.max[a]));
        errors[i] = distance_upper(source, mapped, entity)?;
        if errors[i] > options.tolerance_mm {
            return Err(fail(
                ErrorKind::Geometry,
                "selected p-curve endpoint agreement not certified",
            ));
        }
        source_boxes[i] = Bounds3 {
            min: source.map(|x| x.lo),
            max: source.map(|x| x.hi),
        };
        mapped_boxes[i] = b;
        state.checked_endpoints += 1;
        publish(&mut progress, state)?;
    }
    publish(&mut progress, state)?;
    Ok(Report {
        face: map.face_reference(),
        surface: map.surface_reference(),
        request,
        source_vertices,
        source_vertex_bounds_mm: source_boxes,
        mapped_endpoint_bounds_mm: mapped_boxes,
        endpoint_error_upper_mm: errors,
        interval_world_bounds_mm: whole,
        wrapper,
        enclosures: state.enclosures,
    })
}
