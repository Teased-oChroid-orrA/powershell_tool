use solid_kernel::{
    analytic_step::{AnalyticModel, Options, Surface},
    geometry_bounds::Bounds3,
    step::{inventory, ErrorKind, Limits, Value},
    step_body::BodyContext,
    step_cylinder_map::CylinderParameterMap,
};
const SOURCE: &str = include_str!("../../nema17-motor-bracket.step");
fn contains(bounds: Bounds3, point: [f64; 3]) {
    for i in 0..3 {
        assert!(
            bounds.min[i] <= point[i] && point[i] <= bounds.max[i],
            "{point:?} {bounds:?}"
        );
    }
}
fn replace(s: &str, offset: usize, record: &str) -> String {
    let end = offset + s[offset..].find(';').unwrap() + 1;
    format!("{}{}{}", &s[..offset], record, &s[end..])
}
#[test]
fn raw_cylinder_angles_and_lengths_bound_actual_fixture_and_whole_rectangles() {
    let data = inventory(SOURCE, Limits::default()).unwrap();
    let context = BodyContext::read(&data).unwrap();
    let model = AnalyticModel::read(&data, Options::default()).unwrap();
    let mut count = 0;
    for face in context.body().faces.values() {
        let Some(Surface::Cylinder { radius, .. }) = model.surface(face.id) else {
            continue;
        };
        let map =
            CylinderParameterMap::read(&context, &data, data.entity_reference(face.id).unwrap())
                .unwrap();
        assert_eq!(map.face_reference().entity, face.id);
        assert_eq!(map.surface_reference().entity, face.surface);
        assert_eq!(map.length_context(), context.length_context());
        assert!(data.resolve_reference(map.angle_unit_reference()).is_ok());
        for u in [-8., -6.28318530718, -1., 0., 1., 6.28318530718, 8.] {
            for v in [-10., 0., 10.] {
                let bounds = map.point_bounds([u, v]).unwrap();
                contains(
                    bounds,
                    model
                        .surface(face.id)
                        .unwrap()
                        .point([u * radius, v])
                        .unwrap(),
                );
                assert!(bounds
                    .min
                    .iter()
                    .zip(bounds.max)
                    .all(|(lo, hi)| hi - lo < 1e-7));
            }
        }
        let whole = map.bounds([[-8., 8.], [-10., 10.]]).unwrap();
        for i in 0..65 {
            let u = -8. + 16. * i as f64 / 64.;
            contains(
                whole,
                model
                    .surface(face.id)
                    .unwrap()
                    .point([u * radius, 5.])
                    .unwrap(),
            );
        }
        for uv in [
            [[0., 9.], [0., 1.]],
            [[1., 0.], [0., 1.]],
            [[0., 1.], [f64::NAN, 1.]],
            [[0., 0.], [1e308, 1e308]],
        ] {
            assert!(map.bounds(uv).is_err());
        }
        count += 1;
    }
    assert!(count > 0);
}
#[test]
fn cylinder_placement_and_metre_conversion_leave_radian_parameters_unscaled() {
    let data = inventory(SOURCE, Limits::default()).unwrap();
    let context = BodyContext::read(&data).unwrap();
    let face = context
        .body()
        .faces
        .values()
        .find(|f| data.entities()[&f.surface].components[0].name == "CYLINDRICAL_SURFACE")
        .unwrap();
    let Value::Ref(frame) = data.entities()[&face.surface].components[0].args[1] else {
        panic!("fixture")
    };
    let placement = &data.entities()[&frame];
    let next = data.entities().keys().last().unwrap() + 1;
    let mut source = replace(
        SOURCE,
        placement.offset,
        &format!(
            "#{frame}=AXIS2_PLACEMENT_3D('',#{next},#{},#{});",
            next + 1,
            next + 2
        ),
    );
    let end = source.rfind("ENDSEC;").unwrap();
    source.insert_str(end,&format!("#{next}=CARTESIAN_POINT('',(1.,2.,3.));#{}=DIRECTION('',(0.,1.,0.));#{}=DIRECTION('',(1.E-310,0.,0.));",next+1,next+2));
    let d = inventory(&source, Limits::default()).unwrap();
    let c = BodyContext::read(&d).unwrap();
    // Only the placement is changed; full body geometry is intentionally not
    // coherent after this fixture edit. Read its unchanged radius from the
    // original valid body rather than asking the body validator to accept it.
    let native = AnalyticModel::read(&data, Options::default()).unwrap();
    let Surface::Cylinder { radius, .. } = native.surface(face.id).unwrap() else {
        panic!("fixture")
    };
    let map = CylinderParameterMap::read(&c, &d, d.entity_reference(face.id).unwrap()).unwrap();
    contains(map.point_bounds([0., 4.]).unwrap(), [1. + radius, 6., 3.]);
    let length = d
        .length_contexts()
        .unwrap()
        .into_iter()
        .find(|l| l.context == c.length_context().entity)
        .unwrap();
    let unit = &d.entities()[&length.unit];
    let metric = replace(
        &source,
        unit.offset,
        &format!(
            "#{}=(LENGTH_UNIT() NAMED_UNIT(*) SI_UNIT($,.METRE.));",
            unit.id
        ),
    );
    let m = inventory(&metric, Limits::default()).unwrap();
    let mc = BodyContext::read(&m).unwrap();
    let scaled = CylinderParameterMap::read(&mc, &m, m.entity_reference(face.id).unwrap()).unwrap();
    assert_eq!(scaled.millimetres_per_source_unit(), 1000.);
    for u in [0., 0.5, 1., 6.28318530718] {
        contains(
            scaled.point_bounds([u, 4.]).unwrap(),
            [
                (1. + radius * u.cos()) * 1000.,
                6000.,
                (3. - radius * u.sin()) * 1000.,
            ],
        );
    }
    assert!(CylinderParameterMap::read(&c, &m, m.entity_reference(face.id).unwrap()).is_err());
    assert!(CylinderParameterMap::read(&mc, &m, d.entity_reference(face.id).unwrap()).is_err());
}
#[test]
fn angular_units_must_be_explicit_unique_unprefixed_radians() {
    let data = inventory(SOURCE, Limits::default()).unwrap();
    let c = BodyContext::read(&data).unwrap();
    let cylinder = c
        .body()
        .faces
        .values()
        .find(|f| data.entities()[&f.surface].components[0].name == "CYLINDRICAL_SURFACE")
        .unwrap()
        .id;
    let map =
        CylinderParameterMap::read(&c, &data, data.entity_reference(cylinder).unwrap()).unwrap();
    let unit = data.resolve_reference(map.angle_unit_reference()).unwrap();
    for record in [
        format!(
            "#{}=(NAMED_UNIT(*) PLANE_ANGLE_UNIT() SI_UNIT(.MILLI.,.RADIAN.));",
            unit.id
        ),
        format!(
            "#{}=(NAMED_UNIT(*) PLANE_ANGLE_UNIT() SI_UNIT($,.DEGREE.));",
            unit.id
        ),
        format!(
            "#{}=(NAMED_UNIT(*) SOLID_ANGLE_UNIT() SI_UNIT($,.STERADIAN.));",
            unit.id
        ),
        format!(
            "#{}=(NAMED_UNIT(*) PLANE_ANGLE_UNIT() PLANE_ANGLE_UNIT());",
            unit.id
        ),
    ] {
        let changed = inventory(&replace(SOURCE, unit.offset, &record), Limits::default()).unwrap();
        let context = BodyContext::read(&changed).unwrap();
        let error = CylinderParameterMap::read(
            &context,
            &changed,
            changed.entity_reference(cylinder).unwrap(),
        )
        .unwrap_err();
        assert_eq!(error.kind, ErrorKind::Units);
    }
    let plane = c
        .body()
        .faces
        .values()
        .find(|f| data.entities()[&f.surface].components[0].name == "PLANE")
        .unwrap();
    assert!(
        CylinderParameterMap::read(&c, &data, data.entity_reference(plane.id).unwrap()).is_err()
    );
}

