use solid_kernel::{
    analytic_step::{AnalyticModel, Options, Surface},
    step::{inventory, Limits},
};
const SOURCE: &str = include_str!("../../nema17-motor-bracket.step");
#[test]
fn bracket_analytic_edges_and_surfaces_are_consistent_without_occt() {
    let data = inventory(SOURCE, Limits::default()).unwrap();
    let model = AnalyticModel::read(&data, Options::default()).unwrap();
    assert_eq!(model.body().faces.len(), 38);
    assert_eq!(model.millimetres_per_source_unit(), 1.);
    let mut planes = 0;
    let mut cylinders = 0;
    for face in model.body().faces.values() {
        let surface = model.surface(face.id).unwrap();
        match surface {
            Surface::Plane(_) => planes += 1,
            Surface::Cylinder { .. } => cylinders += 1,
        };
        for bound in &face.bounds {
            for e in &bound.edges {
                for &p in model.edge_points(e.edge).unwrap() {
                    let projection = surface.project(p).unwrap();
                    assert!(projection.residual_mm <= 1e-7);
                    let q = surface.point(projection.chart).unwrap();
                    let error = (0..3).map(|i| (p[i] - q[i]).powi(2)).sum::<f64>().sqrt();
                    assert!(error < 1e-7);
                    let n = surface.normal(projection.chart).unwrap();
                    assert!((n.iter().map(|x| x * x).sum::<f64>() - 1.).abs() < 1e-13);
                }
            }
        }
    }
    assert_eq!((planes, cylinders), (28, 10));
}
#[test]
fn declared_units_normalize_coordinates_and_sampling_budgets_are_hard() {
    let base = inventory(SOURCE, Limits::default()).unwrap();
    let original = AnalyticModel::read(&base, Options::default()).unwrap();
    let changed = SOURCE.replace(".MILLI.", ".CENTI.");
    let data = inventory(&changed, Limits::default()).unwrap();
    let scaled = AnalyticModel::read(&data, Options::default()).unwrap();
    assert_eq!(scaled.millimetres_per_source_unit(), 10.);
    for &id in original.body().vertices.keys() {
        assert_eq!(
            scaled.vertex(id).unwrap(),
            original.vertex(id).unwrap().map(|x| 10. * x)
        );
    }
    assert!(AnalyticModel::read(
        &base,
        Options {
            max_total_points: 1,
            ..Options::default()
        }
    )
    .is_err());
    assert!(AnalyticModel::read(
        &base,
        Options {
            chord_mm: f64::NAN,
            ..Options::default()
        }
    )
    .is_err());
}

