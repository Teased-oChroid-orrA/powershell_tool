//! Parity of the native modeller with OpenCASCADE (`occt` feature): OpenCASCADE reads the STEP text `NativeBrep` writes and finds the
//! same volume and face count as the OpenCASCADE-built shape of the same construction. `docs/issue-46-phase-120.md`.
#![cfg(feature = "occt")]

use solid_kernel::native_brep::NativeBrep;
use solid_kernel::occt::{Brep, Loop, Profile};

fn check(name: &str, native: &NativeBrep, occt: &Brep, exact: f64) {
    let text = native.to_step_text();
    let read = Brep::from_step_text(&text).unwrap_or_else(|e| panic!("{name}: OpenCASCADE refused the native STEP: {e}"));
    let (v_read, v_occt) = (read.solid(0.002).unwrap().volume(), occt.solid(0.002).unwrap().volume());
    eprintln!("{name:<24} faces native/OCCT {}/{} | exact {exact:.5} | OCCT reads native {v_read:.5} | OCCT built {v_occt:.5}", read.face_count(), occt.face_count());
    assert_eq!(read.face_count(), occt.face_count(), "{name}");
    assert!((v_read / exact - 1.).abs() < 2e-3, "{name}: {v_read} vs {exact}");
    assert!((v_read / v_occt - 1.).abs() < 1e-3, "{name}: {v_read} vs {v_occt}");
}

#[test]
fn opencascade_reads_what_the_native_writer_writes() {
    let pi = std::f64::consts::PI;
    let l = Profile { outer: Loop::Polygon(vec![[0., 0.], [4., 0.], [4., 1.], [1., 1.], [1., 3.], [0., 3.]]), holes: vec![] };
    let plate = Profile {
        outer: Loop::Polygon(vec![[0., 0.], [12., 0.], [12., 4.], [0., 4.]]),
        holes: vec![Loop::Circle { centre: [6., 2.], radius: 0.5 }, Loop::Polygon(vec![[1., 1.], [3., 1.], [3., 3.], [1., 3.]])],
    };
    let native = |p: &Profile, d: f64| NativeBrep::extrude(p, 0., d).unwrap();
    let occt = |p: &Profile, d: f64| Brep::extrude(p, 0., d).unwrap();
    check("extruded L", &native(&l, 2.), &occt(&l, 2.), 12.);
    check("plate with two holes", &native(&plate, 0.25), &occt(&plate, 0.25), (48. - 4. - pi * 0.25) * 0.25);
    check("box", &NativeBrep::box_from_corners([0.; 3], [2., 3., 4.]).unwrap(), &Brep::box_from_corners([0.; 3], [2., 3., 4.]), 24.);
    check("cylinder", &NativeBrep::cylinder_z([0.; 3], 2., 3.).unwrap(), &Brep::cylinder([0.; 3], 2., [0., 0., 1.], 3.), pi * 12.);
    check(
        "frustum",
        &NativeBrep::lathe(&[[0., 0.], [6., 0.], [3., 8.], [0., 8.]]).unwrap(),
        &Brep::cone([0.; 3], 6., 3., [0., 0., 1.], 8.),
        pi * 8. * 63. / 3.,
    );
}

/// The solid editor's Hole / Boss, done by OpenCASCADE booleans (as `fea_problem::solid::apply_op` does) and natively.
#[test]
fn native_hole_and_boss_against_opencascade_booleans() {
    use solid_kernel::occt::Bool;
    let pi = std::f64::consts::PI;
    let (native, occt) = (NativeBrep::box_from_corners([0.; 3], [10., 10., 4.]).unwrap(), Brep::box_from_corners([0.; 3], [10., 10., 4.]));
    // Through hole r = 1 at (3, 6) from the top face.
    let hole = Brep::cylinder([3., 6., 4. + 20.], 1., [0., 0., -1.], 60.);
    check("through hole", &native.drill(5, [3., 6., 4.], 1., None).unwrap(), &occt.boolean(Bool::Cut, &hole).unwrap(), 400. - pi * 4.);
    // Boss r = 2, h = 3 at (5, 5), starting slightly inside as the editor does.
    let boss = Brep::cylinder([5., 5., 4. - 0.02], 2., [0., 0., 1.], 3. + 0.02);
    check("boss", &native.boss(5, [5., 5., 4.], 2., 3.).unwrap(), &occt.boolean(Bool::Union, &boss).unwrap(), 400. + pi * 12.);
    // Then a hole through the boss and block.
    let n2 = native.boss(5, [5., 5., 4.], 2., 3.).unwrap().drill(7, [5., 5., 7.], 0.5, None).unwrap();
    let o2 = occt.boolean(Bool::Union, &boss).unwrap().boolean(Bool::Cut, &Brep::cylinder([5., 5., 27.], 0.5, [0., 0., -1.], 60.)).unwrap();
    check("hole through boss", &n2, &o2, 400. + pi * 12. - pi * 0.25 * 7.);
}
