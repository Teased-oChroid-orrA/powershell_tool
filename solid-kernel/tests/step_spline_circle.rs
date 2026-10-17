use solid_kernel::{
    nurbs,
    step::{inventory, ErrorKind, Inventory, Limits},
    step_body::BodyContext,
    step_pcurve::{self, ParametricCurve},
    step_plane_map::PlaneParameterMap,
    step_spline_circle::Options,
};
const SOURCE: &str = include_str!("../../nema17-motor-bracket.step");
fn cases(d: &Inventory, c: &BodyContext) -> Vec<(u64, u64, u64)> {
    let mut found = std::collections::BTreeSet::new();
    for face in c
        .body()
        .faces
        .values()
        .filter(|f| d.entities()[&f.surface].components[0].name == "PLANE")
    {
        for usage in face.bounds.iter().flat_map(|b| &b.edges) {
            let edge = &c.body().edges[&usage.edge];
            let wrapper =
                step_pcurve::read_surface_curve(d, d.entity_reference(edge.curve).unwrap())
                    .unwrap();
            for p in wrapper.pcurves {
                let pc = step_pcurve::read_pcurve(d, p, nurbs::Limits::default()).unwrap();
                if pc.basis_surface.entity == face.surface
                    && matches!(pc.geometry, ParametricCurve::Spline(_))
                {
                    found.insert((face.id, edge.id, p.entity));
                }
            }
        }
    }
    found.into_iter().collect()
}
#[test]
fn all_bracket_quadratic_spline_pcurves_bound_complete_span_circle_deviation() {
    let d = inventory(SOURCE, Limits::default()).unwrap();
    let c = BodyContext::read(&d).unwrap();
    let found = cases(&d, &c);
    assert_eq!(found.len(), 15);
    for (face, edge, pcurve) in found {
        let map = PlaneParameterMap::read(&c, &d, d.entity_reference(face).unwrap()).unwrap();
        let mut progress = Vec::new();
        let report = map
            .verify_spline_circle_deviation(
                &c,
                &d,
                d.entity_reference(edge).unwrap(),
                d.entity_reference(pcurve).unwrap(),
                Options {
                    require_circle_coverage: true,
                    ..Options::default()
                },
                |p| {
                    progress.push(p);
                    true
                },
            )
            .unwrap();
        assert_eq!(report.spans, 3);
        assert!(report.full_circle_coverage);
        assert!(matches!(report.angular_orientation, Some(-1) | Some(1)));
        assert_eq!(report.face.entity, face);
        assert_eq!(report.edge.entity, edge);
        assert_eq!(report.pcurve.entity, pcurve);
        assert!(report.curve_to_circle_upper_mm <= 1e-7);
        assert_eq!(progress.last().unwrap().spans, 3);
        assert_eq!(progress.last().unwrap().total_spans, 3);
        assert!(progress.windows(2).all(|p| p[0].spans <= p[1].spans));
    }
}
#[test]
fn cancellation_budget_stale_and_changed_rational_controls_refuse() {
    let d = inventory(SOURCE, Limits::default()).unwrap();
    let c = BodyContext::read(&d).unwrap();
    let (face, edge, pcurve) = cases(&d, &c)[0];
    let map = PlaneParameterMap::read(&c, &d, d.entity_reference(face).unwrap()).unwrap();
    for stop in [0, 1, 2, 3, 4] {
        let mut calls = 0;
        let error = map
            .verify_spline_circle_deviation(
                &c,
                &d,
                d.entity_reference(edge).unwrap(),
                d.entity_reference(pcurve).unwrap(),
                Options::default(),
                |_| {
                    calls += 1;
                    calls <= stop
                },
            )
            .unwrap_err();
        assert_eq!(error.kind, ErrorKind::Cancelled);
    }
    let error = map
        .verify_spline_circle_deviation(
            &c,
            &d,
            d.entity_reference(edge).unwrap(),
            d.entity_reference(pcurve).unwrap(),
            Options {
                max_spans: 2,
                ..Options::default()
            },
            |_| true,
        )
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::Limit);
    let pc = step_pcurve::read_pcurve(
        &d,
        d.entity_reference(pcurve).unwrap(),
        nurbs::Limits::default(),
    )
    .unwrap();
    let record = d.resolve_reference(pc.geometry_source).unwrap();
    let end = record.offset + SOURCE[record.offset..].find(';').unwrap() + 1;
    let replacement = SOURCE[record.offset..end].replace("0.5", "0.6");
    assert_ne!(replacement, SOURCE[record.offset..end]);
    let source = format!(
        "{}{}{}",
        &SOURCE[..record.offset],
        replacement,
        &SOURCE[end..]
    );
    let changed = inventory(&source, Limits::default()).unwrap();
    let cc = BodyContext::read(&changed).unwrap();
    let cm =
        PlaneParameterMap::read(&cc, &changed, changed.entity_reference(face).unwrap()).unwrap();
    assert!(cm
        .verify_spline_circle_deviation(
            &cc,
            &changed,
            changed.entity_reference(edge).unwrap(),
            changed.entity_reference(pcurve).unwrap(),
            Options::default(),
            |_| true
        )
        .is_err());
    assert!(map
        .verify_spline_circle_deviation(
            &cc,
            &changed,
            changed.entity_reference(edge).unwrap(),
            changed.entity_reference(pcurve).unwrap(),
            Options::default(),
            |_| true
        )
        .is_err());
    assert!(map
        .verify_spline_circle_deviation(
            &c,
            &d,
            d.entity_reference(edge).unwrap(),
            d.entity_reference(pcurve).unwrap(),
            Options {
                tolerance_mm: f64::NAN,
                ..Options::default()
            },
            |_| true
        )
        .is_err());
}

