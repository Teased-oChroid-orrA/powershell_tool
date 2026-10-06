//! Anisotropic elasticity: equivalence with the isotropic kernels when the law is isotropic,
//! rotation algebra, orthotropic patch / thermal / bending checks, and frame invariance.

#![allow(clippy::needless_range_loop)] // index-parallel numeric kernels read clearer as loops

use fea_core::element::ALL_KINDS;
use fea_core::generate::grid;
use fea_core::kernel::{self, Work};
use fea_core::*;

const E: f64 = 10.0e6;
const NU: f64 = 0.3;

fn iso_d(e: f64, nu: f64) -> [[f64; 6]; 6] {
    let (lam, mu) = (e * nu / ((1.0 + nu) * (1.0 - 2.0 * nu)), e / (2.0 * (1.0 + nu)));
    let mut d = [[0.0; 6]; 6];
    for i in 0..3 {
        for j in 0..3 {
            d[i][j] = lam + if i == j { 2.0 * mu } else { 0.0 };
        }
        d[i + 3][i + 3] = mu;
    }
    d
}

fn physics_variants(kind: ElementKind) -> Vec<Physics> {
    if kind.dim() == 3 {
        vec![Physics::Solid]
    } else {
        vec![Physics::PlaneStress { thickness: 0.4 }, Physics::PlaneStrain { thickness: 0.7 }, Physics::Axisymmetric]
    }
}

/// A mildly distorted element (reference nodes through a smooth map); `+2` keeps `x > 0` for axisymmetry.
fn distorted(kind: ElementKind) -> Vec<[f64; 3]> {
    kind.node_coords().iter().map(|x| [2.0 + 0.5 * x[0] + 0.07 * x[1] + 0.03 * x[2] * x[0], 0.4 * x[1] + 0.05 * x[0] * x[0] + 0.02 * x[2], 0.6 * x[2] + 0.04 * x[0] - 0.03 * x[1]]).collect()
}

#[test]
fn isotropic_law_through_the_anisotropic_path_equals_the_isotropic_kernel() {
    let iso = Elastic::new(E, NU).with_alpha(1.2e-5);
    let an = Elastic { aniso: Elastic::anisotropic(iso_d(E, NU)).unwrap().aniso, ..iso };
    for kind in ALL_KINDS {
        for physics in physics_variants(kind) {
            let xyz = distorted(kind);
            let (nn, d) = (kind.n_nodes(), kind.dim());
            let (mut work, mut k1, mut k2) = (Work::new(), vec![0.0; nn * nn * d * d], vec![0.0; nn * nn * d * d]);
            kernel::stiffness(kind, physics, &iso, &xyz, &mut work, &mut k1).unwrap();
            kernel::stiffness(kind, physics, &an, &xyz, &mut work, &mut k2).unwrap();
            let scale = k1.iter().fold(0.0f64, |m, v| m.max(v.abs()));
            let diff = k1.iter().zip(&k2).fold(0.0f64, |m, (a, b)| m.max((a - b).abs())) / scale;
            assert!(diff < 1e-12, "{kind:?} {physics:?}: stiffness differs by {diff:e}");
            // Thermal load.
            let (mut f1, mut f2) = (vec![0.0; nn * d], vec![0.0; nn * d]);
            kernel::thermal_load(kind, physics, &iso, 25.0, &xyz, &mut work, &mut f1).unwrap();
            kernel::thermal_load(kind, physics, &an, 25.0, &xyz, &mut work, &mut f2).unwrap();
            let fs = f1.iter().fold(0.0f64, |m, v| m.max(v.abs()));
            let fd = f1.iter().zip(&f2).fold(0.0f64, |m, (a, b)| m.max((a - b).abs())) / fs;
            assert!(fd < 1e-12, "{kind:?} {physics:?}: thermal load differs by {fd:e}");
            // Stress from an arbitrary strain.
            let strain = [1.1e-3, -4.0e-4, if matches!(physics, Physics::Axisymmetric) { 3.0e-4 } else { 0.0 }, 7.0e-4, if kind.dim() == 3 { 2.0e-4 } else { 0.0 }, if kind.dim() == 3 { -5.0e-4 } else { 0.0 }];
            let (s1, s2) = (kernel::stress_from_strain(physics, &iso, strain, 15.0), kernel::stress_from_strain(physics, &an, strain, 15.0));
            for c in 0..6 {
                assert!((s1[c] - s2[c]).abs() < 1e-9 * (1.0 + s1[c].abs()), "{kind:?} {physics:?}: stress comp {c}: {} vs {}", s1[c], s2[c]);
            }
        }
    }
}

