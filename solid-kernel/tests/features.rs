//! Assembly feature detection (epic Phase A): closed-form fixtures built with `NativeBrep` (plates, drilled holes), read back
//! through the STEP path every real model takes, so the detector sees `AnalyticModel`s exactly as production does.
use solid_kernel::analytic_step::{AnalyticModel, Options};
use solid_kernel::features::{classify_faces, ClassifiedFace, FaceGeometry, Side};
use solid_kernel::native_brep::NativeBrep;
use solid_kernel::step::{inventory, Limits};
use solid_kernel::V3;

/// Read a solid back as an `AnalyticModel`.
fn read(b: &NativeBrep) -> AnalyticModel {
    let text = b.to_step_text();
    let data = inventory(&text, Limits::default()).unwrap();
    AnalyticModel::read(&data, Options::default()).unwrap()
}

/// A plate `[lo, hi]` with a round hole drilled down through its top face at `(x, y)`.
fn plate_with_holes(lo: V3, hi: V3, holes: &[([f64; 2], f64)]) -> NativeBrep {
    let mut b = NativeBrep::box_from_corners(lo, hi).unwrap();
    for (c, r) in holes {
        let top = b.planar_faces_through([lo[0], lo[1], hi[2]], [0., 0., 1.])[0];
        b = b.drill(top, [c[0], c[1], hi[2]], *r, None).unwrap();
    }
    b
}

fn planes(faces: &[ClassifiedFace]) -> Vec<&ClassifiedFace> {
    faces.iter().filter(|f| matches!(f.geometry, FaceGeometry::Plane { .. })).collect()
}

#[test]
fn a_drilled_plate_classifies_six_planes_and_one_concave_cylinder_with_closed_form_numbers() {
    let plate = plate_with_holes([0., 0., 0.], [40., 30., 10.], &[([20., 15.], 2.)]);
    let model = read(&plate);
    let chord = model.chord_tolerance_mm();
    let faces = classify_faces(&[model]).unwrap();
    assert_eq!(faces.len(), 7);
    assert_eq!(planes(&faces).len(), 6);
    // areas: the two big faces lose the hole's disk; the four sides are plain rectangles
    let mut areas: Vec<f64> = planes(&faces)
        .iter()
        .map(|f| match &f.geometry {
            FaceGeometry::Plane { area, .. } => *area,
            _ => unreachable!(),
        })
        .collect();
    areas.sort_by(f64::total_cmp);
    let disk = std::f64::consts::PI * 4.;
    let expected = [300., 300., 400., 400., 1200. - disk, 1200. - disk];
    // The area is the sampled polygon's: a curved boundary reads inside by at most its perimeter times the chord tolerance.
    let bound = 2. * std::f64::consts::PI * 2. * chord;
    for (a, e) in areas.iter().zip(expected) {
        assert!((a - e).abs() <= bound + 1e-9, "{a} vs {e} (bound {bound})");
    }
    // and only the two faces with the hole are affected: the plain rectangles are exact
    assert!(areas[..4].iter().zip(&expected[..4]).all(|(a, e)| (a - e).abs() < 1e-9));
    let cylinder = faces.iter().find(|f| matches!(f.geometry, FaceGeometry::Cylinder { .. })).unwrap();
    let FaceGeometry::Cylinder { axis_point, axis, radius, side, range, .. } = &cylinder.geometry else { unreachable!() };
    // a drilled bore is one full ring
    assert!((cylinder.geometry.wrap().unwrap() - std::f64::consts::TAU).abs() < 1e-9);
    assert_eq!(*side, Side::Concave);
    assert!((radius - 2.).abs() < 1e-9);
    assert!(axis[2].abs() > 1. - 1e-12);
    // the axis passes through (20, 15); the face runs the 10 mm of the plate
    assert!((axis_point[0] - 20.).abs() < 1e-9 && (axis_point[1] - 15.).abs() < 1e-9);
    assert!((range[1] - range[0] - 10.).abs() < 1e-9, "{range:?}");
}

