use solid_kernel::{
    geometry_bounds::Bounds3,
    nurbs, nurbs_surface,
    step::{inventory, ErrorKind, Limits, Value},
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

#[test]
fn exact_domain_boundary_parameter_curves_do_not_spill_outside_uv_domain() {
    let (source, face, pc) = fixture();
    let data = inventory(&source, Limits::default()).unwrap();
    let declaration = solid_kernel::step_pcurve::read_pcurve(
        &data,
        data.entity_reference(pc[0]).unwrap(),
        nurbs::Limits::default(),
    )
    .unwrap();
    let line = data.resolve_reference(declaration.geometry_source).unwrap();
    let Value::Ref(point) = line.components[0].args[1] else {
        panic!("fixture")
    };
    let Value::Ref(vector) = line.components[0].args[2] else {
        panic!("fixture")
    };
    let Value::Ref(direction) = data.entities()[&vector].components[0].args[1] else {
        panic!("fixture")
    };
    for reversed in [false, true] {
        let mut edits = vec![
            (
                point,
                format!(
                    "#{point}=CARTESIAN_POINT('',({},0.));",
                    if reversed { 1. } else { 0. }
                ),
            ),
            (vector, format!("#{vector}=VECTOR('',#{direction},1.);")),
            (
                direction,
                format!(
                    "#{direction}=DIRECTION('',({},0.));",
                    if reversed { "-1.E-310" } else { "1.E-310" }
                ),
            ),
        ];
        edits.sort_by_key(|(id, _)| std::cmp::Reverse(data.entities()[id].offset));
        let mut changed = source.clone();
        for (id, record) in edits {
            changed = replace(&changed, data.entities()[&id].offset, &record);
        }
        let d = inventory(&changed, Limits::default()).unwrap();
        let c = BodyContext::read(&d).unwrap();
        let map = SplineParameterMap::read(
            &c,
            &d,
            d.entity_reference(face).unwrap(),
            nurbs_surface::Limits::default(),
        )
        .unwrap();
        let bounds = map
            .pcurve_bounds(
                &d,
                d.entity_reference(pc[0]).unwrap(),
                [0., 1.],
                nurbs::Limits::default(),
                10,
                10,
            )
            .unwrap();
        for uv in [[0., 0.], [0.5, 0.], [1., 0.]] {
            contains(bounds, surface_point(uv));
        }
    }
    let declaration = solid_kernel::step_pcurve::read_pcurve(
        &data,
        data.entity_reference(pc[1]).unwrap(),
        nurbs::Limits::default(),
    )
    .unwrap();
    let circle = data.resolve_reference(declaration.geometry_source).unwrap();
    let Value::Ref(frame) = circle.components[0].args[1] else {
        panic!("fixture")
    };
    let changed = replace(
        &source,
        circle.offset,
        &format!("#{}=CIRCLE('',#{frame},0.5);", circle.id),
    );
    let d = inventory(&changed, Limits::default()).unwrap();
    let c = BodyContext::read(&d).unwrap();
    let map = SplineParameterMap::read(
        &c,
        &d,
        d.entity_reference(face).unwrap(),
        nurbs_surface::Limits::default(),
    )
    .unwrap();
    assert!(map
        .pcurve_bounds(
            &d,
            d.entity_reference(pc[1]).unwrap(),
            [0., 6.28318530718],
            nurbs::Limits::default(),
            10,
            10
        )
        .is_ok());
}

fn planar_fixture() -> (String, u64, [u64; 3]) {
    let (source, face, pc) = fixture();
    let d = inventory(&source, Limits::default()).unwrap();
    let c = BodyContext::read(&d).unwrap();
    let declaration = c
        .read_spline_surface(
            &d,
            d.entity_reference(face).unwrap(),
            nurbs_surface::Limits::default(),
        )
        .unwrap();
    let positions = [
        [0., 0., 0.],
        [0.5, 0., 0.],
        [1., 0., 0.],
        [0., 1., 0.],
        [0.5, 1., 0.],
        [1., 1., 0.],
    ];
    let mut edits = declaration
        .control_point_sources
        .iter()
        .zip(positions)
        .map(|(r, p)| {
            (
                r.entity,
                format!(
                    "#{}=CARTESIAN_POINT('',({},{},{}));",
                    r.entity, p[0], p[1], p[2]
                ),
            )
        })
        .collect::<Vec<_>>();
    let entity = d.resolve_reference(declaration.source).unwrap();
    let end = entity.offset + source[entity.offset..].find(';').unwrap() + 1;
    let record = source[entity.offset..end]
        .replace(".CYLINDRICAL_SURF.", ".PLANE_SURF.")
        .replace("(0.7071067811865476,0.7071067811865476)", "(1.,1.)");
    edits.push((entity.id, record));
    edits.sort_by_key(|(id, _)| std::cmp::Reverse(d.entities()[id].offset));
    let mut result = source;
    for (id, record) in edits {
        result = replace(&result, d.entities()[&id].offset, &record);
    }
    (result, face, pc)
}
#[test]
fn world_trace_distances_bound_global_objectives_for_all_parameter_curve_classes() {
    use solid_kernel::step_pcurve_distance::{distance, Options, SearchOptions};
    let (source, face, pc) = planar_fixture();
    let d = inventory(&source, Limits::default()).unwrap();
    let c = BodyContext::read(&d).unwrap();
    let map = SplineParameterMap::read(
        &c,
        &d,
        d.entity_reference(face).unwrap(),
        nurbs_surface::Limits::default(),
    )
    .unwrap();
    let options = Options {
        search: SearchOptions {
            distance_tolerance: 1e-6,
            ..SearchOptions::default()
        },
        ..Options::default()
    };
    for (i, query) in [[0.4, 0.25, 2.], [0.5, 0.625, 2.], [0.75, 0.25, 2.]]
        .into_iter()
        .enumerate()
    {
        let range = if i == 1 {
            [0., 6.28318530718]
        } else {
            [0., 1.]
        };
        let mut previous = 0;
        let r = distance(
            &map,
            &d,
            d.entity_reference(pc[i]).unwrap(),
            query,
            range,
            options,
            |p| {
                assert!(p.enclosures >= previous);
                previous = p.enclosures;
                true
            },
        )
        .unwrap();
        // The planar surface is the exact degree-elevated identity map. Each
        // query projects to its trace; the exact physical minimum is 2 mm.
        assert!(
            r.distance_bounds_mm[0] <= 2. && 2. <= r.distance_bounds_mm[1],
            "{r:?}"
        );
        assert!(r.distance_bounds_mm[1] - r.distance_bounds_mm[0] <= 1e-6);
        assert!(range[0] <= r.candidate_parameter && r.candidate_parameter <= range[1]);
        contains(r.candidate_enclosure_mm, r.enclosure_midpoint_mm);
        assert_eq!(r.face.entity, face);
        assert_eq!(r.pcurve.entity, pc[i]);
    }
    let full = distance(
        &map,
        &d,
        d.entity_reference(pc[0]).unwrap(),
        [0.5, 0.25, 0.],
        [0., 1.],
        options,
        |_| true,
    )
    .unwrap();
    assert_eq!(full.distance_bounds_mm[0], 0.);
    assert!(full.distance_bounds_mm[1] <= 1e-6);
    let partial = distance(
        &map,
        &d,
        d.entity_reference(pc[0]).unwrap(),
        [0.5, 0.25, 0.],
        [0., 0.5],
        options,
        |_| true,
    )
    .unwrap();
    assert!(partial.distance_bounds_mm[0] <= 0.125 && 0.125 <= partial.distance_bounds_mm[1]);
    assert!(partial.distance_bounds_mm[1] - partial.distance_bounds_mm[0] <= 1e-6);
}
#[test]
fn trace_distance_respects_physical_units_and_handles_nonunique_singular_maps() {
    use solid_kernel::step_pcurve_distance::{distance, Options};
    let (source, face, pc) = planar_fixture();
    let d = inventory(&source, Limits::default()).unwrap();
    let c = BodyContext::read(&d).unwrap();
    let unit = d
        .length_contexts()
        .unwrap()
        .into_iter()
        .find(|u| u.context == c.length_context().entity)
        .unwrap()
        .unit;
    let metres = replace(
        &source,
        d.entities()[&unit].offset,
        &format!("#{unit}=(LENGTH_UNIT() NAMED_UNIT(*) SI_UNIT($,.METRE.));"),
    );
    let m = inventory(&metres, Limits::default()).unwrap();
    let mc = BodyContext::read(&m).unwrap();
    let map = SplineParameterMap::read(
        &mc,
        &m,
        m.entity_reference(face).unwrap(),
        nurbs_surface::Limits::default(),
    )
    .unwrap();
    let r = distance(
        &map,
        &m,
        m.entity_reference(pc[0]).unwrap(),
        [400., 250., 2000.],
        [0., 1.],
        Options::default(),
        |_| true,
    )
    .unwrap();
    assert!(r.distance_bounds_mm[0] <= 2000. && 2000. <= r.distance_bounds_mm[1]);
    assert!(r.distance_bounds_mm[1] - r.distance_bounds_mm[0] <= 0.001);
    let declaration = c
        .read_spline_surface(
            &d,
            d.entity_reference(face).unwrap(),
            nurbs_surface::Limits::default(),
        )
        .unwrap();
    let mut edits = declaration
        .control_point_sources
        .iter()
        .enumerate()
        .map(|(i, r)| {
            (
                r.entity,
                format!(
                    "#{}=CARTESIAN_POINT('',(0.,{},0.));",
                    r.entity,
                    if i < 3 { 0. } else { 1. }
                ),
            )
        })
        .collect::<Vec<_>>();
    edits.sort_by_key(|(id, _)| std::cmp::Reverse(d.entities()[id].offset));
    let mut singular = source;
    for (id, record) in edits {
        singular = replace(&singular, d.entities()[&id].offset, &record);
    }
    let s = inventory(&singular, Limits::default()).unwrap();
    let sc = BodyContext::read(&s).unwrap();
    let sm = SplineParameterMap::read(
        &sc,
        &s,
        s.entity_reference(face).unwrap(),
        nurbs_surface::Limits::default(),
    )
    .unwrap();
    let r = distance(
        &sm,
        &s,
        s.entity_reference(pc[0]).unwrap(),
        [1., 0.25, 0.],
        [0., 1.],
        Options::default(),
        |_| true,
    )
    .unwrap();
    // Every parameter on this degenerate trace is equally close. A valid
    // distance bracket must not be mistaken for unique regular projection.
    assert!(r.distance_bounds_mm[0] <= 1. && 1. <= r.distance_bounds_mm[1]);
}
#[test]
fn trace_distance_work_budgets_cancellation_domains_and_stale_sources_refuse() {
    use solid_kernel::step_pcurve_distance::{distance, Options, SearchOptions};
    let (source, face, pc) = planar_fixture();
    let d = inventory(&source, Limits::default()).unwrap();
    let c = BodyContext::read(&d).unwrap();
    let map = SplineParameterMap::read(
        &c,
        &d,
        d.entity_reference(face).unwrap(),
        nurbs_surface::Limits::default(),
    )
    .unwrap();
    let p = d.entity_reference(pc[2]).unwrap();
    for options in [
        Options {
            search: SearchOptions {
                max_enclosures: 1,
                ..SearchOptions::default()
            },
            ..Options::default()
        },
        Options {
            search: SearchOptions {
                max_pending: 0,
                ..SearchOptions::default()
            },
            ..Options::default()
        },
        Options {
            max_curve_spans_per_enclosure: 0,
            ..Options::default()
        },
        Options {
            max_surface_patches_per_enclosure: 0,
            ..Options::default()
        },
        Options {
            curve_limits: nurbs::Limits {
                control_points: 2,
                ..nurbs::Limits::default()
            },
            ..Options::default()
        },
    ] {
        assert_eq!(
            distance(&map, &d, p, [0., 0., 1.], [0., 1.], options, |_| true)
                .unwrap_err()
                .kind,
            ErrorKind::Limit
        );
    }
    assert_eq!(
        distance(
            &map,
            &d,
            p,
            [0., 0., 1.],
            [0., 1.],
            Options::default(),
            |_| false
        )
        .unwrap_err()
        .kind,
        ErrorKind::Cancelled
    );
    assert_eq!(
        distance(
            &map,
            &d,
            p,
            [0.75, 0.25, 2.],
            [0., 1.],
            Options::default(),
            |s| s.enclosures == 0 || s.pending_intervals != 0
        )
        .unwrap_err()
        .kind,
        ErrorKind::Cancelled
    );
    for range in [[-1., 1.], [1., 0.], [0., f64::NAN]] {
        assert!(
            distance(&map, &d, p, [0., 0., 1.], range, Options::default(), |_| {
                true
            })
            .is_err()
        );
    }
    let changed = inventory(&format!("{source}\n"), Limits::default()).unwrap();
    assert!(distance(
        &map,
        &changed,
        changed.entity_reference(pc[0]).unwrap(),
        [0., 0., 1.],
        [0., 1.],
        Options::default(),
        |_| true
    )
    .is_err());
}

#[test]
fn trace_distance_covers_curved_surfaces_and_all_clipped_c0_knot_cells() {
    use solid_kernel::step_pcurve_distance::{distance, Options, SearchOptions};
    let (source, face, pc) = fixture();
    let data = inventory(&source, Limits::default()).unwrap();
    let declaration = solid_kernel::step_pcurve::read_pcurve(
        &data,
        data.entity_reference(pc[0]).unwrap(),
        nurbs::Limits::default(),
    )
    .unwrap();
    let line = data.resolve_reference(declaration.geometry_source).unwrap();
    let Value::Ref(point) = line.components[0].args[1] else {
        panic!("fixture")
    };
    let Value::Ref(vector) = line.components[0].args[2] else {
        panic!("fixture")
    };
    let Value::Ref(direction) = data.entities()[&vector].components[0].args[1] else {
        panic!("fixture")
    };
    let mut edits = vec![
        (point, format!("#{point}=CARTESIAN_POINT('',(0.,0.));")),
        (vector, format!("#{vector}=VECTOR('',#{direction},1.);")),
    ];
    edits.sort_by_key(|(id, _)| std::cmp::Reverse(data.entities()[id].offset));
    let mut curved = source;
    for (id, record) in edits {
        curved = replace(&curved, data.entities()[&id].offset, &record);
    }
    let d = inventory(&curved, Limits::default()).unwrap();
    let c = BodyContext::read(&d).unwrap();
    let map = SplineParameterMap::read(
        &c,
        &d,
        d.entity_reference(face).unwrap(),
        nurbs_surface::Limits::default(),
    )
    .unwrap();
    let r = distance(
        &map,
        &d,
        d.entity_reference(pc[0]).unwrap(),
        [1., 0., 2.],
        [0., 1.],
        Options::default(),
        |_| true,
    )
    .unwrap();
    assert!(r.distance_bounds_mm[0] <= 2. && 2. <= r.distance_bounds_mm[1]);
    let (source, face, pc) = planar_fixture();
    let d = inventory(&source, Limits::default()).unwrap();
    let declaration = solid_kernel::step_pcurve::read_pcurve(
        &d,
        d.entity_reference(pc[2]).unwrap(),
        nurbs::Limits::default(),
    )
    .unwrap();
    let solid_kernel::step_pcurve::ParametricCurve::Spline(spline) = declaration.geometry else {
        panic!("fixture")
    };
    let refs = spline.control_point_sources();
    let entity = d.resolve_reference(declaration.geometry_source).unwrap();
    let source=replace(&source,entity.offset,&format!("#{}=B_SPLINE_CURVE_WITH_KNOTS('C0',1,(#{},#{},#{}),.UNSPECIFIED.,.F.,.F.,(2,1,2),(0.,0.5,1.),.UNSPECIFIED.);",entity.id,refs[0].entity,refs[1].entity,refs[2].entity));
    let d = inventory(&source, Limits::default()).unwrap();
    let c = BodyContext::read(&d).unwrap();
    let map = SplineParameterMap::read(
        &c,
        &d,
        d.entity_reference(face).unwrap(),
        nurbs_surface::Limits::default(),
    )
    .unwrap();
    let p = d.entity_reference(pc[2]).unwrap();
    let limited = Options {
        search: SearchOptions {
            max_pending: 1,
            ..SearchOptions::default()
        },
        ..Options::default()
    };
    assert_eq!(
        distance(&map, &d, p, [0.75, 0.5, 2.], [0., 1.], limited, |_| true)
            .unwrap_err()
            .kind,
        ErrorKind::Limit
    );
    for range in [[0., 1.], [0., 0.5]] {
        let r = distance(
            &map,
            &d,
            p,
            [0.75, 0.5, 2.],
            range,
            Options::default(),
            |_| true,
        )
        .unwrap();
        assert!(r.distance_bounds_mm[0] <= 2. && 2. <= r.distance_bounds_mm[1]);
    }
    // Clipping to the first cell fits the one-cell admission budget.
    assert!(distance(&map, &d, p, [0.75, 0.5, 2.], [0., 0.5], limited, |_| true).is_ok());
}
