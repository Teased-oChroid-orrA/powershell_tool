//! Closed-form checks of every element kernel on an affinely mapped reference element: symmetry, rigid-body null space,
//! exact strain energy of an affine displacement, `stiffness_fast == stiffness`, total mass, body load, free thermal
//! expansion (`K u_th = f_th`) and conduction.

use fea_core::element::{ElementKind, ALL_KINDS};
use fea_core::kernel::{self, Layout, Work};
use fea_core::{Elastic, Physics};

const A3: [[f64; 3]; 3] = [[1.3, 0.2, -0.1], [0.15, 0.9, 0.25], [-0.05, 0.3, 1.1]];
const A2: [[f64; 3]; 3] = [[1.3, 0.25, 0.0], [-0.2, 0.9, 0.0], [0.0, 0.0, 1.0]];

fn det3(a: &[[f64; 3]; 3]) -> f64 {
    a[0][0] * (a[1][1] * a[2][2] - a[1][2] * a[2][1]) - a[0][1] * (a[1][0] * a[2][2] - a[1][2] * a[2][0]) + a[0][2] * (a[1][0] * a[2][1] - a[1][1] * a[2][0])
}

fn element(kind: ElementKind, shift: [f64; 3]) -> (Vec<[f64; 3]>, f64) {
    let a = if kind.dim() == 3 { A3 } else { A2 };
    let xyz = kind
        .node_coords()
        .iter()
        .map(|p| std::array::from_fn(|i| (0..3).map(|k| a[i][k] * p[k]).sum::<f64>() + shift[i]))
        .map(|mut x: [f64; 3]| {
            if kind.dim() == 2 {
                x[2] = 0.0;
            }
            x
        })
        .collect();
    (xyz, kind.reference_measure() * det3(&a))
}

fn full(kind: ElementKind, physics: Physics, mat: &Elastic, xyz: &[[f64; 3]], fast: bool) -> Vec<Vec<f64>> {
    let (nn, d) = (kind.n_nodes(), physics.dim());
    let mut work = Work::new();
    let mut ke = vec![0.0; 27 * 27 * 10];
    let layout = if fast { kernel::stiffness_fast(kind, physics, mat, xyz, &mut work, &mut ke).unwrap() } else {
        kernel::stiffness(kind, physics, mat, xyz, &mut work, &mut ke).unwrap();
        Layout::NodeBlock
    };
    let mut k = vec![vec![0.0; nn * d]; nn * d];
    for a in 0..nn {
        for b in 0..nn {
            for i in 0..d {
                for j in 0..d {
                    k[a * d + i][b * d + j] = kernel::entry(layout, &ke, nn, d, a, b, i, j);
                }
            }
        }
    }
    k
}

fn mul(k: &[Vec<f64>], u: &[f64]) -> Vec<f64> {
    k.iter().map(|r| r.iter().zip(u).map(|(a, b)| a * b).sum()).collect()
}

fn norm(k: &[Vec<f64>]) -> f64 {
    k.iter().flatten().fold(0.0f64, |m, v| m.max(v.abs()))
}

/// Affine displacement gradient `h[i][j] = du_i/dx_j`.
fn affine(xyz: &[[f64; 3]], d: usize, h: [[f64; 3]; 3]) -> Vec<f64> {
    xyz.iter().flat_map(|x| (0..d).map(move |i| (0..d).map(|j| h[i][j] * x[j]).sum::<f64>())).collect()
}

fn energy_density(physics: Physics, mat: &Elastic, h: [[f64; 3]; 3]) -> f64 {
    let (lam, mu) = (mat.lambda(physics), mat.mu());
    let d = physics.dim();
    let eps = |i: usize, j: usize| 0.5 * (h[i][j] + h[j][i]);
    let tr: f64 = (0..d).map(|i| eps(i, i)).sum();
    let mut e2 = 0.0;
    for i in 0..d {
        for j in 0..d {
            e2 += eps(i, j) * eps(i, j);
        }
    }
    lam * tr * tr + 2.0 * mu * e2 // = eps : sigma
}