#[test]
fn a_face_key_is_the_body_and_the_face_so_equal_ids_in_two_bodies_stay_distinct() {
    let a = read(&plate_with_holes([0., 0., 0.], [40., 30., 10.], &[([20., 15.], 2.)]));
    let b = read(&plate_with_holes([0., 0., 10.], [40., 30., 20.], &[([20., 15.], 2.)]));
    let faces = classify_faces(&[a, b]).unwrap();
    assert_eq!(faces.len(), 14);
    let mut keys: Vec<_> = faces.iter().map(|f| f.key).collect();
    keys.sort();
    keys.dedup();
    assert_eq!(keys.len(), 14);
    // same construction: the same face ids repeat in the second body
    assert!(faces[..7].iter().zip(&faces[7..]).all(|(x, y)| x.key.face == y.key.face && x.key.body == 0 && y.key.body == 1));
}

// ---------------------------------------------------------------------------------------------------------------------------
// Holes, fastener sites, interfaces
// ---------------------------------------------------------------------------------------------------------------------------
use solid_kernel::features::{detect, find_holes, Closure, FitClass, HoleEnd};
use std::f64::consts::PI;

fn detected(plates: &[NativeBrep]) -> (Vec<AnalyticModel>, solid_kernel::features::Features) {
    let models: Vec<AnalyticModel> = plates.iter().map(read).collect();
    let f = detect(&models).unwrap();
    (models, f)
}

/// Two 40 x 30 x 10 plates stacked in z, each with the given holes (the same positions and radii in both).
fn stack(holes: &[([f64; 2], f64)]) -> [NativeBrep; 2] {
    [plate_with_holes([0., 0., 0.], [40., 30., 10.], holes), plate_with_holes([0., 0., 10.], [40., 30., 20.], holes)]
}

#[test]
fn a_through_hole_is_one_hole_with_diameter_depth_and_two_open_ends() {
    let (_, f) = detected(&[plate_with_holes([0., 0., 0.], [40., 30., 10.], &[([20., 15.], 2.5)])]);
    assert_eq!(f.holes.len(), 1);
    let h = &f.holes[0];
    assert!((h.diameter() - 5.).abs() < 1e-9 && (h.depth() - 10.).abs() < 1e-9, "{h:?}");
    assert_eq!((h.closure(), h.ends), (Closure::Through, [HoleEnd::Open; 2]));
    assert!(f.sites.is_empty() && f.interfaces.is_empty());
}

#[test]
fn a_blind_hole_has_a_flat_bottom_and_a_counterbore_is_a_second_section_on_the_same_axis() {
    // blind: 4 mm deep from the top of a 10 mm plate
    let mut b = NativeBrep::box_from_corners([0., 0., 0.], [40., 30., 10.]).unwrap();
    let top = b.planar_faces_through([0., 0., 10.], [0., 0., 1.])[0];
    b = b.drill(top, [20., 15., 10.], 2., Some(4.)).unwrap();
    let (_, f) = detected(&[b.clone()]);
    assert_eq!(f.holes.len(), 1);
    let h = &f.holes[0];
    assert!((h.depth() - 4.).abs() < 1e-9);
    assert_eq!(h.closure(), Closure::Blind);
    assert!(h.ends.contains(&HoleEnd::FlatBottom) && h.ends.contains(&HoleEnd::Open), "{:?}", h.ends);
    // counterbore: a 3 mm radius pocket 3 mm deep, then a 1.5 mm radius hole through the rest, from the pocket's floor
    let mut c = NativeBrep::box_from_corners([0., 0., 0.], [40., 30., 10.]).unwrap();
    let top = c.planar_faces_through([0., 0., 10.], [0., 0., 1.])[0];
    c = c.drill(top, [20., 15., 10.], 3., Some(3.)).unwrap();
    let floor = c.planar_faces_through([20., 15., 7.], [0., 0., 1.])[0];
    c = c.drill(floor, [20., 15., 7.], 1.5, None).unwrap();
    let (_, f) = detected(&[c]);
    assert_eq!(f.holes.len(), 2, "{:?}", f.holes);
    let wide = f.holes.iter().find(|h| (h.radius - 3.).abs() < 1e-9).unwrap();
    let narrow = f.holes.iter().find(|h| (h.radius - 1.5).abs() < 1e-9).unwrap();
    assert!(narrow.ends.iter().any(|e| matches!(e, HoleEnd::Counterbore { radius } if (radius - 3.).abs() < 1e-9)), "{:?}", narrow.ends);
    assert!(wide.ends.iter().any(|e| matches!(e, HoleEnd::Narrows { radius } if (radius - 1.5).abs() < 1e-9)), "{:?}", wide.ends);
}

