//! Findings before any solve: rigid-body motions the supports leave free, and a displacement far too large for a linear model.
use fea_problem::diagnose::{diagnose, diagnose_mesh, free_rigid_motions, predicted_displacement, FailureClass};
use fea_problem::{solve, templates, Analysis, Support};

fn plate() -> fea_problem::Problem {
    let mut p = templates::templates().remove(0).1;
    p.mesh.size = 1.0;
    p.mesh.adapt_passes = 0;
    p
}
fn block() -> fea_problem::Problem {
    let mut p = templates::templates().into_iter().map(|(_, p)| p).find(|p| p.analysis == Analysis::Solid).unwrap();
    p.mesh.size = 1.0;
    p
}
fn mesh_of(p: &fea_problem::Problem) -> fea_core::Mesh {
    fea_problem::solve::preview(p, None).unwrap()
}

#[test]
fn a_sound_support_set_leaves_no_motion_free_and_the_solver_agrees() {
    let p = plate();
    assert_eq!(free_rigid_motions(&p, &mesh_of(&p)), 0);
    assert!(diagnose_mesh(&p, &mesh_of(&p)).is_empty());
    assert!(solve(&p, None).is_ok());
    let b = block();
    assert_eq!(free_rigid_motions(&b, &mesh_of(&b)), 0, "a fixed face holds all six");
}

#[test]
fn a_roller_alone_leaves_the_part_free_to_slide_before_the_solver_has_to_say_so() {
    let mut p = plate();
    p.supports = vec![Support::roller("left", 0)];
    let mesh = mesh_of(&p);
    assert_eq!(free_rigid_motions(&p, &mesh), 1, "only the y translation is free: x held along an edge also stops the turn");
    let found = diagnose_mesh(&p, &mesh);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].class, FailureClass::UnderConstrained);
    assert!(found[0].summary.starts_with("Before solving"), "{}", found[0].summary);
    assert!(!found[0].remedies.is_empty(), "it offers the fixes the post-failure finding offers");
    // The solver trips over exactly this.
    let err = solve(&p, None).err().expect("the solve fails");
    assert!(err.contains("rigid-body") || err.to_lowercase().contains("singular") || err.contains("constrain"), "{err}");
}

#[test]
fn a_solid_held_in_one_direction_on_a_face_keeps_two_slides_and_a_turn() {
    let mut b = block();
    b.supports = vec![Support::roller("left", 0)];
    assert_eq!(free_rigid_motions(&b, &mesh_of(&b)), 3, "y and z translation and the turn about x");
    // Add a point that holds y and z where it sits: the turn about x is still free until that point is off the axis.
    b.supports = vec![Support::roller("left", 0), Support::Point { x: 0.0, y: 0.0, z: 0.0, ux: None, uy: Some(0.0), uz: Some(0.0) }];
    assert!(free_rigid_motions(&b, &mesh_of(&b)) <= 1);
}

#[test]
fn a_load_case_that_switches_the_supports_off_is_seen() {
    let mut p = plate();
    p.add_case("Free");
    p.cases[1].set_support(0, false);
    let mesh = mesh_of(&p);
    assert!(free_rigid_motions(&p, &mesh) >= 1, "case 2 keeps only the point support");
    p.active_case = 0;
    assert_eq!(free_rigid_motions(&p, &mesh), 0);
}

#[test]
fn a_displacement_that_is_a_large_share_of_the_part_is_forecast_from_the_loads_and_the_modulus() {
    let mut p = plate();
    let (u, long) = predicted_displacement(&p).expect("a forecast");
    // The real answer for the same model is of the same order as the bar estimate.
    let real = solve(&p, None).unwrap().summary.max_displacement.value;
    assert!(u / real > 0.3 && u / real < 3.0, "forecast {u:.3e} against the solved {real:.3e}");
    assert!(!diagnose(&p, None).iter().any(|d| d.class == FailureClass::ScaleSuspect), "a sound model raises nothing");
    // The classic slip: the modulus typed in ksi (29000) beside loads in psi.
    p.material.e = 29_000.0;
    let (u_slip, _) = predicted_displacement(&p).unwrap();
    assert!((u_slip / u - 1000.0).abs() < 1.0, "a thousand times softer, a thousand times the displacement");
    let found: Vec<_> = diagnose(&p, None).into_iter().filter(|d| d.class == FailureClass::ScaleSuspect).collect();
    assert_eq!(found.len(), 1, "forecast {u_slip:.3e} on {long}");
    assert!(found[0].summary.starts_with("Before solving"), "{}", found[0].summary);
    assert_eq!(found[0].questions[0].options.len(), 2);
    // Answering "1000 times too small" repairs the model and the finding goes away.
    let mut fixed = p.clone();
    found[0].questions[0].options[0].1.apply(&mut fixed).unwrap();
    assert!((fixed.material.e - 29.0e6).abs() < 1.0);
    assert!(!diagnose(&fixed, None).iter().any(|d| d.class == FailureClass::ScaleSuspect));
}

#[test]
fn nothing_is_forecast_without_supports_or_loads_and_a_load_case_changes_the_forecast() {
    let mut p = plate();
    p.loads.clear();
    assert!(predicted_displacement(&p).is_none());
    let mut p = plate();
    p.supports.clear();
    assert!(predicted_displacement(&p).is_none());
    // A case that doubles the load doubles the forecast.
    let mut p = plate();
    let (base, _) = predicted_displacement(&p).unwrap();
    p.add_case("Double");
    p.cases[1].set_factor(0, 2.0);
    let (double, _) = predicted_displacement(&p).unwrap();
    assert!((double / base - 2.0).abs() < 1e-9);
}
