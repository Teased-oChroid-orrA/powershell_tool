use solid_kernel::{
    analytic_step::{AnalyticModel, Options, Surface},
    step::{inventory, Limits},
};
const SOURCE: &str = include_str!("../../nema17-motor-bracket.step");
const TILTED_SOURCE: &str = include_str!("data/tilted-cylinder.step");
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

#[test]
fn all_bracket_charts_triangulate_with_shared_boundary_identity() {
    use solid_kernel::chart_mesh::SampleKey;
    use std::collections::BTreeMap;
    let model = AnalyticModel::read(
        &inventory(SOURCE, Limits::default()).unwrap(),
        Options::default(),
    )
    .unwrap();
    let mut shared = BTreeMap::new();
    for face in model.body().faces.values() {
        let mesh = model.triangulate_sampled_chart(face.id, 2_000_000).unwrap();
        assert!(!mesh.triangles.is_empty());
        if let Some(Surface::Cylinder { radius, .. }) = model.surface(face.id) {
            for t in &mesh.triangles {
                for i in 0..3 {
                    let span = (mesh.vertices[t[i]].chart[0]
                        - mesh.vertices[t[(i + 1) % 3]].chart[0])
                        .abs();
                    assert!(span / radius <= std::f64::consts::FRAC_PI_2);
                    let sagitta = radius * (1. - (span / (2. * radius)).cos());
                    assert!(sagitta <= model.chord_tolerance_mm(), "{sagitta}");
                }
            }
        }
        assert!(model.triangulate_sampled_chart(face.id, 0).is_err());
        for v in mesh.vertices {
            if let Some(p) = shared.insert(v.source, v.world) {
                assert_eq!(p, v.world);
            }
            if let SampleKey::Vertex(id) = v.source {
                assert_eq!(v.world, model.vertex(id).unwrap());
            }
        }
    }
    assert_eq!(
        shared
            .keys()
            .filter(|k| !matches!(k, SampleKey::FacePoint { .. }))
            .count(),
        model.body().vertices.len()
            + model
                .body()
                .edges
                .keys()
                .map(|id| model.edge_points(*id).unwrap().len() - 2)
                .sum::<usize>()
    );
}

#[test]
fn bracket_native_sampled_surface_is_closed_without_coordinate_welding() {
    use solid_kernel::native_surface::Limits as SurfaceLimits;
    let model = AnalyticModel::read(
        &inventory(SOURCE, Limits::default()).unwrap(),
        Options::default(),
    )
    .unwrap();
    let mut completed = Vec::new();
    let surface = model
        .sampled_surface(SurfaceLimits::default(), |p| {
            completed.push(p.completed_faces);
            true
        })
        .unwrap();
    assert_eq!(surface.source_faces.len(), 38);
    assert_eq!(completed, (0..=38).collect::<Vec<_>>());
    assert!(surface.signed_volume_mm3 > 0.);
    assert!(matches!(
        model
            .sampled_surface(SurfaceLimits::default(), |_| false)
            .unwrap_err()
            .kind,
        solid_kernel::step::ErrorKind::Cancelled
    ));
    assert!(matches!(
        model
            .sampled_surface(
                SurfaceLimits {
                    vertices: 1,
                    ..SurfaceLimits::default()
                },
                |_| true
            )
            .unwrap_err()
            .kind,
        solid_kernel::step::ErrorKind::Limit
    ));
}

#[cfg(feature = "occt")]
#[test]
fn independent_cube_native_surface_retains_exact_volume_and_all_faces() {
    let text = solid_kernel::cad::Brep::box_from_corners([0., 0., 0.], [2., 3., 4.])
        .to_step_text()
        .unwrap();
    let model = AnalyticModel::read(
        &inventory(&text, Limits::default()).unwrap(),
        Options::default(),
    )
    .unwrap();
    let surface = model
        .sampled_surface(solid_kernel::native_surface::Limits::default(), |_| true)
        .unwrap();
    assert_eq!(surface.source_faces.len(), 6);
    assert_eq!(surface.mesh.vertices.len(), 8);
    assert_eq!(surface.mesh.triangles.len(), 12);
    assert!((surface.signed_volume_mm3 - 24.).abs() < 1e-12);
}

