//! Level 4 of the derivative suite, unilateral contact: the primal-dual active-set (semismooth Newton) solver, sensitivities on a
//! strictly complementary branch, directional (generalized) derivatives at a biactive point where the solution has a kink, and the
//! regularised law converging to both.

use fea_core::derivative::complementarity::*;
use fea_core::derivative::linear_sens::{LinearParam, LinearSensitivityEngine};
use fea_core::derivative::policy::DerivativeSemantics;
use fea_core::generate::grid;
use fea_core::linear::{Ordering, Reduced};
use fea_core::*;

/// A block `[0, 4] x [0, 1]` held at the top (`u_y = 0`) and at the left (`u_x = 0`), above a tilted rigid wall that rises into it by
/// `delta - slope x` at the bottom nodes: `g_i = -(delta - slope x_i) + u_y(i) >= 0`.
struct Setup {
    model: Model,
    bc: Dirichlet,
    bottom: Vec<usize>,
}

fn setup(e: f64) -> Setup {
    let mesh = grid(Physics::PlaneStrain { thickness: 1.0 }, ElementKind::Quad4, Elastic::new(e, 0.3), [12, 3, 1], &|p| [4.0 * p[0], p[1], 0.0]).unwrap();
    let model = Model::new(mesh).unwrap();
    let mut bc = model.dirichlet();
    for &n in model.mesh.node_set("v1").unwrap() {
        bc.fix(n, 1, 0.0);
    }
    for &n in model.mesh.node_set("u0").unwrap() {
        bc.fix(n, 0, 0.0);
    }
    let mut bottom: Vec<usize> = model.mesh.node_set("v0").unwrap().to_vec();
    bottom.sort_by(|&a, &b| model.mesh.nodes[a][0].total_cmp(&model.mesh.nodes[b][0]));
    Setup { model, bc, bottom }
}

const SLOPE: f64 = 6.0e-4;

fn constraints(s: &Setup, delta: f64) -> Vec<Constraint> {
    s.bottom.iter().map(|&n| Constraint { terms: vec![(n * 2 + 1, 1.0)], g0: -(delta - SLOPE * s.model.mesh.nodes[n][0]) }).collect()
}

fn solve_with<R>(e: f64, delta: f64, f: impl FnOnce(&Setup, &ObstacleAnalysis, ObstacleSolution) -> R) -> R {
    let s = setup(e);
    let k = s.model.assemble().unwrap();
    let red = Reduced::with_ordering(&s.model.pattern, &s.bc, Some(&s.model.mesh.nodes), Ordering::Auto).unwrap();
    let free: Vec<usize> = red.free_dofs().iter().map(|&d| d as usize).collect();
    let f_ext = vec![0.0; s.model.mesh.n_dofs()];
    let an = ObstacleAnalysis::new(&red, &s.model.pattern, &k, s.model.mesh.n_dofs(), &free, &f_ext, constraints(&s, delta)).unwrap();
    let sol = an.solve(&ObstacleOptions::default()).unwrap();
    f(&s, &an, sol)
}

/// J = mean vertical displacement of the bottom nodes.
fn qoi(s: &Setup) -> Vec<f64> {
    let mut g = vec![0.0; s.model.mesh.n_dofs()];
    for &n in &s.bottom {
        g[n * 2 + 1] = 1.0 / s.bottom.len() as f64;
    }
    g
}

fn value(s: &Setup, u: &[f64]) -> f64 {
    qoi(s).iter().zip(u).map(|(a, b)| a * b).sum()
}

