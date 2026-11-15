//! Adaptive refinement of natural frequencies: the leading-order eigenvalue error estimate against closed forms, repeated
//! eigenvalues, mode tracking across remeshing and the stop when a mode cannot be followed.

use fea_core::adaptivity::*;
use fea_core::delaunay::MeshOptions;
use fea_core::geometry::{Loop, Region};
use fea_core::mesh2d::mesh_region;
use fea_core::*;
use std::f64::consts::PI;

const PHYS: Physics = Physics::PlaneStress { thickness: 1.0 };

fn rect(w: f64, h: f64) -> Region {
    Region::new(Loop::rectangle(0.0, 0.0, w, h).unwrap(), vec![], Elastic::new(1.0, 0.0)).unwrap()
}

/// Axial bar of length 1 (E = rho = 1, nu = 0): only axial motion, fixed at x = 0. omega_n = (2n - 1) pi / 2.
fn axial_bar(size: &dyn Fn([f64; 2]) -> f64) -> Result<MeshedProblem, String> {
    let mut mesh = mesh_region(&rect(1.0, 0.25), PHYS, ElementKind::Tri6, size, MeshOptions::default())?;
    mesh.set_density_all(1.0)?;
    let model = Model::new(mesh)?;
    let mut bc = model.dirichlet();
    for (n, x) in model.mesh.nodes.iter().enumerate() {
        bc.fix(n, 1, 0.0);
        if x[0] < 1e-12 {
            bc.fix(n, 0, 0.0);
        }
    }
    Ok(MeshedProblem { model, loads: Loads::default(), bc, labels: None, quad_scale: 1.0 })
}

fn exact_w2(n: usize) -> f64 {
    ((2 * n + 1) as f64 * PI / 2.0).powi(2)
}

#[test]
fn the_leading_order_eigenvalue_error_estimate_follows_the_true_error_of_an_axial_bar() {
    for h in [0.4, 0.2, 0.1] {
        let p = axial_bar(&|_| h).unwrap();
        let modes = p.model.modal(&p.bc, &ModalOptions { n_modes: 3, ..Default::default() }).unwrap().modes;
        let est = estimate_modes(&p.model, &modes, 0, 0).unwrap();
        let true_err = modes[0].omega2 - exact_w2(0);
        eprintln!("h={h}: omega2 {:.6} exact {:.6} true error {true_err:.3e} estimate {:.3e} effectivity {:.2}", modes[0].omega2, exact_w2(0), est.error, est.error / true_err);
        assert!(true_err > 0.0, "a conforming mesh with a consistent mass converges from above");
        assert!(est.reliability == Reliability::Reliable);
        let eff = est.error / true_err;
        assert!(eff > 0.8 && eff < 1.6, "h={h}: effectivity {eff}");
        if h <= 0.1 {
            assert!((eff - 1.0).abs() < 0.1, "h={h}: asymptotic effectivity {eff}");
        }
        // the corrected value is closer to the exact one
        assert!((est.corrected - exact_w2(0)).abs() < true_err.abs());
    }
}

#[test]
fn the_adaptive_loop_reaches_a_frequency_tolerance_and_keeps_following_the_same_mode() {
    let mut mesher = |s: &AdaptSize| axial_bar(&|p| s.at([p[0], p[1], 0.0]));
    // follow the second axial mode (index 1)
    let cfg = EigenConfig { modes: vec![1], rel_tol: 2e-4, max_cycles: 10, ..EigenConfig::default() };
    let r = run_eigen_amr(axial_bar(&|_| 0.3).unwrap(), &mut mesher, &cfg).unwrap();
    for c in &r.history {
        eprintln!("cycle {} dofs {:>5}: followed {:?} omega2 {:.6} (exact {:.6}) estimate {:.2e} MAC {:?} marked {}", c.cycle, c.dofs, c.tracked, c.omega2[0], exact_w2(1), c.relative_error[0], c.mac, c.marked);
    }
    assert_eq!(r.stop, EigenStop::Converged, "{:?}", r.stop);
    let last = r.history.last().unwrap();
    let true_rel = (last.omega2[0] - exact_w2(1)).abs() / exact_w2(1);
    assert!(true_rel < 1e-3, "true relative error {true_rel}");
    // every cycle followed the mode with the same character: the eigenvalue stays near the second axial one
    assert!(r.history.iter().all(|c| (c.omega2[0] / exact_w2(1) - 1.0).abs() < 0.2));
    assert!(r.history.iter().skip(1).all(|c| c.mac.iter().all(|&m| m > 0.8)));
}

