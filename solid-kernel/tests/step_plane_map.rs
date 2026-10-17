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
