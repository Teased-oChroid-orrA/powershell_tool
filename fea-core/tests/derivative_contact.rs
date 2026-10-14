//! Level 4 of the derivative suite: contact. The classification of contact points and the semantics of the derivative it permits, the
//! exact tangent of the implemented contact law (directional Taylor tests on every branch: open, closed, sticking, sliding), and the
//! sensitivities of the penalty-regularised contact problem against finite differences of its own solve, including through the
//! unsymmetric tangent of Coulomb friction.

use fea_core::contact::{ContactSpec, CpState, RigidMaster, RigidShape};
use fea_core::derivative::contact::*;
use fea_core::derivative::verify::taylor_test;
use fea_core::derivative::{ContactStatus, DerivativeSemantics, SensitivityMethod};
use fea_core::generate::grid;
use fea_core::*;

const E: f64 = 10.0e6;
const NU: f64 = 0.3;

struct Fixture {
    model: Model,
    bc: Dirichlet,
    loads: Loads,
    specs: Vec<ContactSpec>,
}

/// Block `[0, 2] x [0, 1]` of Quad9 on a rigid line `y = wall`, pressure `q` on top, the plane moving by `shift` x load factor.
fn fixture(mu: f64, q: f64, shift: f64, wall: f64, e: f64) -> Fixture {
    let mesh = grid(Physics::PlaneStrain { thickness: 1.0 }, ElementKind::Quad9, Elastic::new(e, NU), [6, 3, 1], &|p| [2.0 * p[0], p[1], 0.0]).unwrap();
    let model = Model::new(mesh).unwrap();
    let mut bc = model.dirichlet();
    if mu > 0.0 {
        for &n in model.mesh.node_set("v1").unwrap() {
            bc.fix(n, 0, 0.0);
        }
    } else {
        let mid = (0..model.mesh.nodes.len()).find(|&n| (model.mesh.nodes[n][0] - 1.0).abs() < 1e-12 && (model.mesh.nodes[n][1] - 1.0).abs() < 1e-12).unwrap();
        bc.fix(mid, 0, 0.0);
    }
    let loads = Loads { faces: model.mesh.surfaces["v1"].iter().map(|f| (f.clone(), SurfaceLoad::Pressure(q))).collect(), ..Loads::default() };
    let plane = RigidMaster::new(RigidShape::Plane { p: [0.0, wall, 0.0], n: [0.0, 1.0, 0.0] }).with_travel([shift, 0.0, 0.0]);
    let eps = if mu > 0.0 { 3.0e7 } else { 1.0e9 };
    let mut spec = ContactSpec::rigid("base", model.mesh.surfaces["v0"].clone(), plane, eps);
    if mu > 0.0 {
        spec = spec.with_friction(mu, 3.0e7);
    }
    Fixture { model, bc, loads, specs: vec![spec] }
}

fn quantity(model: &Model) -> Vec<f64> {
    // vertical displacement of the top-middle node
    let top = (0..model.mesh.nodes.len()).find(|&n| (model.mesh.nodes[n][0] - 1.0).abs() < 1e-12 && (model.mesh.nodes[n][1] - 1.0).abs() < 1e-12).unwrap();
    let mut g = vec![0.0; model.mesh.n_dofs()];
    g[top * 2 + 1] = 1.0;
    g
}

fn random_free(bc: &Dirichlet, seed: u64) -> Vec<f64> {
    let mut s = seed;
    (0..bc.fixed.len())
        .map(|d| {
            s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            if bc.fixed[d] { 0.0 } else { ((s >> 33) as f64) / ((1u64 << 31) as f64) - 0.5 }
        })
        .collect()
}

/// Directional Taylor test of the contact residual at `u` along a random direction, against the exact tangent `K + D` (or only `K`).
/// The fitted order is `INFINITY` when the remainder stays at the rounding floor (the residual is linear along the direction).
fn taylor_order(fx: &Fixture, pc: &PenaltyContact, u: &[f64], lambda: f64, include_defect: bool) -> f64 {
    let v: Vec<f64> = random_free(&fx.bc, 5).iter().map(|x| x * 1e-3).collect();
    let mask = |a: Vec<f64>| -> Vec<f64> { a.iter().zip(&fx.bc.fixed).map(|(x, f)| if *f { 0.0 } else { *x }).collect() };
    let f = |x: &[f64]| mask(pc.evaluate(x, lambda, 1.0, false).unwrap().0);
    let jv = |x: &[f64], d: &[f64]| mask(pc.tangent_product(x, lambda, d, include_defect).unwrap());
    let steps: Vec<f64> = (1..=6).map(|p| 10f64.powi(-p)).collect();
    taylor_test(&f, &jv, u, &v, &steps, 1e-12).observed_order
}