#[test]
fn orthotropic_constants_rotation_and_directional_modulus() {
    let m = Elastic::orthotropic(140e9, 10e9, 10e9, 0.3, 0.3, 0.45, 5e9, 3.5e9, 5e9).unwrap();
    let an = m.aniso.unwrap();
    // Compliance gives back the engineering constants.
    assert!((1.0 / an.c[0][0] - 140e9).abs() < 1e-3 * 140e9 * 1e-9 + 1.0 && (an.c[0][1] + 0.3 / 140e9).abs() < 1e-22);
    // 90 degrees swaps the x and y axes; +phi then -phi is the identity; isotropy is invariant.
    let r90 = m.rotated_z(std::f64::consts::FRAC_PI_2).unwrap().aniso.unwrap();
    assert!((r90.d[0][0] - an.d[1][1]).abs() < 1e-6 * an.d[1][1] && (r90.d[1][1] - an.d[0][0]).abs() < 1e-6 * an.d[0][0]);
    assert!((r90.d[3][3] - an.d[3][3]).abs() < 1e-6 * an.d[3][3], "G12 unchanged");
    assert!((r90.d[4][4] - an.d[5][5]).abs() < 1e-6 * an.d[5][5] && (r90.d[5][5] - an.d[4][4]).abs() < 1e-6 * an.d[4][4], "G23 and G13 swap");
    let back = m.rotated_z(0.7).unwrap().rotated_z(-0.7).unwrap().aniso.unwrap();
    for i in 0..6 {
        for j in 0..6 {
            assert!((back.d[i][j] - an.d[i][j]).abs() < 1e-9 * an.d[0][0], "round trip ({i},{j})");
        }
    }
    let isotropic = Elastic::anisotropic(iso_d(E, NU)).unwrap();
    let ri = isotropic.rotated_z(0.4).unwrap().aniso.unwrap();
    for i in 0..6 {
        for j in 0..6 {
            assert!((ri.d[i][j] - iso_d(E, NU)[i][j]).abs() < 1e-6, "isotropic rotation invariance ({i},{j})");
        }
    }
    // Directional Young's modulus in the plane: 1/E(phi) = c^4/E1 + (1/G12 - 2 nu12/E1) c^2 s^2 + s^4/E2.
    for phi in [0.0f64, 0.3, 0.9, 1.4] {
        let rot = m.rotated_z(phi).unwrap().aniso.unwrap();
        let (c, s) = (phi.cos(), phi.sin());
        let want = c.powi(4) / 140e9 + (1.0 / 5e9 - 2.0 * 0.3 / 140e9) * c * c * s * s + s.powi(4) / 10e9;
        // Axis-aligned global x of the rotated material is direction -phi in the material frame; the formula is even in phi.
        assert!((rot.c[0][0] - want).abs() < 1e-9 * want, "phi {phi}: 1/E = {} vs {want}", rot.c[0][0]);
    }
    assert!(Elastic::orthotropic(1.0, 1.0, 1.0, 0.9, 0.9, 0.9, 1.0, 1.0, 1.0).is_err(), "non positive definite constants rejected");
    let mut bad = iso_d(E, NU);
    bad[0][1] += 1.0;
    assert!(Elastic::anisotropic(bad).unwrap_err().contains("symmetric"));
}

/// Mesh with a skewed affine map so no cell is a rectangle.
fn skew(p: [f64; 3]) -> [f64; 3] {
    [2.0 * p[0] + 0.3 * p[1] + 0.1 * p[2], 0.2 * p[0] + 1.5 * p[1] + 0.2 * p[2], 0.1 * p[0] - 0.1 * p[1] + 1.2 * p[2]]
}