#[test]
fn short_curved_edges_cannot_collapse_to_the_straight_endpoint_chord() {
    use solid_kernel::step::Value;
    let data = inventory(SOURCE, Limits::default()).unwrap();
    let model = AnalyticModel::read(&data, Options::default()).unwrap();
    let mut circles = 0;
    for edge in model.body().edges.values() {
        let mut curve = edge.curve;
        loop {
            let call = &data.entities()[&curve].components[0];
            if matches!(call.name.as_str(), "SURFACE_CURVE" | "SEAM_CURVE") {
                let Value::Ref(base) = call.args[1] else {
                    panic!("base curve")
                };
                curve = base;
                continue;
            }
            if call.name == "CIRCLE" {
                assert!(model.edge_points(edge.id).unwrap().len() >= 3);
                circles += 1;
            }
            break;
        }
    }
    assert_eq!(circles, 26);
    let small = AnalyticModel::read(
        &data,
        Options {
            max_edge_points: 2,
            ..Options::default()
        },
    );
    assert!(matches!(
        small.unwrap_err().kind,
        solid_kernel::step::ErrorKind::Limit
    ));
}

#[cfg(feature = "occt")]
#[test]
fn native_bracket_volume_converges_toward_independent_occt_reference() {
    let data = inventory(SOURCE, Limits::default()).unwrap();
    let reference = solid_kernel::cad::Brep::from_step_text(SOURCE)
        .unwrap()
        .solid(0.005)
        .unwrap()
        .volume();
    let mut observations = Vec::new();
    for chord_mm in [0.05, 0.005] {
        let start = std::time::Instant::now();
        let model = AnalyticModel::read(
            &data,
            Options {
                chord_mm,
                ..Options::default()
            },
        )
        .unwrap();
        let surface = model
            .sampled_surface(solid_kernel::native_surface::Limits::default(), |_| true)
            .unwrap();
        observations.push((
            surface.signed_volume_mm3,
            (surface.signed_volume_mm3 - reference).abs(),
        ));
        eprintln!("native bracket chord={chord_mm} mm: V={} mm3, reference={reference}, vertices={}, triangles={}, seconds={:.6}",surface.signed_volume_mm3,surface.mesh.vertices.len(),surface.mesh.triangles.len(),start.elapsed().as_secs_f64());
    }
    assert!(observations[1].1 < observations[0].1, "{observations:?}");
    assert!(observations[1].1 / reference < 1e-3, "{observations:?}");
}

#[test]
fn bracket_sampled_facets_pass_global_intersection_checks() {
    let model = AnalyticModel::read(
        &inventory(SOURCE, Limits::default()).unwrap(),
        Options::default(),
    )
    .unwrap();
    let surface = model
        .sampled_surface(solid_kernel::native_surface::Limits::default(), |_| true)
        .unwrap();
    let checked = surface.check_intersections(2_000_000, |_| true).unwrap();
    assert_eq!(checked.surface().source_faces.len(), 38);
    assert!(checked.pairs_checked() > 0);
}

#[cfg(feature = "occt")]
#[test]
fn native_step_to_checked_distance_solid_and_both_tet_orders_pass_tension_patch() {
    use fea_core::{Elastic, Loads, Model, SurfaceLoad};
    use solid_kernel::VolumeParams;
    let text = solid_kernel::cad::Brep::box_from_corners([0., 0., 0.], [1., 1., 3.])
        .to_step_text()
        .unwrap();
    let model = AnalyticModel::read(
        &inventory(&text, Limits::default()).unwrap(),
        Options::default(),
    )
    .unwrap();
    let native = model
        .sampled_surface(solid_kernel::native_surface::Limits::default(), |_| true)
        .unwrap()
        .check_intersections(10000, |_| true)
        .unwrap()
        .into_distance_solid();
    let solid = native.solid();
    assert_eq!(native.source_faces().len(), 6);
    assert_eq!(solid.triangle_count(), 12);
    assert!((solid.signed_distance([0.5, 0.5, 1.5]) + 0.5).abs() < 1e-12);
    let normals = solid.face_normals();
    let top = normals.iter().position(|n| n[2] > 0.9).unwrap();
    let bottom = normals.iter().position(|n| n[2] < -0.9).unwrap();
    let top_reference = native.metadata().face_reference(top).unwrap();
    let bottom_reference = native.metadata().face_reference(bottom).unwrap();
    let (e, nu, sigma) = (200e3, 0.3, 50.);
    for quadratic in [false, true] {
        let generated = native
            .mesh_volume(
                Elastic::new(e, nu),
                VolumeParams {
                    base: 0.5,
                    boundary_size: 0.4,
                    quadratic,
                },
                &|_| 1.,
            )
            .unwrap();
        assert!((generated.report().volume - 3.).abs() < 1e-6);
        assert!(generated.unresolved_faces().is_empty());
        let top_faces = generated.boundary(top_reference).unwrap().to_vec();
        let bottom_nodes: Vec<_> = generated
            .boundary(bottom_reference)
            .unwrap()
            .iter()
            .flatten()
            .copied()
            .collect();
        let (mesh, _, metadata) = generated.into_parts();
        assert_eq!(metadata.face_reference(top), Some(top_reference));
        let model = Model::new(mesh).unwrap();
        let mut bc = model.dirichlet();
        for n in bottom_nodes {
            bc.fix(n, 2, 0.);
        }
        for (n, x) in model.mesh.nodes.iter().enumerate() {
            if x[0].abs() < 1e-9 && x[2].abs() < 1e-9 {
                bc.fix(n, 0, 0.);
            }
            if x[1].abs() < 1e-9 && x[2].abs() < 1e-9 {
                bc.fix(n, 1, 0.);
            }
        }
        let mut loads = Loads::default();
        for face in top_faces {
            loads
                .faces
                .push((face, SurfaceLoad::Traction([0., 0., sigma])));
        }
        let solution = model.solve_static(&loads, &bc).unwrap();
        let expected = sigma * 3. / e;
        for (n, x) in model.mesh.nodes.iter().enumerate() {
            if (x[2] - 3.).abs() < 1e-9 {
                assert!((solution.u[n * 3 + 2] - expected).abs() < 1e-6 * expected);
            }
        }
    }
}

