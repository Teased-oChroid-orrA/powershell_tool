//! Native modelling (no OpenCASCADE): the shapes `NativeBrep` builds are written as STEP, read back by the native importer, and
//! compared with closed-form volumes and face counts. `docs/issue-46-phase-120.md`.
use solid_kernel::analytic_step::{AnalyticModel, Options};
use solid_kernel::cad::{Loop, Profile};
use solid_kernel::native_brep::{extrude_volume, lathe_volume, NativeBrep};
use solid_kernel::native_surface::{Limits as SurfaceLimits, SampledSurface};
use solid_kernel::step::{inventory, Limits};
use std::f64::consts::PI;

/// Write, read back with the native importer, sample at `chord` mm: (face count, sampled surface).
fn round_trip(b: &NativeBrep, chord: f64) -> (usize, SampledSurface) {
    let text = b.to_step_text();
    let data = inventory(&text, Limits::default()).unwrap_or_else(|e| panic!("inventory: {e}\n{text}"));
    let model = AnalyticModel::read(&data, Options { chord_mm: chord, ..Options::default() }).unwrap_or_else(|e| panic!("read: {e}"));
    let faces = model.body().faces.len();
    let s = model.sampled_surface(SurfaceLimits::default(), |_| true).unwrap_or_else(|e| panic!("sample: {e}"));
    (faces, s)
}

fn poly(p: &[[f64; 2]]) -> Loop {
    Loop::Polygon(p.to_vec())
}

#[test]
fn an_extruded_l_profile_round_trips_with_the_exact_volume() {
    let p = Profile { outer: poly(&[[0., 0.], [4., 0.], [4., 1.], [1., 1.], [1., 3.], [0., 3.]]), holes: vec![] };
    let b = NativeBrep::extrude(&p, 0., 2.).unwrap();
    let (faces, s) = round_trip(&b, 0.01);
    assert_eq!(faces, 6 + 2);
    assert!((extrude_volume(&p, 2.) - 12.).abs() < 1e-12);
    assert!((s.signed_volume_mm3 - 12.).abs() < 1e-9, "{}", s.signed_volume_mm3);
}

#[test]
fn clockwise_input_and_downward_depth_give_the_same_solid() {
    let ccw = Profile { outer: poly(&[[0., 0.], [3., 0.], [3., 2.], [0., 2.]]), holes: vec![] };
    let cw = Profile { outer: poly(&[[0., 2.], [3., 2.], [3., 0.], [0., 0.]]), holes: vec![] };
    for (p, z0, d) in [(&ccw, 1., 4.), (&cw, 1., 4.), (&cw, 5., -4.)] {
        let (_, s) = round_trip(&NativeBrep::extrude(p, z0, d).unwrap(), 0.01);
        assert!((s.signed_volume_mm3 - 24.).abs() < 1e-9, "{}", s.signed_volume_mm3);
    }
    let b = NativeBrep::extrude(&cw, 5., -4.).unwrap();
    assert!(b.vertices.iter().all(|v| v[2] == 1. || v[2] == 5.));
}

#[test]
fn a_plate_with_a_polygon_hole_and_a_circle_hole() {
    let p = Profile {
        outer: poly(&[[0., 0.], [12., 0.], [12., 4.], [0., 4.]]),
        holes: vec![poly(&[[1., 1.], [3., 1.], [3., 3.], [1., 3.]]), Loop::Circle { centre: [8., 2.], radius: 1. }],
    };
    let b = NativeBrep::extrude(&p, 0., 0.25).unwrap();
    let (faces, s) = round_trip(&b, 0.001);
    assert_eq!(faces, 4 + 1 + 4 + 2);
    let exact = extrude_volume(&p, 0.25);
    assert!((exact - (48. - 4. - PI) * 0.25).abs() < 1e-12);
    // A sampled circle is inscribed: the hole is a little too big, the volume a little too large, by under the chord tolerance.
    assert!((s.signed_volume_mm3 / exact - 1.).abs() < 1e-3, "{} vs {exact}", s.signed_volume_mm3);
}