fn replace_record(source: &str, offset: usize, record: &str) -> String {
    let end = offset + source[offset..].find(';').unwrap() + 1;
    format!("{}{}{}", &source[..offset], record, &source[end..])
}
#[test]
fn inconsistent_geometry_unknown_contexts_and_assembly_are_not_partially_accepted() {
    use solid_kernel::step::Value;
    let data = inventory(SOURCE, Limits::default()).unwrap();
    let body = data.raw_bodies().unwrap().remove(0);
    let surface = body
        .faces
        .values()
        .map(|f| &data.entities()[&f.surface])
        .find(|e| e.components[0].name == "CYLINDRICAL_SURFACE")
        .unwrap();
    let args = &surface.components[0].args;
    let (frame, radius) = match (&args[1], &args[2]) {
        (Value::Ref(f), Value::Real(r)) => (*f, *r),
        _ => panic!("fixture cylinder"),
    };
    let changed = replace_record(
        SOURCE,
        surface.offset,
        &format!(
            "#{}=CYLINDRICAL_SURFACE('',#{frame},{});",
            surface.id,
            radius + 1.
        ),
    );
    let invalid = inventory(&changed, Limits::default()).unwrap();
    assert!(AnalyticModel::read(&invalid, Options::default())
        .unwrap_err()
        .detail
        .contains("disagrees"));
    let unsupported = replace_record(
        SOURCE,
        surface.offset,
        &format!("#{}=TOROIDAL_SURFACE('',#{frame},1.,0.5);", surface.id),
    );
    assert!(AnalyticModel::read(
        &inventory(&unsupported, Limits::default()).unwrap(),
        Options::default()
    )
    .is_err());
    let representation = data
        .entities()
        .values()
        .find(|e| e.components[0].name == "ADVANCED_BREP_SHAPE_REPRESENTATION")
        .unwrap();
    let changed = replace_record(
        SOURCE,
        representation.offset,
        &format!(
            "#{}=ADVANCED_BREP_SHAPE_REPRESENTATION('',(#{}),#{});",
            representation.id, body.id, body.id
        ),
    );
    assert!(AnalyticModel::read(
        &inventory(&changed, Limits::default()).unwrap(),
        Options::default()
    )
    .unwrap_err()
    .detail
    .contains("assigned length context"));
    let end = SOURCE.rfind("ENDSEC;").unwrap();
    let assembly = format!(
        "{}#999999=MAPPED_ITEM('',#{},#{});{}",
        &SOURCE[..end],
        body.id,
        body.id,
        &SOURCE[end..]
    );
    assert!(AnalyticModel::read(
        &inventory(&assembly, Limits::default()).unwrap(),
        Options::default()
    )
    .unwrap_err()
    .detail
    .contains("assembly/placement"));
}
#[test]
fn tiny_direction_magnitudes_normalize_without_reciprocal_overflow() {
    use solid_kernel::step::Value;
    let data = inventory(SOURCE, Limits::default()).unwrap();
    let body = data.raw_bodies().unwrap().remove(0);
    let face = body
        .faces
        .values()
        .find(|f| {
            let surface = &data.entities()[&f.surface].components[0];
            if surface.name != "PLANE" {
                return false;
            }
            let Value::Ref(frame) = surface.args[1] else {
                return false;
            };
            let Value::Ref(axis) = data.entities()[&frame].components[0].args[2] else {
                return false;
            };
            let Value::List(values) = &data.entities()[&axis].components[0].args[1] else {
                return false;
            };
            values.iter().all(|v| match v {
                Value::Real(x) => matches!(*x, -1. | 0. | 1.),
                Value::Integer(x) => matches!(*x, -1 | 0 | 1),
                _ => false,
            })
        })
        .unwrap();
    let frame = match data.entities()[&face.surface].components[0].args[1] {
        Value::Ref(id) => &data.entities()[&id],
        _ => panic!("frame"),
    };
    let axis = match frame.components[0].args[2] {
        Value::Ref(id) => &data.entities()[&id],
        _ => panic!("axis"),
    };
    let components = match &axis.components[0].args[1] {
        Value::List(v) => v,
        _ => panic!("direction"),
    };
    let values = components
        .iter()
        .map(|v| match v {
            Value::Real(v) => *v,
            Value::Integer(v) => *v as f64,
            _ => panic!("direction"),
        })
        .collect::<Vec<_>>();
    assert!(values.iter().all(|v| matches!(*v, -1. | 0. | 1.)));
    let literal = values
        .iter()
        .map(|v| format!("{:e}", v * 1e-320))
        .collect::<Vec<_>>()
        .join(",");
    let changed = replace_record(
        SOURCE,
        axis.offset,
        &format!("#{}=DIRECTION('',({literal}));", axis.id),
    );
    AnalyticModel::read(
        &inventory(&changed, Limits::default()).unwrap(),
        Options::default(),
    )
    .unwrap();
}
#[cfg(feature = "occt")]
#[test]
fn independent_occt_cube_and_cylinder_exports_have_correct_normals_and_chords() {
    use solid_kernel::cad::Brep;
    let cube = Brep::box_from_corners([0., 0., 0.], [2., 3., 4.])
        .to_step_text()
        .unwrap();
    let model = AnalyticModel::read(
        &inventory(&cube, Limits::default()).unwrap(),
        Options::default(),
    )
    .unwrap();
    assert_eq!(model.body().faces.len(), 6);
    let mut cube_area = 0.;
    for face in model.body().faces.values() {
        let trims = model.sampled_planar_trims(face.id, 1000).unwrap();
        assert_eq!(trims.loops.len(), 1);
        let points = &trims.loops[0].points;
        assert_eq!(points.len(), 4);
        cube_area += (0..points.len())
            .map(|i| {
                let a = points[i];
                let b = points[(i + 1) % points.len()];
                a[0] * b[1] - a[1] * b[0]
            })
            .sum::<f64>()
            .abs()
            / 2.;
        let id = face.bounds[0].edges[0].edge;
        let p = model.edge_points(id).unwrap()[0];
        let uv = model.surface(face.id).unwrap().project(p).unwrap().chart;
        let n = model.declared_face_normal(face.id, uv).unwrap();
        let outward = (p[0] - 1.) * n[0] + (p[1] - 1.5) * n[1] + (p[2] - 2.) * n[2];
        assert!(outward > 0.);
    }
    assert!((cube_area - 52.).abs() < 1e-12);
    let cylinder = Brep::cylinder([0., 0., 0.], 2., [0., 0., 1.], 3.)
        .to_step_text()
        .unwrap();
    let model = AnalyticModel::read(
        &inventory(&cylinder, Limits::default()).unwrap(),
        Options::default(),
    )
    .unwrap();
    assert_eq!(model.body().faces.len(), 3);
    let mut circles = 0;
    for edge in model.body().edges.values() {
        let p = model.edge_points(edge.id).unwrap();
        if p.len() > 2 {
            circles += 1;
            for pair in p.windows(2) {
                let mid = [
                    (pair[0][0] + pair[1][0]) / 2.,
                    (pair[0][1] + pair[1][1]) / 2.,
                ];
                assert!(2. - mid[0].hypot(mid[1]) <= Options::default().chord_mm);
            }
        }
    }
    assert_eq!(circles, 2);
}