/// Patch test: boundary displaced by a constant-strain field, interior and stress must follow.
fn patch(kind: ElementKind, physics: Physics, mat: Elastic, strain: [f64; 6]) -> (Model, Vec<f64>) {
    let d = kind.dim();
    let div = if d == 2 { [3, 3, 1] } else { [2, 2, 2] };
    let mesh = grid(physics, kind, mat, div, &skew).unwrap();
    let model = Model::new(mesh).unwrap();
    // u = H x with H the symmetric strain tensor.
    let h = [[strain[0], 0.5 * strain[3], 0.5 * strain[5]], [0.5 * strain[3], strain[1], 0.5 * strain[4]], [0.5 * strain[5], 0.5 * strain[4], strain[2]]];
    let u_of = |x: &[f64; 3], i: usize| (0..d).map(|k| h[i][k] * x[k]).sum::<f64>();
    let mut bc = model.dirichlet();
    for set in model.mesh.node_sets.values() {
        for &n in set {
            for i in 0..d {
                bc.fix(n, i, u_of(&model.mesh.nodes[n], i));
            }
        }
    }
    let sol = model.solve_static(&Loads::default(), &bc).unwrap();
    (model, sol.u)
}

#[test]
fn orthotropic_patch_tests_recover_the_exact_constant_stress() {
    let mat = Elastic::orthotropic(140e6, 10e6, 9e6, 0.3, 0.28, 0.4, 5e6, 3.5e6, 4.5e6).unwrap().rotated_z(0.5).unwrap();
    let an = mat.aniso.unwrap();
    // Solid: any strain.
    let strain = [1.0e-3, -3.0e-4, 5.0e-4, 6.0e-4, -2.0e-4, 4.0e-4];
    for kind in [ElementKind::Hex8, ElementKind::Hex20, ElementKind::Hex27, ElementKind::Tet4, ElementKind::Tet10] {
        let (model, u) = patch(kind, Physics::Solid, mat, strain);
        let want: [f64; 6] = std::array::from_fn(|i| (0..6).map(|j| an.d[i][j] * strain[j]).sum());
        let scale = want.iter().fold(0.0f64, |m, v| m.max(v.abs()));
        for blk in model.gauss_stresses(&u, 0.0).unwrap() {
            for (_, s) in blk {
                for c in 0..6 {
                    assert!((s[c] - want[c]).abs() < 1e-9 * scale, "{kind:?} comp {c}: {} vs {}", s[c], want[c]);
                }
            }
        }
    }
    // Plane strain: stress from the (xx, yy, zz, xy) block with eps_zz = 0.
    let strain = [1.0e-3, -3.0e-4, 0.0, 6.0e-4, 0.0, 0.0];
    for kind in [ElementKind::Quad4, ElementKind::Quad8, ElementKind::Quad9, ElementKind::Tri3, ElementKind::Tri6] {
        let (model, u) = patch(kind, Physics::PlaneStrain { thickness: 1.0 }, mat, strain);
        let want: [f64; 4] = std::array::from_fn(|i| (0..4).map(|j| an.d[i][j] * strain[j]).sum());
        let scale = want.iter().fold(0.0f64, |m, v| m.max(v.abs()));
        for blk in model.gauss_stresses(&u, 0.0).unwrap() {
            for (_, s) in blk {
                for c in 0..4 {
                    assert!((s[c] - want[c]).abs() < 1e-9 * scale, "plane strain {kind:?} comp {c}: {} vs {}", s[c], want[c]);
                }
            }
        }
    }
    // Plane stress on the unrotated material: uniaxial stress along the fibre gives eps_x = sigma / E1,
    // eps_y = -nu12 sigma / E1 and no shear (checks the sigma_zz = 0 condensation).
    let m0 = Elastic::orthotropic(140e6, 10e6, 9e6, 0.3, 0.28, 0.4, 5e6, 3.5e6, 4.5e6).unwrap();
    let sigma = 1000.0;
    let strain = [sigma / 140e6, -0.3 * sigma / 140e6, 0.0, 0.0, 0.0, 0.0];
    let (model, u) = patch(ElementKind::Quad9, Physics::PlaneStress { thickness: 0.5 }, m0, strain);
    for blk in model.gauss_stresses(&u, 0.0).unwrap() {
        for (_, s) in blk {
            assert!((s[0] - sigma).abs() < 1e-9 * sigma && s[1].abs() < 1e-9 * sigma && s[3].abs() < 1e-9 * sigma, "{s:?}");
        }
    }
}

