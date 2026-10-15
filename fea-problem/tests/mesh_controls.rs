//! The mesh controls that expose `fea-core`'s predictor, estimators and adaptive drivers to a problem.
use fea_problem::{build, solve, templates, Load, MeshStrategy, Problem};

fn plate() -> Problem {
    let mut p = templates::templates()[0].1.clone();
    p.mesh.size = 0.5;
    p.mesh.hole_factor = 1.0; // the hole itself asks for nothing: only the predictor does
    p.mesh.adapt_passes = 0;
    p
}

#[test]
fn the_size_predictor_only_ever_refines_and_asks_for_more_at_curves_and_point_loads() {
    let mut p = plate();
    p.loads.push(Load::Point { x: 2.0, y: 1.0, z: 0.0, fx: 0.0, fy: -100.0, fz: 0.0 });
    let plain = build::size_function(&p);
    p.mesh.predict_sizes = true;
    let predicted = build::size_function(&p);
    let mut finer = 0;
    for i in 0..60 {
        for j in 0..20 {
            let x = [0.1 + 0.2 * i as f64, 0.1 + 0.2 * j as f64];
            assert!(predicted(x) <= plain(x) + 1e-12, "the predictor coarsened the mesh at {x:?}");
            if predicted(x) < plain(x) - 1e-9 {
                finer += 1;
            }
        }
    }
    assert!(finer > 20, "only {finer} samples were refined");
    // the hole (r = 0.5 at (6, 2)): arc size r * 0.35 = 0.175 at its edge; the point load asks for a quarter of the size
    assert!(predicted([6.5, 2.0]) <= 0.18, "{}", predicted([6.5, 2.0]));
    assert!((predicted([2.0, 1.0]) - 0.125).abs() < 1e-12, "{}", predicted([2.0, 1.0]));
    // far from everything the base size stays
    assert!((predicted([11.5, 0.2]) - 0.5).abs() < 1e-12);
    // the forecast of the element count follows the size field
    let (e_plain, e_pred) = (build::estimate_mesh(&{ let mut q = p.clone(); q.mesh.predict_sizes = false; q }).unwrap().elements, build::estimate_mesh(&p).unwrap().elements);
    assert!(e_pred > 1.3 * e_plain, "{e_plain} -> {e_pred}");
}

#[test]
fn a_predicted_mesh_solves_and_the_hybrid_strategy_keeps_clear_of_a_point_load() {
    let mut p = plate();
    p.loads.clear();
    p.loads.push(Load::Point { x: 12.0, y: 2.0, z: 0.0, fx: 1000.0, fy: 0.0, fz: 0.0 });
    p.mesh.predict_sizes = true;
    p.mesh.strategy = MeshStrategy::Hybrid;
    let mesh = build::sketch_mesh(&p, None).unwrap();
    assert!(mesh.blocks.iter().any(|b| b.kind.n_corners() == 4), "the hybrid mesh must have structured blocks for this test to mean anything");
    // no structured (Quad) element touches the loaded node's neighbourhood
    let near = |n: usize| (mesh.nodes[n][0] - 12.0).hypot(mesh.nodes[n][1] - 2.0) < 0.3;
    for blk in &mesh.blocks {
        if blk.kind.n_corners() == 4 {
            assert!(blk.conn.iter().all(|&n| !near(n)), "a structured element reaches the point load");
        }
    }
    let s = solve::solve(&p, None).unwrap();
    assert!(s.summary.equilibrium_error < 1e-6, "{}", s.summary.equilibrium_error);
}

#[test]
fn a_problem_file_without_the_predictor_flag_reads_as_off() {
    let mut v = serde_json::to_value(plate()).unwrap();
    assert!(v["mesh"].get("predict_sizes").is_none(), "an unused flag stays out of the file");
    v["mesh"].as_object_mut().unwrap().remove("predict_sizes");
    let p: Problem = serde_json::from_value(v).unwrap();
    assert!(!p.mesh.predict_sizes);
}

fn triangle_plate() -> Problem {
    let mut p = templates::templates()[0].1.clone();
    p.mesh.element = fea_problem::ElementChoice::Tri6;
    p.mesh.size = 0.6;
    p.mesh.hole_factor = 0.4;
    p.mesh.adapt_passes = 2;
    p.mesh.target_error = 0.01;
    p
}