fn generator_cases(d: &solid_kernel::step::Inventory, c: &BodyContext) -> Vec<(u64, u64, u64)> {
    cylinder_cases(d, c, "LINE")
}
fn cylinder_cases(
    d: &solid_kernel::step::Inventory,
    c: &BodyContext,
    kind: &str,
) -> Vec<(u64, u64, u64)> {
    use solid_kernel::step_pcurve::{self, ParametricCurve};
    let mut cases = std::collections::BTreeSet::new();
    for face in c
        .body()
        .faces
        .values()
        .filter(|f| d.entities()[&f.surface].components[0].name == "CYLINDRICAL_SURFACE")
    {
        for usage in face.bounds.iter().flat_map(|b| &b.edges) {
            let edge = &c.body().edges[&usage.edge];
            let wrapper =
                step_pcurve::read_surface_curve(d, d.entity_reference(edge.curve).unwrap())
                    .unwrap();
            if d.entities()[&wrapper.geometry_3d.entity].components[0].name != kind {
                continue;
            }
            for pc in wrapper.pcurves {
                let p = step_pcurve::read_pcurve(d, pc, solid_kernel::nurbs::Limits::default())
                    .unwrap();
                if p.basis_surface.entity == face.surface
                    && matches!(p.geometry, ParametricCurve::Line { .. })
                {
                    cases.insert((face.id, edge.id, pc.entity));
                }
            }
        }
    }
    cases.into_iter().collect()
}
#[test]
fn cylinder_generator_pcurves_agree_with_finite_source_lines() {
    let d = inventory(SOURCE, Limits::default()).unwrap();
    let c = BodyContext::read(&d).unwrap();
    let cases = generator_cases(&d, &c);
    assert!(!cases.is_empty());
    for (face, edge, pc) in &cases {
        let map = CylinderParameterMap::read(&c, &d, d.entity_reference(*face).unwrap()).unwrap();
        let result = map
            .verify_generator_locus(
                &c,
                &d,
                d.entity_reference(*edge).unwrap(),
                d.entity_reference(*pc).unwrap(),
                1e-7,
            )
            .unwrap();
        assert_eq!(result.face.entity, *face);
        assert_eq!(result.edge.entity, *edge);
        assert_eq!(result.pcurve.entity, *pc);
        assert_eq!(
            result.checked_pcurve_is_master,
            result.master_reference == result.pcurve
        );
        assert!(result
            .source_to_3d_error_mm
            .iter()
            .chain(&result.source_to_pcurve_error_mm)
            .chain(&result.curve_to_pcurve_error_mm)
            .all(|x| *x <= 1e-7));
    }
    eprintln!("verified {} cylinder generator associations", cases.len());
}
#[test]
fn nonconstant_raw_u_wrong_generator_and_contradictory_sense_are_refused() {
    use solid_kernel::step_pcurve::{self, ParametricCurve};
    let d = inventory(SOURCE, Limits::default()).unwrap();
    let c = BodyContext::read(&d).unwrap();
    let (face, edge, pc) = generator_cases(&d, &c)[0];
    let p = step_pcurve::read_pcurve(
        &d,
        d.entity_reference(pc).unwrap(),
        solid_kernel::nurbs::Limits::default(),
    )
    .unwrap();
    let ParametricCurve::Line { origin, .. } = p.geometry else {
        panic!("fixture")
    };
    let line = d.resolve_reference(p.geometry_source).unwrap();
    let Value::Ref(point) = line.components[0].args[1] else {
        panic!("fixture")
    };
    let Value::Ref(vector) = line.components[0].args[2] else {
        panic!("fixture")
    };
    let Value::Ref(direction) = d.entities()[&vector].components[0].args[1] else {
        panic!("fixture")
    };
    let edge_entity = &d.entities()[&edge];
    let edge_record = &SOURCE[edge_entity.offset
        ..edge_entity.offset + SOURCE[edge_entity.offset..].find(';').unwrap() + 1];
    let sense = if c.body().edges[&edge].same_sense {
        edge_record.replace(".T.", ".F.")
    } else {
        edge_record.replace(".F.", ".T.")
    };
    for changed in [
        replace(
            SOURCE,
            d.entities()[&point].offset,
            &format!(
                "#{point}=CARTESIAN_POINT('',({:.17e},{:.17e}));",
                origin[0] + 0.1,
                origin[1]
            ),
        ),
        // A tiny nonzero U ratio is still a helix, even if display rounding
        // makes its effect visually indistinguishable from a generator.
        replace(
            SOURCE,
            d.entities()[&direction].offset,
            &format!("#{direction}=DIRECTION('',(1.E-310,1.));"),
        ),
        replace(SOURCE, edge_entity.offset, &sense),
    ] {
        let data = inventory(&changed, Limits::default()).unwrap();
        let context = BodyContext::read(&data).unwrap();
        let map = CylinderParameterMap::read(&context, &data, data.entity_reference(face).unwrap())
            .unwrap();
        assert!(map
            .verify_generator_locus(
                &context,
                &data,
                data.entity_reference(edge).unwrap(),
                data.entity_reference(pc).unwrap(),
                1e-7
            )
            .is_err());
    }
    let stale = inventory(&format!("{SOURCE}\n"), Limits::default()).unwrap();
    let sc = BodyContext::read(&stale).unwrap();
    let map = CylinderParameterMap::read(&c, &d, d.entity_reference(face).unwrap()).unwrap();
    assert!(map
        .verify_generator_locus(
            &sc,
            &stale,
            stale.entity_reference(edge).unwrap(),
            stale.entity_reference(pc).unwrap(),
            1e-7
        )
        .is_err());
}