#[test]
fn the_active_set_solver_satisfies_the_complementarity_conditions_and_agrees_with_the_dual_problem() {
    let delta = 1.5e-3; // the wall reaches x = delta / slope = 2.5 along the bottom
    solve_with(1.0e7, delta, |s, an, sol| {
        let (active, inactive, bi) = (sol.count(Complementarity::Active), sol.count(Complementarity::Inactive), sol.count(Complementarity::Biactive));
        assert!(active >= 4 && inactive >= 3 && bi == 0, "{:?}", sol.status);
        assert!(sol.kkt_violation < 1e-9, "KKT violation {:e}", sol.kkt_violation);
        assert!(sol.lambda.iter().all(|&l| l >= 0.0) && sol.gap.iter().all(|&g| g >= -1e-15), "multipliers non-negative, no penetration");
        assert!(!sol.used_dual_fallback && sol.iterations <= 6, "the active-set method converges in a few iterations: {} ({:?})", sol.iterations, sol.changes);
        // the same problem by the dual QP only (the fallback path)
        let dual = an.solve(&ObstacleOptions { max_iter: 0, ..ObstacleOptions::default() }).unwrap();
        assert!(dual.used_dual_fallback);
        let scale = sol.u.iter().fold(0.0f64, |m, v| m.max(v.abs()));
        assert!(sol.u.iter().zip(&dual.u).all(|(a, b)| (a - b).abs() < 1e-7 * scale), "primal-dual and dual solutions differ");
        assert_eq!(sol.status, dual.status);
        // the load the wall carries equals the sum of the multipliers
        let total: f64 = sol.lambda.iter().sum();
        assert!(total > 0.0);
        let _ = s;
    });
}

fn richardson(f: &dyn Fn(f64) -> f64, h: f64) -> f64 {
    let d = |h: f64| (f(h) - f(-h)) / (2.0 * h);
    (4.0 * d(0.5 * h) - d(h)) / 3.0
}

fn state_value(e: f64, delta: f64) -> f64 {
    solve_with(e, delta, |s, _, sol| value(s, &sol.u))
}

#[test]
fn on_a_strictly_complementary_branch_the_derivative_is_classical_direct_equals_adjoint_and_matches_differences() {
    let (delta, e) = (1.5e-3, 1.0e7);
    // wall position
    let (direct, adjoint, du_modulus, dlam_modulus, lam_total) = solve_with(e, delta, |s, an, sol| {
        let (m, n) = (s.bottom.len(), s.model.mesh.n_dofs());
        let (zero, dg_wall) = (vec![0.0; n], vec![-1.0; m]);
        let sens = an.sensitivity(&sol, &zero, &dg_wall).unwrap();
        assert!(sens.differentiable && sens.biactive == 0 && sens.semantics == DerivativeSemantics::ActiveBranch);
        let direct = value(s, &sens.du);
        let adj = an.adjoint(&sol, &qoi(s)).unwrap();
        let adjoint = adj.gradient(&zero, &dg_wall);
        // modulus scale: rhs = -(dK/dp) u
        let no_loads = Loads::default();
        let lin = LinearSensitivityEngine::new(&s.model, &no_loads, &s.bc);
        let dku = lin.dk_u(LinearParam::ModulusScale(0), &sol.u).unwrap();
        let rhs: Vec<f64> = dku.iter().map(|v| -v).collect();
        let zero_gap = vec![0.0; m];
        let ms = an.sensitivity(&sol, &rhs, &zero_gap).unwrap();
        (direct, adjoint, value(s, &ms.du), ms.dlambda.iter().sum::<f64>(), sol.lambda.iter().sum::<f64>())
    });
    assert!((direct - adjoint).abs() < 1e-10 * direct.abs(), "direct {direct:e} vs adjoint {adjoint:e}");
    // displacement-driven problem: the displacements do not depend on the modulus, the wall reaction is proportional to it
    assert!(du_modulus.abs() < 1e-12 * direct.abs().max(1e-30) + 1e-18, "du/dE = {du_modulus:e}");
    assert!((dlam_modulus - lam_total).abs() < 1e-9 * lam_total, "d(sum lambda)/d(scale) {dlam_modulus:e} vs sum lambda {lam_total:e}");
    let fd_wall = richardson(&|h| state_value(e, delta + h), 4e-5);
    assert!((direct - fd_wall).abs() < 1e-6 * fd_wall.abs(), "wall: exact {direct:e} vs finite difference {fd_wall:e}");
    let fd_mod = richardson(&|h| solve_with(e * (1.0 + h), delta, |_, _, sol| sol.lambda.iter().sum::<f64>()), 4e-2);
    assert!((dlam_modulus - fd_mod).abs() < 1e-6 * fd_mod.abs(), "modulus: exact {dlam_modulus:e} vs finite difference {fd_mod:e}");
}

