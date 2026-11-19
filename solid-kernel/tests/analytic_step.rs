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
    .contains("is not a REPRESENTATION_MAP"));
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

/// Some writers (the NIST `stc_06` file) label the hole loop of a planar face `FACE_OUTER_BOUND` and the true outer loop
/// `FACE_BOUND`, a single declared outer that is geometrically nested. The loop geometry decides, as with several declared
/// outers, so declaring the hole of each holed face (bounds #262 and #369) the only `FACE_OUTER_BOUND` must not change the
/// sampled shell or its volume.
#[test]
fn a_single_declared_outer_that_is_the_hole_reads_like_the_correct_labels() {
    let plain = include_str!("data/edit-plate-centre-hole.step");
    let swapped = plain
        .replace("#262 = FACE_BOUND(", "#262 = FACE_OUTER_BOUND(")
        .replace("#369 = FACE_BOUND(", "#369 = FACE_OUTER_BOUND(");
    assert_ne!(plain, swapped);
    let sample = |text: &str| {
        let data = inventory(text, Limits::default()).unwrap();
        AnalyticModel::read(&data, Options::default())
            .unwrap()
            .sampled_surface(solid_kernel::native_surface::Limits::default(), |_| true)
            .unwrap()
    };
    let (a, b) = (sample(plain), sample(&swapped));
    assert_eq!(a.mesh.triangles.len(), b.mesh.triangles.len());
    assert_eq!(a.signed_volume_mm3, b.signed_volume_mm3);
    assert!(a.signed_volume_mm3 > 0.);
}

/// The hole cylinder of the centre-hole plate (`#282`, radius 4) is moved to radius 4.008 while its edge circles stay at 4:
/// a gap of 8 microns, beyond the foreign-CAD geometry tolerance (chord / 10 = 5 microns) and inside its heal limit.
fn plate_with_hole_radius(radius: f64) -> String {
    let plain = include_str!("data/edit-plate-centre-hole.step");
    let edited = plain.replace("#282 = CYLINDRICAL_SURFACE('',#283,4.);", &format!("#282 = CYLINDRICAL_SURFACE('',#283,{radius:.3});"));
    assert_ne!(plain, edited);
    edited
}

#[test]
fn a_hole_cylinder_off_its_edge_circles_is_healed_onto_the_cylinder_with_the_closed_form_volume() {
    let text = plate_with_hole_radius(4.008);
    let data = inventory(&text, Limits::default()).unwrap();
    // strictly read, the 8 micron disagreement is refused
    assert!(AnalyticModel::read(&data, Options::default()).is_err());
    let model = AnalyticModel::read(&data, Options::foreign_cad(0.05)).unwrap();
    let hole = model.surface(402).unwrap();
    let mut samples = 0;
    for bound in &model.body().faces[&402].bounds {
        for e in &bound.edges {
            for &p in model.edge_points(e.edge).unwrap() {
                samples += 1;
                let residual = hole.project(p).unwrap().residual_mm;
                assert!(residual < 1e-7, "edge #{} sample {residual:e} mm off the cylinder", e.edge);
            }
        }
    }
    assert!(samples > 20, "{samples}");
    let volume = |model: &AnalyticModel| {
        model.sampled_surface(solid_kernel::native_surface::Limits::default(), |_| true).unwrap().signed_volume_mm3
    };
    // the plate without the hole is the strict read of the unmodified file plus its hole (radius 4, height 10)
    let strict = AnalyticModel::read(&inventory(include_str!("data/edit-plate-centre-hole.step"), Limits::default()).unwrap(), Options::default()).unwrap();
    let solid = volume(&strict) + std::f64::consts::PI * 16. * 10.;
    let exact = solid - std::f64::consts::PI * 4.008 * 4.008 * 10.;
    let got = volume(&model);
    assert!((got - exact).abs() < 2e-3 * exact, "{got} against {exact}");
}