#[test]
fn the_equilibrated_driver_adapts_on_its_own_estimate_and_agrees_with_zz() {
    let mut p = triangle_plate();
    p.mesh.adapt_mode = fea_problem::AdaptMode::Zz;
    let zz = solve::solve(&p, None).unwrap();
    p.mesh.adapt_mode = fea_problem::AdaptMode::Equilibrated;
    let eq = solve::solve(&p, None).unwrap();
    // the estimate is the equilibrated one: a different number from the ZZ recovery on the same first mesh
    assert!((eq.history[0].zz_error - zz.history[0].zz_error).abs() > 1e-4, "{:?} vs {:?}", eq.history[0], zz.history[0]);
    assert!(eq.history[0].zz_error > 0.0 && eq.history.last().unwrap().zz_error < eq.history[0].zz_error, "{:?}", eq.history);
    // two different estimators drive two adaptive meshes to the same answer
    let rel = |a: f64, b: f64| ((a - b) / b).abs();
    assert!(rel(eq.summary.strain_energy, zz.summary.strain_energy) < 0.01, "energy {} vs {}", eq.summary.strain_energy, zz.summary.strain_energy);
    assert!(rel(eq.summary.max_von_mises.value, zz.summary.max_von_mises.value) < 0.06, "peak {} vs {}", eq.summary.max_von_mises.value, zz.summary.max_von_mises.value);
    assert!(eq.summary.equilibrium_error < 1e-6);
}

#[test]
fn the_equilibrated_driver_refuses_a_model_it_does_not_cover_and_says_why() {
    let mut p = triangle_plate();
    p.mesh.adapt_mode = fea_problem::AdaptMode::Equilibrated;
    p.mesh.element = fea_problem::ElementChoice::Quad8;
    let e = solve::solve(&p, None).unwrap_err();
    assert!(e.contains("equilibrated") && e.contains("ZZ"), "{e}");
    p.mesh.element = fea_problem::ElementChoice::Tri6;
    // A hybrid mesh has quadrilateral blocks, which the equilibrated reconstruction does not cover: the driver meshes the region
    // with triangles instead and says so, rather than refusing the (default) hybrid strategy.
    p.mesh.strategy = MeshStrategy::Hybrid;
    p.mesh.hole_factor = 1.0;
    let s = solve::solve(&p, None).unwrap();
    assert!(s.summary.notes.iter().any(|n| n.contains("triangle meshes only")), "{:?}", s.summary.notes);
    assert!(s.model.mesh.blocks.iter().all(|b| matches!(b.kind, fea_core::ElementKind::Tri3 | fea_core::ElementKind::Tri6)));
}

#[test]
fn the_goal_oriented_result_names_the_estimators_that_ran() {
    let mut p = triangle_plate();
    p.mesh.adapt_mode = fea_problem::AdaptMode::GoalOriented;
    p.mesh.element = fea_problem::ElementChoice::Tri6;
    let s = solve::solve(&p, None).unwrap();
    let note = s.summary.notes.iter().find(|n| n.starts_with("Goal-oriented adaptivity")).expect("a note names the estimators");
    assert!(note.contains("goal estimator") && note.contains("physics estimator"), "{note}");
}

/// Two bars end to end, 6 long each, 2 high: steel on the left, a softer material on the right.
fn two_materials() -> Problem {
    use fea_problem::{Geometry, MaterialSpec, Shape, SketchRegion, Support};
    let mut p = templates::templates()[0].1.clone();
    p.geometry = Geometry::Sketch {
        outer: Shape::Rect { x0: 0.0, y0: 0.0, x1: 6.0, y1: 2.0 },
        holes: Vec::new(),
        depth: 1.0,
        layers: 1,
        regions: vec![SketchRegion { outer: Shape::Rect { x0: 6.0, y0: 0.0, x1: 12.0, y1: 2.0 }, holes: Vec::new() }],
    };
    p.thickness = 1.0;
    p.material = MaterialSpec { name: "stiff".into(), e: 30.0e6, nu: 0.3, alpha: 0.0, yield_stress: Some(50_000.0), density: 0.0, plastic: false, hardening: 0.0 };
    p.region_materials = vec![Some(MaterialSpec { name: "soft".into(), e: 10.0e6, nu: 0.3, alpha: 0.0, yield_stress: Some(30_000.0), density: 0.0, plastic: false, hardening: 0.0 })];
    p.supports = vec![Support::roller("left", 0), Support::Point { x: 0.0, y: 1.0, z: 0.0, ux: None, uy: Some(0.0), uz: None }];
    p.loads = vec![Load::Traction { edge: "r2/right".into(), tx: 1000.0, ty: 0.0, tz: 0.0 }];
    p.mesh.size = 0.4;
    p.mesh.hole_factor = 1.0;
    p.mesh.adapt_passes = 0;
    p.mesh.bond_regions = true;
    p
}

