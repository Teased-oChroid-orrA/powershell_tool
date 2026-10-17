use solid_kernel::{
    step::{inventory, Limits, Value},
    step_body::BodyContext,
    step_spline_edge::{self, Options},
};
const SOURCE: &str = include_str!("../../nema17-motor-bracket.step");
fn replace(source: &str, offset: usize, record: &str) -> String {
    let end = offset + source[offset..].find(';').unwrap() + 1;
    format!("{}{}{}", &source[..offset], record, &source[end..])
}
fn fixture() -> (String, u64, u64) {
    let data = inventory(SOURCE, Limits::default()).unwrap();
    let body = data.raw_bodies().unwrap().remove(0);
    let (edge, line) = body
        .edges
        .values()
        .find_map(|edge| {
            let mut id = edge.curve;
            for _ in 0..32 {
                let entity = &data.entities()[&id];
                let [call] = entity.components.as_slice() else {
                    return None;
                };
                if call.name == "LINE" {
                    return Some((edge, entity));
                }
                if matches!(call.name.as_str(), "SURFACE_CURVE" | "SEAM_CURVE") {
                    if let Some(Value::Ref(base)) = call.args.get(1) {
                        id = *base;
                        continue;
                    }
                }
                return None;
            }
            None
        })
        .unwrap();
    let mut points = [
        body.vertices[&edge.start].point,
        body.vertices[&edge.end].point,
    ];
    if !edge.same_sense {
        points.reverse();
    }
    (replace(SOURCE,line.offset,&format!("#{}=B_SPLINE_CURVE_WITH_KNOTS('edge',1,(#{},#{}),.POLYLINE_FORM.,.F.,.U.,(2,2),(10.,30.),.UNSPECIFIED.);",line.id,points[0],points[1])),edge.id,line.id)
}
#[test]
fn actual_bracket_source_edge_has_bounded_samples_and_preserved_topological_order_and_units() {
    let (source, edge_id, _) = fixture();
    let data = inventory(&source, Limits::default()).unwrap();
    let context = BodyContext::read(&data).unwrap();
    let edge_ref = data.entity_reference(edge_id).unwrap();
    let sampled =
        step_spline_edge::sample(&context, &data, edge_ref, Options::default(), |_| true).unwrap();
    assert_eq!(sampled.edge_reference(), edge_ref);
    assert_eq!(sampled.length_context(), context.length_context());
    assert_eq!(sampled.samples().parameters(), &[10., 30.]);
    assert!(sampled
        .endpoint_error_bounds_mm()
        .iter()
        .all(|x| *x <= 1e-7));
    let order: Vec<_> = sampled.points_in_edge_order().copied().collect();
    assert_eq!(
        order,
        [
            sampled.source_endpoints_mm()[0],
            sampled.source_endpoints_mm()[1]
        ]
    );
    let contexts = data.length_contexts().unwrap();
    let assigned = contexts
        .iter()
        .find(|c| c.context == context.length_context().entity)
        .unwrap();
    let unit = &data.entities()[&assigned.unit];
    let metres = replace(
        &source,
        unit.offset,
        &format!(
            "#{}=(LENGTH_UNIT() NAMED_UNIT(*) SI_UNIT($,.METRE.));",
            unit.id
        ),
    );
    let metric = inventory(&metres, Limits::default()).unwrap();
    let metric_context = BodyContext::read(&metric).unwrap();
    let converted = step_spline_edge::sample(
        &metric_context,
        &metric,
        metric.entity_reference(edge_id).unwrap(),
        Options::default(),
        |_| true,
    )
    .unwrap();
    for i in 0..2 {
        assert_eq!(
            converted.source_endpoints_mm()[i],
            sampled.source_endpoints_mm()[i].map(|x| x * 1000.)
        );
    }
    assert_eq!(
        converted.samples().parameters(),
        sampled.samples().parameters()
    );
    assert!(step_spline_edge::sample(
        &context,
        &metric,
        metric.entity_reference(edge_id).unwrap(),
        Options::default(),
        |_| true
    )
    .is_err());
    assert!(step_spline_edge::sample(
        &metric_context,
        &metric,
        edge_ref,
        Options::default(),
        |_| true
    )
    .is_err());
}
#[test]
fn reversed_curve_orientation_is_explicit_and_wrong_endpoint_or_interior_trims_refuse() {
    let (source, edge_id, curve_id) = fixture();
    let data = inventory(&source, Limits::default()).unwrap();
    let context = BodyContext::read(&data).unwrap();
    let edge = &context.body().edges[&edge_id];
    let entity = &data.entities()[&edge_id];
    let opposite = replace(
        &source,
        entity.offset,
        &format!(
            "#{}=EDGE_CURVE('',#{},#{},#{},.{}.);",
            edge_id,
            edge.start,
            edge.end,
            edge.curve,
            if edge.same_sense { "F" } else { "T" }
        ),
    );
    let wrong = inventory(&opposite, Limits::default()).unwrap();
    let wrong_context = BodyContext::read(&wrong).unwrap();
    let error = step_spline_edge::sample(
        &wrong_context,
        &wrong,
        wrong.entity_reference(edge_id).unwrap(),
        Options::default(),
        |_| true,
    )
    .unwrap_err();
    assert!(error.detail.contains("endpoint agreement"));
    assert_eq!(error.offset, wrong.entities()[&edge_id].offset);
    let c = &wrong.entities()[&curve_id];
    let body = wrong_context.body();
    let mut points = [
        body.vertices[&edge.start].point,
        body.vertices[&edge.end].point,
    ];
    if edge.same_sense {
        points.reverse();
    }
    let reverse=replace(&opposite,c.offset,&format!("#{}=B_SPLINE_CURVE_WITH_KNOTS('reverse',1,(#{},#{}),.POLYLINE_FORM.,.F.,.U.,(2,2),(10.,30.),.UNSPECIFIED.);",curve_id,points[0],points[1]));
    let reverse = inventory(&reverse, Limits::default()).unwrap();
    let reverse_context = BodyContext::read(&reverse).unwrap();
    let accepted = step_spline_edge::sample(
        &reverse_context,
        &reverse,
        reverse.entity_reference(edge_id).unwrap(),
        Options::default(),
        |_| true,
    )
    .unwrap();
    assert_eq!(accepted.same_sense(), !edge.same_sense);
    assert_eq!(
        accepted.points_in_edge_order().copied().collect::<Vec<_>>(),
        accepted.source_endpoints_mm()
    );
    // The same geometric line extended beyond both source vertices requires
    // explicit interior trims; full-domain admission must not silently accept it.
    let coords = [
        context.body().vertices[&edge.start].coordinates,
        context.body().vertices[&edge.end].coordinates,
    ];
    let mut extended: [[f64; 3]; 2] = [
        std::array::from_fn(|i| 2. * coords[0][i] - coords[1][i]),
        std::array::from_fn(|i| 2. * coords[1][i] - coords[0][i]),
    ];
    if !edge.same_sense {
        extended.reverse();
    }
    let next = data.entities().keys().last().unwrap() + 1;
    let c = &data.entities()[&curve_id];
    let mut interior=replace(&source,c.offset,&format!("#{}=B_SPLINE_CURVE_WITH_KNOTS('untrimmed',1,(#{},#{}),.POLYLINE_FORM.,.F.,.U.,(2,2),(10.,30.),.UNSPECIFIED.);",curve_id,next,next+1));
    let records = extended
        .iter()
        .enumerate()
        .map(|(i, p)| {
            format!(
                "#{}=CARTESIAN_POINT('',({:.17e},{:.17e},{:.17e}));\n",
                next + i as u64,
                p[0],
                p[1],
                p[2]
            )
        })
        .collect::<String>();
    let end = interior.rfind("ENDSEC;").unwrap();
    interior.insert_str(end, &records);
    let interior_text = interior;
    let interior = inventory(&interior_text, Limits::default()).unwrap();
    let interior_context = BodyContext::read(&interior).unwrap();
    assert!(step_spline_edge::sample(
        &interior_context,
        &interior,
        interior.entity_reference(edge_id).unwrap(),
        Options::default(),
        |_| true
    )
    .unwrap_err()
    .detail
    .contains("endpoint agreement"));
    // Explicit finite parameters now admit precisely the source-vertex portion,
    // while the unchanged full-domain path above still refuses it.
    let interval = [10. + 20. / 3., 10. + 40. / 3.];
    let trimmed = step_spline_edge::sample_interval(
        &interior_context,
        &interior,
        interior.entity_reference(edge_id).unwrap(),
        interval,
        Options::default(),
        |_| true,
    )
    .unwrap();
    assert_eq!(trimmed.samples().parameters(), &interval);
    assert_eq!(trimmed.same_sense(), edge.same_sense);
    assert!(trimmed
        .endpoint_error_bounds_mm()
        .iter()
        .all(|e| *e <= 1e-7));
    for (p, v) in trimmed
        .points_in_edge_order()
        .zip(trimmed.source_endpoints_mm())
    {
        assert!(p.iter().zip(v).all(|(a, b)| (a - b).abs() <= 1e-7));
    }
    let assigned = interior
        .length_contexts()
        .unwrap()
        .into_iter()
        .find(|c| c.context == interior_context.length_context().entity)
        .unwrap();
    let unit = &interior.entities()[&assigned.unit];
    let metres_text = replace(
        &interior_text,
        unit.offset,
        &format!(
            "#{}=(LENGTH_UNIT() NAMED_UNIT(*) SI_UNIT($,.METRE.));",
            unit.id
        ),
    );
    let metres = inventory(&metres_text, Limits::default()).unwrap();
    let metres_context = BodyContext::read(&metres).unwrap();
    let converted = step_spline_edge::sample_interval(
        &metres_context,
        &metres,
        metres.entity_reference(edge_id).unwrap(),
        interval,
        Options::default(),
        |_| true,
    )
    .unwrap();
    assert_eq!(
        converted.samples().parameters(),
        trimmed.samples().parameters()
    );
    for (a, b) in converted
        .source_endpoints_mm()
        .into_iter()
        .zip(trimmed.source_endpoints_mm())
    {
        assert_eq!(a, b.map(|x| x * 1000.));
    }
    assert!(step_spline_edge::sample_interval(
        &interior_context,
        &metres,
        metres.entity_reference(edge_id).unwrap(),
        interval,
        Options::default(),
        |_| true
    )
    .is_err());
    assert!(step_spline_edge::sample_interval(
        &interior_context,
        &interior,
        interior.entity_reference(edge_id).unwrap(),
        [interval[0] + 0.1, interval[1]],
        Options::default(),
        |_| true,
    )
    .unwrap_err()
    .detail
    .contains("endpoint agreement"));
    for invalid in [[10., 31.], [20., 20.], [30., 10.], [f64::NAN, 20.]] {
        assert!(step_spline_edge::sample_interval(
            &interior_context,
            &interior,
            interior.entity_reference(edge_id).unwrap(),
            invalid,
            Options::default(),
            |_| true,
        )
        .is_err());
    }
    assert_eq!(
        step_spline_edge::sample_interval(
            &interior_context,
            &interior,
            interior.entity_reference(edge_id).unwrap(),
            interval,
            Options::default(),
            |_| false,
        )
        .unwrap_err()
        .kind,
        solid_kernel::step::ErrorKind::Cancelled
    );
    // Reusing the start control twice cannot agree with the distinct end vertex.
    let c = &data.entities()[&curve_id];
    let cp = context.body().vertices[&edge.start].point;
    let bad=replace(&source,c.offset,&format!("#{}=B_SPLINE_CURVE_WITH_KNOTS('wrong',1,(#{cp},#{cp}),.POLYLINE_FORM.,.F.,.U.,(2,2),(10.,30.),.UNSPECIFIED.);",curve_id));
    let bad = inventory(&bad, Limits::default()).unwrap();
    let bad_context = BodyContext::read(&bad).unwrap();
    assert!(step_spline_edge::sample(
        &bad_context,
        &bad,
        bad.entity_reference(edge_id).unwrap(),
        Options::default(),
        |_| true
    )
    .unwrap_err()
    .detail
    .contains("endpoint agreement"));
}
#[test]
fn source_edge_sampling_cancellation_work_limits_and_invalid_tolerances_are_atomic() {
    let (source, edge_id, _) = fixture();
    let data = inventory(&source, Limits::default()).unwrap();
    let context = BodyContext::read(&data).unwrap();
    let edge = data.entity_reference(edge_id).unwrap();
    assert!(
        step_spline_edge::sample(&context, &data, edge, Options::default(), |_| false)
            .unwrap_err()
            .detail
            .contains("cancelled")
    );
    assert_eq!(
        step_spline_edge::sample(&context, &data, edge, Options::default(), |_| false)
            .unwrap_err()
            .kind,
        solid_kernel::step::ErrorKind::Cancelled
    );
    let mut complete_calls = 0;
    assert!(
        step_spline_edge::sample(&context, &data, edge, Options::default(), |p| {
            if p.enclosures > 0 && p.pending_intervals == 0 {
                complete_calls += 1;
            }
            complete_calls < 2
        })
        .unwrap_err()
        .detail
        .contains("cancelled")
    );
    let mut options = Options::default();
    options.sampling.max_segments = 0;
    let error = step_spline_edge::sample(&context, &data, edge, options, |_| true).unwrap_err();
    assert_eq!(error.offset, data.entities()[&edge_id].offset);
    for tolerance in [0., f64::NAN, 0.1] {
        assert!(step_spline_edge::sample(
            &context,
            &data,
            edge,
            Options {
                geometry_tolerance_mm: tolerance,
                ..Options::default()
            },
            |_| true
        )
        .is_err());
    }
}

