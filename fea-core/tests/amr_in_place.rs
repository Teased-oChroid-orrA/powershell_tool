//! Adaptive refinement whose mesher is the in-place metric refinement (`metric_refine::refine_projected`) instead of a fresh
//! remesh (#58, "AMR convergence integration"): the vertices of the previous mesh survive (here exactly: no smoothing), the
//! inserted boundary vertices are projected onto the true surfaces, and the controller's size field becomes an isotropic metric.
//! The body is the thick-walled cylinder of `tests/tetmesh.rs` (a quarter, ends held, Lame solution), so the adapted solution
//! is checked against the exact one. `docs/issue-46-phase-90.md`.

use fea_core::adaptivity::*;
use fea_core::meshing::metric::{Metric, MetricBounds};
use fea_core::meshing::metric_refine::{refine_projected, RefineOptions};
use fea_core::meshing::tet_topology::{Point, TetTopology};
use fea_core::tetmesh::{mesh_solid, FaceHit, Solid, TetOptions};
use fea_core::*;
use std::cell::RefCell;

const E: f64 = 200e3;
const NU: f64 = 0.3;
const A: f64 = 1.0;
const B: f64 = 2.0;
const P: f64 = 100.0;
const LEN: f64 = 0.5;

fn start_topology() -> TetTopology {
    let sdf = |x: [f64; 3]| {
        let r = x[0].hypot(x[1]);
        (r - B).max(A - r).max(-x[0]).max(-x[1]).max((x[2] - LEN / 2.0).abs() - LEN / 2.0)
    };
    let face = |x: [f64; 3]| {
        let r = x[0].hypot(x[1]);
        let d = [(r - A).abs(), (r - B).abs(), x[0].abs(), x[1].abs(), x[2].abs(), (x[2] - LEN).abs()];
        (0..6).min_by(|&i, &j| d[i].total_cmp(&d[j])).unwrap()
    };
    let faces = |x: [f64; 3]| {
        let r = x[0].hypot(x[1]).max(1e-12);
        let radial = [x[0] / r, x[1] / r, 0.0];
        vec![
            FaceHit { face: 0, dist: A - r, normal: [-radial[0], -radial[1], 0.0] },
            FaceHit { face: 1, dist: r - B, normal: radial },
            FaceHit { face: 2, dist: -x[0], normal: [-1.0, 0.0, 0.0] },
            FaceHit { face: 3, dist: -x[1], normal: [0.0, -1.0, 0.0] },
            FaceHit { face: 4, dist: -x[2], normal: [0.0, 0.0, -1.0] },
            FaceHit { face: 5, dist: x[2] - LEN, normal: [0.0, 0.0, 1.0] },
        ]
    };
    let size = |_: [f64; 3]| 10.0;
    let solid = Solid { lo: [0.0, 0.0, 0.0], hi: [2.05, 2.05, LEN], sdf: &sdf, size: &size, face_of: &face, face_dists: Some(&faces) };
    let (mesh, _) = mesh_solid(&solid, Elastic::new(E, NU), TetOptions { base: 0.6, boundary_size: 0.55, quadratic: false, ..Default::default() }).unwrap();
    let cells: Vec<[usize; 4]> = mesh.blocks[0].conn.chunks_exact(4).map(|c| [c[0], c[1], c[2], c[3]]).collect();
    TetTopology::new(mesh.nodes.clone(), cells, 1_000_000).expect("start topology")
}

/// The model surface a boundary point belongs to (the nearest one), numbered like `tetmesh`'s `face1`..`face6`.
fn surface_of(x: Point) -> usize {
    let r = x[0].hypot(x[1]);
    let d = [(r - A).abs(), (r - B).abs(), x[0].abs(), x[1].abs(), x[2].abs(), (x[2] - LEN).abs()];
    (0..6).min_by(|&i, &j| d[i].total_cmp(&d[j])).unwrap()
}

/// Onto the bore or the outer wall when the point is within 0.1 of it (the sagitta of a boundary edge); planes already hold.
fn project(x: Point) -> Point {
    let r = x[0].hypot(x[1]);
    for target in [A, B] {
        if (r - target).abs() < 0.1 && r > 0.0 {
            return [x[0] * target / r, x[1] * target / r, x[2]];
        }
    }
    x
}

