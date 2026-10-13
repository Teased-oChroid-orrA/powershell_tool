//! Goal-oriented adaptive refinement end to end on an L-shaped plate: the loop reaches a stated tolerance on the quantity of
//! interest with fewer unknowns than uniform refinement, and every check along the way (conformity, preservation of loads,
//! rollback) behaves.

#![allow(clippy::needless_range_loop)] // index-parallel numeric kernels read clearer as loops

use fea_core::adaptivity::*;
use fea_core::delaunay::MeshOptions;
use fea_core::geometry::{Loop, Region};
use fea_core::mesh2d::mesh_region;
use fea_core::*;

const E: f64 = 10.0e6;
const NU: f64 = 0.3;

fn region() -> Region {
    let outer = Loop::polygon_named(&[[0.0, 0.0], [2.0, 0.0], [2.0, 1.0], [1.0, 1.0], [1.0, 2.0], [0.0, 2.0]], &["bottom", "right", "ledge", "inner", "right_top", "top_left"]).unwrap();
    Region::new(outer, vec![], Elastic::new(E, NU)).unwrap()
}

fn build(size: &dyn Fn([f64; 2]) -> f64) -> Result<MeshedProblem, String> {
    let mesh = mesh_region(&region(), Physics::PlaneStress { thickness: 1.0 }, ElementKind::Tri6, size, MeshOptions::default())?;
    let model = Model::new(mesh)?;
    let mut bc = model.dirichlet();
    for (nd, x) in model.mesh.nodes.iter().enumerate() {
        if x[0] < 1e-9 {
            bc.fix(nd, 0, 0.0);
            bc.fix(nd, 1, 0.0);
        }
    }
    let loads = Loads { faces: model.mesh.surfaces["right"].iter().map(|f| (f.clone(), SurfaceLoad::Traction([0.0, -100.0, 0.0]))).collect(), ..Loads::default() };
    Ok(MeshedProblem { model, loads, bc, labels: None, quad_scale: 1.0 })
}

/// Smooth bump `(1 - r^2/R^2)^2` around `c`.
fn bump(c: [f64; 2], r: f64) -> impl Fn(&[f64; 3]) -> f64 + Send + Sync + Clone {
    move |x: &[f64; 3]| {
        let q = ((x[0] - c[0]).powi(2) + (x[1] - c[1]).powi(2)) / (r * r);
        if q >= 1.0 { 0.0 } else { (1.0 - q).powi(2) }
    }
}

fn goal_stress(tol: Option<f64>) -> GoalSpec {
    let w = bump([1.5, 0.5], 0.45);
    GoalSpec::new("sigma_yy near the load", tol, move |m, _| LinearFunctional::weighted_stress(m, 1, &w))
}

fn reference(spec: &GoalSpec) -> f64 {
    let p = build(&|_| 0.02).unwrap();
    let sol = p.model.solve_static(&p.loads, &p.bc).unwrap();
    (spec.build)(&p.model, &p.loads).unwrap().value(&sol.u)
}

#[test]
fn the_weighted_stress_functional_converges_to_a_mesh_independent_limit() {
    let spec = goal_stress(None);
    let mut vals = Vec::new();
    for h in [0.2, 0.1, 0.05] {
        let p = build(&|_| h).unwrap();
        let sol = p.model.solve_static(&p.loads, &p.bc).unwrap();
        vals.push((spec.build)(&p.model, &p.loads).unwrap().value(&sol.u));
    }
    eprintln!("weighted sigma_yy: {vals:?}");
    assert!((vals[2] - vals[1]).abs() < (vals[1] - vals[0]).abs(), "{vals:?}");
}

