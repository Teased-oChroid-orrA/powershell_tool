//! Stage-by-stage timings of the hybrid meshing and goal-oriented adaptivity machinery on one machine
//! (`cargo test -p fea-core --release --test amr_bench -- --ignored --nocapture`; wrap with `rtk proxy` to see the output).
//! The first test is also a regression gate on quantities that are deterministic (sizes and counts, not times).

#![allow(clippy::needless_range_loop)] // index-parallel numeric kernels read clearer as loops

use fea_core::adaptivity::dwr::{solve_duals, DwrEstimator};
use fea_core::adaptivity::equilibrated::reconstruct;
use fea_core::adaptivity::*;
use fea_core::delaunay::MeshOptions;
use fea_core::geometry::{Loop, Region};
use fea_core::meshing::hybrid::{mesh_hybrid_2d, mesh_unstructured_2d, ElementFamily, HybridOptions2d};
use fea_core::mesh2d::mesh_region;
use fea_core::*;
use std::time::Instant;

const PHYS: Physics = Physics::PlaneStress { thickness: 1.0 };

fn region() -> Region {
    Region::new(Loop::rectangle(0.0, 0.0, 12.0, 4.0).unwrap(), vec![Loop::circle([6.0, 2.0], 0.6, "hole").unwrap()], Elastic::new(1.0e7, 0.3)).unwrap()
}

fn tension(mesh: Mesh) -> (Model, Loads, Dirichlet) {
    let model = Model::new(mesh).unwrap();
    let mut bc = model.dirichlet();
    for &n in model.mesh.node_set("left").unwrap() {
        bc.fix(n, 0, 0.0);
    }
    let anchor = *model.mesh.node_set("left").unwrap().iter().min_by(|&&a, &&b| (model.mesh.nodes[a][1] - 2.0).abs().total_cmp(&(model.mesh.nodes[b][1] - 2.0).abs())).unwrap();
    bc.fix(anchor, 1, 0.0);
    let loads = Loads { faces: model.mesh.surfaces["right"].iter().map(|f| (f.clone(), SurfaceLoad::Traction([1000.0, 0.0, 0.0]))).collect(), ..Loads::default() };
    (model, loads, bc)
}

fn time<T>(f: impl FnOnce() -> T) -> (T, f64) {
    let t = Instant::now();
    let r = f();
    (r, 1e3 * t.elapsed().as_secs_f64())
}