/// The vertex of the hole's top rim circle (`#266`, at (24, 15, 10)) is raised 0.02 mm out of the circle's plane: off its
/// declared circle by more than the foreign-CAD geometry tolerance (5 microns) and inside the heal limit (50), as the
/// NIST ctc_05 rd file writes one arc end (1.8e-2 mm). The edge is healed with its vertex onto the top plane and the hole.
#[test]
fn a_vertex_off_its_edge_circle_within_the_heal_limit_is_healed_onto_the_faces() {
    let plain = include_str!("data/edit-plate-centre-hole.step");
    let text = plain.replace("#267 = CARTESIAN_POINT('',(24.,15.,10.));", "#267 = CARTESIAN_POINT('',(24.,15.,10.02));");
    assert_ne!(plain, text);
    let data = inventory(&text, Limits::default()).unwrap();
    let strict = AnalyticModel::read(&data, Options::default()).unwrap_err();
    assert!(strict.detail.contains("not on its declared circle"), "{}", strict.detail);
    // beyond the heal limit the foreign read refuses too
    let far = plain.replace("#267 = CARTESIAN_POINT('',(24.,15.,10.));", "#267 = CARTESIAN_POINT('',(24.,15.,10.06));");
    assert!(AnalyticModel::read(&inventory(&far, Limits::default()).unwrap(), Options::foreign_cad(0.05)).is_err());
    let model = AnalyticModel::read(&data, Options::foreign_cad(0.05)).unwrap();
    let points = model.edge_points(265).unwrap();
    assert!(points.len() > 10, "{}", points.len());
    for p in points {
        assert!((p[2] - 10.).abs() < 1e-7, "rim sample {p:?} is off the top plane");
        assert!(((p[0] - 20.).hypot(p[1] - 15.) - 4.).abs() < 1e-7, "rim sample {p:?} is off the hole");
    }
    let volume = |model: &AnalyticModel| {
        model.sampled_surface(solid_kernel::native_surface::Limits::default(), |_| true).unwrap().signed_volume_mm3
    };
    let conforming = AnalyticModel::read(&inventory(plain, Limits::default()).unwrap(), Options::foreign_cad(0.05)).unwrap();
    let (got, exact) = (volume(&model), volume(&conforming));
    assert!((got - exact).abs() < 1e-9 * exact, "{got} against {exact}");
}

/// Healing is for foreign CAD only: a conforming file reads the same as it did, and a strict read still refuses the gap.
#[test]
fn a_strict_read_never_heals_and_a_conforming_file_reads_the_same_either_way() {
    let plain = include_str!("data/edit-plate-centre-hole.step");
    let data = inventory(plain, Limits::default()).unwrap();
    let strict = AnalyticModel::read(&data, Options::default()).unwrap();
    let foreign = AnalyticModel::read(&data, Options::foreign_cad(0.05)).unwrap();
    for &edge in strict.body().edges.keys() {
        let (a, b) = (strict.edge_points(edge).unwrap(), foreign.edge_points(edge).unwrap());
        // the foreign read samples to a coarser chord, but never moves a vertex of a conforming edge
        assert_eq!(a[0], b[0], "edge #{edge}");
        assert_eq!(a.last(), b.last(), "edge #{edge}");
    }
    let gap = inventory(&plate_with_hole_radius(4.008), Limits::default()).unwrap();
    assert!(AnalyticModel::read(&gap, Options::default()).is_err());
}

