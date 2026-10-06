//! Stress recovery (SPR) and the ZZ error estimate.

#![allow(clippy::needless_range_loop)] // index-parallel numeric kernels read clearer as loops

use fea_core::element::ALL_KINDS;
use fea_core::generate::grid;
use fea_core::*;
use std::f64::consts::PI;

const E: f64 = 10.0e6;
const NU: f64 = 0.3;

fn physics_for(kind: ElementKind) -> Physics {
    if kind.dim() == 3 {
        Physics::Solid
    } else {
        Physics::PlaneStrain { thickness: 1.0 }
    }
}

/// Exact stress of the plane-strain/solid isotropic law for a displacement gradient.
fn stress_of(kind: ElementKind, grad: impl Fn(usize, usize) -> f64) -> [f64; 6] {
    let d = kind.dim();
    let mat = Elastic::new(E, NU);
    let mut h = [[0.0; 3]; 3];
    for i in 0..d {
        for k in 0..d {
            h[i][k] = grad(i, k);
        }
    }
    let strain = [h[0][0], h[1][1], h[2][2], h[0][1] + h[1][0], h[1][2] + h[2][1], h[2][0] + h[0][2]];
    fea_core::kernel::stress_from_strain(physics_for(kind), &mat, strain, 0.0)
}

#[test]
fn spr_is_exact_for_a_linear_stress_field_on_every_element_type() {
    // A quadratic displacement field: quadratic and cubic-free elements reproduce it exactly, so the
    // finite-element stress is the exact linear field and SPR (degree >= 1) must recover it at every
    // node, including boundary and domain corners.
    let q = [[1e-3, 2e-4, -1e-4], [3e-4, -2e-3, 5e-4], [-2e-4, 1e-4, 4e-4]];
    let c = [[4e-4, -1e-4, 2e-4], [1e-4, 3e-4, -2e-4], [-3e-4, 2e-4, 1e-4]];
    for kind in ALL_KINDS {
        let d = kind.dim();
        let quadratic = kind.n_nodes() > kind.n_corners();
        let div = if d == 2 { [3, 3, 1] } else { [2, 2, 2] };
        let mesh = grid(physics_for(kind), kind, Elastic::new(E, NU), div, &|p| [p[0] + 0.1 * p[1], p[1] - 0.05 * p[0], p[2] + 0.1 * p[0]]).unwrap();
        let model = Model::new(mesh).unwrap();
        // Quadratic elements carry a quadratic field; linear elements only the linear part.
        let uf = |x: &[f64; 3], i: usize| -> f64 { (0..d).map(|k| c[i][k] * x[k]).sum::<f64>() + if quadratic { (0..d).map(|k| q[i][k] * x[k] * x[(k + 1) % d]).sum::<f64>() } else { 0.0 } };
        let gradf = |x: &[f64; 3], i: usize, k: usize| -> f64 {
            let mut g = c[i][k];
            if quadratic {
                for a in 0..d {
                    let b = (a + 1) % d;
                    if a == k {
                        g += q[i][a] * x[b];
                    }
                    if b == k {
                        g += q[i][a] * x[a];
                    }
                }
            }
            g
        };
        let mut bc = model.dirichlet();
        for n in 0..model.mesh.nodes.len() {
            let on_boundary = model.mesh.node_sets.values().any(|s| s.contains(&n));
            if on_boundary {
                for i in 0..d {
                    bc.fix(n, i, uf(&model.mesh.nodes[n], i));
                }
            }
        }
        // The quadratic field is in equilibrium only with the body force -div(sigma) (constant, since sigma is linear).
        let sig_at = |x: [f64; 3]| stress_of(kind, |i, k| gradf(&x, i, k));
        let s0 = sig_at([0.0; 3]);
        let mut body = [0.0; 3];
        let sidx = |i: usize, k: usize| -> usize { if i == k { i } else { [[0, 3, 5], [3, 1, 4], [5, 4, 2]][i][k] } };
        for k in 0..d {
            let mut xk = [0.0; 3];
            xk[k] = 1.0;
            let sk = sig_at(xk);
            for i in 0..d {
                body[i] -= sk[sidx(i, k)] - s0[sidx(i, k)];
            }
        }
        let loads = Loads { body: Some(body), ..Loads::default() };
        let sol = model.solve_static(&loads, &bc).unwrap();
        let gauss = model.gauss_stresses(&sol.u, 0.0).unwrap();
        let rec = model.recover_spr(&gauss).unwrap();
        let mut gerr = 0.0f64;
        for blk in &gauss {
            for (x, st) in blk {
                let ex = stress_of(kind, |i, k| gradf(x, i, k));
                for s in 0..if d == 2 { 4 } else { 6 } {
                    gerr = gerr.max((st[s] - ex[s]).abs());
                }
            }
        }
        eprintln!("{kind:?}: FE Gauss stress vs exact field: {gerr:e}");
        let scale = stress_of(kind, |i, k| gradf(&[0.5; 3], i, k)).iter().fold(0.0f64, |m, v| m.max(v.abs())).max(1.0);
        for (n, x) in model.mesh.nodes.iter().enumerate() {
            let ex = stress_of(kind, |i, k| gradf(x, i, k));
            // The kernel stores the in-plane shear in slot 3 only for 2D; compare the used slots.
            let nc = if d == 2 { 4 } else { 6 };
            for s in 0..nc {
                let err = (rec[n][s] - ex[s]).abs();
                assert!(err < 1e-8 * scale, "{kind:?} node {n} comp {s}: {err:e} (stress scale {scale:e})");
            }
        }
    }
}

