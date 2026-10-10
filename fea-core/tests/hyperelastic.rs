//! Compressible neo-Hookean hyperelasticity: the tangent against finite differences, the small-strain limit against linear
//! elasticity, and uniaxial tension (free lateral faces) against the closed form.

use fea_core::generate::grid;
use fea_core::material::{neo_hookean_update, Moduli, J2};
use fea_core::nonlinear::Stop;
use fea_core::*;

const E: f64 = 1.0e7;
const NU: f64 = 0.3;

#[test]
fn the_neo_hookean_tangent_matches_finite_differences() {
    let mo = Moduli::new(E, NU);
    let f = [[0.3, 0.2, -0.1], [0.05, -0.1, 0.15], [-0.1, 0.07, 0.1]]; // the displacement gradient H
    let u = neo_hookean_update(mo, &f, &[1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0]).unwrap();
    let h = 1e-6;
    for k in 0..3 {
        for l in 0..3 {
            let (mut fp, mut fm) = (f, f);
            fp[k][l] += h;
            fm[k][l] -= h;
            let sp = neo_hookean_update(mo, &fp, &[0.0; 7]).unwrap().stress;
            let sm = neo_hookean_update(mo, &fm, &[0.0; 7]).unwrap().stress;
            for i in 0..3 {
                for j in 0..3 {
                    let fd = (sp[i][j] - sm[i][j]) / (2.0 * h);
                    let an = u.tangent[3 * i + j][3 * k + l];
                    assert!((fd - an).abs() < 1e-5 * (1.0 + an.abs()), "A[{i}{j}{k}{l}]: {an} vs {fd}");
                }
            }
        }
    }
    // The reference state is stress free.
    let r = neo_hookean_update(mo, &[[0.0; 3]; 3], &[0.0; 7]).unwrap();
    assert!(r.stress.iter().flatten().all(|v| *v == 0.0));
    assert!(neo_hookean_update(mo, &[[-2.0, 0.0, 0.0], [0.0, 0.0, 0.0], [0.0, 0.0, 0.0]], &[0.0; 7]).is_none(), "an inverted stretch");
    // A tiny strain keeps its digits: sigma = lambda tr(H) I + G (H + H^T) to first order, with a relative error far below G eps / sigma.
    let tiny = [[1e-9, 2e-9, 0.0], [0.0, -3e-9, 0.0], [0.0, 0.0, 5e-10]];
    let s = neo_hookean_update(mo, &tiny, &[0.0; 7]).unwrap().stress;
    let lam = mo.k - 2.0 * mo.g / 3.0;
    let tr = tiny[0][0] + tiny[1][1] + tiny[2][2];
    let lin01 = mo.g * (tiny[0][1] + tiny[1][0]);
    assert!((s[0][1] - lin01).abs() < 1e-6 * lin01.abs(), "{} vs {lin01}", s[0][1]);
    assert!((s[0][0] - (lam * tr + 2.0 * mo.g * tiny[0][0])).abs() < 1e-6 * s[0][0].abs());
}

/// Uniaxial stretch `s` of a block, lateral faces free, symmetry planes on the low faces; returns the axial first Piola-Kirchhoff
/// stress (force over reference area) and the lateral stretch.
fn tension(s: f64, hyper: bool) -> (f64, f64, usize) {
    let mut mesh = grid(Physics::Solid, ElementKind::Hex20, Elastic::new(E, NU), [2, 2, 2], &|p| p).unwrap();
    if hyper {
        mesh.set_plasticity(0, J2::neo_hookean()).unwrap();
    }
    let model = Model::new(mesh).unwrap();
    let mut bc = model.dirichlet();
    for (i, x) in model.mesh.nodes.iter().enumerate() {
        for (c, xc) in x.iter().enumerate() {
            if *xc < 1e-12 {
                bc.fix(i, c, 0.0);
            }
        }
        if (x[0] - 1.0).abs() < 1e-12 {
            bc.fix(i, 0, s - 1.0);
        }
    }
    let sol = model.solve_nonlinear(&Loads::default(), &bc, &NlOptions { steps: 4, tol: 1e-10, ..NlOptions::default() }).unwrap();
    assert_eq!(sol.stop, Stop::Completed);
    let force: f64 = -(0..model.mesh.nodes.len()).filter(|&i| model.mesh.nodes[i][0] < 1e-12).map(|i| sol.reactions[i * 3]).sum::<f64>();
    let free = (0..model.mesh.nodes.len()).find(|&i| (model.mesh.nodes[i][1] - 1.0).abs() < 1e-12 && model.mesh.nodes[i][0] < 1e-12 && model.mesh.nodes[i][2] < 1e-12).unwrap();
    (force, 1.0 + sol.u[free * 3 + 1], sol.steps.iter().map(|s| s.iterations).sum())
}

