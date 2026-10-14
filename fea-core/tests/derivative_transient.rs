//! Level 6 of the derivative suite: sensitivities of the Newmark / HHT-alpha response through the time steps, direct and adjoint
//! (stored and checkpointed trajectories), against each other and against Richardson-extrapolated central differences of the
//! full transient solve.

use fea_core::derivative::linear_sens::LinearParam;
use fea_core::derivative::transient::*;
use fea_core::dynamics::{MassKind, TransientOptions};
use fea_core::generate::grid;
use fea_core::loads::SurfaceLoad;
use fea_core::*;

const FREQ: f64 = 150.0;

fn g(t: f64) -> f64 {
    (2.0 * std::f64::consts::PI * FREQ * t).sin() + 0.3 * t * 1000.0
}

struct Case {
    model: Model,
    bc: Dirichlet,
    loads: Loads,
    opt: TransientOptions,
    tip: usize,
}

#[derive(Clone, Copy)]
enum Shift {
    Modulus(f64),
    Poisson(f64),
    Density(f64),
    Thickness(f64),
    Coord { node: usize, axis: usize, by: f64 },
    RayleighMass(f64),
    RayleighStiffness(f64),
}

fn case(kind: ElementKind, mass: MassKind, alpha: f64, shift: Option<Shift>) -> Case {
    let mut mesh = grid(Physics::PlaneStress { thickness: 0.5 }, kind, Elastic::new(1.0e5, 0.3), [6, 3, 1], &|p| [2.0 * p[0], p[1], 0.0]).unwrap();
    mesh.set_density(0, 1.0e-4).unwrap();
    let mut rayleigh = (2.0, 1.0e-5);
    match shift {
        Some(Shift::Modulus(s)) => mesh.blocks[0].material.e *= 1.0 + s,
        Some(Shift::Poisson(s)) => mesh.blocks[0].material.nu += s,
        Some(Shift::Density(s)) => mesh.set_density(0, 1.0e-4 * (1.0 + s)).unwrap(),
        Some(Shift::Thickness(s)) => mesh.physics = Physics::PlaneStress { thickness: 0.5 + s },
        Some(Shift::Coord { node, axis, by }) => mesh.nodes[node][axis] += by,
        Some(Shift::RayleighMass(s)) => rayleigh.0 += s,
        Some(Shift::RayleighStiffness(s)) => rayleigh.1 += s,
        None => {}
    }
    let model = Model::new(mesh).unwrap();
    let mut bc = model.dirichlet();
    for &n in model.mesh.node_set("u0").unwrap() {
        bc.fix(n, 0, 0.0);
        bc.fix(n, 1, 0.0);
    }
    let right = model.mesh.surfaces["u1"].clone();
    let loads = Loads { faces: right.iter().map(|f| (f.clone(), SurfaceLoad::Traction([0.0, -400.0, 0.0]))).collect(), ..Loads::default() };
    let tip = *model.mesh.node_set("u1").unwrap().iter().max_by(|&&a, &&b| model.mesh.nodes[a][1].total_cmp(&model.mesh.nodes[b][1])).unwrap();
    let opt = TransientOptions { dt: 2.0e-4, steps: 24, alpha, rayleigh, mass, record: vec![tip * 2 + 1] };
    Case { model, bc, loads, opt, tip }
}

fn param(shift: Shift) -> TransientParam {
    match shift {
        Shift::Modulus(_) => TransientParam::Static(LinearParam::ModulusScale(0)),
        Shift::Poisson(_) => TransientParam::Static(LinearParam::PoissonRatio(0)),
        Shift::Density(_) => TransientParam::DensityScale(0),
        Shift::Thickness(_) => TransientParam::Static(LinearParam::Thickness),
        Shift::Coord { node, axis, .. } => TransientParam::Static(LinearParam::Coord { node, axis }),
        Shift::RayleighMass(_) => TransientParam::RayleighMass,
        Shift::RayleighStiffness(_) => TransientParam::RayleighStiffness,
    }
}