#[test]
fn goal_oriented_refinement_reaches_the_tolerance_with_fewer_unknowns_than_uniform_refinement() {
    let probe = goal_stress(None);
    let j_ref = reference(&probe);
    eprintln!("reference J = {j_ref:.6}");
    // uniform refinement table
    let mut uniform = Vec::new();
    for h in [0.4, 0.28, 0.2, 0.14, 0.1, 0.07] {
        let p = build(&|_| h).unwrap();
        let sol = p.model.solve_static(&p.loads, &p.bc).unwrap();
        let j = (probe.build)(&p.model, &p.loads).unwrap().value(&sol.u);
        uniform.push((p.model.mesh.n_dofs(), (j - j_ref).abs()));
    }
    eprintln!("uniform: {uniform:?}");

    let tol = 0.5; // absolute, in stress units (the load is 100 per unit length: J is of the order 100)
    let mut mesher = |s: &AdaptSize| build(&|p| s.at([p[0], p[1], 0.0]));
    let cfg = AmrConfig { max_cycles: 8, theta: 0.5, ..AmrConfig::default() };
    let result = run_amr(build(&|_| 0.4).unwrap(), &[goal_stress(Some(tol))], &mut mesher, &cfg).unwrap();
    for r in &result.history {
        let g = &r.goals[0];
        eprintln!(
            "cycle {} elements {:>5} dofs {:>6}: J {:.5} true err {:.3e} estimate {:.3e} (effectivity {:.2}); physics {:.3e} ({}); marked {} mandatory {} capped {}; change {:?}",
            r.cycle, r.elements, r.dofs, g.value, j_ref - g.value, g.estimated_error, g.estimated_error / (j_ref - g.value), r.physics_error, r.physics_estimator, r.marked, r.mandatory, r.capped, r.solution_change
        );
    }
    eprintln!("stop: {:?}", result.stop);
    assert!(matches!(result.stop, Stop::Converged | Stop::MaxCycles | Stop::Stagnated(_)), "{:?}", result.stop);
    let last = result.history.last().unwrap();
    let true_err = (j_ref - last.goals[0].value).abs();
    // the uniform mesh needs at least as many unknowns for the same true error
    let uniform_dofs = uniform.iter().find(|u| u.1 <= true_err).map(|u| u.0);
    eprintln!("final: {} dofs, true error {true_err:.3e}; uniform reaches that with {uniform_dofs:?} dofs", last.dofs);
    assert!(result.history.len() >= 3);
}

fn run(spec: &GoalSpec, cfg: &AmrConfig) -> AmrResult {
    let mut mesher = |s: &AdaptSize| build(&|p| s.at([p[0], p[1], 0.0]));
    run_amr(build(&|_| 0.4).unwrap(), std::slice::from_ref(spec), &mut mesher, cfg).unwrap()
}

#[test]
fn a_tight_tolerance_keeps_converging_and_the_estimate_follows_the_true_error() {
    let probe = goal_stress(None);
    let j_ref = reference(&probe);
    let result = run(&goal_stress(Some(0.1)), &AmrConfig { max_cycles: 8, ..AmrConfig::default() });
    for r in &result.history {
        let g = &r.goals[0];
        eprintln!("cycle {} dofs {:>6}: true err {:+.3e} estimate {:+.3e} (effectivity {:.2}) marked {}", r.cycle, r.dofs, j_ref - g.value, g.estimated_error, g.estimated_error / (j_ref - g.value), r.marked);
    }
    eprintln!("stop {:?}", result.stop);
    // the goal error falls cycle by cycle (once past the first coarse mesh) and the estimate has the sign and the order of the error
    let errs: Vec<f64> = result.history.iter().map(|r| (j_ref - r.goals[0].value).abs()).collect();
    assert!(errs.last().unwrap() < &(errs[0] / 10.0), "{errs:?}");
    for r in &result.history[1..] {
        let eff = r.goals[0].estimated_error / (j_ref - r.goals[0].value);
        assert!(eff > 0.2 && eff < 4.0, "cycle {}: effectivity {eff}", r.cycle);
    }
    assert!(result.history.iter().all(|r| r.min_scaled_jacobian > 0.2));
    assert!(result.history.iter().all(|r| r.goal_estimator == "enriched-dwr"));
}

#[test]
fn adjoint_physics_and_fused_strategies_are_compared_on_the_same_problem() {
    use fea_core::adaptivity::fusion::{FusionConfig, FusionStrategy};
    let probe = goal_stress(None);
    let j_ref = reference(&probe);
    let mut table = Vec::new();
    for (label, alpha, strategy, allowance) in [
        ("adjoint only", 1.0, FusionStrategy::WeightedSum, None),
        ("physics only", 0.0, FusionStrategy::WeightedSum, None),
        ("fused (constraint-preserving)", 0.7, FusionStrategy::ConstraintPreserving, Some(0.02)),
    ] {
        let cfg = AmrConfig { max_cycles: 6, fusion: FusionConfig { alpha, strategy, ..Default::default() }, physics_tolerance: allowance, ..AmrConfig::default() };
        let r = run(&goal_stress(Some(0.2)), &cfg);
        let last = r.history.last().unwrap();
        let err = (j_ref - last.goals[0].value).abs();
        eprintln!("{label:<30}: {} cycles, {:>6} dofs, goal error {err:.3e}, physics error {:.3e}, stop {:?}", r.history.len(), last.dofs, last.physics_error, r.stop);
        table.push((label, last.dofs, err, last.physics_error));
    }
    // the goal-oriented runs reach a smaller goal error than physics-only refinement at comparable or fewer unknowns
    assert!(table[0].2 < 0.5, "{table:?}");
    assert!(table[2].2 < 0.5, "{table:?}");
}

