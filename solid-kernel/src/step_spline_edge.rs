//! Source-bound spline edges: endpoint agreement and bounded samples.
//! Explicit caller trims are checked at endpoints; no inferred trim, simplicity,
//! surface-locus or complete B-Rep certificate.
use crate::{
    analytic_step::point,
    geometry_distance, nurbs, nurbs_sample,
    step::{EntityReference, Error, ErrorKind, Inventory},
    step_body::BodyContext,
    step_nurbs::StepSplineDeclaration,
    V3,
};
#[derive(Debug, Clone, Copy)]
pub struct Options {
    pub geometry_tolerance_mm: f64,
    pub curve_limits: nurbs::Limits,
    pub sampling: nurbs_sample::Options,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            geometry_tolerance_mm: 1e-7,
            curve_limits: nurbs::Limits::default(),
            sampling: nurbs_sample::Options::default(),
        }
    }
}
#[derive(Debug)]
pub struct SampledSplineEdge {
    edge: EntityReference,
    vertices: [EntityReference; 2],
    context: EntityReference,
    source_endpoints_mm: [V3; 2],
    endpoint_errors_mm: [f64; 2],
    same_sense: bool,
    declaration: StepSplineDeclaration,
    samples: nurbs_sample::SampledCurve,
}
impl SampledSplineEdge {
    pub fn edge_reference(&self) -> EntityReference {
        self.edge
    }
    /// Topological start/end VERTEX_POINT identities, not control-point refs.
    pub fn vertex_references(&self) -> [EntityReference; 2] {
        self.vertices
    }
    pub fn length_context(&self) -> EntityReference {
        self.context
    }
    pub fn source_endpoints_mm(&self) -> [V3; 2] {
        self.source_endpoints_mm
    }
    /// Upper distance bounds from exact curve endpoints to normalized source
    /// vertices, in topological start/end order. No endpoint relocation occurs.
    pub fn endpoint_error_bounds_mm(&self) -> [f64; 2] {
        self.endpoint_errors_mm
    }
    pub fn same_sense(&self) -> bool {
        self.same_sense
    }
    /// Complete underlying curve declaration, including portions outside a
    /// selected interval. Sample parameters identify the actual checked portion.
    pub fn declaration(&self) -> &StepSplineDeclaration {
        &self.declaration
    }
    /// Natural increasing curve parameters. Use points_in_edge_order for the
    /// topological start/end direction. These endpoints are never snapped.
    pub fn samples(&self) -> &nurbs_sample::SampledCurve {
        &self.samples
    }
    pub fn points_in_edge_order(&self) -> impl DoubleEndedIterator<Item = &V3> + ExactSizeIterator {
        let points = self.samples.points();
        (0..points.len()).map(move |i| {
            &points[if self.same_sense {
                i
            } else {
                points.len() - 1 - i
            }]
        })
    }
}
/// Supports only edges covering the complete clamped spline domain. Source
/// vertices at interior parameters refuse here; use `sample_interval` with
/// independently supplied parameters to check a finite portion.
/// Source offsets and identities survive errors; no partial samples escape.
pub fn sample(
    context: &BodyContext,
    data: &Inventory,
    edge: EntityReference,
    opt: Options,
    progress: impl FnMut(nurbs_sample::Progress) -> bool,
) -> Result<SampledSplineEdge, Error> {
    read_and_sample(context, data, edge, None, opt, progress)
}
/// Check a caller-selected increasing natural spline interval against both
/// source vertices, then sample it. Source edge same_sense determines vertex
/// order. This does not discover trims or establish the edge's surface locus.
pub fn sample_interval(
    context: &BodyContext,
    data: &Inventory,
    edge: EntityReference,
    interval: [f64; 2],
    opt: Options,
    progress: impl FnMut(nurbs_sample::Progress) -> bool,
) -> Result<SampledSplineEdge, Error> {
    read_and_sample(context, data, edge, Some(interval), opt, progress)
}
fn read_and_sample(
    context: &BodyContext,
    data: &Inventory,
    edge: EntityReference,
    interval: Option<[f64; 2]>,
    opt: Options,
    mut progress: impl FnMut(nurbs_sample::Progress) -> bool,
) -> Result<SampledSplineEdge, Error> {
    let entity = data.resolve_reference(edge)?;
    if !progress(nurbs_sample::Progress {
        enclosures: 0,
        accepted_segments: 0,
        pending_intervals: 0,
    }) {
        return Err(Error {
            offset: entity.offset,
            kind: ErrorKind::Cancelled,
            detail: "spline edge sampling cancelled".into(),
        });
    }
    let declaration = context.read_spline_curve(data, edge, opt.curve_limits)?;
    let interval = interval.unwrap_or_else(|| declaration.curve.domain());
    sample_declaration_interval(context, data, edge, interval, opt, declaration, progress)
}
/// Internal reuse of the source-bound declaration; callers must preserve the
/// same context/edge. Numeric source identity is checked before endpoint work.
pub(crate) fn sample_declaration(
    context: &BodyContext,
    data: &Inventory,
    edge: EntityReference,
    opt: Options,
    declaration: StepSplineDeclaration,
    progress: impl FnMut(nurbs_sample::Progress) -> bool,
) -> Result<SampledSplineEdge, Error> {
    let interval = declaration.curve.domain();
    sample_declaration_interval(context, data, edge, interval, opt, declaration, progress)
}
fn sample_declaration_interval(
    context: &BodyContext,
    data: &Inventory,
    edge: EntityReference,
    interval: [f64; 2],
    opt: Options,
    declaration: StepSplineDeclaration,
    mut progress: impl FnMut(nurbs_sample::Progress) -> bool,
) -> Result<SampledSplineEdge, Error> {
    let entity = data.resolve_reference(edge)?;
    let fail = |detail: &str| Error {
        offset: entity.offset,
        kind: ErrorKind::Geometry,
        detail: format!("spline edge #{}: {detail}", edge.entity),
    };
    if !opt.geometry_tolerance_mm.is_finite()
        || opt.geometry_tolerance_mm <= 0.
        || !opt.sampling.tolerance.is_finite()
        || opt.geometry_tolerance_mm > opt.sampling.tolerance
    {
        return Err(fail(
            "require finite 0 < geometry tolerance <= sampling tolerance",
        ));
    }
    let cancelled = || {
        let mut error = fail("sampling cancelled");
        error.kind = ErrorKind::Cancelled;
        error
    };
    if !progress(nurbs_sample::Progress {
        enclosures: 0,
        accepted_segments: 0,
        pending_intervals: 0,
    }) {
        return Err(cancelled());
    }
    if interval.iter().any(|x| !x.is_finite())
        || interval[0] >= interval[1]
        || interval[0] < declaration.curve.domain()[0]
        || interval[1] > declaration.curve.domain()[1]
    {
        return Err(fail("invalid finite spline edge interval"));
    }
    data.resolve_reference(context.length_context())?;
    let topology = context
        .body()
        .edges
        .get(&edge.entity)
        .ok_or_else(|| fail("source reference is not an edge of this body"))?;
    let geometry = crate::analytic_step::curve_geometry(data, topology.curve)?;
    if data.entity_reference(geometry.id) != Some(declaration.source) {
        return Err(fail("declaration does not belong to this source edge"));
    }
    let vertex_ids = [topology.start, topology.end];
    let vertices = vertex_ids.map(|id| data.entity_reference(id).unwrap());
    let mut endpoints = [[0.; 3]; 2];
    let mut errors = [0.; 2];
    for i in 0..2 {
        let vertex = &context.body().vertices[&vertex_ids[i]];
        endpoints[i] = point(data, vertex.point, context.millimetres_per_source_unit())?;
        let parameter = interval[if topology.same_sense { i } else { 1 - i }];
        let enclosure = declaration.curve.enclose([parameter; 2], 2)?;
        errors[i] = geometry_distance::distance_bounds(enclosure, endpoints[i])?[1];
        if errors[i] > opt.geometry_tolerance_mm {
            return Err(fail("selected-interval endpoint agreement not certified; unsupported trim or inconsistent orientation/geometry"));
        }
    }
    let mut last = None;
    let samples = nurbs_sample::sample_interval(&declaration.curve, interval, opt.sampling, |p| {
        last = Some(p);
        progress(p)
    })
    .map_err(|mut e| {
        if e.offset == 0 {
            e.offset = entity.offset;
        }
        e.detail = format!("spline edge #{}: {}", edge.entity, e.detail);
        e
    })?;
    let result = SampledSplineEdge {
        edge,
        vertices,
        context: context.length_context(),
        source_endpoints_mm: endpoints,
        endpoint_errors_mm: errors,
        same_sense: topology.same_sense,
        declaration,
        samples,
    };
    if !progress(last.unwrap()) {
        return Err(cancelled());
    }
    Ok(result)
}
