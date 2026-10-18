//! Mode I benchmark (#60/#62 plan, `docs/issue-46-benchmarks.md` section A): an edge crack of length `a` in a plate of width
//! `W` under uniform tension, whose stress intensity factor is `K_I = Y(a/W) sigma sqrt(pi a)` with the handbook edge-crack
//! correction `Y = 1.12 - 0.231 l + 10.55 l^2 - 21.72 l^3 + 30.39 l^4` (`l = a/W`, Tada, Paris & Irwin; about 0.5 % for `l <= 0.6`
//! when the plate is tall enough). The kernel has no crack-tip element or J-integral, so `K_I` is extracted from the crack-face
//! opening near the tip: for plane stress `u_y(r, theta = pi) = K_I (kappa + 1) / (2 mu) sqrt(r / 2 pi)`, extrapolated to `r -> 0`.
//! The half model is the upper half of the cracked plate: the crack face is free, the ligament carries the symmetry condition.

use fea_core::adaptivity::*;
use fea_core::delaunay::MeshOptions;
use fea_core::geometry::{Loop, Region};
use fea_core::mesh2d::mesh_region;
use fea_core::*;

const E: f64 = 1.0e7;
const NU: f64 = 0.3;
const SIGMA: f64 = 1000.0;
const W: f64 = 1.0;
const H: f64 = 2.0; // half height of the plate (the full height is 4 W)

fn handbook_y(l: f64) -> f64 {
    1.12 - 0.231 * l + 10.55 * l * l - 21.72 * l.powi(3) + 30.39 * l.powi(4)
}

fn region(a: f64) -> Region {
    let outer = Loop::polygon_named(
        &[[0.0, 0.0], [a, 0.0], [W, 0.0], [W, H], [0.0, H]],
        &["crack", "ligament", "right", "top", "left"],
    )
    .unwrap();
    Region::new(outer, vec![], Elastic::new(E, NU)).unwrap()
}