#[test]
#[ignore = "timings: run with --release --ignored --nocapture"]
fn stage_profile_of_one_estimate_on_a_20k_dof_plate() {
    let size = |p: [f64; 2]| (0.04 + 0.15 * ((p[0] - 6.0).hypot(p[1] - 2.0) - 0.6).max(0.0)).min(0.5);
    let opt = HybridOptions2d::new(PHYS, ElementFamily::Quadratic);
    let (base, t_unst) = time(|| mesh_unstructured_2d(&region(), &size, &opt, "baseline").unwrap());
    let (hyb, t_hyb) = time(|| mesh_hybrid_2d(&region(), &size, &opt).unwrap());
    eprintln!("mesh generation: unstructured {t_unst:.1} ms ({} dofs), hybrid {t_hyb:.1} ms ({} dofs, {:.0} % structured elements)", base.mesh.n_dofs(), hyb.mesh.n_dofs(), 100.0 * hyb.report.structured_elements as f64 / hyb.mesh.n_elems() as f64);
    let (model, loads, bc) = tension(base.mesh.clone());
    let ((), t_adj) = time(|| {
        let _ = model.mesh.adjacency();
    });
    let (sol, t_solve) = time(|| model.solve_static(&loads, &bc).unwrap());
    eprintln!("solve ({} dofs, factor nnz {}): total {t_solve:.1} ms = assemble {:.1} + symbolic {:.1} + factor {:.1} + back-solve {:.1}; adjacency {t_adj:.1} ms", model.mesh.n_dofs(), sol.factor_nnz, sol.timings.assemble_ms, sol.timings.symbolic_ms, sol.timings.factor_ms, sol.timings.solve_ms);
    let adj = model.mesh.adjacency();
    let ctx = EstimatorContext::new(&model, &loads, &bc, &sol.u, &adj).unwrap();
    let w = bump_goal(&model);
    let (_, t_zz) = time(|| model.zz_error(&sol.u, 0.0).unwrap());
    let (gauss, t_gauss) = time(|| model.gauss_stresses(&sol.u, 0.0).unwrap());
    let (_, t_spr) = time(|| model.recover_spr_by_material(&gauss).unwrap());
    let (_, t_eq) = time(|| reconstruct(&ctx).unwrap());
    let (_, t_dual) = time(|| solve_duals(&ctx, &[&w]).unwrap());
    let (_, t_rdwr) = time(|| DwrEstimator.estimate_goals(&ctx, &[&w]).unwrap());
    let builder = |m: &Model, _: &Loads| Ok(bump_goal(m));
    let (_, t_edwr) = time(|| EnrichedDwrEstimator.estimate_goals(&ctx, &[&w], &[&builder]).unwrap());
    let (rf, t_refine) = time(|| fea_core::adaptivity::enrich::refine_uniform_2d(&model.mesh).unwrap());
    eprintln!("estimators: gauss stresses {t_gauss:.1} ms, SPR+ {t_spr:.1}, ZZ (gauss + SPR + error rule) {t_zz:.1}, equilibrated reconstruction {t_eq:.1}");
    eprintln!("            coarse dual solve {t_dual:.1} ms, recovered-stress DWR {t_rdwr:.1} ms, enriched DWR {t_edwr:.1} ms (of which uniform refinement {t_refine:.1} ms to {} dofs)", rf.mesh.n_dofs());
    let cost = fea_core::meshing::cost::MeshCost::of(&model.mesh).unwrap();
    let (_, t_cost) = time(|| fea_core::meshing::cost::MeshCost::of(&model.mesh).unwrap());
    eprintln!("cost model prediction {t_cost:.1} ms: factor nnz {} ({:.1} MB), assembly units {:.2e}", cost.factor_nnz, 8e-6 * cost.factor_nnz as f64, cost.assembly_units);
    let snap = SolverSnapshot::new(&model.mesh, sol.u.clone(), 0.0, "bench").unwrap();
    let (fine_model, ..) = tension(mesh_unstructured_2d(&region(), &|p| size(p) * 0.8, &opt, "baseline").unwrap().mesh);
    let (_, t_transfer) = time(|| NodalInterpolation { max_unlocated_fraction: 0.2 }.transfer(&model, &snap, &fine_model.mesh).unwrap());
    eprintln!("state transfer to a {} dof mesh: {t_transfer:.1} ms", fine_model.mesh.n_dofs());
    // one controller cycle, broken down by its own stage clocks
    let goals = [GoalSpec::new("w", None, |m, _| Ok(bump_goal(m)))];
    let first = MeshedProblem { model: model.clone(), loads: loads.clone(), bc: bc.clone(), labels: None, quad_scale: 1.0 };
    let mut mesher = |s: &AdaptSize| -> Result<MeshedProblem, String> {
        let m = mesh_unstructured_2d(&region(), &|p| s.at([p[0], p[1], 0.0]), &opt, "baseline").map_err(|e| e.to_string())?;
        let (model, loads, bc) = tension(m.mesh);
        Ok(MeshedProblem { model, loads, bc, labels: None, quad_scale: 1.0 })
    };
    let (r, t_cycle) = time(|| run_amr(first, &goals, &mut mesher, &AmrConfig { max_cycles: 1, ..AmrConfig::default() }).unwrap());
    let s = &r.history[0].seconds;
    eprintln!("one controller cycle (+ final evaluation) {t_cycle:.0} ms; cycle 0 stages: solve {:.1} ms, goals {:.1}, physics {:.1}, fusion {:.1}, marking+sizing {:.1}, meshing {:.1}, checks {:.1}", 1e3 * s.solve, 1e3 * s.goals, 1e3 * s.physics, 1e3 * s.fusion, 1e3 * s.marking_and_sizing, 1e3 * s.meshing, 1e3 * s.checks);
}

fn bump_goal(m: &Model) -> LinearFunctional {
    let w = |x: &[f64; 3]| {
        let q = ((x[0] - 6.0).powi(2) + (x[1] - 2.6).powi(2)) / 0.25;
        if q >= 1.0 { 0.0 } else { (1.0 - q).powi(2) }
    };
    LinearFunctional::weighted_stress(m, 0, &w).unwrap()
}