#[test]
fn a_bore_split_into_two_half_faces_is_one_hole() {
    let model = read(&plate_with_holes([0., 0., 0.], [40., 30., 10.], &[([20., 15.], 2.)]));
    let mut faces = classify_faces(std::slice::from_ref(&model)).unwrap();
    let i = faces.iter().position(|f| matches!(f.geometry, FaceGeometry::Cylinder { .. })).unwrap();
    // split the one cylinder face into a lower and an upper half along the axis, with a new id for the upper one
    let mut upper = faces[i].clone();
    let FaceGeometry::Cylinder { range, .. } = &mut faces[i].geometry else { unreachable!() };
    let mid = 0.5 * (range[0] + range[1]);
    let top = range[1];
    range[1] = mid;
    let FaceGeometry::Cylinder { range, .. } = &mut upper.geometry else { unreachable!() };
    *range = [mid, top];
    upper.key.face += 100_000;
    faces.push(upper);
    let holes = find_holes(std::slice::from_ref(&model), &faces);
    assert_eq!(holes.len(), 1);
    assert_eq!(holes[0].faces.len(), 2);
    assert!((holes[0].depth() - 10.).abs() < 1e-9);
    // a gap between the halves larger than the tolerance is two bores, not one
    let FaceGeometry::Cylinder { range, .. } = &mut faces.last_mut().unwrap().geometry else { unreachable!() };
    range[0] += 1.0;
    assert_eq!(find_holes(std::slice::from_ref(&model), &faces).len(), 2);
}

#[test]
fn stacked_plates_with_one_two_and_four_coaxial_holes_give_that_many_sites_and_one_interface() {
    for holes in [
        vec![([20., 15.], 2.5)],
        vec![([10., 15.], 2.5), ([30., 15.], 2.5)],
        vec![([10., 8.], 2.5), ([30., 8.], 2.5), ([10., 22.], 2.5), ([30., 22.], 2.5)],
    ] {
        let (_, f) = detected(&stack(&holes));
        assert_eq!(f.holes.len(), 2 * holes.len());
        assert_eq!(f.sites.len(), holes.len(), "{:?}", f.sites);
        for s in &f.sites {
            assert_eq!(s.bodies, vec![0, 1]);
            assert!((s.span[1] - s.span[0] - 20.).abs() < 1e-9, "{:?}", s.span);
            assert!(!s.diameter_mismatch);
            assert_eq!(s.fastener, Some("M5"));
            assert!(s.bores.iter().all(|b| b.matches.iter().any(|m| m.size == "M5" && m.fit == FitClass::Nominal)));
        }
        // one contact: the top of plate 0 on the bottom of plate 1; the common area is the plate face less the holes
        assert_eq!(f.interfaces.len(), 1, "{:?}", f.interfaces);
        let i = &f.interfaces[0];
        assert!(i.gap.abs() < 1e-9 && i.normal[2] > 1. - 1e-12);
        let FaceGeometry::Plane { area, .. } = &f.faces.iter().find(|x| x.key == i.a).unwrap().geometry else { unreachable!() };
        assert!((i.overlap_area - area).abs() < 1e-6, "{} vs {area}", i.overlap_area);
        assert!(i.overlap_area < 1200. && i.overlap_area > 1200. - holes.len() as f64 * PI * 6.25 - 1.0);
        assert!((i.overlap_centroid[2] - 10.).abs() < 1e-9);
        assert!(f.unresolved.is_empty());
    }
}

#[test]
fn holes_offset_by_the_tolerance_still_pair_and_past_it_do_not() {
    let tol = read(&plate_with_holes([0., 0., 0.], [40., 30., 10.], &[])).geometry_tolerance_mm();
    let pair = |dx: f64| {
        let a = plate_with_holes([0., 0., 0.], [40., 30., 10.], &[([20., 15.], 2.5)]);
        let b = plate_with_holes([0., 0., 10.], [40., 30., 20.], &[([20. + dx, 15.], 2.5)]);
        detected(&[a, b]).1.sites.len()
    };
    assert_eq!(pair(0.5 * tol), 1);
    assert_eq!(pair(2. * tol), 0);
}

