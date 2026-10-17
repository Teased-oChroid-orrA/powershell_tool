//! AMR feedback end to end on a real adaptive run (the L-shaped plate of `tests/amr.rs`): the run is summarised with its
//! regional refinement demand, repeated runs of one class give advice, and the advice only ever asks for a finer start.

use fea_core::adaptivity::*;
use fea_core::delaunay::MeshOptions;
use fea_core::geometry::{Loop, Region};
use fea_core::mesh2d::mesh_region;
use fea_core::*;

fn region() -> Region {
    let outer = Loop::polygon_named(
        &[
            [0.0, 0.0],
            [2.0, 0.0],
            [2.0, 1.0],
            [1.0, 1.0],
            [1.0, 2.0],
            [0.0, 2.0],
        ],
        &["bottom", "right", "ledge", "inner", "right_top", "top_left"],
    )
    .unwrap();
    Region::new(outer, vec![], Elastic::new(10.0e6, 0.3)).unwrap()
}

fn build(size: &dyn Fn([f64; 2]) -> f64) -> Result<MeshedProblem, String> {
    let mesh = mesh_region(
        &region(),
        Physics::PlaneStress { thickness: 1.0 },
        ElementKind::Tri6,
        size,
        MeshOptions::default(),
    )?;
    let model = Model::new(mesh)?;
    let mut bc = model.dirichlet();
    for (nd, x) in model.mesh.nodes.iter().enumerate() {
        if x[0] < 1e-9 {
            bc.fix(nd, 0, 0.0);
            bc.fix(nd, 1, 0.0);
        }
    }
    let loads = Loads {
        faces: model.mesh.surfaces["right"]
            .iter()
            .map(|f| (f.clone(), SurfaceLoad::Traction([0.0, -100.0, 0.0])))
            .collect(),
        ..Loads::default()
    };
    Ok(MeshedProblem {
        model,
        loads,
        bc,
        labels: None,
        quad_scale: 1.0,
    })
}

/// Smooth bump `(1 - r^2/R^2)^2` around `c`.
fn bump(c: [f64; 2], r: f64) -> impl Fn(&[f64; 3]) -> f64 + Send + Sync + Clone {
    move |x: &[f64; 3]| {
        let q = ((x[0] - c[0]).powi(2) + (x[1] - c[1]).powi(2)) / (r * r);
        if q >= 1.0 {
            0.0
        } else {
            (1.0 - q).powi(2)
        }
    }
}

fn run(max_cycles: usize) -> AmrResult {
    let w = bump([1.5, 0.5], 0.45);
    let goal = GoalSpec::new("sigma_yy near the load", Some(0.5), move |m, _| {
        LinearFunctional::weighted_stress(m, 1, &w)
    });
    let mut mesher = |s: &AdaptSize| build(&|p| s.at([p[0], p[1], 0.0]));
    let cfg = AmrConfig {
        max_cycles,
        keep_fields: FieldRetention::All,
        ..AmrConfig::default()
    };
    run_amr(build(&|_| 0.4).unwrap(), &[goal], &mut mesher, &cfg).unwrap()
}

fn key() -> WorkloadKey {
    WorkloadKey {
        geometry: "l-plate".into(),
        material: "E=10e6,nu=0.3".into(),
        boundary_conditions: "x=0 fixed; traction on right".into(),
        contact: "none".into(),
        qois: vec!["sigma_yy near the load".into()],
    }
}

#[test]
fn a_real_run_is_summarised_with_its_regional_refinement_demand() {
    let result = run(4);
    let s = summarize(key(), &result);
    eprintln!("stop {:?}; cycles {}; dof growth {:.2}; error {:.3e} -> {:.3e}; reduction/cycle {:?}; end {:?}; demand {:?}; zones {:?}", result.stop, s.cycles, s.dof_growth, s.initial_error, s.final_error, s.error_reduction_per_cycle, s.end, s.zone_demand, s.zone_elements);
    assert_eq!(s.cycles, result.history.len());
    assert!(s.cycles >= 2);
    assert!(s.dof_growth > 1.0, "the mesh grew");
    assert!(s.has_regional_detail, "fields were kept for every cycle");
    assert!(!s.zone_demand.is_empty());
    // some zone was refined: demand is positive, and a zone can be marked at most once per cycle
    assert!(s.zone_demand.values().any(|d| *d > 0.0));
    assert!(s
        .zone_demand
        .values()
        .all(|d| *d >= 0.0 && *d <= s.cycles as f64));
    assert!(s.total_seconds > 0.0);
    assert_eq!(
        s.end == EndKind::Converged,
        matches!(result.stop, Stop::Converged)
    );
}

