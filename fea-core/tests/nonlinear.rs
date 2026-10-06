//! Nonlinear analysis: material nonlinearity (small and finite strain) against closed-form
//! solutions, quadratic Newton convergence, limit load of a thick cylinder with arc-length control.

#![allow(clippy::needless_range_loop)] // index-parallel numeric kernels read clearer as loops

use fea_core::generate::grid;
use fea_core::material::J2;
use fea_core::nonlinear::Stop;
use fea_core::*;
use mechanics_core::hardening::Hardening;

const E: f64 = 10.0e6;
const NU: f64 = 0.3;

fn sy_law() -> Hardening {
    Hardening::linear(40_000.0, 2.0e5, 5.0).unwrap()
}

/// A uniform uniaxial-tension specimen: symmetry planes on the low faces, the axial displacement
/// `delta` (scaled by the load factor) on the opposite face, every other face free. Axisymmetric
/// specimens are a cylinder (radius 1, length 2, axis = the `y` axis); the others are a 2 x 1 (x 0.5) block
/// stretched along `x`. Returns the model, its constraints, the reference cross-section area and
/// `(axial component, coordinate index of the axial direction)`.
fn tension_specimen(physics: Physics, kind: ElementKind, j2: Option<J2>, delta: f64) -> (Model, Dirichlet, f64, (usize, usize)) {
    let (lx, ly, lz) = (2.0, 1.0, 0.5);
    let d = physics.dim();
    let axisym = matches!(physics, Physics::Axisymmetric);
    let axial = if axisym { 1 } else { 0 };
    let mut mesh = grid(physics, kind, Elastic::new(E, NU), [2, 2, 2], &move |p| if axisym { [ly * p[0], lx * p[1], 0.0] } else { [lx * p[0], ly * p[1], if d == 3 { lz * p[2] } else { 0.0 }] }).unwrap();
    if let Some(j) = j2 {
        mesh.set_plasticity(0, j).unwrap();
    }
    let model = Model::new(mesh).unwrap();
    let mut bc = model.dirichlet();
    for n in 0..model.mesh.nodes.len() {
        let x = model.mesh.nodes[n];
        for c in 0..d {
            if x[c] < 1e-12 {
                bc.fix(n, c, 0.0); // symmetry planes (the axis for r = 0)
            }
        }
        if (x[axial] - lx).abs() < 1e-12 {
            bc.fix(n, axial, delta);
        }
    }
    let area = match physics {
        Physics::PlaneStress { thickness } | Physics::PlaneStrain { thickness } => ly * thickness,
        Physics::Axisymmetric => std::f64::consts::PI * ly * ly,
        Physics::Solid => ly * lz,
    };
    (model, bc, area, (axial, axial))
}

/// Force carried by the specimen: minus the axial reactions on the `x_axial = 0` face.
fn axial_force(model: &Model, sol: &fea_core::NlSolution, axial: (usize, usize)) -> f64 {
    let d = model.mesh.dim();
    -(0..model.mesh.nodes.len()).filter(|&n| model.mesh.nodes[n][axial.1] < 1e-12).map(|n| sol.reactions[n * d + axial.0]).sum::<f64>()
}