fn tip_displacement(s: &solve::Solved) -> f64 {
    let nodes: Vec<usize> = (0..s.model.mesh.nodes.len()).filter(|&n| (s.model.mesh.nodes[n][0] - 12.0).abs() < 1e-9).collect();
    nodes.iter().map(|&n| s.u[2 * n]).sum::<f64>() / nodes.len() as f64
}

#[test]
fn bonded_regions_of_different_materials_carry_load_in_series() {
    // equal Poisson ratios: the bars extend as springs in series, u = sigma (L1 / E1 + L2 / E2)
    let want = 1000.0 * (6.0 / 30.0e6 + 6.0 / 10.0e6);
    for strategy in [MeshStrategy::Unstructured, MeshStrategy::Hybrid] {
        let mut p = two_materials();
        p.mesh.strategy = strategy;
        let s = solve::solve(&p, None).unwrap_or_else(|e| panic!("{strategy:?}: {e}"));
        let got = tip_displacement(&s);
        assert!((got / want - 1.0).abs() < 5e-3, "{strategy:?}: elongation {got:e} against {want:e}");
        assert!(s.summary.equilibrium_error < 1e-6);
        // each bar carries sigma = 1000 on its own side of the interface, and the stress is not averaged over the jump
        let (mut left, mut right) = (Vec::new(), Vec::new());
        for (n, x) in s.model.mesh.nodes.iter().enumerate() {
            if (x[0] - 3.0).abs() < 0.5 { left.push(s.nodal[n][0]); }
            if (x[0] - 9.0).abs() < 0.5 { right.push(s.nodal[n][0]); }
        }
        let mean = |v: &[f64]| v.iter().sum::<f64>() / v.len() as f64;
        assert!((mean(&left) - 1000.0).abs() < 20.0 && (mean(&right) - 1000.0).abs() < 20.0, "{strategy:?}: {} / {}", mean(&left), mean(&right));
        assert!(s.summary.margin.is_none(), "the yield margin of one material would mislead");
        assert!(s.summary.notes.iter().any(|n| n.contains("material interface")), "{:?}", s.summary.notes);
        // two blocks' materials really differ
        let es: Vec<f64> = s.model.mesh.blocks.iter().map(|b| b.material.e).collect();
        assert!(es.iter().any(|&e| e == 30.0e6) && es.iter().any(|&e| e == 10.0e6), "{es:?}");
    }
}

#[test]
fn unbonded_regions_are_separate_bodies_and_a_mismatch_is_refused() {
    let mut p = two_materials();
    p.mesh.bond_regions = false;
    let e = solve::solve(&p, None).unwrap_err();
    assert!(e.to_lowercase().contains("constrain") || e.to_lowercase().contains("rigid") || e.contains("r2"), "the second body is free: {e}");
    // shift the second bar so its boundary no longer meets the first
    let mut q = two_materials();
    if let fea_problem::Geometry::Sketch { regions, .. } = &mut q.geometry {
        regions[0].outer = fea_problem::Shape::Rect { x0: 6.0, y0: 0.5, x1: 12.0, y1: 2.5 };
    }
    q.loads = vec![Load::Traction { edge: "r2/right".into(), tx: 1000.0, ty: 0.0, tz: 0.0 }];
    let e = solve::solve(&q, None).unwrap_err();
    assert!(e.contains("interface") || e.contains("conform") || e.contains("weld"), "{e}");
}

