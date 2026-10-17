use solid_kernel::{
    analytic_step::{AnalyticModel, Options},
    step::{inventory, Limits, Value},
    step_body::BodyContext,
    step_plane_map::PlaneParameterMap,
};
const SOURCE: &str = include_str!("../../nema17-motor-bracket.step");
fn replace(s: &str, offset: usize, record: &str) -> String {
    let end = offset + s[offset..].find(';').unwrap() + 1;
    format!("{}{}{}", &s[..offset], record, &s[end..])
}
fn contains(b: solid_kernel::geometry_bounds::Bounds3, p: [f64; 3]) {
    for i in 0..3 {
        assert!(b.min[i] <= p[i] && p[i] <= b.max[i], "{p:?} {b:?}");
    }
}
fn axis_fixture(tiny: bool, parallel: bool) -> (String, u64) {
    let d = inventory(SOURCE, Limits::default()).unwrap();
    let c = BodyContext::read(&d).unwrap();
    let face = c
        .body()
        .faces
        .values()
        .find(|f| d.entities()[&f.surface].components[0].name == "PLANE")
        .unwrap();
    let Value::Ref(frame) = d.entities()[&face.surface].components[0].args[1] else {
        panic!("fixture");
    };
    let e = &d.entities()[&frame];
    let next = d.entities().keys().last().unwrap() + 1;
    let x = if tiny {
        format!("#{}", next + 1)
    } else {
        "$".into()
    };
    let mut source = replace(
        SOURCE,
        e.offset,
        &format!("#{}=AXIS2_PLACEMENT_3D('',#{next},$,{x});", e.id),
    );
    let extra = if tiny {
        format!(
            "#{}=DIRECTION('',({}));",
            next + 1,
            if parallel {
                "0.,0.,1.E-310"
            } else {
                "1.E-310,0.,0."
            }
        )
    } else {
        String::new()
    };
    let end = source.rfind("ENDSEC;").unwrap();
    source.insert_str(
        end,
        &format!("#{next}=CARTESIAN_POINT('',(1.,2.,3.));{extra}"),
    );
    (source, face.id)
}
#[test]
fn source_plane_forward_maps_bound_actual_bracket_plane_frames() {
    let d = inventory(SOURCE, Limits::default()).unwrap();
    let c = BodyContext::read(&d).unwrap();
    let model = AnalyticModel::read(&d, Options::default()).unwrap();
    let mut count = 0;
    for face in c.body().faces.values() {
        if d.entities()[&face.surface].components[0].name != "PLANE" {
            continue;
        }
        let r = d.entity_reference(face.id).unwrap();
        let map = PlaneParameterMap::read(&c, &d, r).unwrap();
        assert_eq!(map.face_reference(), r);
        assert_eq!(
            map.surface_reference(),
            d.entity_reference(face.surface).unwrap()
        );
        assert_eq!(map.length_context(), c.length_context());
        for uv in [[0., 0.], [0.25, -0.75], [10., 20.]] {
            contains(
                map.point_bounds(uv).unwrap(),
                model.surface(face.id).unwrap().point(uv).unwrap(),
            );
        }
        count += 1;
    }
    assert!(count > 0);
}
#[test]
fn source_length_units_and_tiny_direction_ratios_map_without_reciprocal_overflow() {
    for tiny in [false, true] {
        let (source, face) = axis_fixture(tiny, false);
        let d = inventory(&source, Limits::default()).unwrap();
        let c = BodyContext::read(&d).unwrap();
        let map = PlaneParameterMap::read(&c, &d, d.entity_reference(face).unwrap()).unwrap();
        contains(map.point_bounds([4., 5.]).unwrap(), [5., 7., 3.]);
        let bounds = map.bounds([[0., 1.], [-1., 2.]]).unwrap();
        for p in [[1., 1., 3.], [2., 4., 3.]] {
            contains(bounds, p);
        }
        let contexts = d.length_contexts().unwrap();
        let length = contexts
            .iter()
            .find(|x| x.context == c.length_context().entity)
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
        let mm = PlaneParameterMap::read(&mc, &m, m.entity_reference(face).unwrap()).unwrap();
        assert_eq!(mm.millimetres_per_source_unit(), 1000.);
        contains(mm.point_bounds([4., 5.]).unwrap(), [5000., 7000., 3000.]);
        assert!(PlaneParameterMap::read(&c, &m, m.entity_reference(face).unwrap()).is_err());
        assert!(PlaneParameterMap::read(&mc, &m, d.entity_reference(face).unwrap()).is_err());
    }
}
#[test]
fn nonplane_stale_parallel_and_unbounded_queries_do_not_infer_parameter_semantics() {
    let (source, face) = axis_fixture(false, false);
    let d = inventory(&source, Limits::default()).unwrap();
    let c = BodyContext::read(&d).unwrap();
    let map = PlaneParameterMap::read(&c, &d, d.entity_reference(face).unwrap()).unwrap();
    let cylinder = c
        .body()
        .faces
        .values()
        .find(|f| d.entities()[&f.surface].components[0].name == "CYLINDRICAL_SURFACE")
        .unwrap();
    assert!(PlaneParameterMap::read(&c, &d, d.entity_reference(cylinder.id).unwrap()).is_err());
    for p in [
        [[1., 0.], [0., 1.]],
        [[0., f64::NAN], [0., 1.]],
        [[1e308, 1e308], [0., 0.]],
    ] {
        assert!(map.bounds(p).is_err());
    }
    let (parallel, face) = axis_fixture(true, true);
    let p = inventory(&parallel, Limits::default()).unwrap();
    let pc = BodyContext::read(&p).unwrap();
    assert!(PlaneParameterMap::read(&pc, &p, p.entity_reference(face).unwrap()).is_err());
}
fn line_cases(d: &solid_kernel::step::Inventory, c: &BodyContext) -> Vec<(u64, u64, u64)> {
    use solid_kernel::step_pcurve::{self, ParametricCurve};
    let mut cases = std::collections::BTreeSet::new();
    for face in c.body().faces.values() {
        if d.entities()[&face.surface].components[0].name != "PLANE" {
            continue;
        }
        for usage in face.bounds.iter().flat_map(|b| &b.edges) {
            let edge = &c.body().edges[&usage.edge];
            let wrapper =
                step_pcurve::read_surface_curve(d, d.entity_reference(edge.curve).unwrap())
                    .unwrap();
            if d.entities()[&wrapper.geometry_3d.entity].components[0].name != "LINE" {
                continue;
            }
            for r in wrapper.pcurves {
                let p =
                    step_pcurve::read_pcurve(d, r, solid_kernel::nurbs::Limits::default()).unwrap();
                if p.basis_surface.entity == face.surface
                    && matches!(p.geometry, ParametricCurve::Line { .. })
                {
                    cases.insert((face.id, edge.id, r.entity));
                }
            }
        }
    }
    cases.into_iter().collect()
}
#[test]
fn actual_bracket_plane_line_pcurves_have_bounded_finite_3d_locus_agreement() {
    let d = inventory(SOURCE, Limits::default()).unwrap();
    let c = BodyContext::read(&d).unwrap();
    let cases = line_cases(&d, &c);
    assert!(!cases.is_empty());
    for (face, edge, pcurve) in &cases {
        let map = PlaneParameterMap::read(&c, &d, d.entity_reference(*face).unwrap()).unwrap();
        let report = map
            .verify_line_locus(
                &c,
                &d,
                d.entity_reference(*edge).unwrap(),
                d.entity_reference(*pcurve).unwrap(),
                1e-7,
            )
            .unwrap();
        assert_eq!(report.face.entity, *face);
        assert_eq!(report.edge.entity, *edge);
        assert_eq!(report.pcurve.entity, *pcurve);
        assert_eq!(
            report.checked_pcurve_is_master,
            report.master_reference == report.pcurve
        );
        for error in report
            .source_to_3d_error_mm
            .iter()
            .chain(&report.source_to_pcurve_error_mm)
            .chain(&report.curve_to_pcurve_error_mm)
        {
            assert!(*error <= 1e-7);
        }
        assert!(report.natural_direction_dot.iter().all(|x| x.is_finite()));
    }
    eprintln!("verified {} finite plane/line p-curve loci", cases.len());
}
#[test]
fn corrupted_parameter_line_and_contradictory_source_sense_refuse_without_changing_tolerance() {
    use solid_kernel::step_pcurve::{self, ParametricCurve};
    let d = inventory(SOURCE, Limits::default()).unwrap();
    let c = BodyContext::read(&d).unwrap();
    let (face, edge, pcurve) = line_cases(&d, &c)[0];
    let p = step_pcurve::read_pcurve(
        &d,
        d.entity_reference(pcurve).unwrap(),
        solid_kernel::nurbs::Limits::default(),
    )
    .unwrap();
    let ParametricCurve::Line {
        origin, direction, ..
    } = p.geometry
    else {
        panic!("fixture");
    };
    let Value::Ref(point) = d.entities()[&p.geometry_source.entity].components[0].args[1] else {
        panic!("fixture");
    };
    let e = &d.entities()[&point];
    let bad = replace(
        SOURCE,
        e.offset,
        &format!(
            "#{}=CARTESIAN_POINT('',({:.17e},{:.17e}));",
            e.id,
            origin[0] - direction[1],
            origin[1] + direction[0]
        ),
    );
    let bad = inventory(&bad, Limits::default()).unwrap();
    let bc = BodyContext::read(&bad).unwrap();
    let map = PlaneParameterMap::read(&bc, &bad, bad.entity_reference(face).unwrap()).unwrap();
    assert!(map
        .verify_line_locus(
            &bc,
            &bad,
            bad.entity_reference(edge).unwrap(),
            bad.entity_reference(pcurve).unwrap(),
            1e-7
        )
        .unwrap_err()
        .detail
        .contains("locus agreement"));
    let edge_data = &c.body().edges[&edge];
    let e = &d.entities()[&edge];
    let sense = replace(
        SOURCE,
        e.offset,
        &format!(
            "#{}=EDGE_CURVE('',#{},#{},#{},.{}.);",
            edge,
            edge_data.start,
            edge_data.end,
            edge_data.curve,
            if edge_data.same_sense { "F" } else { "T" }
        ),
    );
    let sd = inventory(&sense, Limits::default()).unwrap();
    let sc = BodyContext::read(&sd).unwrap();
    let sm = PlaneParameterMap::read(&sc, &sd, sd.entity_reference(face).unwrap()).unwrap();
    assert!(sm
        .verify_line_locus(
            &sc,
            &sd,
            sd.entity_reference(edge).unwrap(),
            sd.entity_reference(pcurve).unwrap(),
            1e-7
        )
        .unwrap_err()
        .detail
        .contains("extent/sense"));
}
#[test]
fn line_locus_agreement_uses_source_length_scale_and_rejects_stale_contexts() {
    let d = inventory(SOURCE, Limits::default()).unwrap();
    let c = BodyContext::read(&d).unwrap();
    let (face, edge, pcurve) = line_cases(&d, &c)[0];
    let contexts = d.length_contexts().unwrap();
    let length = contexts
        .iter()
        .find(|x| x.context == c.length_context().entity)
        .unwrap();
    let unit = &d.entities()[&length.unit];
    let text = replace(
        SOURCE,
        unit.offset,
        &format!(
            "#{}=(LENGTH_UNIT() NAMED_UNIT(*) SI_UNIT($,.METRE.));",
            unit.id
        ),
    );
    let m = inventory(&text, Limits::default()).unwrap();
    let mc = BodyContext::read(&m).unwrap();
    let map = PlaneParameterMap::read(&mc, &m, m.entity_reference(face).unwrap()).unwrap();
    let r = map
        .verify_line_locus(
            &mc,
            &m,
            m.entity_reference(edge).unwrap(),
            m.entity_reference(pcurve).unwrap(),
            1e-7,
        )
        .unwrap();
    assert!(r.curve_to_pcurve_error_mm.iter().all(|e| *e <= 1e-7));
    assert!(map
        .verify_line_locus(
            &c,
            &m,
            m.entity_reference(edge).unwrap(),
            m.entity_reference(pcurve).unwrap(),
            1e-7
        )
        .is_err());
    assert!(map
        .verify_line_locus(
            &mc,
            &m,
            d.entity_reference(edge).unwrap(),
            m.entity_reference(pcurve).unwrap(),
            1e-7
        )
        .is_err());
}

