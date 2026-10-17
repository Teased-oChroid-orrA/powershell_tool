use solid_kernel::{
    nurbs,
    step::{inventory, ErrorKind, Limits},
    step_body::BodyContext,
    step_parameter_map::ParameterMapRef,
    step_pcurve::{self, ParametricCurve},
    step_pcurve_candidates::{discover, Options, Proposal, Search},
    step_plane_map::PlaneParameterMap,
};
const SOURCE: &str = include_str!("../../nema17-motor-bracket.step");

/// Every planar spline p-curve of the bracket: (map, edge, pcurve, active domain).
fn spline_pcurves(
    data: &solid_kernel::step::Inventory,
    context: &BodyContext,
) -> Vec<(
    PlaneParameterMap,
    [solid_kernel::step::EntityReference; 2],
    [f64; 2],
)> {
    let mut out = Vec::new();
    for face in context.body().faces.values() {
        if data.entities()[&face.surface].components[0].name != "PLANE" {
            continue;
        }
        let map = PlaneParameterMap::read(context, data, data.entity_reference(face.id).unwrap())
            .unwrap();
        let mut seen = std::collections::BTreeSet::new();
        for use_ in face.bounds.iter().flat_map(|b| &b.edges) {
            let edge = &context.body().edges[&use_.edge];
            let wrapper =
                step_pcurve::read_surface_curve(data, data.entity_reference(edge.curve).unwrap())
                    .unwrap();
            for pc in wrapper.pcurves {
                let d = step_pcurve::read_pcurve(data, pc, nurbs::Limits::default()).unwrap();
                if d.basis_surface != ParameterMapRef::from(&map).surface_reference()
                    || !seen.insert((edge.id, pc.entity))
                {
                    continue;
                }
                if let ParametricCurve::Spline(s) = &d.geometry {
                    out.push((
                        PlaneParameterMap::read(
                            context,
                            data,
                            data.entity_reference(face.id).unwrap(),
                        )
                        .unwrap(),
                        [data.entity_reference(edge.id).unwrap(), pc],
                        s.curve().domain(),
                    ));
                }
            }
        }
    }
    out
}

#[test]
fn bracket_spline_pcurves_discover_endpoints_for_open_edges_and_refuse_closed_arcs() {
    let data = inventory(SOURCE, Limits::default()).unwrap();
    let context = BodyContext::read(&data).unwrap();
    let all = spline_pcurves(&data, &context);
    assert_eq!(all.len(), 15);
    let (mut open, mut closed) = (0, 0);
    for (map, [edge, pcurve], domain) in &all {
        let mut last = (0, 0);
        let r = discover(
            map,
            &context,
            &data,
            Search {
                edge: *edge,
                pcurve: *pcurve,
                domain: *domain,
            },
            Options::default(),
            |p| {
                assert!(p.search_enclosures >= last.0 && p.recheck_enclosures >= last.1);
                last = (p.search_enclosures, p.recheck_enclosures);
                true
            },
        )
        .unwrap();
        assert!(r.candidates.iter().all(|c| c.distance_bounds_mm[1] < 1e-7));
        assert_eq!(last.0, r.search_enclosures);
        match r.proposal {
            Proposal::Coincident => {
                assert_eq!(r.candidates[0].parameter, r.candidates[1].parameter);
                closed += 1;
            }
            Proposal::Interval { request, recheck } => {
                let rc = (*recheck).unwrap();
                assert!(rc.endpoint_error_upper_mm.iter().all(|e| *e <= 1e-7));
                assert!(request.interval[0] < request.interval[1]);
                assert!(request.interval[0] >= domain[0] && request.interval[1] <= domain[1]);
                open += 1;
            }
        }
    }
    assert_eq!((open, closed), (10, 5));
}

#[test]
fn discovery_budgets_cancellation_domains_and_stale_sources_refuse() {
    let data = inventory(SOURCE, Limits::default()).unwrap();
    let context = BodyContext::read(&data).unwrap();
    let all = spline_pcurves(&data, &context);
    let (map, [edge, pcurve], domain) = &all[0];
    let run = |d, o, f: &mut dyn FnMut(_) -> bool| {
        discover(
            map,
            &context,
            &data,
            Search {
                edge: *edge,
                pcurve: *pcurve,
                domain: d,
            },
            o,
            f,
        )
    };
    let full = run(*domain, Options::default(), &mut |_| true).unwrap();
    // cumulative search cap: one enclosure fewer than needed refuses
    let mut tight = Options::default();
    tight.search.search.max_enclosures = full.search_enclosures - 1;
    assert_eq!(
        run(*domain, tight, &mut |_| true).unwrap_err().kind,
        ErrorKind::Limit
    );
    // a recheck budget below the verifier's three enclosures is a Limit, not a refusal
    if matches!(full.proposal, Proposal::Interval { .. }) {
        let mut o = Options::default();
        o.endpoints.max_enclosures = 2;
        assert_eq!(
            run(*domain, o, &mut |_| true).unwrap_err().kind,
            ErrorKind::Limit
        );
    }
    // cancellation at the start, mid-search and at the end
    assert_eq!(
        run(*domain, Options::default(), &mut |_| false)
            .unwrap_err()
            .kind,
        ErrorKind::Cancelled
    );
    let mut calls = 0;
    assert_eq!(
        run(*domain, Options::default(), &mut |_| {
            calls += 1;
            calls < 3
        })
        .unwrap_err()
        .kind,
        ErrorKind::Cancelled
    );
    let mut calls = 0;
    let total = {
        run(*domain, Options::default(), &mut |_| {
            calls += 1;
            true
        })
        .unwrap();
        calls
    };
    let mut n = 0;
    assert_eq!(
        run(*domain, Options::default(), &mut |_| {
            n += 1;
            n < total
        })
        .unwrap_err()
        .kind,
        ErrorKind::Cancelled
    );
    for bad in [[0., 0.], [domain[1], domain[0]], [f64::NAN, 1.]] {
        assert_eq!(
            run(bad, Options::default(), &mut |_| true)
                .unwrap_err()
                .kind,
            ErrorKind::Geometry
        );
    }
    // search domain outside the active spline domain
    assert!(
        run([domain[0] - 1., domain[1]], Options::default(), &mut |_| {
            true
        })
        .is_err()
    );
    let stale = inventory(&format!("{SOURCE}\n"), Limits::default()).unwrap();
    assert!(discover(
        map,
        &context,
        &stale,
        Search {
            edge: *edge,
            pcurve: *pcurve,
            domain: *domain,
        },
        Options::default(),
        |_| true
    )
    .is_err());
}