#[test]
fn plates_with_a_gap_or_side_by_side_share_no_interface_and_no_site() {
    let hole = [([20., 15.], 2.5)];
    // a 0.5 mm gap: far more than the tolerance
    let gapped = [plate_with_holes([0., 0., 0.], [40., 30., 10.], &hole), plate_with_holes([0., 0., 10.5], [40., 30., 20.5], &hole)];
    let f = detected(&gapped).1;
    assert!(f.interfaces.is_empty() && f.sites.is_empty(), "{f:?}");
    // the same plane but no shared area: side by side, touching along an edge only
    let apart = [
        NativeBrep::box_from_corners([0., 0., 0.], [40., 30., 10.]).unwrap(),
        NativeBrep::box_from_corners([40., 0., 0.], [80., 30., 10.]).unwrap(),
    ];
    let f = detected(&apart).1;
    // their x = 40 faces face each other with the whole 30 x 10 face in common; the tops are coplanar but same-facing
    assert_eq!(f.interfaces.len(), 1, "{:?}", f.interfaces);
    assert!((f.interfaces[0].overlap_area - 300.).abs() < 1e-9);
}

#[test]
fn partly_overlapping_plates_report_the_exact_common_area_and_its_centroid() {
    let a = NativeBrep::box_from_corners([0., 0., 0.], [40., 30., 10.]).unwrap();
    let b = NativeBrep::box_from_corners([25., 5., 10.], [65., 35., 20.]).unwrap();
    let f = detected(&[a, b]).1;
    assert_eq!(f.interfaces.len(), 1);
    let i = &f.interfaces[0];
    // x 25..40 and y 5..30
    assert!((i.overlap_area - 15. * 25.).abs() < 1e-9, "{}", i.overlap_area);
    assert!((i.overlap_centroid[0] - 32.5).abs() < 1e-9 && (i.overlap_centroid[1] - 17.5).abs() < 1e-9 && (i.overlap_centroid[2] - 10.).abs() < 1e-9);
}

#[test]
fn a_hole_in_one_plate_only_does_not_make_a_site_and_unequal_bores_are_reported_not_hidden() {
    let a = plate_with_holes([0., 0., 0.], [40., 30., 10.], &[([20., 15.], 2.5)]);
    let blank = NativeBrep::box_from_corners([0., 0., 10.], [40., 30., 20.]).unwrap();
    let f = detected(&[a.clone(), blank]).1;
    assert!(f.sites.is_empty());
    assert_eq!(f.interfaces.len(), 1);
    // 5.0 mm over 6.6 mm is an M6 joint: the M6 tap drill in one plate (5.0 is also the M5 nominal), the M6 normal clearance
    // hole in the other
    let b = plate_with_holes([0., 0., 10.], [40., 30., 20.], &[([20., 15.], 3.3)]);
    let f = detected(&[a.clone(), b]).1;
    assert_eq!(f.sites.len(), 1);
    assert!(f.sites[0].diameter_mismatch);
    assert_eq!(f.sites[0].fastener, Some("M6"));
    let fits: Vec<_> = f.sites[0].bores.iter().map(|b| b.matches.iter().filter(|m| m.size == "M6").map(|m| m.fit).collect::<Vec<_>>()).collect();
    assert_eq!(fits, vec![vec![FitClass::Tap], vec![FitClass::Normal]]);
    // 5.0 mm over 7.8 mm matches no one size: reported as a mismatch with no fastener
    let b = plate_with_holes([0., 0., 10.], [40., 30., 20.], &[([20., 15.], 3.9)]);
    let f = detected(&[a.clone(), b]).1;
    assert_eq!((f.sites.len(), f.sites[0].diameter_mismatch, f.sites[0].fastener), (1, true, None));
    // 0.201 in (the 1/4-20 tap drill, also the #10-24 free clearance) over 0.250 in: 1/4-20, tapped in one part and a
    // nominal bore in the other
    let (r1, r2) = (0.201 * 25.4 / 2., 0.250 * 25.4 / 2.);
    let a = plate_with_holes([0., 0., 0.], [40., 30., 10.], &[([20., 15.], r1)]);
    let b = plate_with_holes([0., 0., 10.], [40., 30., 20.], &[([20., 15.], r2)]);
    let f = detected(&[a, b]).1;
    assert_eq!(f.sites.len(), 1);
    assert!(f.sites[0].diameter_mismatch);
    assert_eq!(f.sites[0].fastener, Some("1/4-20"));
    let fits: Vec<_> = f.sites[0].bores.iter().map(|b| b.matches.iter().filter(|m| m.size == "1/4-20").map(|m| m.fit).collect::<Vec<_>>()).collect();
    assert_eq!(fits, vec![vec![FitClass::Tap], vec![FitClass::Nominal]]);
}