#[test]
fn a_plastic_block_with_a_huge_yield_stress_is_the_linear_solution_in_one_newton_step() {
    let elastic = Hardening::linear(1.0e30, 0.0, 1.0).unwrap();
    for (physics, kind) in [(Physics::PlaneStrain { thickness: 1.0 }, ElementKind::Quad9), (Physics::PlaneStress { thickness: 0.3 }, ElementKind::Quad8), (Physics::Solid, ElementKind::Hex20)] {
        let d = physics.dim();
        let mut mesh = grid(physics, kind, Elastic::new(E, NU), [3, 2, 2], &|p| [3.0 * p[0], 1.0 * p[1], 0.7 * p[2]]).unwrap();
        mesh.set_plasticity(0, J2::small(elastic)).unwrap();
        let model = Model::new(mesh).unwrap();
        let mut bc = model.dirichlet();
        for &n in model.mesh.node_set("u0").unwrap() {
            bc.fix_node(n);
        }
        let loads = Loads { faces: model.mesh.surfaces["u1"].iter().map(|f| (f.clone(), SurfaceLoad::Traction([4000.0, -2500.0, 800.0]))).collect(), body: Some([0.0, -30.0, 10.0]), ..Loads::default() };
        let lin = model.solve_static(&loads, &bc).unwrap();
        let nl = model.solve_nonlinear(&loads, &bc, &NlOptions { steps: 1, ..NlOptions::default() }).unwrap();
        assert_eq!(nl.stop, Stop::Completed);
        let scale = lin.u.iter().fold(0.0f64, |m, v| m.max(v.abs()));
        let diff = lin.u.iter().zip(&nl.u).fold(0.0f64, |m, (a, b)| m.max((a - b).abs())) / scale;
        eprintln!("{kind:?} d={d}: nonlinear vs linear {diff:.2e}, iterations {}", nl.steps[0].iterations);
        assert!(diff < 1e-9, "{kind:?}: {diff:e}");
        assert!(nl.steps[0].iterations <= 3, "a linear problem converges at once: {:?}", nl.steps[0].residuals);
    }
}

#[test]
fn small_strain_uniaxial_tension_follows_the_hardening_curve_in_every_analysis_type() {
    let law = sy_law();
    let delta = 0.02; // strain 1 %
    for (physics, kind) in [(Physics::Solid, ElementKind::Hex8), (Physics::Solid, ElementKind::Hex27), (Physics::PlaneStress { thickness: 0.4 }, ElementKind::Quad9), (Physics::Axisymmetric, ElementKind::Quad9)] {
        let (model, bc, area, ax) = tension_specimen(physics, kind, Some(J2::small(law)), delta);
        let lx = 2.0;
        let sol = model.solve_nonlinear(&Loads::default(), &bc, &NlOptions::default()).unwrap();
        assert_eq!(sol.stop, Stop::Completed, "{physics:?} {kind:?}");
        let strain = delta / lx;
        // Uniaxial stress: eps = sigma / E + ep, sigma = sigma_0 + H ep.
        let (h, s0) = (2.0e5, 40_000.0);
        let ep = (E * strain - s0) / (E + h);
        let sigma = s0 + h * ep;
        let force = axial_force(&model, &sol, ax);
        let got = force / area;
        eprintln!("{physics:?} {kind:?}: stress {got:.4} vs {sigma:.4}, ep {:.6} vs {ep:.6}, iterations {:?}", sol.state.max_ep(), sol.steps.iter().map(|s| s.iterations).collect::<Vec<_>>());
        assert!((got - sigma).abs() < 1e-6 * sigma, "{physics:?} {kind:?}: stress {got} vs {sigma}");
        assert!((sol.state.max_ep() - ep).abs() < 1e-8, "{physics:?} {kind:?}: ep {} vs {ep}", sol.state.max_ep());
    }
}

