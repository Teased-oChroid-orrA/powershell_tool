//! Solve reports: the conditioning estimate against a dense eigen oracle, the a-posteriori residual of linear solves,
//! and the decision log / JSON report of the nonlinear driver.

use fea_core::dense::sym_eigen;
use fea_core::generate::grid;
use fea_core::linear::Reduced;
use fea_core::material::J2;
use fea_core::*;
use mechanics_core::hardening::Hardening;

fn clamped_plate(n: usize) -> (Model, Dirichlet) {
    let mesh = grid(Physics::PlaneStress { thickness: 1.0 }, ElementKind::Quad4, Elastic::new(1.0e7, 0.3), [n, n, 1], &|p| [p[0] * 2.0, p[1], 0.0]).unwrap();
    let model = Model::new(mesh).unwrap();
    let mut bc = model.dirichlet();
    for n in 0..model.mesh.nodes.len() {
        if model.mesh.nodes[n][0] < 1e-12 {
            bc.fix_node(n);
        }
    }
    (model, bc)
}

fn dense_reduced(model: &Model, bc: &Dirichlet) -> (Vec<f64>, usize) {
    let k = model.assemble().unwrap();
    let red = Reduced::new(&model.pattern, bc).unwrap();
    let m = red.n_free();
    let (cp, ri, vals) = red.csc(&k);
    let mut a = vec![0.0; m * m];
    for j in 0..m {
        for p in cp[j] as usize..cp[j + 1] as usize {
            let i = ri[p] as usize;
            a[i * m + j] = vals[p];
            a[j * m + i] = vals[p];
        }
    }
    (a, m)
}

#[test]
fn the_condition_estimate_matches_a_dense_eigen_decomposition() {
    let (model, bc) = clamped_plate(4);
    let (a, m) = dense_reduced(&model, &bc);
    let (val, _) = sym_eigen(&a, m);
    let exact = val[m - 1] / val[0];
    let est = model.condition_estimate(&bc, 1e-10, 2000).unwrap();
    assert!(est.converged, "{est:?}");
    assert!((est.cond / exact - 1.0).abs() < 2e-3, "estimate {} vs dense {exact}", est.cond);
    assert!(est.lambda_max <= val[m - 1] * (1.0 + 1e-9) && est.lambda_min >= val[0] * (1.0 - 1e-9), "bounds: {est:?}");
}

#[test]
fn conditioning_worsens_with_refinement_at_the_expected_rate() {
    // A second-order elliptic stiffness matrix has cond ~ h^-2: doubling the divisions quadruples it (up to the aspect ratio).
    let c = |n| {
        let (model, bc) = clamped_plate(n);
        model.condition_estimate(&bc, 1e-8, 5000).unwrap().cond
    };
    let (c4, c8) = (c(4), c(8));
    let ratio = c8 / c4;
    assert!((3.0..5.5).contains(&ratio), "cond ratio {ratio}");
}

#[test]
fn a_linear_solve_reports_its_method_and_a_roundoff_residual() {
    let (model, bc) = clamped_plate(6);
    let n_nodes = model.mesh.nodes.len();
    let loads = Loads { nodal: vec![(n_nodes - 1, [0.0, -1000.0, 0.0])], ..Loads::default() };
    let sol = model.solve_static(&loads, &bc).unwrap();
    assert_eq!(sol.method, SolveMethod::Direct);
    assert!(sol.rel_residual < 1e-10, "{}", sol.rel_residual);
    assert!(sol.factor_nnz > 0);
}

/// Iteration budget small enough that the load steps have to be cut.
const CUT_ITERS: usize = 2;

fn plastic_bar(max_iter: usize) -> NlSolution {
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
            bc.fix(n, 1, 0.0); // a partly supported edge: the field is not uniform, Newton needs several iterations
        }
        if (x[0] - 2.0).abs() < 1e-12 {
            bc.fix(n, 0, 0.02);
        }
    }
    model.solve_nonlinear(&Loads::default(), &bc, &NlOptions { steps: 1, max_iter, ..NlOptions::default() }).unwrap()
}

#[test]
fn every_step_cut_is_logged_and_the_report_is_deterministic() {
    let sol = plastic_bar(CUT_ITERS);
    let cuts: usize = sol.steps.iter().map(|s| s.cuts).sum();
    assert!(cuts > 0, "the test needs a run with cuts");
    assert!(sol.complete());
    assert_eq!(sol.count_events(EventKind::StepCut), cuts);
    assert!(sol.events.iter().all(|e| e.kind != EventKind::StepCut || e.lambda > 0.0 && !e.detail.is_empty()));
    let json = sol.report_json();
    for key in ["\"stop\":\"Completed\"", "\"events\":[{\"kind\":\"step_cut\"", "\"steps\":[", "\"residuals\":["] {
        assert!(json.contains(key), "{key} missing in {json}");
    }
    // Balanced braces / brackets (a cheap well-formedness check; no JSON parser in the dependency set).
    let (mut depth, mut in_str, mut esc) = (0i32, false, false);
    for c in json.chars() {
        if in_str {
            match (esc, c) {
                (true, _) => esc = false,
                (false, '\\') => esc = true,
                (false, '"') => in_str = false,
                _ => {}
            }
        } else {
            match c {
                '"' => in_str = true,
                '{' | '[' => depth += 1,
                '}' | ']' => depth -= 1,
                _ => {}
            }
            assert!(depth >= 0);
        }
    }
    assert_eq!(depth, 0);
    // Same input, same decisions and residual histories (only the times differ).
    let again = plastic_bar(CUT_ITERS).report_json();
    let tail = |s: &str| s[s.find("\"events\"").unwrap()..].to_string();
    assert_eq!(tail(&json), tail(&again));
}

#[test]
fn a_run_without_trouble_has_no_events_and_a_profile() {
    let sol = plastic_bar(25);
    assert!(sol.events.is_empty(), "{:?}", sol.events);
    assert!(sol.profile.factorisation_ms > 0.0 && sol.profile.evaluation_ms > 0.0);
}

#[test]
fn acceptance_reports_never_certify_nonfinite_or_empty_evidence() {
    use fea_core::report::{AcceptanceCheck, AcceptanceReport};
    assert!(!AcceptanceReport::default().passed());
    for value in [f64::NAN, f64::INFINITY, -1.0, 2.0] {
        let report = AcceptanceReport {
            checks: vec![AcceptanceCheck {
                name: "residual".into(),
                value,
                limit: 1.0,
            }],
            diagnostics: vec![],
        };
        assert!(report.require().is_err());
        assert!(report.json().contains("\"passed\":false"));
    }
}

#[test]
fn incomplete_transient_history_fails_unified_acceptance() {
    let result = fea_core::Transient {
        times: vec![0.0, 1.0],
        history: vec![vec![0.0]],
        energy: vec![0.0, 0.0],
        residuals: vec![0.0],
        u: vec![0.0],
        v: vec![0.0],
    };
    let report = fea_core::report::AcceptanceReport::transient(&result, 1e-8, None);
    assert!(report.require().is_err());
}
