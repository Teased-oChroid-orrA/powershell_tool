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
