//! Bounded whole-body diagnostic of supported p-curve loci. This is not finite
//! arc/master/trim/body validity or a production-backend admission certificate.
use crate::{
    step::{EntityReference, Error, ErrorKind, Inventory},
    step_body::BodyContext,
    step_curve_agreement::{CircleCarrier, LineLocus},
    step_cylinder_map::CylinderParameterMap,
    step_pcurve::{self, ParametricCurve},
    step_plane_map::PlaneParameterMap,
    step_spline_circle::{self, SplineCircleDeviation},
};
use std::collections::{BTreeMap, BTreeSet};
#[derive(Debug, Clone, Copy)]
pub struct Options {
    pub tolerance_mm: f64,
    pub max_edge_uses: usize,
    pub max_associations: usize,
    pub max_spline_spans: usize,
    pub max_curve_controls: usize,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            tolerance_mm: 1e-7,
            max_edge_uses: 200_000,
            max_associations: 4096,
            max_spline_spans: 4096,
            max_curve_controls: 32768,
        }
    }
}
#[derive(Debug, Clone, Copy)]
pub struct Progress {
    pub edge_uses_seen: usize,
    pub associations_done: usize,
    /// None during bounded source discovery, then a stable total.
    pub total_associations: Option<usize>,
    pub spline_spans_done: usize,
    pub curve_controls_done: usize,
}
#[derive(Debug)]
pub enum LocusCheck {
    FiniteLine(LineLocus),
    UnsignedCircle(CircleCarrier),
    ClosedQuadraticCircle(SplineCircleDeviation),
}
#[derive(Debug)]
pub struct Report {
    pub source_body: EntityReference,
    pub edge_uses_seen: usize,
    pub spline_spans: usize,
    pub curve_controls: usize,
    pub checks: Vec<LocusCheck>,
}
enum Map {
    Plane(PlaneParameterMap),
    Cylinder(CylinderParameterMap),
}
fn error(offset: usize, kind: ErrorKind, message: &str) -> Error {
    Error {
        offset,
        kind,
        detail: message.into(),
    }
}
fn publish(progress: &mut impl FnMut(Progress) -> bool, state: Progress) -> Result<(), Error> {
    if progress(state) {
        Ok(())
    } else {
        Err(error(
            0,
            ErrorKind::Cancelled,
            "body p-curve audit cancelled",
        ))
    }
}
/// Refuse unsupported faces, absent face p-curves or unsupported carriers rather
/// than silently returning a successful partial audit. All results are atomic.
/// Limits cover repeated edge uses, distinct face/edge/p-curve associations and
/// cumulative spline spans; lower-level declaration/geometry caps also apply.
pub fn audit(
    context: &BodyContext,
    data: &Inventory,
    options: Options,
    mut progress: impl FnMut(Progress) -> bool,
) -> Result<Report, Error> {
    data.resolve_reference(context.length_context())?;
    if !options.tolerance_mm.is_finite() || options.tolerance_mm <= 0. {
        return Err(error(
            0,
            ErrorKind::Geometry,
            "body audit tolerance must be finite and positive",
        ));
    }
    let mut state = Progress {
        edge_uses_seen: 0,
        associations_done: 0,
        total_associations: None,
        spline_spans_done: 0,
        curve_controls_done: 0,
    };
    publish(&mut progress, state)?;
    let mut maps = BTreeMap::new();
    let mut associations = BTreeSet::new();
    for face in context.body().faces.values() {
        let source = &data.entities()[&face.surface];
        let reference = data.entity_reference(face.id).unwrap();
        let map = if source.components.iter().any(|c| c.name == "PLANE") {
            Map::Plane(PlaneParameterMap::read(context, data, reference)?)
        } else if source
            .components
            .iter()
            .any(|c| c.name == "CYLINDRICAL_SURFACE")
        {
            Map::Cylinder(CylinderParameterMap::read(context, data, reference)?)
        } else {
            return Err(error(
                source.offset,
                ErrorKind::Geometry,
                "body p-curve audit supports actual planes/cylinders only",
            ));
        };
        for usage in face.bounds.iter().flat_map(|b| &b.edges) {
            if state.edge_uses_seen >= options.max_edge_uses {
                return Err(error(
                    source.offset,
                    ErrorKind::Limit,
                    "body p-curve edge-use budget exceeded",
                ));
            }
            state.edge_uses_seen += 1;
            publish(&mut progress, state)?;
            let edge = &context.body().edges[&usage.edge];
            let mut current = edge.curve;
            let mut seen = BTreeSet::new();
            let mut found = false;
            loop {
                let record = &data.entities()[&current];
                if !record
                    .components
                    .iter()
                    .any(|c| matches!(c.name.as_str(), "SURFACE_CURVE" | "SEAM_CURVE"))
                {
                    break;
                }
                if seen.len() >= 32 || !seen.insert(current) {
                    return Err(error(
                        record.offset,
                        ErrorKind::Geometry,
                        "cyclic/excessively deep p-curve wrapper audit",
                    ));
                }
                let wrapper =
                    step_pcurve::read_surface_curve(data, data.entity_reference(current).unwrap())?;
                current = wrapper.geometry_3d.entity;
                for pc in wrapper.pcurves {
                    if step_pcurve::basis_surface(data, pc)?.entity != face.surface {
                        continue;
                    }
                    found = true;
                    let key = (face.id, edge.id, pc.entity);
                    if !associations.contains(&key) {
                        if associations.len() >= options.max_associations {
                            return Err(error(
                                record.offset,
                                ErrorKind::Limit,
                                "body p-curve association budget exceeded",
                            ));
                        }
                        associations.insert(key);
                    }
                }
            }
            if !found {
                return Err(error(
                    data.entities()[&edge.id].offset,
                    ErrorKind::Geometry,
                    "body face edge has no associated source p-curve; audit incomplete",
                ));
            }
        }
        maps.insert(face.id, map);
    }
    state.total_associations = Some(associations.len());
    publish(&mut progress, state)?;
    let mut checks = Vec::with_capacity(associations.len());
    for (face, edge, pc) in associations {
        let edge = data.entity_reference(edge).unwrap();
        let pc = data.entity_reference(pc).unwrap();
        let mut limits = crate::nurbs::Limits::default();
        limits.control_points = limits
            .control_points
            .min(options.max_curve_controls - state.curve_controls_done);
        let declaration = step_pcurve::read_pcurve(data, pc, limits)?;
        if let ParametricCurve::Spline(s) = &declaration.geometry {
            state.curve_controls_done += s.curve().control_points().len();
        }

        let check = match (&maps[&face], declaration.geometry) {
            (Map::Plane(map), ParametricCurve::Line { .. }) => LocusCheck::FiniteLine(
                map.verify_line_locus(context, data, edge, pc, options.tolerance_mm)?,
            ),
            (Map::Plane(map), ParametricCurve::Circle { .. }) => LocusCheck::UnsignedCircle(
                map.verify_circle_carrier(context, data, edge, pc, options.tolerance_mm)?,
            ),
            (Map::Plane(map), ParametricCurve::Spline(_)) => {
                let base = state.spline_spans_done;
                let remaining = options.max_spline_spans - base;
                let report = map.verify_spline_circle_deviation(
                    context,
                    data,
                    edge,
                    pc,
                    step_spline_circle::Options {
                        tolerance_mm: options.tolerance_mm,
                        max_spans: remaining,
                        require_circle_coverage: true,
                    },
                    |p| {
                        let mut nested = state;
                        nested.spline_spans_done = base + p.spans;
                        progress(nested)
                    },
                )?;
                state.spline_spans_done += report.spans;
                LocusCheck::ClosedQuadraticCircle(report)
            }
            (Map::Cylinder(map), ParametricCurve::Line { .. }) => {
                let raw = step_pcurve::line_direction_ratios(data, declaration.geometry_source)?;
                if raw[0] == 0. {
                    LocusCheck::FiniteLine(map.verify_generator_locus(
                        context,
                        data,
                        edge,
                        pc,
                        options.tolerance_mm,
                    )?)
                } else if raw[1] == 0. {
                    LocusCheck::UnsignedCircle(map.verify_circle_carrier(
                        context,
                        data,
                        edge,
                        pc,
                        options.tolerance_mm,
                    )?)
                } else {
                    return Err(error(
                        data.resolve_reference(pc)?.offset,
                        ErrorKind::Geometry,
                        "body audit does not certify sloped cylinder p-curves",
                    ));
                }
            }
            _ => {
                return Err(error(
                    data.resolve_reference(pc)?.offset,
                    ErrorKind::Geometry,
                    "unsupported body p-curve carrier",
                ))
            }
        };
        checks.push(check);
        state.associations_done = checks.len();
        publish(&mut progress, state)?;
    }
    Ok(Report {
        source_body: data.entity_reference(context.body().id).unwrap(),
        edge_uses_seen: state.edge_uses_seen,
        spline_spans: state.spline_spans_done,
        curve_controls: state.curve_controls_done,
        checks,
    })
}