#[test]
fn rational_single_span_body_import_is_geometric_and_curved_or_piecewise_loci_stay_gated() {
    use solid_kernel::analytic_step::{AnalyticModel, Options as BodyOptions};
    let (source, edge_id, curve_id) = fixture();
    let data = inventory(&source, Limits::default()).unwrap();
    let curve = &data.entities()[&curve_id];
    let Value::List(cp) = &curve.components[0].args[2] else {
        panic!("fixture");
    };
    let (Value::Ref(a), Value::Ref(b)) = (&cp[0], &cp[1]) else {
        panic!("fixture");
    };
    let rational=replace(&source,curve.offset,&format!("#{}=(BOUNDED_CURVE() B_SPLINE_CURVE(1,(#{a},#{b}),.UNSPECIFIED.,.F.,.U.) B_SPLINE_CURVE_WITH_KNOTS((2,2),(10.,30.),.UNSPECIFIED.) CURVE() GEOMETRIC_REPRESENTATION_ITEM() RATIONAL_B_SPLINE_CURVE((3.,7.)) REPRESENTATION_ITEM('nonuniform'));",curve_id));
    let rational = inventory(&rational, Limits::default()).unwrap();
    let imported = AnalyticModel::read(&rational, BodyOptions::default()).unwrap();
    let reference = AnalyticModel::read(&data, BodyOptions::default()).unwrap();
    assert_eq!(
        imported.edge_points(edge_id),
        reference.edge_points(edge_id)
    );
    // Even a collinear quadratic with a line-like form declaration is not the
    // verified degree-one path: no inference or accidental broad capability.
    let quadratic=replace(&source,curve.offset,&format!("#{}=B_SPLINE_CURVE_WITH_KNOTS('unsupported',2,(#{a},#{a},#{b}),.POLYLINE_FORM.,.F.,.U.,(3,3),(10.,30.),.UNSPECIFIED.);",curve_id));
    let quadratic = inventory(&quadratic, Limits::default()).unwrap();
    let error = AnalyticModel::read(&quadratic, BodyOptions::default()).unwrap_err();
    assert!(error
        .detail
        .contains("only full-domain two-control degree-one"));
    assert_eq!(error.offset, quadratic.entities()[&curve_id].offset);
    let context = BodyContext::read(&data).unwrap();
    let edge = &context.body().edges[&edge_id];
    let edge_entity = &data.entities()[&edge_id];
    let wrong = replace(
        &source,
        edge_entity.offset,
        &format!(
            "#{}=EDGE_CURVE('',#{},#{},#{},.{}.);",
            edge_id,
            edge.start,
            edge.end,
            edge.curve,
            if edge.same_sense { "F" } else { "T" }
        ),
    );
    assert!(AnalyticModel::read(
        &inventory(&wrong, Limits::default()).unwrap(),
        BodyOptions::default()
    )
    .unwrap_err()
    .detail
    .contains("endpoint agreement"));
}