#[test]
fn every_element_kernel_matches_closed_forms() {
    let mat = Elastic::new(200.0, 0.3).with_alpha(1.5e-2);
    let h = [[0.011, -0.004, 0.007], [0.003, -0.006, 0.002], [0.005, 0.009, 0.004]];
    for kind in ALL_KINDS {
        let physics_list: Vec<Physics> = if kind.dim() == 3 {
            vec![Physics::Solid]
        } else {
            vec![Physics::PlaneStress { thickness: 0.7 }, Physics::PlaneStrain { thickness: 0.7 }, Physics::Axisymmetric]
        };
        for physics in physics_list {
            let axisym = matches!(physics, Physics::Axisymmetric);
            let (xyz, vol) = element(kind, if axisym { [3.0, 0.5, 0.0] } else { [0.4, -0.2, 0.1] });
            let thick = match physics {
                Physics::PlaneStress { thickness } | Physics::PlaneStrain { thickness } => thickness,
                _ => 1.0,
            };
            let d = physics.dim();
            let nn = kind.n_nodes();
            let tag = format!("{kind:?} {physics:?}");
            let k = full(kind, physics, &mat, &xyz, false);
            let kf = full(kind, physics, &mat, &xyz, true);
            let scale = norm(&k);
            for r in 0..nn * d {
                for c in 0..nn * d {
                    assert!((k[r][c] - k[c][r]).abs() < 1e-12 * scale, "{tag}: asymmetric");
                    assert!((k[r][c] - kf[r][c]).abs() < 1e-12 * scale, "{tag}: fast != reference");
                }
            }
            // Rigid-body modes.
            let mut modes: Vec<Vec<f64>> = Vec::new();
            if axisym {
                modes.push(xyz.iter().flat_map(|_| [0.0, 1.0]).collect());
            } else {
                for i in 0..d {
                    modes.push(xyz.iter().flat_map(|_| (0..d).map(move |c| (c == i) as u8 as f64)).collect());
                }
                let mut rot = |p: usize, q: usize| {
                    let mut hh = [[0.0; 3]; 3];
                    hh[p][q] = 1.0;
                    hh[q][p] = -1.0;
                    modes.push(affine(&xyz, d, hh));
                };
                rot(0, 1);
                if d == 3 {
                    rot(1, 2);
                    rot(0, 2);
                }
            }
            for m in &modes {
                let r = mul(&k, m);
                let rn = r.iter().fold(0.0f64, |a, v| a.max(v.abs()));
                assert!(rn < 1e-10 * scale, "{tag}: rigid mode residual {rn:e}");
            }
            // Exact energy of an affine field (not axisymmetric: the hoop strain is not constant).
            if !axisym {
                let u = affine(&xyz, d, h);
                let e: f64 = u.iter().zip(mul(&k, &u)).map(|(a, b)| a * b).sum();
                let want = vol * thick * energy_density(physics, &mat, h);
                assert!((e - want).abs() < 1e-10 * want.abs(), "{tag}: energy {e} want {want}");
            }
            // Mass, body load.
            let mut work = Work::new();
            let mut me = vec![0.0; 27 * 27];
            kernel::mass(kind, physics, 2.5, &xyz, &mut work, &mut me).unwrap();
            let total: f64 = me[..nn * nn].iter().sum();
            let mut f = vec![0.0; 27 * 3];
            kernel::body_load(kind, physics, [1.0, -2.0, 0.5], &xyz, &mut work, &mut f).unwrap();
            let fy: f64 = (0..nn).map(|a| f[a * d + 1]).sum();
            if axisym {
                // r-weighted: the ratio of mass to body load is rho / b_z regardless of the radial weight.
                assert!((total / fy - 2.5 / -2.0).abs() < 1e-12, "{tag}: mass/body ratio");
            } else {
                assert!((total - 2.5 * vol * thick).abs() < 1e-12 * total, "{tag}: mass {total} want {}", 2.5 * vol * thick);
                assert!((fy + 2.0 * vol * thick).abs() < 1e-12 * vol, "{tag}: body {fy}");
            }
            // Mass matrix exactness: u^T M u for a linear field equals rho int |u|^2.
            // Free thermal expansion: K (alpha dT x) = f_th. Axisymmetric too (u_r = alpha dT r is exact).
            let dt = 3.0;
            let mut fth = vec![0.0; 27 * 3];
            kernel::thermal_load(kind, physics, &mat, dt, &xyz, &mut work, &mut fth).unwrap();
            let mut hth = [[0.0; 3]; 3];
            for i in 0..3 {
                hth[i][i] = mat.alpha * dt;
            }
            if matches!(physics, Physics::PlaneStrain { .. }) {
                // eps_zz = 0 is imposed: the free in-plane expansion is (1 + nu) alpha dT.
                for i in 0..2 {
                    hth[i][i] *= 1.0 + mat.nu;
                }
            }
            let uth = affine(&xyz, d, hth);
            let kth = mul(&k, &uth);
            let fmax = fth[..nn * d].iter().fold(0.0f64, |a, v| a.max(v.abs()));
            for r in 0..nn * d {
                assert!((kth[r] - fth[r]).abs() < 1e-9 * fmax, "{tag}: thermal row {r}: K u = {} f = {}", kth[r], fth[r]);
            }
            // Conduction: constant field in null space, linear field energy = k |g|^2 V.
            let tn: Vec<f64> = xyz.iter().map(|x| 0.3 * x[0] - 0.7 * x[1] + if d == 3 { 0.2 * x[2] } else { 0.0 }).collect();
            let mut kc = vec![0.0; 27 * 27];
            kernel::conduction(kind, physics, &|t| 4.0 + 0.0 * t, &tn, &xyz, &mut work, &mut kc).unwrap();
            let kcm: Vec<Vec<f64>> = (0..nn).map(|a| kc[a * nn..(a + 1) * nn].to_vec()).collect();
            let c0 = mul(&kcm, &vec![1.0; nn]);
            assert!(c0.iter().all(|v| v.abs() < 1e-10 * norm(&kcm)), "{tag}: conduction constant mode");
            if !axisym {
                let e: f64 = tn.iter().zip(mul(&kcm, &tn)).map(|(a, b)| a * b).sum();
                let g2 = 0.09 + 0.49 + if d == 3 { 0.04 } else { 0.0 };
                let want = 4.0 * g2 * vol * thick;
                assert!((e - want).abs() < 1e-10 * want, "{tag}: conduction energy {e} want {want}");
            }
            // Isotropic material through the anisotropic path must give the same stiffness.
            let (lam, mu) = (mat.lambda(Physics::Solid), mat.mu());
            let mut dm = [[0.0; 6]; 6];
            for i in 0..3 {
                for j in 0..3 {
                    dm[i][j] = lam + if i == j { 2.0 * mu } else { 0.0 };
                }
                dm[i + 3][i + 3] = mu;
            }
            let an = Elastic::anisotropic(dm).unwrap().with_alpha(mat.alpha);
            let ka = full(kind, physics, &an, &xyz, false);
            for r in 0..nn * d {
                for c in 0..nn * d {
                    assert!((k[r][c] - ka[r][c]).abs() < 1e-9 * scale, "{tag}: aniso path differs at ({r},{c}): {} vs {}", k[r][c], ka[r][c]);
                }
            }
            let mut fa = vec![0.0; 27 * 3];
            kernel::thermal_load(kind, physics, &an, dt, &xyz, &mut work, &mut fa).unwrap();
            for r in 0..nn * d {
                assert!((fa[r] - fth[r]).abs() < 1e-9 * fmax, "{tag}: aniso thermal row {r}: {} vs {}", fa[r], fth[r]);
            }
        }
    }
}

