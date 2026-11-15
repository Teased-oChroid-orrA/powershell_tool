//! Level 7 of the derivative suite: Hessians of linear static responses with exact second derivatives of the element stiffness,
//! forward over reverse against reverse over forward, and both against differences of the exact first-order gradients.

use fea_core::derivative::linear_sens::{LinearParam, LinearSensitivityEngine};
use fea_core::derivative::second_order::SecondOrderEngine;
use fea_core::generate::grid;
use fea_core::*;

#[derive(Clone, Copy)]
struct Shift {
    modulus: f64,
    nu: f64,
    thickness: f64,
    node: usize,
    dx: f64,
    dy: f64,
}

struct Case {
    model: Model,
    bc: Dirichlet,
    loads: Loads,
    tip: usize,
}

fn case(kind: ElementKind, s: Shift) -> Case {
    let mut mesh = grid(Physics::PlaneStress { thickness: 0.5 + s.thickness }, kind, Elastic::new(1.0e5, 0.3), [5, 3, 1], &|p| [2.0 * p[0] + 0.1 * p[1], p[1] + 0.05 * p[0] * p[0], 0.0]).unwrap();
    mesh.blocks[0].material.e *= 1.0 + s.modulus;
    mesh.blocks[0].material.nu += s.nu;
    mesh.nodes[s.node][0] += s.dx;
    mesh.nodes[s.node][1] += s.dy;
    let model = Model::new(mesh).unwrap();
    let mut bc = model.dirichlet();
    for &n in model.mesh.node_set("u0").unwrap() {
        bc.fix(n, 0, 0.0);
        bc.fix(n, 1, 0.0);
    }
    let tip = *model.mesh.node_set("u1").unwrap().iter().max_by(|&&a, &&b| model.mesh.nodes[a][1].total_cmp(&model.mesh.nodes[b][1])).unwrap();
    let loads = Loads { nodal: vec![(tip, [30.0, -250.0, 0.0])], ..Loads::default() };
    Case { model, bc, loads, tip }
}

const ZERO: Shift = Shift { modulus: 0.0, nu: 0.0, thickness: 0.0, node: 0, dx: 0.0, dy: 0.0 };

fn interior(c: &Case) -> usize {
    (0..c.model.mesh.nodes.len()).find(|&n| (c.model.mesh.nodes[n][0] - 0.8).abs() < 0.2 && (c.model.mesh.nodes[n][1] - 1.0 / 3.0).abs() < 0.1 && c.model.mesh.nodes[n][0] > 0.5).unwrap()
}

fn shifted(base: Shift, which: usize, h: f64) -> Shift {
    let mut s = base;
    match which {
        0 => s.modulus += h,
        1 => s.nu += h,
        2 => s.thickness += h,
        3 => s.dx += h,
        _ => s.dy += h,
    }
    s
}

fn gradient(kind: ElementKind, s: Shift, node: usize) -> Vec<f64> {
    let mut s = s;
    s.node = node;
    let c = case(kind, s);
    let mut g = vec![0.0; c.model.mesh.n_dofs()];
    g[c.tip * 2 + 1] = 1.0;
    let eng = LinearSensitivityEngine::new(&c.model, &c.loads, &c.bc);
    let (out, _) = eng.sensitivities(&[g], &params(node), None).unwrap();
    // `ModulusScale` is the multiplier of the block's current modulus: at a shifted modulus the derivative with respect to the
    // absolute scale `s` of `E0 (1 + s)` is the engine's divided by `1 + s`
    let mut g = out[0].clone();
    g[0] /= 1.0 + s.modulus;
    g
}

fn params(node: usize) -> Vec<LinearParam> {
    vec![LinearParam::ModulusScale(0), LinearParam::PoissonRatio(0), LinearParam::Thickness, LinearParam::Coord { node, axis: 0 }, LinearParam::Coord { node, axis: 1 }]
}