#[test]
fn contact_points_are_classified_and_the_derivative_semantics_follow_from_the_state() {
    let fx = fixture(0.3, 1e4, 0.0, 0.0, E);
    let spec = &fx.specs[0];
    let pts = |v: Vec<CpState>| vec![v];
    // a point with pressure `p` whose trial friction traction is `t` (slip = t / eps_t along x, normal +y)
    let eps_t = fx.specs[0].eps_t;
    let cp = |p: f64, t: f64, g: f64, active: bool| CpState { p, g, active, n: [0.0, 1.0, 0.0], cur: [t / eps_t, 0.0, 0.0], hist: [0.0; 3], has_hist: true, ..CpState::default() };
    // a sticking, a sliding and an open point, none near a transition
    let r = classify_contact(&fx.specs, &pts(vec![cp(1000.0, 100.0, -1e-6, true), cp(1000.0, 500.0, -1e-6, true), cp(0.0, 0.0, 5e-3, false)]), 1e-3);
    let i = &r.interfaces[0];
    assert_eq!((i.stick, i.slip, i.open, i.near_normal_transition, i.near_friction_transition), (1, 1, 1, 0, 0), "{i:?}");
    assert_eq!((r.semantics, r.contact), (DerivativeSemantics::ActiveBranch, ContactStatus::FixedActiveSet));
    assert!(r.notes.iter().any(|n| n.contains("slip history")), "{:?}", r.notes);
    // a point on the friction cone, a point whose pressure has just vanished, and an open point about to close
    let near_cone = classify_contact(&fx.specs, &pts(vec![cp(1000.0, 299.9, -1e-6, true), cp(1000.0, 10.0, -1e-6, true)]), 1e-3);
    // a point sliding just past the cone is as close to sticking again, which the capped traction would hide
    let just_slipping = classify_contact(&fx.specs, &pts(vec![cp(1000.0, 300.2, -1e-6, true)]), 1e-3);
    assert_eq!((just_slipping.interfaces[0].slip, just_slipping.interfaces[0].near_friction_transition), (1, 1));
    assert_eq!(near_cone.interfaces[0].near_friction_transition, 1);
    assert_eq!((near_cone.semantics, near_cone.contact), (DerivativeSemantics::Generalized, ContactStatus::Transition));
    assert!(near_cone.notes[0].contains("not differentiable in the classical sense"));
    let near_zero = classify_contact(&fx.specs, &pts(vec![cp(1000.0, 10.0, -1e-6, true), cp(0.5, 0.0, -1e-9, true)]), 1e-3);
    assert_eq!(near_zero.interfaces[0].near_normal_transition, 1, "{:?}", near_zero.interfaces[0]);
    let about_to_close = classify_contact(&fx.specs, &pts(vec![cp(1000.0, 10.0, -1e-6, true), cp(0.0, 0.0, 1e-12, false)]), 1e-3);
    assert_eq!(about_to_close.contact, ContactStatus::Transition);
    // all open: the map is classical (the contact law is inactive)
    let open = classify_contact(&fx.specs, &pts(vec![cp(0.0, 0.0, 5e-3, false)]), 1e-3);
    assert_eq!((open.semantics, open.contact), (DerivativeSemantics::Classical, ContactStatus::None));
    assert_eq!(spec.name, "base");
}

#[test]
fn the_exact_tangent_is_verified_by_taylor_tests_on_every_branch_of_the_contact_law() {
    // open: the plane is below the block, which is held in y and loaded along x
    let mut open = fixture(0.0, 0.0, 0.0, -0.05, E);
    for &n in open.model.mesh.node_set("v1").unwrap() {
        open.bc.fix(n, 1, 0.0);
    }
    open.loads = Loads { faces: open.model.mesh.surfaces["u1"].iter().map(|f| (f.clone(), SurfaceLoad::Traction([1e3, 0.0, 0.0]))).collect(), ..Loads::default() };
    let pc = PenaltyContact::new(&open.model, open.specs.clone(), &open.loads, &open.bc).unwrap();
    let sol = pc.solve(1.0, 1e-10, 30).unwrap();
    assert!(sol.stats.n_active == 0, "no point may touch: {}", sol.stats.n_active);
    // with every point open the residual is the linear elastic one: the remainder of the Taylor sweep is rounding, at any step
    let order = taylor_order(&open, &pc, &sol.u, 1.0, true);
    assert!(order > 1.8 || order == f64::INFINITY, "open contact: order {order}");
    // closed, frictionless
    let closed = fixture(0.0, 1e4, 0.0, 0.0, E);
    let pc = PenaltyContact::new(&closed.model, closed.specs.clone(), &closed.loads, &closed.bc).unwrap();
    let sol = pc.solve(1.0, 1e-10, 30).unwrap();
    assert!(sol.stats.n_active > 10);
    assert!(taylor_order(&closed, &pc, &sol.u, 1.0, true) > 1.8, "closed frictionless");
    let rep = pc.regularisation_report(&sol);
    assert!(rep.max_penetration <= 3.0 * rep.penetration_scale + 1e-12 && rep.note.contains("O(stress / eps_n)"), "{rep:?}");
    // sticking friction: the unsymmetric part of the tangent matters, and the full tangent converges at second order
    let stick = fixture(0.3, 1e4, 2e-5, 0.0, E);
    let pc = PenaltyContact::new(&stick.model, stick.specs.clone(), &stick.loads, &stick.bc).unwrap();
    let sol = pc.solve(1.0, 1e-10, 40).unwrap();
    let state = classify_contact(&stick.specs, &pc.evaluate(&sol.u, 1.0, 1.0, false).unwrap().4, 1e-6);
    assert!(state.total(|i| i.stick) > 10 && state.total(|i| i.slip) == 0, "{:?}", state.interfaces);
    let full = taylor_order(&stick, &pc, &sol.u, 1.0, true);
    assert!(full > 1.8, "sticking, exact tangent: order {full}");
    // sliding friction
    let slide = fixture(0.3, 1e4, 0.05, 0.0, E);
    let pc = PenaltyContact::new(&slide.model, slide.specs.clone(), &slide.loads, &slide.bc).unwrap();
    let sol = pc.solve(1.0, 1e-10, 60).unwrap();
    let state = classify_contact(&slide.specs, &pc.evaluate(&sol.u, 1.0, 1.0, false).unwrap().4, 1e-6);
    assert!(state.total(|i| i.slip) > 10, "{:?}", state.interfaces);
    let full = taylor_order(&slide, &pc, &sol.u, 1.0, true);
    let sym_only = taylor_order(&slide, &pc, &sol.u, 1.0, false);
    assert!(full > 1.8, "sliding, exact tangent: order {full}");
    assert!(sym_only < 1.5, "the symmetric part alone is only a first-order tangent of a sliding contact: order {sym_only}");
}