#[test]
fn explicit_tilted_cylinder_chart_canonicalization_reports_every_bounded_adjustment() {
    let text = TILTED_SOURCE;
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
    assert!(model.sampled_cylindrical_trims(face.id, 100000).is_err());
    assert_eq!(
        model
            .canonicalized_cylindrical_trims(face.id, 0)
            .unwrap_err()
            .kind,
        solid_kernel::step::ErrorKind::Limit
    );
    let canonical = model
        .canonicalized_cylindrical_trims(face.id, 100000)
        .unwrap();
    assert!(!canonical.adjustments.is_empty());
    let mut reported = std::collections::BTreeSet::new();
    for adjustment in canonical.adjustments {
        assert!(reported.insert((
            adjustment.face,
            adjustment.bound,
            adjustment.edge_use,
            adjustment.sample
        )));
        let bound = model.body().faces[&adjustment.face]
            .bounds
            .iter()
            .find(|b| b.id == adjustment.bound)
            .unwrap();
        assert!(bound
            .edges
            .iter()
            .any(|u| u.id == adjustment.edge_use && u.edge == adjustment.edge));
        assert_ne!(adjustment.before, adjustment.after);
        assert!(adjustment.world_deviation_mm <= model.geometry_tolerance_mm());
        assert!(
            (adjustment.before[0] - adjustment.after[0])
                .hypot(adjustment.before[1] - adjustment.after[1])
                <= model.geometry_tolerance_mm()
        );
    }
}

#[test]
fn explicit_canonicalized_tilted_cylinder_passes_facet_checks_and_keeps_reports() {
    use solid_kernel::{cylindrical_step::ChartPolicy, native_surface::Limits as SurfaceLimits};
    let text = TILTED_SOURCE;
    let model = AnalyticModel::read(
        &inventory(&text, Limits::default()).unwrap(),
        Options {
            chord_mm: 0.001,
            ..Options::default()
        },
    )
    .unwrap();
    assert!(model
        .sampled_surface(SurfaceLimits::default(), |_| true)
        .unwrap_err()
        .detail
        .contains("single-turn"));
    for face in model.body().faces.values() {
        let chart = model
            .triangulate_sampled_chart_with_policy(
                face.id,
                2_000_000,
                ChartPolicy::CanonicalizeWithinTolerance,
            )
            .unwrap();
        for v in chart.vertices {
            match v.source {
                solid_kernel::chart_mesh::SampleKey::Vertex(id) => {
                    assert_eq!(v.world, model.vertex(id).unwrap())
                }
                solid_kernel::chart_mesh::SampleKey::EdgePoint { edge, index } => {
                    assert_eq!(v.world, model.edge_points(edge).unwrap()[index])
                }
                _ => {}
            }
        }
    }
    let surface = model
        .sampled_surface(
            SurfaceLimits {
                chart_policy: ChartPolicy::CanonicalizeWithinTolerance,
                ..SurfaceLimits::default()
            },
            |_| true,
        )
        .unwrap();
    let count = surface.chart_adjustments.len();
    assert!(count > 0);
    let checked = surface.check_intersections(2_000_000, |_| true).unwrap();
    let native = checked.into_distance_solid();
    assert_eq!(native.chart_adjustments().len(), count);
    let exact = 12. * std::f64::consts::PI;
    assert!((native.solid().volume() - exact).abs() / exact < 1e-3);
    for adjustment in native.chart_adjustments() {
        assert!(adjustment.world_deviation_mm <= model.geometry_tolerance_mm());
    }
}

