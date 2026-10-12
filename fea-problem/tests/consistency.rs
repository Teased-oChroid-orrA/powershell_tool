//! The linear solve of a 45k-dof Hex27 model must be self-consistent however it is computed: the stiffness matrix must
//! annihilate a rigid translation, every method must satisfy `K u = f`, and the methods must agree. This model once solved
//! to a wrong answer (reactions 12 % short of the loads) on some hosted Windows runners and correctly everywhere else
//! (docs/issue-12-phase-26.md); the test prints what it measured so a bad host shows where the numbers part.
use fea_core::{Model, SolveMethod};
use fea_problem::solve::{apply_supports, build_loads};
use fea_problem::{build, templates, ElementChoice};

fn norm(v: &[f64]) -> f64 {
    v.iter().map(|x| x * x).sum::<f64>().sqrt()
}

#[test]
fn every_way_of_solving_the_large_hex27_model_agrees() {
    let mut p = templates::templates().into_iter().find(|(n, _)| *n == "Extruded plate with a hole (3D)").unwrap().1;
    p.mesh.element = ElementChoice::Quad9;
    p.mesh.size = 0.50737531894725;
    let mut mesh = build::sketch_mesh(&p, None).unwrap();
    let bc = apply_supports(&mesh, &p.supports).unwrap();
    let loads = build_loads(&mut mesh, &p).unwrap();
    let model = Model::new(mesh).unwrap();
    let n = model.mesh.n_dofs();

    // 1. Rigid translation: K t = 0 (assembly intact), in x, y and z.
    let k = model.assemble().unwrap();
    let diag = k.diagonal(&model.pattern);
    let scale = diag.iter().fold(0.0f64, |m, v| m.max(v.abs()));
    for axis in 0..3 {
        let t: Vec<f64> = (0..n).map(|i| if i % 3 == axis { 1.0 } else { 0.0 }).collect();
        let mut y = vec![0.0; n];
        k.matvec_add(&model.pattern, &t, &mut y);
        let worst = y.iter().fold(0.0f64, |m, v| m.max(v.abs())) / scale;
        eprintln!("K * translation({axis}): largest entry {worst:.2e} of the largest diagonal");
        assert!(worst < 1e-9, "the assembled matrix does not annihilate a translation along {axis}: {worst:e}");
    }

    // 2. Each method satisfies its equations and balances the loads.
    let f = fea_core::loads::assemble(&model.mesh, &loads).unwrap();
    let direct = model.solve_static_with(&loads, &bc, SolveMethod::Direct).unwrap();
    let sequential = fea_core::sequentially(|| model.solve_static_with(&loads, &bc, SolveMethod::Direct)).unwrap();
    let iterative = model.solve_static_with(&loads, &bc, SolveMethod::iterative()).unwrap();
    for (label, sol) in [("direct", &direct), ("direct on one thread", &sequential), ("iterative", &iterative)] {
        let (mut r, mut a) = ([0.0; 3], [0.0; 3]);
        for i in 0..n {
            r[i % 3] += sol.reactions[i];
            a[i % 3] += f[i];
        }
        let imbalance = ((0..3).map(|c| (r[c] + a[c]).powi(2)).sum::<f64>()).sqrt() / a[0].abs().max(1e-300);
        eprintln!("{label}: residual {:.2e}, force imbalance {imbalance:.2e}, reaction {:?}", sol.rel_residual, r);
        assert!(sol.rel_residual < 1e-6, "{label}: K u = f fails by {:e}", sol.rel_residual);
        assert!(imbalance < 1e-6, "{label}: the reactions miss the loads by {imbalance:e}");
    }

    // 3. The displacement fields agree.
    let reference = norm(&direct.u);
    for (label, sol) in [("one thread", &sequential), ("iterative", &iterative)] {
        let diff: Vec<f64> = direct.u.iter().zip(&sol.u).map(|(a, b)| a - b).collect();
        let rel = norm(&diff) / reference;
        eprintln!("direct vs {label}: relative difference {rel:.2e}");
        assert!(rel < 1e-5, "direct and {label} disagree by {rel:e}");
    }
}