/// The half model on a mesh with element size `size(x, y)`: the ligament is held in y, one ligament node in x, the top edge
/// carries the tension.
fn build(a: f64, size: &dyn Fn([f64; 2]) -> f64) -> Result<MeshedProblem, String> {
    let physics = Physics::PlaneStress { thickness: 1.0 };
    let mut mesh = mesh_region(
        &region(a),
        physics,
        ElementKind::Tri6,
        size,
        MeshOptions::default(),
    )?;
    // the crack mouth (x = y = 0) as a node set of its own: its opening is a mesh-independent goal
    let mouth = mesh
        .nodes
        .iter()
        .position(|n| n[0].abs() < 1e-12 && n[1].abs() < 1e-12)
        .ok_or("no node at the crack mouth")?;
    mesh.node_sets.insert("mouth".into(), vec![mouth]);
    let model = Model::new(mesh)?;
    let mut bc = model.dirichlet();
    for &n in model.mesh.node_set("ligament").unwrap() {
        bc.fix(n, 1, 0.0);
    }
    let anchor = *model
        .mesh
        .node_set("ligament")
        .unwrap()
        .iter()
        .max_by(|&&p, &&q| model.mesh.nodes[p][0].total_cmp(&model.mesh.nodes[q][0]))
        .unwrap();
    bc.fix(anchor, 0, 0.0);
    let loads = Loads {
        faces: model.mesh.surfaces["top"]
            .iter()
            .map(|f| (f.clone(), SurfaceLoad::Traction([0.0, SIGMA, 0.0])))
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

/// Solve the half model on a mesh graded toward the tip with element size `size(distance to the tip)`.
fn solve(a: f64, size: impl Fn(f64) -> f64) -> (Model, Vec<f64>) {
    let p = build(a, &move |q| size((q[0] - a).hypot(q[1]))).unwrap();
    let sol = p.model.solve_static(&p.loads, &p.bc).unwrap();
    (p.model, sol.u)
}

/// `K_I` from the crack-face opening: `K(r) = 2 mu u_y / (kappa + 1) sqrt(2 pi / r)` on the crack-face nodes at distances
/// `r` in `[lo, hi] a`, extrapolated linearly to `r = 0` (a least-squares line through `K(r)`).
fn k_from_opening(model: &Model, u: &[f64], a: f64, lo: f64, hi: f64) -> f64 {
    let mu = E / (2.0 * (1.0 + NU));
    let kappa = (3.0 - NU) / (1.0 + NU);
    let pts: Vec<(f64, f64)> = model
        .mesh
        .node_set("crack")
        .unwrap()
        .iter()
        .filter_map(|&n| {
            let r = a - model.mesh.nodes[n][0];
            (r >= lo * a && r <= hi * a).then(|| {
                (
                    r,
                    2.0 * mu * u[2 * n + 1] / (kappa + 1.0)
                        * (2.0 * std::f64::consts::PI / r).sqrt(),
                )
            })
        })
        .collect();
    assert!(
        pts.len() >= 6,
        "only {} crack-face nodes in the fitting window",
        pts.len()
    );
    let n = pts.len() as f64;
    let (sx, sy) = (
        pts.iter().map(|p| p.0).sum::<f64>(),
        pts.iter().map(|p| p.1).sum::<f64>(),
    );
    let (sxx, sxy) = (
        pts.iter().map(|p| p.0 * p.0).sum::<f64>(),
        pts.iter().map(|p| p.0 * p.1).sum::<f64>(),
    );
    let slope = (n * sxy - sx * sy) / (n * sxx - sx * sx);
    (sy - slope * sx) / n
}

#[test]
fn the_edge_crack_stress_intensity_factor_matches_the_handbook() {
    for l in [0.2, 0.3, 0.4] {
        let a = l * W;
        let exact = handbook_y(l) * SIGMA * (std::f64::consts::PI * a).sqrt();
        let mut errors = Vec::new();
        for tip_size in [a / 25.0, a / 50.0, a / 100.0, a / 200.0, a / 400.0] {
            let (model, u) = solve(a, |d| (tip_size + 0.25 * d).min(0.15));
            let k = k_from_opening(&model, &u, a, 0.02, 0.25);
            eprintln!("a/W = {l}: tip size {tip_size:.4}, {} dofs: K_I = {k:.2} (handbook {exact:.2}, {:+.2} %)", model.mesh.n_dofs(), 100.0 * (k / exact - 1.0));
            errors.push((k / exact - 1.0).abs());
        }
        // the extraction converges from below as the tip is resolved, to within the handbook's own accuracy
        assert!(
            errors.windows(2).all(|w| w[1] < w[0]),
            "a/W = {l}: K_I error does not decrease with the tip size: {errors:?}"
        );
        assert!(
            *errors.last().unwrap() < 0.01,
            "a/W = {l}: K_I off by {:.2} % on the finest mesh",
            100.0 * errors.last().unwrap()
        );
    }
}

// ---------------------------------------------------------------- time to accuracy (#62)

const A: f64 = 0.3;
const START: f64 = 0.2;

/// The goal: the opening of the crack mouth (CMOD). A mean over the crack-face nodes is not mesh independent (a finer tip adds
/// nodes where the opening is small), a point value is.
fn goal(tol: Option<f64>) -> GoalSpec {
    GoalSpec::mean_displacement("crack mouth opening", "mouth", 1, tol)
}

fn j_ref() -> f64 {
    let p = build(A, &|q| (0.0008 + 0.25 * (q[0] - A).hypot(q[1])).min(0.04)).unwrap();
    let sol = p.model.solve_static(&p.loads, &p.bc).unwrap();
    (goal(None).build)(&p.model, &p.loads)
        .unwrap()
        .value(&sol.u)
}

struct Run {
    cycles: usize,
    dofs: usize,
    goal_error: f64,
    seconds: f64,
    stop: Stop,
    result: AmrResult,
}

fn converge(problem: MeshedProblem, j_ref: f64, tol: f64) -> Run {
    let mut mesher = |s: &AdaptSize| build(A, &|p| s.at([p[0], p[1], 0.0]));
    let cfg = AmrConfig {
        max_cycles: 12,
        keep_fields: FieldRetention::All,
        ..AmrConfig::default()
    };
    let t = std::time::Instant::now();
    let result = run_amr(problem, &[goal(Some(tol))], &mut mesher, &cfg).unwrap();
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

/// Default uniform start, the advice of three agreeing runs applied near the tip, and the finished mesh as a perfect oracle.
#[test]
fn the_edge_crack_is_measured_with_and_without_advice() {
    let j_ref = j_ref();
    for rel_tol in [2e-3, 5e-4] {
        let tol = rel_tol * j_ref.abs();
        let default = converge(build(A, &|_| START).unwrap(), j_ref, tol);
        let key = WorkloadKey {
            geometry: "edge-crack".into(),
            material: "E=1e7,nu=0.3".into(),
            boundary_conditions: "ligament held in y; traction on top".into(),
            contact: "none".into(),
            qois: vec!["crack mouth opening".into()],
        };
        let mut summary = summarize(key.clone(), &default.result);
        summary.end = EndKind::Converged;
        let history = vec![summary.clone(), summary.clone(), summary];
        let advice = advise(&history, &key, &AdviceOptions::default());
        let scale = advice
            .as_ref()
            .map_or(1.0, |a| a.zone_scale.values().copied().fold(1.0, f64::min));
        eprintln!(
            "[crack tol {rel_tol:e}] J_ref {j_ref:.5e}; advice zones {:?}",
            advice.as_ref().map(|a| &a.zone_scale)
        );
        eprintln!(
            "  default: {} cycles, {} dofs, goal error {:.3e}, {:.2} s, {:?}",
            default.cycles, default.dofs, default.goal_error, default.seconds, default.stop
        );
        let advised = converge(
            build(A, &move |p| {
                if (p[0] - A).hypot(p[1]) < 0.3 {
                    START * scale
                } else {
                    START
                }
            })
            .unwrap(),
            j_ref,
            tol,
        );
        eprintln!("  advised (near the tip x{scale:.3}): {} cycles, {} dofs, goal error {:.3e}, {:.2} s, {:?}", advised.cycles, advised.dofs, advised.goal_error, advised.seconds, advised.stop);
        let p = &default.result.problem;
        let same = MeshedProblem {
            model: p.model.clone(),
            loads: p.loads.clone(),
            bc: p.bc.clone(),
            labels: None,
            quad_scale: 1.0,
        };
        let oracle = converge(same, j_ref, tol);
        eprintln!(
            "  finished-mesh oracle: {} cycles, {} dofs, {:.2} s (time ratio {:.2})",
            oracle.cycles,
            oracle.dofs,
            oracle.seconds,
            oracle.seconds / default.seconds
        );
        assert!(
            matches!(default.stop, Stop::Converged)
                && matches!(advised.stop, Stop::Converged)
                && matches!(oracle.stop, Stop::Converged),
            "{:?} {:?} {:?}",
            default.stop,
            advised.stop,
            oracle.stop
        );
        // the estimator's own claim is checked against the reference: the converged goal is within a few tolerances of it
        assert!(
            default.goal_error < 5.0 * tol,
            "default run: goal error {:e} against tolerance {tol:e}",
            default.goal_error
        );
        assert!(
            advised.goal_error < 5.0 * tol && oracle.goal_error < 5.0 * tol,
            "advised {:e}, oracle {:e}, tolerance {tol:e}",
            advised.goal_error,
            oracle.goal_error
        );
    }
}
