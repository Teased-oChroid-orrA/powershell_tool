//! Bounded endpoint-parameter candidate discovery for a source p-curve, rechecked
//! by `step_pcurve_endpoints`. A candidate is a proposal only: no unique
//! parameter, oriented arc, winding, wrap, master or B-Rep validity claim.
use crate::{
    step::{EntityReference, Error, ErrorKind, Inventory},
    step_body::BodyContext,
    step_curve_agreement::world_point,
    step_parameter_map::ParameterMapRef,
    step_pcurve,
    step_pcurve_distance::{self, SearchOptions},
    step_pcurve_endpoints::{self, ParameterOrder, Request},
};
#[derive(Debug, Clone, Copy)]
pub struct Options {
    /// Per-vertex distance search; its `max_enclosures` is a cumulative cap over
    /// both vertex searches.
    pub search: step_pcurve_distance::Options,
    /// Recheck of the proposed interval; independent enclosure budget.
    pub endpoints: step_pcurve_endpoints::Options,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            search: step_pcurve_distance::Options {
                search: SearchOptions {
                    distance_tolerance: 1e-9,
                    ..SearchOptions::default()
                },
                ..step_pcurve_distance::Options::default()
            },
            endpoints: step_pcurve_endpoints::Options::default(),
        }
    }
}
/// Source edge, its p-curve and the finite interval to search.
#[derive(Debug, Clone, Copy)]
pub struct Search {
    pub edge: EntityReference,
    pub pcurve: EntityReference,
    pub domain: [f64; 2],
}
#[derive(Debug, Clone, Copy)]
pub struct Progress {
    /// Cumulative distance-search enclosures over completed and running searches.
    pub search_enclosures: usize,
    pub searched_vertices: usize,
    /// Recheck enclosures, zero until the endpoint recheck starts.
    pub recheck_enclosures: usize,
}
/// Candidate for one source topological vertex, in start/end order.
#[derive(Debug, Clone, Copy)]
pub struct Candidate {
    pub vertex: EntityReference,
    /// Raw p-curve parameter whose trace point is within the search gap of the
    /// minimum distance. Uniqueness and parameter error are not bounded.
    pub parameter: f64,
    pub distance_bounds_mm: [f64; 2],
}
#[derive(Debug)]
pub enum Proposal {
    /// The edge has one topological vertex, or both searches returned the same
    /// parameter. No empty or inferred arc is constructed.
    Coincident,
    /// Sorted interval and the order mapping it to topological start/end.
    Interval {
        request: Request,
        /// `Err` only for an ordinary geometric refusal of the proposal.
        recheck: Box<Result<step_pcurve_endpoints::Report, Error>>,
    },
}
#[derive(Debug)]
pub struct Report {
    pub face: EntityReference,
    pub surface: EntityReference,
    pub edge: EntityReference,
    pub pcurve: EntityReference,
    pub domain: [f64; 2],
    pub candidates: [Candidate; 2],
    pub proposal: Proposal,
    pub search_enclosures: usize,
}
/// Search `domain` (a finite caller-selected interval) for the parameter nearest
/// each source vertex of `edge`, then recheck the sorted pair with the endpoint
/// verifier. Source association is exact-document; vertex points are converted
/// to mm with the body's unit scale and queried at the outward box midpoint, so
/// only the recheck (not the search) carries the outward uncertainty.
pub fn discover<'a>(
    map: impl Into<ParameterMapRef<'a>>,
    context: &BodyContext,
    data: &Inventory,
    search: Search,
    options: Options,
    mut progress: impl FnMut(Progress) -> bool,
) -> Result<Report, Error> {
    let map = map.into();
    let Search {
        edge,
        pcurve,
        domain,
    } = search;
    let entity = data.resolve_reference(pcurve)?;
    let fail = |kind, message: &str| Error {
        offset: entity.offset,
        kind,
        detail: message.into(),
    };
    let cancelled = || {
        fail(
            ErrorKind::Cancelled,
            "p-curve candidate discovery cancelled",
        )
    };
    let mut state = Progress {
        search_enclosures: 0,
        searched_vertices: 0,
        recheck_enclosures: 0,
    };
    if !progress(state) {
        return Err(cancelled());
    }
    if domain.iter().any(|x| !x.is_finite()) || domain[0] >= domain[1] {
        return Err(fail(
            ErrorKind::Geometry,
            "require an increasing finite candidate search domain",
        ));
    }
    data.resolve_reference(map.length_context())?;
    if map.length_context() != context.length_context() {
        return Err(fail(
            ErrorKind::Geometry,
            "map and source body unit context differ",
        ));
    }
    let (raw_edge, _, _) = step_pcurve::associated_edge_pcurve_with_limits(
        context,
        data,
        map.face_reference(),
        map.surface_reference(),
        edge,
        pcurve,
        options.search.curve_limits,
    )?;
    let ids = [raw_edge.start, raw_edge.end];
    let mut candidates = [None; 2];
    for (i, id) in ids.iter().enumerate() {
        let vertex = &context.body().vertices[id];
        let p = world_point(data, vertex.point, context.millimetres_per_source_unit())?;
        let query = p.map(|x| x.lo + (x.hi - x.lo) * 0.5);
        let used = state.search_enclosures;
        let mut search = options.search;
        search.search.max_enclosures = search.search.max_enclosures.saturating_sub(used);
        let mut stopped = false;
        let r = step_pcurve_distance::distance(map, data, pcurve, query, domain, search, |p| {
            let live = Progress {
                search_enclosures: used + p.enclosures,
                ..state
            };
            stopped = !progress(live);
            !stopped
        });
        let r = match r {
            Err(e) if stopped => {
                debug_assert_eq!(e.kind, ErrorKind::Cancelled);
                return Err(cancelled());
            }
            r => r?,
        };
        state.search_enclosures = used + r.enclosures;
        state.searched_vertices += 1;
        if !progress(state) {
            return Err(cancelled());
        }
        candidates[i] = Some(Candidate {
            vertex: data.entity_reference(*id).unwrap(),
            parameter: r.candidate_parameter,
            distance_bounds_mm: r.distance_bounds_mm,
        });
    }
    let candidates = candidates.map(Option::unwrap);
    let [a, b] = candidates.map(|c| c.parameter);
    let proposal = if ids[0] == ids[1] || a == b {
        Proposal::Coincident
    } else {
        let request = Request {
            edge,
            pcurve,
            interval: [a.min(b), a.max(b)],
            order: if a < b {
                ParameterOrder::Forward
            } else {
                ParameterOrder::Reversed
            },
        };
        let mut stopped = false;
        let recheck =
            step_pcurve_endpoints::verify(map, context, data, request, options.endpoints, |p| {
                state.recheck_enclosures = p.enclosures;
                stopped = !progress(state);
                !stopped
            });
        match recheck {
            Err(_) if stopped => return Err(cancelled()),
            Err(e) if matches!(e.kind, ErrorKind::Limit | ErrorKind::Cancelled) => return Err(e),
            recheck => Proposal::Interval {
                request,
                recheck: Box::new(recheck),
            },
        }
    };
    if !progress(state) {
        return Err(cancelled());
    }
    Ok(Report {
        face: map.face_reference(),
        surface: map.surface_reference(),
        edge,
        pcurve,
        domain,
        candidates,
        proposal,
        search_enclosures: state.search_enclosures,
    })
}