/// The wall position at which constraint `k` joins the active set (`lambda_k = 0 = g_k`), by bisection on the status.
fn joining_delta(e: f64, k: usize) -> f64 {
    let is_active = |d: f64| solve_with(e, d, |_, _, sol| sol.status[k] != Complementarity::Inactive);
    let (mut lo, mut hi) = (1.0e-4, 4.0e-3);
    assert!(!is_active(lo) && is_active(hi), "the node must join between the bounds");
    for _ in 0..80 {
        let mid = 0.5 * (lo + hi);
        if is_active(mid) { hi = mid } else { lo = mid }
    }
    hi
}

#[test]
fn at_a_biactive_point_the_solution_has_a_kink_and_the_one_sided_derivatives_are_generalized_derivatives() {
    let e = 1.0e7;
    // the first bottom node that is open at delta = 1.5e-3
    let k = solve_with(e, 1.5e-3, |_, _, sol| sol.status.iter().position(|&s| s == Complementarity::Inactive).unwrap());
    let d_star = joining_delta(e, k);
    solve_with(e, d_star, |s, an, sol| {
        assert_eq!(sol.status[k], Complementarity::Biactive, "{:?} (lambda {:e}, gap {:e})", sol.status[k], sol.lambda[k], sol.gap[k]);
        let (m, n) = (s.bottom.len(), s.model.mesh.n_dofs());
        let sens = an.sensitivity(&sol, &vec![0.0; n], &vec![-1.0; m]).unwrap();
        assert_eq!((sens.semantics, sens.biactive), (DerivativeSemantics::Generalized, 1));
        assert!(!sens.differentiable, "the response to the wall position has a kink here");
        let (right, left) = (value(s, &sens.du), value(s, sens.du_opposite.as_ref().unwrap()));
        // one-sided finite differences of the full solve
        let h = 1e-7;
        let base = value(s, &sol.u);
        let fd_right = (state_value(e, d_star + h) - base) / h;
        let fd_left = (base - state_value(e, d_star - h)) / h;
        assert!((right - fd_right).abs() < 1e-4 * fd_right.abs(), "right derivative {right:e} vs forward difference {fd_right:e}");
        assert!((left - fd_left).abs() < 1e-4 * fd_left.abs(), "left derivative {left:e} vs backward difference {fd_left:e}");
        assert!((right - left).abs() > 1e-3 * right.abs(), "the two one-sided derivatives must differ: {right:e} / {left:e}");
        // the adjoint refuses a non-differentiable point instead of returning one of the two
        assert!(an.adjoint(&sol, &qoi(s)).unwrap_err().contains("biactive"));
    });
}