/// A half cone (apex at (0, 0, 8), base circle radius 5 in z = 0, the y >= 0 half) with the cone's apex as a vertex: three
/// faces, the cone face bounded by two generators and a half-circle arc (a wedge, as the NIST AP242 files write drill
/// points). The head and the units come from the OpenCASCADE cone fixture.
fn half_cone_document() -> String {
    let cone = include_str!("data/cone.step");
    let head = &cone[..cone.find("#15 = MANIFOLD_SOLID_BREP").unwrap()];
    let tail = &cone[cone.find("#82 = ").unwrap()..];
    let body = "\
#15 = MANIFOLD_SOLID_BREP('',#16);
#16 = CLOSED_SHELL('',(#100,#101,#102));
#20 = CARTESIAN_POINT('',(0.,0.,8.));
#21 = VERTEX_POINT('',#20);
#22 = CARTESIAN_POINT('',(5.,0.,0.));
#23 = VERTEX_POINT('',#22);
#24 = CARTESIAN_POINT('',(-5.,0.,0.));
#25 = VERTEX_POINT('',#24);
#26 = EDGE_CURVE('',#21,#23,#27,.T.);
#27 = LINE('',#20,#28);
#28 = VECTOR('',#29,1.);
#29 = DIRECTION('',(0.529998940003,0.,-0.847998304005));
#30 = EDGE_CURVE('',#21,#25,#31,.T.);
#31 = LINE('',#20,#32);
#32 = VECTOR('',#33,1.);
#33 = DIRECTION('',(-0.529998940003,0.,-0.847998304005));
#34 = EDGE_CURVE('',#23,#25,#35,.T.);
#35 = CIRCLE('',#36,5.);
#36 = AXIS2_PLACEMENT_3D('',#37,#38,#39);
#37 = CARTESIAN_POINT('',(0.,0.,0.));
#38 = DIRECTION('',(0.,0.,1.));
#39 = DIRECTION('',(1.,0.,0.));
#40 = EDGE_CURVE('',#25,#23,#41,.T.);
#41 = LINE('',#24,#42);
#42 = VECTOR('',#43,1.);
#43 = DIRECTION('',(1.,0.,0.));
#44 = AXIS2_PLACEMENT_3D('',#37,#45,#39);
#45 = DIRECTION('',(0.,0.,-1.));
#46 = PLANE('',#44);
#47 = AXIS2_PLACEMENT_3D('',#37,#48,#39);
#48 = DIRECTION('',(0.,-1.,0.));
#49 = PLANE('',#47);
#50 = AXIS2_PLACEMENT_3D('',#20,#45,#39);
#51 = CONICAL_SURFACE('',#50,0.,0.558599315344);
#52 = ORIENTED_EDGE('',*,*,#40,.F.);
#53 = ORIENTED_EDGE('',*,*,#34,.F.);
#54 = EDGE_LOOP('',(#52,#53));
#55 = FACE_BOUND('',#54,.T.);
#56 = ORIENTED_EDGE('',*,*,#40,.T.);
#57 = ORIENTED_EDGE('',*,*,#26,.F.);
#58 = ORIENTED_EDGE('',*,*,#30,.T.);
#59 = EDGE_LOOP('',(#56,#57,#58));
#60 = FACE_BOUND('',#59,.T.);
#61 = ORIENTED_EDGE('',*,*,#34,.T.);
#62 = ORIENTED_EDGE('',*,*,#30,.F.);
#63 = ORIENTED_EDGE('',*,*,#26,.T.);
#64 = EDGE_LOOP('',(#61,#62,#63));
#65 = FACE_BOUND('',#64,.T.);
#100 = ADVANCED_FACE('',(#55),#46,.T.);
#101 = ADVANCED_FACE('',(#60),#49,.T.);
#102 = ADVANCED_FACE('',(#65),#51,.T.);
";
    format!("{head}{body}{tail}")
}

/// The apex is a vertex of the wedge's loop, which then has no angle there: the cone-tip seam (for a circle that winds once
/// around the axis) must leave it alone, and the half-turn arc (exactly half a period in the developed chart, to rounding)
/// must chart. Volume: half of pi r^2 h / 3.
#[test]
fn a_cone_wedge_through_its_apex_samples_its_closed_form_volume() {
    let text = half_cone_document();
    let data = inventory(&text, Limits::default()).unwrap();
    let sampled = AnalyticModel::read(&data, Options::default())
        .unwrap()
        .sampled_surface(solid_kernel::native_surface::Limits::default(), |_| true)
        .unwrap();
    let exact = std::f64::consts::PI * 25. * 8. / 6.;
    assert!(
        (sampled.signed_volume_mm3 - exact).abs() < 2e-2 * exact,
        "{} against {exact}",
        sampled.signed_volume_mm3
    );
}

