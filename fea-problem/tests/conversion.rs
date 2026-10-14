//! Converting a support or load to another kind keeps what it means.
use fea_problem::conversion::{nearest_face, LoadKind, SupportKind};
use fea_problem::facts::Facts;
use fea_problem::{solve, templates, Load, Problem, Support};

fn plate() -> Problem {
    let mut p = templates::templates().remove(0).1;
    p.mesh.size = 1.0;
    p.mesh.adapt_passes = 0;
    p.loads = vec![Load::Traction { edge: "right".into(), tx: 10_000.0, ty: 0.0, tz: 0.0 }];
    p
}
fn facts(p: &Problem) -> Facts {
    Facts::of(p).unwrap()
}

#[test]
fn a_traction_becomes_the_force_it_adds_up_to_and_back_and_the_response_is_unchanged() {
    let mut p = plate();
    let f = facts(&p);
    let before = solve(&p, None).unwrap();
    let notes = p.convert_load(0, LoadKind::Force, None, &f).unwrap();
    assert!(notes.is_empty());
    // The right end of the 12 x 4 plate of thickness 0.25 has area 1: 10 000 psi is 10 000 lbf.
    assert!(matches!(&p.loads[0], Load::Force { edge, fx, fy, .. } if edge == "right" && (fx - 10_000.0).abs() < 1e-9 && fy.abs() < 1e-12), "{:?}", p.loads[0]);
    let after = solve(&p, None).unwrap();
    assert!((after.summary.applied[0] - before.summary.applied[0]).abs() < 1e-6 * before.summary.applied[0].abs());
    assert!((after.summary.max_displacement.value / before.summary.max_displacement.value - 1.0).abs() < 1e-6);
    p.convert_load(0, LoadKind::Traction, None, &f).unwrap();
    assert!(matches!(&p.loads[0], Load::Traction { tx, .. } if (tx - 10_000.0).abs() < 1e-6), "{:?}", p.loads[0]);
}

#[test]
fn a_pressure_a_point_force_and_a_pin_load_convert_to_each_other_on_a_chosen_face() {
    let mut p = plate();
    let f = facts(&p);
    // Pressure 2000 on the top face (area 12 x 0.25 = 3): pushes down, a force of 6000 in -y.
    p.loads = vec![Load::Pressure { edge: "top".into(), p: 2000.0 }];
    p.convert_load(0, LoadKind::Force, None, &f).unwrap();
    assert!(matches!(&p.loads[0], Load::Force { fy, fx, .. } if (fy + 6000.0).abs() < 1e-9 && fx.abs() < 1e-9), "{:?}", p.loads[0]);
    // A point force becomes a pressure on the face nearest to it, or on the one picked; the along-face part is reported.
    p.loads = vec![Load::Point { x: 12.0, y: 2.0, z: 0.0, fx: 1000.0, fy: 0.0, fz: 0.0 }];
    p.convert_load(0, LoadKind::Pressure, None, &f).unwrap();
    // The nearest face to (12, 2) is "right" (area 1): a pull of 1000 is a pressure of -1000 (tension, pointing out).
    assert!(matches!(&p.loads[0], Load::Pressure { edge, p } if edge == "right" && (p + 1000.0).abs() < 1e-9), "{:?}", p.loads[0]);
    // A diagonal force on a face: the normal part becomes the pressure and the rest is dropped with a note.
    p.loads = vec![Load::Force { edge: "right".into(), fx: 600.0, fy: 800.0, fz: 0.0 }];
    let notes = p.convert_load(0, LoadKind::Pressure, None, &f).unwrap();
    assert!(matches!(&p.loads[0], Load::Pressure { p, .. } if (p + 600.0).abs() < 1e-9), "{:?}", p.loads[0]);
    assert_eq!(notes.len(), 1);
    assert!(notes[0].contains("dropped"), "{notes:?}");
    // A force on a hole becomes a pin load; a pin load becomes a point force at the hole.
    p.loads = vec![Load::Force { edge: "hole1".into(), fx: 500.0, fy: 0.0, fz: 0.0 }];
    p.convert_load(0, LoadKind::Bearing, None, &f).unwrap();
    assert!(matches!(&p.loads[0], Load::Bearing { hole: 1, fx, .. } if (fx - 500.0).abs() < 1e-9));
    p.convert_load(0, LoadKind::Force, Some("right"), &f).unwrap();
    assert!(matches!(&p.loads[0], Load::Force { edge, fx, .. } if edge == "right" && (fx - 500.0).abs() < 1e-9));
    p.convert_load(0, LoadKind::Point, None, &f).unwrap();
    assert!(matches!(&p.loads[0], Load::Point { x, y, fx, .. } if (x - 12.0).abs() < 1e-9 && (y - 2.0).abs() < 1e-9 && (fx - 500.0).abs() < 1e-9), "{:?}", p.loads[0]);
}