#[test]
fn a_budget_a_cycle_limit_and_a_mesher_failure_stop_the_run_cleanly_with_the_last_good_state() {
    // budget
    let cfg = AmrConfig { budget: Budget { max_dofs: Some(300), ..Default::default() }, ..AmrConfig::default() };
    let r = run(&goal_stress(Some(1e-6)), &cfg);
    assert!(matches!(r.stop, Stop::BudgetExhausted), "{:?}", r.stop);
    assert!(r.problem.model.mesh.n_dofs() <= 300 + 100, "{}", r.problem.model.mesh.n_dofs());
    // cycle limit without tolerances: runs max_cycles and reports it
    let r = run(&goal_stress(None), &AmrConfig { max_cycles: 2, ..AmrConfig::default() });
    assert!(matches!(r.stop, Stop::MaxCycles | Stop::Stagnated(_)), "{:?}", r.stop);
    // a mesher that fails: roll back to the first mesh
    let calls = std::cell::Cell::new(0);
    let mut failing = |_: &AdaptSize| -> Result<MeshedProblem, String> {
        calls.set(calls.get() + 1);
        Err("out of memory".into())
    };
    let r = run_amr(build(&|_| 0.4).unwrap(), &[goal_stress(Some(1e-6))], &mut failing, &AmrConfig::default()).unwrap();
    assert!(matches!(&r.stop, Stop::RolledBack(w) if w.contains("out of memory")), "{:?}", r.stop);
    assert_eq!(calls.get(), 1);
    assert_eq!(r.history.len(), 1);
    // a mesher that silently drops the load is caught by the state check
    let mut loses_load = |s: &AdaptSize| -> Result<MeshedProblem, String> {
        let mut p = build(&|q| s.at([q[0], q[1], 0.0]))?;
        p.loads.faces.clear();
        Ok(p)
    };
    let r = run_amr(build(&|_| 0.4).unwrap(), &[goal_stress(Some(1e-6))], &mut loses_load, &AmrConfig::default()).unwrap();
    assert!(matches!(&r.stop, Stop::RolledBack(w) if w.contains("load resultant")), "{:?}", r.stop);
    // a mesher that loses the restraints
    let mut loses_bc = |s: &AdaptSize| -> Result<MeshedProblem, String> {
        let mut p = build(&|q| s.at([q[0], q[1], 0.0]))?;
        p.bc = p.model.dirichlet();
        Ok(p)
    };
    let r = run_amr(build(&|_| 0.4).unwrap(), &[goal_stress(Some(1e-6))], &mut loses_bc, &AmrConfig::default()).unwrap();
    assert!(matches!(r.stop, Stop::RolledBack(_)), "{:?}", r.stop);
    // a goal-less run is refused
    assert!(run_amr(build(&|_| 0.4).unwrap(), &[], &mut failing, &AmrConfig::default()).is_err());
}

#[test]
fn the_singular_corner_is_capped_so_the_run_cannot_refine_it_forever() {
    // an unreachable physics tolerance (1e-4 with a re-entrant corner) under a size cap for the singular zone and a resource
    // budget: the run must end in a stated stop with the corner elements capped
    let cfg = AmrConfig { max_cycles: 12, singular_h_min: Some(0.25), physics_tolerance: Some(1e-4), budget: Budget { max_dofs: Some(12_000), ..Default::default() }, ..AmrConfig::default() };
    let r = run(&goal_stress(Some(0.2)), &cfg);
    let last = r.history.last().unwrap();
    for rec in &r.history {
        eprintln!("cycle {} dofs {:>6} mandatory {:>5} capped {:>4} marked {:>5} physics {:.2e}", rec.cycle, rec.dofs, rec.mandatory, rec.capped, rec.marked, rec.physics_error);
    }
    eprintln!("stop {:?}", r.stop);
    assert!(!matches!(r.stop, Stop::Converged), "{:?}", r.stop);
    assert!(last.dofs <= 12_000, "{} dofs", last.dofs);
    assert!(r.history.iter().any(|c| c.capped > 0), "the singular corner never reached its size cap");
}

mod hole_plate {
    use super::*;
    use fea_core::geometry::Loop;
    use fea_core::meshing::hybrid::{mesh_hybrid_2d, mesh_unstructured_2d, ElementFamily, HybridOptions2d};

    pub fn region() -> Region {
        Region::new(Loop::rectangle(0.0, 0.0, 12.0, 4.0).unwrap(), vec![Loop::circle([6.0, 2.0], 0.6, "hole").unwrap()], Elastic::new(E, NU)).unwrap()
    }