#[test]
fn the_regularised_law_converges_to_the_unilateral_solution_and_its_derivative_lies_between_the_one_sided_ones_at_a_kink() {
    let e = 1.0e7;
    let s = setup(e);
    let k_mat = s.model.assemble().unwrap();
    let red = Reduced::with_ordering(&s.model.pattern, &s.bc, Some(&s.model.mesh.nodes), Ordering::Auto).unwrap();
    let free: Vec<usize> = red.free_dofs().iter().map(|&d| d as usize).collect();
    let n = s.model.mesh.n_dofs();
    let m = s.bottom.len();
    // exact references: a strictly complementary state and a biactive one
    let first_open = solve_with(e, 1.5e-3, |_, _, sol| sol.status.iter().position(|&x| x == Complementarity::Inactive).unwrap());
    let kink = joining_delta(e, first_open);
    let reference = |delta: f64| solve_with(e, delta, |s, an, sol| {
        let sens = an.sensitivity(&sol, &vec![0.0; n], &vec![-1.0; m]).unwrap();
        (sol.u.clone(), value(s, &sens.du), sens.du_opposite.as_ref().map(|d| value(s, d)))
    });
    let (u_ref, d_ref, _) = reference(1.5e-3);
    let (_, d_right, d_left) = reference(kink);
    let d_left = d_left.unwrap();
    let mut errors = Vec::new();
    for rho in [1e-6, 1e-7, 1e-8] {
        let law = SoftplusLaw { eps: 1.0e11, rho };
        let at = |delta: f64| {
            let reg = RegularisedObstacle::new(&red, &s.model.pattern, &k_mat, n, &free, vec![0.0; n], constraints(&s, delta), law);
            let sol = reg.solve(1e-10, 60).unwrap();
            let du = reg.sensitivity(&sol, &vec![0.0; n], &vec![-1.0; m]).unwrap();
            (sol, value(&s, &du))
        };
        let (sol, d_reg) = at(1.5e-3);
        let u_err = sol.u.iter().zip(&u_ref).fold(0.0f64, |a, (x, y)| a.max((x - y).abs())) / u_ref.iter().fold(0.0f64, |a, v| a.max(v.abs()));
        let d_err = (d_reg - d_ref).abs() / d_ref.abs();
        // the regularised derivative at the kink is a smooth interpolation of the two one-sided ones
        let (_, d_kink) = at(kink);
        let (lo, hi) = (d_left.min(d_right), d_left.max(d_right));
        let tol = 1e-3 * (hi - lo).abs();
        assert!(d_kink >= lo - tol && d_kink <= hi + tol, "rho {rho:e}: regularised derivative at the kink {d_kink:e} outside [{lo:e}, {hi:e}]");
        eprintln!("rho {rho:e}: solution error {u_err:.2e}, derivative error {d_err:.2e}, penetration {:.2e}, derivative at the kink {d_kink:.4e} in [{lo:.4e}, {hi:.4e}]", sol.max_penetration);
        errors.push((u_err, d_err, sol.max_penetration));
    }
    assert!(errors[2].0 < errors[0].0 * 0.2 && errors[2].1 < errors[0].1 * 0.2 + 1e-9, "the regularisation error must fall with rho: {errors:?}");
    assert!(errors[2].0 < 1e-4 && errors[2].1 < 1e-4, "{errors:?}");
}

#[test]
fn the_softplus_law_is_ad_differentiable_and_tends_to_the_penalty_law() {
    use fea_core::derivative::dual::Dual;
    let law = SoftplusLaw { eps: 1.0e6, rho: 1e-4 };
    for g in [-1e-2, -1e-3, -1e-5, 0.0, 1e-5, 1e-3] {
        let (p, dp) = law.with_derivative(g);
        let exact_p = 1.0e6 * (-g).max(0.0);
        assert!((p - exact_p).abs() <= 1.0e6 * 1e-4 * 2.0f64.ln() + 1e-9, "g {g}: p {p} vs {exact_p}");
        let h = 1e-7;
        let fd = (law.pressure(g + h) - law.pressure(g - h)) / (2.0 * h);
        assert!((dp - fd).abs() < 1e-3 * fd.abs().max(1.0), "g {g}: AD {dp} vs difference {fd}");
        // second order by nesting
        let dd = law.pressure(Dual::<Dual<f64, 1>, 1>::with(Dual::with(g, [1.0]), [Dual::with(1.0, [0.0])]));
        assert!(dd.d[0].d[0].is_finite() && dd.d[0].d[0] >= 0.0, "the law is convex: p'' >= 0 ({})", dd.d[0].d[0]);
    }
}