#[test]
fn explicit_orientation_repair_restores_corrupted_source_face_and_preserves_reports() {
    use solid_kernel::{
        native_surface::{GenerationPhase, Limits as SurfaceLimits},
        orientation_repair::OrientationPolicy,
    };
    let original_data = inventory(SOURCE, Limits::default()).unwrap();
    let body = original_data.raw_bodies().unwrap().remove(0);
    let face = body.faces.values().next().unwrap();
    let id = face.id;
    let original_sense = face.same_sense;
    let entity = &original_data.entities()[&id];
    let end = entity.offset + SOURCE[entity.offset..].find(';').unwrap() + 1;
    let mut record = SOURCE[entity.offset..end].to_string();
    let flag = if original_sense { ".T." } else { ".F." };
    let pos = record.rfind(flag).unwrap();
    record.replace_range(pos..pos + 3, if original_sense { ".F." } else { ".T." });
    let changed = replace_record(SOURCE, entity.offset, &record);
    let model = AnalyticModel::read(
        &inventory(&changed, Limits::default()).unwrap(),
        Options::default(),
    )
    .unwrap();
    assert!(model
        .sampled_surface(SurfaceLimits::default(), |_| true)
        .is_err());
    let limits = SurfaceLimits {
        orientation_policy: OrientationPolicy::RepairWithReport,
        ..SurfaceLimits::default()
    };
    let mut phases = Vec::new();
    let surface = model
        .sampled_surface(limits, |p| {
            phases.push(p.phase);
            true
        })
        .unwrap();
    assert!(phases.contains(&GenerationPhase::RepairingOrientation));
    let original = AnalyticModel::read(&original_data, Options::default())
        .unwrap()
        .sampled_surface(SurfaceLimits::default(), |_| true)
        .unwrap();
    assert_eq!(surface.mesh, original.mesh);
    assert_eq!(surface.face_orientation_adjustments.len(), 1);
    let adjustment = &surface.face_orientation_adjustments[0];
    assert_eq!(adjustment.source_face, id);
    assert_eq!(adjustment.before_same_sense, !original_sense);
    assert_eq!(adjustment.after_same_sense, original_sense);
    assert_eq!(model.body().faces[&id].same_sense, !original_sense);
    let native = surface
        .check_intersections(2_000_000, |_| true)
        .unwrap()
        .into_distance_solid();
    assert_eq!(native.face_orientation_adjustments().len(), 1);
    assert_eq!(native.face_orientation_adjustments()[0].source_face, id);
    assert_eq!(
        model
            .sampled_surface(limits, |p| p.phase != GenerationPhase::RepairingOrientation)
            .unwrap_err()
            .kind,
        solid_kernel::step::ErrorKind::Cancelled
    );
}

#[test]
fn native_face_associations_keep_exact_source_identity_through_geometry_checks() {
    let text = include_str!("../../nema17-motor-bracket.step");
    let data = inventory(text, Limits::default()).unwrap();
    let model = AnalyticModel::read(&data, Options::default()).unwrap();
    assert_eq!(model.document_fingerprint(), data.document_fingerprint());
    let native = model
        .sampled_surface(solid_kernel::native_surface::Limits::default(), |_| true)
        .unwrap()
        .check_intersections(2_000_000, |_| true)
        .unwrap()
        .into_distance_solid();
    let (_, metadata) = native.into_parts();
    assert_eq!(metadata.source_document, Some(data.document_fingerprint()));
    for (tag, id) in metadata.source_faces.iter().enumerate() {
        let reference = metadata.face_reference(tag).unwrap();
        assert_eq!(reference.entity, *id);
        let entity = data.resolve_reference(reference).unwrap();
        assert!(entity.components.iter().any(|c| c.name == "ADVANCED_FACE"));
    }
    assert!(metadata
        .face_reference(metadata.source_faces.len())
        .is_none());
    let changed = inventory(&format!("{text}\n"), Limits::default()).unwrap();
    assert!(changed
        .resolve_reference(metadata.face_reference(0).unwrap())
        .is_err());
}

