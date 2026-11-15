//! Stress recovery must not smooth across a material interface (epic #25, section 5.1).

#![allow(clippy::needless_range_loop)] // index-parallel numeric kernels read clearer as loops

use fea_core::*;

const EPS: f64 = 1e-3;
const NU: f64 = 0.3;

/// A two-layer bar (soft below `y = 1`, stiff above) stretched uniformly along x: the finite-element solution is exact (linear
/// displacement field, continuous), the true stress jumps at the interface (`sigma_xx = E_i * EPS`) and the true error is zero.
fn layered_bar(kind: ElementKind, nx: usize, ny: usize) -> (Model, Vec<f64>) {
    let mut mesh = fea_core::generate::grid(Physics::PlaneStress { thickness: 1.0 }, kind, Elastic::new(1e7, NU), [nx, ny, 1], &|p| [4.0 * p[0], 2.0 * p[1], 0.0]).unwrap();
    let blk = mesh.blocks.remove(0);
    let (mut soft, mut stiff) = (Vec::new(), Vec::new());
    for e in 0..blk.n_elems() {
        let c = blk.elem(e);
        let y = c.iter().map(|&n| mesh.nodes[n][1]).sum::<f64>() / c.len() as f64;
        if y < 1.0 { soft.extend_from_slice(c) } else { stiff.extend_from_slice(c) }
    }
    mesh.add_block(kind, soft, Elastic::new(1e7, NU), "soft").unwrap();
    mesh.add_block(kind, stiff, Elastic::new(3e7, NU), "stiff").unwrap();
    let model = Model::new(mesh).unwrap();
    let mut bc = model.dirichlet();
    let anchor = model.mesh.nodes.iter().position(|x| x[0] < 1e-9 && x[1] < 1e-9).unwrap();
    for (n, x) in model.mesh.nodes.iter().enumerate() {
        if x[0] < 1e-9 {
            bc.fix(n, 0, 0.0);
        }
        if x[0] > 4.0 - 1e-9 {
            bc.fix(n, 0, EPS * 4.0);
        }
    }
    bc.fix(anchor, 1, 0.0);
    let sol = model.solve_static(&Loads::default(), &bc).unwrap();
    (model, sol.u)
}

#[test]
fn the_estimate_of_an_exact_layered_solution_is_zero_and_the_interface_stress_is_not_smoothed() {
    for kind in [ElementKind::Quad4, ElementKind::Quad8, ElementKind::Tri6] {
        let (model, u) = layered_bar(kind, 8, 8);
        // the discrete solution is exact: the displacement is EPS x
        for (n, x) in model.mesh.nodes.iter().enumerate() {
            assert!((u[2 * n] - EPS * x[0]).abs() < 1e-12, "{kind:?}: node {n}");
        }
        let zz = model.zz_error(&u, 0.0).unwrap();
        assert!(zz.total < 1e-6 * zz.norm, "{kind:?}: ZZ error {} against a recovered-stress norm {} for an exact solution", zz.total, zz.norm);
        // the recovered stress on either side of the interface is the true one
        let gauss = model.gauss_stresses(&u, 0.0).unwrap();
        let rec = model.recover_spr_by_material(&gauss).unwrap();
        for (bi, blk) in model.mesh.blocks.iter().enumerate() {
            let e_mod = if blk.name == "soft" { 1e7 } else { 3e7 };
            // plane stress: sigma_xx = E eps, sigma_yy = 0
            for &n in &blk.conn {
                let s = rec.at(bi, n);
                assert!((s[0] - e_mod * EPS).abs() < 1e-6 * e_mod * EPS, "{kind:?} {}: node {n} sigma_xx {} vs {}", blk.name, s[0], e_mod * EPS);
                assert!(s[1].abs() < 1e-6 * e_mod * EPS);
            }
        }
        assert!(rec.interface_nodes > 0);
    }
}

#[test]
fn a_single_material_mesh_gives_the_plain_spr_result_bit_for_bit() {
    let mesh = fea_core::generate::grid(Physics::PlaneStress { thickness: 1.0 }, ElementKind::Quad8, Elastic::new(1e7, NU), [6, 4, 1], &|p| [3.0 * p[0], 2.0 * p[1], 0.0]).unwrap();
    let model = Model::new(mesh).unwrap();
    let mut bc = model.dirichlet();
    for &n in model.mesh.node_set("u0").unwrap() {
        bc.fix(n, 0, 0.0);
        bc.fix(n, 1, 0.0);
    }
    let loads = Loads { faces: model.mesh.surfaces["u1"].iter().map(|f| (f.clone(), SurfaceLoad::Traction([0.0, -50.0, 0.0]))).collect(), ..Loads::default() };
    let u = model.solve_static(&loads, &bc).unwrap().u;
    let gauss = model.gauss_stresses(&u, 0.0).unwrap();
    let plain = model.recover_spr(&gauss).unwrap();
    let rec = model.recover_spr_by_material(&gauss).unwrap();
    for n in 0..model.mesh.nodes.len() {
        assert_eq!(rec.at(0, n), plain[n], "node {n}");
    }
    assert_eq!(rec.interface_nodes, 0);
    assert!(rec.diagnostics.corner_nodes > 0 && rec.diagnostics.fitted + rec.diagnostics.borrowed + rec.diagnostics.lower_degree + rec.diagnostics.averaged == rec.diagnostics.corner_nodes);
}

#[test]
fn plain_spr_does_smooth_across_the_interface_which_is_why_the_material_aware_recovery_exists() {
    let (model, u) = layered_bar(ElementKind::Quad8, 8, 8);
    let gauss = model.gauss_stresses(&u, 0.0).unwrap();
    let plain = model.recover_spr(&gauss).unwrap();
    let mut worst = 0.0f64;
    for (n, x) in model.mesh.nodes.iter().enumerate() {
        if (x[1] - 1.0).abs() < 1e-9 {
            // on the interface the true stress is 1e7 * EPS below and 3e7 * EPS above; the plain recovery blends them
            worst = worst.max((plain[n][0] - 1e7 * EPS).abs().min((plain[n][0] - 3e7 * EPS).abs()) / (1e7 * EPS));
        }
    }
    assert!(worst > 0.2, "plain SPR kept the jump (deviation {worst}): the premise of the test is gone");
}
