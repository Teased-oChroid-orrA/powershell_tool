//! Oriented finite arc selection for a closed single-winding planar spline
//! p-curve on a source circle edge. Decides which of the two arcs between the
//! endpoint parameters is the edge's arc, using certified half-plane sign tests
//! at one interior witness per arc. No master, trim-loop or body certificate.
use crate::{
    analytic_step::{call, curve_geometry},
    geometry_bounds::Bounds3,
    native_interval::Interval as I,
    step::{EntityReference, Error, ErrorKind, Inventory, Value},
    step_body::BodyContext,
    step_curve_agreement::world_point,
    step_frame::{dot, SourceFrame, V},
    step_parameter_map::ParameterMapRef,
    step_pcurve::{self, ParametricCurve},
    step_pcurve_endpoints::{ParameterOrder, Request},
    step_plane_map::PlaneParameterMap,
    step_spline_circle,
};
#[derive(Debug, Clone, Copy)]
pub struct Options {
    /// Closed single-winding proof of the p-curve against the source circle.
    pub circle: step_spline_circle::Options,
    pub max_curve_spans_per_enclosure: usize,
    pub max_surface_patches_per_enclosure: usize,
    /// Witness-point enclosures (two are needed).
    pub max_enclosures: usize,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            circle: step_spline_circle::Options {
                require_circle_coverage: true,
                require_single_winding: true,
                ..step_spline_circle::Options::default()
            },
            max_curve_spans_per_enclosure: 3,
            max_surface_patches_per_enclosure: 64,
            max_enclosures: 2,
        }
    }
}
#[derive(Debug, Clone, Copy)]
pub struct Progress {
    pub enclosures: usize,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arc {
    /// The edge arc is the p-curve parameter interval itself.
    Interior,
    /// The edge arc is the complement through the domain ends.
    Wrapping,
}
#[derive(Debug)]
pub struct Report {
    pub edge: EntityReference,
    pub pcurve: EntityReference,
    pub arc: Arc,
    /// Parameters of the topological start and end vertices on the p-curve.
    pub start_parameter: f64,
    pub end_parameter: f64,
    /// Whether the parameter increases (through the domain end when
    /// `arc == Wrapping`) from the start vertex to the end vertex.
    pub increasing: bool,
    pub witness_parameters: [f64; 2],
}
type P = [I; 2];
fn fail(e: &crate::step::Entity, kind: ErrorKind, m: &str) -> Error {
    Error {
        offset: e.offset,
        kind,
        detail: m.into(),
    }
}
fn cross2(a: P, b: P) -> I {
    a[0] * b[1] - a[1] * b[0]
}
/// `Some(true)` when `m` is certified inside the counter-clockwise arc a→b of
/// a circle, `Some(false)` when certified outside, otherwise `None`.
fn in_ccw_arc(a: P, b: P, m: P) -> Option<bool> {
    let ab = cross2(a, b);
    if ab.lo > 0. {
        let (am, mb) = (cross2(a, m), cross2(m, b));
        if am.lo > 0. && mb.lo > 0. {
            return Some(true);
        }
        if am.hi < 0. || mb.hi < 0. {
            return Some(false);
        }
    } else if ab.hi < 0. {
        // The complement b→a is shorter than half a turn.
        let (bm, ma) = (cross2(b, m), cross2(m, a));
        if bm.lo > 0. && ma.lo > 0. {
            return Some(false);
        }
        if bm.hi < 0. || ma.hi < 0. {
            return Some(true);
        }
    } else {
        // Half-turn arc: endpoints antipodal within a relative 1e-6. The arc is the
        // closed half-plane counter-clockwise of `a`; the witness must be strictly
        // inside either side, so a witness near an endpoint is refused.
        let back = a[0] * b[0] + a[1] * b[1];
        let spread = ab.lo.abs().max(ab.hi.abs());
        if back.hi < 0. && spread <= -back.hi * 1e-6 {
            let am = cross2(a, m);
            if am.lo > 0. {
                return Some(true);
            }
            if am.hi < 0. {
                return Some(false);
            }
        }
    }
    None
}
fn local(frame: &SourceFrame, p: V) -> P {
    let d: V = std::array::from_fn(|i| p[i] - frame.origin[i]);
    [dot(d, frame.x), dot(d, frame.y)]
}
fn interval_point(b: Bounds3) -> V {
    std::array::from_fn(|i| I::new(b.min[i], b.max[i]))
}
/// Select the oriented arc for a candidate `request` produced by endpoint
/// discovery. Requires a planar closed spline p-curve proven single-winding
/// against the edge's source CIRCLE; both witnesses must be certified or the
/// selection is refused.
pub fn select(
    map: &PlaneParameterMap,
    context: &BodyContext,
    data: &Inventory,
    request: Request,
    options: Options,
    mut progress: impl FnMut(Progress) -> bool,
) -> Result<Report, Error> {
    let entity = data.resolve_reference(request.pcurve)?;
    let cancelled = || fail(entity, ErrorKind::Cancelled, "arc selection cancelled");
    let mut enclosures = 0;
    if !progress(Progress { enclosures }) {
        return Err(cancelled());
    }
    if request.interval.iter().any(|x| !x.is_finite()) || request.interval[0] >= request.interval[1]
    {
        return Err(fail(
            entity,
            ErrorKind::Geometry,
            "require an increasing finite arc interval",
        ));
    }
    if !(options.circle.require_circle_coverage && options.circle.require_single_winding) {
        return Err(fail(
            entity,
            ErrorKind::Geometry,
            "arc selection requires the single-winding circle proof",
        ));
    }
    let (edge, declaration, _) = step_pcurve::associated_edge_pcurve_with_limits(
        context,
        data,
        map.face_reference(),
        map.surface_reference(),
        request.edge,
        request.pcurve,
        Default::default(),
    )?;
    let ParametricCurve::Spline(spline) = &declaration.geometry else {
        return Err(fail(
            entity,
            ErrorKind::Geometry,
            "arc selection requires a planar spline p-curve",
        ));
    };
    let domain = spline.curve().domain();
    if request.interval[0] < domain[0] || request.interval[1] > domain[1] {
        return Err(fail(
            entity,
            ErrorKind::Geometry,
            "arc interval outside the active spline domain",
        ));
    }
    if edge.start == edge.end {
        return Err(fail(
            entity,
            ErrorKind::Geometry,
            "closed edges have no endpoint-delimited arc",
        ));
    }
    let proof = map.verify_spline_circle_deviation(
        context,
        data,
        request.edge,
        request.pcurve,
        options.circle,
        |_| true,
    )?;
    if !proof.single_winding {
        return Err(fail(
            entity,
            ErrorKind::Geometry,
            "single winding not certified",
        ));
    }
    let scale = context.millimetres_per_source_unit();
    let geometry = curve_geometry(data, edge.curve)?;
    let placement = match call(geometry, "CIRCLE")? {
        [Value::String(_), Value::Ref(frame), _] => &data.entities()[frame],
        _ => {
            return Err(fail(
                geometry,
                ErrorKind::Geometry,
                "requires a source CIRCLE",
            ))
        }
    };
    let frame = SourceFrame::read(data, placement, scale)?;
    let vertex = |id: u64| -> Result<P, Error> {
        let v = &context.body().vertices[&id];
        Ok(local(&frame, world_point(data, v.point, scale)?))
    };
    let (s, e) = (vertex(edge.start)?, vertex(edge.end)?);
    // EDGE_CURVE.same_sense: start→end follows the counter-clockwise parameter.
    let (from, to) = if edge.same_sense { (s, e) } else { (e, s) };
    let mut witness = |t: f64, enclosures: &mut usize| -> Result<Option<bool>, Error> {
        if *enclosures >= options.max_enclosures {
            return Err(fail(
                entity,
                ErrorKind::Limit,
                "arc witness budget exceeded",
            ));
        }
        let b = ParameterMapRef::from(map).bounds_for_declaration(
            data,
            &declaration,
            [t; 2],
            options.max_curve_spans_per_enclosure,
            options.max_surface_patches_per_enclosure,
        )?;
        *enclosures += 1;
        if !progress(Progress {
            enclosures: *enclosures,
        }) {
            return Err(cancelled());
        }
        Ok(in_ccw_arc(from, to, local(&frame, interval_point(b))))
    };
    let [lo, hi] = request.interval;
    let inner_t = lo + (hi - lo) * 0.5;
    let outer_t = if hi < domain[1] {
        hi + (domain[1] - hi) * 0.5
    } else if domain[0] < lo {
        domain[0] + (lo - domain[0]) * 0.5
    } else {
        return Err(fail(
            entity,
            ErrorKind::Geometry,
            "interval covers the whole domain; no complementary arc",
        ));
    };
    let inner = witness(inner_t, &mut enclosures)?;
    let outer = witness(outer_t, &mut enclosures)?;
    let arc = match (inner, outer) {
        (Some(true), Some(false)) => Arc::Interior,
        (Some(false), Some(true)) => Arc::Wrapping,
        _ => {
            return Err(fail(
                entity,
                ErrorKind::Geometry,
                "arc witnesses not certified on exactly one side",
            ))
        }
    };
    let (start_parameter, end_parameter) = if request.order == ParameterOrder::Forward {
        (lo, hi)
    } else {
        (hi, lo)
    };
    let increasing = (request.order == ParameterOrder::Forward) == (arc == Arc::Interior);
    Ok(Report {
        edge: request.edge,
        pcurve: request.pcurve,
        arc,
        start_parameter,
        end_parameter,
        increasing,
        witness_parameters: [inner_t, outer_t],
    })
}
