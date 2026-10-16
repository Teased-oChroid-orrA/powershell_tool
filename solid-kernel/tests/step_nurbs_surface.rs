use solid_kernel::{
    nurbs_surface::Limits as SurfaceLimits,
    step::{inventory, ErrorKind, Limits},
    step_nurbs::Logical,
    step_nurbs_surface::read_surface,
};
fn file(surface: &str) -> String {
    format!("ISO-10303-21; HEADER; FILE_DESCRIPTION(('surface-test'),'2;1'); FILE_NAME('fixture','',(''),(''),'','',''); FILE_SCHEMA(('TEST')); ENDSEC; DATA; {surface} #2=CARTESIAN_POINT('',(1.,0.,0.)); #3=CARTESIAN_POINT('',(1.,0.,3.)); #4=CARTESIAN_POINT('',(1.,1.,0.)); #5=CARTESIAN_POINT('',(1.,1.,3.)); #6=CARTESIAN_POINT('',(0.,1.,0.)); #7=CARTESIAN_POINT('',(0.,1.,3.)); ENDSEC; END-ISO-10303-21;")
}
const RATIONAL:&str="#1=(BOUNDED_SURFACE() B_SPLINE_SURFACE(2,1,((#2,#3),(#4,#5),(#6,#7)),.CYLINDRICAL_SURF.,.F.,.F.,.U.) B_SPLINE_SURFACE_WITH_KNOTS((3,3),(2,2),(0.,1.),(0.,1.),.QUASI_UNIFORM_KNOTS.) SURFACE() GEOMETRIC_REPRESENTATION_ITEM() RATIONAL_B_SPLINE_SURFACE(((1.,1.),(0.7071067811865476,0.7071067811865476),(1.,1.))) REPRESENTATION_ITEM('quarter-cylinder'));";
#[test]
fn rational_surface_maps_step_u_rows_to_evaluator_v_major_with_source_ids_and_units() {
    let data = inventory(&file(RATIONAL), Limits::default()).unwrap();
    let declaration = read_surface(&data, 1, 25.4, SurfaceLimits::default()).unwrap();
    assert_eq!(declaration.surface.dimensions(), [3, 2]);
    assert_eq!(declaration.surface.degrees(), [2, 1]);
    assert_eq!(declaration.surface.domain(), [[0., 1.], [0., 1.]]);
    assert_eq!(
        declaration
            .control_point_sources
            .iter()
            .map(|r| r.entity)
            .collect::<Vec<_>>(),
        vec![2, 4, 6, 3, 5, 7]
    );
    assert_eq!(
        declaration.control_point_reference([1, 1]).unwrap().entity,
        5
    );
    assert!(declaration.control_point_reference([3, 0]).is_none());
    assert!(declaration.control_point_reference([0, 2]).is_none());
    assert_eq!(declaration.source, data.entity_reference(1).unwrap());
    assert_eq!(declaration.declared_name, "quarter-cylinder");
    assert_eq!(declaration.declared_form, "CYLINDRICAL_SURF");
    assert_eq!(declaration.declared_u_closed, Logical::False);
    assert_eq!(declaration.declared_v_closed, Logical::False);
    assert_eq!(declaration.declared_self_intersection, Logical::Unknown);
    assert_eq!(declaration.declared_knot_spec, "QUASI_UNIFORM_KNOTS");
    assert_eq!(declaration.millimetres_per_source_unit, 25.4);
    for reference in &declaration.control_point_sources {
        assert_eq!(
            data.resolve_reference(*reference).unwrap().components[0].name,
            "CARTESIAN_POINT"
        );
    }
    for u in [0., 0.25, 0.5, 0.75, 1.] {
        for v in [0., 0.25, 0.75, 1.] {
            let p = declaration.surface.evaluate([u, v]).unwrap();
            assert!((p[0].hypot(p[1]) - 25.4).abs() / 25.4 < 1e-12);
            assert!((p[2] - 3. * 25.4 * v).abs() < 1e-12);
        }
    }
}
#[test]
fn polynomial_variants_preserve_flags_without_inferring_the_declared_surface_form() {
    let simple="#1=B_SPLINE_SURFACE_WITH_KNOTS('polynomial',1,1,((#2,#3),(#6,#7)),.UNSPECIFIED.,.U.,.U.,.U.,(2,2),(2,2),(0.,1.),(0.,1.),.UNSPECIFIED.);";
    let data = inventory(&file(simple), Limits::default()).unwrap();
    let declaration = read_surface(&data, 1, 1., SurfaceLimits::default()).unwrap();
    assert_eq!(declaration.surface.weights(), &[1.; 4]);
    assert_eq!(
        declaration.surface.evaluate([0.25, 0.75]).unwrap(),
        [0.75, 0.25, 2.25]
    );
    assert_eq!(declaration.declared_u_closed, Logical::Unknown);
    assert_eq!(declaration.declared_v_closed, Logical::Unknown);
    let polynomial = RATIONAL.replace(
        " RATIONAL_B_SPLINE_SURFACE(((1.,1.),(0.7071067811865476,0.7071067811865476),(1.,1.)))",
        "",
    );
    let data = inventory(&file(&polynomial), Limits::default()).unwrap();
    let declaration = read_surface(&data, 1, 1., SurfaceLimits::default()).unwrap();
    assert_eq!(declaration.declared_form, "CYLINDRICAL_SURF");
    assert_eq!(
        declaration.surface.evaluate([0.5, 0.5]).unwrap(),
        [0.75, 0.75, 1.5]
    );
}
#[test]
fn malformed_matrices_components_knots_and_aggregate_budgets_refuse() {
    for source in [
        RATIONAL.replace("((#2,#3),(#4,#5),(#6,#7))", "((#2,#3),(#4),(#6,#7))"),
        RATIONAL.replace(
            "((1.,1.),(0.7071067811865476,0.7071067811865476),(1.,1.))",
            "((1.,1.),(1.,1.))",
        ),
        RATIONAL.replace("(3,3)", "(9223372036854775807,3)"),
        RATIONAL.replace(" SURFACE()", " SURFACE() SURFACE()"),
        RATIONAL.replace("GEOMETRIC_REPRESENTATION_ITEM()", "UNKNOWN_COMPONENT()"),
        RATIONAL.replace(" SURFACE()", ""),
        RATIONAL.replace(".CYLINDRICAL_SURF.", ".UNKNOWN_FORM."),
        RATIONAL.replace(".F.,.F.,.U.", ".F.,.INVALID.,.U."),
        RATIONAL.replace("0.7071067811865476", "0."),
        RATIONAL.replace("(#4,#5)", "(#4,1.)"),
    ] {
        let data = inventory(&file(&source), Limits::default()).unwrap();
        assert!(
            read_surface(&data, 1, 1., SurfaceLimits::default()).is_err(),
            "{source}"
        );
    }
    let data = inventory(&file(RATIONAL), Limits::default()).unwrap();
    for limits in [
        SurfaceLimits {
            control_points: 5,
            ..SurfaceLimits::default()
        },
        SurfaceLimits {
            degree: 1,
            ..SurfaceLimits::default()
        },
        SurfaceLimits {
            evaluation_blends: 1,
            ..SurfaceLimits::default()
        },
    ] {
        assert_eq!(
            read_surface(&data, 1, 1., limits).unwrap_err().kind,
            ErrorKind::Limit
        );
    }
    assert_eq!(
        read_surface(&data, 999, 1., SurfaceLimits::default())
            .unwrap_err()
            .kind,
        ErrorKind::DanglingReference
    );
    assert!(solid_kernel::step_nurbs::read_curve(
        &data,
        1,
        1.,
        solid_kernel::nurbs::Limits::default()
    )
    .is_err());
}
#[test]
fn source_control_underflow_and_invalid_scale_refuse_at_the_original_record() {
    let source = file(RATIONAL).replace("(1.,0.,0.)", "(1.E-310,0.,0.)");
    let data = inventory(&source, Limits::default()).unwrap();
    let error = read_surface(&data, 1, 1e-21, SurfaceLimits::default()).unwrap_err();
    assert!(error.detail.contains("underflow"));
    assert_eq!(error.offset, data.entities()[&2].offset);
    let data = inventory(&file(RATIONAL), Limits::default()).unwrap();
    for scale in [0., -1., f64::NAN, f64::INFINITY] {
        assert!(read_surface(&data, 1, scale, SurfaceLimits::default()).is_err());
    }
}
#[test]
fn matching_planar_spline_declaration_does_not_enable_partial_native_body_import() {
    use solid_kernel::analytic_step::{AnalyticModel, Options};
    let source = include_str!("../../nema17-motor-bracket.step");
    let data = inventory(source, Limits::default()).unwrap();
    let model = AnalyticModel::read(&data, Options::default()).unwrap();
    assert_eq!(model.millimetres_per_source_unit(), 1.);
    let face = model
        .body()
        .faces
        .values()
        .find(|f| data.entities()[&f.surface].components[0].name == "PLANE")
        .unwrap();
    let plane = model.surface(face.id).unwrap();
    let entity = &data.entities()[&face.surface];
    let first = data.entities().keys().last().unwrap() + 1;
    let uv = [
        [-1000., -1000.],
        [-1000., 1000.],
        [1000., -1000.],
        [1000., 1000.],
    ];
    let mut records = String::new();
    for (i, parameters) in uv.iter().enumerate() {
        let p = plane.point(*parameters).unwrap();
        records.push_str(&format!(
            "#{}=CARTESIAN_POINT('',({:.17e},{:.17e},{:.17e}));\n",
            first + i as u64,
            p[0],
            p[1],
            p[2]
        ));
    }
    let mut changed = source.to_string();
    let end = source[entity.offset..].find(';').unwrap() + entity.offset + 1;
    changed.replace_range(entity.offset..end,&format!("#{}=B_SPLINE_SURFACE_WITH_KNOTS('plane',1,1,((#{},#{}),(#{},#{})),.PLANE_SURF.,.F.,.F.,.U.,(2,2),(2,2),(-1000.,1000.),(-1000.,1000.),.UNSPECIFIED.);",entity.id,first,first+1,first+2,first+3));
    let end = changed.rfind("ENDSEC;").unwrap();
    changed.insert_str(end, &records);
    let data = inventory(&changed, Limits::default()).unwrap();
    let declaration = read_surface(&data, entity.id, 1., SurfaceLimits::default()).unwrap();
    for parameters in [[0., 0.], [0.25, -0.75], [-20., 30.]] {
        let expected = plane.point(parameters).unwrap();
        let actual = declaration.surface.evaluate(parameters).unwrap();
        for axis in 0..3 {
            assert!((expected[axis] - actual[axis]).abs() < 1e-12);
        }
    }
    let error = AnalyticModel::read(&data, Options::default()).unwrap_err();
    assert_eq!(error.detail, "unsupported native analytic surface");
    assert_eq!(error.offset, data.entities()[&entity.id].offset);
}

#[test]
fn invalid_degree_cannot_override_mathematical_admission_with_a_relaxed_resource_cap() {
    let source = file(&RATIONAL.replace(
        "B_SPLINE_SURFACE(2,1,",
        "B_SPLINE_SURFACE(9223372036854775807,1,",
    ));
    let data = inventory(&source, Limits::default()).unwrap();
    let result = std::panic::catch_unwind(|| {
        read_surface(
            &data,
            1,
            1.,
            SurfaceLimits {
                degree: usize::MAX,
                ..SurfaceLimits::default()
            },
        )
    });
    assert!(
        result.is_ok(),
        "invalid degree must return an error before allocation"
    );
    assert_eq!(result.unwrap().unwrap_err().kind, ErrorKind::Geometry);
}