#[test]
fn an_extruded_hybrid_plate_is_hex_and_wedge_and_agrees_with_the_unstructured_extrusion() {
    let mut p = templates::templates().into_iter().find(|(n, _)| n.contains("3D")).unwrap().1;
    p.mesh.size = 0.25;
    p.mesh.hole_factor = 0.4;
    p.mesh.adapt_passes = 0;
    p.mesh.strategy = MeshStrategy::Unstructured;
    let plain = solve::solve(&p, None).unwrap();
    p.mesh.strategy = MeshStrategy::Hybrid;
    let hybrid = solve::solve(&p, None).unwrap();
    let kinds = solve::kinds(&hybrid.model.mesh);
    assert!(kinds.contains(&fea_core::ElementKind::Hex20) && kinds.contains(&fea_core::ElementKind::Wedge15), "{kinds:?}");
    assert!(!hybrid.summary.notes.iter().any(|n| n.contains("Hybrid meshing was not used")), "{:?}", hybrid.summary.notes);
    let rel = |a: f64, b: f64| ((a - b) / b).abs();
    assert!(rel(hybrid.summary.strain_energy, plain.summary.strain_energy) < 0.02, "energy {} vs {}", hybrid.summary.strain_energy, plain.summary.strain_energy);
    assert!(rel(hybrid.summary.max_von_mises.value, plain.summary.max_von_mises.value) < 0.08, "peak {} vs {}", hybrid.summary.max_von_mises.value, plain.summary.max_von_mises.value);
    assert!(hybrid.summary.equilibrium_error < 1e-6);
    eprintln!("3D: dofs {} -> {} (hybrid), energy {:.6e} / {:.6e}, peak {:.1} / {:.1}", plain.summary.dofs, hybrid.summary.dofs, plain.summary.strain_energy, hybrid.summary.strain_energy, plain.summary.max_von_mises.value, hybrid.summary.max_von_mises.value);
}

fn plastic_bar() -> Problem {
    use fea_problem::{Geometry, MaterialSpec, Shape, Support};
    let mut p = templates::templates()[0].1.clone();
    p.geometry = Geometry::Sketch { outer: Shape::Rect { x0: 0.0, y0: 0.0, x1: 10.0, y1: 1.0 }, holes: Vec::new(), depth: 1.0, layers: 1, regions: Vec::new() };
    p.thickness = 1.0;
    p.material = MaterialSpec { name: "bilinear".into(), e: 10.0e6, nu: 0.3, alpha: 0.0, yield_stress: Some(20_000.0), density: 0.0, plastic: true, hardening: 1.0e6 };
    p.supports = vec![Support::roller("left", 0), Support::Point { x: 0.0, y: 0.5, z: 0.0, ux: None, uy: Some(0.0), uz: None }];
    p.loads = vec![Load::Traction { edge: "right".into(), tx: 30_000.0, ty: 0.0, tz: 0.0 }];
    p.mesh.size = 0.5;
    p.mesh.hole_factor = 1.0;
    p.mesh.adapt_passes = 0;
    p
}

#[test]
fn a_bar_stretched_past_yield_follows_the_bilinear_stress_strain_curve() {
    // uniform tension 30000 over yield 20000 with H = 1e6: ep = 0.01, strain = 30000 / 1e7 + 0.01, tip displacement 10 * strain
    let want = 10.0 * (30_000.0 / 10.0e6 + 10_000.0 / 1.0e6);
    for element in [fea_problem::ElementChoice::Quad8, fea_problem::ElementChoice::Tri6] {
        let mut p = plastic_bar();
        p.mesh.element = element;
        let s = solve::solve(&p, None).unwrap_or_else(|e| panic!("{element:?}: {e}"));
        let tip: f64 = {
            let nodes: Vec<usize> = (0..s.model.mesh.nodes.len()).filter(|&n| (s.model.mesh.nodes[n][0] - 10.0).abs() < 1e-9).collect();
            nodes.iter().map(|&n| s.u[2 * n]).sum::<f64>() / nodes.len() as f64
        };
        assert!((tip / want - 1.0).abs() < 1e-4, "{element:?}: tip {tip} against {want}");
        let q = s.summary.plastic.as_ref().expect("a plastic summary");
        assert!((q.max_strain - 0.01).abs() < 1e-4 && q.yielded_fraction > 0.99, "{q:?}");
        let ep = s.node_values(fea_problem::Field::PlasticStrain);
        assert!(ep.iter().all(|&v| (v - 0.01).abs() < 1e-4), "uniform plastic strain");
        assert!(s.fields().contains(&fea_problem::Field::PlasticStrain));
        assert!(s.summary.equilibrium_error < 1e-6, "{}", s.summary.equilibrium_error);
        assert!((s.summary.max_von_mises.value / 30_000.0 - 1.0).abs() < 1e-3, "stress {}", s.summary.max_von_mises.value);
    }
}