#[test]
fn a_small_closure_gap_is_not_promoted_to_full_circle_coverage() {
    use solid_kernel::step::Value;
    let d = inventory(SOURCE, Limits::default()).unwrap();
    let c = BodyContext::read(&d).unwrap();
    let (face, edge, pcurve) = cases(&d, &c)[0];
    let pc = step_pcurve::read_pcurve(
        &d,
        d.entity_reference(pcurve).unwrap(),
        nurbs::Limits::default(),
    )
    .unwrap();
    let ParametricCurve::Spline(spline) = pc.geometry else {
        panic!("fixture")
    };
    let last = d
        .resolve_reference(*spline.control_point_sources().last().unwrap())
        .unwrap();
    let Value::List(coordinates) = &last.components[0].args[1] else {
        panic!("fixture")
    };
    let value = |v: &Value| match v {
        Value::Real(x) => *x,
        Value::Integer(x) => *x as f64,
        _ => panic!("fixture"),
    };
    let end = last.offset + SOURCE[last.offset..].find(';').unwrap() + 1;
    let source = format!(
        "{}#{}=CARTESIAN_POINT('',({:.17e},{:.17e}));{}",
        &SOURCE[..last.offset],
        last.id,
        value(&coordinates[0]) + 1e-9,
        value(&coordinates[1]),
        &SOURCE[end..]
    );
    let changed = inventory(&source, Limits::default()).unwrap();
    let cc = BodyContext::read(&changed).unwrap();
    let map =
        PlaneParameterMap::read(&cc, &changed, changed.entity_reference(face).unwrap()).unwrap();
    let report = map
        .verify_spline_circle_deviation(
            &cc,
            &changed,
            changed.entity_reference(edge).unwrap(),
            changed.entity_reference(pcurve).unwrap(),
            Options::default(),
            |_| true,
        )
        .unwrap();
    assert!(!report.full_circle_coverage && report.angular_orientation.is_none());
    let error = map
        .verify_spline_circle_deviation(
            &cc,
            &changed,
            changed.entity_reference(edge).unwrap(),
            changed.entity_reference(pcurve).unwrap(),
            Options {
                require_circle_coverage: true,
                ..Options::default()
            },
            |_| true,
        )
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::Geometry);
    assert!(error.detail.contains("exactly closed"));
}
#[test]
fn valid_unsegmented_quadratic_knots_do_not_get_a_bezier_certificate() {
    let d = inventory(SOURCE, Limits::default()).unwrap();
    let c = BodyContext::read(&d).unwrap();
    let (face, edge, pcurve) = cases(&d, &c)[0];
    let pc = step_pcurve::read_pcurve(
        &d,
        d.entity_reference(pcurve).unwrap(),
        nurbs::Limits::default(),
    )
    .unwrap();
    let record = d.resolve_reference(pc.geometry_source).unwrap();
    let end = record.offset + SOURCE[record.offset..].find(';').unwrap() + 1;
    let replacement = SOURCE[record.offset..end].replace("(1,2,2,2,2,1)", "(2,1,2,2,1,2)");
    assert_ne!(replacement, SOURCE[record.offset..end]);
    let source = format!(
        "{}{}{}",
        &SOURCE[..record.offset],
        replacement,
        &SOURCE[end..]
    );
    let changed = inventory(&source, Limits::default()).unwrap();
    let cc = BodyContext::read(&changed).unwrap();
    // The evaluator still accepts this explicit active-domain spline. The
    // specialized certificate must refuse its non-Bezier-separated span.
    step_pcurve::read_pcurve(
        &changed,
        changed.entity_reference(pcurve).unwrap(),
        nurbs::Limits::default(),
    )
    .unwrap();
    let map =
        PlaneParameterMap::read(&cc, &changed, changed.entity_reference(face).unwrap()).unwrap();
    let error = map
        .verify_spline_circle_deviation(
            &cc,
            &changed,
            changed.entity_reference(edge).unwrap(),
            changed.entity_reference(pcurve).unwrap(),
            Options::default(),
            |_| true,
        )
        .unwrap_err();
    assert!(error.detail.contains("Bezier-separated"));
}
