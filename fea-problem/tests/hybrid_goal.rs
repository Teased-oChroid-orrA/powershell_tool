//! The hybrid mesh strategy and goal-oriented adaptivity as the workbench uses them: through `Problem` and `solve`.
use fea_problem::{build, solve, templates, AdaptMode, MeshStrategy, Problem};

fn plate() -> Problem {
    let mut p = templates::templates()[0].1.clone();
    p.mesh.size = 0.3;
    p.mesh.hole_factor = 0.3;
    p.mesh.adapt_passes = 0;
    p
}

#[test]
fn a_hybrid_plate_has_both_element_families_the_same_names_and_the_same_physics() {
    let mut p = plate();
    p.mesh.strategy = MeshStrategy::Unstructured;
    let plain = solve::solve(&p, None).unwrap();
    p.mesh.strategy = MeshStrategy::Hybrid;
    let mixed = build::sketch_mesh(&p, None).unwrap();
    let kinds = solve::kinds(&mixed);
    assert!(kinds.len() >= 2, "a hybrid mesh mixes element kinds: {kinds:?}");
    let plain_mesh = build::sketch_mesh(&{ let mut q = p.clone(); q.mesh.strategy = MeshStrategy::Unstructured; q }, None).unwrap();
    let names = |m: &fea_core::Mesh| -> Vec<String> { let mut v: Vec<String> = m.surfaces.keys().cloned().collect(); v.sort(); v };
    assert_eq!(names(&mixed), names(&plain_mesh), "the supports and loads of the problem must find their surfaces");
    let hybrid = solve::solve(&p, None).unwrap();
    let rel = |a: f64, b: f64| ((a - b) / b).abs();
    assert!(rel(hybrid.summary.strain_energy, plain.summary.strain_energy) < 0.01, "energy {} vs {}", hybrid.summary.strain_energy, plain.summary.strain_energy);
    // measured: 7.5k against 37k dofs, energy 1e-6 and peak stress 1.7 % from the plain mesh
    assert!(hybrid.summary.dofs * 3 < plain.summary.dofs, "{} vs {} dofs", hybrid.summary.dofs, plain.summary.dofs);
    assert!(rel(hybrid.summary.max_von_mises.value, plain.summary.max_von_mises.value) < 0.03, "peak {} vs {}", hybrid.summary.max_von_mises.value, plain.summary.max_von_mises.value);
    assert!(hybrid.summary.equilibrium_error < 1e-6);
}

#[test]
fn a_hybrid_request_that_cannot_be_met_falls_back_and_says_why() {
    let mut p = plate();
    p.mesh.size = 0.5;
    p.mesh.hole_factor = 0.05; // grading too steep for structured blocks next to the hole
    p.mesh.strategy = MeshStrategy::Hybrid;
    let s = solve::solve(&p, None).unwrap();
    assert!(s.summary.notes.iter().any(|n| n.contains("Hybrid meshing was not used")), "{:?}", s.summary.notes);
    assert!(s.summary.equilibrium_error < 1e-6);
}

#[test]
fn a_problem_file_without_the_new_fields_still_reads_as_unstructured_zz() {
    let mut v = serde_json::to_value(plate()).unwrap();
    let mesh = v["mesh"].as_object_mut().unwrap();
    mesh.remove("strategy");
    mesh.remove("adapt_mode");
    let p: Problem = serde_json::from_value(v).unwrap();
    assert_eq!((p.mesh.strategy, p.mesh.adapt_mode), (MeshStrategy::Unstructured, AdaptMode::Zz));
}

#[test]
fn goal_oriented_adaptivity_reduces_the_error_estimate_and_solves_through_the_ordinary_path() {
    let mut p = plate();
    p.mesh.size = 1.0;
    p.mesh.adapt_passes = 3;
    p.mesh.target_error = 0.002;
    p.mesh.adapt_mode = AdaptMode::GoalOriented;
    let s = solve::solve(&p, None).unwrap();
    assert!(s.history.len() >= 2, "{:?}", s.history.len());
    let first = s.history.first().unwrap();
    let last = s.history.last().unwrap();
    assert!(last.zz_error < first.zz_error, "error {} -> {}", first.zz_error, last.zz_error);
    assert!(last.elements > first.elements && last.zz_error < 0.002, "{last:?}");
    assert_eq!((last.nodes, last.elements), (s.summary.nodes, s.summary.elements));
    assert!(s.summary.equilibrium_error < 1e-6);
    p.bushings.push(fea_problem::Bushing { hole: 1, inner_diameter: 0.8, offset: [0.0, 0.0], interference: 0.001, friction: 0.2, material: p.material.clone() });
    assert!(solve::solve(&p, None).unwrap_err().contains("bushings"));
}