#[test]
fn below_yield_the_elastic_plastic_solve_is_the_elastic_one() {
    let mut p = plastic_bar();
    p.loads = vec![Load::Traction { edge: "right".into(), tx: 15_000.0, ty: 0.0, tz: 0.0 }];
    let plastic = solve::solve(&p, None).unwrap();
    p.material.plastic = false;
    let elastic = solve::solve(&p, None).unwrap();
    assert_eq!(plastic.summary.plastic.as_ref().unwrap().max_strain, 0.0);
    let rel = |a: f64, b: f64| ((a - b) / b).abs();
    assert!(rel(plastic.summary.max_displacement.value, elastic.summary.max_displacement.value) < 1e-7);
    assert!(rel(plastic.summary.max_von_mises.value, elastic.summary.max_von_mises.value) < 1e-6);
    assert!(elastic.plastic.is_none() && !elastic.fields().contains(&fea_problem::Field::PlasticStrain));
}

#[test]
fn elastic_plastic_requests_that_cannot_be_solved_are_refused_with_the_reason() {
    let mut p = plastic_bar();
    p.material.yield_stress = None;
    assert!(solve::solve(&p, None).unwrap_err().contains("yield stress"));
    let mut p = plastic_bar();
    p.material.hardening = -1.0;
    assert!(solve::solve(&p, None).unwrap_err().contains("hardening"));
    let mut p = plastic_bar();
    p.mesh.adapt_mode = fea_problem::AdaptMode::GoalOriented;
    p.mesh.adapt_passes = 1;
    assert!(solve::solve(&p, None).unwrap_err().contains("elastic materials"));
    // a load far above the collapse load of a perfectly plastic bar: the solver says it stopped
    let mut p = plastic_bar();
    p.material.hardening = 0.0;
    p.loads = vec![Load::Traction { edge: "right".into(), tx: 40_000.0, ty: 0.0, tz: 0.0 }];
    let e = solve::solve(&p, None).unwrap_err();
    assert!(e.contains("elastic-plastic"), "{e}");
}

#[test]
fn adaptive_passes_refine_an_elastic_plastic_plate_toward_the_fine_mesh_answer() {
    let mut p = templates::templates()[0].1.clone();
    p.material.plastic = true;
    p.material.hardening = 2.0e5;
    p.material.yield_stress = Some(40_000.0);
    p.material.e = 10.0e6;
    p.loads = vec![Load::Traction { edge: "right".into(), tx: 21_000.0, ty: 0.0, tz: 0.0 }];
    p.mesh.size = 1.0;
    p.mesh.hole_factor = 0.5;
    p.mesh.target_error = 0.02;
    p.mesh.adapt_passes = 0;
    let coarse = solve::solve(&p, None).unwrap();
    assert!(coarse.summary.plastic.as_ref().unwrap().max_strain > 0.0, "the hole edge must yield");
    let mut fine = p.clone();
    fine.mesh.size = 0.15;
    fine.mesh.hole_factor = 0.4;
    let reference = solve::solve(&fine, None).unwrap();
    p.mesh.adapt_passes = 3;
    let adaptive = solve::solve(&p, None).unwrap();
    assert!(adaptive.history.len() > 1 && adaptive.summary.elements > coarse.summary.elements, "{:?}", adaptive.history);
    assert!(adaptive.summary.notes.iter().any(|n| n.starts_with("Elastic-plastic adaptivity")), "{:?}", adaptive.summary.notes);
    let err = |s: &solve::Solved| (s.summary.strain_energy / reference.summary.strain_energy - 1.0).abs();
    eprintln!("work error against the fine mesh: coarse {:.2e} ({} el), adaptive {:.2e} ({} el); fine {} el", err(&coarse), coarse.summary.elements, err(&adaptive), adaptive.summary.elements, reference.summary.elements);
    assert!(err(&adaptive) < err(&coarse), "adaptive {:e} vs coarse {:e}", err(&adaptive), err(&coarse));
    assert!(adaptive.summary.equilibrium_error < 1e-6);
}
