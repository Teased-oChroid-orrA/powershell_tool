//! Story F: matrix-free derivative operators. The matrix-free finite-element tangent against the assembled one, product consistency
//! (`w^T (A v) = (A^T w)^T v`), Krylov solves with different preconditioners, a Newton-Krylov solve driven by AD Jacobian-vector
//! products, and the transposed solve through an unsymmetric tangent.

use fea_core::delaunay::MeshOptions;
use fea_core::derivative::constitutive::lu_solve;
use fea_core::derivative::linalg::*;
use fea_core::derivative::matrix_free::*;
use fea_core::derivative::ops::{jacobian_forward, jvp, vjp, DerivativeWorkspace, SmoothKernel};
use fea_core::derivative::path::{solve_path, PathOptions};
use fea_core::derivative::scalar::Scalar;
use fea_core::geometry::{Loop, Region};
use fea_core::linear::{Ordering, Reduced};
use fea_core::material::J2;
use fea_core::mesh2d::mesh_region;
use fea_core::nonlinear::assemble_nl_public;
use fea_core::*;
use mechanics_core::hardening::Hardening;

fn plastic_plate(physics: Physics, kind: ElementKind) -> (Model, Loads, Dirichlet) {
    let region = Region::new(Loop::rectangle(0.0, 0.0, 6.0, 2.0).unwrap(), vec![Loop::circle([3.0, 1.0], 0.45, "hole").unwrap()], Elastic::new(10.0e6, 0.3)).unwrap();
    let size = |p: [f64; 2]| (0.2 + 0.3 * ((p[0] - 3.0).hypot(p[1] - 1.0) - 0.45).max(0.0)).min(0.55);
    let mut mesh = mesh_region(&region, physics, kind, &size, MeshOptions::default()).unwrap();
    mesh.set_plasticity(0, J2::small(Hardening::linear(40_000.0, 2.0e5, 5.0).unwrap())).unwrap();
    let model = Model::new(mesh).unwrap();
    let mut bc = model.dirichlet();
    for &n in model.mesh.node_set("left").unwrap() {
        bc.fix(n, 0, 0.0);
    }
    let anchor = *model.mesh.node_set("left").unwrap().iter().min_by(|&&a, &&b| (model.mesh.nodes[a][1] - 1.0).abs().total_cmp(&(model.mesh.nodes[b][1] - 1.0).abs())).unwrap();
    bc.fix(anchor, 1, 0.0);
    let loads = Loads { faces: model.mesh.surfaces["right"].iter().map(|f| (f.clone(), SurfaceLoad::Traction([21_000.0, 0.0, 0.0]))).collect(), ..Loads::default() };
    (model, loads, bc)
}

fn pseudo_random(n: usize, seed: u64) -> Vec<f64> {
    let mut s = seed;
    (0..n)
        .map(|_| {
            s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((s >> 33) as f64) / ((1u64 << 31) as f64) - 0.5
        })
        .collect()
}