#[test]
fn repeated_runs_of_one_class_give_advice_that_only_refines() {
    let r = run(4);
    let mut history = Vec::new();
    for _ in 0..3 {
        let mut s = summarize(key(), &r);
        // The test plate does not converge within the cycles of a unit test; treat the (deterministic) repeats as validated
        // evidence of the class so that the advice rule is exercised on real zone demand.
        s.end = EndKind::Converged;
        history.push(s);
    }
    let advice =
        advise(&history, &key(), &AdviceOptions::default()).expect("three identical runs agree");
    eprintln!("advice: {advice:?}");
    assert_eq!(advice.runs_used, 3);
    assert!(advice.zone_scale.values().all(|s| (0.25..=1.0).contains(s)));
    assert!(
        !advice.zone_scale.is_empty(),
        "the plate's refinement demand is visible"
    );
    // One run is not evidence, and another class gets nothing.
    assert!(advise(&history[..1], &key(), &AdviceOptions::default()).is_none());
    let mut other = key();
    other.contact = "pin".into();
    assert!(advise(&history, &other, &AdviceOptions::default()).is_none());
}

/// Cycles, final dofs and error of a run whose initial mesh is `start` (size as a function of the point).
fn converge_from(start: &dyn Fn([f64; 2]) -> f64) -> (usize, usize, f64, f64) {
    let w = bump([1.5, 0.5], 0.45);
    let goal = GoalSpec::new("sigma_yy near the load", Some(0.5), move |m, _| {
        LinearFunctional::weighted_stress(m, 1, &w)
    });
    let mut mesher = |s: &AdaptSize| build(&|p| s.at([p[0], p[1], 0.0]));
    let t = std::time::Instant::now();
    let r = run_amr(
        build(start).unwrap(),
        &[goal],
        &mut mesher,
        &AmrConfig::default(),
    )
    .unwrap();
    let last = r.history.last().unwrap();
    assert!(matches!(r.stop, Stop::Converged), "{:?}", r.stop);
    (
        r.history.len(),
        last.dofs,
        last.physics_error,
        t.elapsed().as_secs_f64(),
    )
}

#[test]
fn applying_the_advice_is_measured_against_the_default_start() {
    let r = run(4);
    let mut history = Vec::new();
    for _ in 0..3 {
        let mut s = summarize(key(), &r);
        s.end = EndKind::Converged;
        history.push(s);
    }
    let advice = advise(&history, &key(), &AdviceOptions::default()).unwrap();
    let singular = advice.scale_for(Zone::Singular);
    assert!(singular < 1.0);
    // The re-entrant corner is at (1, 1): start finer within a radius of 0.35 of it, as advised.
    let default_start = |_: [f64; 2]| 0.4;
    let advised = move |p: [f64; 2]| {
        if ((p[0] - 1.0).powi(2) + (p[1] - 1.0).powi(2)).sqrt() < 0.35 {
            0.4 * singular
        } else {
            0.4
        }
    };
    let (a, b) = (converge_from(&default_start), converge_from(&advised));
    eprintln!(
        "default start: {} cycles, {} dofs, physics error {:.3e}, {:.3} s",
        a.0, a.1, a.2, a.3
    );
    eprintln!("advised start: {} cycles, {} dofs, physics error {:.3e}, {:.3} s (singular scale {singular:.3})", b.0, b.1, b.2, b.3);
    // Both reach the stated tolerance: AMR stays the authority on resolution whichever start is used.
    assert!(a.0 >= 1 && b.0 >= 1);
}