#[test]
fn a_cylinder_and_a_box_match_closed_form() {
    let (faces, s) = round_trip(&NativeBrep::cylinder_z([1., 2., 3.], 2., 3.).unwrap(), 0.002);
    assert_eq!(faces, 3);
    let exact = PI * 4. * 3.;
    assert!(s.signed_volume_mm3 <= exact * (1. + 1e-12) && (s.signed_volume_mm3 / exact - 1.).abs() < 1e-3, "{}", s.signed_volume_mm3);
    let (faces, s) = round_trip(&NativeBrep::box_from_corners([0., 0., 0.], [2., 3., 4.]).unwrap(), 0.01);
    assert_eq!((faces, (s.signed_volume_mm3 - 24.).abs() < 1e-12), (6, true));
}

#[test]
fn lathe_cylinder_frustum_and_ring_match_closed_form() {
    // Solid cylinder R2 x H3 as a profile on the axis.
    let cyl = [[0., 0.], [2., 0.], [2., 3.], [0., 3.]];
    // Frustum R6 -> R3 over 8.
    let fru = [[0., 0.], [6., 0.], [3., 8.], [0., 8.]];
    // A ring (washer with a conical outer wall): 2 <= r, outer 5 -> 4 over height 2.
    let ring = [[2., 0.], [5., 0.], [4., 2.], [2., 2.]];
    for (name, pts, faces, exact) in [
        ("cylinder", &cyl[..], 3, PI * 4. * 3.),
        ("frustum", &fru[..], 3, PI * 8. * (36. + 18. + 9.) / 3.),
        ("ring", &ring[..], 4, PI * 2. * (25. + 20. + 16.) / 3. - PI * 4. * 2.),
    ] {
        assert!((lathe_volume(pts) - exact).abs() < 1e-9 * exact, "{name}: closed form {} vs {exact}", lathe_volume(pts));
        // Orientation of the input does not matter.
        let mut rev = pts.to_vec();
        rev.reverse();
        for p in [pts.to_vec(), rev] {
            let (f, s) = round_trip(&NativeBrep::lathe(&p).unwrap(), 0.002);
            assert_eq!(f, faces, "{name}");
            assert!((s.signed_volume_mm3 / exact - 1.).abs() < 2e-3, "{name}: {} vs {exact}", s.signed_volume_mm3);
        }
    }
}

#[test]
fn a_flared_lathe_with_a_widening_cone_is_oriented_outward() {
    // Radius grows with z (cone opening toward +z), a stepped bushing with a bore.
    let pts = [[1., 0.], [3., 0.], [3., 2.], [5., 4.], [5., 6.], [1., 6.]];
    let (f, s) = round_trip(&NativeBrep::lathe(&pts).unwrap(), 0.002);
    assert_eq!(f, 6);
    let exact = lathe_volume(&pts);
    assert!(s.signed_volume_mm3 > 0.0 && (s.signed_volume_mm3 / exact - 1.).abs() < 3e-3, "{} vs {exact}", s.signed_volume_mm3);
}

#[test]
fn degenerate_input_is_refused() {
    let line = Profile { outer: poly(&[[0., 0.], [1., 0.], [2., 0.]]), holes: vec![] };
    assert!(NativeBrep::extrude(&line, 0., 1.).is_err());
    assert!(NativeBrep::extrude(&Profile { outer: poly(&[[0., 0.], [1., 0.], [1., 1.]]), holes: vec![] }, 0., 0.).is_err());
    assert!(NativeBrep::extrude(&Profile { outer: Loop::Circle { centre: [0., 0.], radius: -1. }, holes: vec![] }, 0., 1.).is_err());
    // A cone apex on the axis is not built natively (yet).
    assert!(NativeBrep::lathe(&[[0., 0.], [3., 0.], [0., 4.]]).is_err());
    assert!(NativeBrep::lathe(&[[1., 0.], [2., 0.]]).is_err());
}