#[test]
fn stored_face_selections_remap_across_changed_native_triangulations() {
    use solid_kernel::step::{DocumentFingerprint, EntityReference};
    let data = inventory(SOURCE, Limits::default()).unwrap();
    let generated = |chord_mm| {
        AnalyticModel::read(
            &data,
            Options {
                chord_mm,
                ..Options::default()
            },
        )
        .unwrap()
        .sampled_surface(solid_kernel::native_surface::Limits::default(), |_| true)
        .unwrap()
        .check_intersections(2_000_000, |_| true)
        .unwrap()
        .into_distance_solid()
    };
    let coarse = generated(0.05);
    let fine = generated(0.005);
    assert!(fine.solid().triangle_count() > coarse.solid().triangle_count());
    let references: Vec<_> = (0..coarse.source_faces().len())
        .map(|tag| coarse.metadata().face_reference(tag).unwrap())
        .collect();
    for reference in &references {
        let tag = fine.metadata().face_tag(*reference).unwrap();
        assert_eq!(fine.metadata().face_reference(tag), Some(*reference));
    }
    let (_, mut reordered) = fine.into_parts();
    reordered.source_faces.reverse();
    for reference in &references {
        let tag = reordered.face_tag(*reference).unwrap();
        assert_eq!(reordered.source_faces[tag], reference.entity);
    }
    let reference = references[0];
    assert!(reordered
        .face_tag(EntityReference {
            document: DocumentFingerprint([0; 32]),
            ..reference
        })
        .is_err());
    assert!(reordered
        .face_tag(EntityReference {
            entity: u64::MAX,
            ..reference
        })
        .is_err());
    reordered.source_faces.push(reference.entity);
    assert!(reordered.face_tag(reference).is_err());
    reordered.source_document = None;
    assert!(reordered.face_tag(reference).is_err());
}

#[test]
fn validated_plc_preserves_every_bracket_constraint_and_source_face() {
    use solid_kernel::plc::{Limits as PlcLimits, ValidatedPlc};
    use solid_kernel::step::ErrorKind;
    let data = inventory(SOURCE, Limits::default()).unwrap();
    let model = AnalyticModel::read(&data, Options::default()).unwrap();
    let checked = || {
        model
            .sampled_surface(solid_kernel::native_surface::Limits::default(), |_| true)
            .unwrap()
            .check_intersections(2_000_000, |_| true)
            .unwrap()
    };
    let original = checked();
    let vertices = original.surface().mesh.vertices.clone();
    let facets = original.surface().mesh.triangles.clone();
    let mut progress = Vec::new();
    let plc = ValidatedPlc::new(original, PlcLimits::default(), |done, total| {
        progress.push((done, total));
        true
    })
    .unwrap();
    assert_eq!(plc.vertices(), vertices);
    assert_eq!(plc.facets(), facets);
    assert_eq!(plc.segments().len(), 1467);
    assert_eq!(progress.first(), Some(&(0, 978)));
    assert_eq!(progress.last(), Some(&(978, 978)));
    for (i, tri) in plc.facets().iter().enumerate() {
        let reference = plc.facet_source_reference(i).unwrap();
        assert!(data
            .resolve_reference(reference)
            .unwrap()
            .components
            .iter()
            .any(|c| c.name == "ADVANCED_FACE"));
        for j in 0..3 {
            let mut edge = [tri[j], tri[(j + 1) % 3]];
            edge.sort();
            assert!(plc.segments().binary_search(&edge).is_ok());
        }
    }
    assert!(plc.facet_source_reference(facets.len()).is_none());
    assert_eq!(
        plc.into_distance_solid().metadata().source_document,
        Some(data.document_fingerprint())
    );
    let limited = ValidatedPlc::new(
        checked(),
        PlcLimits {
            segments: 1,
            ..PlcLimits::default()
        },
        |_, _| true,
    );
    assert_eq!(limited.err().unwrap().kind, ErrorKind::Limit);
    let limited = ValidatedPlc::new(
        checked(),
        PlcLimits {
            facets: 1,
            ..PlcLimits::default()
        },
        |_, _| true,
    );
    assert_eq!(limited.err().unwrap().kind, ErrorKind::Limit);
    let cancelled = ValidatedPlc::new(checked(), PlcLimits::default(), |done, _| done < 256);
    assert_eq!(cancelled.err().unwrap().kind, ErrorKind::Cancelled);
}