/// The meshed problem of a topology: named outward faces, symmetry planes and ends held, internal pressure on the bore.
fn problem_of(top: &TetTopology) -> Result<MeshedProblem, String> {
    let mut mesh = Mesh::new(Physics::Solid);
    for p in top.points() {
        mesh.add_node(*p);
    }
    mesh.add_block(ElementKind::Tet4, top.cells().iter().flatten().copied().collect(), Elastic::new(E, NU), "solid")?;
    let mut named: std::collections::BTreeMap<usize, Vec<Vec<usize>>> = Default::default();
    let mut count = 0;
    for (c, cell) in top.cells().iter().enumerate() {
        for k in 0..4 {
            if top.neighbors()[c][k].is_some() {
                continue;
            }
            count += 1;
            let f: Vec<usize> = (0..4).filter(|&i| i != k).map(|i| cell[i]).collect();
            let (a, b, d) = (top.points()[f[0]], top.points()[f[1]], top.points()[f[2]]);
            let apex = top.points()[cell[k]];
            let n = [(b[1] - a[1]) * (d[2] - a[2]) - (b[2] - a[2]) * (d[1] - a[1]), (b[2] - a[2]) * (d[0] - a[0]) - (b[0] - a[0]) * (d[2] - a[2]), (b[0] - a[0]) * (d[1] - a[1]) - (b[1] - a[1]) * (d[0] - a[0])];
            let outward = n.iter().zip(apex.iter().zip(&a)).map(|(n, (p, q))| n * (p - q)).sum::<f64>() < 0.0;
            let mut nodes = f.clone();
            if !outward {
                nodes.swap(1, 2);
            }
            let centre: Point = std::array::from_fn(|i| (a[i] + b[i] + d[i]) / 3.0);
            named.entry(surface_of(centre)).or_default().push(nodes);
        }
    }
    assert_eq!(count, top.boundary().len());
    for (i, list) in named {
        mesh.surfaces.insert(format!("face{}", i + 1), list);
    }
    let model = Model::new(mesh)?;
    let mut bc = model.dirichlet();
    for (n, x) in model.mesh.nodes.iter().enumerate() {
        if x[0].abs() < 1e-9 {
            bc.fix(n, 0, 0.0);
        }
        if x[1].abs() < 1e-9 {
            bc.fix(n, 1, 0.0);
        }
        if x[2].abs() < 1e-9 || (x[2] - LEN).abs() < 1e-9 {
            bc.fix(n, 2, 0.0);
        }
    }
    let mut loads = Loads::default();
    for f in model.mesh.surfaces.get("face1").cloned().unwrap_or_default() {
        loads.faces.push((f, SurfaceLoad::Pressure(P)));
    }
    Ok(MeshedProblem { model, loads, bc, labels: None, quad_scale: 1.0 })
}

/// Rms relative error of the radial displacement against the Lame solution over the nodes of the model.
fn lame_error(p: &MeshedProblem, u: &[f64]) -> f64 {
    let (aa, bb) = (P * A * A / (B * B - A * A), P * A * A * B * B / (B * B - A * A));
    let ur = |r: f64| (1.0 + NU) / E * ((1.0 - 2.0 * NU) * aa * r + bb / r);
    let (mut num, mut den) = (0.0, 0.0);
    for (n, x) in p.model.mesh.nodes.iter().enumerate() {
        let r = x[0].hypot(x[1]);
        if r < 1e-9 {
            continue;
        }
        let ux = (u[n * 3] * x[0] + u[n * 3 + 1] * x[1]) / r;
        num += (ux - ur(r)).powi(2);
        den += ur(r).powi(2);
    }
    (num / den).sqrt()
}

struct Outcome {
    cycles: Vec<(usize, usize, f64, f64)>,
    first_error: f64,
    final_error: f64,
    stop: Stop,
    start_vertices: usize,
    end_vertices: usize,
    surface_vertices_on_surface: (usize, usize),
}