#[test]
fn orthotropic_thermal_expansion_is_free_and_constrained_stress_is_minus_d_alpha_dt() {
    let mat = Elastic::orthotropic(140e6, 10e6, 9e6, 0.3, 0.28, 0.4, 5e6, 3.5e6, 4.5e6).unwrap().rotated_z(0.35).unwrap().with_alpha(1.0e-5);
    let an = mat.aniso.unwrap();
    let dt = 80.0;
    for kind in [ElementKind::Hex8, ElementKind::Hex27, ElementKind::Tet10] {
        let mesh = grid(Physics::Solid, kind, mat, [2, 2, 2], &skew).unwrap();
        let model = Model::new(mesh).unwrap();
        // Free expansion: only rigid-body constraints, u = alpha dT x, stress-free.
        let mut bc = model.dirichlet();
        for &n in model.mesh.node_set("u0").unwrap() {
            // clamp one face without over-constraining: fix by the exact expansion field
            let x = model.mesh.nodes[n];
            for i in 0..3 {
                bc.fix(n, i, mat.alpha * dt * x[i]);
            }
        }
        let loads = Loads { delta_t: dt, ..Loads::default() };
        let sol = model.solve_static(&loads, &bc).unwrap();
        for (n, x) in model.mesh.nodes.iter().enumerate() {
            for i in 0..3 {
                assert!((sol.u[n * 3 + i] - mat.alpha * dt * x[i]).abs() < 1e-9 * mat.alpha * dt * 5.0, "{kind:?}: free expansion field");
            }
        }
        for blk in model.gauss_stresses(&sol.u, dt).unwrap() {
            for (_, s) in blk {
                assert!(s.iter().all(|v| v.abs() < 1e-6 * an.d[0][0] * mat.alpha * dt), "{kind:?}: free expansion should be stress free: {s:?}");
            }
        }
        // Fully constrained: sigma = -D alpha dT [1,1,1,0,0,0].
        let mut bc = model.dirichlet();
        for n in 0..model.mesh.nodes.len() {
            bc.fix_node(n);
        }
        // Leave one node free so the system is not empty: instead check stresses from zero displacement.
        bc.fixed[0] = false;
        bc.fixed[1] = false;
        bc.fixed[2] = false;
        let sol = model.solve_static(&loads, &bc).unwrap();
        let want: [f64; 6] = std::array::from_fn(|i| -(0..3).map(|j| an.d[i][j]).sum::<f64>() * mat.alpha * dt);
        // An element not touching the free node has the exact fully constrained stress.
        let blk0 = &model.mesh.blocks[0];
        let nn = blk0.kind.n_nodes();
        let g = model.gauss_stresses(&sol.u, dt).unwrap();
        let ngp = blk0.kind.table().ngp;
        let mut checked = 0;
        for e in 0..blk0.n_elems() {
            if !blk0.elem(e).contains(&0) {
                for gp in 0..ngp {
                    let s = g[0][e * ngp + gp].1;
                    for c in 0..6 {
                        assert!((s[c] - want[c]).abs() < 1e-8 * want[0].abs().max(want[1].abs()), "{kind:?}: constrained stress comp {c}: {} vs {}", s[c], want[c]);
                    }
                }
                checked += 1;
            }
        }
        assert!(checked > 0 && nn > 0);
    }
}