use fea_core::assembly::Pattern;
use fea_core::material::J2;
use fea_core::mesh::{Block, Mesh};
use fea_core::nonlinear::{assemble_nl_public, NlState};
use mechanics_core::hardening::Hardening;

fn single(kind: ElementKind, physics: Physics, mat: Elastic, j2: J2) -> Mesh {
    let shift = if matches!(physics, Physics::Axisymmetric) { [3.0, 0.5, 0.0] } else { [0.4, -0.2, 0.1] };
    let (xyz, _) = element(kind, shift);
    let mut mesh = Mesh::new(physics);
    mesh.nodes = xyz;
    mesh.blocks.push(Block { kind, conn: (0..kind.n_nodes()).collect(), material: mat, name: "b".into(), plasticity: Some(j2), density: 0.0, thermal: None });
    mesh
}

#[test]
fn every_nonlinear_element_tangent_matches_finite_differences() {
    let law = Hardening::linear(0.5, 20.0, 1.0).unwrap();
    let iso = Elastic::new(200.0, 0.3);
    let ortho = Elastic::orthotropic(200.0, 120.0, 90.0, 0.3, 0.25, 0.2, 60.0, 40.0, 50.0).unwrap();
    let h1 = [[0.011, -0.004, 0.007], [0.003, -0.006, 0.002], [0.005, 0.009, 0.004]];
    let mut worst: Vec<(String, f64)> = Vec::new();
    for kind in ALL_KINDS {
        let physics_list: Vec<Physics> = if kind.dim() == 3 { vec![Physics::Solid] } else { vec![Physics::PlaneStress { thickness: 0.7 }, Physics::PlaneStrain { thickness: 0.7 }, Physics::Axisymmetric] };
        for physics in physics_list {
            let ps = matches!(physics, Physics::PlaneStress { .. });
            let mut cases: Vec<(&str, Elastic, J2)> = vec![("small", iso, J2::small(law)), ("finite", iso, J2::finite(law)), ("small-aniso", ortho, J2::small(law))];
            if !ps {
                cases.push(("neo", iso, J2::neo_hookean()));
            }
            for (name, mat, j2) in cases {
                let mesh = single(kind, physics, mat, j2);
                let pat = Pattern::new(&mesh).unwrap();
                let d = physics.dim();
                let n = mesh.nodes.len() * d;
                let wiggle = |s: f64| -> Vec<f64> {
                    let mut u = affine(&mesh.nodes, d, h1).iter().map(|v| v * s).collect::<Vec<_>>();
                    for (i, v) in u.iter_mut().enumerate() {
                        *v += 1e-3 * s * ((i as f64 * 1.7).sin());
                    }
                    u
                };
                let s0 = NlState::new(&mesh);
                let mut s1 = s0.clone();
                let u1 = wiggle(1.0);
                assemble_nl_public(&mesh, &pat, &u1, &s0, &mut s1, false).unwrap();
                let tag = format!("{kind:?} {physics:?} {name}");
                assert!(name == "neo" || s1.max_ep() > 0.0, "{tag}: did not yield");
                // Evaluate at a partially unloaded/reloaded point from the yielded state.
                let u2 = wiggle(1.4);
                let mut tmp = s1.clone();
                let (_, k) = assemble_nl_public(&mesh, &pat, &u2, &s1, &mut tmp, true).unwrap();
                let k = k.unwrap();
                let mut kd = vec![vec![0.0; n]; n];
                for c in 0..n {
                    let mut e = vec![0.0; n];
                    e[c] = 1.0;
                    let mut y = vec![0.0; n];
                    k.matvec_add(&pat, &e, &mut y);
                    for r in 0..n {
                        kd[r][c] = y[r];
                    }
                }
                let scale = norm(&kd);
                let step = 1e-7;
                let mut err = 0.0f64;
                for c in 0..n {
                    let (mut up, mut um) = (u2.clone(), u2.clone());
                    up[c] += step;
                    um[c] -= step;
                    let (fp, _) = assemble_nl_public(&mesh, &pat, &up, &s1, &mut tmp, false).unwrap();
                    let (fm, _) = assemble_nl_public(&mesh, &pat, &um, &s1, &mut tmp, false).unwrap();
                    for r in 0..n {
                        err = err.max(((fp[r] - fm[r]) / (2.0 * step) - kd[r][c]).abs() / scale);
                    }
                }
                worst.push((tag, err));
            }
        }
    }
    let bad: Vec<_> = worst.iter().filter(|(_, e)| *e > 1e-5).collect();
    assert!(bad.is_empty(), "tangent mismatches: {bad:?}");
}
