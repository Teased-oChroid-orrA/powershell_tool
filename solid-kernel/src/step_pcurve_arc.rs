//! Oriented finite arc selection for a source circle edge whose p-curve is a
//! closed single-winding planar spline, a plane parameter CIRCLE or a cylinder
//! constant-V parameter LINE (one period per turn, exact by construction). Decides which of the two arcs between the
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
    step_pcurve_endpoints::{self, ParameterOrder, Request},
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
pub fn select<'a>(
    map: impl Into<ParameterMapRef<'a>>,
    context: &BodyContext,
    data: &Inventory,
    request: Request,
    options: Options,
    mut progress: impl FnMut(Progress) -> bool,
) -> Result<Report, Error> {
    let map = map.into();
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
    let curve = Carrier::of(map, data, &declaration)?;
    let [w0, w1] = curve.window;
    if request.interval[0] < w0 || request.interval[1] > w1 {
        return Err(fail(
            entity,
            ErrorKind::Geometry,
            "arc interval outside the carrier's single-turn parameter window",
        ));
    }
    if edge.start == edge.end {
        return Err(fail(
            entity,
            ErrorKind::Geometry,
            "closed edges have no endpoint-delimited arc",
        ));
    }
    if let ParameterMapRef::Plane(plane) = map {
        if curve.spline {
            let proof = plane.verify_spline_circle_deviation(
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
        }
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
        let b = map.bounds_for_declaration(
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
    let (outer_t, outer) = match curve.period {
        None => {
            let t = if hi < w1 {
                hi + (w1 - hi) * 0.5
            } else if w0 < lo {
                w0 + (lo - w0) * 0.5
            } else {
                return Err(fail(
                    entity,
                    ErrorKind::Geometry,
                    "interval covers the whole domain; no complementary arc",
                ));
            };
            (t, None)
        }
        Some(period) => {
            let t = hi + (lo + period - hi) * 0.5;
            (t, Some(period))
        }
    };
    let inner = witness(inner_t, &mut enclosures)?;
    let outer_side = match witness(outer_t, &mut enclosures) {
        // The unwrapped complement can leave the map's raw angular range; the
        // same arc is then witnessed one period lower.
        Err(e) if outer.is_some() && e.kind == ErrorKind::Geometry => {
            witness(outer_t - outer.unwrap(), &mut enclosures)?
        }
        other => other?,
    };
    let arc = match (inner, outer_side) {
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
    let forward = request.order == ParameterOrder::Forward;
    let start = if forward { lo } else { hi };
    let (end, increasing) = match (outer, arc) {
        (_, Arc::Interior) => (if forward { hi } else { lo }, forward),
        // Spline: the arc runs through the domain end; report the vertex parameter.
        (None, Arc::Wrapping) => (if forward { hi } else { lo }, !forward),
        // Periodic carrier: the unwrapped end lies one period away.
        (Some(period), Arc::Wrapping) => {
            (if forward { hi - period } else { lo + period }, !forward)
        }
    };
    let (mut start_parameter, mut end_parameter) = (start, end);
    if let (Some(period), Arc::Wrapping) = (outer, arc) {
        // The unwrapped complement is a different parameter interval: check its
        // endpoints against both source vertices too. It may leave the map's raw
        // angular range, in which case the same arc one period over is used.
        let mut last = None;
        for shift in [0., period, -period] {
            let (a, b) = (start + shift, end + shift);
            let endpoints = Request {
                interval: [a.min(b), a.max(b)],
                order: if b > a {
                    ParameterOrder::Forward
                } else {
                    ParameterOrder::Reversed
                },
                ..request
            };
            match step_pcurve_endpoints::verify(
                map,
                context,
                data,
                endpoints,
                step_pcurve_endpoints::Options {
                    tolerance_mm: options.circle.tolerance_mm,
                    ..Default::default()
                },
                |_| true,
            ) {
                Ok(_) => {
                    (start_parameter, end_parameter) = (a, b);
                    last = None;
                    break;
                }
                Err(e) if e.kind == ErrorKind::Geometry => last = Some(e),
                Err(e) => return Err(e),
            }
        }
        if let Some(e) = last {
            return Err(e);
        }
    }
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

#[derive(Debug)]
pub enum Resolved {
    /// One topological vertex: the p-curve is the whole closed trace.
    Closed,
    Arc(Report),
    /// Straight p-curve image (plane LINE or cylinder generator): finite
    /// interval and order whose endpoints agree with both source vertices.
    Segment(Box<step_pcurve_endpoints::Report>),
}
/// Discover the endpoint parameters of `pcurve` over its whole active domain,
/// recheck them against both source vertices and select the oriented arc.
/// Any uncertified step is an error; nothing partial is returned.
pub fn resolve<'a>(
    map: impl Into<ParameterMapRef<'a>>,
    context: &BodyContext,
    data: &Inventory,
    edge: EntityReference,
    pcurve: EntityReference,
    tolerance_mm: f64,
    mut progress: impl FnMut() -> bool,
) -> Result<Resolved, Error> {
    use crate::step_pcurve_candidates::{discover, Options as Discover, Proposal, Search};
    let map = map.into();
    let declaration = step_pcurve::read_pcurve(data, pcurve, Default::default())?;
    // Plane LINEs and cylinder generators (constant raw U) image to straight
    // world segments; the constant-V cylinder LINE is a circle (periodic arc).
    let straight = match (&declaration.geometry, map) {
        (ParametricCurve::Line { .. }, ParameterMapRef::Cylinder(_)) => {
            step_pcurve::line_direction_ratios(data, declaration.geometry_source)?[0] == 0.
        }
        (ParametricCurve::Line { .. }, _) => true,
        _ => false,
    };
    if straight {
        return segment(map, context, data, edge, pcurve, &declaration, tolerance_mm)
            .map(|r| Resolved::Segment(Box::new(r)));
    }
    let window = Carrier::of(map, data, &declaration)?.window;
    let mut options = Discover::default();
    options.endpoints.tolerance_mm = tolerance_mm;
    let found = discover(
        map,
        context,
        data,
        Search {
            edge,
            pcurve,
            domain: window,
        },
        options,
        |_| progress(),
    )?;
    match found.proposal {
        Proposal::Coincident => Ok(Resolved::Closed),
        Proposal::Interval { request, recheck } => {
            (*recheck)?;
            let mut arc = Options::default();
            arc.circle.tolerance_mm = tolerance_mm;
            select(map, context, data, request, arc, |_| progress()).map(Resolved::Arc)
        }
    }
}

/// How a p-curve maps one turn of the source circle.
struct Carrier {
    /// Parameter window covering exactly one turn.
    window: [f64; 2],
    /// Parameter period of an exactly periodic carrier; `None` for splines.
    period: Option<f64>,
    spline: bool,
}
impl Carrier {
    fn of(
        map: ParameterMapRef,
        data: &Inventory,
        declaration: &step_pcurve::PcurveDeclaration,
    ) -> Result<Self, Error> {
        let entity = data.resolve_reference(declaration.source)?;
        let turn = std::f64::consts::TAU;
        match (&declaration.geometry, map) {
            (ParametricCurve::Spline(spline), ParameterMapRef::Plane(_)) => Ok(Self {
                window: spline.curve().domain(),
                period: None,
                spline: true,
            }),
            (ParametricCurve::Circle { .. }, ParameterMapRef::Plane(_)) => Ok(Self {
                window: [-turn / 2., turn / 2.],
                period: Some(turn),
                spline: false,
            }),
            (ParametricCurve::Line { magnitude, .. }, ParameterMapRef::Cylinder(_)) => {
                let ratios = step_pcurve::line_direction_ratios(data, declaration.geometry_source)?;
                if ratios[1] != 0. || ratios[0] == 0. || !magnitude.is_finite() || *magnitude == 0.
                {
                    return Err(fail(
                        entity,
                        ErrorKind::Geometry,
                        "cylinder arc requires a constant-V parameter LINE",
                    ));
                }
                // Raw U is the angle in radians; one turn spans 2π/|magnitude| in t.
                let period = turn / magnitude.abs();
                Ok(Self {
                    window: [-period / 2., period / 2.],
                    period: Some(period),
                    spline: false,
                })
            }
            _ => Err(fail(
                entity,
                ErrorKind::Geometry,
                "arc selection supports plane spline/circle and cylinder circle p-curves",
            )),
        }
    }
}

/// World-straight p-curve image: solve each vertex's line parameter by
/// projecting its source point onto the mapped line (binary64), then let the
/// endpoint verifier certify the proposal with outward rounding. The projection
/// is only a proposal; the certificate is the verifier's.
fn segment(
    map: ParameterMapRef,
    context: &BodyContext,
    data: &Inventory,
    edge: EntityReference,
    pcurve: EntityReference,
    declaration: &step_pcurve::PcurveDeclaration,
    tolerance_mm: f64,
) -> Result<step_pcurve_endpoints::Report, Error> {
    let entity = data.resolve_reference(pcurve)?;
    let (raw, _, _) = step_pcurve::associated_edge_pcurve_with_limits(
        context,
        data,
        map.face_reference(),
        map.surface_reference(),
        edge,
        pcurve,
        Default::default(),
    )?;
    let mid = |t: f64| -> Result<[f64; 3], Error> {
        let b = map.bounds_for_declaration(data, declaration, [t; 2], 3, 64)?;
        Ok(std::array::from_fn(|i| {
            b.min[i] + (b.max[i] - b.min[i]) * 0.5
        }))
    };
    let (a, b) = (mid(0.)?, mid(1.)?);
    let delta: [f64; 3] = std::array::from_fn(|i| b[i] - a[i]);
    let len2: f64 = delta.iter().map(|x| x * x).sum();
    if !(len2.is_finite() && len2 > 0.) {
        return Err(fail(
            entity,
            ErrorKind::Geometry,
            "line p-curve has a degenerate world image",
        ));
    }
    let scale = context.millimetres_per_source_unit();
    let parameter = |id: u64| {
        let p = context.body().vertices[&id].coordinates;
        (0..3)
            .map(|i| (p[i] * scale - a[i]) * delta[i])
            .sum::<f64>()
            / len2
    };
    let (s, e) = (parameter(raw.start), parameter(raw.end));
    if !(s.is_finite() && e.is_finite()) || s == e {
        return Err(fail(
            entity,
            ErrorKind::Geometry,
            "line endpoint parameters are not distinct",
        ));
    }
    step_pcurve_endpoints::verify(
        map,
        context,
        data,
        Request {
            edge,
            pcurve,
            interval: [s.min(e), s.max(e)],
            order: if s < e {
                ParameterOrder::Forward
            } else {
                ParameterOrder::Reversed
            },
        },
        step_pcurve_endpoints::Options {
            tolerance_mm,
            ..Default::default()
        },
        |_| true,
    )
}