#[test]
fn a_hole_along_x_pairs_like_one_along_z() {
    // two 10 mm slabs stacked in x, each drilled from its +x face along -x
    let drill_x = |x0: f64| {
        let mut b = NativeBrep::box_from_corners([x0, 0., 0.], [x0 + 10., 40., 30.]).unwrap();
        let face = b.planar_faces_through([x0 + 10., 0., 0.], [1., 0., 0.])[0];
        b = b.drill(face, [x0 + 10., 20., 15.], 2.5, None).unwrap();
        b
    };
    let f = detected(&[drill_x(0.), drill_x(10.)]).1;
    assert_eq!(f.sites.len(), 1, "{:?}", f.holes);
    assert!(f.sites[0].axis[0].abs() > 1. - 1e-12);
    assert!((f.sites[0].span[1] - f.sites[0].span[0] - 20.).abs() < 1e-9);
    assert_eq!(f.interfaces.len(), 1);
    assert!(f.interfaces[0].normal[0].abs() > 1. - 1e-12);
}

#[test]
fn a_non_convex_face_and_holes_inside_holes_give_the_exact_common_area() {
    use solid_kernel::cad::{Loop, Profile};
    // an L section extruded 20 mm (area 40 x 10 + 10 x 20 = 600), a block resting on its top
    let l = Profile { outer: Loop::Polygon(vec![[0., 0.], [40., 0.], [40., 10.], [10., 10.], [10., 30.], [0., 30.]]), holes: vec![] };
    let angle = NativeBrep::extrude(&l, 0., 20.).unwrap();
    let block = NativeBrep::box_from_corners([0., 0., 20.], [25., 25., 30.]).unwrap();
    let f = detected(&[angle, block]).1;
    assert_eq!(f.interfaces.len(), 1, "{:?}", f.interfaces);
    let i = &f.interfaces[0];
    // [0, 25] x [0, 10] and [0, 10] x [10, 25]
    assert!((i.overlap_area - 400.).abs() < 1e-9, "{}", i.overlap_area);
    assert!((i.overlap_centroid[0] - 9.6875).abs() < 1e-9 && (i.overlap_centroid[1] - 9.6875).abs() < 1e-9, "{:?}", i.overlap_centroid);
    // a 5 mm hole in the lower plate and a 3 mm hole in the upper one on the same axis: the upper face's hole lies inside the
    // lower's, so the common area is the lower face's own area
    let a = plate_with_holes([0., 0., 0.], [40., 30., 10.], &[([20., 15.], 5.)]);
    let b = plate_with_holes([0., 0., 10.], [40., 30., 20.], &[([20., 15.], 3.)]);
    let f = detected(&[a, b]).1;
    assert_eq!(f.interfaces.len(), 1);
    let FaceGeometry::Plane { area, .. } = &f.faces.iter().find(|x| x.key == f.interfaces[0].a).unwrap().geometry else { unreachable!() };
    assert!((f.interfaces[0].overlap_area - area).abs() < 1e-6, "{} vs {area}", f.interfaces[0].overlap_area);
    // two concentric bores of different radius are one site, with a mismatch
    assert_eq!(f.sites.len(), 1);
    assert!(f.sites[0].diameter_mismatch);
}

