//! The hybrid mesh strategy and goal-oriented adaptivity as the workbench uses them: through `Problem` and `solve`.
use fea_problem::{build, solve, templates, AdaptMode, Geometry, MeshStrategy, Problem, Shape};

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
fn a_steep_grading_next_to_the_hole_stays_hybrid_and_agrees_with_the_plain_mesh() {
    // Regression: the grid used to be cut along the whole strip by the fine size next to the hole, which sliced the regular
    // end blocks into slivers and tripped the size-jump gate (jump 8.1 against 6.0, so the mesh fell back to unstructured).
    let mut p = plate();
    p.mesh.size = 0.5;
    p.mesh.hole_factor = 0.05;
    p.mesh.strategy = MeshStrategy::Hybrid;
    let hybrid = solve::solve(&p, None).unwrap();
    assert!(!hybrid.summary.notes.iter().any(|n| n.contains("Hybrid meshing was not used")), "{:?}", hybrid.summary.notes);
    p.mesh.strategy = MeshStrategy::Unstructured;
    let plain = solve::solve(&p, None).unwrap();
    let rel = |a: f64, b: f64| ((a - b) / b).abs();
    assert!(rel(hybrid.summary.strain_energy, plain.summary.strain_energy) < 0.01, "energy {} vs {}", hybrid.summary.strain_energy, plain.summary.strain_energy);
    assert!(rel(hybrid.summary.max_von_mises.value, plain.summary.max_von_mises.value) < 0.05, "peak {} vs {}", hybrid.summary.max_von_mises.value, plain.summary.max_von_mises.value);
    assert!(hybrid.summary.equilibrium_error < 1e-6);
}

#[test]
fn a_hybrid_request_that_cannot_be_met_falls_back_and_says_why() {
    // A hole that fills most of a small plate leaves no worthwhile regular block.
    let mut p = plate();
    p.geometry = Geometry::Sketch { outer: Shape::Rect { x0: 0.0, y0: 0.0, x1: 4.0, y1: 4.0 }, holes: vec![Shape::Circle { cx: 2.0, cy: 2.0, r: 1.6 }], depth: 1.0, layers: 1, regions: Vec::new() };
    p.mesh.size = 0.4;
    p.mesh.hole_factor = 0.5;
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

#[test]
fn a_new_problem_meshes_hybrid_by_default_at_the_template_sizes() {
    // The reported case: the default plate-with-a-hole template, as is and with other sizes at the hole, was all-unstructured.
    assert_eq!(fea_problem::MeshSpec::default().strategy, MeshStrategy::Hybrid);
    for hole_factor in [0.1, 0.2, 0.4, 0.8] {
        let mut p = fea_problem::templates::templates().remove(0).1;
        p.mesh.hole_factor = hole_factor;
        assert_eq!(p.mesh.strategy, MeshStrategy::Hybrid, "the template carries the default");
        let s = fea_problem::solve(&p, None).unwrap();
        let kinds: Vec<_> = s.model.mesh.blocks.iter().map(|b| b.kind).collect();
        assert!(kinds.contains(&fea_core::ElementKind::Quad8), "regular quadrilateral blocks at hole factor {hole_factor}: {kinds:?}");
        assert!(kinds.contains(&fea_core::ElementKind::Tri6), "unstructured triangles around the hole: {kinds:?}");
        assert!(s.summary.notes.iter().all(|n| !n.contains("Hybrid meshing was not used")), "{:?}", s.summary.notes);
    }
}
