use solid_kernel::{
    nurbs::Limits as CurveLimits,
    step::{inventory, ErrorKind, Limits},
    step_nurbs::{read_curve, Logical},
};
fn file(curve: &str) -> String {
    format!("ISO-10303-21; HEADER; FILE_DESCRIPTION(('spline-test'),'2;1'); FILE_NAME('fixture','',(''),(''),'','',''); FILE_SCHEMA(('TEST')); ENDSEC; DATA; {curve} #2=CARTESIAN_POINT('',(1.,0.,0.)); #3=CARTESIAN_POINT('',(1.,1.,0.)); #4=CARTESIAN_POINT('',(0.,1.,0.)); ENDSEC; END-ISO-10303-21;")
}
const RATIONAL:&str="#1=(BOUNDED_CURVE() B_SPLINE_CURVE(2,(#2,#3,#4),.CIRCULAR_ARC.,.F.,.U.) B_SPLINE_CURVE_WITH_KNOTS((3,3),(0.,1.),.QUASI_UNIFORM_KNOTS.) CURVE() GEOMETRIC_REPRESENTATION_ITEM() RATIONAL_B_SPLINE_CURVE((1.,0.7071067811865476,1.)) REPRESENTATION_ITEM('quarter'));";
#[test]
fn rational_complex_declaration_retains_identity_metadata_and_explicit_length_scale() {
    let source = file(RATIONAL);
    let data = inventory(&source, Limits::default()).unwrap();
    let declaration = read_curve(&data, 1, 25.4, CurveLimits::default()).unwrap();
    assert_eq!(declaration.source, data.entity_reference(1).unwrap());
    assert_eq!(
        declaration
            .control_point_sources
            .iter()
            .map(|r| r.entity)
            .collect::<Vec<_>>(),
        vec![2, 3, 4]
    );
    for reference in &declaration.control_point_sources {
        assert_eq!(
            data.resolve_reference(*reference).unwrap().components[0].name,
            "CARTESIAN_POINT"
        );
    }
    assert_eq!(declaration.declared_name, "quarter");
    assert_eq!(declaration.declared_form, "CIRCULAR_ARC");
    assert_eq!(declaration.declared_closed, Logical::False);
    assert_eq!(declaration.declared_self_intersection, Logical::Unknown);
    assert_eq!(declaration.declared_knot_spec, "QUASI_UNIFORM_KNOTS");
    assert_eq!(declaration.millimetres_per_source_unit, 25.4);
    assert_eq!(declaration.curve.evaluate(0.).unwrap(), [25.4, 0., 0.]);
    for u in [0., 0.25, 0.5, 0.75, 1.] {
        let p = declaration.curve.evaluate(u).unwrap();
        assert!((p[0].hypot(p[1]) - 25.4).abs() / 25.4 < 1e-12);
    }
    let changed = inventory(&format!("{source}\n"), Limits::default()).unwrap();
    assert!(changed.resolve_reference(declaration.source).is_err());
}
#[test]
fn polynomial_variants_do_not_infer_geometry_from_the_declared_form() {
    let simple="#1=B_SPLINE_CURVE_WITH_KNOTS('polynomial',2,(#2,#3,#4),.UNSPECIFIED.,.U.,.U.,(3,3),(0.,1.),.UNSPECIFIED.);";
    let data = inventory(&file(simple), Limits::default()).unwrap();
    let declaration = read_curve(&data, 1, 1., CurveLimits::default()).unwrap();
    assert_eq!(declaration.curve.weights(), &[1.; 3]);
    assert_eq!(declaration.curve.evaluate(0.5).unwrap(), [0.75, 0.75, 0.]);
    assert_eq!(declaration.declared_closed, Logical::Unknown);
    let polynomial = RATIONAL.replace(" RATIONAL_B_SPLINE_CURVE((1.,0.7071067811865476,1.))", "");
    let data = inventory(&file(&polynomial), Limits::default()).unwrap();
    let declaration = read_curve(&data, 1, 1., CurveLimits::default()).unwrap();
    assert_eq!(declaration.declared_form, "CIRCULAR_ARC");
    assert_eq!(declaration.curve.evaluate(0.5).unwrap(), [0.75, 0.75, 0.]);
}
#[test]
fn malformed_unknown_duplicate_and_expansion_inputs_are_not_partially_extracted() {
    for source in [
        RATIONAL.replace("(3,3)", "(9223372036854775807,3)"),
        RATIONAL.replace("(0.,1.)", "(0.,0.)"),
        RATIONAL.replace("GEOMETRIC_REPRESENTATION_ITEM()", "UNSUPPORTED_COMPONENT()"),
        RATIONAL.replace(" CURVE()", ""),
        RATIONAL.replace(" CURVE()", " CURVE() CURVE()"),
        RATIONAL.replace("0.7071067811865476", "-1."),
        RATIONAL.replace("(#2,#3,#4)", "(#2,1.,#4)"),
        RATIONAL.replace(".CIRCULAR_ARC.", ".UNKNOWN_FORM."),
        RATIONAL.replace(".F.,.U.", ".INVALID.,.U."),
        RATIONAL.replace("(3,3)", "(2,4)"),
    ] {
        let data = inventory(&file(&source), Limits::default()).unwrap();
        assert!(
            read_curve(&data, 1, 1., CurveLimits::default()).is_err(),
            "{source}"
        );
    }
    let data = inventory(&file(RATIONAL), Limits::default()).unwrap();
    for limits in [
        CurveLimits {
            degree: 1,
            ..CurveLimits::default()
        },
        CurveLimits {
            control_points: 2,
            ..CurveLimits::default()
        },
    ] {
        assert_eq!(
            read_curve(&data, 1, 1., limits).unwrap_err().kind,
            ErrorKind::Limit
        );
    }
    assert_eq!(
        read_curve(&data, 999, 1., CurveLimits::default())
            .unwrap_err()
            .kind,
        ErrorKind::DanglingReference
    );
}
#[test]
fn explicit_scaling_refuses_underflow_and_never_guesses_units() {
    let source = file(RATIONAL).replace("(1.,0.,0.)", "(1.E-310,0.,0.)");
    let data = inventory(&source, Limits::default()).unwrap();
    let error = read_curve(&data, 1, 1e-21, CurveLimits::default()).unwrap_err();
    assert!(error.detail.contains("underflow"));
    assert_eq!(error.offset, data.entities()[&2].offset);
    let data = inventory(&file(RATIONAL), Limits::default()).unwrap();
    for scale in [0., -1., f64::NAN, f64::INFINITY] {
        assert!(read_curve(&data, 1, scale, CurveLimits::default()).is_err());
    }
}
#[test]
fn verified_single_span_linear_spline_can_enter_the_native_body_pipeline() {
    let source = include_str!("../../nema17-motor-bracket.step");
    let data = inventory(source, Limits::default()).unwrap();
    let body = data.raw_bodies().unwrap().remove(0);
    let (edge, line) = body
        .edges
        .values()
        .find_map(|edge| {
            let mut id = edge.curve;
            for _ in 0..32 {
                let e = &data.entities()[&id];
                let [call] = e.components.as_slice() else {
                    return None;
                };
                if call.name == "LINE" {
                    return Some((edge, e));
                }
                if matches!(call.name.as_str(), "SURFACE_CURVE" | "SEAM_CURVE") {
                    if let Some(solid_kernel::step::Value::Ref(base)) = call.args.get(1) {
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
    let mut changed = source.to_string();
    let end = source[line.offset..].find(';').unwrap() + line.offset + 1;
    changed.replace_range(line.offset..end,&format!("#{}=B_SPLINE_CURVE_WITH_KNOTS('diagnostic',1,(#{},#{}),.POLYLINE_FORM.,.F.,.U.,(2,2),(0.,1.),.UNSPECIFIED.);",line.id,points[0],points[1]));
    let data = inventory(&changed, Limits::default()).unwrap();
    let binding = solid_kernel::step_body::BodyContext::read(&data).unwrap();
    let edge_ref = data.entity_reference(edge.id).unwrap();
    let declaration = binding
        .read_spline_curve(&data, edge_ref, CurveLimits::default())
        .unwrap();
    assert_eq!(declaration.curve.degree(), 1);
    assert_eq!(declaration.source, data.entity_reference(line.id).unwrap());
    assert_eq!(declaration.millimetres_per_source_unit, 1.);
    assert!(binding
        .read_spline_curve(
            &data,
            data.entity_reference(body.id).unwrap(),
            CurveLimits::default()
        )
        .unwrap_err()
        .detail
        .contains("not an edge"));
    let changed_document = inventory(&(changed.clone() + "\n"), Limits::default()).unwrap();
    assert!(binding
        .read_spline_curve(
            &changed_document,
            changed_document.entity_reference(edge.id).unwrap(),
            CurveLimits::default()
        )
        .is_err());
    let new_binding = solid_kernel::step_body::BodyContext::read(&changed_document).unwrap();
    assert!(new_binding
        .read_spline_curve(&changed_document, edge_ref, CurveLimits::default())
        .is_err());
    let contexts = data.length_contexts().unwrap();
    let assigned = contexts
        .iter()
        .find(|c| c.context == binding.length_context().entity)
        .unwrap();
    let unit = &data.entities()[&assigned.unit];
    let mut metres = changed.clone();
    let end = unit.offset + metres[unit.offset..].find(';').unwrap() + 1;
    metres.replace_range(
        unit.offset..end,
        &format!(
            "#{}=(LENGTH_UNIT() NAMED_UNIT(*) SI_UNIT($,.METRE.));",
            unit.id
        ),
    );
    let metric_data = inventory(&metres, Limits::default()).unwrap();
    let metric_binding = solid_kernel::step_body::BodyContext::read(&metric_data).unwrap();
    let metric = metric_binding
        .read_spline_curve(
            &metric_data,
            metric_data.entity_reference(edge.id).unwrap(),
            CurveLimits::default(),
        )
        .unwrap();
    assert_eq!(metric.millimetres_per_source_unit, 1000.);
    assert_eq!(metric.curve.domain(), declaration.curve.domain());
    for parameter in declaration.curve.domain() {
        assert_eq!(
            metric.curve.evaluate(parameter).unwrap(),
            declaration
                .curve
                .evaluate(parameter)
                .unwrap()
                .map(|x| x * 1000.)
        );
    }
    // A cyclic 3D wrapper must refuse before reaching either geometry reader.
    let mut cycle = changed.clone();
    let current = &data.entities()[&line.id];
    let end = current.offset + cycle[current.offset..].find(';').unwrap() + 1;
    cycle.replace_range(
        current.offset..end,
        &format!("#{}=SURFACE_CURVE('',#{},(),.CURVE_3D.);", line.id, line.id),
    );
    let cyclic = inventory(&cycle, Limits::default()).unwrap();
    let cyclic_binding = solid_kernel::step_body::BodyContext::read(&cyclic).unwrap();
    assert!(cyclic_binding
        .read_spline_curve(
            &cyclic,
            cyclic.entity_reference(edge.id).unwrap(),
            CurveLimits::default()
        )
        .unwrap_err()
        .detail
        .contains("cyclic/deep"));
    let original = solid_kernel::analytic_step::AnalyticModel::read(
        &inventory(source, Limits::default()).unwrap(),
        solid_kernel::analytic_step::Options::default(),
    )
    .unwrap();
    let imported = solid_kernel::analytic_step::AnalyticModel::read(
        &data,
        solid_kernel::analytic_step::Options::default(),
    )
    .unwrap();
    assert_eq!(imported.edge_points(edge.id), original.edge_points(edge.id));
    let surface = imported
        .sampled_surface(solid_kernel::native_surface::Limits::default(), |_| true)
        .unwrap();
    let reference_surface = original
        .sampled_surface(solid_kernel::native_surface::Limits::default(), |_| true)
        .unwrap();
    assert_eq!(surface.mesh, reference_surface.mesh);
    let checked = surface.check_intersections(1_000_000, |_| true).unwrap();
    assert!(!checked.surface().mesh.triangles.is_empty());
}

#[test]
fn invalid_degree_cannot_override_mathematical_admission_with_a_relaxed_resource_cap() {
    let source =
        file(&RATIONAL.replace("B_SPLINE_CURVE(2,", "B_SPLINE_CURVE(9223372036854775807,"));
    let data = inventory(&source, Limits::default()).unwrap();
    let result = std::panic::catch_unwind(|| {
        read_curve(
            &data,
            1,
            1.,
            CurveLimits {
                degree: usize::MAX,
                ..CurveLimits::default()
            },
        )
    });
    assert!(
        result.is_ok(),
        "invalid degree must return an error before allocation"
    );
    assert_eq!(result.unwrap().unwrap_err().kind, ErrorKind::Geometry);
}
