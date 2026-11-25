//! In-plane free-edge distance of a hole (`solid_kernel::free_edge`): closed-form plates built with `NativeBrep` and read back
//! through the STEP path every real model takes, then the repository's own STEP files.
use solid_kernel::analytic_step::{AnalyticModel, Options};
use solid_kernel::cad::{Loop, Profile};
use solid_kernel::features::{classify_faces, find_holes, find_near_closed, ClassifiedFace, FaceGeometry, Hole};
use solid_kernel::free_edge::{free_edge_distance, EdgeKind, FreeEdge, FreeEdgeError};
use solid_kernel::native_brep::NativeBrep;
use solid_kernel::step::{inventory, Limits};

fn read(b: &NativeBrep) -> AnalyticModel {
    let text = b.to_step_text();
    AnalyticModel::read(&inventory(&text, Limits::default()).unwrap(), Options::default()).unwrap()
}

fn read_text(text: &str) -> AnalyticModel {
    AnalyticModel::read(&inventory(text, Limits::default()).unwrap(), Options::default()).unwrap()
}

/// Every hole of the models with its free-edge reading.
fn readings(models: &[AnalyticModel]) -> Vec<(Hole, Result<FreeEdge, FreeEdgeError>)> {
    let faces = classify_faces(models).unwrap();
    find_holes(models, &faces).into_iter().map(|h| (h.clone(), free_edge_distance(models, &faces, &h))).collect()
}

