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
            other => panic!("the bracket has only planes and cylinders: {other:?}"),
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
fn assert_axial_bar_patch(
    mesh: fea_core::Mesh,
    top_faces: Vec<Vec<usize>>,
    bottom_nodes: Vec<usize>,
) {
    use fea_core::{Loads, Model, SurfaceLoad};
    let (e, sigma) = (200e3, 50.);
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

#[cfg(feature = "occt")]
#[test]
fn native_step_to_checked_distance_solid_and_both_tet_orders_pass_tension_patch() {
    use fea_core::Elastic;
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
    let (e, nu) = (200e3, 0.3);
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
        assert_axial_bar_patch(mesh, top_faces, bottom_nodes);
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
    assert_eq!(plc.segments().len(), 1431);
    assert_eq!(progress.first(), Some(&(0, 954)));
    assert_eq!(progress.last(), Some(&(954, 954)));
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
    let face_at = |z: f64| {
        let facet = seed
            .plc()
            .facets()
            .iter()
            .position(|f| f.iter().all(|v| seed.plc().vertices()[*v][2] == z))
            .unwrap();
        seed.plc().facet_source_reference(facet).unwrap()
    };
    let (top_reference, bottom_reference) = (face_at(3.), face_at(0.));
    let generated = seed
        .into_solver_mesh(fea_core::Elastic::new(200e3, 0.3))
        .unwrap();
    assert!(generated.unresolved_faces().is_empty());
    let top_faces = generated.boundary(top_reference).unwrap().to_vec();
    let bottom_nodes = generated
        .boundary(bottom_reference)
        .unwrap()
        .iter()
        .flatten()
        .copied()
        .collect();
    let (mesh, metadata) = generated.into_parts();
    assert_eq!(metadata.source_document, Some(data.document_fingerprint()));
    assert_axial_bar_patch(mesh, top_faces, bottom_nodes);
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

#[cfg(feature = "occt")]
#[test]
fn native_hollow_tube_preserves_the_through_hole_and_analytic_volume() {
    use solid_kernel::cad::{Brep, Loop, Profile};
    let brep = Brep::extrude(
        &Profile {
            outer: Loop::Circle {
                centre: [0., 0.],
                radius: 2.,
            },
            holes: vec![Loop::Circle {
                centre: [0., 0.],
                radius: 1.,
            }],
        },
        0.,
        3.,
    )
    .unwrap();
    let text = brep.to_step_text().unwrap();
    let data = inventory(&text, Limits::default()).unwrap();
    let model = AnalyticModel::read(
        &data,
        Options {
            chord_mm: 0.001,
            ..Options::default()
        },
    )
    .unwrap();
    let surface = model
        .sampled_surface(solid_kernel::native_surface::Limits::default(), |_| true)
        .unwrap();
    assert_eq!(surface.source_faces.len(), 4);
    let exact = std::f64::consts::PI * 9.;
    assert!((surface.signed_volume_mm3 - exact).abs() / exact < 1e-3);
    let checked = surface.check_intersections(2_000_000, |_| true).unwrap();
    let solid = checked.into_distance_solid();
    assert!(solid.solid().signed_distance([0., 0., 1.5]) > 0.99);
    assert!(solid.solid().signed_distance([1.5, 0., 1.5]) < -0.49);
    assert!(solid.solid().signed_distance([3., 0., 1.5]) > 0.99);
}

#[cfg(feature = "occt")]
#[test]
fn valid_thin_wall_and_near_boundary_seeds_cannot_bypass_solver_quality_limits() {
    use solid_kernel::{
        plc::{Limits as PlcLimits, ValidatedPlc},
        plc_star::StarSeed,
    };
    for (height, center) in [(0.01, [0.5, 0.5, 0.005]), (3., [1e-12, 0.5, 1.5])] {
        let text = solid_kernel::cad::Brep::box_from_corners([0., 0., 0.], [1., 1., height])
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
        assert!((surface.signed_volume_mm3 - height).abs() < 1e-12);
        let plc = ValidatedPlc::new(
            surface.check_intersections(10000, |_| true).unwrap(),
            PlcLimits::default(),
            |_, _| true,
        )
        .unwrap();
        let seed = StarSeed::new(plc, center, 100, |_, _| true).unwrap();
        assert_eq!(seed.topology().cells().len(), 12);
        let error = seed
            .into_solver_mesh(fea_core::Elastic::new(200e3, 0.3))
            .unwrap_err();
        assert!(
            matches!(error,fea_core::meshing::MeshError::Failed(ref detail) if detail.contains("mesh quality below the limits"))
        );
    }
}

#[cfg(feature = "occt")]
#[test]
fn bounded_kernel_trial_improves_interior_quality_without_moving_cad_constraints() {
    use solid_kernel::{
        plc::{Limits as PlcLimits, ValidatedPlc},
        plc_star::{StarSeed, TrialPhase},
    };
    let text = solid_kernel::cad::Brep::box_from_corners([0., 0., 0.], [1., 1., 3.])
        .to_step_text()
        .unwrap();
    let data = inventory(&text, Limits::default()).unwrap();
    let model = AnalyticModel::read(&data, Options::default()).unwrap();
    let seed = |point| {
        let surface = model
            .sampled_surface(solid_kernel::native_surface::Limits::default(), |_| true)
            .unwrap();
        let plc = ValidatedPlc::new(
            surface.check_intersections(10000, |_| true).unwrap(),
            PlcLimits::default(),
            |_, _| true,
        )
        .unwrap();
        StarSeed::new(plc, point, 100, |_, _| true).unwrap()
    };
    let original = seed([1e-12, 0.5, 1.5]);
    let boundary_points = original.plc().vertices().to_vec();
    let boundary_facets = original.plc().facets().to_vec();
    let mut phases = Vec::new();
    let (improved, report) = original
        .try_vertex_mean_kernel(|p| {
            phases.push(p.phase);
            true
        })
        .unwrap();
    assert!(report.accepted);
    assert_eq!(report.before_point, [1e-12, 0.5, 1.5]);
    assert_eq!(report.candidate_point, [0.5, 0.5, 1.5]);
    assert!(report.after_quality.min_scaled_jacobian > report.before_quality.min_scaled_jacobian);
    assert!(report.after_quality.max_edge_ratio <= report.before_quality.max_edge_ratio);
    assert_eq!(report.candidate_quality, Some(report.after_quality));
    assert_eq!(improved.plc().vertices(), boundary_points);
    assert_eq!(
        &improved.topology().points()[..boundary_points.len()],
        boundary_points
    );
    assert_eq!(improved.plc().facets(), boundary_facets);
    assert!(phases.contains(&TrialPhase::CandidateGeometry));
    assert_eq!(phases.last(), Some(&TrialPhase::Complete));
    eprintln!("kernel quality trial: {report:?}");
    let reference_at = |z| {
        let i = improved
            .plc()
            .facets()
            .iter()
            .position(|f| f.iter().all(|v| improved.plc().vertices()[*v][2] == z))
            .unwrap();
        improved.plc().facet_source_reference(i).unwrap()
    };
    let (top, bottom) = (reference_at(3.), reference_at(0.));
    let mesh = improved
        .into_solver_mesh(fea_core::Elastic::new(200e3, 0.3))
        .unwrap();
    let top_faces = mesh.boundary(top).unwrap().to_vec();
    let bottom_nodes = mesh
        .boundary(bottom)
        .unwrap()
        .iter()
        .flatten()
        .copied()
        .collect();
    assert_axial_bar_patch(mesh.into_parts().0, top_faces, bottom_nodes);
    let (unchanged, report) = seed([0.5, 0.5, 1.5])
        .try_vertex_mean_kernel(|_| true)
        .unwrap();
    assert!(!report.accepted);
    assert_eq!(report.before_quality, report.after_quality);
    assert_eq!(unchanged.topology().points().last(), Some(&[0.5, 0.5, 1.5]));
    let cancelled =
        seed([1e-12, 0.5, 1.5]).try_vertex_mean_kernel(|p| p.phase != TrialPhase::Complete);
    assert_eq!(
        cancelled.err().unwrap().kind,
        solid_kernel::step::ErrorKind::Cancelled
    );
}

#[cfg(feature = "occt")]
#[test]
fn kernel_trial_refuses_a_vertex_mean_outside_a_concave_domains_kernel() {
    use solid_kernel::{
        cad::{Brep, Loop, Profile},
        plc::{Limits as PlcLimits, ValidatedPlc},
        plc_star::StarSeed,
    };
    let text = Brep::extrude(
        &Profile {
            outer: Loop::Polygon(vec![
                [0., 0.],
                [3., 0.],
                [3., 1.],
                [1., 1.],
                [1., 3.],
                [0., 3.],
            ]),
            holes: vec![],
        },
        0.,
        1.,
    )
    .unwrap()
    .to_step_text()
    .unwrap();
    let model = AnalyticModel::read(
        &inventory(&text, Limits::default()).unwrap(),
        Options::default(),
    )
    .unwrap();
    let checked = model
        .sampled_surface(solid_kernel::native_surface::Limits::default(), |_| true)
        .unwrap()
        .check_intersections(10000, |_| true)
        .unwrap();
    let plc = ValidatedPlc::new(checked, PlcLimits::default(), |_, _| true).unwrap();
    assert!((plc.checked_surface().surface().signed_volume_mm3 - 5.).abs() < 1e-12);
    let seed = StarSeed::new(plc, [0.5; 3], 100, |_, _| true).unwrap();
    let before_points = seed.topology().points().to_vec();
    let before_cells = seed.topology().cells().to_vec();
    let (seed, report) = seed.try_vertex_mean_kernel(|_| true).unwrap();
    assert!(!report.accepted);
    assert!(report.candidate_point[0] > 1. && report.candidate_point[1] > 1.);
    assert!(report
        .refusal
        .as_ref()
        .unwrap()
        .contains("not strictly inside boundary facet"));
    assert!(report.candidate_quality.is_none());
    assert_eq!(report.before_quality, report.after_quality);
    assert_eq!(seed.topology().points(), before_points);
    assert_eq!(seed.topology().cells(), before_cells);
}

#[test]
fn source_vertex_unit_underflow_refuses_at_its_original_point_record() {
    let data = inventory(SOURCE, Limits::default()).unwrap();
    let body = data.raw_bodies().unwrap().remove(0);
    let vertex = body.vertices.values().next().unwrap();
    let point = &data.entities()[&vertex.point];
    let changed = replace_record(
        SOURCE,
        point.offset,
        &format!("#{}=CARTESIAN_POINT('',(1.E-310,0.,0.));", point.id),
    )
    .replace(".MILLI.", ".YOCTO.");
    let data = inventory(&changed, Limits::default()).unwrap();
    let error = AnalyticModel::read(&data, Options::default()).unwrap_err();
    assert_eq!(error.kind, solid_kernel::step::ErrorKind::Geometry);
    assert_eq!(error.offset, data.entities()[&point.id].offset);
    assert!(error.detail.contains("underflow"));
}

/// Onshape (the ABC dataset) writes the hole loops of a face as `FACE_OUTER_BOUND` too. The roles are decided by the loop
/// geometry, so relabelling every hole loop of a plate with a hole must not change the sampled shell or its volume.
#[test]
fn hole_loops_declared_as_outer_bounds_read_like_face_bounds() {
    let plain = include_str!("data/edit-plate-centre-hole.step");
    assert!(plain.contains("FACE_BOUND("), "the fixture has hole loops");
    let relabelled = plain.replace("FACE_BOUND(", "FACE_OUTER_BOUND(");
    let sample = |text: &str| {
        let data = inventory(text, Limits::default()).unwrap();
        AnalyticModel::read(&data, Options::default())
            .unwrap()
            .sampled_surface(solid_kernel::native_surface::Limits::default(), |_| true)
            .unwrap()
    };
    let (a, b) = (sample(plain), sample(&relabelled));
    assert_eq!(a.mesh.triangles.len(), b.mesh.triangles.len());
    assert_eq!(a.signed_volume_mm3, b.signed_volume_mm3);
    assert!(a.signed_volume_mm3 > 0.);
}

/// A committed fixture merged with a copy of itself (ids of the copy shifted): a multi-body part with two equal solids.
fn two_body_document() -> String {
    let a = include_str!("data/edit-plate.step").to_string();
    let b = a.clone();
    let data_of = |t: &str| {
        let start = t.find("DATA;").unwrap() + "DATA;".len();
        let end = t.rfind("ENDSEC;").unwrap();
        (start, end)
    };
    let (_, a_end) = data_of(&a);
    let (b_start, b_end) = data_of(&b);
    let max_id = a
        .split('#')
        .skip(1)
        .filter_map(|s| s.split(|c: char| !c.is_ascii_digit()).next()?.parse::<u64>().ok())
        .max()
        .unwrap();
    let mut shifted = String::new();
    let body = &b[b_start..b_end];
    let mut rest = body;
    while let Some(i) = rest.find('#') {
        shifted.push_str(&rest[..i]);
        let digits: String = rest[i + 1..].chars().take_while(char::is_ascii_digit).collect();
        shifted.push_str(&format!("#{}", digits.parse::<u64>().unwrap() + max_id));
        rest = &rest[i + 1 + digits.len()..];
    }
    shifted.push_str(rest);
    format!("{}{}{}", &a[..a_end], shifted, &a[a_end..])
}

#[test]
fn multi_body_document_reads_one_model_per_solid() {
    let text = two_body_document();
    let data = inventory(&text, Limits::default()).unwrap();
    assert!(
        AnalyticModel::read(&data, Options::default()).is_err(),
        "the single-body reader still refuses a multi-body document"
    );
    let models = AnalyticModel::read_all(&data, Options::default()).unwrap();
    assert_eq!(models.len(), 2);
    let volumes: Vec<f64> = models
        .iter()
        .map(|m| {
            m.sampled_surface(solid_kernel::native_surface::Limits::default(), |_| true)
                .unwrap()
                .signed_volume_mm3
        })
        .collect();
    assert!(volumes[0] > 0. && volumes[0] == volumes[1], "{volumes:?}");
    let single = inventory(include_str!("data/edit-plate.step"), Limits::default()).unwrap();
    let one = AnalyticModel::read(&single, Options::default())
        .unwrap()
        .sampled_surface(solid_kernel::native_surface::Limits::default(), |_| true)
        .unwrap()
        .signed_volume_mm3;
    assert_eq!(volumes[0], one);
}

/// Onshape writes the wall of a cylinder (a through-hole) or the cone of a chamfer as one face bounded by its two circles and
/// no seam edge. An OpenCASCADE fixture with the seam removed and the circles split into two bounds must sample to the same
/// shell as the seamed original.
#[test]
fn unseamed_cylinder_and_cone_bands_get_a_seam_and_sample_like_the_seamed_original() {
    for seamed in [
        include_str!("data/tilted-cylinder.step"),
        include_str!("data/frustum.step"),
    ] {
        let unseamed = seamed
            .replace(
                "#17 = ADVANCED_FACE('',(#18),#31,.T.);",
                "#17 = ADVANCED_FACE('',(#18,#900),#31,.T.);",
            )
            .replace(
                "#19 = EDGE_LOOP('',(#20,#54,#77,#104));",
                "#19 = EDGE_LOOP('',(#20));\n#900 = FACE_BOUND('',#901,.T.);\n#901 = EDGE_LOOP('',(#77));",
            );
        assert_ne!(seamed, unseamed);
        let sample = |text: &str| {
            let data = inventory(text, Limits::default()).unwrap();
            AnalyticModel::read(&data, Options::default())
                .unwrap()
                .sampled_surface(
                    solid_kernel::native_surface::Limits {
                        chart_policy:
                            solid_kernel::cylindrical_step::ChartPolicy::CanonicalizeWithinTolerance,
                        ..Default::default()
                    },
                    |_| true,
                )
                .unwrap()
        };
        let (a, b) = (sample(seamed), sample(&unseamed));
        assert_eq!(a.mesh.triangles.len(), b.mesh.triangles.len());
        assert_eq!(a.mesh.vertices.len(), b.mesh.vertices.len());
        assert!((a.signed_volume_mm3 - b.signed_volume_mm3).abs() < 1e-9 * a.signed_volume_mm3);
        assert!(b.signed_volume_mm3 > 0.);
    }
}

#[test]
fn shallow_cone_band_seams_when_axial_gap_is_below_geometry_tolerance() {
    let seamed = solid_kernel::native_brep::NativeBrep::lathe(&[
        [0., 0.], [9.6493746, 0.], [10.0073439, 0.002238917], [0., 0.002238917],
    ]).unwrap().to_step_text();
    let unseamed = seamed
        .replace("#56 = ADVANCED_FACE('',(#55),#49,.T.);", "#56 = ADVANCED_FACE('',(#55,#74),#49,.T.);")
        .replace("#54 = EDGE_LOOP('',(#50,#51,#52,#53));", "#54 = EDGE_LOOP('',(#50));\n#74 = FACE_BOUND('',#75,.T.);\n#75 = EDGE_LOOP('',(#52));");
    assert_ne!(seamed, unseamed);
    let data = inventory(&unseamed, Limits::default()).unwrap();
    let model = AnalyticModel::read(&data, Options::foreign_cad(0.05)).unwrap();
    assert_eq!(model.body().faces[&56].bounds.len(), 1);
    assert!(model.sampled_surface(solid_kernel::native_surface::Limits {
        chart_policy: solid_kernel::cylindrical_step::ChartPolicy::CanonicalizeWithinTolerance,
        ..Default::default()
    }, |_| true).unwrap().signed_volume_mm3 > 0.);
}

/// The wall of a tilted hole is a cylinder bounded by two ellipses. Without a seam the second ellipse has no vertex at the
/// first one's angle, so the heal must split an ellipse edge (before, only circle and line edges could be split and the face
/// stayed unseamed: ABC corpus, 12 files with ellipse or spline loops). The shell must equal the seamed original's.
#[test]
fn unseamed_cylinder_band_with_ellipse_loops_is_seamed_by_splitting_an_ellipse() {
    let seamed = include_str!("data/tilted-hole.step");
    let unseamed = seamed
        .replace(
            "#456 = ADVANCED_FACE('',(#457),#282,.F.);",
            "#456 = ADVANCED_FACE('',(#457,#900),#282,.F.);",
        )
        .replace(
            "#458 = EDGE_LOOP('',(#459,#460,#481,#482));",
            "#458 = EDGE_LOOP('',(#459));\n#900 = FACE_BOUND('',#901,.F.);\n#901 = EDGE_LOOP('',(#481));",
        );
    assert_ne!(seamed, unseamed);
    let sample = |text: &str| {
        let data = inventory(text, Limits::default()).unwrap();
        AnalyticModel::read(&data, Options::foreign_cad(0.05))
            .unwrap()
            .sampled_surface(
                solid_kernel::native_surface::Limits {
                    chart_policy: solid_kernel::cylindrical_step::ChartPolicy::CanonicalizeWithinTolerance,
                    ..Default::default()
                },
                |_| true,
            )
            .unwrap()
    };
    let (a, b) = (sample(seamed), sample(&unseamed));
    assert!(b.signed_volume_mm3 > 0.);
    assert!((a.signed_volume_mm3 - b.signed_volume_mm3).abs() < 1e-3 * a.signed_volume_mm3, "{} {}", a.signed_volume_mm3, b.signed_volume_mm3);
}

/// Onshape also writes a fillet ring (a torus band between two circles) with no seam. The OpenCASCADE fillet of
/// `cylinder-filleted-top.step` with the seam (a meridian quarter circle used twice) taken out of its loop and the two circles
/// made two bounds must read to the same shell: the seam is a circle arc of the tube, chosen of the two arcs between the
/// circles by the side the face lies on. With the bounds' senses both reversed the loops no longer agree with the faces they
/// share edges with, and the shell is refused rather than closed over the wrong band.
#[test]
fn unseamed_torus_band_gets_a_meridian_seam_and_samples_like_the_seamed_original() {
    let seamed = include_str!("data/cylinder-filleted-top.step");
    let unseamed = seamed
        .replace(
            "#105 = ADVANCED_FACE('',(#106),#66,.T.);",
            "#105 = ADVANCED_FACE('',(#106,#900),#66,.T.);",
        )
        .replace(
            "#107 = EDGE_LOOP('',(#108,#130,#131,#132));",
            "#107 = EDGE_LOOP('',(#130));\n#900 = FACE_BOUND('',#901,.T.);\n#901 = EDGE_LOOP('',(#132));",
        );
    assert_ne!(seamed, unseamed);
    let sample = |text: &str| {
        let data = inventory(text, Limits::default()).unwrap();
        AnalyticModel::read(&data, Options::default()).and_then(|m| {
            m.sampled_surface(
                solid_kernel::native_surface::Limits {
                    chart_policy: solid_kernel::cylindrical_step::ChartPolicy::CanonicalizeWithinTolerance,
                    ..Default::default()
                },
                |_| true,
            )
        })
    };
    let a = sample(seamed).unwrap();
    let b = sample(&unseamed).unwrap();
    assert!(b.signed_volume_mm3 > 0.);
    assert!((a.signed_volume_mm3 - b.signed_volume_mm3).abs() < 1e-3 * a.signed_volume_mm3, "{} {}", a.signed_volume_mm3, b.signed_volume_mm3);
    assert_eq!(a.mesh.face.iter().collect::<std::collections::BTreeSet<_>>().len(), b.mesh.face.iter().collect::<std::collections::BTreeSet<_>>().len());
    // both bounds reversed: the other band of the ring lies on the left, which no neighbouring face agrees with
    let reversed = unseamed
        .replace("#900 = FACE_BOUND('',#901,.T.);", "#900 = FACE_BOUND('',#901,.F.);")
        .replace("#106 = FACE_BOUND('',#107,.T.);", "#106 = FACE_BOUND('',#107,.F.);");
    assert_ne!(unseamed, reversed);
    let refusal = sample(&reversed).err().expect("a wrongly oriented band must be refused");
    assert!(refusal.to_string().contains("consistently oriented"), "{refusal}");
}

/// Onshape writes the pole of a spherical face as an extra bound made of a `VERTEX_LOOP`. The face's chart is local, so the
/// bound carries nothing: adding one to the hemisphere fixture must not change its sampled shell, and neither may the same point bound on
/// a face that keeps an edge loop (before, every non-sphere/cone surface refused with `unsupported EDGE_LOOP entity encoding`;
/// 10 ABC files). A face with only a point is refused.
#[test]
fn point_vertex_loop_bounds_are_skipped_unless_they_are_all_a_face_has() {
    let plain = include_str!("data/hemisphere.step");
    let with_pole = plain.replace(
        "#17 = ADVANCED_FACE('',(#18),#33,.T.);",
        "#17 = ADVANCED_FACE('',(#18,#901),#33,.T.);\n#901 = FACE_BOUND('',#902,.T.);\n#902 = VERTEX_LOOP('',#22);",
    );
    assert_ne!(plain, with_pole);
    let sample = |text: &str| {
        let data = inventory(text, Limits::default()).unwrap();
        AnalyticModel::read(&data, Options::default())
            .unwrap()
            .sampled_surface(solid_kernel::native_surface::Limits::default(), |_| true)
            .unwrap()
    };
    let (a, b) = (sample(plain), sample(&with_pole));
    assert_eq!(a.mesh.triangles.len(), b.mesh.triangles.len());
    assert_eq!(a.signed_volume_mm3, b.signed_volume_mm3);
    // a point bound on the plane face (#79) is no pole but also encloses nothing (a Parasolid acorn vertex): skipped while the
    // face keeps its edge loop, so the shell is the same one
    let on_plane = plain.replace(
        "#79 = ADVANCED_FACE('',(#80),#68,.F.);",
        "#79 = ADVANCED_FACE('',(#80,#901),#68,.F.);\n#901 = FACE_BOUND('',#902,.T.);\n#902 = VERTEX_LOOP('',#22);",
    );
    assert_ne!(plain, on_plane);
    let c = sample(&on_plane);
    assert_eq!(a.mesh.triangles.len(), c.mesh.triangles.len());
    assert_eq!(a.signed_volume_mm3, c.signed_volume_mm3);
    // a plane face bounded by nothing but a point is still refused
    let only_point = plain.replace(
        "#79 = ADVANCED_FACE('',(#80),#68,.F.);",
        "#79 = ADVANCED_FACE('',(#901),#68,.F.);\n#901 = FACE_BOUND('',#902,.T.);\n#902 = VERTEX_LOOP('',#22);",
    );
    assert_ne!(plain, only_point);
    let data = inventory(&only_point, Limits::default()).unwrap();
    assert!(AnalyticModel::read(&data, Options::default()).is_err());
}

/// The two circles of an unseamed band rarely start at the same angle. The tilted cylinder's lower circle is restarted 2 rad
/// away from the upper one: a vertex is inserted on that circle at the upper circle's angle, its edge is split in the
/// cylinder face and in the plane face that shares it, and the shell is the same solid (volume within the facet error).
#[test]
fn unseamed_band_with_misaligned_circle_starts_gets_a_split_vertex_and_a_generator_seam() {
    let seamed = include_str!("data/tilted-cylinder.step");
    let moved = seamed
        .replace(
            "#17 = ADVANCED_FACE('',(#18),#31,.T.);",
            "#17 = ADVANCED_FACE('',(#18,#900),#31,.T.);",
        )
        .replace(
            "#19 = EDGE_LOOP('',(#20,#54,#77,#104));",
            "#19 = EDGE_LOOP('',(#20));\n#900 = FACE_BOUND('',#901,.T.);\n#901 = EDGE_LOOP('',(#77));",
        )
        .replace(
            "#57 = CARTESIAN_POINT('',(7.,-2.335899411324,0.89059960755));",
            "#57 = CARTESIAN_POINT('',(5.247558149682,-4.422903757164,2.866083121547));",
        );
    assert!(moved.contains("5.247558149682"));
    let options = Options::foreign_cad(0.05);
    let read = |text: &str| AnalyticModel::read(&inventory(text, Limits::default()).unwrap(), options).unwrap();
    let model = read(&moved);
    let cylinder = model
        .body()
        .faces
        .values()
        .find(|f| matches!(model.surface(f.id), Some(Surface::Cylinder { .. })))
        .unwrap();
    assert_eq!(cylinder.bounds.len(), 1, "one merged loop with a seam");
    let uses = &cylinder.bounds[0].edges;
    assert_eq!(uses.iter().filter(|u| u.edge == uses[0].edge).count(), 1);
    let seam_uses: Vec<_> = uses
        .iter()
        .filter(|u| uses.iter().filter(|v| v.edge == u.edge).count() == 2)
        .collect();
    assert_eq!(seam_uses.len(), 2, "the new seam is used once each way");
    // the plane face sharing the split circle now lists both of its pieces
    let plane_pieces: usize = model
        .body()
        .faces
        .values()
        .filter(|f| matches!(model.surface(f.id), Some(Surface::Plane(_))))
        .map(|f| f.bounds.iter().map(|b| b.edges.len()).sum::<usize>())
        .sum();
    assert_eq!(plane_pieces, 3, "one plane keeps its circle, the other has two pieces");
    let sample = |m: &AnalyticModel| {
        m.sampled_surface(
            solid_kernel::native_surface::Limits {
                chart_policy: solid_kernel::cylindrical_step::ChartPolicy::CanonicalizeWithinTolerance,
                ..Default::default()
            },
            |_| true,
        )
        .unwrap()
    };
    let (a, b) = (sample(&read(seamed)), sample(&model));
    let relative = (a.signed_volume_mm3 - b.signed_volume_mm3).abs() / a.signed_volume_mm3;
    assert!(relative < 2e-3, "{} vs {}", a.signed_volume_mm3, b.signed_volume_mm3);
}