fn with(shift: Shift, h: f64) -> Shift {
    match shift {
        Shift::Modulus(_) => Shift::Modulus(h),
        Shift::Poisson(_) => Shift::Poisson(h),
        Shift::Density(_) => Shift::Density(h),
        Shift::Thickness(_) => Shift::Thickness(h),
        Shift::Coord { node, axis, .. } => Shift::Coord { node, axis, by: h },
        Shift::RayleighMass(_) => Shift::RayleighMass(h),
        Shift::RayleighStiffness(_) => Shift::RayleighStiffness(h),
    }
}

/// `(tip displacement at the last step, trapezoid integral of the tip displacement)` of the library's own integrator.
fn qois_of(c: &Case) -> (f64, f64) {
    let r = c.model.transient(&c.loads, &g, &c.bc, None, &c.opt).unwrap();
    let h = &r.history[0];
    let last = *h.last().unwrap();
    let integral: f64 = (0..h.len() - 1).map(|i| 0.5 * c.opt.dt * (h[i] + h[i + 1])).sum();
    (last, integral)
}

fn central(kind: ElementKind, mass: MassKind, alpha: f64, shift: Shift, h: f64) -> (f64, f64) {
    let d = |h: f64| {
        let (p, m) = (qois_of(&case(kind, mass, alpha, Some(with(shift, h)))), qois_of(&case(kind, mass, alpha, Some(with(shift, -h)))));
        ((p.0 - m.0) / (2.0 * h), (p.1 - m.1) / (2.0 * h))
    };
    let (a, b) = (d(h), d(0.5 * h));
    ((4.0 * b.0 - a.0) / 3.0, (4.0 * b.1 - a.1) / 3.0)
}

fn qoi_set(c: &Case) -> Vec<TransientQoi> {
    let n = c.model.mesh.n_dofs();
    vec![TransientQoi::displacement(c.opt.steps, c.tip * 2 + 1, n), TransientQoi::displacement_integral(c.tip * 2 + 1, c.opt.steps, c.opt.dt, n)]
}

#[test]
fn the_sensitivity_engine_reproduces_the_library_integrator() {
    for (alpha, mass) in [(0.0, MassKind::Consistent), (-0.1, MassKind::Consistent), (-0.1, MassKind::Lumped)] {
        let c = case(ElementKind::Quad4, mass, alpha, None);
        let eng = TransientSensitivity::new(&c.model, &c.loads, &g, &c.bc, None, &c.opt);
        let states = eng.response().unwrap();
        let lib = c.model.transient(&c.loads, &g, &c.bc, None, &c.opt).unwrap();
        let dof = c.tip * 2 + 1;
        for (s, st) in states.iter().enumerate() {
            let scale = lib.history[0].iter().fold(0.0f64, |m, v| m.max(v.abs()));
            assert!((st.u[dof] - lib.history[0][s]).abs() < 1e-11 * scale, "alpha {alpha} step {s}: {} vs {}", st.u[dof], lib.history[0][s]);
        }
    }
}

fn check(kind: ElementKind, mass: MassKind, alpha: f64, shifts: &[(Shift, f64)], tol: f64) {
    let c = case(kind, mass, alpha, None);
    let eng = TransientSensitivity::new(&c.model, &c.loads, &g, &c.bc, None, &c.opt);
    let params: Vec<TransientParam> = shifts.iter().map(|(s, _)| param(*s)).collect();
    let qois = qoi_set(&c);
    let (direct, _) = eng.direct(&params, &qois).unwrap();
    let (adj, rep) = eng.adjoint(&params, &qois, Storage::Store).unwrap();
    // a parameter that cancels out of the response (the thickness scales stiffness, mass, damping and the traction alike) has an exact
    // sensitivity of zero; compare such values against the size of the response, not against themselves
    let base = qois_of(&c);
    let floor = [1e-5 * base.0.abs(), 1e-5 * base.1.abs()];
    let (adj_ck, rep_ck) = eng.adjoint(&params, &qois, Storage::Checkpoint(5)).unwrap();
    assert!(rep_ck.peak_states < rep.peak_states, "checkpointing must hold fewer states: {} vs {}", rep_ck.peak_states, rep.peak_states);
    for (pi, (shift, h)) in shifts.iter().enumerate() {
        let fd = central(kind, mass, alpha, *shift, *h);
        for (qi, fdv) in [fd.0, fd.1].into_iter().enumerate() {
            let (d, a, k) = (direct[qi][pi], adj[qi][pi], adj_ck[qi][pi]);
            let scale = d.abs().max(floor[qi]);
            assert!((d - a).abs() < 1e-9 * scale, "{kind:?} alpha {alpha} param {pi} qoi {qi}: direct {d:e} vs adjoint {a:e}");
            assert_eq!(a.to_bits(), k.to_bits(), "{kind:?} param {pi} qoi {qi}: checkpointed adjoint differs from stored: {a:e} vs {k:e}");
            assert!((d - fdv).abs() < tol * scale, "{kind:?} alpha {alpha} param {pi} qoi {qi}: exact {d:e} vs finite difference {fdv:e}");
        }
    }
}