fn run_case(min_size: f64, cycles: usize, max_cells: usize) -> Outcome {
    let start = start_topology();
    let n0 = start.points().len();
    let first = problem_of(&start).unwrap();
    let state = RefCell::new(start.clone());
    let bounds = MetricBounds::new(0.01, 1.0, 10.0).unwrap();
    let options = RefineOptions { max_passes: 30, coarsen_passes: 0, max_cells, ..RefineOptions::default() };
    let mut mesher = |s: &AdaptSize| -> Result<MeshedProblem, String> {
        let field = |p: Point| Metric::from_sizes([[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]], std::array::from_fn(|_| s.at(p).clamp(min_size, 0.9)));
        let (next, _) = refine_projected(&state.borrow(), &field, &bounds, options, &project).map_err(|e| format!("{e:?}"))?;
        *state.borrow_mut() = next.clone();
        problem_of(&next)
    };
    // The signed-distance start mesh is itself at a scaled Jacobian of 0.032, so the floor for the refined meshes is "not
    // worse than the start" (0.03): the default 0.1 cannot be met by refining that mesh in place.
    let mut quality = AmrConfig::default().quality;
    quality.min_scaled_jacobian = 0.03;
    let cfg = AmrConfig { max_cycles: cycles, physics_tolerance: Some(0.04), keep_fields: FieldRetention::None, quality, ..AmrConfig::default() };
    let goal = GoalSpec::new("compliance", None, LinearFunctional::compliance);
    let result = run_amr(first, &[goal], &mut mesher, &cfg).unwrap();
    let p0 = problem_of(&start).unwrap();
    let sol = p0.model.solve_static(&p0.loads, &p0.bc).unwrap();
    // Inserted bore vertices (index beyond the start's) that lie on the bore.
    let end = state.borrow();
    let (mut on, mut total) = (0, 0);
    for &v in end.boundary().iter().flatten().collect::<std::collections::BTreeSet<_>>() {
        let x = end.points()[v];
        if v >= n0 && (x[0].hypot(x[1]) - A).abs() < 0.1 {
            total += 1;
            if (x[0].hypot(x[1]) - A).abs() < 1e-9 {
                on += 1;
            }
        }
    }
    Outcome {
        cycles: result.history.iter().map(|r| (r.elements, r.dofs, r.physics_error, r.min_scaled_jacobian)).collect(),
        first_error: lame_error(&p0, &sol.u),
        final_error: lame_error(&result.problem, &result.u),
        stop: result.stop.clone(),
        start_vertices: n0,
        end_vertices: end.points().len(),
        surface_vertices_on_surface: (on, total),
    }
}

fn report(o: &Outcome) {
    for (i, (elements, dofs, physics, jac)) in o.cycles.iter().enumerate() {
        eprintln!("cycle {i} elements {elements:>6} dofs {dofs:>6} physics error {physics:.3e} min Jacobian {jac:.3}");
    }
    eprintln!("stop {:?}; rms radial error against Lame: start {:.3e}, final {:.3e}; vertices {} -> {}; inserted bore vertices on the bore {:?}", o.stop, o.first_error, o.final_error, o.start_vertices, o.end_vertices, o.surface_vertices_on_surface);
}

#[test]
fn an_adaptive_run_driven_by_in_place_refinement_improves_the_answer_on_a_curved_body() {
    let o = run_case(0.3, 1, 8_000);
    report(&o);
    assert!(o.cycles.len() >= 2, "the run adapted at least once");
    assert!(o.cycles[1].2 < o.cycles[0].2, "the physics error fell: {:?}", o.cycles);
    assert!(o.final_error < o.first_error, "{} vs {}", o.final_error, o.first_error);
    assert!(o.end_vertices > o.start_vertices);
    let (on, total) = o.surface_vertices_on_surface;
    assert!(total >= 3 && on as f64 >= 0.9 * total as f64, "{on} of {total}");
}

#[test]
#[ignore = "study: about six minutes in release; run with --release --ignored --nocapture"]
fn the_same_run_with_two_adaptation_cycles() {
    let o = run_case(0.12, 2, 40_000);
    report(&o);
    assert!(o.final_error < 0.2 * o.first_error);
}