#[test]
fn placed_instances_of_a_step_assembly_classify_in_the_root_frame_and_do_not_touch() {
    let data = inventory(include_str!("data/assembly-two-instances.step"), Limits::default()).unwrap();
    let models = AnalyticModel::read_all(&data, Options::default()).unwrap();
    let f = detect(&models).unwrap();
    // two 2 x 1 x 3 boxes, one rotated into x -6..-4 and one into x 9..10: six planes each, nothing shared
    assert_eq!(f.faces.len(), 12);
    assert!(f.faces.iter().all(|x| matches!(x.geometry, FaceGeometry::Plane { .. })));
    assert!(f.holes.is_empty() && f.sites.is_empty() && f.interfaces.is_empty());
    // one solid placed twice: the same face ids in both bodies, told apart by the body index
    let ids = |b: usize| f.faces.iter().filter(|x| x.key.body == b).map(|x| x.key.face).collect::<Vec<_>>();
    assert_eq!(ids(0), ids(1));
    // the faces carry the placement: every normal is a rotated axis, not the source frame's
    for x in &f.faces {
        let FaceGeometry::Plane { normal, .. } = &x.geometry else { unreachable!() };
        assert!(normal.iter().filter(|c| c.abs() > 1. - 1e-9).count() == 1, "{normal:?}");
    }
}

#[test]
fn a_bore_must_cover_the_full_circle_so_halves_make_a_hole_and_a_lone_half_or_a_fillet_does_not() {
    let model = read(&plate_with_holes([0., 0., 0.], [40., 30., 10.], &[([20., 15.], 2.)]));
    let tol = model.geometry_tolerance_mm();
    let faces = classify_faces(std::slice::from_ref(&model)).unwrap();
    let i = faces.iter().position(|f| matches!(f.geometry, FaceGeometry::Cylinder { .. })).unwrap();
    // replace the bore's one face by faces covering the given angular intervals, one face per list
    let with_arcs = |lists: &[Vec<[f64; 2]>]| {
        let mut out = faces.clone();
        let template = out[i].clone();
        out.remove(i);
        for (k, l) in lists.iter().enumerate() {
            let mut f = template.clone();
            f.key.face += 100_000 * (k as u64 + 1);
            let FaceGeometry::Cylinder { arcs, .. } = &mut f.geometry else { unreachable!() };
            *arcs = l.clone();
            out.push(f);
        }
        find_holes(std::slice::from_ref(&model), &out)
    };
    let pi = std::f64::consts::PI;
    // two halves side by side in angle: one hole of two faces; the same half twice (stacked faces over one another): not
    let h = with_arcs(&[vec![[0., pi]], vec![[pi, 2. * pi]]]);
    assert_eq!((h.len(), h[0].faces.len()), (1, 2));
    assert!(with_arcs(&[vec![[0., pi]], vec![[0., pi]]]).is_empty());
    // one half face alone, or a quarter turn (an inside fillet): not a hole
    assert!(with_arcs(&[vec![[0., pi]]]).is_empty());
    assert!(with_arcs(&[vec![[0.3, 0.3 + 0.5 * pi]]]).is_empty());
    // a ring short of the full circle by more than the tolerance subtends at the radius is not a hole; by less it still is
    assert!(with_arcs(&[vec![[0., 2. * pi - 10. * tol / 2.]]]).is_empty());
    assert_eq!(with_arcs(&[vec![[0., 2. * pi - 0.5 * tol / 2.]]]).len(), 1);
}

