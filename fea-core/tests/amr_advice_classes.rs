//! Does starting-mesh advice (`adaptivity::feedback`) reduce time-to-accuracy? Measured on a class where the demand is
//! localised away from the default start (a plate in tension with a circular hole: stress concentration at the hole edge) and
//! on a smooth control (the same plate without the hole, where there is nothing to advise). Reference value: a fine uniform
//! mesh. `docs/issue-46-benchmarks.md` section D; the measurements are in `docs/issue-46-phase-80.md`.

use fea_core::adaptivity::*;
use fea_core::delaunay::MeshOptions;
use fea_core::geometry::{Loop, Region};
use fea_core::mesh2d::mesh_region;
use fea_core::*;

const PHYS: Physics = Physics::PlaneStress { thickness: 1.0 };
const START: f64 = 1.0;
const HOLE: [f64; 2] = [6.0, 2.0];

fn region(hole: bool) -> Region {
    let holes = if hole {
        vec![Loop::circle(HOLE, 0.6, "hole").unwrap()]
    } else {
        vec![]
    };
    Region::new(
        Loop::rectangle(0.0, 0.0, 12.0, 4.0).unwrap(),
        holes,
        Elastic::new(1.0e7, 0.3),
    )
    .unwrap()
}

fn build(hole: bool, size: &dyn Fn([f64; 2]) -> f64) -> Result<MeshedProblem, String> {
    let mesh = mesh_region(&region(hole), PHYS, ElementKind::Tri6, size, MeshOptions::default())?;
    let model = Model::new(mesh)?;
    let mut bc = model.dirichlet();
    for &n in model.mesh.node_set("left").unwrap() {
        bc.fix(n, 0, 0.0);
    }
    let anchor = *model
        .mesh
        .node_set("left")
        .unwrap()
        .iter()
        .min_by(|&&a, &&b| {
            (model.mesh.nodes[a][1] - 2.0)
                .abs()
                .total_cmp(&(model.mesh.nodes[b][1] - 2.0).abs())
        })
        .unwrap();
    bc.fix(anchor, 1, 0.0);
    let loads = Loads {
        faces: model.mesh.surfaces["right"]
            .iter()
            .map(|f| (f.clone(), SurfaceLoad::Traction([1000.0, 0.0, 0.0])))
            .collect(),
        ..Loads::default()
    };
    Ok(MeshedProblem { model, loads, bc, labels: None, quad_scale: 1.0 })
}

/// The goal: sigma_xx weighted by a bump around the top of the hole (where the concentration is) or its position on the plain plate.
fn goal(tol: Option<f64>) -> GoalSpec {
    let (c, r) = ([HOLE[0], HOLE[1] + 0.6], 0.35);
    let w = move |x: &[f64; 3]| {
        let q = ((x[0] - c[0]).powi(2) + (x[1] - c[1]).powi(2)) / (r * r);
        if q >= 1.0 { 0.0 } else { (1.0 - q).powi(2) }
    };
    GoalSpec::new("sigma_xx at the hole top", tol, move |m, _| LinearFunctional::weighted_stress(m, 0, &w))
}

fn reference(hole: bool) -> f64 {
    let p = build(hole, &|_| 0.05).unwrap();
    let sol = p.model.solve_static(&p.loads, &p.bc).unwrap();
    (goal(None).build)(&p.model, &p.loads).unwrap().value(&sol.u)
}

fn key(hole: bool) -> WorkloadKey {
    WorkloadKey {
        geometry: if hole { "plate-with-hole" } else { "plain-plate" }.into(),
        material: "E=1e7,nu=0.3".into(),
        boundary_conditions: "left fixed x; traction on right".into(),
        contact: "none".into(),
        qois: vec!["sigma_xx at the hole top".into()],
    }
}

struct Run {
    cycles: usize,
    dofs: usize,
    goal_error: f64,
    seconds: f64,
    stop: Stop,
    result: AmrResult,
}

fn converge(hole: bool, start: &dyn Fn([f64; 2]) -> f64, j_ref: f64, tol: f64) -> Run {
    let mut mesher = |s: &AdaptSize| build(hole, &|p| s.at([p[0], p[1], 0.0]));
    let cfg = AmrConfig { max_cycles: 10, keep_fields: FieldRetention::All, ..AmrConfig::default() };
    let t = std::time::Instant::now();
    let result = run_amr(build(hole, start).unwrap(), &[goal(Some(tol))], &mut mesher, &cfg).unwrap();
    let seconds = t.elapsed().as_secs_f64();
    let last = result.history.last().unwrap();
    Run {
        cycles: result.history.len(),
        dofs: last.dofs,
        goal_error: (j_ref - last.goals[0].value).abs(),
        seconds,
        stop: result.stop.clone(),
        result,
    }
}

fn experiment(hole: bool, rel_tol: f64) {
    let j_ref = reference(hole);
    let tol = rel_tol * j_ref.abs();
    let default = converge(hole, &|_| START, j_ref, tol);
    let mut s = summarize(key(hole), &default.result);
    // The deterministic repeats of one class stand in for three validated runs of it.
    s.end = EndKind::Converged;
    let history = vec![s.clone(), s.clone(), s];
    let advice = advise(&history, &key(hole), &AdviceOptions::default());
    eprintln!("[{}] J_ref {j_ref:.4}, tol {tol:.3}; advice {:?}", if hole { "hole" } else { "plain" }, advice.as_ref().map(|a| &a.zone_scale));
    eprintln!("  default: {} cycles, {} dofs, goal error {:.3e}, {:.2} s, {:?}", default.cycles, default.dofs, default.goal_error, default.seconds, default.stop);
    if !hole {
        // Nothing to refine away from the loaded edge: no advice may be given for a zone that was never refined.
        assert!(advice.is_none_or(|a| a.scale_for(Zone::GeometricDiscontinuity) == 1.0));
        return;
    }
    drop(advice);
    // The shipped depth fraction (0.5) and a stronger one (1.0: the full mean demand pre-applied): an exploratory sweep, the
    // default is not tuned on one benchmark.
    for depth in [0.5, 1.0] {
        let opts = AdviceOptions { depth_fraction: depth, ..AdviceOptions::default() };
        let advice = advise(&history, &key(hole), &opts).expect("three agreeing runs");
        // Which zone the hole's refinement lands in is the classifier's business: apply the strongest advised refinement near the hole.
        let scale = advice.zone_scale.values().copied().fold(1.0, f64::min);
        let advised = move |p: [f64; 2]| {
            if ((p[0] - HOLE[0]).powi(2) + (p[1] - HOLE[1]).powi(2)).sqrt() < 1.2 { START * scale } else { START }
        };
        let adv = converge(hole, &advised, j_ref, tol);
        eprintln!("  advised depth {depth} (near the hole x{scale:.3}): {} cycles, {} dofs, goal error {:.3e}, {:.2} s, {:?}", adv.cycles, adv.dofs, adv.goal_error, adv.seconds, adv.stop);
        assert!(matches!(default.stop, Stop::Converged) && matches!(adv.stop, Stop::Converged), "{:?} {:?}", default.stop, adv.stop);
    }
}

#[test]
fn the_plate_with_a_hole_is_measured_with_and_without_advice() {
    for rel_tol in [2e-3, 5e-4, 1e-4] {
        experiment(true, rel_tol);
    }
}

#[test]
fn the_plain_plate_control_gets_no_advice_for_the_hole_zone() {
    experiment(false, 2e-3);
}
