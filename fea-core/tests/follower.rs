//! Follower (non-dead) pressure: the load stiffness against finite differences, and the exact large-deformation state a
//! follower pressure produces (a homogeneous hydrostatic compression of a Hencky body, `Cauchy = -p I`), which a dead
//! pressure does not.

use fea_core::generate::grid;
use fea_core::loads::{follower_face, Loads, SurfaceLoad};
use fea_core::material::J2;
use fea_core::*;

const E: f64 = 1.0e7;
const NU: f64 = 0.3;

/// `dg/du` of a follower face equals the central difference of its force, 2D edge and 3D faces.
#[test]
fn the_follower_load_stiffness_is_the_derivative_of_the_follower_force() {
    for (physics, kind, div, map) in [
        (Physics::PlaneStrain { thickness: 0.7 }, ElementKind::Quad9, [2, 2, 1], Box::new(|p: [f64; 3]| [p[0] * 1.3 + 0.1 * p[1], p[1], 0.0]) as Box<dyn Fn([f64; 3]) -> [f64; 3]>),
        (Physics::Solid, ElementKind::Hex20, [1, 1, 1], Box::new(|p: [f64; 3]| [p[0] + 0.1 * p[2], p[1] * 1.2, p[2]])),
        (Physics::Solid, ElementKind::Hex27, [1, 1, 1], Box::new(|p: [f64; 3]| [p[0], p[1], p[2] * 0.8])),
    ] {
        let mut mesh = grid(physics, kind, Elastic::new(E, NU), div, &*map).unwrap();
        let top = mesh.select_faces("top", |p| if physics.dim() == 2 { p[1] > 0.99 * map([0.0, 1.0, 0.0])[1] } else { p[2] > 0.99 * map([0.0, 0.0, 1.0])[2] }).to_vec();
        assert!(!top.is_empty());
        let face = top[0].clone();
        let d = mesh.dim();
        let u0: Vec<f64> = (0..mesh.n_dofs()).map(|i| 0.03 * ((i * 17 + 3) as f64 * 0.37).sin()).collect();
        let (_, jac) = follower_face(&mesh, &face, 123.0, &u0).unwrap();
        let nd = face.len() * d;
        let dofs: Vec<usize> = face.iter().flat_map(|&n| (0..d).map(move |c| n * d + c)).collect();
        for col in 0..nd {
            let h = 1e-6;
            let (mut up, mut um) = (u0.clone(), u0.clone());
            up[dofs[col]] += h;
            um[dofs[col]] -= h;
            let (gp, _) = follower_face(&mesh, &face, 123.0, &up).unwrap();
            let (gm, _) = follower_face(&mesh, &face, 123.0, &um).unwrap();
            for row in 0..nd {
                let fd = (gp[row] - gm[row]) / (2.0 * h);
                let scale = jac.iter().fold(0.0f64, |m, v| m.max(v.abs())).max(1e-300);
                assert!((fd - jac[row * nd + col]).abs() < 1e-6 * scale, "{kind:?} J[{row}][{col}] {} vs FD {fd}", jac[row * nd + col]);
            }
        }
    }
}

/// Hencky hydrostatic state: `(n lambda_L + 2 mu) ln(s) = -p s^n` for the stretch `s` (n = 2 plane strain, 3 solid).
fn hencky_stretch(p: f64, n: i32, plane_strain: bool) -> f64 {
    let lam = E * NU / ((1.0 + NU) * (1.0 - 2.0 * NU));
    let mu = E / (2.0 * (1.0 + NU));
    // plane strain: tau_1 = (2 lam + 2 mu) ln s (eps_3 = 0); solid: (3 lam + 2 mu) ln s.
    let k = if plane_strain { 2.0 * lam + 2.0 * mu } else { 3.0 * lam + 2.0 * mu };
    let mut s = 0.9f64;
    for _ in 0..60 {
        let f = k * s.ln() + p * s.powi(n);
        let df = k / s + n as f64 * p * s.powi(n - 1);
        s -= f / df;
    }
    s
}

fn compress(physics: Physics, kind: ElementKind, follower: bool, p: f64) -> f64 {
    let d = physics.dim();
    let mut mesh = grid(physics, kind, Elastic::new(E, NU), if d == 2 { [2, 2, 1] } else { [2, 2, 2] }, &|q| q).unwrap();
    mesh.set_plasticity(0, J2::elastic_finite()).unwrap();
    let faces: Vec<Vec<Vec<usize>>> = (0..d).map(|c| mesh.select_faces(&format!("far{c}"), move |x| x[c] > 1.0 - 1e-12).to_vec()).collect();
    let model = Model::new(mesh).unwrap();
    let mut bc = model.dirichlet();
    for n in 0..model.mesh.nodes.len() {
        for c in 0..d {
            if model.mesh.nodes[n][c] < 1e-12 {
                bc.fix(n, c, 0.0); // symmetry planes
            }
        }
    }
    let all: Vec<Vec<usize>> = faces.into_iter().flatten().collect();
    let loads = if follower {
        Loads { followers: all.into_iter().map(|f| (f, p)).collect(), ..Loads::default() }
    } else {
        Loads { faces: all.into_iter().map(|f| (f, SurfaceLoad::Pressure(p))).collect(), ..Loads::default() }
    };
    let sol = model.solve_nonlinear(&loads, &bc, &NlOptions { steps: 6, tol: 1e-10, ..NlOptions::default() }).unwrap();
    assert!(sol.complete(), "{:?}", sol.stop);
    let far = (0..model.mesh.nodes.len()).find(|&n| model.mesh.nodes[n][..d].iter().all(|&x| (x - 1.0).abs() < 1e-12)).unwrap();
    1.0 + sol.u[far * d]
}

#[test]
fn a_follower_pressure_gives_the_exact_hencky_compression_that_a_dead_pressure_does_not() {
    for (physics, kind, n, plane) in [(Physics::PlaneStrain { thickness: 1.0 }, ElementKind::Quad9, 2, true), (Physics::Solid, ElementKind::Hex20, 3, false)] {
        // 30 % of the modulus: a 12 % compression, far into the finite-strain range, exact to rounding.
        let p = 0.3 * E;
        let s = hencky_stretch(p, n, plane);
        let stretch = compress(physics, kind, true, p);
        assert!((stretch - s).abs() < 1e-9, "{kind:?} follower: stretch {stretch} vs exact {s}");
        // A dead pressure does not stay normal to the shrinking surface: a different (and, this far, unstable) state.
        let p_small = 0.1 * E;
        let exact_small = hencky_stretch(p_small, n, plane);
        let dead = compress(physics, kind, false, p_small);
        assert!((dead - exact_small).abs() > 1e-3, "{kind:?}: a dead pressure must not reproduce the follower state ({dead} vs {exact_small})");
    }
}