#[test]
fn repeated_eigenvalues_are_a_cluster_with_a_degraded_estimate() {
    // A square with rollers on all four sides and nu = 0: u_x = sin(pi x) and u_y = sin(pi y) share one eigenvalue.
    let build = |size: &dyn Fn([f64; 2]) -> f64| -> Result<MeshedProblem, String> {
        let mut mesh = mesh_region(&rect(1.0, 1.0), PHYS, ElementKind::Quad8, size, MeshOptions::default())?;
        mesh.set_density_all(1.0)?;
        let model = Model::new(mesh)?;
        let mut bc = model.dirichlet();
        for (n, x) in model.mesh.nodes.iter().enumerate() {
            if x[0] < 1e-12 || x[0] > 1.0 - 1e-12 {
                bc.fix(n, 0, 0.0);
            }
            if x[1] < 1e-12 || x[1] > 1.0 - 1e-12 {
                bc.fix(n, 1, 0.0);
            }
        }
        Ok(MeshedProblem { model, loads: Loads::default(), bc, labels: None, quad_scale: 1.0 })
    };
    let p = build(&|_| 0.1).unwrap();
    let modes = p.model.modal(&p.bc, &ModalOptions { n_modes: 5, ..Default::default() }).unwrap().modes;
    let w: Vec<f64> = modes.iter().map(|m| m.omega2).collect();
    eprintln!("lowest eigenvalues: {w:?} (pi^2 = {:.5})", PI * PI);
    let cl = clusters(&w, 1e-3);
    // u_x = sin(pi x), u_y = sin(pi y) and the coupled mode share the eigenvalue pi^2 (nu = 0): a cluster of at least two
    assert!(cl[0].0 == 0 && cl[0].1 >= 1, "the lowest modes are a repeated eigenvalue pi^2: {w:?}");
    let est = estimate_modes(&p.model, &modes, cl[0].0, cl[0].1).unwrap();
    assert!(est.modes.len() >= 2);
    assert!(matches!(est.reliability, Reliability::Degraded(ref why) if why.contains("cluster")), "{:?}", est.reliability);
    // following one member of the cluster follows the whole cluster
    let mut mesher = |s: &AdaptSize| build(&|p| s.at([p[0], p[1], 0.0]));
    let cfg = EigenConfig { modes: vec![0], rel_tol: 5e-3, max_cycles: 6, cluster_tol: 5e-3, ..EigenConfig::default() };
    let r = run_eigen_amr(build(&|_| 0.3).unwrap(), &mut mesher, &cfg).unwrap();
    for c in &r.history {
        eprintln!("cycle {} dofs {:>5}: clusters {:?} omega2 {:.5} estimate {:.2e}", c.cycle, c.dofs, c.clusters, c.omega2[0], c.relative_error[0]);
    }
    assert!(r.history.iter().all(|c| c.clusters.first().is_some_and(|g| g.0 == 0 && g.1 >= 1)), "the cluster must be followed as a unit");
    assert!(matches!(r.stop, EigenStop::Converged | EigenStop::MaxCycles), "{:?}", r.stop);
}

#[test]
fn a_mode_that_cannot_be_followed_stops_the_run() {
    let calls = std::cell::Cell::new(0);
    // after the first remesh the bar is clamped at both ends: the followed mode no longer exists in that form
    let mut changes_the_physics = |s: &AdaptSize| -> Result<MeshedProblem, String> {
        calls.set(calls.get() + 1);
        let mut p = axial_bar(&|q| s.at([q[0], q[1], 0.0]))?;
        for (n, x) in p.model.mesh.nodes.iter().enumerate() {
            if x[0] > 1.0 - 1e-12 {
                p.bc.fix(n, 0, 0.0);
            }
        }
        Ok(p)
    };
    let cfg = EigenConfig { modes: vec![0], rel_tol: 1e-9, mac_min: 0.95, ..EigenConfig::default() };
    let r = run_eigen_amr(axial_bar(&|_| 0.3).unwrap(), &mut changes_the_physics, &cfg).unwrap();
    assert!(matches!(r.stop, EigenStop::ModeLost(_)), "{:?}", r.stop);
    assert_eq!(calls.get(), 1);
    assert!(run_eigen_amr(axial_bar(&|_| 0.3).unwrap(), &mut changes_the_physics, &EigenConfig { modes: vec![], ..EigenConfig::default() }).is_err());
}