#[test]
fn whole_spline_pcurve_intervals_map_with_budget_and_source_guards() {
    use solid_kernel::{
        nurbs,
        step::ErrorKind,
        step_pcurve::{self, ParametricCurve},
    };
    let data = inventory(SOURCE, Limits::default()).unwrap();
    let context = BodyContext::read(&data).unwrap();
    let mut checked = 0;
    for entity in data
        .entities()
        .values()
        .filter(|e| e.components[0].name == "PCURVE")
    {
        let reference = data.entity_reference(entity.id).unwrap();
        let declaration =
            step_pcurve::read_pcurve(&data, reference, nurbs::Limits::default()).unwrap();
        let ParametricCurve::Spline(spline) = &declaration.geometry else {
            continue;
        };
        let face = context
            .body()
            .faces
            .values()
            .find(|f| f.surface == declaration.basis_surface.entity)
            .unwrap();
        let map = PlaneParameterMap::read(&context, &data, data.entity_reference(face.id).unwrap())
            .unwrap();
        let domain = spline.curve().domain();
        // Independent point evaluation probes validate enclosure containment;
        // the certificate itself comes from whole-interval de Boor bounds.
        for part in 0..8 {
            let a = domain[0] + (domain[1] - domain[0]) * part as f64 / 8.;
            let b = domain[0] + (domain[1] - domain[0]) * (part + 1) as f64 / 8.;
            let bounds = map
                .spline_pcurve_bounds(&data, reference, [a, b], nurbs::Limits::default(), 100)
                .unwrap();
            for i in 0..17 {
                let t = a + (b - a) * i as f64 / 16.;
                let uv = spline.curve().evaluate(t).unwrap();
                let point = map.point_bounds([uv[0], uv[1]]).unwrap();
                // A separately rounded evaluation can protrude by a few ulps;
                // require overlap with its outward-rounded mapped enclosure.
                for axis in 0..3 {
                    assert!(
                        bounds.min[axis] <= point.max[axis] && point.min[axis] <= bounds.max[axis]
                    );
                }
            }
        }
        let error = map
            .spline_pcurve_bounds(&data, reference, domain, nurbs::Limits::default(), 0)
            .unwrap_err();
        assert_eq!(error.kind, ErrorKind::Limit);
        assert_eq!(error.offset, entity.offset);
        assert!(map
            .spline_pcurve_bounds(
                &data,
                reference,
                [domain[0] - 1., domain[1]],
                nurbs::Limits::default(),
                100
            )
            .is_err());
        let changed = inventory(&format!("{SOURCE}\n"), Limits::default()).unwrap();
        assert!(map
            .spline_pcurve_bounds(
                &changed,
                changed.entity_reference(entity.id).unwrap(),
                domain,
                nurbs::Limits::default(),
                100
            )
            .is_err());
        let other = context
            .body()
            .faces
            .values()
            .find(|f| {
                f.surface != face.surface
                    && data.entities()[&f.surface].components[0].name == "PLANE"
            })
            .unwrap();
        let wrong =
            PlaneParameterMap::read(&context, &data, data.entity_reference(other.id).unwrap())
                .unwrap();
        assert!(wrong
            .spline_pcurve_bounds(&data, reference, domain, nurbs::Limits::default(), 100)
            .is_err());
        checked += 1;
    }
    assert_eq!(checked, 15);
}

