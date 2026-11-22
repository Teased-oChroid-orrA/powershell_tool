//! `Loads::couplings`: a penalty relation `k/2 (sum c_j u_j - offset)^2` between displacement components, for the nonlinear driver.
//! Two separate plane-stress blocks of height 1 (nu = 0, so a pull is a uniform axial state) are joined edge to edge by one coupling
//! per node pair, each of stiffness `k0 w_i` (`w_i` the node's tributary edge length). The relative displacement of the two edges is
//! then `sigma / k0 + offset` exactly, and the tip moves `2 sigma L / E` plus that.

use fea_core::generate::grid;
use fea_core::*;

const E: f64 = 10.0e6;
const SIGMA: f64 = 1.0e3;

/// The two blocks (A at `y` in 0..1, held at its bottom; B at 3..4, held only by the couplings) and the couplings between A's top
/// and B's bottom nodes, each along `y` with stiffness `k0 w_i`.
fn blocks(k0: f64, offset: f64) -> (Model, Dirichlet, Loads, Vec<usize>) {
    let mat = Elastic::new(E, 0.0);
    let physics = Physics::PlaneStress { thickness: 1.0 };
    let mut mesh = grid(physics, ElementKind::Quad4, mat, [4, 3, 1], &|p| [2.0 * p[0], p[1], 0.0]).unwrap();
    let top = grid(physics, ElementKind::Quad4, mat, [4, 3, 1], &|p| [2.0 * p[0], 3.0 + p[1], 0.0]).unwrap();
    mesh.append(&top, "top_").unwrap();
    let model = Model::new(mesh).unwrap();
    let mut bc = model.dirichlet();
    for &n in model.mesh.node_set("v0").unwrap() {
        bc.fix(n, 1, 0.0);
    }
    bc.fix(model.mesh.node_set("v0").unwrap()[0], 0, 0.0);
    let by_x = |name: &str| {
        let mut v = model.mesh.node_set(name).unwrap().to_vec();
        v.sort_by(|&a, &b| model.mesh.nodes[a][0].total_cmp(&model.mesh.nodes[b][0]));
        v
    };
    let (a, b) = (by_x("v1"), by_x("top_v0"));
    assert_eq!(a.len(), 5);
    let h = 0.5;
    // `y` is the coupled direction under test; `x` is held by an unloaded spring of the same stiffness so B has no free sliding
    let couplings = a
        .iter()
        .zip(&b)
        .enumerate()
        .flat_map(|(i, (&na, &nb))| {
            let w = if i == 0 || i == 4 { 0.5 * h } else { h };
            [Coupling { terms: vec![(2 * nb + 1, 1.0), (2 * na + 1, -1.0)], k: k0 * w, offset }, Coupling { terms: vec![(2 * nb, 1.0), (2 * na, -1.0)], k: k0 * w, offset: 0.0 }]
        })
        .collect();
    let loads = Loads { faces: model.mesh.surfaces["top_v1"].iter().map(|f| (f.clone(), SurfaceLoad::Traction([0.0, SIGMA, 0.0]))).collect(), couplings, ..Loads::default() };
    let tip = by_x("top_v1");
    (model, bc, loads, tip)
}

fn tip_of(k0: f64, offset: f64) -> f64 {
    let (model, bc, loads, tip) = blocks(k0, offset);
    let sol = model.solve_nonlinear(&loads, &bc, &NlOptions::default()).unwrap();
    assert!(sol.complete(), "{:?}", sol.stop);
    // every node of the free edge moves alike: the state is uniform
    let uy: Vec<f64> = tip.iter().map(|&n| sol.u[2 * n + 1]).collect();
    let (lo, hi) = uy.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |a, &v| (a.0.min(v), a.1.max(v)));
    assert!((hi - lo).abs() < 1e-9 * hi.abs(), "{lo:e} {hi:e}");
    hi
}

#[test]
fn a_coupling_is_a_spring_between_two_edges_and_holds_the_body_it_joins() {
    // a pure spring: the bodies' own shortening plus sigma / k0
    let k0 = 4.0 * E;
    let want = 2.0 * SIGMA / E + SIGMA / k0;
    let got = tip_of(k0, 0.0);
    assert!((got / want - 1.0).abs() < 1e-9, "{got:e} vs {want:e}");
}

#[test]
fn a_coupling_offset_is_the_relative_displacement_at_which_it_is_unloaded() {
    let (k0, offset) = (4.0 * E, 2.5e-4);
    let want = 2.0 * SIGMA / E + SIGMA / k0 + offset;
    let got = tip_of(k0, offset);
    assert!((got / want - 1.0).abs() < 1e-9, "{got:e} vs {want:e}");
    // with no load the block B it holds is unstressed (the spring force must vanish, so the relative displacement is the offset):
    // B moves rigidly up by the offset and A stays where it is
    let (model, bc, mut loads, tip) = blocks(k0, offset);
    loads.faces.clear();
    let sol = model.solve_nonlinear(&loads, &bc, &NlOptions::default()).unwrap();
    for &n in &tip {
        assert!((sol.u[2 * n + 1] / offset - 1.0).abs() < 1e-9, "{:e} vs {offset:e}", sol.u[2 * n + 1]);
    }
    assert!(sol.u[..2 * model.mesh.node_set("v1").unwrap().len()].iter().all(|v| v.abs() < 1e-9 * offset));
}

#[test]
fn a_coupling_at_a_large_stiffness_is_a_bonded_joint() {
    // the penalty limit: the stack of the two blocks is one bar of length 2
    let got = tip_of(1.0e6 * E, 0.0);
    let want = 2.0 * SIGMA / E;
    assert!((got / want - 1.0).abs() < 1e-5, "{got:e} vs {want:e}");
}

#[test]
fn a_coupling_that_names_a_missing_dof_is_refused() {
    let (model, bc, mut loads, _) = blocks(E, 0.0);
    loads.couplings[0].terms[0].0 = 10_000_000;
    let err = model.solve_nonlinear(&loads, &bc, &NlOptions::default()).err().expect("refused");
    assert!(err.contains("coupling"), "{err}");
}