// Kirsch plate (shared with classic.rs in spirit; kept local so each file stands alone).
fn kirsch(sig: f64, a: f64, x: f64, y: f64) -> [f64; 6] {
    let (r, th) = (x.hypot(y), y.atan2(x));
    let (q, q2) = ((a / r).powi(2), (a / r).powi(4));
    let (c2, s2) = ((2.0 * th).cos(), (2.0 * th).sin());
    let sr = 0.5 * sig * (1.0 - q) + 0.5 * sig * (1.0 - 4.0 * q + 3.0 * q2) * c2;
    let st = 0.5 * sig * (1.0 + q) - 0.5 * sig * (1.0 + 3.0 * q2) * c2;
    let tr = -0.5 * sig * (1.0 + 2.0 * q - 3.0 * q2) * s2;
    let (c, s) = (th.cos(), th.sin());
    [sr * c * c + st * s * s - 2.0 * tr * s * c, sr * s * s + st * c * c + 2.0 * tr * s * c, 0.0, (sr - st) * s * c + tr * (c * c - s * s), 0.0, 0.0]
}

struct Run {
    avg_hole: f64,
    spr_hole: f64,
    avg_rms: f64,
    spr_rms: f64,
    eta: f64,
    exact: f64,
    dofs: usize,
}

fn kirsch_run(kind: ElementKind, m: usize) -> Run {
    let (sig, a, half) = (1000.0, 1.0, 8.0);
    let map = move |p: [f64; 3]| {
        let th = 0.5 * PI * p[1];
        let rho = half / th.cos().abs().max(th.sin().abs());
        let s = p[0].powf(1.2);
        let r = (1.0 - s) * a + s * rho;
        [r * th.cos(), r * th.sin(), 0.0]
    };
    let mesh = grid(Physics::PlaneStress { thickness: 1.0 }, kind, Elastic::new(E, NU), [6 * m, 8 * m, 1], &map).unwrap();
    let model = Model::new(mesh).unwrap();
    let mut bc = model.dirichlet();
    for &n in model.mesh.node_set("v0").unwrap() {
        bc.fix(n, 1, 0.0);
    }
    for &n in model.mesh.node_set("v1").unwrap() {
        bc.fix(n, 0, 0.0);
    }
    let field = FaceField::new(move |x, n| {
        let s = kirsch(sig, a, x[0], x[1]);
        [s[0] * n[0] + s[3] * n[1], s[3] * n[0] + s[1] * n[1], 0.0]
    });
    let loads = Loads { field_faces: model.mesh.surfaces["u1"].iter().map(|f| (f.clone(), field.clone())).collect(), ..Loads::default() };
    let sol = model.solve_static(&loads, &bc).unwrap();
    let avg = model.nodal_stresses(&sol.u, 0.0).unwrap();
    let gauss = model.gauss_stresses(&sol.u, 0.0).unwrap();
    let spr = model.recover_spr(&gauss).unwrap();
    let hole = |st: &[[f64; 6]]| -> f64 {
        let mut e = 0.0f64;
        for &n in model.mesh.node_set("u0").unwrap() {
            let x = model.mesh.nodes[n];
            let (c, s) = (x[0] / a, x[1] / a);
            let hoop = st[n][0] * s * s + st[n][1] * c * c - 2.0 * st[n][3] * s * c;
            e = e.max((hoop - sig * (1.0 - 2.0 * (2.0 * x[1].atan2(x[0])).cos())).abs() / (3.0 * sig));
        }
        e
    };
    let rms = |st: &[[f64; 6]]| -> f64 {
        let mut sum = 0.0;
        for (n, x) in model.mesh.nodes.iter().enumerate() {
            let ex = kirsch(sig, a, x[0], x[1]);
            sum += [0, 1, 3].iter().map(|&c| (st[n][c] - ex[c]).powi(2)).sum::<f64>();
        }
        (sum / model.mesh.nodes.len() as f64).sqrt() / sig
    };
    let zz = model.zz_error(&sol.u, 0.0).unwrap();
    let exact = model.stress_error_energy(&sol.u, 0.0, |x| kirsch(sig, a, x[0], x[1])).unwrap();
    Run { avg_hole: hole(&avg), spr_hole: hole(&spr), avg_rms: rms(&avg), spr_rms: rms(&spr), eta: zz.total, exact, dofs: model.mesh.n_dofs() }
}