#[cfg(feature = "occt")]
#[test]
fn exact_kernel_star_seed_preserves_bar_boundary_and_refuses_nonvisible_center() {
    use solid_kernel::step::ErrorKind;
    use solid_kernel::{
        plc::{Limits as PlcLimits, ValidatedPlc},
        plc_star::StarSeed,
    };
    let text = solid_kernel::cad::Brep::box_from_corners([0., 0., 0.], [1., 1., 3.])
        .to_step_text()
        .unwrap();
    let data = inventory(&text, Limits::default()).unwrap();
    let model = AnalyticModel::read(&data, Options::default()).unwrap();
    let plc = || {
        ValidatedPlc::new(
            model
                .sampled_surface(solid_kernel::native_surface::Limits::default(), |_| true)
                .unwrap()
                .check_intersections(10000, |_| true)
                .unwrap(),
            PlcLimits::default(),
            |_, _| true,
        )
        .unwrap()
    };
    let seed = StarSeed::new(plc(), [0.5, 0.5, 1.5], 100, |_, _| true).unwrap();
    assert_eq!(seed.topology().points().len(), 9);
    assert_eq!(seed.topology().cells().len(), 12);
    assert_eq!(seed.topology().boundary().len(), 12);
    let volume: f64 = seed
        .topology()
        .cells()
        .iter()
        .map(|t| {
            let p = t.map(|id| seed.topology().points()[id]);
            let a: [f64; 3] = std::array::from_fn(|i| p[1][i] - p[0][i]);
            let b: [f64; 3] = std::array::from_fn(|i| p[2][i] - p[0][i]);
            let c: [f64; 3] = std::array::from_fn(|i| p[3][i] - p[0][i]);
            (a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
                + a[2] * (b[0] * c[1] - b[1] * c[0]))
                / 6.
        })
        .sum();
    assert!((volume - 3.).abs() < 1e-12);
    seed.topology()
        .to_mesh(fea_core::Elastic::new(200e3, 0.3))
        .unwrap();
    for i in 0..12 {
        assert!(data
            .resolve_reference(seed.plc().facet_source_reference(i).unwrap())
            .is_ok());
    }
    for center in [[0., 0., 0.], [2., 0.5, 1.5], [f64::NAN, 0., 0.]] {
        assert_eq!(
            StarSeed::new(plc(), center, 100, |_, _| true)
                .err()
                .unwrap()
                .kind,
            ErrorKind::Geometry
        );
    }
    assert_eq!(
        StarSeed::new(plc(), [0.5, 0.5, 1.5], 1, |_, _| true)
            .err()
            .unwrap()
            .kind,
        ErrorKind::Limit
    );
    assert_eq!(
        StarSeed::new(plc(), [0.5, 0.5, 1.5], 100, |_, _| false)
            .err()
            .unwrap()
            .kind,
        ErrorKind::Cancelled
    );
}

#[test]
fn bracket_holes_are_refused_by_the_limited_star_seed_instead_of_filled() {
    use solid_kernel::{
        plc::{Limits as PlcLimits, ValidatedPlc},
        plc_star::StarSeed,
    };
    let data = inventory(SOURCE, Limits::default()).unwrap();
    let model = AnalyticModel::read(&data, Options::default()).unwrap();
    let checked = model
        .sampled_surface(solid_kernel::native_surface::Limits::default(), |_| true)
        .unwrap()
        .check_intersections(2_000_000, |_| true)
        .unwrap();
    let plc = ValidatedPlc::new(checked, PlcLimits::default(), |_, _| true).unwrap();
    let center = std::array::from_fn(|axis| {
        let lo = plc
            .vertices()
            .iter()
            .map(|p| p[axis])
            .fold(f64::INFINITY, f64::min);
        let hi = plc
            .vertices()
            .iter()
            .map(|p| p[axis])
            .fold(f64::NEG_INFINITY, f64::max);
        lo + (hi - lo) * 0.5
    });
    let error = StarSeed::new(plc, center, 10000, |_, _| true)
        .err()
        .unwrap();
    assert_eq!(error.kind, solid_kernel::step::ErrorKind::Geometry);
    assert!(error.detail.contains("not strictly inside boundary facet"));
}