#[test]
fn the_matrix_free_tangent_is_the_assembled_tangent_in_both_modes_and_deterministic_across_thread_counts() {
    for (physics, kind) in [(Physics::PlaneStrain { thickness: 1.0 }, ElementKind::Tri6), (Physics::PlaneStress { thickness: 1.0 }, ElementKind::Quad8)] {
        let (model, loads, bc) = plastic_plate(physics, kind);
        let sol = solve_path(&model, &loads, &bc, &PathOptions { steps: 3, ..PathOptions::default() }).unwrap();
        assert!(sol.states.last().unwrap().plastic_fraction() > 0.01);
        let (u, old) = (sol.u.last().unwrap(), &sol.states[sol.steps - 1]);
        let mut new = old.clone();
        let (_, k) = assemble_nl_public(&model.mesh, &model.pattern, u, old, &mut new, true).unwrap();
        let k = k.unwrap();
        let mut v = pseudo_random(model.mesh.n_dofs(), 7);
        for (x, f) in v.iter_mut().zip(&bc.fixed) {
            if *f {
                *x = 0.0;
            }
        }
        let mut want = vec![0.0; v.len()];
        k.matvec_add(&model.pattern, &v, &mut want);
        let cached = MatrixFreeTangent::new(&model, &bc, u, old, MatrixFreeMode::Cached).unwrap();
        let recompute = MatrixFreeTangent::new(&model, &bc, u, old, MatrixFreeMode::Recompute).unwrap();
        let (a, b) = (cached.apply_full(&v), recompute.apply_full(&v));
        let scale = want.iter().fold(0.0f64, |m, x| m.max(x.abs()));
        for d in 0..v.len() {
            if bc.fixed[d] {
                continue;
            }
            assert!((a[d] - want[d]).abs() < 1e-9 * scale, "{kind:?} cached dof {d}: {} vs assembled {}", a[d], want[d]);
            assert!((b[d] - want[d]).abs() < 1e-9 * scale, "{kind:?} recompute dof {d}: {} vs assembled {}", b[d], want[d]);
        }
        assert!(cached.stored_bytes() > recompute.stored_bytes(), "recompute mode stores less");
        assert!(cached.is_symmetric());
        // the product is bit-identical for any number of threads
        let run = |threads: usize| rayon::ThreadPoolBuilder::new().num_threads(threads).build().unwrap().install(|| cached.apply_full(&v));
        assert_eq!(run(1), run(4), "the matrix-free product must not depend on the thread count");
        // adjoint consistency of the operator
        let nf = cached.dim();
        let (x, w) = (pseudo_random(nf, 3), pseudo_random(nf, 5));
        let (mut ax, mut atw) = (vec![0.0; nf], vec![0.0; nf]);
        cached.apply(&x, &mut ax);
        cached.apply_transpose(&w, &mut atw);
        let l: f64 = w.iter().zip(&ax).map(|(p, q)| p * q).sum();
        let r: f64 = atw.iter().zip(&x).map(|(p, q)| p * q).sum();
        assert!((l - r).abs() < 1e-10 * l.abs().max(r.abs()), "{l} vs {r}");
    }
}

#[test]
fn krylov_solves_of_the_tangent_system_agree_with_the_direct_solve_and_a_reused_factorization_preconditions_best() {
    let (model, loads, bc) = plastic_plate(Physics::PlaneStrain { thickness: 1.0 }, ElementKind::Tri6);
    let sol = solve_path(&model, &loads, &bc, &PathOptions { steps: 3, ..PathOptions::default() }).unwrap();
    let (u, old) = (sol.u.last().unwrap(), &sol.states[sol.steps - 1]);
    let mf = MatrixFreeTangent::new(&model, &bc, u, old, MatrixFreeMode::Cached).unwrap();
    let nf = mf.dim();
    let b = pseudo_random(nf, 11);
    // reference: the assembled tangent and its sparse Cholesky
    let mut new = old.clone();
    let (_, k) = assemble_nl_public(&model.mesh, &model.pattern, u, old, &mut new, true).unwrap();
    let k = k.unwrap();
    let red = Reduced::with_ordering(&model.pattern, &bc, Some(&model.mesh.nodes), Ordering::Auto).unwrap();
    let free: Vec<usize> = red.free_dofs().iter().map(|&d| d as usize).collect();
    assert_eq!(free, mf.free_dofs(), "free dofs in the same order");
    let fac = red.factor_any(&k).unwrap();
    let mut direct = b.clone();
    fac.solve_reduced(&mut direct);
    // Jacobi
    let diag = mf.diagonal().unwrap();
    let jacobi = |r: &[f64], z: &mut [f64]| {
        for i in 0..r.len() {
            z[i] = r[i] / diag[i];
        }
    };
    let mut xj = vec![0.0; nf];
    let rj = cg(&mf, &b, &mut xj, &jacobi, 1e-11, 5000);
    // the elastic stiffness factored once and reused as the preconditioner of every plastic tangent
    let elastic_k = model.assemble().unwrap();
    let efac = red.factor_any(&elastic_k).unwrap();
    let reuse = |r: &[f64], z: &mut [f64]| {
        z.copy_from_slice(r);
        efac.solve_reduced(z);
    };
    let mut xe = vec![0.0; nf];
    let re = cg(&mf, &b, &mut xe, &reuse, 1e-11, 5000);
    // the exact factor as a preconditioner converges in one step
    let exact = |r: &[f64], z: &mut [f64]| {
        z.copy_from_slice(r);
        fac.solve_reduced(z);
    };
    let mut xx = vec![0.0; nf];
    let rx = cg(&mf, &b, &mut xx, &exact, 1e-11, 50);
    let scale = direct.iter().fold(0.0f64, |m, v| m.max(v.abs()));
    for (name, x, rep) in [("jacobi", &xj, rj), ("elastic factor", &xe, re), ("exact factor", &xx, rx)] {
        assert!(rep.converged, "{name}: {rep:?}");
        let gap = x.iter().zip(&direct).fold(0.0f64, |m, (a, c)| m.max((a - c).abs())) / scale;
        assert!(gap < 1e-7, "{name}: differs from the direct solve by {gap:e}");
    }
    eprintln!("CG iterations: jacobi {}, elastic factor {}, exact factor {}", rj.iterations, re.iterations, rx.iterations);
    assert!(rx.iterations <= 2 && re.iterations < rj.iterations, "{} / {} / {}", rx.iterations, re.iterations, rj.iterations);
}