#[test]
fn uniaxial_tension_follows_the_closed_form() {
    let mo = Moduli::new(E, NU);
    let (g, lam) = (mo.g, mo.k - 2.0 * mo.g / 3.0);
    for s in [1.05f64, 1.3, 1.8] {
        // Lateral stretch l from P_22 = 0: G (l - 1/l) + lambda ln(s l^2) / l = 0 (Newton on l).
        let mut l = 1.0 / s.sqrt();
        for _ in 0..60 {
            let f = g * (l - 1.0 / l) + lam * (s * l * l).ln() / l;
            let df = g * (1.0 + 1.0 / (l * l)) + lam * (2.0 / l * 1.0 / l - (s * l * l).ln() / (l * l));
            l -= f / df;
        }
        let p1 = g * (s - 1.0 / s) + lam * (s * l * l).ln() / s;
        let (force, lateral, _) = tension(s, true);
        assert!((force / p1 - 1.0).abs() < 1e-6, "stretch {s}: force {force} vs {p1}");
        assert!((lateral / l - 1.0).abs() < 1e-6, "stretch {s}: lateral {lateral} vs {l}");
    }
}

#[test]
fn the_small_strain_limit_is_linear_elasticity() {
    let (nh, _, _) = tension(1.0 + 1e-6, true);
    let (lin, _, _) = tension(1.0 + 1e-6, false);
    assert!((nh / lin - 1.0).abs() < 1e-4, "{nh} vs {lin}");
}

#[test]
fn newton_converges_quadratically_with_the_consistent_tangent() {
    let (_, _, iterations) = tension(1.8, true);
    assert!(iterations <= 4 * 6, "4 steps in at most 6 iterations each: {iterations}");
}

#[test]
fn plane_stress_is_refused() {
    let mut mesh = grid(Physics::PlaneStress { thickness: 1.0 }, ElementKind::Quad4, Elastic::new(E, NU), [1, 1, 1], &|p| p).unwrap();
    assert!(mesh.set_plasticity(0, J2::neo_hookean()).is_err());
}

#[test]
fn rigid_rotation_is_stress_free_and_rotated_stretch_is_objective() {
    // Independent constitutive reference: F=R U, P(R U)=R P(U), with proper R.
    let mo = Moduli::new(E, NU);
    let angle = 0.73_f64;
    let (s, c) = angle.sin_cos();
    let r = [[c, -s, 0.0], [s, c, 0.0], [0.0, 0.0, 1.0]];
    let stretch = [1.2, 0.9, 1.05];
    let mut hr = r;
    let mut hu = [[0.0; 3]; 3];
    let mut hru = [[0.0; 3]; 3];
    for i in 0..3 {
        hr[i][i] -= 1.0;
        hu[i][i] = stretch[i] - 1.0;
        for j in 0..3 { hru[i][j] = r[i][j] * stretch[j] - if i == j { 1.0 } else { 0.0 }; }
    }
    let rotation = neo_hookean_update(mo, &hr, &[0.0; 7]).unwrap();
    assert!(rotation.stress.iter().flatten().all(|v| v.abs() < 1e-8));
    let base = neo_hookean_update(mo, &hu, &[0.0; 7]).unwrap();
    let rotated = neo_hookean_update(mo, &hru, &[0.0; 7]).unwrap();
    for i in 0..3 { for j in 0..3 {
        let expected: f64 = (0..3).map(|k| r[i][k] * base.stress[k][j]).sum();
        assert!((rotated.stress[i][j] - expected).abs() < 1e-8 * E);
    }}
}
