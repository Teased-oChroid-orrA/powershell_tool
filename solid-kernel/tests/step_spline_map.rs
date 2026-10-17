use solid_kernel::{
    geometry_bounds::Bounds3,
    nurbs, nurbs_surface,
    step::{inventory, ErrorKind, Inventory, Limits, Value},
    step_body::BodyContext,
    step_spline_map::SplineParameterMap,
};
const SOURCE: &str = include_str!("../../nema17-motor-bracket.step");
fn replace(s: &str, offset: usize, record: &str) -> String {
    let end = offset + s[offset..].find(';').unwrap() + 1;
    format!("{}{}{}", &s[..offset], record, &s[end..])
}
fn fixture() -> (String, u64, [u64; 3]) {
    let d = inventory(SOURCE, Limits::default()).unwrap();
    let c = BodyContext::read(&d).unwrap();
    let face = c
        .body()
        .faces
        .values()
        .find(|f| d.entities()[&f.surface].components[0].name == "PLANE")
        .unwrap();
    let mut next = d.entities().keys().last().unwrap() + 1;
    let mut alloc = || {
        let n = next;
        next += 1;
        n
    };
    let cp: [u64; 6] = std::array::from_fn(|_| alloc());
    let pc: [u64; 3] = std::array::from_fn(|_| alloc());
    let reps: [u64; 3] = std::array::from_fn(|_| alloc());
    let curves: [u64; 3] = std::array::from_fn(|_| alloc());
    let context = alloc();
    let linepoint = alloc();
    let linedirection = alloc();
    let vector = alloc();
    let circlepoint = alloc();
    let circleframe = alloc();
    let uv: [u64; 3] = std::array::from_fn(|_| alloc());
    let coords = [
        [1., 0., 0.],
        [1., 0., 3.],
        [1., 1., 0.],
        [1., 1., 3.],
        [0., 1., 0.],
        [0., 1., 3.],
    ];
    let mut records = String::new();
    for (id, p) in cp.into_iter().zip(coords) {
        records.push_str(&format!(
            "#{id}=CARTESIAN_POINT('',({},{},{}));",
            p[0], p[1], p[2]
        ));
    }
    records.push_str(&format!("#{context}=(GEOMETRIC_REPRESENTATION_CONTEXT(2) PARAMETRIC_REPRESENTATION_CONTEXT() REPRESENTATION_CONTEXT('raw',''));
#{linepoint}=CARTESIAN_POINT('',(0.25,0.25));#{linedirection}=DIRECTION('',(1.E-310,0.));#{vector}=VECTOR('',#{linedirection},0.25);
#{}=LINE('',#{linepoint},#{vector});#{circlepoint}=CARTESIAN_POINT('',(0.5,0.5));#{circleframe}=AXIS2_PLACEMENT_2D('',#{circlepoint},$);#{}=CIRCLE('',#{circleframe},0.125);",curves[0],curves[1]));
    for (id, p) in uv
        .into_iter()
        .zip([[0.75, 0.25], [0.75, 0.75], [0.25, 0.75]])
    {
        records.push_str(&format!("#{id}=CARTESIAN_POINT('',({},{}));", p[0], p[1]));
    }
    records.push_str(&format!("#{}=(BOUNDED_CURVE() B_SPLINE_CURVE(2,(#{},#{},#{}),.UNSPECIFIED.,.F.,.F.) B_SPLINE_CURVE_WITH_KNOTS((3,3),(0.,1.),.UNSPECIFIED.) CURVE() GEOMETRIC_REPRESENTATION_ITEM() RATIONAL_B_SPLINE_CURVE((1.,0.7071067811865476,1.)) REPRESENTATION_ITEM('')); ",curves[2],uv[0],uv[1],uv[2]));
    for i in 0..3 {
        records.push_str(&format!(
            "#{}=PCURVE('',#{},#{});#{}=DEFINITIONAL_REPRESENTATION('',(#{}),#{context});",
            pc[i], face.surface, reps[i], reps[i], curves[i]
        ));
    }
    let surface=format!("#{}=(BOUNDED_SURFACE() B_SPLINE_SURFACE(2,1,((#{},#{}),(#{},#{}),(#{},#{})),.CYLINDRICAL_SURF.,.F.,.F.,.U.) B_SPLINE_SURFACE_WITH_KNOTS((3,3),(2,2),(0.,1.),(0.,1.),.UNSPECIFIED.) SURFACE() GEOMETRIC_REPRESENTATION_ITEM() RATIONAL_B_SPLINE_SURFACE(((1.,1.),(0.7071067811865476,0.7071067811865476),(1.,1.))) REPRESENTATION_ITEM(''));",face.surface,cp[0],cp[1],cp[2],cp[3],cp[4],cp[5]);
    let mut source = replace(SOURCE, d.entities()[&face.surface].offset, &surface);
    let end = source.rfind("ENDSEC;").unwrap();
    source.insert_str(end, &records);
    // Raw declaration/body binding fixture only: the changed face and new
    // parameter curves are not claimed to form a coherent edited solid.
    (source, face.id, pc)
}
fn contains(b: Bounds3, p: [f64; 3]) {
    for i in 0..3 {
        assert!(b.min[i] <= p[i] && p[i] <= b.max[i], "{b:?} {p:?}");
    }
}
fn surface_point([u, v]: [f64; 2]) -> [f64; 3] {
    let w = 0.7071067811865476;
    let a = (1. - u) * (1. - u);
    let b = 2. * w * u * (1. - u);
    let c = u * u;
    let d = a + b + c;
    [(a + b) / d, (b + c) / d, 3. * v]
}
fn parameter_point(kind: usize, t: f64) -> [f64; 2] {
    match kind {
        0 => [0.25 + 0.25 * t, 0.25],
        1 => [0.5 + 0.125 * t.cos(), 0.5 + 0.125 * t.sin()],
        _ => {
            let w = 0.7071067811865476;
            let a = (1. - t) * (1. - t);
            let b = 2. * w * t * (1. - t);
            let c = t * t;
            let d = a + b + c;
            [
                (0.75 * a + 0.75 * b + 0.25 * c) / d,
                (0.25 * a + 0.75 * b + 0.75 * c) / d,
            ]
        }
    }
}
#[test]
fn whole_parameter_intervals_map_line_circle_and_rational_spline_to_curved_surface() {
    let (source, face, pc) = fixture();
    let d = inventory(&source, Limits::default()).unwrap();
    let c = BodyContext::read(&d).unwrap();
    let map = SplineParameterMap::read(
        &c,
        &d,
        d.entity_reference(face).unwrap(),
        nurbs_surface::Limits::default(),
    )
    .unwrap();
    assert_eq!(map.geometry_source_units().domain(), [[0., 1.], [0., 1.]]);
    assert_eq!(map.face_reference().entity, face);
    assert_eq!(map.length_context(), c.length_context());
    for (kind, id) in pc.into_iter().enumerate() {
        let end = if kind == 1 { 6.28318530718 } else { 1. };
        for part in 0..8 {
            let a = end * part as f64 / 8.;
            let b = end * (part + 1) as f64 / 8.;
            let bounds = map
                .pcurve_bounds(
                    &d,
                    d.entity_reference(id).unwrap(),
                    [a, b],
                    nurbs::Limits::default(),
                    10,
                    10,
                )
                .unwrap();
            for j in 0..17 {
                let t = a + (b - a) * j as f64 / 16.;
                contains(bounds, surface_point(parameter_point(kind, t)));
            }
        }
    }
    contains(
        map.point_bounds([0.25, 0.5], 1).unwrap(),
        surface_point([0.25, 0.5]),
    );
}
#[test]
fn source_unit_conversion_scales_world_bounds_once_without_changing_uv_or_knots() {
    let (source, face, pc) = fixture();
    let d = inventory(&source, Limits::default()).unwrap();
    let c = BodyContext::read(&d).unwrap();
    let unit = d
        .length_contexts()
        .unwrap()
        .into_iter()
        .find(|l| l.context == c.length_context().entity)
        .unwrap()
        .unit;
    let metric = replace(
        &source,
        d.entities()[&unit].offset,
        &format!("#{unit}=(LENGTH_UNIT() NAMED_UNIT(*) SI_UNIT($,.METRE.));"),
    );
    let m = inventory(&metric, Limits::default()).unwrap();
    let mc = BodyContext::read(&m).unwrap();
    let map = SplineParameterMap::read(
        &mc,
        &m,
        m.entity_reference(face).unwrap(),
        nurbs_surface::Limits::default(),
    )
    .unwrap();
    assert_eq!(map.millimetres_per_source_unit(), 1000.);
    assert_eq!(map.geometry_source_units().domain(), [[0., 1.], [0., 1.]]);
    assert_eq!(
        map.geometry_source_units().control_points()[0],
        [1., 0., 0.]
    );
    for (kind, id) in pc.into_iter().enumerate() {
        let t = 0.5;
        let bounds = map
            .pcurve_bounds(
                &m,
                m.entity_reference(id).unwrap(),
                [t, t],
                nurbs::Limits::default(),
                10,
                10,
            )
            .unwrap();
        contains(
            bounds,
            surface_point(parameter_point(kind, t)).map(|x| x * 1000.),
        );
    }
    assert!(SplineParameterMap::read(
        &c,
        &m,
        m.entity_reference(face).unwrap(),
        nurbs_surface::Limits::default()
    )
    .is_err());
    assert!(map
        .pcurve_bounds(
            &d,
            d.entity_reference(pc[0]).unwrap(),
            [0., 1.],
            nurbs::Limits::default(),
            10,
            10
        )
        .is_err());
}
#[test]
fn domains_source_bindings_and_both_curve_surface_work_budgets_refuse() {
    let (source, face, pc) = fixture();
    let d = inventory(&source, Limits::default()).unwrap();
    let c = BodyContext::read(&d).unwrap();
    let map = SplineParameterMap::read(
        &c,
        &d,
        d.entity_reference(face).unwrap(),
        nurbs_surface::Limits::default(),
    )
    .unwrap();
    assert_eq!(
        map.pcurve_bounds(
            &d,
            d.entity_reference(pc[2]).unwrap(),
            [0., 1.],
            nurbs::Limits::default(),
            0,
            10
        )
        .unwrap_err()
        .kind,
        ErrorKind::Limit
    );
    assert_eq!(
        map.pcurve_bounds(
            &d,
            d.entity_reference(pc[0]).unwrap(),
            [0., 1.],
            nurbs::Limits::default(),
            10,
            0
        )
        .unwrap_err()
        .kind,
        ErrorKind::Limit
    );
    assert_eq!(
        map.pcurve_bounds(
            &d,
            d.entity_reference(pc[2]).unwrap(),
            [0., 1.],
            nurbs::Limits {
                control_points: 2,
                ..nurbs::Limits::default()
            },
            10,
            10
        )
        .unwrap_err()
        .kind,
        ErrorKind::Limit
    );
    for interval in [[1., 0.], [0., f64::NAN], [-3., 3.]] {
        assert!(map
            .pcurve_bounds(
                &d,
                d.entity_reference(pc[0]).unwrap(),
                interval,
                nurbs::Limits::default(),
                10,
                10
            )
            .is_err());
    }
    assert!(map
        .pcurve_bounds(
            &d,
            d.entity_reference(pc[1]).unwrap(),
            [0., 9.],
            nurbs::Limits::default(),
            10,
            10
        )
        .is_err());
    let other=d.entities().values().find(|e|e.components[0].name=="PCURVE" && matches!(e.components[0].args[1],Value::Ref(s) if s!=map.surface_reference().entity)).unwrap();
    assert!(map
        .pcurve_bounds(
            &d,
            d.entity_reference(other.id).unwrap(),
            [0., 1.],
            nurbs::Limits::default(),
            10,
            10
        )
        .is_err());
    assert!(SplineParameterMap::read(
        &c,
        &d,
        d.entity_reference(c.body().id).unwrap(),
        nurbs_surface::Limits::default()
    )
    .is_err());
}