#[test]
fn spr_beats_nodal_averaging_and_zz_converges_on_the_kirsch_plate() {
    // The hole region is pre-asymptotic (high stress gradients) so only the trend is asserted here;
    // the effectivity index is checked on the smooth sphere below.
    for kind in [ElementKind::Quad9, ElementKind::Quad8, ElementKind::Tri6, ElementKind::Quad4] {
        let runs: Vec<Run> = [2, 4, 8].iter().map(|&m| kirsch_run(kind, m)).collect();
        for (m, r) in [2, 4, 8].iter().zip(&runs) {
            eprintln!("{kind:?} m={m} ({} dofs): hole hoop error averaging {:.2e} -> SPR {:.2e}; nodal RMS error averaging {:.2e} -> SPR {:.2e}; ZZ eta {:.4e} vs exact error {:.4e} (effectivity {:.3})", r.dofs, r.avg_hole, r.spr_hole, r.avg_rms, r.spr_rms, r.eta, r.exact, r.eta / r.exact);
        }
        let last = runs.last().unwrap();
        assert!(last.spr_rms < 0.8 * last.avg_rms, "{kind:?}: SPR should beat averaging globally ({:e} vs {:e})", last.spr_rms, last.avg_rms);
        assert!(runs[2].eta < runs[1].eta && runs[1].eta < runs[0].eta && runs[2].eta < runs[0].eta / 1.8, "{kind:?}: eta not converging");
        assert!(runs[2].exact < runs[0].exact / 1.8, "{kind:?}: true error not converging");
        assert!((0.9..1.3).contains(&(last.eta / last.exact)), "{kind:?}: effectivity {} at the finest mesh", last.eta / last.exact);
    }
}