/// Bratu-type nonlinear system on a line: F_i = 2 x_i - x_{i-1} - x_{i+1} - h^2 (lambda exp(x_i) + f_i), x_0 = x_{n+1} = 0.
struct Bratu {
    n: usize,
}

impl SmoothKernel for Bratu {
    fn n_in(&self) -> usize {
        self.n
    }
    fn n_out(&self) -> usize {
        self.n
    }
    fn eval<S: Scalar>(&self, x: &[S], out: &mut [S]) {
        let h = 1.0 / (self.n as f64 + 1.0);
        for i in 0..self.n {
            let left = if i > 0 { x[i - 1] } else { S::cst(0.0) };
            let right = if i + 1 < self.n { x[i + 1] } else { S::cst(0.0) };
            out[i] = x[i].scale(2.0) - left - right - (x[i].exp().scale(1.5) + S::cst(1.0 + (i as f64 * 0.37).sin())).scale(h * h);
        }
    }
}

#[test]
fn a_newton_krylov_solve_driven_by_ad_jacobian_vector_products_matches_newton_with_the_assembled_jacobian() {
    let k = Bratu { n: 60 };
    let n = k.n;
    let resid = |x: &[f64]| fea_core::derivative::ops::evaluate(&k, x);
    let norm = |v: &[f64]| v.iter().map(|a| a * a).sum::<f64>().sqrt();
    // Newton with the assembled (AD) Jacobian and a dense solve
    let mut xa = vec![0.0; n];
    let mut newton_iters = 0;
    for _ in 0..30 {
        let r = resid(&xa);
        if norm(&r) < 1e-13 {
            break;
        }
        let j = jacobian_forward(&k, &xa);
        let dx = lu_solve(n, 1, &j.data, &r).unwrap();
        for i in 0..n {
            xa[i] -= dx[i];
        }
        newton_iters += 1;
    }
    // Newton-Krylov: GMRES on J dx = r with J v by forward AD and J^T w by reverse AD, no Jacobian formed
    let mut xk = vec![0.0; n];
    let (mut outer, mut inner) = (0, 0);
    for _ in 0..30 {
        let r = resid(&xk);
        if norm(&r) < 1e-13 {
            break;
        }
        let xc = xk.clone();
        let xc2 = xk.clone();
        let op = ClosureOperator {
            n,
            forward: |v: &[f64], y: &mut [f64]| y.copy_from_slice(&jvp(&k, &xc, v)),
            transpose: |w: &[f64], y: &mut [f64]| y.copy_from_slice(&vjp(&k, &xc2, w, &mut DerivativeWorkspace::new(10_000)).unwrap().0),
            symmetric: false,
        };
        let mut dx = vec![0.0; n];
        let rep = gmres(&op, false, &r, &mut dx, &|v, z| z.copy_from_slice(v), 1e-12, 80, 400);
        assert!(rep.converged, "{rep:?}");
        inner += rep.iterations;
        for i in 0..n {
            xk[i] -= dx[i];
        }
        outer += 1;
    }
    let gap = xa.iter().zip(&xk).fold(0.0f64, |m, (a, b)| m.max((a - b).abs()));
    assert!(gap < 1e-10, "{gap:e}");
    assert_eq!(outer, newton_iters, "inexact Newton with a tight inner tolerance takes the same outer steps");
    eprintln!("Newton-Krylov: {outer} Newton steps, {inner} GMRES iterations; no Jacobian assembled");
}

