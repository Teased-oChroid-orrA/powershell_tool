//! Preconditioned conjugate gradients with smoothed-aggregation multigrid: agreement with the
//! direct solver, mesh-independent iteration counts, and the near-null-space handling of 2D,
//! axisymmetric and 3D models.

#![allow(clippy::needless_range_loop)] // index-parallel numeric kernels read clearer as loops

use fea_core::generate::grid;
use fea_core::*;

const E: f64 = 10.0e6;

fn cube(kind: ElementKind, n: usize) -> (Model, Dirichlet, Loads) {
    let mut mesh = grid(Physics::Solid, kind, Elastic::new(E, 0.3), [n, n, n], &|p| [p[0], 0.8 * p[1], 0.6 * p[2]]).unwrap();
    // Clamp the z = 0 face, load the z = 0.6 face (selected geometrically: simplex grids have no named faces).
    mesh.select_nodes("base", |x| x[2] < 1e-9);
    let top: Vec<Vec<usize>> = mesh.select_faces("top", |c| c[2] > 0.6 - 1e-9).to_vec();
    let model = Model::new(mesh).unwrap();
    let mut bc = model.dirichlet();
    for &nd in model.mesh.node_set("base").unwrap() {
        bc.fix_node(nd);
    }
    let faces = top.into_iter().map(|f| (f, SurfaceLoad::Traction([100.0, 50.0, -300.0]))).collect();
    (model, bc, Loads { faces, body: Some([0.0, 0.0, -10.0]), ..Loads::default() })
}

fn max_rel_diff(a: &[f64], b: &[f64]) -> f64 {
    let scale = b.iter().fold(0.0f64, |m, v| m.max(v.abs()));
    a.iter().zip(b).fold(0.0f64, |m, (x, y)| m.max((x - y).abs())) / scale
}

#[test]
fn iterative_matches_direct_on_every_3d_element_type() {
    for (kind, n) in [(ElementKind::Hex8, 8), (ElementKind::Hex20, 5), (ElementKind::Hex27, 4), (ElementKind::Tet4, 10), (ElementKind::Tet10, 4)] {
        let (model, bc, loads) = cube(kind, n);
        let direct = model.solve_static_with(&loads, &bc, SolveMethod::Direct).unwrap();
        let iter = model.solve_static_with(&loads, &bc, SolveMethod::iterative()).unwrap();
        let diff = max_rel_diff(&iter.u, &direct.u);
        eprintln!("{kind:?} n={n} ({} free dofs): {} CG iterations, max relative difference {diff:.2e}", iter.n_free, iter.iterations.unwrap());
        assert!(diff < 1e-8, "{kind:?}: iterative differs from direct by {diff:e}");
        assert!(iter.iterations.unwrap() < 60, "{kind:?}: {} iterations", iter.iterations.unwrap());
        assert!(direct.iterations.is_none());
    }
}

#[test]
fn iteration_count_does_not_grow_with_the_mesh() {
    let counts: Vec<usize> = [10, 16, 24, 32]
        .iter()
        .map(|&n| {
            let (model, bc, loads) = cube(ElementKind::Hex8, n);
            let sol = model.solve_static_with(&loads, &bc, SolveMethod::iterative()).unwrap();
            eprintln!("Hex8 {n}^3 ({} dofs): {} iterations", sol.n_free, sol.iterations.unwrap());
            sol.iterations.unwrap()
        })
        .collect();
    let (lo, hi) = (*counts.iter().min().unwrap(), *counts.iter().max().unwrap());
    assert!(hi <= lo + lo / 2 + 3, "iteration counts grow with the mesh: {counts:?}");
    assert!(hi < 30, "{counts:?}");
}

#[test]
fn iterative_handles_plane_and_axisymmetric_models() {
    // Plane stress plate with a clamped edge and an edge traction; axisymmetric thick cylinder.
    let mesh = grid(Physics::PlaneStress { thickness: 0.5 }, ElementKind::Quad9, Elastic::new(E, 0.3), [30, 20, 1], &|p| [3.0 * p[0], 2.0 * p[1], 0.0]).unwrap();
    let model = Model::new(mesh).unwrap();
    let mut bc = model.dirichlet();
    for &n in model.mesh.node_set("u0").unwrap() {
        bc.fix_node(n);
    }
    let loads = Loads { faces: model.mesh.surfaces["u1"].iter().map(|f| (f.clone(), SurfaceLoad::Traction([200.0, -500.0, 0.0]))).collect(), ..Loads::default() };
    let direct = model.solve_static_with(&loads, &bc, SolveMethod::Direct).unwrap();
    let iter = model.solve_static_with(&loads, &bc, SolveMethod::iterative()).unwrap();
    assert!(max_rel_diff(&iter.u, &direct.u) < 1e-8, "plane");
    eprintln!("plane Quad9: {} iterations", iter.iterations.unwrap());

    let mesh = grid(Physics::Axisymmetric, ElementKind::Quad9, Elastic::new(E, 0.3), [20, 40, 1], &|p| [1.0 + p[0], 8.0 * p[1], 0.0]).unwrap();
    let model = Model::new(mesh).unwrap();
    let mut bc = model.dirichlet();
    for &n in model.mesh.node_set("v0").unwrap() {
        bc.fix(n, 1, 0.0);
    }
    let loads = Loads { faces: model.mesh.surfaces["u0"].iter().map(|f| (f.clone(), SurfaceLoad::Pressure(1000.0))).collect(), ..Loads::default() };
    let direct = model.solve_static_with(&loads, &bc, SolveMethod::Direct).unwrap();
    let iter = model.solve_static_with(&loads, &bc, SolveMethod::iterative()).unwrap();
    assert!(max_rel_diff(&iter.u, &direct.u) < 1e-8, "axisymmetric");
    eprintln!("axisymmetric Quad9: {} iterations", iter.iterations.unwrap());
}

#[test]
fn prescribed_displacements_and_reactions_survive_the_iterative_path() {
    let (model, mut bc, _) = cube(ElementKind::Hex20, 4);
    for n in (0..model.mesh.nodes.len()).filter(|&n| model.mesh.nodes[n][2] > 0.6 - 1e-9) {
        bc.fix(n, 0, 2e-3);
    }
    let direct = model.solve_static_with(&Loads::default(), &bc, SolveMethod::Direct).unwrap();
    let iter = model.solve_static_with(&Loads::default(), &bc, SolveMethod::iterative()).unwrap();
    assert!(max_rel_diff(&iter.u, &direct.u) < 1e-8);
    assert!(max_rel_diff(&iter.reactions, &direct.reactions) < 1e-6, "reactions");
}

#[test]
fn an_unconverged_iteration_is_an_error_not_a_wrong_answer() {
    let (model, bc, loads) = cube(ElementKind::Hex8, 10);
    let err = model.solve_static_with(&loads, &bc, SolveMethod::Iterative { tol: 1e-14, max_iter: 1 }).err().expect("one iteration cannot converge");
    assert!(err.contains("did not converge"), "{err}");
}