/// `edit-plate-centre-hole.step` with its bore's `CYLINDRICAL_SURFACE` replaced by the same cylinder as a closed rational quadratic
/// B-spline surface (nine control points round, two rows, the weights of an exact circle) and the bore's edges stripped of their
/// pcurves: the way Open CASCADE writes AS1's cylinders (there as two cubic halves; `curved_surface`'s unit tests cover that form).
fn plate_with_a_spline_bore() -> String {
    let text = include_str!("data/edit-plate-centre-hole.step");
    let (r, cx, cy, z0, z1) = (4.0_f64, 20.0_f64, 15.0_f64, -1.0_f64, 10.0_f64);
    let w = std::f64::consts::FRAC_1_SQRT_2;
    let ring = [(r, 0.), (r, r), (0., r), (-r, r), (-r, 0.), (-r, -r), (0., -r), (r, -r), (r, 0.)];
    let weights = [1., w, 1., w, 1., w, 1., w, 1.];
    let mut points = String::new();
    let mut columns = Vec::new();
    for (i, (x, y)) in ring.iter().enumerate() {
        let (a, b) = (900 + 2 * i, 901 + 2 * i);
        points += &format!("#{a} = CARTESIAN_POINT('',({:.12},{:.12},{z0:.12}));\n#{b} = CARTESIAN_POINT('',({:.12},{:.12},{z1:.12}));\n", cx + x, cy + y, cx + x, cy + y);
        columns.push(format!("(#{a},#{b})"));
    }
    let rows: Vec<String> = weights.iter().map(|w| format!("({w:.14},{w:.14})")).collect();
    let spline = format!(
        "#282 = ( BOUNDED_SURFACE() B_SPLINE_SURFACE(2,1,({}),.UNSPECIFIED.,.F.,.F.,.F.) B_SPLINE_SURFACE_WITH_KNOTS((3,2,2,2,3),(2,2),(0.,0.25,0.5,0.75,1.),(0.,1.),.UNSPECIFIED.) GEOMETRIC_REPRESENTATION_ITEM() RATIONAL_B_SPLINE_SURFACE(({})) REPRESENTATION_ITEM('') SURFACE() );\n{points}",
        columns.join(","),
        rows.join(",")
    );
    let old = "#282 = CYLINDRICAL_SURFACE('',#283,4.);";
    assert!(text.contains(old));
    let mut out = text.replace(old, &spline);
    // an edge curve that is a SURFACE_CURVE becomes its 3D curve: the pcurves are in the analytic cylinder's (angle, height)
    let wrapped: Vec<(String, String)> = out
        .lines()
        .filter_map(|l| {
            let (id, rest) = l.split_once(" = SURFACE_CURVE('',")?;
            Some((id.to_string(), rest.split(',').next()?.to_string()))
        })
        .collect();
    for (id, curve) in wrapped {
        out = out.lines().map(|l| if l.contains("EDGE_CURVE(") { l.replace(&format!(",{id},"), &format!(",{curve},")) } else { l.to_string() }).collect::<Vec<_>>().join("\n") + "\n";
    }
    out
}

#[test]
fn a_bore_written_as_a_rational_spline_cylinder_is_the_same_hole_as_the_analytic_one() {
    let analytic = {
        let data = inventory(include_str!("data/edit-plate-centre-hole.step"), Limits::default()).unwrap();
        AnalyticModel::read(&data, Options::default()).unwrap()
    };
    let spline = {
        let text = plate_with_a_spline_bore();
        let data = inventory(&text, Limits::default()).unwrap();
        AnalyticModel::read(&data, Options::default()).unwrap_or_else(|e| panic!("the spline bore did not read: {}", e.detail))
    };
    let cyl = |m: &AnalyticModel| {
        let faces = classify_faces(std::slice::from_ref(m)).unwrap();
        faces.into_iter().find(|f| matches!(f.geometry, FaceGeometry::Cylinder { .. })).expect("a cylinder face")
    };
    let (a, s) = (cyl(&analytic), cyl(&spline));
    let (FaceGeometry::Cylinder { axis_point: pa, axis: xa, radius: ra, side: sa, range: ga, .. }, FaceGeometry::Cylinder { axis_point: ps, axis: xs, radius: rs, side: ss, range: gs, .. }) = (&a.geometry, &s.geometry) else { unreachable!() };
    assert_eq!(sa, ss, "concave bore stays concave");
    assert!((ra - rs).abs() < 1e-6 && (0..3).all(|k| (pa[k] - ps[k]).abs() < 1e-6 && (xa[k] - xs[k]).abs() < 1e-6), "{:?} vs {:?}", a.geometry, s.geometry);
    assert!((ga[0] - gs[0]).abs() < 1e-6 && (ga[1] - gs[1]).abs() < 1e-6);
    assert!((s.geometry.wrap().unwrap() - std::f64::consts::TAU).abs() < 1e-6, "a full ring: {:?}", s.geometry);
    let holes = detect(&[spline]).unwrap().holes;
    assert_eq!(holes.len(), 1);
    assert_eq!(holes[0].closure(), Closure::Through);
}