#[test]
fn cylinder_circular_pcurves_agree_with_whole_source_circle_carriers() {
    let d = inventory(SOURCE, Limits::default()).unwrap();
    let c = BodyContext::read(&d).unwrap();
    let cases = cylinder_cases(&d, &c, "CIRCLE");
    assert_eq!(cases.len(), 24);
    for (face, edge, pc) in cases {
        let map = CylinderParameterMap::read(&c, &d, d.entity_reference(face).unwrap()).unwrap();
        let report = map
            .verify_circle_carrier(
                &c,
                &d,
                d.entity_reference(edge).unwrap(),
                d.entity_reference(pc).unwrap(),
                1e-7,
            )
            .unwrap();
        assert_eq!(report.face.entity, face);
        assert_eq!(report.edge.entity, edge);
        assert_eq!(report.pcurve.entity, pc);
        assert!(report.hausdorff_upper_mm <= 1e-7);
        assert!(map
            .verify_generator_locus(
                &c,
                &d,
                d.entity_reference(edge).unwrap(),
                d.entity_reference(pc).unwrap(),
                1e-7
            )
            .is_err());
    }
}
#[test]
fn cylinder_circular_pcurves_reject_axial_shifts_and_nonconstant_raw_v() {
    use solid_kernel::step_pcurve::{self, ParametricCurve};
    let d = inventory(SOURCE, Limits::default()).unwrap();
    let c = BodyContext::read(&d).unwrap();
    let (face, edge, pc) = cylinder_cases(&d, &c, "CIRCLE")[0];
    let p = step_pcurve::read_pcurve(
        &d,
        d.entity_reference(pc).unwrap(),
        solid_kernel::nurbs::Limits::default(),
    )
    .unwrap();
    let ParametricCurve::Line { origin, .. } = p.geometry else {
        panic!("fixture")
    };
    let line = d.resolve_reference(p.geometry_source).unwrap();
    let Value::Ref(point) = line.components[0].args[1] else {
        panic!("fixture")
    };
    let Value::Ref(vector) = line.components[0].args[2] else {
        panic!("fixture")
    };
    let Value::Ref(direction) = d.entities()[&vector].components[0].args[1] else {
        panic!("fixture")
    };
    for changed in [
        replace(
            SOURCE,
            d.entities()[&point].offset,
            &format!(
                "#{point}=CARTESIAN_POINT('',({:.17e},{:.17e}));",
                origin[0],
                origin[1] + 1.
            ),
        ),
        replace(
            SOURCE,
            d.entities()[&direction].offset,
            &format!("#{direction}=DIRECTION('',(1.,1.E-310));"),
        ),
    ] {
        let data = inventory(&changed, Limits::default()).unwrap();
        let context = BodyContext::read(&data).unwrap();
        let map = CylinderParameterMap::read(&context, &data, data.entity_reference(face).unwrap())
            .unwrap();
        assert!(map
            .verify_circle_carrier(
                &context,
                &data,
                data.entity_reference(edge).unwrap(),
                data.entity_reference(pc).unwrap(),
                1e-7
            )
            .is_err());
    }
    let map = CylinderParameterMap::read(&c, &d, d.entity_reference(face).unwrap()).unwrap();
    for tol in [0., -1., f64::NAN] {
        assert!(map
            .verify_circle_carrier(
                &c,
                &d,
                d.entity_reference(edge).unwrap(),
                d.entity_reference(pc).unwrap(),
                tol
            )
            .is_err());
    }
}
