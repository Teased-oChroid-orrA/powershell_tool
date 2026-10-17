use solid_kernel::{
    nurbs::Limits as SplineLimits,
    step::{inventory, Limits, Value},
    step_pcurve::{self, CurveMaster, ParametricCurve},
};
fn file(point: &str, context: &str) -> String {
    format!("ISO-10303-21; HEADER; FILE_DESCRIPTION(('pcurve'),'2;1'); FILE_NAME('fixture','',(''),(''),'','',''); FILE_SCHEMA(('TEST')); ENDSEC; DATA; #1=PLANE('',#2); #2=AXIS2_PLACEMENT_3D('',#3,$,$); #3=CARTESIAN_POINT('',(0.,0.,0.)); #4=PCURVE('raw',#1,#5); #5=DEFINITIONAL_REPRESENTATION('',(#6),#7); #6=LINE('',#8,#9); #7={context}; #8=CARTESIAN_POINT('',{point}); #9=VECTOR('',#10,2.); #10=DIRECTION('',(3.,4.)); #11=SURFACE_CURVE('master',#12,(#4),.PCURVE_S1.); #12=LINE('',#3,#13); #13=VECTOR('',#14,1.); #14=DIRECTION('',(1.,0.,0.)); ENDSEC; END-ISO-10303-21;")
}
const CONTEXT:&str="(GEOMETRIC_REPRESENTATION_CONTEXT(2) PARAMETRIC_REPRESENTATION_CONTEXT() REPRESENTATION_CONTEXT('2D',''))";
#[test]
fn actual_bracket_all_parameter_curves_and_declared_master_associations_decode_without_length_conversion(
) {
    let data = inventory(
        include_str!("../../nema17-motor-bracket.step"),
        Limits::default(),
    )
    .unwrap();
    let mut counts = [0; 3];
    let mut wrappers = 0;
    for e in data.entities().values() {
        if e.components.iter().any(|c| c.name == "PCURVE") {
            let p = step_pcurve::read_pcurve(
                &data,
                data.entity_reference(e.id).unwrap(),
                SplineLimits::default(),
            )
            .unwrap();
            assert!(data.resolve_reference(p.parameter_context).is_ok());
            match &p.geometry {
                ParametricCurve::Line {
                    origin,
                    direction,
                    magnitude,
                } => {
                    counts[0] += 1;
                    assert!(origin.iter().chain(direction).all(|x| x.is_finite()));
                    assert!(*magnitude > 0.);
                }
                ParametricCurve::Circle { radius, .. } => {
                    counts[1] += 1;
                    assert!(*radius > 0.);
                }
                ParametricCurve::Spline(s) => {
                    counts[2] += 1;
                    assert_eq!(s.source(), p.geometry_source);
                    let d = s.curve().domain();
                    assert!(s.curve().evaluate((d[0] + d[1]) * 0.5).is_ok());
                    for r in s.control_point_sources() {
                        assert!(data.resolve_reference(*r).is_ok());
                    }
                }
            }
        }
        if e.components
            .iter()
            .any(|c| matches!(c.name.as_str(), "SURFACE_CURVE" | "SEAM_CURVE"))
        {
            let s = step_pcurve::read_surface_curve(&data, data.entity_reference(e.id).unwrap())
                .unwrap();
            assert!(!s.pcurves.is_empty());
            for p in s.pcurves {
                assert!(step_pcurve::read_pcurve(&data, p, SplineLimits::default()).is_ok());
            }
            wrappers += 1;
        }
    }
    assert_eq!(counts, [200, 13, 15]);
    assert!(wrappers > 0);
    let e = &data.entities()[&237];
    let declaration = step_pcurve::read_pcurve(
        &data,
        data.entity_reference(235).unwrap(),
        SplineLimits::default(),
    )
    .unwrap();
    let ParametricCurve::Spline(s) = declaration.geometry else {
        panic!("fixture");
    };
    let c = &data.entities()[&s.control_point_sources()[0].entity];
    let Value::List(raw) = &c.components[0].args[1] else {
        panic!("fixture");
    };
    let scalar = |v: &Value| match v {
        Value::Real(x) => *x,
        Value::Integer(x) => *x as f64,
        _ => panic!("fixture"),
    };
    assert_eq!(
        s.curve().control_points()[0],
        [scalar(&raw[0]), scalar(&raw[1]), 0.]
    );
    assert_eq!(s.source().entity, e.id);
}
#[test]
fn raw_parameter_values_tiny_numbers_and_source_references_are_not_physical_mm() {
    let text = file("(1.E-310,90.)", CONTEXT);
    let data = inventory(&text, Limits::default()).unwrap();
    let reference = data.entity_reference(4).unwrap();
    let p = step_pcurve::read_pcurve(&data, reference, SplineLimits::default()).unwrap();
    let ParametricCurve::Line {
        origin,
        direction,
        magnitude,
    } = p.geometry
    else {
        panic!("line");
    };
    assert_eq!(origin, [1e-310, 90.]);
    assert!((direction[0] - 0.6).abs() < 1e-15);
    assert!((direction[1] - 0.8).abs() < 1e-15);
    assert_eq!(magnitude, 2.);
    assert_eq!(p.source, reference);
    assert_eq!(p.basis_surface.entity, 1);
    assert_eq!(p.geometry_source.entity, 6);
    let changed = inventory(&(text + "\n"), Limits::default()).unwrap();
    assert!(step_pcurve::read_pcurve(&changed, reference, SplineLimits::default()).is_err());
    let master =
        step_pcurve::read_surface_curve(&data, data.entity_reference(11).unwrap()).unwrap();
    assert_eq!(master.declared_master, CurveMaster::Pcurve1);
    assert_eq!(master.pcurves, vec![reference]);
}
#[test]
fn wrong_dimensions_components_items_and_master_indices_refuse_at_source_entities() {
    let valid = file("(1.,2.)", CONTEXT);
    for text in [
        file("(1.,2.,3.)", CONTEXT),
        file("(1.,2.)", &CONTEXT.replace("CONTEXT(2)", "CONTEXT(3)")),
        file(
            "(1.,2.)",
            &CONTEXT.replace(
                "PARAMETRIC_REPRESENTATION_CONTEXT()",
                "PARAMETRIC_REPRESENTATION_CONTEXT() PARAMETRIC_REPRESENTATION_CONTEXT()",
            ),
        ),
        valid.replace("('',(#6),#7)", "('',(#6,#6),#7)"),
        valid.replace("DIRECTION('',(3.,4.))", "DIRECTION('',(0.,0.))"),
    ] {
        let d = inventory(&text, Limits::default()).unwrap();
        assert!(step_pcurve::read_pcurve(
            &d,
            d.entity_reference(4).unwrap(),
            SplineLimits::default()
        )
        .is_err());
    }
    for text in [
        valid.replace(".PCURVE_S1.", ".PCURVE_S2."),
        valid.replace("(#4),.PCURVE_S1.", "(#4,#4),.PCURVE_S1."),
        valid.replace("(#4),.PCURVE_S1.", "(),.CURVE_3D."),
    ] {
        let d = inventory(&text, Limits::default()).unwrap();
        assert!(step_pcurve::read_surface_curve(&d, d.entity_reference(11).unwrap()).is_err());
    }
}
#[test]
fn spline_caps_and_stale_master_references_stay_explicit() {
    let text = include_str!("../../nema17-motor-bracket.step");
    let d = inventory(text, Limits::default()).unwrap();
    let e = step_pcurve::read_pcurve(
        &d,
        d.entity_reference(235).unwrap(),
        SplineLimits {
            control_points: 1,
            ..SplineLimits::default()
        },
    )
    .unwrap_err();
    assert_eq!(e.kind, solid_kernel::step::ErrorKind::Limit);
    let wrapper = d
        .entities()
        .values()
        .find(|e| e.components.iter().any(|c| c.name == "SURFACE_CURVE"))
        .unwrap();
    let changed = inventory(&(text.to_string() + "\n"), Limits::default()).unwrap();
    assert!(
        step_pcurve::read_surface_curve(&changed, d.entity_reference(wrapper.id).unwrap()).is_err()
    );
}
