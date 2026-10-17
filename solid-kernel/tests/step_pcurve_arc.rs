mod common;
use common::{spline_pcurves, SOURCE};
use solid_kernel::{
    nurbs,
    step::{inventory, ErrorKind, Limits, Value},
    step_body::BodyContext,
    step_pcurve::{self, ParametricCurve},
    step_pcurve_arc::{select, Arc, Options},
    step_pcurve_candidates::{discover, Proposal, Search},
    step_pcurve_endpoints::ParameterOrder,
};

type V3 = [f64; 3];
fn sub(a: V3, b: V3) -> V3 {
    std::array::from_fn(|i| a[i] - b[i])
}
fn dot(a: V3, b: V3) -> f64 {
    (0..3).map(|i| a[i] * b[i]).sum()
}
fn cross(a: V3, b: V3) -> V3 {
    std::array::from_fn(|i| a[(i + 1) % 3] * b[(i + 2) % 3] - a[(i + 2) % 3] * b[(i + 1) % 3])
}
fn unit(a: V3) -> V3 {
    let n = dot(a, a).sqrt();
    a.map(|x| x / n)
}
fn reals(v: &Value) -> V3 {
    match v {
        Value::List(l) => std::array::from_fn(|i| match l[i] {
            Value::Real(x) => x,
            Value::Integer(x) => x as f64,
            _ => panic!("coordinate"),
        }),
        _ => panic!("vector"),
    }
}
/// Circle frame (origin, x, y) in source units, parsed independently of the kernel.
fn frame(data: &solid_kernel::step::Inventory, curve: u64) -> [V3; 3] {
    let mut id = curve;
    loop {
        let e = &data.entities()[&id];
        let c = &e.components[0];
        match c.name.as_str() {
            "CIRCLE" => {
                let Value::Ref(f) = c.args[1] else { panic!() };
                let f = &data.entities()[&f].components[0];
                let point = |r: &Value| {
                    let Value::Ref(r) = r else { panic!() };
                    reals(&data.entities()[r].components[0].args[1])
                };
                let (o, z, xr) = (
                    point(&f.args[1]),
                    unit(point(&f.args[2])),
                    point(&f.args[3]),
                );
                let y = unit(cross(z, xr));
                return [o, unit(cross(y, z)), y];
            }
            "SURFACE_CURVE" | "SEAM_CURVE" => {
                let Value::Ref(r) = c.args[1] else { panic!() };
                id = r;
            }
            other => panic!("unexpected edge carrier {other}"),
        }
    }
}

/// Independent oracle: dense f64 sampling and atan2 in the circle frame.
fn oracle(
    data: &solid_kernel::step::Inventory,
    context: &BodyContext,
    map: &solid_kernel::step_plane_map::PlaneParameterMap,
    edge: solid_kernel::step::EntityReference,
    pcurve: solid_kernel::step::EntityReference,
    domain: [f64; 2],
    report: &solid_kernel::step_pcurve_arc::Report,
) {
    let scale = context.millimetres_per_source_unit();
    let raw = &context.body().edges[&edge.entity];
    let [o, x, y] = frame(data, raw.curve);
    let angle = |p: V3| {
        let d = sub(p, o.map(|c| c * scale));
        dot(d, y).atan2(dot(d, x))
    };
    let pc = match step_pcurve::read_pcurve(data, pcurve, nurbs::Limits::default())
        .unwrap()
        .geometry
    {
        ParametricCurve::Spline(s) => s,
        _ => unreachable!(),
    };
    let at = |t: f64| {
        let uv = pc.curve().evaluate(t).unwrap();
        let b = map.bounds([[uv[0]; 2], [uv[1]; 2]]).unwrap();
        let m: V3 = std::array::from_fn(|i| b.min[i] + (b.max[i] - b.min[i]) * 0.5);
        angle(m)
    };
    let tau = std::f64::consts::TAU;
    let ccw = |from: f64, to: f64| (to - from).rem_euclid(tau);
    let vertex = |id: u64| angle(context.body().vertices[&id].coordinates.map(|c| c * scale));
    let (from, to) = if raw.same_sense {
        (vertex(raw.start), vertex(raw.end))
    } else {
        (vertex(raw.end), vertex(raw.start))
    };
    let span = ccw(from, to);
    // Parameters visited from start to end vertex, per the report.
    let [lo, hi] = [domain[0], domain[1]];
    let (a, b) = (report.start_parameter, report.end_parameter);
    let n = 400;
    let path = |k: usize| {
        let f = k as f64 / n as f64;
        match (report.increasing, a <= b) {
            (true, true) | (false, false) => a + (b - a) * f,
            (true, false) => {
                let t = a + (hi - a + b - lo) * f;
                if t > hi {
                    t - (hi - lo)
                } else {
                    t
                }
            }
            (false, true) => {
                let t = a - (a - lo + hi - b) * f;
                if t < lo {
                    t + (hi - lo)
                } else {
                    t
                }
            }
        }
    };
    // Samples run from the start vertex to the end vertex; `along` is measured
    // counter-clockwise from the arc's counter-clockwise origin `from`.
    let sign = if raw.same_sense { 1. } else { -1. };
    let mut prev = if raw.same_sense { 0. } else { span };
    for k in 1..n {
        let along = ccw(from, at(path(k)));
        assert!(along <= span + 1e-6, "sample leaves the edge arc");
        assert!(
            (along - prev) * sign >= -1e-6,
            "arc is not traversed monotonically"
        );
        prev = along;
    }
}

