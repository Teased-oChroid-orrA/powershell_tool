//! Multi-point constraints and reference-point rigid coupling against beam theory (exact when
//! `nu = 0`: pure bending and axial stretch lie in the quadratic displacement space).

#![allow(clippy::needless_range_loop)] // index-parallel numeric kernels read clearer as loops

use fea_core::generate::grid;
use fea_core::*;

const E: f64 = 10.0e6;

/// Cantilever along `x` with the clamped face `u0` and a rigid end face `u1` coupled to a reference
/// point at the tip centroid; returns the model, its constraints and the reference point.
fn rigid_tip_beam(kind: ElementKind, physics: Physics, nu: f64, l: f64, h: f64, w: f64, div: [usize; 3]) -> (Model, Constraints, RefPoint) {
    let d = physics.dim();
    let mesh = grid(physics, kind, Elastic::new(E, nu), div, &move |p| [l * p[0], h * (p[1] - 0.5), if d == 3 { w * (p[2] - 0.5) } else { 0.0 }]).unwrap();
    let model = Model::new(mesh).unwrap();
    let mut cons = Constraints::new();
    let rp = cons.add_ref_point(&model.mesh, [l, 0.0, 0.0]);
    let tip: Vec<usize> = model.mesh.node_set("u1").unwrap().to_vec();
    cons.rigid(&model.mesh, &rp, &tip);
    (model, cons, rp)
}

fn clamp(model: &Model) -> Dirichlet {
    let mut bc = model.dirichlet();
    for &n in model.mesh.node_set("u0").unwrap() {
        bc.fix_node(n);
    }
    bc
}

#[test]
fn rigid_tip_moment_gives_exact_pure_bending_3d_and_2d() {
    let (l, h, w, m) = (10.0, 1.0, 0.6, 250.0);
    // 3D Hex27, nu = 0: theta = M L / (E I), tip deflection M L^2 / (2 E I).
    let (model, cons, rp) = rigid_tip_beam(ElementKind::Hex27, Physics::Solid, 0.0, l, h, w, [3, 1, 1]);
    let bc = clamp(&model);
    let sol = model.solve_static_constrained(&Loads::default(), &bc, &cons, &[(rp.rot(2), m)]).unwrap();
    let ei = E * w * h.powi(3) / 12.0;
    let (theta, defl) = (sol.extra[rp.rot(2) - model.mesh.n_dofs()], sol.extra[rp.trans(1) - model.mesh.n_dofs()]);
    eprintln!("3D: theta {theta:.9e} (exact {:.9e}), tip deflection {defl:.9e} (exact {:.9e})", m * l / ei, m * l * l / (2.0 * ei));
    assert!((theta - m * l / ei).abs() < 1e-9 * theta.abs(), "rotation {theta}");
    assert!((defl - m * l * l / (2.0 * ei)).abs() < 1e-9 * defl.abs(), "deflection {defl}");
    // The clamp carries the applied moment back: sum (x r_y - y r_x) = -M, no net force.
    let r = &sol.reactions;
    let (mut fy, mut fx, mut mz) = (0.0, 0.0, 0.0);
    for (n, x) in model.mesh.nodes.iter().enumerate() {
        fx += r[n * 3];
        fy += r[n * 3 + 1];
        mz += x[0] * r[n * 3 + 1] - x[1] * r[n * 3];
    }
    assert!(fx.abs() < 1e-8 * m && fy.abs() < 1e-8 * m, "net reaction ({fx}, {fy})");
    assert!((mz + m).abs() < 1e-8 * m, "moment reaction {mz}");
    // Every end-face node follows the rigid motion exactly.
    for &n in model.mesh.node_set("u1").unwrap() {
        let x = model.mesh.nodes[n];
        let err = (sol.u[n * 3] - (-theta * x[1])).abs();
        assert!(err < 1e-12 * theta.abs() * l, "end face not planar at node {n}: u_x {} vs rigid {} (err {err:e})", sol.u[n * 3], -theta * x[1]);
    }
    // 2D Quad9 plane stress, nu = 0.
    let t = 0.5;
    let (model, cons, rp) = rigid_tip_beam(ElementKind::Quad9, Physics::PlaneStress { thickness: t }, 0.0, l, h, 0.0, [3, 1, 1]);
    let bc = clamp(&model);
    let sol = model.solve_static_constrained(&Loads::default(), &bc, &cons, &[(rp.rot(0), m)]).unwrap();
    let ei = E * t * h.powi(3) / 12.0;
    let (theta, defl) = (sol.extra[rp.rot(0) - model.mesh.n_dofs()], sol.extra[rp.trans(1) - model.mesh.n_dofs()]);
    assert!((theta - m * l / ei).abs() < 1e-9 * theta.abs(), "2D rotation {theta}");
    assert!((defl - m * l * l / (2.0 * ei)).abs() < 1e-9 * defl.abs(), "2D deflection {defl}");
}

