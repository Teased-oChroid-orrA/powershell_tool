use solid_kernel::{
    nurbs,
    step::{inventory, ErrorKind, Limits},
    step_body::BodyContext,
    step_cylinder_map::CylinderParameterMap,
    step_parameter_map::ParameterMapRef,
    step_pcurve::{self, ParametricCurve},
    step_pcurve_endpoints::{self, Options, ParameterOrder, Request},
    step_plane_map::PlaneParameterMap,
};
const SOURCE: &str = include_str!("../../nema17-motor-bracket.step");
fn midpoint(b: solid_kernel::geometry_bounds::Bounds3) -> [f64; 3] {
    std::array::from_fn(|i| b.min[i] + (b.max[i] - b.min[i]) * 0.5)
}
#[test]
fn supplied_finite_parameters_match_all_bracket_plane_lines_and_cylinder_generators() {
    let data = inventory(SOURCE, Limits::default()).unwrap();
    let context = BodyContext::read(&data).unwrap();
    let mut counts = [0; 2];
    for face in context.body().faces.values() {
        let plane;
        let cylinder;
        let kind = if data.entities()[&face.surface].components[0].name == "PLANE" {
            0
        } else {
            1
        };
        let map = if kind == 0 {
            plane =
                PlaneParameterMap::read(&context, &data, data.entity_reference(face.id).unwrap())
                    .unwrap();
            ParameterMapRef::from(&plane)
        } else {
            cylinder = CylinderParameterMap::read(
                &context,
                &data,
                data.entity_reference(face.id).unwrap(),
            )
            .unwrap();
            ParameterMapRef::from(&cylinder)
        };
        let mut seen = std::collections::BTreeSet::new();
        for use_ in face.bounds.iter().flat_map(|b| &b.edges) {
            let edge = &context.body().edges[&use_.edge];
            let wrapper =
                step_pcurve::read_surface_curve(&data, data.entity_reference(edge.curve).unwrap())
                    .unwrap();
            for pc in wrapper.pcurves {
                let d = step_pcurve::read_pcurve(&data, pc, nurbs::Limits::default()).unwrap();
                if d.basis_surface != map.surface_reference() || !seen.insert((edge.id, pc.entity))
                {
                    continue;
                }
                let ParametricCurve::Line {
                    origin,
                    direction,
                    magnitude,
                } = d.geometry
                else {
                    continue;
                };
                if kind == 1 && direction[0] != 0. {
                    continue;
                }
                let a = midpoint(map.bounds(origin.map(|x| [x, x]), 0).unwrap());
                let b = midpoint(
                    map.bounds(
                        std::array::from_fn(|i| {
                            let x = origin[i] + direction[i] * magnitude;
                            [x, x]
                        }),
                        0,
                    )
                    .unwrap(),
                );
                let delta: [f64; 3] = std::array::from_fn(|i| b[i] - a[i]);
                let len2 = delta.iter().map(|x| x * x).sum::<f64>();
                let parameters = [edge.start, edge.end].map(|id| {
                    let p = context.body().vertices[&id].coordinates;
                    (0..3)
                        .map(|i| (p[i] * context.millimetres_per_source_unit() - a[i]) * delta[i])
                        .sum::<f64>()
                        / len2
                });
                let request = Request {
                    edge: data.entity_reference(edge.id).unwrap(),
                    pcurve: pc,
                    interval: [
                        parameters[0].min(parameters[1]),
                        parameters[0].max(parameters[1]),
                    ],
                    order: if parameters[0] < parameters[1] {
                        ParameterOrder::Forward
                    } else {
                        ParameterOrder::Reversed
                    },
                };
                let mut work = 0;
                let mut done = 0;
                let r = step_pcurve_endpoints::verify(
                    map,
                    &context,
                    &data,
                    request,
                    Options::default(),
                    |p| {
                        assert!(p.enclosures >= work && p.checked_endpoints >= done);
                        work = p.enclosures;
                        done = p.checked_endpoints;
                        true
                    },
                )
                .unwrap();
                assert_eq!(r.enclosures, 3);
                assert_eq!(work, 3);
                assert_eq!(done, 2);
                assert_eq!(r.source_vertices.map(|v| v.entity), [edge.start, edge.end]);
                assert!(r.endpoint_error_upper_mm.iter().all(|e| *e <= 1e-7));
                counts[kind] += 1;
                if counts[kind] == 1 {
                    let mut bad = request;
                    bad.interval[0] += 0.01;
                    assert!(step_pcurve_endpoints::verify(
                        map,
                        &context,
                        &data,
                        bad,
                        Options::default(),
                        |_| true
                    )
                    .is_err());
                    bad = request;
                    bad.order = if request.order == ParameterOrder::Forward {
                        ParameterOrder::Reversed
                    } else {
                        ParameterOrder::Forward
                    };
                    assert!(step_pcurve_endpoints::verify(
                        map,
                        &context,
                        &data,
                        bad,
                        Options::default(),
                        |_| true
                    )
                    .is_err());
                    for cap in 0..3 {
                        let o = Options {
                            max_enclosures: cap,
                            ..Options::default()
                        };
                        assert_eq!(
                            step_pcurve_endpoints::verify(map, &context, &data, request, o, |_| {
                                true
                            })
                            .unwrap_err()
                            .kind,
                            ErrorKind::Limit
                        );
                    }
                    assert_eq!(
                        step_pcurve_endpoints::verify(
                            map,
                            &context,
                            &data,
                            request,
                            Options::default(),
                            |_| false
                        )
                        .unwrap_err()
                        .kind,
                        ErrorKind::Cancelled
                    );
                    let mut final_calls = 0;
                    assert_eq!(
                        step_pcurve_endpoints::verify(
                            map,
                            &context,
                            &data,
                            request,
                            Options::default(),
                            |p| {
                                if p.checked_endpoints == 2 {
                                    final_calls += 1;
                                }
                                final_calls < 2
                            }
                        )
                        .unwrap_err()
                        .kind,
                        ErrorKind::Cancelled
                    );
                    let stale = inventory(&format!("{SOURCE}\n"), Limits::default()).unwrap();
                    assert!(step_pcurve_endpoints::verify(
                        map,
                        &context,
                        &stale,
                        request,
                        Options::default(),
                        |_| true
                    )
                    .is_err());
                    for interval in [[0., 0.], [2., 1.], [f64::NAN, 1.]] {
                        bad = request;
                        bad.interval = interval;
                        assert_eq!(
                            step_pcurve_endpoints::verify(
                                map,
                                &context,
                                &data,
                                bad,
                                Options::default(),
                                |_| true
                            )
                            .unwrap_err()
                            .kind,
                            ErrorKind::Geometry
                        );
                    }
                }
            }
        }
    }
    assert_eq!(counts, [148, 28]);
}