#[test]
fn bracket_open_spline_edges_select_the_arc_found_by_independent_dense_sampling() {
    let data = inventory(SOURCE, Limits::default()).unwrap();
    let context = BodyContext::read(&data).unwrap();
    let (mut interior, mut wrapping, mut total) = (0, 0, 0);
    for (map, [edge, pcurve], domain) in spline_pcurves(&data, &context) {
        let r = discover(
            &map,
            &context,
            &data,
            Search {
                edge,
                pcurve,
                domain,
            },
            Default::default(),
            |_| true,
        )
        .unwrap();
        let Proposal::Interval { request, .. } = r.proposal else {
            continue;
        };
        let mut seen = 0;
        let report = select(&map, &context, &data, request, Options::default(), |p| {
            assert!(p.enclosures >= seen);
            seen = p.enclosures;
            true
        })
        .unwrap();
        assert_eq!(seen, 2);
        oracle(&data, &context, &map, edge, pcurve, domain, &report);
        match report.arc {
            Arc::Interior => interior += 1,
            Arc::Wrapping => wrapping += 1,
        }
        assert_eq!(
            report.increasing,
            (request.order == ParameterOrder::Forward) == (report.arc == Arc::Interior)
        );
        total += 1;
    }
    assert_eq!(total, 10);
    assert_eq!((interior, wrapping), (10, 0));
}

#[test]
fn arc_selection_refuses_closed_edges_bad_intervals_budgets_and_cancellation() {
    let data = inventory(SOURCE, Limits::default()).unwrap();
    let context = BodyContext::read(&data).unwrap();
    let all = spline_pcurves(&data, &context);
    let mut checked = false;
    for (map, [edge, pcurve], domain) in &all {
        let r = discover(
            map,
            &context,
            &data,
            Search {
                edge: *edge,
                pcurve: *pcurve,
                domain: *domain,
            },
            Default::default(),
            |_| true,
        )
        .unwrap();
        let Proposal::Interval { request, .. } = r.proposal else {
            // closed edge: no endpoint-delimited arc, whatever the interval
            let mut req = solid_kernel::step_pcurve_endpoints::Request {
                edge: *edge,
                pcurve: *pcurve,
                interval: [domain[0], domain[1]],
                order: ParameterOrder::Forward,
            };
            req.interval[1] = domain[0] + (domain[1] - domain[0]) * 0.5;
            assert!(select(map, &context, &data, req, Options::default(), |_| true).is_err());
            continue;
        };
        if checked {
            continue;
        }
        checked = true;
        let run = |req, o, f: &mut dyn FnMut(_) -> bool| select(map, &context, &data, req, o, f);
        for cap in 0..2 {
            let o = Options {
                max_enclosures: cap,
                ..Options::default()
            };
            assert_eq!(
                run(request, o, &mut |_| true).unwrap_err().kind,
                ErrorKind::Limit
            );
        }
        assert_eq!(
            run(request, Options::default(), &mut |_| false)
                .unwrap_err()
                .kind,
            ErrorKind::Cancelled
        );
        let mut n = 0;
        assert_eq!(
            run(request, Options::default(), &mut |_| {
                n += 1;
                n < 3
            })
            .unwrap_err()
            .kind,
            ErrorKind::Cancelled
        );
        let mut bad = request;
        bad.interval = [request.interval[1], request.interval[0]];
        assert_eq!(
            run(bad, Options::default(), &mut |_| true)
                .unwrap_err()
                .kind,
            ErrorKind::Geometry
        );
        bad.interval = [domain[0], domain[1] + 1.];
        assert!(run(bad, Options::default(), &mut |_| true).is_err());
        bad.interval = [domain[0], domain[1]];
        assert!(run(bad, Options::default(), &mut |_| true).is_err());
        let weak = Options {
            circle: Default::default(),
            ..Options::default()
        };
        assert!(run(request, weak, &mut |_| true).is_err());
        let stale = inventory(&format!("{SOURCE}\n"), Limits::default()).unwrap();
        assert!(select(map, &context, &stale, request, Options::default(), |_| true).is_err());
    }
    assert!(checked);
}

#[test]
fn reversing_an_edge_sense_selects_the_wrapping_complement() {
    let data = inventory(SOURCE, Limits::default()).unwrap();
    let context = BodyContext::read(&data).unwrap();
    let mut flipped = 0;
    for (map, [edge, pcurve], domain) in spline_pcurves(&data, &context) {
        let r = discover(
            &map,
            &context,
            &data,
            Search {
                edge,
                pcurve,
                domain,
            },
            Default::default(),
            |_| true,
        )
        .unwrap();
        let Proposal::Interval { request, .. } = r.proposal else {
            continue;
        };
        // Flip EDGE_CURVE.same_sense in the source text; the other arc is now the edge.
        let offset = data.entities()[&edge.entity].offset;
        let end = offset + SOURCE[offset..].find(';').unwrap();
        let record = &SOURCE[offset..end];
        let swapped = if record.contains(".T.") {
            record.replace(".T.", ".F.")
        } else {
            record.replace(".F.", ".T.")
        };
        assert_ne!(record, swapped);
        let text = format!("{}{}{}", &SOURCE[..offset], swapped, &SOURCE[end..]);
        let data2 = inventory(&text, Limits::default()).unwrap();
        let context2 = BodyContext::read(&data2).unwrap();
        let (map2, [edge2, pcurve2], _) = spline_pcurves(&data2, &context2)
            .into_iter()
            .find(|(_, [e, p], _)| e.entity == edge.entity && p.entity == pcurve.entity)
            .unwrap();
        let mut request = request;
        request.edge = edge2;
        request.pcurve = pcurve2;
        let report = select(
            &map2,
            &context2,
            &data2,
            request,
            Options::default(),
            |_| true,
        )
        .unwrap();
        assert_eq!(report.arc, Arc::Wrapping);
        oracle(&data2, &context2, &map2, edge2, pcurve2, domain, &report);
        flipped += 1;
    }
    assert_eq!(flipped, 10);
}