/// A wedge of a cone (apex at (0, 0, 8), base circle radius 5 in z = 0, the sector from angle 0 to `wedge` radians about the
/// axis, `wedge` over half a turn) with the apex and the axis foot as vertices: four faces. The cone face is the wedge bounded by
/// two generators and a rim arc, which turns at the apex by `wedge`; `loop_start` picks which of its three edges the loop
/// starts with. The head and the units come from the OpenCASCADE cone fixture.
fn cone_wedge_document(turn: f64, wedge: f64, loop_start: usize, semi_angle: f64) -> String {
    let cone = include_str!("data/cone.step");
    let head = &cone[..cone.find("#15 = MANIFOLD_SOLID_BREP").unwrap()];
    let tail = &cone[cone.find("#82 = ").unwrap()..];
    let (s0, c0) = turn.sin_cos();
    let (sw, cw) = (turn + wedge).sin_cos();
    let height = 5. / semi_angle.tan();
    let (rim, drop) = (semi_angle.sin(), semi_angle.cos());
    let order = [["#301", "#302", "#303"], ["#302", "#303", "#301"], ["#303", "#301", "#302"]][loop_start];
    let body = format!(
        "\
#15 = MANIFOLD_SOLID_BREP('',#16);
#16 = CLOSED_SHELL('',(#100,#101,#102,#103));
#200 = CARTESIAN_POINT('',(0.,0.,{height:e}));
#201 = VERTEX_POINT('',#200);
#202 = CARTESIAN_POINT('',({p0x:e},{p0y:e},0.));
#203 = VERTEX_POINT('',#202);
#204 = CARTESIAN_POINT('',({px:e},{py:e},0.));
#205 = VERTEX_POINT('',#204);
#206 = CARTESIAN_POINT('',(0.,0.,0.));
#207 = VERTEX_POINT('',#206);
#210 = EDGE_CURVE('',#201,#203,#211,.T.);
#211 = LINE('',#200,#212);
#212 = VECTOR('',#213,1.);
#213 = DIRECTION('',({r0x:e},{r0y:e},{nd:e}));
#214 = EDGE_CURVE('',#201,#205,#215,.T.);
#215 = LINE('',#200,#216);
#216 = VECTOR('',#217,1.);
#217 = DIRECTION('',({qx:e},{qy:e},{nd:e}));
#218 = EDGE_CURVE('',#203,#205,#219,.T.);
#219 = CIRCLE('',#220,5.);
#220 = AXIS2_PLACEMENT_3D('',#206,#221,#222);
#221 = DIRECTION('',(0.,0.,1.));
#222 = DIRECTION('',({c0:e},{s0:e},0.));
#223 = EDGE_CURVE('',#207,#203,#224,.T.);
#224 = LINE('',#206,#225);
#225 = VECTOR('',#222,1.);
#226 = EDGE_CURVE('',#207,#205,#227,.T.);
#227 = LINE('',#206,#228);
#228 = VECTOR('',#229,1.);
#229 = DIRECTION('',({cw:e},{sw:e},0.));
#230 = EDGE_CURVE('',#201,#207,#231,.T.);
#231 = LINE('',#200,#232);
#232 = VECTOR('',#233,1.);
#233 = DIRECTION('',(0.,0.,-1.));
#240 = AXIS2_PLACEMENT_3D('',#206,#241,#222);
#241 = DIRECTION('',(0.,0.,-1.));
#242 = PLANE('',#240);
#243 = AXIS2_PLACEMENT_3D('',#206,#244,#222);
#244 = DIRECTION('',({s0:e},{nc0:e},0.));
#245 = PLANE('',#243);
#246 = AXIS2_PLACEMENT_3D('',#206,#247,#248);
#247 = DIRECTION('',({nx:e},{ny:e},0.));
#248 = DIRECTION('',(0.,0.,1.));
#249 = PLANE('',#246);
#250 = AXIS2_PLACEMENT_3D('',#200,#241,#222);
#251 = CONICAL_SURFACE('',#250,0.,{semi_angle:e});
#260 = ORIENTED_EDGE('',*,*,#226,.T.);
#261 = ORIENTED_EDGE('',*,*,#218,.F.);
#262 = ORIENTED_EDGE('',*,*,#223,.F.);
#263 = EDGE_LOOP('',(#260,#261,#262));
#264 = FACE_BOUND('',#263,.T.);
#270 = ORIENTED_EDGE('',*,*,#223,.T.);
#271 = ORIENTED_EDGE('',*,*,#210,.F.);
#272 = ORIENTED_EDGE('',*,*,#230,.T.);
#273 = EDGE_LOOP('',(#270,#271,#272));
#274 = FACE_BOUND('',#273,.T.);
#280 = ORIENTED_EDGE('',*,*,#230,.F.);
#281 = ORIENTED_EDGE('',*,*,#214,.T.);
#282 = ORIENTED_EDGE('',*,*,#226,.F.);
#283 = EDGE_LOOP('',(#280,#281,#282));
#284 = FACE_BOUND('',#283,.T.);
#301 = ORIENTED_EDGE('',*,*,#218,.T.);
#302 = ORIENTED_EDGE('',*,*,#214,.F.);
#303 = ORIENTED_EDGE('',*,*,#210,.T.);
#304 = EDGE_LOOP('',({o0},{o1},{o2}));
#305 = FACE_BOUND('',#304,.T.);
#100 = ADVANCED_FACE('',(#264),#242,.T.);
#101 = ADVANCED_FACE('',(#274),#245,.T.);
#102 = ADVANCED_FACE('',(#284),#249,.T.);
#103 = ADVANCED_FACE('',(#305),#251,.T.);
",
        p0x = 5. * c0,
        p0y = 5. * s0,
        r0x = rim * c0,
        r0y = rim * s0,
        s0 = s0,
        c0 = c0,
        nc0 = -c0,
        px = 5. * cw,
        py = 5. * sw,
        qx = rim * cw,
        qy = rim * sw,
        nd = -drop,
        height = height,
        semi_angle = semi_angle,
        nx = -sw,
        ny = cw,
        cw = cw,
        sw = sw,
        o0 = order[0],
        o1 = order[1],
        o2 = order[2],
    );
    format!("{head}{body}{tail}")
}