#[test]
fn orthotropic_beam_in_pure_bending_is_exact_with_zero_poisson_coupling() {
    // Hex27 beam along the stiff fibre direction, nu12 = nu13 = 0, rigid tip: theta = M L / (E1 I) exactly.
    let (l, h, w, m) = (10.0, 1.0, 0.6, 250.0);
    let e1 = 140e6;
    let mat = Elastic::orthotropic(e1, 10e6, 9e6, 0.0, 0.0, 0.4, 5e6, 3.5e6, 4.5e6).unwrap();
    let mesh = grid(Physics::Solid, ElementKind::Hex27, mat, [3, 1, 1], &move |p| [l * p[0], h * (p[1] - 0.5), w * (p[2] - 0.5)]).unwrap();
    let model = Model::new(mesh).unwrap();
    let mut cons = Constraints::new();
    let rp = cons.add_ref_point(&model.mesh, [l, 0.0, 0.0]);
    let tip: Vec<usize> = model.mesh.node_set("u1").unwrap().to_vec();
    cons.rigid(&model.mesh, &rp, &tip);
    let mut bc = model.dirichlet();
    for &n in model.mesh.node_set("u0").unwrap() {
        bc.fix_node(n);
    }
    let sol = model.solve_static_constrained(&Loads::default(), &bc, &cons, &[(rp.rot(2), m)]).unwrap();
    let ei = e1 * w * h.powi(3) / 12.0;
    let theta = sol.extra[rp.rot(2) - model.mesh.n_dofs()];
    assert!((theta - m * l / ei).abs() < 1e-9 * theta.abs(), "rotation {theta} vs {}", m * l / ei);
}

#[test]
fn rotating_mesh_and_material_together_leaves_energy_and_displacements_invariant() {
    // Plane stress plate, fixed left edge, point loads at two free-edge nodes; the same problem rotated by phi.
    let build = |phi: f64| -> (f64, Vec<[f64; 2]>) {
        let mat0 = Elastic::orthotropic(140e6, 10e6, 10e6, 0.3, 0.3, 0.4, 5e6, 3.5e6, 5e6).unwrap();
        let mat = if phi == 0.0 { mat0 } else { mat0.rotated_z(phi).unwrap() };
        let (c, s) = (phi.cos(), phi.sin());
        let rot = move |v: [f64; 2]| [c * v[0] - s * v[1], s * v[0] + c * v[1]];
        let mesh = grid(Physics::PlaneStress { thickness: 0.3 }, ElementKind::Quad9, mat, [5, 3, 1], &move |p| {
            let r = rot([4.0 * p[0], 2.0 * p[1]]);
            [r[0], r[1], 0.0]
        })
        .unwrap();
        let model = Model::new(mesh).unwrap();
        let mut bc = model.dirichlet();
        for &n in model.mesh.node_set("u0").unwrap() {
            bc.fix_node(n);
        }
        let right: Vec<usize> = model.mesh.node_set("u1").unwrap().to_vec();
        let (f1, f2) = (rot([300.0, -700.0]), rot([-150.0, 400.0]));
        let loads = Loads { nodal: vec![(right[0], [f1[0], f1[1], 0.0]), (*right.last().unwrap(), [f2[0], f2[1], 0.0])], ..Loads::default() };
        let sol = model.solve_static(&loads, &bc).unwrap();
        let e = model.strain_energy(&sol.u).unwrap();
        let out = (0..model.mesh.nodes.len()).map(|n| [c * sol.u[n * 2] + s * sol.u[n * 2 + 1], -s * sol.u[n * 2] + c * sol.u[n * 2 + 1]]).collect();
        (e, out)
    };
    let (e0, u0) = build(0.0);
    for phi in [0.4f64, 1.1, -0.8, 2.5] {
        let (e1, u1) = build(phi);
        assert!((e1 - e0).abs() < 1e-9 * e0, "phi {phi}: energy {e1} vs {e0}");
        let scale = u0.iter().fold(0.0f64, |m, v| m.max(v[0].abs()).max(v[1].abs()));
        for (a, b) in u0.iter().zip(&u1) {
            assert!((a[0] - b[0]).abs() < 1e-9 * scale && (a[1] - b[1]).abs() < 1e-9 * scale, "phi {phi}: displacement {a:?} vs {b:?}");
        }
    }
}
