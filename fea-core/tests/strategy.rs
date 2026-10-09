//! Validation, the adaptive linear solve (verification, reversion) and the nonlinear ladder.

use fea_core::generate::grid;
use fea_core::loads::SurfaceLoad;
use fea_core::material::J2;
use fea_core::*;
use mechanics_core::hardening::Hardening;

fn plate(kind: ElementKind, n: usize) -> (Model, Dirichlet, Loads) {
    let mesh = grid(Physics::PlaneStress { thickness: 1.0 }, kind, Elastic::new(1.0e7, 0.3), [n, n, 1], &|p| [2.0 * p[0], p[1], 0.0]).unwrap();
    let model = Model::new(mesh).unwrap();
    let mut bc = model.dirichlet();
    for nd in 0..model.mesh.nodes.len() {
        if model.mesh.nodes[nd][0] < 1e-12 {
            bc.fix_node(nd);
        }
    }
    let faces = model.mesh.surfaces["u1"].clone();
    let loads = Loads { faces: faces.into_iter().map(|f| (f, SurfaceLoad::Traction([0.0, -100.0, 0.0]))).collect(), ..Loads::default() };
    (model, bc, loads)
}

fn codes(issues: &[Issue]) -> Vec<&'static str> {
    issues.iter().map(|i| i.code).collect()
}

#[test]
fn a_sound_model_has_no_errors() {
    let (model, bc, loads) = plate(ElementKind::Quad9, 4);
    let issues = model.validate(&loads, &bc);
    assert!(issues.iter().all(|i| i.severity != Severity::Error), "{issues:?}");
}

#[test]
fn validation_reports_every_defect_it_finds() {
    let (model, _, loads) = plate(ElementKind::Quad4, 3);
    // Nothing supported: rigid body; no load: a warning.
    let free = model.dirichlet();
    let c = codes(&model.validate(&Loads::default(), &free));
    assert!(c.contains(&"rigid_body") && c.contains(&"no_load"), "{c:?}");
    // A node that belongs to no element.
    let mut mesh = model.mesh.clone();
    mesh.add_node([9.0, 9.0, 0.0]);
    let m2 = Model::new(mesh).unwrap();
    let mut bc = m2.dirichlet();
    for nd in 0..m2.mesh.nodes.len() {
        if m2.mesh.nodes[nd][0] < 1e-12 {
            bc.fix_node(nd);
        }
    }
    assert!(codes(&m2.validate(&loads, &bc)).contains(&"orphan_node"));
    // An inverted element.
    let mut mesh = model.mesh.clone();
    mesh.nodes.swap(0, 1);
    let m3 = Model::new(mesh).unwrap();
    let mut bc = m3.dirichlet();
    bc.fix_node(5);
    bc.fix_node(2);
    assert!(codes(&m3.validate(&loads, &bc)).contains(&"inverted_element"));
    // Constant-strain elements are flagged.
    let mut mesh = grid(Physics::PlaneStress { thickness: 1.0 }, ElementKind::Tri3, Elastic::new(1.0, 0.3), [2, 2, 1], &|p| p).unwrap();
    mesh.set_density_all(1.0).unwrap();
    let tri = Model::new(mesh).unwrap();
    let mut bc = tri.dirichlet();
    for nd in 0..tri.mesh.nodes.len() {
        if tri.mesh.nodes[nd][0] < 1e-12 {
            bc.fix_node(nd);
        }
    }
    assert!(codes(&tri.validate(&Loads::default(), &bc)).contains(&"low_order_element"));
}

#[test]
fn the_adaptive_solve_matches_the_plain_solve_and_reports_its_checks() {
    let (model, bc, loads) = plate(ElementKind::Quad9, 6);
    let ad = model.solve_adaptive(&loads, &bc, &Requirements::default()).unwrap();
    let plain = model.solve_static(&loads, &bc).unwrap();
    assert_eq!(ad.method, SolveMethod::Direct);
    assert!(ad.verification.passed && ad.verification.rel_residual < 1e-10 && ad.verification.equilibrium < 1e-9, "{:?}", ad.verification);
    assert!(ad.solution.u.iter().zip(&plain.u).all(|(a, b)| a == b));
    let json = ad.report_json();
    assert!(json.contains("\"passed\":true") && json.contains("\"method\":\"Direct\""), "{json}");
}