#[test]
fn rigid_tip_axial_force_and_transverse_force() {
    let (l, h, w) = (10.0, 1.0, 0.6);
    let a = h * w;
    let (model, cons, rp) = rigid_tip_beam(ElementKind::Hex27, Physics::Solid, 0.0, l, h, w, [4, 1, 1]);
    let bc = clamp(&model);
    let f = 1000.0;
    let sol = model.solve_static_constrained(&Loads::default(), &bc, &cons, &[(rp.trans(0), f)]).unwrap();
    let ux = sol.extra[rp.trans(0) - model.mesh.n_dofs()];
    assert!((ux - f * l / (E * a)).abs() < 1e-9 * ux, "axial {ux} vs {}", f * l / (E * a));
    // Transverse tip force: Euler-Bernoulli bending plus shear (kappa = 5/6), nu = 0 so G = E / 2.
    let sol = model.solve_static_constrained(&Loads::default(), &bc, &cons, &[(rp.trans(1), f)]).unwrap();
    let v = sol.extra[rp.trans(1) - model.mesh.n_dofs()];
    let ei = E * w * h.powi(3) / 12.0;
    let beam = f * l.powi(3) / (3.0 * ei) + f * l / (5.0 / 6.0 * 0.5 * E * a);
    eprintln!("transverse tip deflection {v:.6e}, Timoshenko beam {beam:.6e} ({:+.2}%)", 100.0 * (v / beam - 1.0));
    assert!((v / beam - 1.0).abs() < 0.03, "{v} vs {beam}");
    // Equilibrium of the supports: sum of reaction forces equals the applied force.
    let fy: f64 = (0..model.mesh.nodes.len()).map(|n| sol.reactions[n * 3 + 1]).sum();
    assert!((fy + f).abs() < 1e-8 * f, "reaction sum {fy}");
}

#[test]
fn general_equations_with_chains_and_offsets_are_satisfied_and_stationary() {
    let mesh = grid(Physics::PlaneStress { thickness: 1.0 }, ElementKind::Quad9, Elastic::new(E, 0.3), [2, 2, 1], &|p| [p[0], p[1], 0.0]).unwrap();
    let model = Model::new(mesh).unwrap();
    let mut bc = model.dirichlet();
    for &n in model.mesh.node_set("u0").unwrap() {
        bc.fix_node(n);
    }
    let top: Vec<usize> = model.mesh.node_set("v1").unwrap().iter().copied().filter(|&n| !bc.fixed[n * 2]).collect();
    let (a, b, c) = (top[0], top[1], top[2]);
    let mut cons = Constraints::new();
    // u_y(c) = 2 u_y(b) + 1e-4 ; u_y(b) = u_y(a) + 5e-5 (a chain: c depends on b depends on a).
    cons.add_equation(c * 2 + 1, vec![(b * 2 + 1, 2.0)], 1e-4);
    cons.add_equation(b * 2 + 1, vec![(a * 2 + 1, 1.0)], 5e-5);
    let loads = Loads { nodal: vec![(top[1], [300.0, -800.0, 0.0])], ..Loads::default() };
    let sol = model.solve_static_constrained(&loads, &bc, &cons, &[]).unwrap();
    let uy = |n: usize| sol.u[n * 2 + 1];
    assert!((uy(b) - uy(a) - 5e-5).abs() < 1e-14, "second equation");
    assert!((uy(c) - 2.0 * uy(b) - 1e-4).abs() < 1e-14, "first equation");
    // Stationarity: the residual K u - f is a combination of the constraint forces only.
    let k = model.assemble().unwrap();
    let f = fea_core::loads::assemble(&model.mesh, &loads).unwrap();
    let mut res = vec![0.0; f.len()];
    k.matvec_add(&model.pattern, &sol.u, &mut res);
    for i in 0..res.len() {
        res[i] -= f[i];
    }
    // Constraint forces: lam1 on c-y, lam2 on b-y, mirrored onto the masters. Free dofs of the other nodes carry no residual.
    let (rc, rb, ra) = (res[c * 2 + 1], res[b * 2 + 1], res[a * 2 + 1]);
    assert!((rb + 2.0 * rc + ra).abs() < 1e-8 * (rc.abs() + rb.abs() + ra.abs()), "virtual-work balance {}", rb + 2.0 * rc + ra);
    for n in 0..model.mesh.nodes.len() {
        if ![a, b, c].contains(&n) && !bc.fixed[n * 2] {
            assert!(res[n * 2].abs() < 1e-6 * 800.0 && res[n * 2 + 1].abs() < 1e-6 * 800.0, "residual at free node {n}");
        }
    }
}

#[test]
fn bad_constraint_sets_are_rejected() {
    let mesh = grid(Physics::PlaneStress { thickness: 1.0 }, ElementKind::Quad4, Elastic::new(E, 0.3), [1, 1, 1], &|p| [p[0], p[1], 0.0]).unwrap();
    let model = Model::new(mesh).unwrap();
    let mut bc = model.dirichlet();
    bc.fix_node(0);
    bc.fix_node(1);
    // Circular chain.
    let mut cons = Constraints::new();
    cons.add_equation(4, vec![(5, 1.0)], 0.0);
    cons.add_equation(5, vec![(4, 1.0)], 0.0);
    let err = model.solve_static_constrained(&Loads::default(), &bc, &cons, &[]).err().unwrap();
    assert!(err.contains("circular"), "{err}");
    // Slave that is also prescribed.
    let mut cons = Constraints::new();
    cons.add_equation(0, vec![(5, 1.0)], 0.0);
    assert!(model.solve_static_constrained(&Loads::default(), &bc, &cons, &[]).unwrap_err().contains("prescribed"));
    // Two equations for one slave.
    let mut cons = Constraints::new();
    cons.add_equation(5, vec![(4, 1.0)], 0.0);
    cons.add_equation(5, vec![(6, 1.0)], 0.0);
    assert!(model.solve_static_constrained(&Loads::default(), &bc, &cons, &[]).unwrap_err().contains("two equations"));
    // A reference point nobody couples to is a singular system, reported rather than solved.
    let mut cons = Constraints::new();
    let _ = cons.add_ref_point(&model.mesh, [0.0; 3]);
    assert!(model.solve_static_constrained(&Loads::default(), &bc, &cons, &[]).is_err());
}