fn check(kind: ElementKind) {
    let probe = case(kind, ZERO);
    let node = interior(&probe);
    let steps = [1e-2, 1e-3, 1e-3, 1e-4, 1e-4];
    let p = params(node);
    let n = p.len();
    let mut g = vec![0.0; probe.model.mesh.n_dofs()];
    g[probe.tip * 2 + 1] = 1.0;
    let eng = SecondOrderEngine::new(&probe.model, &probe.loads, &probe.bc);
    let (h_for, rep) = eng.hessian_forward_over_reverse(&p, &g).unwrap();
    let (h_rof, rep2) = eng.hessian_reverse_over_forward(&p, &g).unwrap();
    // difference of the exact first-order gradients: column j is d(gradient)/dp_j
    let mut h_fd = vec![0.0; n * n];
    for j in 0..n {
        let d = |h: f64| {
            let (gp, gm) = (gradient(kind, shifted(ZERO, j, h), node), gradient(kind, shifted(ZERO, j, -h), node));
            (0..n).map(|i| (gp[i] - gm[i]) / (2.0 * h)).collect::<Vec<_>>()
        };
        let (a, b) = (d(steps[j]), d(0.5 * steps[j]));
        for i in 0..n {
            h_fd[i * n + j] = (4.0 * b[i] - a[i]) / 3.0;
        }
    }
    let scale = |i: usize, j: usize| h_for[i * n + j].abs().max(h_for[j * n + i].abs());
    let norm = h_for.iter().fold(0.0f64, |m, v| m.max(v.abs()));
    for i in 0..n {
        for j in 0..n {
            let (a, b, f) = (h_for[i * n + j], h_rof[i * n + j], h_fd[i * n + j]);
            let tol = 1e-9 * scale(i, j).max(1e-6 * norm);
            assert!((a - b).abs() < tol, "{kind:?} H[{i}][{j}]: forward-over-reverse {a:e} vs reverse-over-forward {b:e}");
            assert!((a - h_for[j * n + i]).abs() < tol, "{kind:?} H[{i}][{j}] is not symmetric: {a:e} vs {:e}", h_for[j * n + i]);
            assert!((a - f).abs() < 2e-5 * scale(i, j).max(1e-4 * norm), "{kind:?} H[{i}][{j}]: exact {a:e} vs difference of gradients {f:e}");
        }
    }
    // a Hessian-vector product is one column combination, at a cost independent of the number of parameters
    let v = [0.3, -1.0, 0.5, 2.0, -0.7];
    let (hv, repv) = eng.hessian_vector(&p, &v, &g).unwrap();
    for i in 0..n {
        let want: f64 = (0..n).map(|j| h_for[i * n + j] * v[j]).sum();
        assert!((hv[i] - want).abs() < 1e-9 * norm.max(1e-300), "{kind:?} (H v)[{i}]: {:e} vs {want:e}", hv[i]);
    }
    assert_eq!(repv.tangent_solves, 4, "u, lambda, du_v, dlambda_v");
    assert_eq!(rep.tangent_solves, 4 * n);
    assert_eq!(rep2.tangent_solves, 2 + n);
}

#[test]
fn quad4_hessian_agrees_between_the_two_organisations_and_with_differences_of_gradients() {
    check(ElementKind::Quad4);
}

#[test]
fn quad8_and_triangle_hessians_too() {
    check(ElementKind::Quad8);
    check(ElementKind::Tri3);
}

#[test]
fn surface_loads_are_refused_because_their_geometry_dependence_is_numerical() {
    let c = case(ElementKind::Quad4, ZERO);
    let loads = Loads { faces: c.model.mesh.surfaces["u1"].iter().map(|f| (f.clone(), loads::SurfaceLoad::Pressure(10.0))).collect(), ..Loads::default() };
    let eng = SecondOrderEngine::new(&c.model, &loads, &c.bc);
    let g = vec![0.0; c.model.mesh.n_dofs()];
    let e = eng.hessian_vector(&[LinearParam::Thickness], &[1.0], &g).unwrap_err();
    assert!(e.contains("nodal loads only"), "{e}");
}