#[test]
fn the_adaptive_solve_refuses_an_invalid_model_with_every_error() {
    let (model, _, loads) = plate(ElementKind::Quad9, 3);
    let err = model.solve_adaptive(&loads, &model.dirichlet(), &Requirements::default()).unwrap_err();
    assert!(err.contains("rigid_body"), "{err}");
}

#[test]
fn an_unmet_discretisation_requirement_is_an_error_that_says_so() {
    let (model, bc, loads) = plate(ElementKind::Quad4, 2);
    let req = Requirements { max_zz_error: Some(1e-9), ..Requirements::default() };
    let err = model.solve_adaptive(&loads, &bc, &req).unwrap_err();
    assert!(err.contains("ZZ") && err.contains("refine the mesh"), "{err}");
    // Reported (not required) it is just a number.
    let ok = model.solve_adaptive(&loads, &bc, &Requirements { max_zz_error: Some(1.0), ..Requirements::default() }).unwrap();
    assert!(ok.verification.zz_error.is_some_and(|e| e > 0.0 && e < 1.0));
}

#[test]
fn a_forced_iterative_solve_is_verified_and_a_failing_one_is_reverted() {
    let mut mesh = grid(Physics::Solid, ElementKind::Hex8, Elastic::new(1.0e7, 0.3), [8, 8, 8], &|p| p).unwrap();
    mesh.set_density_all(1.0).unwrap();
    let model = Model::new(mesh).unwrap();
    let mut bc = model.dirichlet();
    for nd in 0..model.mesh.nodes.len() {
        if model.mesh.nodes[nd][2] < 1e-12 {
            bc.fix_node(nd);
        }
    }
    let faces = model.mesh.surfaces["w1"].clone();
    let loads = Loads { faces: faces.into_iter().map(|f| (f, SurfaceLoad::Pressure(50.0))).collect(), ..Loads::default() };
    let direct = model.solve_adaptive(&loads, &bc, &Requirements::default()).unwrap();
    let it = model.solve_adaptive(&loads, &bc, &Requirements { force_method: Some(SolveMethod::iterative()), ..Requirements::default() }).unwrap();
    assert!(matches!(it.method, SolveMethod::Iterative { .. }) && it.verification.passed);
    let scale = direct.solution.u.iter().fold(0.0f64, |m, v| m.max(v.abs()));
    assert!(direct.solution.u.iter().zip(&it.solution.u).all(|(a, b)| (a - b).abs() < 1e-7 * scale));
    // One iteration cannot reach the tolerance: the solver fails, the protocol records it and restores the direct solve.
    let bad = model.solve_adaptive(&loads, &bc, &Requirements { force_method: Some(SolveMethod::Iterative { tol: 1e-12, max_iter: 1 }), ..Requirements::default() }).unwrap();
    assert_eq!(bad.method, SolveMethod::Direct);
    assert!(bad.events.iter().any(|e| e.kind == EventKind::StrategyChange && e.detail.contains("reverting")), "{:?}", bad.events);
    assert!(bad.verification.passed);
}

fn plastic_bar() -> (Model, Dirichlet) {
    let mut mesh = grid(Physics::PlaneStress { thickness: 0.5 }, ElementKind::Quad4, Elastic::new(10.0e6, 0.3), [4, 2, 1], &|p| [2.0 * p[0], p[1], 0.0]).unwrap();
    mesh.set_plasticity(0, J2::small(Hardening::linear(40_000.0, 2.0e5, 5.0).unwrap())).unwrap();
    let model = Model::new(mesh).unwrap();
    let mut bc = model.dirichlet();
    for n in 0..model.mesh.nodes.len() {
        let x = model.mesh.nodes[n];
        if x[0] < 1e-12 {
            bc.fix(n, 0, 0.0);
        }
        if x[1] < 1e-12 && x[0] < 1.5 {
            bc.fix(n, 1, 0.0);
        }
        if (x[0] - 2.0).abs() < 1e-12 {
            bc.fix(n, 0, 0.02);
        }
    }
    (model, bc)
}