#[test]
fn the_adaptive_run_on_the_l_shape_is_deterministic_in_its_counts() {
    // The L-shape of `tests/amr.rs` with the same configuration must give the same cycle sizes every time (a change in the
    // estimators, the marking, the sizing or the meshers moves them; update the expected numbers knowingly, with the evidence).
    let region = Region::new(Loop::polygon_named(&[[0.0, 0.0], [2.0, 0.0], [2.0, 1.0], [1.0, 1.0], [1.0, 2.0], [0.0, 2.0]], &["bottom", "right", "ledge", "inner", "right_top", "top_left"]).unwrap(), vec![], Elastic::new(1.0e7, 0.3)).unwrap();
    let build = |size: &dyn Fn([f64; 2]) -> f64| -> Result<MeshedProblem, String> {
        let mesh = mesh_region(&region, PHYS, ElementKind::Tri6, size, MeshOptions::default())?;
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
    };
    let goal = GoalSpec::new("sigma_yy", Some(0.5), |m, _| {
        let w = |x: &[f64; 3]| {
            let q = ((x[0] - 1.5).powi(2) + (x[1] - 0.5).powi(2)) / (0.45 * 0.45);
            if q >= 1.0 { 0.0 } else { (1.0 - q).powi(2) }
        };
        LinearFunctional::weighted_stress(m, 1, &w)
    });
    let run = || {
        let mut mesher = |s: &AdaptSize| build(&|p| s.at([p[0], p[1], 0.0]));
        run_amr(build(&|_| 0.4).unwrap(), std::slice::from_ref(&goal), &mut mesher, &AmrConfig::default()).unwrap()
    };
    let (a, b) = (run(), run());
    let sizes = |r: &AmrResult| r.history.iter().map(|c| c.dofs).collect::<Vec<_>>();
    assert_eq!(sizes(&a), sizes(&b), "two runs must be identical");
    let one = fea_core::sequentially(run);
    assert_eq!(sizes(&a), sizes(&one), "the run must not depend on the thread count");
    assert_eq!(a.history.iter().map(|c| c.goals[0].value.to_bits()).collect::<Vec<_>>(), one.history.iter().map(|c| c.goals[0].value.to_bits()).collect::<Vec<_>>());
    assert_eq!(a.history.iter().map(|c| c.goals[0].estimated_error.to_bits()).collect::<Vec<_>>(), one.history.iter().map(|c| c.goals[0].estimated_error.to_bits()).collect::<Vec<_>>());
    // recorded 2026-10-13: 270, 472, 892 dofs (see docs/hybrid-meshing-amr-evidence.md, Phase 8); a deliberate change updates this
    let got = sizes(&a);
    let want = [270usize, 472, 892];
    assert_eq!(got.len(), want.len(), "{got:?}");
    for (g, w) in got.iter().zip(want) {
        assert!((*g as f64 / w as f64 - 1.0).abs() < 0.15, "{got:?} against {want:?}");
    }
}

#[test]
#[ignore = "timings: run with --release --ignored --nocapture"]
fn estimator_cost_against_problem_size() {
    let opt = HybridOptions2d::new(PHYS, ElementFamily::Quadratic);
    eprintln!("{:>8} {:>9} {:>8} {:>8} {:>8} {:>9} {:>9} {:>9}", "dofs", "elements", "solve", "ZZ", "equil.", "DWR-rec", "DWR-enr", "eigen-1");
    for scale in [2.0f64, 1.0, 0.5] {
        let size = move |p: [f64; 2]| scale * (0.08 + 0.3 * ((p[0] - 6.0).hypot(p[1] - 2.0) - 0.6).max(0.0)).min(0.8);
        let m = mesh_unstructured_2d(&region(), &size, &opt, "baseline").unwrap();
        let (model, loads, bc) = tension(m.mesh);
        let (sol, t_solve) = time(|| model.solve_static(&loads, &bc).unwrap());
        let adj = model.mesh.adjacency();
        let ctx = EstimatorContext::new(&model, &loads, &bc, &sol.u, &adj).unwrap();
        let w = bump_goal(&model);
        let (_, t_zz) = time(|| model.zz_error(&sol.u, 0.0).unwrap());
        let (_, t_eq) = time(|| reconstruct(&ctx).unwrap());
        let (_, t_rdwr) = time(|| DwrEstimator.estimate_goals(&ctx, &[&w]).unwrap());
        let builder = |m: &Model, _: &Loads| Ok(bump_goal(m));
        let (_, t_edwr) = time(|| EnrichedDwrEstimator.estimate_goals(&ctx, &[&w], &[&builder]).unwrap());
        // one eigenvalue estimate: the lowest mode of the same plate with density
        let mut meshd = model.mesh.clone();
        meshd.set_density_all(1.0e-3).unwrap();
        let md = Model::new(meshd).unwrap();
        let mut bcd = md.dirichlet();
        for &n in md.mesh.node_set("left").unwrap() {
            bcd.fix_node(n);
        }
        let (modes, t_modal) = time(|| md.modal(&bcd, &ModalOptions { n_modes: 1, ..Default::default() }).unwrap().modes);
        let (_, t_eigen_est) = time(|| estimate_modes(&md, &modes, 0, 0).unwrap());
        eprintln!("{:>8} {:>9} {:>7.1}  {:>7.1}  {:>7.1}  {:>8.1}  {:>8.1}  {:>8.1} (modal solve {t_modal:.0} ms)", model.mesh.n_dofs(), model.mesh.n_elems(), t_solve, t_zz, t_eq, t_rdwr, t_edwr, t_eigen_est);
    }
}
