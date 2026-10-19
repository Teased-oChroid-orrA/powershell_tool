//! Native cylinder booleans (hole, boss) on `NativeBrep` against closed-form volumes, through the STEP round trip and the native
//! reader. `docs/issue-46-phase-121.md`.
use solid_kernel::analytic_step::{AnalyticModel, Options};
use solid_kernel::cad::{Loop, Profile};
use solid_kernel::native_brep::NativeBrep;
use solid_kernel::native_surface::{Limits as SurfaceLimits, SampledSurface};
use solid_kernel::step::{inventory, Limits};
use std::f64::consts::PI;

fn sample(b: &NativeBrep, chord: f64) -> (usize, SampledSurface) {
    let text = b.to_step_text();
    let data = inventory(&text, Limits::default()).unwrap_or_else(|e| panic!("inventory: {e}"));
    let model = AnalyticModel::read(&data, Options { chord_mm: chord, ..Options::default() }).unwrap_or_else(|e| panic!("read: {e}"));
    let faces = model.body().faces.len();
    (faces, model.sampled_surface(SurfaceLimits::default(), |_| true).unwrap_or_else(|e| panic!("sample: {e}")))
}

fn within(got: f64, exact: f64, tol: f64, what: &str) {
    assert!((got / exact - 1.).abs() < tol, "{what}: {got} vs {exact}");
}

/// 10 x 10 x 4 block: faces 0..3 sides, 4 bottom (z = 0), 5 top (z = 4).
fn block() -> NativeBrep {
    NativeBrep::box_from_corners([0., 0., 0.], [10., 10., 4.]).unwrap()
}

#[test]
fn a_through_hole_removes_pi_r2_t() {
    let b = block().drill(5, [5., 5., 4.], 1., None).unwrap();
    let (faces, s) = sample(&b, 0.002);
    assert_eq!(faces, 6 + 1);
    within(s.signed_volume_mm3, 400. - PI * 4., 1e-3, "through hole");
    assert!(s.signed_volume_mm3 >= (400. - PI * 4.) * (1. - 1e-9), "an inscribed polygon removes a little less than the circle");
}

#[test]
fn a_blind_hole_removes_pi_r2_d_and_has_a_bottom() {
    let b = block().drill(5, [3., 7., 4.], 0.8, Some(1.5)).unwrap();
    let (faces, s) = sample(&b, 0.002);
    assert_eq!(faces, 6 + 2);
    within(s.signed_volume_mm3, 400. - PI * 0.64 * 1.5, 1e-3, "blind hole");
    // A depth that reaches the far face is the through hole.
    let through = block().drill(5, [3., 7., 4.], 0.8, Some(4.)).unwrap();
    let (faces, s) = sample(&through, 0.002);
    assert_eq!(faces, 7);
    within(s.signed_volume_mm3, 400. - PI * 0.64 * 4., 1e-3, "depth = thickness");
}

#[test]
fn a_boss_adds_pi_r2_h_and_holes_follow_through_the_step_text() {
    let b = block().boss(5, [5., 5., 4.], 2., 3.).unwrap();
    let (faces, s) = sample(&b, 0.002);
    assert_eq!(faces, 6 + 2);
    within(s.signed_volume_mm3, 400. + PI * 4. * 3., 1e-3, "boss");
    // Written, read back as a STEP solid, then drilled through the boss from its top (face 7) and from the bottom.
    let back = NativeBrep::from_step_text(&b.to_step_text()).unwrap();
    assert_eq!(back.faces.len(), 8);
    let drilled = back.drill(7, [5., 5., 7.], 0.5, None).unwrap();
    let (faces, s) = sample(&drilled, 0.002);
    assert_eq!(faces, 9);
    within(s.signed_volume_mm3, 400. + PI * 4. * 3. - PI * 0.25 * 7., 1e-3, "hole through boss and block");
}

#[test]
fn a_hole_through_two_arms_crosses_the_gap() {
    // A C-shaped bracket: arms y in [0,1] and [3,4], thickness 2 along z; drill along +y from the outside of the lower arm.
    let c = Profile { outer: Loop::Polygon(vec![[0., 0.], [6., 0.], [6., 1.], [1., 1.], [1., 3.], [6., 3.], [6., 4.], [0., 4.]]), holes: vec![] };
    let bracket = NativeBrep::extrude(&c, 0., 2.).unwrap();
    let (_, whole) = sample(&bracket, 0.01);
    let exact = (6. * 1. + 1. * 2. + 6. * 1.) * 2.;
    within(whole.signed_volume_mm3, exact, 1e-9, "bracket");
    // The face on y = 0 is the bottom edge of the profile: face 0 (normal -y).
    let drilled = bracket.drill(0, [3.5, 0., 1.], 0.5, None).unwrap();
    let (faces, s) = sample(&drilled, 0.002);
    assert_eq!(faces, 10 + 2);
    within(s.signed_volume_mm3, exact - PI * 0.25 * 2., 1e-3, "two arms, one gap");
    // A blind hole that ends in the gap removes only the first arm's share.
    let blind = bracket.drill(0, [3.5, 0., 1.], 0.5, Some(2.)).unwrap();
    let (_, s) = sample(&blind, 0.002);
    within(s.signed_volume_mm3, exact - PI * 0.25 * 1., 1e-3, "blind into the gap");
    // A blind hole into the second arm has a bottom.
    let into = bracket.drill(0, [3.5, 0., 1.], 0.5, Some(3.5)).unwrap();
    let (faces, s) = sample(&into, 0.002);
    assert_eq!(faces, 10 + 3);
    within(s.signed_volume_mm3, exact - PI * 0.25 * 1.5, 1e-3, "blind into the second arm");
}