fn the_reading(b: &NativeBrep) -> Result<FreeEdge, FreeEdgeError> {
    let r = readings(&[read(b)]);
    assert_eq!(r.len(), 1, "one hole expected");
    r.into_iter().next().unwrap().1
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

fn box_with_hole(c: [f64; 2], r: f64, depth: Option<f64>) -> NativeBrep {
    let b = NativeBrep::box_from_corners([0., 0., 0.], [40., 30., 10.]).unwrap();
    let top = b.planar_faces_through([0., 0., 10.], [0., 0., 1.])[0];
    b.drill(top, [c[0], c[1], 10.], r, depth).unwrap()
}

#[test]
fn a_hole_near_one_edge_reads_that_edge_as_a_line_with_the_plate_thickness() {
    let e = the_reading(&box_with_hole([12., 15.], 2., None)).unwrap();
    assert!(near(e.distance, 12.), "{e:?}");
    assert_eq!(e.kind, EdgeKind::Line);
    assert!(near(e.thickness, 10.), "{e:?}");
    // the nearest point is on the x = 0 side, level with the axis, in the plane of the face the distance was read on
    assert!(near(e.point[0], 0.) && near(e.point[1], 15.), "{e:?}");
    assert_ne!(e.face, e.rim);
}

#[test]
fn a_hole_near_a_corner_reads_the_nearer_of_the_two_edges() {
    let e = the_reading(&box_with_hole([6., 5.], 2., None)).unwrap();
    assert!(near(e.distance, 5.), "the y = 0 side, not the x = 0 side at 6: {e:?}");
    assert!(near(e.point[0], 6.) && near(e.point[1], 0.), "{e:?}");
}

#[test]
fn a_blind_hole_reads_the_plate_face_and_skips_its_own_flat_bottom() {
    let e = the_reading(&box_with_hole([12., 15.], 2., Some(4.))).unwrap();
    assert!(near(e.distance, 12.) && near(e.thickness, 10.), "{e:?}");
}

#[test]
fn a_hole_in_a_side_face_reads_in_that_plane_whatever_the_axis_direction() {
    // axis along x through (y, z) = (15, 5): the z = 0 side is 5 away, the y sides 15
    let b = NativeBrep::box_from_corners([0., 0., 0.], [40., 30., 10.]).unwrap();
    let side = b.planar_faces_through([40., 15., 5.], [1., 0., 0.])[0];
    let e = the_reading(&b.drill(side, [40., 15., 5.], 2., None).unwrap()).unwrap();
    assert!(near(e.distance, 5.) && near(e.thickness, 40.), "{e:?}");
}

#[test]
fn a_round_plate_reads_its_circular_edge_exactly() {
    let profile = Profile { outer: Loop::Circle { centre: [0., 0.], radius: 25. }, holes: vec![] };
    let b = NativeBrep::extrude(&profile, 0., 10.).unwrap();
    let top = b.planar_faces_through([0., 0., 10.], [0., 0., 1.])[0];
    let e = the_reading(&b.drill(top, [15., 0., 10.], 2., None).unwrap()).unwrap();
    assert_eq!(e.kind, EdgeKind::Arc { radius: 25. });
    assert!(near(e.distance, 10.), "{e:?}");
    assert!(near(e.point[0], 25.) && near(e.point[1], 0.), "{e:?}");
    // off the line from the centre the exact circle, not its chords, gives the distance
    let e = the_reading(&b.drill(top, [0., 18., 10.], 2., None).unwrap()).unwrap();
    assert!(near(e.distance, 7.), "{e:?}");
}

/// An L bracket: leg A `x` 0..40, `y` 0..4; leg B `x` 0..4, `y` 0..30; 24 wide in z.
fn l_bracket() -> NativeBrep {
    let outline = vec![[0., 0.], [40., 0.], [40., 4.], [4., 4.], [4., 30.], [0., 30.]];
    NativeBrep::extrude(&Profile { outer: Loop::Polygon(outline), holes: vec![] }, 0., 24.).unwrap()
}

#[test]
fn a_hole_in_an_l_bracket_flange_ignores_the_bend_because_the_neighbouring_flange_is_not_free() {
    let b = l_bracket();
    let top = b.planar_faces_through([20., 4., 10.], [0., 1., 0.])[0];
    // 5 from the inner corner (x = 4) where leg B stands, 10 from the z = 0 side, 14 from the z = 24 side, 31 from the end
    let e = the_reading(&b.drill(top, [9., 4., 10.], 3., None).unwrap()).unwrap();
    assert!(near(e.distance, 10.), "the bend at 5 is not a free edge: {e:?}");
    assert_eq!(e.kind, EdgeKind::Line);
    assert!(near(e.thickness, 4.), "{e:?}");
}

#[test]
fn the_same_flange_read_from_the_outer_face_agrees() {
    let b = l_bracket();
    let bottom = b.planar_faces_through([20., 0., 10.], [0., -1., 0.])[0];
    let e = the_reading(&b.drill(bottom, [9., 0., 10.], 3., None).unwrap()).unwrap();
    assert!(near(e.distance, 10.) && near(e.thickness, 4.), "{e:?}");
}

#[test]
fn a_hole_whose_every_edge_joins_material_that_goes_on_has_no_free_edge() {
    // a 12 mm counterbore 3 deep, then a blind 4 mm hole from its floor: the floor's only boundary is the counterbore wall
    let b = NativeBrep::box_from_corners([0., 0., 0.], [40., 30., 10.]).unwrap();
    let top = b.planar_faces_through([0., 0., 10.], [0., 0., 1.])[0];
    let b = b.drill(top, [20., 15., 10.], 6., Some(3.)).unwrap();
    let floor = b.planar_faces_through([20., 15., 7.], [0., 0., 1.])[0];
    let b = b.drill(floor, [20., 15., 7.], 2., Some(4.)).unwrap();
    let models = [read(&b)];
    let r = readings(&models);
    let narrow = r.iter().find(|(h, _)| (h.radius - 2.).abs() < 1e-9).expect("the narrow bore");
    assert_eq!(narrow.1, Err(FreeEdgeError::NoFreeEdge));
    // the counterbore itself is a hole in the top face: its plate edges are free
    let wide = r.iter().find(|(h, _)| (h.radius - 6.).abs() < 1e-9).expect("the counterbore");
    assert!(near(wide.1.as_ref().unwrap().distance, 15.), "{:?}", wide.1);
}

#[test]
fn a_rounded_edge_nearer_than_any_free_edge_is_refused_not_guessed() {
    // every convex edge rounded: the top face meets fillets on all four sides, none is a side face
    let b = NativeBrep::box_from_corners([0., 0., 0.], [40., 30., 10.]).unwrap().fillet_all_convex(1.).unwrap();
    let top = b.planar_faces_through([20., 15., 10.], [0., 0., 1.])[0];
    let r = the_reading(&b.drill(top, [8., 15., 10.], 2., None).unwrap());
    let Err(FreeEdgeError::UnreadBoundary { distance, reason }) = r else { panic!("{r:?}") };
    // the fillet's tangent edge is at x = 1
    assert!(near(distance, 7.), "{distance}");
    assert!(reason.contains("rounded"), "{reason}");
}

#[test]
fn a_bore_covering_less_than_a_turn_is_not_a_full_circle() {
    let model = read(&box_with_hole([20., 15.], 2., None));
    let mut faces = classify_faces(std::slice::from_ref(&model)).unwrap();
    let hole = find_holes(std::slice::from_ref(&model), &faces).remove(0);
    for f in &mut faces {
        if let FaceGeometry::Cylinder { arcs, .. } = &mut f.geometry {
            *arcs = vec![[0.5, 6.0]];
        }
    }
    assert!(find_holes(std::slice::from_ref(&model), &faces).is_empty(), "no longer a hole");
    assert_eq!(free_edge_distance(std::slice::from_ref(&model), &faces, &hole), Err(FreeEdgeError::NotFullCircle));
    let slit = find_near_closed(std::slice::from_ref(&model), &faces, 0.5);
    assert_eq!(free_edge_distance(std::slice::from_ref(&model), &faces, &slit[0].hole), Err(FreeEdgeError::NotFullCircle));
}

#[test]
fn a_hole_with_a_missing_body_is_refused() {
    let model = read(&box_with_hole([20., 15.], 2., None));
    let faces = classify_faces(std::slice::from_ref(&model)).unwrap();
    let mut hole = find_holes(std::slice::from_ref(&model), &faces).remove(0);
    hole.body = 3;
    assert_eq!(free_edge_distance(std::slice::from_ref(&model), &faces, &hole), Err(FreeEdgeError::UnknownBody));
}

fn only_planes(faces: &[ClassifiedFace]) -> usize {
    faces.iter().filter(|f| matches!(f.geometry, FaceGeometry::Plane { .. })).count()
}

#[test]
fn an_edit_plate_step_file_has_its_hole_12_mm_from_the_left_side() {
    // the 40 x 30 x 10 plate with a 4 mm hole at (12, 15) (`solid-kernel/tests/data/README.md`)
    let models = [read_text(include_str!("data/edit-plate.step"))];
    let r = readings(&models);
    assert_eq!(r.len(), 1);
    let e = r[0].1.as_ref().unwrap();
    assert!(near(e.distance, 12.) && near(e.thickness, 10.) && e.kind == EdgeKind::Line, "{e:?}");
    assert!(near(e.point[0], 0.) && near(e.point[1], 15.), "{e:?}");
    assert_eq!(only_planes(&classify_faces(&models).unwrap()), 6);
}

#[test]
fn the_moved_and_twin_hole_plates_follow_the_hole() {
    let moved = readings(&[read_text(include_str!("data/edit-plate-hole-moved.step"))]);
    assert!(near(moved[0].1.as_ref().unwrap().distance, 14.), "{:?}", moved[0].1);
    // holes at x = 15 and 25 of 40: the other hole is not an edge, so both read their own nearer side
    let twin = readings(&[read_text(include_str!("data/edit-plate-twin-holes.step"))]);
    assert_eq!(twin.len(), 2);
    assert!(twin.iter().all(|(_, r)| near(r.as_ref().unwrap().distance, 15.)), "{twin:?}");
}

#[test]
fn the_stacked_plates_assembly_reads_every_hole_in_its_own_body() {
    let text = include_str!("data/assembly-stacked-plates.step");
    let models = AnalyticModel::read_all(&inventory(text, Limits::default()).unwrap(), Options::default()).unwrap();
    let r = readings(&models);
    assert_eq!(r.len(), 2);
    assert_eq!((r[0].0.body, r[1].0.body), (0, 1));
    for (_, e) in &r {
        let e = e.as_ref().unwrap();
        assert!(near(e.distance, 12.) && near(e.thickness, 10.), "{e:?}");
    }
}

#[test]
fn a_bore_through_a_curved_wall_and_a_tilted_bore_are_refused_as_not_planar() {
    // cross-bore.step: bores through the walls of a cylinder
    let models = [read_text(include_str!("data/cross-bore.step"))];
    let r = readings(&models);
    assert!(!r.is_empty());
    for (_, e) in &r {
        assert!(matches!(e, Err(FreeEdgeError::NotPlanar { .. })), "{e:?}");
    }
    // tilted-hole.step: the plate faces are not square to the bore
    let models = [read_text(include_str!("data/tilted-hole.step"))];
    let r = readings(&models);
    assert!(!r.is_empty());
    for (_, e) in &r {
        let Err(FreeEdgeError::NotPlanar { found }) = e else { panic!("{e:?}") };
        assert!(found.contains("not square"), "{found}");
    }
}
