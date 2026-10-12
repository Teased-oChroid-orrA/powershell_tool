use fea_problem::advice::{boundary_options, recommend, surface_centroids, surface_kind, Advice, BcKind, SurfaceKind};
use fea_problem::{templates, Analysis, Geometry, Load, Problem, Shape, Support};

fn plate() -> Problem {
    let mut p = templates::templates()[0].1.clone();
    p.supports.clear();
    p.loads.clear();
    p.mesh.size = 0.5;
    p
}

#[test]
fn a_free_model_gets_a_support_far_from_the_load_and_a_loaded_one_does_not() {
    let mut p = plate();
    p.loads.push(Load::Pressure { edge: "right".into(), p: 100.0 });
    let adv = recommend(&p);
    let b: Vec<_> = adv.iter().filter_map(|a| if let Advice::Boundary { surface, kind, reason } = a { Some((surface.as_str(), *kind, reason.as_str())) } else { None }).collect();
    assert_eq!(b.len(), 1);
    assert_eq!((b[0].0, b[0].1), ("left", BcKind::Fixed));
    assert!(b[0].2.contains("farthest from the load"));
    // Apply it and the same advice disappears (idempotent: the model is no longer free).
    let Advice::Boundary { .. } = &adv.iter().find(|a| matches!(a, Advice::Boundary { .. })).unwrap() else { unreachable!() };
    adv.iter().find(|a| matches!(a, Advice::Boundary { .. })).unwrap().apply(&mut p).unwrap();
    assert!(p.supports.contains(&Support::fixed("left")));
    assert!(!recommend(&p).iter().any(|a| matches!(a, Advice::Boundary { kind: BcKind::Fixed, .. })));
}

#[test]
fn supported_but_unloaded_gets_a_pin_load_on_the_unused_hole_then_an_opposite_edge_pressure() {
    let mut p = plate();
    p.supports.push(Support::fixed("left"));
    let adv = recommend(&p);
    let kinds: Vec<(String, BcKind)> = adv.iter().filter_map(|a| if let Advice::Boundary { surface, kind, .. } = a { Some((surface.clone(), *kind)) } else { None }).collect();
    assert_eq!(kinds[0], ("hole1".to_string(), BcKind::Bearing));
    assert_eq!(kinds[1], ("right".to_string(), BcKind::Pressure), "the edge farthest from the support");
    // Solids have no bearing load: only the pressure suggestion remains.
    p.analysis = Analysis::Solid;
    let k2: Vec<BcKind> = recommend(&p).iter().filter_map(|a| if let Advice::Boundary { kind, .. } = a { Some(*kind) } else { None }).collect();
    assert_eq!(k2, [BcKind::Pressure]);
    let mut q = plate();
    q.supports.push(Support::fixed("left"));
    Advice::Boundary { surface: "hole1".into(), kind: BcKind::Bearing, reason: String::new() }.apply(&mut q).unwrap();
    assert!(matches!(q.loads[0], Load::Bearing { hole: 1, .. }));
    assert!(Advice::Boundary { surface: "right".into(), kind: BcKind::Bearing, reason: String::new() }.apply(&mut q).is_err());
}

#[test]
fn coarse_mesh_for_the_geometry_is_flagged_with_numbers() {
    let mut p = plate();
    p.mesh.size = 2.0; // plate is 12 x 4: 2 elements across the 4 thick side
    p.supports.push(Support::fixed("left"));
    p.loads.push(Load::Pressure { edge: "right".into(), p: 1.0 });
    let adv = recommend(&p);
    let size = adv.iter().find_map(|a| if let Advice::MeshSize { size, reason } = a { Some((*size, reason.clone())) } else { None }).expect("mesh advice");
    assert!(size.0 <= 4.0 / 6.0 * 1.0 + 1e-9 && size.0 > 0.0, "{size:?}");
    assert!(size.1.contains("fewer than 6 elements"));
    assert!(adv.iter().any(|a| matches!(a, Advice::HoleFactor { .. })), "hole radius 0.5 with 2.0 x 0.1 elements");
    // Applying the advice makes it go away.
    for a in &adv {
        a.apply(&mut p).unwrap();
    }
    assert!(recommend(&p).iter().all(|a| !matches!(a, Advice::MeshSize { .. } | Advice::HoleFactor { .. })), "{:?}", recommend(&p));
}

#[test]
fn thin_extrusions_get_more_layers() {
    let mut p = plate();
    p.analysis = Analysis::Solid;
    p.supports.push(Support::fixed("left"));
    p.loads.push(Load::Pressure { edge: "right".into(), p: 1.0 });
    p.geometry = Geometry::Sketch { outer: Shape::Rect { x0: 0.0, y0: 0.0, x1: 12.0, y1: 4.0 }, holes: vec![], depth: 0.25, layers: 1, regions: Vec::new() };
    let adv = recommend(&p);
    let a = adv.iter().find(|a| matches!(a, Advice::Layers { .. })).expect("layers");
    a.apply(&mut p).unwrap();
    let Geometry::Sketch { layers, .. } = &p.geometry else { panic!() };
    assert_eq!(*layers, 3);
    assert!(!recommend(&p).iter().any(|a| matches!(a, Advice::Layers { .. })));
}

#[test]
fn options_follow_the_intent_and_the_surface() {
    let mut p = plate();
    assert_eq!(boundary_options(&p, "left"), [BcKind::Fixed, BcKind::Roller, BcKind::Pressure]);
    assert_eq!(boundary_options(&p, "hole1"), [BcKind::Fixed, BcKind::Bearing, BcKind::Pressure]);
    p.supports.push(Support::fixed("left"));
    assert_eq!(boundary_options(&p, "right"), [BcKind::Pressure, BcKind::Fixed, BcKind::Roller]);
    assert_eq!(boundary_options(&p, "hole1")[0], BcKind::Bearing);
    p.analysis = Analysis::Solid;
    assert!(!boundary_options(&p, "hole1").contains(&BcKind::Bearing));
}

#[test]
fn surface_kinds_and_centroids() {
    assert_eq!(surface_kind("hole2"), SurfaceKind::Hole);
    assert_eq!(surface_kind("edge7"), SurfaceKind::OuterEdge);
    assert_eq!(surface_kind("start"), SurfaceKind::EndFace);
    assert_eq!(surface_kind("whatever"), SurfaceKind::Other);
    let c = surface_centroids(&plate());
    assert_eq!(c.iter().find(|(n, _)| n == "left").unwrap().1, [0.0, 2.0]);
    assert_eq!(c.iter().find(|(n, _)| n == "hole1").unwrap().1, [6.0, 2.0]);
}