#[test]
fn a_plate_with_holes_takes_more_holes_and_a_boss_beside_them() {
    let p = Profile { outer: Loop::Polygon(vec![[0., 0.], [12., 0.], [12., 4.], [0., 4.]]), holes: vec![Loop::Circle { centre: [3., 2.], radius: 0.5 }] };
    let plate = NativeBrep::extrude(&p, 0., 0.25).unwrap();
    let top = plate.faces.len() - 1;
    let two = plate.drill(top, [9., 2., 0.25], 0.5, None).unwrap();
    let three = two.boss(top, [6., 2., 0.25], 0.75, 1.0).unwrap();
    let (faces, s) = sample(&three, 0.001);
    assert_eq!(faces, 4 + 2 + 1 + 1 + 2);
    let exact = (48. - 2. * PI * 0.25) * 0.25 + PI * 0.75 * 0.75 * 1.0;
    within(s.signed_volume_mm3, exact, 1e-3, "plate, two holes, a boss");
}

#[test]
fn unsupported_or_unsafe_requests_are_refused_not_approximated() {
    let b = block();
    // Bigger than the face, hanging over an edge, or touching it.
    assert!(b.drill(5, [5., 5., 4.], 6., None).is_err());
    assert!(b.drill(5, [0.5, 5., 4.], 1., None).is_err());
    assert!(b.drill(5, [1., 5., 4.], 1., None).is_err(), "tangent to the edge");
    assert!(b.boss(5, [9.5, 5., 4.], 1., 1.).is_err());
    // A side face is flat but a curved face is not accepted as a start.
    let cyl = NativeBrep::cylinder_z([0.; 3], 3., 4.).unwrap();
    assert!(cyl.drill(0, [3., 0., 2.], 0.5, None).is_err(), "the wall is curved");
    // A drill that would break through a neighbouring hole's wall.
    let two = block().drill(5, [4., 5., 4.], 1., None).unwrap();
    assert!(two.drill(5, [5.5, 5., 4.], 1., None).is_err(), "overlapping holes");
    assert!(two.drill(5, [7., 5., 4.], 1., None).is_ok(), "clear of the first hole");
    assert!(b.drill(5, [5., 5., 4.], -1., None).is_err() && b.drill(5, [5., 5., 4.], 1., Some(0.)).is_err());
}

/// Fixtures written by OpenCASCADE (p-curves, seam curves, `FACE_BOUND` without an outer marker): read into the editable model,
/// written again, and sampled; the native importer's volume of the original and of the re-written text agree.
#[test]
fn opencascade_files_read_into_the_editable_model_and_round_trip() {
    let files = [
        ("edit-plate", include_str!("data/edit-plate.step")),
        ("edit-plate-twin-holes", include_str!("data/edit-plate-twin-holes.step")),
        ("frustum", include_str!("data/frustum.step")),
        ("cone", include_str!("data/cone.step")),
        ("cylinder-filleted-top", include_str!("data/cylinder-filleted-top.step")),
        ("cylinder-chamfered-top", include_str!("data/cylinder-chamfered-top.step")),
        ("filleted-box", include_str!("data/filleted-box.step")),
        ("torus", include_str!("data/torus.step")),
        ("tilted-cylinder", include_str!("data/tilted-cylinder.step")),
        ("tilted-hole", include_str!("data/tilted-hole.step")),
        ("cross-bore", include_str!("data/cross-bore.step")),
    ];
    let volume = |text: &str| -> Option<(usize, f64)> {
        let data = inventory(text, Limits::default()).ok()?;
        let m = AnalyticModel::read(&data, Options { chord_mm: 0.005, ..Options::default() }).ok()?;
        let n = m.body().faces.len();
        Some((n, m.sampled_surface(SurfaceLimits::default(), |_| true).ok()?.signed_volume_mm3))
    };
    let mut ok = 0;
    for (name, text) in files {
        let Some((faces, v)) = volume(text) else { continue };
        let b = NativeBrep::from_step_text(text).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(b.faces.len(), faces, "{name}");
        let (f2, v2) = volume(&b.to_step_text()).unwrap_or_else(|| panic!("{name}: the re-written text is not importable"));
        assert_eq!(f2, faces, "{name}");
        assert!((v2 / v - 1.).abs() < 1e-9, "{name}: {v2} vs {v}");
        ok += 1;
    }
    assert!(ok >= 8, "only {ok} fixtures round-tripped");
}

#[test]
fn a_hole_drilled_into_an_opencascade_plate() {
    // 40 x 30 x 10 plate with a 4 mm hole at (12, 15) (`tests/data/README.md`). Find its top face (flat, normal +z, at z = 10).
    let plate = NativeBrep::from_step_text(include_str!("data/edit-plate.step")).unwrap();
    let top = (0..plate.faces.len())
        .find(|&i| plate.drill(i, [30., 15., 10.], 1.5, None).is_ok())
        .expect("a face accepts a hole at (30, 15)");
    let drilled = plate.drill(top, [30., 15., 10.], 1.5, None).unwrap();
    let (_, s) = sample(&drilled, 0.002);
    let exact = 40. * 30. * 10. - PI * 16. * 10. - PI * 2.25 * 10.;
    within(s.signed_volume_mm3, exact, 1e-3, "plate with two holes");
    // The same hole from the other side lands in the same solid.
    let bottom = (0..plate.faces.len()).find(|&i| i != top && plate.drill(i, [30., 15., 0.], 1.5, None).is_ok()).unwrap();
    let (_, s2) = sample(&plate.drill(bottom, [30., 15., 0.], 1.5, None).unwrap(), 0.002);
    within(s2.signed_volume_mm3, exact, 1e-3, "from below");
}