#[test]
fn the_ladder_escalates_past_a_rung_that_fails_and_records_it() {
    let (model, bc) = plastic_bar();
    let cheap = Rung { name: "single step, 1 iteration".into(), options: NlOptions { steps: 1, max_iter: 1, max_cuts: 0, ..NlOptions::default() } };
    let robust = Rung { name: "default".into(), options: NlOptions::default() };
    let balance = |s: &NlSolution| -> Result<(), String> {
        let sum: f64 = (0..model.mesh.nodes.len()).map(|n| s.reactions[n * 2]).sum();
        let scale: f64 = s.reactions.iter().map(|v| v.abs()).sum::<f64>().max(1e-300);
        if sum.abs() / scale < 1e-6 {
            Ok(())
        } else {
            Err(format!("x-force balance {:.2e}", sum.abs() / scale))
        }
    };
    let l = model.solve_nonlinear_ladder(&Loads::default(), &bc, &[], &[cheap.clone(), robust.clone()], &balance).unwrap();
    assert_eq!(l.rung, 1);
    assert_eq!(l.events.len(), 1);
    assert!(l.events[0].detail.contains("rung 0") && l.events[0].detail.contains("escalating"), "{:?}", l.events);
    // A verifier nothing satisfies exhausts the ladder: an error naming the last reason, never a result.
    let err = model.solve_nonlinear_ladder(&Loads::default(), &bc, &[], &[robust], &|_| Err("never good enough".into())).err().expect("an error");
    assert!(err.contains("never good enough"), "{err}");
}

#[test]
fn an_empty_ladder_is_refused() {
    let (model, bc) = plastic_bar();
    assert!(model.solve_nonlinear_ladder(&Loads::default(), &bc, &[], &[], &|_| Ok(())).is_err());
}

#[test]
fn a_parallel_factorisation_is_bit_for_bit_reproducible() {
    // 3D blocks of this size factor on all threads (`linear.rs::parallel_factor_threshold`); the result must not depend on the
    // scheduling of the threads.
    let mesh = grid(Physics::Solid, ElementKind::Hex20, Elastic::new(1.0e7, 0.3), [7, 7, 7], &|p| p).unwrap();
    let model = Model::new(mesh).unwrap();
    let mut bc = model.dirichlet();
    for nd in 0..model.mesh.nodes.len() {
        if model.mesh.nodes[nd][2] < 1e-12 {
            bc.fix_node(nd);
        }
    }
    let faces = model.mesh.surfaces["w1"].clone();
    let loads = Loads { faces: faces.into_iter().map(|f| (f, SurfaceLoad::Pressure(10.0))).collect(), ..Loads::default() };
    let first = model.solve_static(&loads, &bc).unwrap();
    assert!(first.factor_nnz > 1_000_000, "the test must exercise the parallel path ({})", first.factor_nnz);
    for _ in 0..4 {
        let again = model.solve_static(&loads, &bc).unwrap();
        assert!(first.u.iter().zip(&again.u).all(|(a, b)| a.to_bits() == b.to_bits()), "the solve changed between runs");
    }
}

#[test]
fn the_iterative_path_is_chosen_from_the_factor_size_not_the_dof_count() {
    use fea_core::analysis::{AUTO_ANALYSE_DOFS, AUTO_ITERATIVE_FACTOR_NNZ};
    use fea_core::strategy::iterative_pays_off;
    assert!(!iterative_pays_off(3, 10 * AUTO_ANALYSE_DOFS, AUTO_ITERATIVE_FACTOR_NNZ - 1));
    assert!(iterative_pays_off(3, AUTO_ANALYSE_DOFS, AUTO_ITERATIVE_FACTOR_NNZ));
    assert!(!iterative_pays_off(3, AUTO_ANALYSE_DOFS - 1, 10 * AUTO_ITERATIVE_FACTOR_NNZ), "small systems are not even analysed");
    assert!(!iterative_pays_off(2, 1_000_000, 10 * AUTO_ITERATIVE_FACTOR_NNZ), "2D factors stay cheap");
}
