use solid_kernel::{
    nurbs,
    step::{inventory, ErrorKind, Limits, Value},
    step_body::BodyContext,
    step_cylinder_map::CylinderParameterMap,
    step_parameter_map::ParameterMapRef,
    step_pcurve::{self, ParametricCurve},
    step_pcurve_distance::{self, Options},
    step_plane_map::PlaneParameterMap,
};
const SOURCE: &str = include_str!("../../nema17-motor-bracket.step");
#[test]
fn all_bracket_parameter_traces_reuse_source_maps_and_bounded_search() {
    let data = inventory(SOURCE, Limits::default()).unwrap();
    let context = BodyContext::read(&data).unwrap();
    let mut counts = [0; 3];
    let mut surfaces = [0; 2];
    for entity in data.entities().values() {
        if entity.components[0].name != "PCURVE" {
            continue;
        }
        let pc = data.entity_reference(entity.id).unwrap();
        let declaration = step_pcurve::read_pcurve(&data, pc, nurbs::Limits::default()).unwrap();
        let face = context
            .body()
            .faces
            .values()
            .find(|f| f.surface == declaration.basis_surface.entity)
            .unwrap();
        let face_ref = data.entity_reference(face.id).unwrap();
        let plane;
        let cylinder;
        let map = if data.entities()[&face.surface].components[0].name == "PLANE" {
            plane = PlaneParameterMap::read(&context, &data, face_ref).unwrap();
            surfaces[0] += 1;
            ParameterMapRef::from(&plane)
        } else {
            cylinder = CylinderParameterMap::read(&context, &data, face_ref).unwrap();
            surfaces[1] += 1;
            ParameterMapRef::from(&cylinder)
        };
        assert_eq!(map.face_reference(), face_ref);
        assert_eq!(map.surface_reference(), declaration.basis_surface);
        assert_eq!(map.length_context(), context.length_context());
        let (uv, interval) = match &declaration.geometry {
            ParametricCurve::Line { origin, .. } => {
                counts[0] += 1;
                (*origin, [0., 0.125])
            }
            ParametricCurve::Circle {
                origin,
                x_direction,
                radius,
            } => {
                counts[1] += 1;
                (
                    std::array::from_fn(|i| origin[i] + x_direction[i] * radius),
                    [0., 0.125],
                )
            }
            ParametricCurve::Spline(s) => {
                counts[2] += 1;
                let d = s.curve().domain();
                let p = s.curve().evaluate(d[0]).unwrap();
                ([p[0], p[1]], d)
            }
        };
        let b = map.bounds(uv.map(|x| [x, x]), 0).unwrap();
        let query = std::array::from_fn(|i| b.min[i] + (b.max[i] - b.min[i]) * 0.5);
        let mut work = 0;
        let r = step_pcurve_distance::distance(
            map,
            &data,
            pc,
            query,
            interval,
            Options::default(),
            |p| {
                assert!(p.enclosures >= work);
                work = p.enclosures;
                true
            },
        )
        .unwrap();
        assert_eq!(r.face, face_ref);
        assert_eq!(r.surface, declaration.basis_surface);
        assert_eq!(r.parameter_interval, interval);
        assert!(r.distance_bounds_mm[0] >= 0.);
        assert!(
            r.distance_bounds_mm[1] <= 1e-7,
            "{:?}",
            r.distance_bounds_mm
        );
        assert_eq!(r.enclosures, work);
    }
    assert_eq!(counts, [200, 13, 15]);
    assert_eq!(surfaces.iter().sum::<usize>(), 228);
    assert!(surfaces.iter().all(|n| *n > 0));
}
#[test]
fn analytic_maps_preserve_basis_source_budgets_and_cancellation_guards() {
    let data = inventory(SOURCE, Limits::default()).unwrap();
    let context = BodyContext::read(&data).unwrap();
    let face = context
        .body()
        .faces
        .values()
        .find(|f| data.entities()[&f.surface].components[0].name == "PLANE")
        .unwrap();
    let map =
        PlaneParameterMap::read(&context, &data, data.entity_reference(face.id).unwrap()).unwrap();
    let mut matching = None;
    let mut other = None;
    for e in data.entities().values() {
        if e.components[0].name != "PCURVE" {
            continue;
        }
        let pc = data.entity_reference(e.id).unwrap();
        let d = step_pcurve::read_pcurve(&data, pc, nurbs::Limits::default()).unwrap();
        if d.basis_surface == map.surface_reference() {
            matching = Some(pc)
        } else {
            other = Some(pc)
        }
    }
    let pc = matching.unwrap();
    assert!(step_pcurve_distance::distance(
        &map,
        &data,
        other.unwrap(),
        [0.; 3],
        [0., 1.],
        Options::default(),
        |_| true
    )
    .is_err());
    assert_eq!(
        step_pcurve_distance::distance(
            &map,
            &data,
            pc,
            [0.; 3],
            [0., 1.],
            Options::default(),
            |_| false
        )
        .unwrap_err()
        .kind,
        ErrorKind::Cancelled
    );
    let mut o = Options::default();
    o.search.max_enclosures = 0;
    assert_eq!(
        step_pcurve_distance::distance(&map, &data, pc, [0.; 3], [0., 1.], o, |_| true)
            .unwrap_err()
            .kind,
        ErrorKind::Limit
    );
    o = Options::default();
    o.search.max_pending = 0;
    assert_eq!(
        step_pcurve_distance::distance(&map, &data, pc, [0.; 3], [0., 1.], o, |_| true)
            .unwrap_err()
            .kind,
        ErrorKind::Limit
    );
    let stale = inventory(&format!("{SOURCE}\n"), Limits::default()).unwrap();
    assert!(step_pcurve_distance::distance(
        &map,
        &stale,
        pc,
        [0.; 3],
        [0., 1.],
        Options::default(),
        |_| true
    )
    .is_err());
    // Keep the fixture's raw unit declaration explicit rather than guessing it.
    assert!(data.entities().values().any(|e| e.components.iter().any(
        |c| c.name == "SI_UNIT" && matches!(c.args.first(),Some(Value::Enum(x)) if x=="MILLI")
    )));
}
#[test]
fn plane_query_has_analytic_normal_distance_in_mm_and_metres() {
    let data = inventory(SOURCE, Limits::default()).unwrap();
    let context = BodyContext::read(&data).unwrap();
    let (face_id, pc_id, uv) = data
        .entities()
        .values()
        .find_map(|e| {
            if e.components[0].name != "PCURVE" {
                return None;
            }
            let d = step_pcurve::read_pcurve(
                &data,
                data.entity_reference(e.id)?,
                nurbs::Limits::default(),
            )
            .ok()?;
            if data.entities()[&d.basis_surface.entity].components[0].name != "PLANE" {
                return None;
            }
            let ParametricCurve::Line { origin, .. } = d.geometry else {
                return None;
            };
            let face = context
                .body()
                .faces
                .values()
                .find(|f| f.surface == d.basis_surface.entity)?;
            Some((face.id, e.id, origin))
        })
        .unwrap();
    let face = &context.body().faces[&face_id];
    let Value::Ref(frame) = data.entities()[&face.surface].components[0].args[1] else {
        panic!("fixture");
    };
    let e = &data.entities()[&frame];
    let next = data.entities().keys().last().unwrap() + 1;
    let replace = |text: &str, offset: usize, record: &str| {
        let end = offset + text[offset..].find(';').unwrap() + 1;
        format!("{}{}{}", &text[..offset], record, &text[end..])
    };
    let mut source = replace(
        SOURCE,
        e.offset,
        &format!("#{frame}=AXIS2_PLACEMENT_3D('',#{next},$,$);"),
    );
    let end = source.rfind("ENDSEC;").unwrap();
    source.insert_str(end, &format!("#{next}=CARTESIAN_POINT('',(1.,2.,3.));"));
    for scale in [1., 1000.] {
        let text = if scale == 1. {
            source.clone()
        } else {
            let d = inventory(&source, Limits::default()).unwrap();
            let c = BodyContext::read(&d).unwrap();
            let u = d
                .length_contexts()
                .unwrap()
                .into_iter()
                .find(|u| u.context == c.length_context().entity)
                .unwrap();
            replace(
                &source,
                d.entities()[&u.unit].offset,
                &format!(
                    "#{}=(LENGTH_UNIT() NAMED_UNIT(*) SI_UNIT($,.METRE.));",
                    u.unit
                ),
            )
        };
        let d = inventory(&text, Limits::default()).unwrap();
        let c = BodyContext::read(&d).unwrap();
        let map = PlaneParameterMap::read(&c, &d, d.entity_reference(face_id).unwrap()).unwrap();
        let query = [(1. + uv[0]) * scale, (2. + uv[1]) * scale, 5. * scale];
        let r = step_pcurve_distance::distance(
            &map,
            &d,
            d.entity_reference(pc_id).unwrap(),
            query,
            [0., 0.125],
            Options::default(),
            |_| true,
        )
        .unwrap();
        assert!(
            r.distance_bounds_mm[0] <= 2. * scale && r.distance_bounds_mm[1] >= 2. * scale,
            "{:?}",
            r.distance_bounds_mm
        );
        assert!(r.distance_bounds_mm[1] - r.distance_bounds_mm[0] <= 0.001);
        assert_eq!(r.parameter_interval, [0., 0.125]);
        assert_eq!(
            step_pcurve_distance::distance(
                &map,
                &d,
                d.entity_reference(pc_id).unwrap(),
                query,
                [0., 0.125],
                Options::default(),
                |p| p.enclosures == 0 || p.pending_intervals != 0
            )
            .unwrap_err()
            .kind,
            ErrorKind::Cancelled
        );
    }
}