#[test]
fn spline_pcurve_world_bounds_scale_lengths_once_and_leave_knots_raw() {
    use solid_kernel::{
        nurbs,
        step_pcurve::{self, ParametricCurve},
    };
    let data = inventory(SOURCE, Limits::default()).unwrap();
    let context = BodyContext::read(&data).unwrap();
    let length = data
        .length_contexts()
        .unwrap()
        .into_iter()
        .find(|c| c.context == context.length_context().entity)
        .unwrap();
    let unit = &data.entities()[&length.unit];
    let source = replace(
        SOURCE,
        unit.offset,
        &format!(
            "#{}=(LENGTH_UNIT() NAMED_UNIT(*) SI_UNIT($,.METRE.));",
            unit.id
        ),
    );
    let metric = inventory(&source, Limits::default()).unwrap();
    let mc = BodyContext::read(&metric).unwrap();
    let pc = data.entity_reference(235).unwrap();
    let original = step_pcurve::read_pcurve(&data, pc, nurbs::Limits::default()).unwrap();
    let converted = step_pcurve::read_pcurve(
        &metric,
        metric.entity_reference(235).unwrap(),
        nurbs::Limits::default(),
    )
    .unwrap();
    let (ParametricCurve::Spline(a), ParametricCurve::Spline(b)) =
        (&original.geometry, &converted.geometry)
    else {
        panic!("fixture")
    };
    assert_eq!(a.curve().knots(), b.curve().knots());
    assert_eq!(a.curve().control_points(), b.curve().control_points());
    let face = context
        .body()
        .faces
        .values()
        .find(|f| f.surface == original.basis_surface.entity)
        .unwrap();
    let map =
        PlaneParameterMap::read(&context, &data, data.entity_reference(face.id).unwrap()).unwrap();
    let scaled =
        PlaneParameterMap::read(&mc, &metric, metric.entity_reference(face.id).unwrap()).unwrap();
    let domain = a.curve().domain();
    let bounds = scaled
        .spline_pcurve_bounds(
            &metric,
            metric.entity_reference(235).unwrap(),
            domain,
            nurbs::Limits::default(),
            100,
        )
        .unwrap();
    for i in 0..33 {
        let p = a
            .curve()
            .evaluate(domain[0] + (domain[1] - domain[0]) * i as f64 / 32.)
            .unwrap();
        let world = map.point_bounds([p[0], p[1]]).unwrap();
        for axis in 0..3 {
            assert!(
                bounds.min[axis] <= world.max[axis] * 1000.
                    && world.min[axis] * 1000. <= bounds.max[axis]
            );
        }
    }
    assert!(scaled
        .spline_pcurve_bounds(&metric, pc, domain, nurbs::Limits::default(), 100)
        .is_err());
}