#[test]
fn bracket_planar_trims_classify_holes_without_bound_order_guesses() {
    let data = inventory(SOURCE, Limits::default()).unwrap();
    let model = AnalyticModel::read(&data, Options::default()).unwrap();
    let mut planes = 0;
    let mut holes = 0;
    for face in model.body().faces.values() {
        if matches!(model.surface(face.id), Some(Surface::Plane(_))) {
            assert!(matches!(
                model.sampled_planar_trims(face.id, 0).unwrap_err().kind,
                solid_kernel::step::ErrorKind::Limit
            ));
            let trims = model.sampled_planar_trims(face.id, 2_000_000).unwrap();
            assert_eq!(trims.loops.len(), face.bounds.len());
            assert_eq!(trims.loops.iter().filter(|l| l.is_outer).count(), 1);
            holes += trims.loops.iter().filter(|l| !l.is_outer).count();
            planes += 1;
        } else {
            assert!(model.sampled_planar_trims(face.id, 2_000_000).is_err());
        }
    }
    assert_eq!(planes, 28);
    assert!(holes > 0);
}

#[test]
fn bracket_cylindrical_trims_close_across_the_periodic_seam() {
    let model = AnalyticModel::read(
        &inventory(SOURCE, Limits::default()).unwrap(),
        Options::default(),
    )
    .unwrap();
    let mut cylinders = 0;
    for face in model.body().faces.values() {
        if matches!(model.surface(face.id), Some(Surface::Cylinder { .. })) {
            assert!(matches!(
                model
                    .sampled_cylindrical_trims(face.id, 0)
                    .unwrap_err()
                    .kind,
                solid_kernel::step::ErrorKind::Limit
            ));
            let trims = model.sampled_cylindrical_trims(face.id, 2_000_000).unwrap();
            assert!(trims.boundary.is_outer);
            for chart in trims.boundary.points {
                assert!(model
                    .surface(face.id)
                    .unwrap()
                    .point(chart)
                    .unwrap()
                    .iter()
                    .all(|x| x.is_finite()));
            }
            cylinders += 1;
        }
    }
    assert_eq!(cylinders, 10);
}

#[cfg(feature = "occt")]
#[test]
fn independent_cylinder_chart_has_exact_developed_area() {
    use solid_kernel::cad::Brep;
    let text = Brep::cylinder([7., -4., 2.], 2., [0., 0., 1.], 3.)
        .to_step_text()
        .unwrap();
    let model = AnalyticModel::read(
        &inventory(&text, Limits::default()).unwrap(),
        Options::default(),
    )
    .unwrap();
    let mut lateral_faces = 0;
    for face in model.body().faces.values() {
        if matches!(model.surface(face.id), Some(Surface::Cylinder { .. })) {
            let trims = model.sampled_cylindrical_trims(face.id, 100_000).unwrap();
            let p = &trims.boundary.points;
            let area = (0..p.len())
                .map(|i| {
                    let a = p[i];
                    let b = p[(i + 1) % p.len()];
                    a[0] * b[1] - a[1] * b[0]
                })
                .sum::<f64>()
                .abs()
                / 2.;
            assert!((area - 12. * std::f64::consts::PI).abs() < 1e-12, "{area}");
            lateral_faces += 1;
        }
    }
    assert_eq!(lateral_faces, 1);
}

#[cfg(feature = "occt")]
#[test]
fn uncertain_tilted_cylinder_chart_is_refused_without_relaxing_turn_bound() {
    use solid_kernel::cad::Brep;
    let text = Brep::cylinder([7., -4., 2.], 2., [1., 2., 3.], 3.)
        .to_step_text()
        .unwrap();
    let model = AnalyticModel::read(
        &inventory(&text, Limits::default()).unwrap(),
        Options::default(),
    )
    .unwrap();
    let face = model
        .body()
        .faces
        .values()
        .find(|f| matches!(model.surface(f.id), Some(Surface::Cylinder { .. })))
        .unwrap();
    // The export's finite precision is sufficient for analytic consistency but
    // not this stricter single-turn chart certificate. No partial chart escapes.
    let error = model
        .sampled_cylindrical_trims(face.id, 100_000)
        .unwrap_err();
    assert!(
        error.detail.contains("cannot certify a single-turn"),
        "{error}"
    );
}