/// Bisection for the uniaxial finite-strain state: `ln(stretch) = tau / E + ep`, `tau = sigma_y(ep)`.
fn finite_uniaxial(law: &Hardening, stretch: f64) -> (f64, f64) {
    let target = stretch.ln();
    let g = |ep: f64| law.stress(ep) / E + ep - target;
    if g(0.0) >= 0.0 {
        return (target * E, 0.0); // elastic
    }
    let (mut lo, mut hi) = (0.0, target);
    for _ in 0..200 {
        let mid = 0.5 * (lo + hi);
        if g(mid) > 0.0 {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    let ep = 0.5 * (lo + hi);
    (law.stress(ep), ep)
}

#[test]
fn finite_strain_uniaxial_tension_matches_the_logarithmic_analytic_solution_to_large_stretch() {
    // Linear true-stress hardening, steep enough that uniform tension stays stable (no necking) up to
    // stretch 2: `d sigma / d eps >= sigma` for `eps <= 0.8`.
    let law = Hardening::linear(40_000.0, 2.0e5, 5.0).unwrap();
    let lx = 2.0;
    for (physics, kind) in [(Physics::Solid, ElementKind::Hex8), (Physics::Solid, ElementKind::Hex20), (Physics::PlaneStress { thickness: 0.4 }, ElementKind::Quad9), (Physics::Axisymmetric, ElementKind::Quad9)] {
        for stretch in [1.002, 1.05, 1.4, 2.0] {
            let delta = (stretch - 1.0) * lx;
            let (model, bc, area, ax) = tension_specimen(physics, kind, Some(J2::finite(law)), delta);
            let sol = model.solve_nonlinear(&Loads::default(), &bc, &NlOptions { steps: 8, ..NlOptions::default() }).unwrap();
            assert_eq!(sol.stop, Stop::Completed, "{physics:?} {kind:?} stretch {stretch}");
            let (tau, ep) = finite_uniaxial(&law, stretch);
            // Nominal stress P = F / A0 and the Kirchhoff stress tau = P * stretch.
            let p = axial_force(&model, &sol, ax) / area;
            eprintln!("{physics:?} {kind:?} stretch {stretch}: tau {:.3} vs {tau:.3}, ep {:.5} vs {ep:.5}", p * stretch, sol.state.max_ep());
            assert!((p * stretch - tau).abs() < 1e-6 * tau, "{physics:?} {kind:?} stretch {stretch}: tau {} vs {tau}", p * stretch);
            assert!((sol.state.max_ep() - ep).abs() < 1e-7, "{physics:?} {kind:?} stretch {stretch}: ep {} vs {ep}", sol.state.max_ep());
        }
    }
}

// ---------------------------------------------------------------- thick-walled cylinder

fn cylinder(nu: f64, nr: usize, physics: Physics, large: bool, p_ref: f64) -> (Model, Dirichlet, Loads) {
    let (a, b) = (1.0f64, 2.0f64);
    let axisym = matches!(physics, Physics::Axisymmetric);
    // Plane problems: a quarter annulus with symmetry edges. Axisymmetric: the wall section (r, z) with
    // plane strain along z (u_z = 0 on both ends).
    let mut mesh = grid(physics, ElementKind::Quad9, Elastic::new(E, nu), [nr, 2, 1], &move |p| {
        if axisym {
            [a + (b - a) * p[0], 0.2 * p[1], 0.0]
        } else {
            let (r, th) = (a + (b - a) * p[0], 0.5 * std::f64::consts::PI * p[1]);
            [r * th.cos(), r * th.sin(), 0.0]
        }
    })
    .unwrap();
    let law = Hardening::linear(40_000.0, 0.0, 5.0).unwrap();
    mesh.set_plasticity(0, if large { J2::finite(law) } else { J2::small(law) }).unwrap();
    let model = Model::new(mesh).unwrap();
    let mut bc = model.dirichlet();
    for name in ["v0", "v1"] {
        for &n in model.mesh.node_set(name).unwrap() {
            if axisym {
                bc.fix(n, 1, 0.0);
            } else {
                bc.fix(n, if name == "v0" { 1 } else { 0 }, 0.0);
            }
        }
    }
    let loads = Loads { faces: model.mesh.surfaces["u0"].iter().map(|f| (f.clone(), SurfaceLoad::Pressure(p_ref))).collect(), ..Loads::default() };
    (model, bc, loads)
}

const SY: f64 = 40_000.0;

/// Exact limit pressure of a plane-strain perfectly plastic thick cylinder, `b / a = 2`.
fn limit_pressure() -> f64 {
    2.0 / 3.0f64.sqrt() * SY * 2.0f64.ln()
}

fn plastic_radius(model: &Model, sol: &fea_core::NlSolution) -> f64 {
    sol.state.gauss_ep(&model.mesh).iter().filter(|(_, ep)| *ep > 1e-9).map(|(x, _)| x[0].hypot(x[1])).fold(0.0f64, f64::max)
}

#[test]
fn newton_converges_quadratically_in_a_plastic_step() {
    for large in [false, true] {
        let (model, bc, loads) = cylinder(0.3, 12, Physics::PlaneStrain { thickness: 1.0 }, large, 0.7 * limit_pressure());
        let sol = model.solve_nonlinear(&loads, &bc, &NlOptions { steps: 10, ..NlOptions::default() }).unwrap();
        assert_eq!(sol.stop, Stop::Completed);
        // The step that spreads the plastic zone: the last residuals fall with order ~ 2.
        let st = sol.steps.iter().rfind(|s| s.max_ep > 0.0 && s.iterations >= 4).expect("a plastic step");
        let r = &st.residuals;
        let n = r.len();
        let order = (r[n - 2] / r[n - 3]).ln() / (r[n - 3] / r[n - 4]).ln();
        eprintln!("large {large}: residuals {:?}, local order {order:.2}", r.iter().map(|v| format!("{v:.1e}")).collect::<Vec<_>>());
        assert!(order > 1.8, "large {large}: convergence order {order:.2} from {:?}", r);
        assert!(r[n - 1] < 1e-6 * r[0]);
    }
}

#[test]
fn thick_cylinder_arc_length_reaches_the_exact_limit_pressure_from_below_as_the_mesh_refines() {
    let pl = limit_pressure();
    let mut peaks = Vec::new();
    for nr in [4, 8, 16] {
        let (model, bc, loads) = cylinder(0.3, nr, Physics::PlaneStrain { thickness: 1.0 }, false, pl);
        let sol = model.solve_nonlinear(&loads, &bc, &NlOptions { control: Control::Arc { ds: 2e-3, lambda_max: 1.3, max_steps: 100 }, ..NlOptions::default() }).unwrap();
        assert_eq!(sol.stop, Stop::StepLimit, "the load plateaus at the limit pressure, never reaching lambda_max");
        let peak = sol.peak_lambda();
        // After the first yielding the load factor never decreases by more than the plastic-plateau noise.
        let tail = &sol.steps[sol.steps.len() / 2..];
        assert!(tail.iter().all(|s| s.lambda > 0.995 * peak), "{nr}: the response must plateau, not collapse");
        assert!(sol.state.max_ep() > 0.05, "the plateau is a plastic flow regime");
        eprintln!("{nr} elements: peak p / p_L = {peak:.5}, {} steps, {} factorisations", sol.steps.len(), sol.factorisations);
        peaks.push(peak);
    }
    assert!(peaks[0] < 1.02 && peaks[1] < 1.003 && peaks[2] < 1.001, "limit loads {peaks:?}");
    assert!(peaks[2] > 0.995, "{peaks:?}");
    assert!(peaks[2] < peaks[1] && peaks[1] < peaks[0], "the limit load converges from above: {peaks:?}");
}

#[test]
fn plastic_zone_grows_as_hills_incompressible_solution() {
    // Hill: p(c) = (2/sqrt 3) sigma_y [ln (c / a) + (1 - c^2 / b^2) / 2], a = 1, b = 2 (nu near 1/2).
    let pl = limit_pressure();
    let hill = |c: f64| 2.0 / 3.0f64.sqrt() * SY * (c.ln() + 0.5 * (1.0 - c * c / 4.0));
    for frac in [0.7, 0.85, 0.95] {
        let (model, bc, loads) = cylinder(0.49, 16, Physics::PlaneStrain { thickness: 1.0 }, false, frac * pl);
        let sol = model.solve_nonlinear(&loads, &bc, &NlOptions { steps: 10, ..NlOptions::default() }).unwrap();
        assert_eq!(sol.stop, Stop::Completed);
        let (mut lo, mut hi) = (1.0f64, 2.0);
        for _ in 0..80 {
            let m = 0.5 * (lo + hi);
            if hill(m) < frac * pl {
                lo = m;
            } else {
                hi = m;
            }
        }
        let c_hill = 0.5 * (lo + hi);
        let c_fe = plastic_radius(&model, &sol);
        eprintln!("p = {frac} p_L: plastic radius FE {c_fe:.3} vs Hill {c_hill:.3}");
        // The FE radius is the outermost yielded Gauss point: within one element of the exact front.
        assert!((c_fe - c_hill).abs() < 0.05, "p = {frac} p_L: {c_fe} vs {c_hill}");
    }
}

#[test]
fn small_and_finite_strain_agree_in_the_elastic_range_and_stay_close_in_the_plastic_range() {
    let pl = limit_pressure();
    let solve = |large: bool, frac: f64| {
        let (model, bc, loads) = cylinder(0.3, 8, Physics::PlaneStrain { thickness: 1.0 }, large, frac * pl);
        let sol = model.solve_nonlinear(&loads, &bc, &NlOptions { steps: 10, ..NlOptions::default() }).unwrap();
        assert_eq!(sol.stop, Stop::Completed);
        let ua = (0..model.mesh.nodes.len()).filter(|&n| (model.mesh.nodes[n][0].hypot(model.mesh.nodes[n][1]) - 1.0).abs() < 1e-9).map(|n| sol.u[n * 2].hypot(sol.u[n * 2 + 1])).fold(0.0f64, f64::max);
        (ua, sol.state.max_ep())
    };
    let (s, f) = (solve(false, 0.3), solve(true, 0.3));
    assert!((s.0 / f.0 - 1.0).abs() < 2e-3 && s.1 == 0.0 && f.1 == 0.0, "elastic: {s:?} vs {f:?}");
    let (s, f) = (solve(false, 0.9), solve(true, 0.9));
    eprintln!("p = 0.9 p_L inner displacement: small {:.5}, finite {:.5} (ep {:.4} / {:.4})", s.0, f.0, s.1, f.1);
    assert!((s.0 / f.0 - 1.0).abs() < 0.08 && s.1 > 0.0 && f.1 > 0.0);
}

#[test]
fn axisymmetric_and_plane_strain_models_of_the_cylinder_agree() {
    let pl = limit_pressure();
    let run = |physics| {
        let (model, bc, loads) = cylinder(0.3, 12, physics, false, 0.85 * pl);
        let sol = model.solve_nonlinear(&loads, &bc, &NlOptions { steps: 10, ..NlOptions::default() }).unwrap();
        assert_eq!(sol.stop, Stop::Completed);
        let ua = (0..model.mesh.nodes.len()).filter(|&n| (model.mesh.nodes[n][0].hypot(model.mesh.nodes[n][1]) - 1.0).abs() < 1e-9).map(|n| sol.u[n * 2].hypot(sol.u[n * 2 + 1])).fold(0.0f64, f64::max);
        (ua, plastic_radius(&model, &sol))
    };
    let (pe, ax) = (run(Physics::PlaneStrain { thickness: 1.0 }), run(Physics::Axisymmetric));
    eprintln!("inner displacement / plastic radius: plane strain {pe:?}, axisymmetric {ax:?}");
    assert!((pe.0 / ax.0 - 1.0).abs() < 2e-3, "{pe:?} vs {ax:?}");
    assert!((pe.1 - ax.1).abs() < 0.04, "{pe:?} vs {ax:?}");
}

#[test]
fn a_single_oversized_step_is_cut_automatically_and_the_answer_is_unchanged() {
    let pl = limit_pressure();
    let (model, bc, loads) = cylinder(0.3, 12, Physics::PlaneStrain { thickness: 1.0 }, false, 0.95 * pl);
    let many = model.solve_nonlinear(&loads, &bc, &NlOptions { steps: 20, ..NlOptions::default() }).unwrap();
    let one = model.solve_nonlinear(&loads, &bc, &NlOptions { steps: 1, max_iter: 5, ..NlOptions::default() }).unwrap();
    assert_eq!(one.stop, Stop::Completed);
    assert!(one.steps.len() > 1, "one huge step cannot converge in 5 iterations, so it must have been split: {} steps", one.steps.len());
    // A plastic, path-dependent state: the displacement differs by far less than a percent between step sizes.
    let scale = many.u.iter().fold(0.0f64, |m, v| m.max(v.abs()));
    let diff = many.u.iter().zip(&one.u).fold(0.0f64, |m, (a, b)| m.max((a - b).abs())) / scale;
    eprintln!("step sizes: {} vs {} steps, solution difference {diff:.2e}", many.steps.len(), one.steps.len());
    assert!(diff < 5e-3, "{diff:e}");
}

#[test]
fn the_failure_strain_stops_the_analysis() {
    let pl = limit_pressure();
    let (model, bc, loads) = cylinder(0.3, 8, Physics::PlaneStrain { thickness: 1.0 }, false, pl);
    let sol = model.solve_nonlinear(&loads, &bc, &NlOptions { control: Control::Arc { ds: 2e-3, lambda_max: 1.3, max_steps: 400 }, failure_ep: Some(0.03), ..NlOptions::default() }).unwrap();
    assert_eq!(sol.stop, Stop::FailureStrain);
    let ep = sol.state.max_ep();
    assert!((0.03..0.05).contains(&ep), "stopped at ep = {ep}");
}

#[test]
fn an_elastic_block_via_the_linear_kernel_equals_the_same_block_through_the_plastic_path() {
    // Left half elastic, right half plastic. The elastic half is computed (a) by the optimised linear
    // kernels and (b) by the plastic code path with an enormous yield stress: the two assemblies of
    // the same physics must agree to rounding in the displacements, the state and the reactions.
    let (lx, ly, nx) = (2.0, 1.0, 4usize);
    let base = grid(Physics::PlaneStress { thickness: 1.0 }, ElementKind::Quad9, Elastic::new(E, NU), [nx, 2, 1], &move |p| [lx * p[0], ly * p[1], 0.0]).unwrap();
    let build = |elastic_through_plastic_path: bool| -> Model {
        let mut mesh = Mesh::new(base.physics);
        mesh.nodes = base.nodes.clone();
        mesh.node_sets = base.node_sets.clone();
        mesh.surfaces = base.surfaces.clone();
        let (mut left, mut right) = (Vec::new(), Vec::new());
        for e in 0..base.blocks[0].n_elems() {
            let c = base.blocks[0].elem(e);
            if base.nodes[c[8]][0] < 0.5 * lx {
                left.extend_from_slice(c);
            } else {
                right.extend_from_slice(c);
            }
        }
        mesh.add_block(ElementKind::Quad9, left, Elastic::new(E, NU), "elastic").unwrap();
        mesh.add_block(ElementKind::Quad9, right, Elastic::new(E, NU), "plastic").unwrap();
        mesh.set_plasticity(1, J2::small(Hardening::linear(40_000.0, 2.0e5, 5.0).unwrap())).unwrap();
        if elastic_through_plastic_path {
            mesh.set_plasticity(0, J2::small(Hardening::linear(1.0e30, 0.0, 1.0).unwrap())).unwrap();
        }
        Model::new(mesh).unwrap()
    };
    let solve = |model: &Model| {
        let mut bc = model.dirichlet();
        for n in 0..model.mesh.nodes.len() {
            let x = model.mesh.nodes[n];
            if x[0] < 1e-12 {
                bc.fix(n, 0, 0.0);
            }
            if x[1] < 1e-12 {
                bc.fix(n, 1, 0.0);
            }
        }
        let loads = Loads { faces: model.mesh.surfaces["u1"].iter().map(|f| (f.clone(), SurfaceLoad::Traction([45_000.0, 0.0, 0.0]))).collect(), ..Loads::default() };
        model.solve_nonlinear(&loads, &bc, &NlOptions { steps: 5, ..NlOptions::default() }).unwrap()
    };
    let (ma, mb) = (build(false), build(true));
    let (a, b) = (solve(&ma), solve(&mb));
    assert_eq!((a.stop.clone(), b.stop.clone()), (Stop::Completed, Stop::Completed));
    assert!(a.state.gp[0].is_empty() && !b.state.gp[0].is_empty(), "elastic blocks carry no state through the linear kernels");
    assert!(b.state.gp[0].iter().all(|s| s[6] == 0.0), "the huge-yield block never yields");
    let scale = a.u.iter().fold(0.0f64, |m, v| m.max(v.abs()));
    let diff = a.u.iter().zip(&b.u).fold(0.0f64, |m, (x, y)| m.max((x - y).abs())) / scale;
    let rscale = a.reactions.iter().fold(0.0f64, |m, v| m.max(v.abs()));
    let rdiff = a.reactions.iter().zip(&b.reactions).fold(0.0f64, |m, (x, y)| m.max((x - y).abs())) / rscale;
    eprintln!("mixed blocks: displacement difference {diff:.2e}, reaction difference {rdiff:.2e}, ep {:.5}", a.state.max_ep());
    assert!(diff < 1e-9 && rdiff < 1e-9, "{diff:e} {rdiff:e}");
    assert!(a.state.max_ep() > 0.0, "the plastic half must have yielded");
    // Yielding is confined to the plastic block (x > L/2).
    assert!(a.state.gauss_ep(&ma.mesh).iter().all(|(x, ep)| *ep == 0.0 || x[0] > 0.5 * lx - 1e-12));
    // Equilibrium: the support reactions carry the applied end force.
    let fx: f64 = (0..ma.mesh.nodes.len()).map(|n| a.reactions[n * 2]).sum();
    assert!((fx + 45_000.0 * ly).abs() < 1e-6 * 45_000.0, "reaction {fx}");
}

#[test]
fn the_failure_strain_can_come_from_the_material_library() {
    // Steel (id "steel"): equivalent plastic strain 0.2267. Unknown and non-metal ids are refused.
    let opt = NlOptions::default().failure_for_material("steel").unwrap();
    assert!((opt.failure_ep.unwrap() - 0.2267).abs() < 1e-4);
    assert!(NlOptions::default().failure_for_material("no_such_material").is_err());
    assert!(NlOptions::default().failure_for_material("cfrp_qi").unwrap_err().contains("does not apply"));
    let pl = limit_pressure();
    let (model, bc, loads) = cylinder(0.3, 8, Physics::PlaneStrain { thickness: 1.0 }, true, pl);
    let sol = model.solve_nonlinear(&loads, &bc, &NlOptions { control: Control::Arc { ds: 2e-3, lambda_max: 1.3, max_steps: 800 }, ..opt }).unwrap();
    assert_eq!(sol.stop, Stop::FailureStrain);
    assert!((0.2267..0.26).contains(&sol.state.max_ep()), "stopped at ep {}", sol.state.max_ep());
}

#[test]
fn chord_newton_saves_factorisations_and_reaches_the_same_solution() {
    let pl = limit_pressure();
    for (large, frac) in [(false, 0.9), (true, 0.9)] {
        let (model, bc, loads) = cylinder(0.3, 16, Physics::PlaneStrain { thickness: 1.0 }, large, frac * pl);
        let full = model.solve_nonlinear(&loads, &bc, &NlOptions { steps: 10, ..NlOptions::default() }).unwrap();
        let chord = model.solve_nonlinear(&loads, &bc, &NlOptions { steps: 10, chord_iters: 5, ..NlOptions::default() }).unwrap();
        assert_eq!((full.stop.clone(), chord.stop.clone()), (Stop::Completed, Stop::Completed));
        let iters = |s: &fea_core::NlSolution| s.steps.iter().map(|st| st.iterations).sum::<usize>();
        eprintln!("large {large}: full Newton {} iterations / {} factorisations / {:.0} ms; chord {} iterations / {} factorisations / {:.0} ms", iters(&full), full.factorisations, full.elapsed_ms, iters(&chord), chord.factorisations, chord.elapsed_ms);
        let scale = full.u.iter().fold(0.0f64, |m, v| m.max(v.abs()));
        let diff = full.u.iter().zip(&chord.u).fold(0.0f64, |m, (a, b)| m.max((a - b).abs())) / scale;
        assert!(diff < 1e-7, "the solutions differ by {diff:e}");
        assert!(chord.factorisations < full.factorisations, "{} vs {}", chord.factorisations, full.factorisations);
        assert!((full.state.max_ep() - chord.state.max_ep()).abs() < 1e-7);
    }
}

#[test]
fn finite_strain_collapse_peaks_just_below_the_limit_load_then_softens_with_an_indefinite_tangent() {
    // With geometric nonlinearity the thinning wall loses capacity: the load peaks slightly under the
    // small-strain limit pressure and then falls. Tracing that needs the arc-length controller, a secant
    // predictor (the tangent is singular on the plateau) and an indefinite factorization.
    let pl = limit_pressure();
    let (model, bc, loads) = cylinder(0.3, 8, Physics::PlaneStrain { thickness: 1.0 }, true, pl);
    let run = |ds: f64| {
        let opt = NlOptions { control: Control::Arc { ds, lambda_max: 1.3, max_steps: 900 }, failure_ep: Some(0.2267), ..NlOptions::default() };
        model.solve_nonlinear(&loads, &bc, &opt).unwrap()
    };
    let (a, b) = (run(2e-3), run(5e-4));
    for s in [&a, &b] {
        assert_eq!(s.stop, Stop::FailureStrain);
        let peak = s.peak_lambda();
        assert!((0.995..0.9985).contains(&peak), "peak p / p_L = {peak}");
        assert!(s.lambda < peak - 0.02, "the load must fall past the peak: peak {peak}, end {}", s.lambda);
        // Everything before the peak has a positive definite tangent; the softening branch does not.
        let i_peak = s.steps.iter().position(|st| st.lambda == peak).unwrap();
        assert!(s.steps[..i_peak / 2].iter().all(|st| !st.indefinite), "no indefinite tangent on the rising branch");
        assert!(s.steps[i_peak..].iter().any(|st| st.indefinite), "the softening branch has an indefinite tangent");
        assert!(s.steps.iter().all(|st| st.cuts <= 1), "the controller should not need repeated step cutting");
    }
    // Step-size independent: the end of the path agrees.
    assert!((a.lambda - b.lambda).abs() < 2e-3 && (a.peak_lambda() - b.peak_lambda()).abs() < 2e-4, "{} {} / {} {}", a.lambda, b.lambda, a.peak_lambda(), b.peak_lambda());
}

#[test]
fn axisymmetric_thick_sphere_collapses_at_the_exact_limit_pressure() {
    // Perfectly plastic hollow sphere (J2): sigma_theta - sigma_r = sigma_y, so p_L = 2 sigma_y ln(b / a), exactly.
    let (a, b) = (1.0f64, 2.0f64);
    let pl = 2.0 * SY * (b / a).ln();
    let mut peaks = Vec::new();
    for nr in [4, 8] {
        let mut mesh = grid(Physics::Axisymmetric, ElementKind::Quad9, Elastic::new(E, 0.3), [nr, 4, 1], &move |p| {
            let (rho, ph) = (a + (b - a) * p[0], 0.5 * std::f64::consts::PI * p[1]);
            [rho * ph.cos(), rho * ph.sin(), 0.0]
        })
        .unwrap();
        mesh.set_plasticity(0, J2::small(Hardening::linear(SY, 0.0, 5.0).unwrap())).unwrap();
        let model = Model::new(mesh).unwrap();
        let mut bc = model.dirichlet();
        for &n in model.mesh.node_set("v0").unwrap() {
            bc.fix(n, 1, 0.0); // equatorial plane
        }
        for &n in model.mesh.node_set("v1").unwrap() {
            bc.fix(n, 0, 0.0); // polar axis
        }
        let loads = Loads { faces: model.mesh.surfaces["u0"].iter().map(|f| (f.clone(), SurfaceLoad::Pressure(pl))).collect(), ..Loads::default() };
        let sol = model.solve_nonlinear(&loads, &bc, &NlOptions { control: Control::Arc { ds: 2e-3, lambda_max: 1.3, max_steps: 120 }, ..NlOptions::default() }).unwrap();
        let peak = sol.peak_lambda();
        eprintln!("sphere {nr}x4 Quad9: peak p / p_L = {peak:.5}");
        assert!(sol.state.max_ep() > 0.03);
        peaks.push(peak);
    }
    assert!(peaks[1] < 1.01 && peaks[1] > 0.99 && peaks[1] <= peaks[0] + 1e-9, "limit pressures {peaks:?}");
}