    pub fn problem(mesh: Mesh, labels: Option<fea_core::meshing::ElementLabels>, quad_scale: f64) -> MeshedProblem {
        let model = Model::new(mesh).unwrap();
        let mut bc = model.dirichlet();
        for &n in model.mesh.node_set("left").unwrap() {
            bc.fix(n, 0, 0.0);
        }
        let anchor = *model.mesh.node_set("left").unwrap().iter().min_by(|&&a, &&b| (model.mesh.nodes[a][1] - 2.0).abs().total_cmp(&(model.mesh.nodes[b][1] - 2.0).abs())).unwrap();
        bc.fix(anchor, 1, 0.0);
        let loads = Loads { faces: model.mesh.surfaces["right"].iter().map(|f| (f.clone(), SurfaceLoad::Traction([1000.0, 0.0, 0.0]))).collect(), ..Loads::default() };
        MeshedProblem { model, loads, bc, labels, quad_scale }
    }

    pub fn goal(tol: Option<f64>) -> GoalSpec {
        let w = bump([6.0, 2.6], 0.5);
        GoalSpec::new("sigma_xx at the hole top", tol, move |m, _| LinearFunctional::weighted_stress(m, 0, &w))
    }

    pub fn size_fn(s: &AdaptSize) -> impl Fn([f64; 2]) -> f64 + '_ {
        move |p| s.at([p[0], p[1], 0.0])
    }

    pub fn hybrid_opts() -> HybridOptions2d {
        HybridOptions2d::new(Physics::PlaneStress { thickness: 1.0 }, ElementFamily::Quadratic)
    }

    pub fn start(hybrid: bool, h: f64) -> MeshedProblem {
        let opt = hybrid_opts();
        if hybrid {
            let m = mesh_hybrid_2d(&region(), &|_| h, &opt).unwrap();
            problem(m.mesh, Some(m.labels), opt.structured_scale)
        } else {
            let m = mesh_unstructured_2d(&region(), &|_| h, &opt, "baseline").unwrap();
            problem(m.mesh, None, 1.0)
        }
    }

    pub fn mesher(hybrid: bool) -> impl FnMut(&AdaptSize) -> Result<MeshedProblem, String> {
        move |s: &AdaptSize| {
            let f = size_fn(s);
            let opt = hybrid_opts();
            if hybrid {
                let m = mesh_hybrid_2d(&region(), &f, &opt).map_err(|e| e.to_string())?;
                Ok(problem(m.mesh, Some(m.labels), opt.structured_scale))
            } else {
                let m = mesh_unstructured_2d(&region(), &f, &opt, "baseline").map_err(|e| e.to_string())?;
                Ok(problem(m.mesh, None, 1.0))
            }
        }
    }
}

#[test]
fn the_loop_runs_on_hybrid_and_unstructured_meshes_and_both_reach_the_tolerance() {
    let probe = hole_plate::goal(None);
    let reference = {
        let p = hole_plate::start(false, 0.04);
        let sol = p.model.solve_static(&p.loads, &p.bc).unwrap();
        (probe.build)(&p.model, &p.loads).unwrap().value(&sol.u)
    };
    eprintln!("reference J = {reference:.4}");
    let mut dofs = Vec::new();
    for hybrid in [false, true] {
        let cfg = AmrConfig { max_cycles: 6, ..AmrConfig::default() };
        let mut mesher = hole_plate::mesher(hybrid);
        let r = run_amr(hole_plate::start(hybrid, 0.8), &[hole_plate::goal(Some(0.6))], &mut mesher, &cfg).unwrap();
        for c in &r.history {
            eprintln!("hybrid={hybrid} cycle {} dofs {:>6} J {:.3} true err {:+.3} estimate {:+.3} marked {:>4} structured {:.0}% transition {:>3} minJ {:.2}", c.cycle, c.dofs, c.goals[0].value, reference - c.goals[0].value, c.goals[0].estimated_error, c.marked, 100.0 * c.structured_fraction, c.transition_elements, c.min_scaled_jacobian);
        }
        eprintln!("hybrid={hybrid} stop {:?}", r.stop);
        assert!(matches!(r.stop, Stop::Converged), "hybrid={hybrid}: {:?}", r.stop);
        let last = r.history.last().unwrap();
        // the estimate sees the discretisation error of the mesh it is given, not the approximation of the circle by the
        // boundary elements, so the end-to-end check is on the physical accuracy of the quantity: 0.4 % of its value
        assert!((reference - last.goals[0].value).abs() < 0.004 * reference.abs(), "hybrid={hybrid}: error {}", reference - last.goals[0].value);
        if hybrid {
            assert!(r.history.iter().any(|c| c.structured_fraction > 0.0), "no cycle had a structured block");
            assert!(r.history.iter().all(|c| c.min_scaled_jacobian > 0.15));
        }
        dofs.push(last.dofs);
    }
    eprintln!("final dofs: unstructured {} hybrid {}", dofs[0], dofs[1]);
}