/// Hollow sphere under internal pressure in axisymmetric form: exact stress `[rr, zz, tt, rz]`.
fn sphere_stress(p: f64, a: f64, b: f64, x: f64, z: f64) -> [f64; 6] {
    let r = x.hypot(z);
    let k = p * a.powi(3) / (b.powi(3) - a.powi(3));
    let (sr, sh) = (k * (1.0 - b.powi(3) / r.powi(3)), k * (1.0 + b.powi(3) / (2.0 * r.powi(3))));
    let (c, s) = (x / r, z / r);
    [sr * c * c + sh * s * s, sr * s * s + sh * c * c, sh, (sr - sh) * s * c, 0.0, 0.0]
}

#[test]
fn zz_effectivity_is_near_one_and_spr_is_superconvergent_on_the_smooth_sphere() {
    let (a, b, p) = (1.0, 2.0, 1000.0);
    for kind in [ElementKind::Quad9, ElementKind::Quad8, ElementKind::Tri6] {
        let mut prev_eta = f64::INFINITY;
        let mut last = (0.0, 0.0, 0.0);
        let mut prev_spr = f64::NAN;
        let mut spr_rate = 0.0;
        for m in [4usize, 8, 16] {
            let mesh = grid(Physics::Axisymmetric, kind, Elastic::new(E, NU), [m, 2 * m, 1], &move |q| {
                let (rho, ph) = (a + (b - a) * q[0], 0.5 * PI * q[1]);
                [rho * ph.cos(), rho * ph.sin(), 0.0]
            })
            .unwrap();
            let model = Model::new(mesh).unwrap();
            let mut bc = model.dirichlet();
            for &n in model.mesh.node_set("v0").unwrap() {
                bc.fix(n, 1, 0.0);
            }
            for &n in model.mesh.node_set("v1").unwrap() {
                bc.fix(n, 0, 0.0);
            }
            let loads = Loads { faces: model.mesh.surfaces["u0"].iter().map(|f| (f.clone(), SurfaceLoad::Pressure(p))).collect(), ..Loads::default() };
            let sol = model.solve_static(&loads, &bc).unwrap();
            let avg = model.nodal_stresses(&sol.u, 0.0).unwrap();
            let spr = model.recover_spr(&model.gauss_stresses(&sol.u, 0.0).unwrap()).unwrap();
            let rms = |st: &[[f64; 6]]| -> f64 {
                let mut sum = 0.0;
                for (n, x) in model.mesh.nodes.iter().enumerate() {
                    let ex = sphere_stress(p, a, b, x[0], x[1]);
                    sum += [0, 1, 2, 3].iter().map(|&c| (st[n][c] - ex[c]).powi(2)).sum::<f64>();
                }
                (sum / model.mesh.nodes.len() as f64).sqrt() / p
            };
            let zz = model.zz_error(&sol.u, 0.0).unwrap();
            let exact = model.stress_error_energy(&sol.u, 0.0, |x| sphere_stress(p, a, b, x[0], x[1])).unwrap();
            eprintln!("sphere {kind:?} m={m}: nodal RMS averaging {:.2e} -> SPR {:.2e}; eta {:.4e}, exact {:.4e}, effectivity {:.3}", rms(&avg), rms(&spr), zz.total, exact, zz.total / exact);
            assert!(zz.total < prev_eta, "{kind:?}: eta must decrease");
            prev_eta = zz.total;
            spr_rate = prev_spr / rms(&spr);
            prev_spr = rms(&spr);
            last = (rms(&avg), rms(&spr), zz.total / exact);
        }
        assert!(last.1 < 0.3 * last.0, "{kind:?}: SPR should cut the nodal RMS error by >3x ({:e} vs {:e})", last.1, last.0);
        // Averaging converges as h^2 (x4 per halving); SPR superconverges (x8 would be h^3).
        assert!(spr_rate > 4.5, "{kind:?}: SPR error ratio per halving {spr_rate}");
        assert!((0.9..1.1).contains(&last.2), "{kind:?}: effectivity {} at the finest mesh", last.2);
    }
}