#[test]
fn the_transposed_solve_through_an_unsymmetric_tangent_is_the_adjoint() {
    // a small elastic mesh plus unsymmetric dense blocks on random dof sets: (K + D) x = b and (K + D)^T x = b
    let mesh = fea_core::generate::grid(Physics::PlaneStress { thickness: 1.0 }, ElementKind::Quad4, Elastic::new(1.0e6, 0.3), [4, 3, 1], &|p| [4.0 * p[0], 3.0 * p[1], 0.0]).unwrap();
    let model = Model::new(mesh).unwrap();
    let mut bc = model.dirichlet();
    for &n in model.mesh.node_set("u0").unwrap() {
        bc.fix_node(n);
    }
    let k = model.assemble().unwrap();
    let red = Reduced::with_ordering(&model.pattern, &bc, Some(&model.mesh.nodes), Ordering::Auto).unwrap();
    let free: Vec<usize> = red.free_dofs().iter().map(|&d| d as usize).collect();
    let n = model.mesh.n_dofs();
    let mut defect: Vec<(Vec<usize>, Vec<f64>)> = Vec::new();
    for s in 0..6 {
        let dofs: Vec<usize> = free.iter().copied().skip(s * 3).step_by(2).take(4).collect();
        let m = pseudo_random(dofs.len() * dofs.len(), 100 + s as u64).into_iter().map(|v| v * 4.0e4).collect();
        defect.push((dofs, m));
    }
    let solver = TangentSolver::new(&red, &model.pattern, &k, n, &free, &defect).unwrap();
    // dense A from the operator
    let nf = free.len();
    let op = solver.operator();
    let mut a = vec![0.0; nf * nf];
    for j in 0..nf {
        let mut e = vec![0.0; nf];
        e[j] = 1.0;
        let mut col = vec![0.0; nf];
        op.apply(&e, &mut col);
        for i in 0..nf {
            a[i * nf + j] = col[i];
        }
    }
    assert!((0..nf).any(|i| (0..nf).any(|j| (a[i * nf + j] - a[j * nf + i]).abs() > 1e-6 * a[i * nf + i].abs())), "the tangent must be unsymmetric");
    let b = pseudo_random(nf, 21);
    let x = solver.solve(&b).unwrap();
    let xt = solver.solve_transpose(&b).unwrap();
    let dense = lu_solve(nf, 1, &a, &b).unwrap();
    let at: Vec<f64> = (0..nf).flat_map(|i| (0..nf).map(move |j| (i, j))).map(|(i, j)| a[j * nf + i]).collect();
    let dense_t = lu_solve(nf, 1, &at, &b).unwrap();
    let scale = dense.iter().fold(0.0f64, |m, v| m.max(v.abs()));
    assert!(x.iter().zip(&dense).all(|(p, q)| (p - q).abs() < 1e-9 * scale), "direct");
    assert!(xt.iter().zip(&dense_t).all(|(p, q)| (p - q).abs() < 1e-9 * scale), "transposed");
    assert!(x.iter().zip(&xt).any(|(p, q)| (p - q).abs() > 1e-6 * scale), "the adjoint solution is not the direct one for an unsymmetric operator");
    assert!(solver.n_krylov_iterations() > 0);
}