fn solve_quantity(fx: &Fixture, lambda: f64) -> f64 {
    let pc = PenaltyContact::new(&fx.model, fx.specs.clone(), &fx.loads, &fx.bc).unwrap();
    let sol = pc.solve(lambda, 1e-11, 60).unwrap();
    quantity(&fx.model).iter().zip(&sol.u).map(|(a, b)| a * b).sum()
}

#[test]
fn penalty_contact_sensitivities_match_finite_differences_direct_equals_adjoint_frictionless_and_frictional() {
    for (mu, shift, label) in [(0.0, 0.0, "frictionless"), (0.3, 2e-5, "sticking friction (unsymmetric tangent)")] {
        let q = 1.0e4;
        let fx = fixture(mu, q, shift, 0.0, E);
        let pc = PenaltyContact::new(&fx.model, fx.specs.clone(), &fx.loads, &fx.bc).unwrap();
        let sol = pc.solve(1.0, 1e-11, 40).unwrap();
        let g = quantity(&fx.model);
        let params = [ContactParam::LoadScale, ContactParam::LoadFactor, ContactParam::ModulusScale(0)];
        let (direct, rd) = pc.sensitivities(&sol, &[g.clone()], &params, SensitivityMethod::Direct, 1e-6).unwrap();
        let (adjoint, ra) = pc.sensitivities(&sol, &[g.clone()], &params, SensitivityMethod::DiscreteAdjoint, 1e-6).unwrap();
        assert_eq!(rd.unsymmetric, mu > 0.0, "{label}");
        assert_eq!(ra.semantics, DerivativeSemantics::ActiveBranch, "{label}: {:?}", ra.state.notes);
        for pi in 0..params.len() {
            let (d, a) = (direct[0][pi], adjoint[0][pi]);
            assert!((d - a).abs() < 1e-8 * d.abs().max(a.abs()).max(1e-14), "{label} p{pi}: direct {d:e} vs adjoint {a:e}");
        }
        // finite differences of the penalty solve itself
        let richardson = |f: &dyn Fn(f64) -> f64, h: f64| {
            let d = |h: f64| (f(h) - f(-h)) / (2.0 * h);
            (4.0 * d(0.5 * h) - d(h)) / 3.0
        };
        // load scale: scale the pressure
        let fd_load = richardson(&|s| solve_quantity(&fixture(mu, q * (1.0 + s), shift, 0.0, E), 1.0), 4e-3);
        // modulus scale
        let fd_mod = richardson(&|s| solve_quantity(&fixture(mu, q, shift, 0.0, E * (1.0 + s)), 1.0), 4e-3);
        // load factor: lambda scales the pressure and the plane's travel
        let fd_lambda = richardson(&|s| solve_quantity(&fixture(mu, q, shift, 0.0, E), 1.0 + s), 4e-3);
        for (name, ad, fd) in [("load scale", adjoint[0][0], fd_load), ("load factor", adjoint[0][1], fd_lambda), ("modulus", adjoint[0][2], fd_mod)] {
            assert!((ad - fd).abs() < 2e-5 * fd.abs().max(ad.abs()).max(1e-14), "{label}: {name}: exact {ad:e} vs finite difference {fd:e}");
        }
    }
}