#[test]
fn a_conversion_that_cannot_be_exact_is_refused_with_the_reason_and_changes_nothing() {
    let mut p = plate();
    let f = facts(&p);
    p.loads = vec![Load::Pressure { edge: "hole1".into(), p: 100.0 }];
    let before = p.clone();
    let e = p.convert_load(0, LoadKind::Force, None, &f).unwrap_err();
    assert!(e.contains("no single direction") || e.contains("not known"), "{e}");
    assert_eq!(p, before);
    p.loads = vec![Load::Traction { edge: "right".into(), tx: 1.0, ty: 0.0, tz: 0.0 }];
    assert!(p.convert_load(0, LoadKind::Traction, None, &f).unwrap_err().contains("already"));
    assert!(p.convert_load(0, LoadKind::Bearing, None, &f).unwrap_err().contains("hole"));
    p.loads = vec![Load::Body { bx: 0.0, by: -0.1, bz: 0.0 }];
    assert!(p.convert_load(0, LoadKind::Force, Some("right"), &f).unwrap_err().contains("body force"));
    assert!(p.convert_load(9, LoadKind::Force, None, &f).is_err());
}

#[test]
fn supports_convert_between_fixed_a_roller_and_a_point() {
    let mut p = plate();
    let f = facts(&p);
    p.supports = vec![Support::roller("left", 0)];
    p.convert_support(0, SupportKind::Fixed, None, &f).unwrap();
    assert_eq!(p.supports[0].comps(), [Some(0.0), Some(0.0), None], "a plane model has two directions");
    // Back to a roller: only the direction normal to the left face (x) is held.
    let notes = p.convert_support(0, SupportKind::Slides, None, &f).unwrap();
    assert!(notes.is_empty());
    assert_eq!(p.supports[0].comps(), [Some(0.0), None, None]);
    // A prescribed displacement survives.
    p.supports = vec![Support::Edge { edge: "left".into(), ux: Some(0.01), uy: None, uz: None }];
    p.convert_support(0, SupportKind::Fixed, None, &f).unwrap();
    assert_eq!(p.supports[0].comps(), [Some(0.01), Some(0.0), None]);
    // To a point at the face centre, and from a point to a face (the nearest, or the one picked).
    p.convert_support(0, SupportKind::Point, None, &f).unwrap();
    assert!(matches!(&p.supports[0], Support::Point { x, y, .. } if x.abs() < 1e-9 && (y - 2.0).abs() < 1e-9), "{:?}", p.supports[0]);
    p.convert_support(0, SupportKind::Slides, None, &f).unwrap();
    assert!(matches!(&p.supports[0], Support::Edge { edge, .. } if edge == "left"));
    p.convert_support(0, SupportKind::Fixed, Some("bottom"), &f).unwrap();
    assert!(matches!(&p.supports[0], Support::Edge { edge, .. } if edge == "bottom"));
    // The converted model still solves when it is supported enough.
    p.supports = vec![Support::roller("left", 0)];
    p.convert_support(0, SupportKind::Fixed, None, &f).unwrap();
    assert!(solve(&p, None).is_ok());
    assert_eq!(nearest_face(&f, [11.9, 2.0, 0.0]).as_deref(), Some("right"));
}