#[test]
fn direct_adjoint_and_checkpointed_adjoint_match_finite_differences_for_newmark() {
    let shifts = [(Shift::Modulus(0.0), 1e-2), (Shift::Density(0.0), 1e-2), (Shift::Thickness(0.0), 1e-3), (Shift::Poisson(0.0), 1e-3), (Shift::RayleighMass(0.0), 0.05), (Shift::RayleighStiffness(0.0), 1e-6)];
    check(ElementKind::Quad4, MassKind::Consistent, 0.0, &shifts, 1e-5);
}

#[test]
fn hht_alpha_and_lumped_mass_are_differentiated_exactly_too() {
    let shifts = [(Shift::Modulus(0.0), 1e-2), (Shift::Density(0.0), 1e-2), (Shift::RayleighStiffness(0.0), 1e-6)];
    check(ElementKind::Quad4, MassKind::Consistent, -0.1, &shifts, 1e-5);
    check(ElementKind::Quad4, MassKind::Lumped, -0.2, &shifts, 1e-5);
}

#[test]
fn geometry_parameters_reach_stiffness_mass_and_the_load() {
    let c = case(ElementKind::Quad4, MassKind::Consistent, -0.1, None);
    // an interior node and a node on the loaded face
    let interior = (0..c.model.mesh.nodes.len()).find(|&n| (c.model.mesh.nodes[n][0] - 2.0 / 3.0).abs() < 1e-9 && (c.model.mesh.nodes[n][1] - 1.0 / 3.0).abs() < 1e-9).unwrap();
    let shifts = [(Shift::Coord { node: interior, axis: 0, by: 0.0 }, 1e-3), (Shift::Coord { node: interior, axis: 1, by: 0.0 }, 1e-3), (Shift::Coord { node: c.tip, axis: 1, by: 0.0 }, 1e-3)];
    check(ElementKind::Quad4, MassKind::Consistent, -0.1, &shifts, 2e-4);
}

#[test]
fn quadratic_elements_are_supported() {
    let shifts = [(Shift::Modulus(0.0), 1e-2), (Shift::Density(0.0), 1e-2)];
    check(ElementKind::Quad8, MassKind::Consistent, -0.05, &shifts, 1e-5);
}

#[test]
fn the_adjoint_cost_does_not_grow_with_the_number_of_parameters_in_solves() {
    let c = case(ElementKind::Quad4, MassKind::Consistent, 0.0, None);
    let eng = TransientSensitivity::new(&c.model, &c.loads, &g, &c.bc, None, &c.opt);
    let qois = qoi_set(&c)[..1].to_vec();
    let few = [TransientParam::Static(LinearParam::ModulusScale(0))];
    let many = [TransientParam::Static(LinearParam::ModulusScale(0)), TransientParam::DensityScale(0), TransientParam::RayleighMass, TransientParam::RayleighStiffness, TransientParam::Static(LinearParam::Thickness)];
    let (_, ra) = eng.adjoint(&few, &qois, Storage::Store).unwrap();
    let (_, rb) = eng.adjoint(&many, &qois, Storage::Store).unwrap();
    assert_eq!(ra.effective_solves, rb.effective_solves, "adjoint solves are per quantity");
    let (_, da) = eng.direct(&few, &qois).unwrap();
    let (_, db) = eng.direct(&many, &qois).unwrap();
    assert_eq!(db.effective_solves, da.effective_solves * 5, "direct solves are per parameter");
}