/// The apex is on the cone face's boundary, and the loop turns there by `wedge`: nearest-angle continuation across the apex
/// goes the short way round for a wedge over half a turn (the face's chart is then folded and the refinement of its chord rule
/// never ends), so the branch after the apex is the one that closes the loop in the angle. Volume: `wedge / 2 pi` of the cone.
#[test]
fn a_cone_wedge_wider_than_half_a_turn_samples_its_closed_form_volume() {
    let mut failures = Vec::new();
    for degrees in [180., 180.00001, 180.001, 180.5, 200., 240., 300., 350.] {
        for (loop_start, turn, semi_angle) in (0..3).flat_map(|l| {
            (0..24).flat_map(move |k| [0.5586, 1.0297].map(move |sa| (l, 0.1 + 0.2617993877991494 * k as f64, sa)))
        }) {
            let wedge = f64::to_radians(degrees);
            let text = cone_wedge_document(turn, wedge, loop_start, semi_angle);
            let exact = wedge * 25. * (5. / semi_angle.tan()) / 6.;
            let outcome = inventory(&text, Limits::default())
                .and_then(|data| AnalyticModel::read(&data, Options::default()))
                .and_then(|model| model.sampled_surface(solid_kernel::native_surface::Limits::default(), |_| true))
                .map_err(|e| e.to_string())
                .and_then(|sampled| {
                    let v = sampled.signed_volume_mm3;
                    if (v - exact).abs() < 2e-2 * exact { Ok(()) } else { Err(format!("volume {v} against {exact}")) }
                });
            if let Err(e) = outcome {
                failures.push(format!("wedge {degrees}, loop start {loop_start}, turn {turn:.3}, semi-angle {semi_angle}: {e}"));
            }
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
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

/// The seam of an unseamed band ends on the second loop at the first loop's angle. A sample of that loop's edge just beside
/// that point must not leave a piece shorter than the geometry tolerance: on ABC `00006600` a cone seam landed 6.47e-4 mm
/// from a circle's midpoint sample (tolerance 0.005 mm), and the corner it left had a 4.7 degree angle that kept cells under
/// the solver gate (Phase 162). Here the frustum's base circle (radius 6) is turned so that one of its interior samples lies
/// 0.002 mm round from the seam's angle: that sample is dropped. (The same gap to the circle's vertex is not handled: the
/// vertex cannot be dropped, and taking it as the seam's end puts the seam off the generator, which the cone chart refuses.)
#[test]
fn a_band_seam_drops_the_loop_sample_beside_its_end() {
    let tolerance = 0.005;
    let seamed = AnalyticModel::read(&inventory(include_str!("data/frustum.step"), Limits::default()).unwrap(), Options::foreign_cad(0.05)).unwrap();
    // the base circle's samples are evenly spaced from its vertex: turn it so that sample `segments / 2` comes 0.002 mm past
    // a full turn, where the seam (at angle 0) ends
    let segments = (seamed.edge_points(78).unwrap().len() - 1) as f64;
    let step = std::f64::consts::TAU / segments;
    let theta = (segments - (segments / 2.).floor()) * step + 0.002 / 6.;
    let fixture = include_str!("data/frustum.step");
    for edited in [
        "#17 = ADVANCED_FACE('',(#18),#31,.T.);",
        "#19 = EDGE_LOOP('',(#20,#54,#77,#104));",
        "#57 = CARTESIAN_POINT('',(6.,-1.469576158977E-15,0.));",
        "#84 = DIRECTION('',(1.,0.,-0.));",
    ] {
        assert_eq!(fixture.matches(edited).count(), 1, "{edited}");
    }
    let unseamed = fixture
        .replace("#17 = ADVANCED_FACE('',(#18),#31,.T.);", "#17 = ADVANCED_FACE('',(#18,#900),#31,.T.);")
        .replace(
            "#19 = EDGE_LOOP('',(#20,#54,#77,#104));",
            "#19 = EDGE_LOOP('',(#20));\n#900 = FACE_BOUND('',#901,.T.);\n#901 = EDGE_LOOP('',(#77));",
        )
        .replace(
            "#57 = CARTESIAN_POINT('',(6.,-1.469576158977E-15,0.));",
            &format!("#57 = CARTESIAN_POINT('',({:.15E},{:.15E},0.));", 6. * theta.cos(), 6. * theta.sin()),
        )
        .replace(
            "#84 = DIRECTION('',(1.,0.,-0.));",
            &format!("#84 = DIRECTION('',({:.15E},{:.15E},0.));", theta.cos(), theta.sin()),
        );
    let data = inventory(&unseamed, Limits::default()).unwrap();
    let model = AnalyticModel::read(&data, Options::foreign_cad(0.05)).unwrap();
    assert_eq!(model.body().faces[&17].bounds.len(), 1, "the band is seamed");
    let mut base = 0;
    for &edge in model.body().edges.keys() {
        let points = model.edge_points(edge).unwrap();
        if points.iter().all(|p| p[2].abs() < 1e-9) {
            base += points.len() - 1;
        }
        for w in points.windows(2) {
            let gap = (0..3).map(|k| (w[1][k] - w[0][k]).powi(2)).sum::<f64>().sqrt();
            assert!(gap >= tolerance, "edge #{edge}: samples {gap:e} mm apart (tolerance {tolerance})");
        }
    }
    // the base circle keeps its sample count: the seam's end replaced the sample beside it
    assert_eq!(base as f64, segments, "theta {theta}");
    let volume = |model: &AnalyticModel| {
        model
            .sampled_surface(
                solid_kernel::native_surface::Limits {
                    chart_policy: solid_kernel::cylindrical_step::ChartPolicy::CanonicalizeWithinTolerance,
                    ..Default::default()
                },
                |_| true,
            )
            .unwrap()
            .signed_volume_mm3
    };
    let (a, b) = (volume(&seamed), volume(&model));
    assert!(b > 0. && (a - b).abs() < 1e-3 * a, "{a} {b}");
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

/// Onshape and SolidWorks write the tip of a cone (a drill point) as one face bounded by its base circle alone, with no seam to
/// the apex. An OpenCASCADE fixture with the seam edge removed from the loop must sample to the same shell as the seamed
/// original, and that shell must hold the closed-form volume of the cone, pi r^2 h / 3 = 200 pi / 3 mm^3,
/// to the sampling chord (0.05 mm on r = 5 mm costs about 1.2 %).
#[test]
fn unseamed_cone_tip_gets_a_seam_to_its_apex_and_samples_like_the_seamed_original() {
    let seamed = include_str!("data/cone.step");
    let unseamed = seamed.replace("#19 = EDGE_LOOP('',(#20,#50,#77));", "#19 = EDGE_LOOP('',(#50));");
    assert_ne!(seamed, unseamed);
    let sample = |text: &str| {
        let data = inventory(text, Limits::default()).unwrap();
        AnalyticModel::read(&data, Options::default())
            .unwrap()
            .sampled_surface(solid_kernel::native_surface::Limits::default(), |_| true)
            .unwrap()
    };
    let (a, b) = (sample(seamed), sample(&unseamed));
    assert_eq!(a.mesh.triangles.len(), b.mesh.triangles.len());
    assert_eq!(a.mesh.vertices.len(), b.mesh.vertices.len());
    assert!((a.signed_volume_mm3 - b.signed_volume_mm3).abs() < 1e-9 * a.signed_volume_mm3);
    let exact = std::f64::consts::PI * 25. * 8. / 3.;
    assert!((b.signed_volume_mm3 - exact).abs() < 2e-2 * exact, "{} against {exact}", b.signed_volume_mm3);
}

/// A sphere face bounded by one circle and no seam, larger than a hemisphere (NIST FTC 06 / STC 06 write a ball segment so).
/// The OpenCASCADE hemisphere fixture with the loop reduced to its rim circle, moved to colatitude `phi`, is the ball of radius
/// 4 cut by the plane z = 4 cos(phi), kept above it: its volume has the closed form of a ball minus a cap. The shell must hold
/// it to the sampling chord, and a face declared on the other side (`same_sense` false: the complementary cap, inward normal)
/// must not read as that solid.
#[test]
fn unseamed_sphere_cap_beyond_a_hemisphere_holds_its_closed_form_volume() {
    use std::f64::consts::PI;
    let source = include_str!("data/hemisphere.step");
    let sample = |text: &str| {
        let data = inventory(text, Limits::default()).unwrap();
        AnalyticModel::read(&data, Options::default())
            .and_then(|m| m.sampled_surface(solid_kernel::native_surface::Limits::default(), |_| true))
    };
    for phi_deg in [100., 130., 145.] {
        let phi = f64::to_radians(phi_deg);
        let (r, z0) = (4. * phi.sin(), 4. * phi.cos());
        let text = source
            .replace("#19 = EDGE_LOOP('',(#20,#51,#52));", "#19 = EDGE_LOOP('',(#52));")
            .replace("#23 = CARTESIAN_POINT('',(4.,-9.797174393179E-16,-4.532431118118E-15));", &format!("#23 = CARTESIAN_POINT('',({r:?},0.,{z0:?}));"))
            .replace("#55 = CIRCLE('',#56,4.);", &format!("#55 = CIRCLE('',#56,{r:?});"))
            .replace("#57 = CARTESIAN_POINT('',(0.,0.,0.));", &format!("#57 = CARTESIAN_POINT('',(0.,0.,{z0:?}));"))
            .replace("#70 = CARTESIAN_POINT('',(-5.,-5.,0.));", &format!("#70 = CARTESIAN_POINT('',(-5.,-5.,{z0:?}));"));
        assert_ne!(text, source);
        let cap = PI * (4. + z0).powi(2) * (3. * 4. - (4. + z0)) / 3.;
        let exact = 4. / 3. * PI * 64. - cap;
        let shell = sample(&text).unwrap_or_else(|e| panic!("colatitude {phi_deg}: {e}"));
        assert!(
            (shell.signed_volume_mm3 - exact).abs() < 2e-2 * exact,
            "colatitude {phi_deg}: {} against {exact}",
            shell.signed_volume_mm3
        );
        // the face on the other side of the same circle is the small cap: a different solid, never this volume
        let flipped = text.replace("#17 = ADVANCED_FACE('',(#18),#33,.T.);", "#17 = ADVANCED_FACE('',(#18),#33,.F.);");
        assert_ne!(flipped, text);
        if let Ok(other) = sample(&flipped) {
            assert!((other.signed_volume_mm3 - exact).abs() > 0.1 * exact, "colatitude {phi_deg}: the flipped face read as the same solid");
        }
    }
}
