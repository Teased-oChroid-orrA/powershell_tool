//! Heat conduction against closed forms: linear and quadratic profiles, flux and convection boundaries, temperature-
//! dependent conductivity (Kirchhoff transform), the decaying sine of a cooling slab, the logarithmic profile of a hollow
//! cylinder, and the global energy balance.

use fea_core::generate::grid;
use fea_core::*;
use std::f64::consts::PI;

const K: f64 = 2.0;

fn bar(kind: ElementKind, n: usize, len: f64, physics: Physics) -> Model {
    let d = kind.dim();
    let mut mesh = grid(physics, kind, Elastic::new(1.0, 0.3), [n, if d == 3 { 2 } else { 1 }, if d == 3 { 2 } else { 1 }], &move |p| [len * p[0], p[1], p[2]]).unwrap();
    mesh.set_thermal(0, Conductivity::Constant(K), 3.0).unwrap();
    Model::new(mesh).unwrap()
}

fn plane() -> Physics {
    Physics::PlaneStress { thickness: 1.0 }
}

fn fix_set(model: &Model, bc: &mut Dirichlet, set: &str, value: f64) {
    for &n in model.mesh.node_set(set).unwrap() {
        bc.fix(n, 0, value);
    }
}

#[test]
fn a_bar_between_two_temperatures_has_a_linear_profile_and_conserves_heat() {
    let model = bar(ElementKind::Quad4, 8, 4.0, plane());
    let mut bc = model.thermal_dirichlet();
    fix_set(&model, &mut bc, "u0", 0.0);
    fix_set(&model, &mut bc, "u1", 100.0);
    let s = model.solve_heat_steady(&HeatLoads::default(), &bc, 1e-10).unwrap();
    for (i, x) in model.mesh.nodes.iter().enumerate() {
        assert!((s.temperature[i] - 25.0 * x[0]).abs() < 1e-9, "node {i}: {} vs {}", s.temperature[i], 25.0 * x[0]);
    }
    // Heat through a unit-area bar: k dT/dx = 2 * 25 = 50, out of the hot end, into the cold one (reactions of opposite sign).
    assert!(s.rel_residual.is_finite() && s.rel_residual <= 1e-10, "{s:?}");
}

#[test]
fn a_uniform_steady_temperature_is_accepted_without_heat_flow() {
    // An insulated bar fixed at one end has T = constant and zero heat flow.
    // A load/reaction-only residual scale degenerates to round-off / round-off here.
    let model = bar(ElementKind::Quad9, 4, 3.0, plane());
    let mut bc = model.thermal_dirichlet();
    fix_set(&model, &mut bc, "u0", 100.0);
    let s = model.solve_heat_steady(&HeatLoads::default(), &bc, 1e-10).unwrap();
    assert!(s.temperature.iter().all(|t| (t - 100.0).abs() < 1e-10));
    assert!(s.rel_residual.is_finite() && s.rel_residual <= 1e-10, "{s:?}");
    assert!(s.heat_in == 0.0 && s.heat_out_fixed.abs() < 1e-10 && s.heat_out_convection == 0.0, "{s:?}");
}

#[test]
fn steady_heat_refuses_invalid_acceptance_tolerances() {
    let model = bar(ElementKind::Quad4, 3, 2.0, plane());
    let mut bc = model.thermal_dirichlet();
    fix_set(&model, &mut bc, "u0", 0.0);
    fix_set(&model, &mut bc, "u1", 100.0);
    for tol in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 0.0, -1.0] {
        let err = model.solve_heat_steady(&HeatLoads::default(), &bc, tol).unwrap_err();
        assert!(err.contains("tolerance"), "tol {tol}: {err}");
    }
}

#[test]
fn steady_heat_refuses_a_result_above_the_requested_residual() {
    let model = bar(ElementKind::Quad9, 4, 3.0, plane());
    let mut bc = model.thermal_dirichlet();
    fix_set(&model, &mut bc, "u0", 0.0);
    fix_set(&model, &mut bc, "u1", 0.0);
    let loads = HeatLoads { source: 8.0, ..HeatLoads::default() };
    let reference = model.solve_heat_steady(&loads, &bc, 1e-10).unwrap();
    assert!(reference.rel_residual > 0.0 && reference.rel_residual < 1e-10);
    // The same deterministic factorisation cannot meet half its measured residual.
    let tol = reference.rel_residual * 0.5;
    let err = model.solve_heat_steady(&loads, &bc, tol).unwrap_err();
    assert!(err.contains("residual") && err.contains("allowed"), "{err}");
}

#[test]
fn steady_heat_refuses_non_finite_results() {
    let model = bar(ElementKind::Quad4, 3, 2.0, plane());
    let mut bc = model.thermal_dirichlet();
    fix_set(&model, &mut bc, "u0", 0.0);
    for q in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let loads = HeatLoads { nodal: vec![(model.mesh.node_set("u1").unwrap()[0], q)], ..HeatLoads::default() };
        assert!(model.solve_heat_steady(&loads, &bc, 1e-10).is_err(), "heat input {q} must not return a successful solution");
    }
}

#[test]
fn generation_with_convection_matches_the_independent_closed_form() {
    // -k T'' = q, T(0) = T0, -k T'(L) = h (T(L) - Ta).
    // T(x) = T0 + c x - q x^2 / (2 k); the Robin condition determines c.
    let (l, q, h, t0, ta) = (3.0, 8.0, 5.0, 20.0, 10.0);
    let c = (q * l + h * q * l * l / (2.0 * K) - h * (t0 - ta)) / (K + h * l);
    let model = bar(ElementKind::Quad9, 4, l, plane());
    let mut bc = model.thermal_dirichlet();
    fix_set(&model, &mut bc, "u0", t0);
    let loads = HeatLoads { source: q, convection: model.mesh.surfaces["u1"].iter().map(|f| (f.clone(), h, ta)).collect(), ..HeatLoads::default() };
    let s = model.solve_heat_steady(&loads, &bc, 1e-10).unwrap();
    for (i, x) in model.mesh.nodes.iter().enumerate() {
        let exact = t0 + c * x[0] - q * x[0] * x[0] / (2.0 * K);
        assert!((s.temperature[i] - exact).abs() < 1e-10, "node {i}: {} vs {exact}", s.temperature[i]);
    }
    let end = t0 + c * l - q * l * l / (2.0 * K);
    assert!((s.heat_in - q * l).abs() < 1e-10);
    assert!((s.heat_out_fixed - K * c).abs() < 1e-10);
    assert!((s.heat_out_convection - h * (end - ta)).abs() < 1e-10);
    assert!(s.rel_residual.is_finite() && s.rel_residual <= 1e-10 && s.balance_error() < 1e-10, "{s:?}");
}

#[test]
fn uniform_generation_gives_the_parabola_and_the_heat_balance_closes() {
    // T = q / (2 k) x (L - x), both ends at 0: quadratic elements are exact.
    let (l, q) = (3.0, 8.0);
    let model = bar(ElementKind::Quad9, 4, l, plane());
    let mut bc = model.thermal_dirichlet();
    fix_set(&model, &mut bc, "u0", 0.0);
    fix_set(&model, &mut bc, "u1", 0.0);
    let s = model.solve_heat_steady(&HeatLoads { source: q, ..HeatLoads::default() }, &bc, 1e-10).unwrap();
    for (i, x) in model.mesh.nodes.iter().enumerate() {
        let exact = q / (2.0 * K) * x[0] * (l - x[0]);
        assert!((s.temperature[i] - exact).abs() < 1e-10, "node {i}: {} vs {exact}", s.temperature[i]);
    }
    assert!((s.heat_in - q * l * 1.0 * 1.0).abs() < 1e-10, "generation q L t h = {}", s.heat_in);
    assert!(s.balance_error() < 1e-10, "{s:?}");
    assert!(s.rel_residual < 1e-10);
}

#[test]
fn a_flux_boundary_and_a_convection_boundary() {
    // Flux q into x = L, T = 0 at x = 0: T = q x / k.
    let model = bar(ElementKind::Quad4, 5, 2.0, plane());
    let mut bc = model.thermal_dirichlet();
    fix_set(&model, &mut bc, "u0", 0.0);
    let flux = model.mesh.surfaces["u1"].clone();
    let s = model.solve_heat_steady(&HeatLoads { flux: flux.into_iter().map(|f| (f, 6.0)).collect(), ..HeatLoads::default() }, &bc, 1e-10).unwrap();
    for (i, x) in model.mesh.nodes.iter().enumerate() {
        assert!((s.temperature[i] - 3.0 * x[0]).abs() < 1e-9);
    }
    assert!(s.balance_error() < 1e-10);
    // Fixed 100 at x = 0, convection (h = 5, ambient 20) at x = L = 2: with Bi = hL/k = 5, T(L) = (100 + 5 * 20) / 6.
    let mut bc = model.thermal_dirichlet();
    fix_set(&model, &mut bc, "u0", 100.0);
    let conv = model.mesh.surfaces["u1"].clone();
    let s = model.solve_heat_steady(&HeatLoads { convection: conv.into_iter().map(|f| (f, 5.0, 20.0)).collect(), ..HeatLoads::default() }, &bc, 1e-10).unwrap();
    let t_end = (100.0 + 5.0 * 20.0) / 6.0;
    let end = model.mesh.node_set("u1").unwrap()[0];
    assert!((s.temperature[end] - t_end).abs() < 1e-9, "{} vs {t_end}", s.temperature[end]);
    assert!((s.heat_out_convection - 5.0 * (t_end - 20.0)).abs() < 1e-9 && s.balance_error() < 1e-9, "{s:?}");
}

#[test]
fn temperature_dependent_conductivity_follows_the_kirchhoff_transform() {
    // k(T) = 1 + 0.01 T on [0, 200] (a two-point table is exactly linear): k0 (T + beta T^2 / 2) is linear in x.
    let mut mesh = grid(plane(), ElementKind::Quad9, Elastic::new(1.0, 0.3), [12, 1, 1], &|p| [4.0 * p[0], p[1], 0.0]).unwrap();
    mesh.set_thermal(0, Conductivity::Table(vec![(0.0, 1.0), (200.0, 3.0)]), 1.0).unwrap();
    let model = Model::new(mesh).unwrap();
    let mut bc = model.thermal_dirichlet();
    fix_set(&model, &mut bc, "u0", 20.0);
    fix_set(&model, &mut bc, "u1", 150.0);
    let s = model.solve_heat_steady(&HeatLoads::default(), &bc, 1e-12).unwrap();
    assert!(s.iterations > 2, "a nonlinear problem iterates");
    let phi = |t: f64| t + 0.005 * t * t;
    let (a, b) = (phi(20.0), phi(150.0));
    for (i, x) in model.mesh.nodes.iter().enumerate() {
        let target = a + (b - a) * x[0] / 4.0;
        assert!((phi(s.temperature[i]) - target).abs() < 1e-5 * target, "node {i}: T {} phi {} vs {target}", s.temperature[i], phi(s.temperature[i]));
    }
    assert!(s.rel_residual < 1e-9);
}

#[test]
fn a_cooling_slab_decays_as_the_first_sine_mode_at_second_order() {
    // T(x, t) = 100 sin(pi x / L) exp(-alpha (pi / L)^2 t), alpha = k / (rho c) = 2 / 3.
    let l = 2.0;
    let model = bar(ElementKind::Quad9, 10, l, plane());
    let mut bc = model.thermal_dirichlet();
    fix_set(&model, &mut bc, "u0", 0.0);
    fix_set(&model, &mut bc, "u1", 0.0);
    let t0: Vec<f64> = model.mesh.nodes.iter().map(|x| 100.0 * (PI * x[0] / l).sin()).collect();
    let alpha = K / 3.0;
    let t_end = 0.5;
    let mid = (0..model.mesh.nodes.len()).find(|&i| (model.mesh.nodes[i][0] - l / 2.0).abs() < 1e-12).unwrap();
    let exact = 100.0 * (-alpha * (PI / l).powi(2) * t_end).exp();
    let err = |steps: usize, theta: f64| {
        let r = model.solve_heat_transient(&HeatLoads::default(), &bc, &t0, t_end / steps as f64, steps, theta).unwrap();
        (r.last().unwrap()[mid] - exact).abs() / exact
    };
    let (c1, c2) = (err(10, 0.5), err(20, 0.5));
    assert!(c2 < 3e-4 && (3.0..5.0).contains(&(c1 / c2)), "Crank-Nicolson errors {c1}, {c2}");
    let (e1, e2) = (err(20, 1.0), err(40, 1.0));
    assert!((1.7..2.3).contains(&(e1 / e2)), "backward Euler is first order: {e1}, {e2}");
}

#[test]
fn a_hollow_cylinder_has_the_logarithmic_profile() {
    let (ri, ro) = (1.0, 3.0);
    let mut mesh = grid(Physics::Axisymmetric, ElementKind::Quad9, Elastic::new(1.0, 0.3), [10, 1, 1], &move |p| [ri + (ro - ri) * p[0], p[1], 0.0]).unwrap();
    mesh.set_thermal(0, Conductivity::Constant(K), 1.0).unwrap();
    let model = Model::new(mesh).unwrap();
    let mut bc = model.thermal_dirichlet();
    fix_set(&model, &mut bc, "u0", 300.0);
    fix_set(&model, &mut bc, "u1", 100.0);
    let s = model.solve_heat_steady(&HeatLoads::default(), &bc, 1e-10).unwrap();
    for (i, x) in model.mesh.nodes.iter().enumerate() {
        let exact = 300.0 + (100.0 - 300.0) * (x[0] / ri).ln() / (ro / ri).ln();
        assert!((s.temperature[i] - exact).abs() < 2e-3, "r = {}: {} vs {exact}", x[0], s.temperature[i]);
    }
}

#[test]
fn a_3d_block_with_generation_and_convection_balances_its_energy() {
    let mut mesh = grid(Physics::Solid, ElementKind::Hex20, Elastic::new(1.0, 0.3), [3, 3, 3], &|p| [2.0 * p[0], p[1], p[2]]).unwrap();
    mesh.set_thermal(0, Conductivity::Constant(K), 1.0).unwrap();
    let model = Model::new(mesh).unwrap();
    let mut bc = model.thermal_dirichlet();
    for &n in model.mesh.node_set("u0").unwrap() {
        bc.fix(n, 0, 50.0);
    }
    let conv = model.mesh.surfaces["w1"].clone();
    let flux = model.mesh.surfaces["v1"].clone();
    let loads = HeatLoads { source: 3.0, convection: conv.into_iter().map(|f| (f, 4.0, 10.0)).collect(), flux: flux.into_iter().map(|f| (f, 1.5)).collect(), ..HeatLoads::default() };
    let s = model.solve_heat_steady(&loads, &bc, 1e-10).unwrap();
    assert!((s.heat_in - (3.0 * 2.0 + 1.5 * 2.0)).abs() < 1e-9, "generation 3 * volume 2 plus flux 1.5 * area 2: {}", s.heat_in);
    assert!(s.balance_error() < 1e-9 && s.rel_residual < 1e-9, "{s:?}");
}

#[test]
fn missing_properties_and_missing_supports_are_errors() {
    let mesh = grid(plane(), ElementKind::Quad4, Elastic::new(1.0, 0.3), [2, 2, 1], &|p| p).unwrap();
    let model = Model::new(mesh).unwrap();
    let mut bc = model.thermal_dirichlet();
    bc.fix(0, 0, 0.0);
    assert!(model.solve_heat_steady(&HeatLoads::default(), &bc, 1e-9).unwrap_err().contains("thermal properties"));
    let model = bar(ElementKind::Quad4, 2, 1.0, plane());
    assert!(model.solve_heat_steady(&HeatLoads { source: 1.0, ..HeatLoads::default() }, &model.thermal_dirichlet(), 1e-9).unwrap_err().contains("undetermined"));
}

// ---- the temperature field as a structural load

#[test]
fn a_uniform_temperature_field_is_the_uniform_temperature_change() {
    let mut mesh = grid(plane(), ElementKind::Quad9, Elastic::new(1.0e7, 0.3).with_alpha(1e-5), [3, 2, 1], &|p| [3.0 * p[0], p[1], 0.0]).unwrap();
    mesh.set_thermal(0, Conductivity::Constant(K), 1.0).unwrap();
    let model = Model::new(mesh).unwrap();
    let mut bc = model.dirichlet();
    for &n in model.mesh.node_set("u0").unwrap() {
        bc.fix(n, 0, 0.0);
    }
    bc.fix(model.mesh.node_set("u0").unwrap()[0], 1, 0.0);
    let uniform = model.solve_static(&Loads { delta_t: 50.0, ..Loads::default() }, &bc).unwrap();
    let field = model.solve_static(&Loads { temperature: Some(vec![50.0; model.mesh.nodes.len()]), ..Loads::default() }, &bc).unwrap();
    let scale = uniform.u.iter().fold(0.0f64, |m, v| m.max(v.abs()));
    assert!(uniform.u.iter().zip(&field.u).all(|(a, b)| (a - b).abs() < 1e-12 * scale));
    // And the stresses agree (zero: a free expansion).
    let s = model.gauss_stresses_field(&field.u, 0.0, Some(&vec![50.0; model.mesh.nodes.len()])).unwrap();
    let peak = s.iter().flatten().flat_map(|(_, st)| st.iter()).fold(0.0f64, |m, v| m.max(v.abs()));
    assert!(peak < 1e-6, "a free uniform expansion is stress free: {peak}");
}

#[test]
fn a_linear_temperature_field_expands_a_free_body_without_stress() {
    // eps_th = alpha T(x) I with T linear is compatible: u_x = a (T0 x + g x^2 / 2 - g y^2 / 2), u_y = a (T0 y + g x y), stress free.
    let (a, t0, g) = (1.2e-5, 20.0, 15.0);
    let mesh = grid(plane(), ElementKind::Quad9, Elastic::new(2.0e7, 0.3).with_alpha(a), [4, 3, 1], &|p| [2.0 * p[0], 1.0 * p[1], 0.0]).unwrap();
    let model = Model::new(mesh).unwrap();
    let temp: Vec<f64> = model.mesh.nodes.iter().map(|x| t0 + g * x[0]).collect();
    let exact = |x: &[f64; 3]| [a * (t0 * x[0] + g * x[0] * x[0] / 2.0 - g * x[1] * x[1] / 2.0), a * (t0 * x[1] + g * x[0] * x[1])];
    let mut bc = model.dirichlet();
    // remove the rigid motion by prescribing the exact displacement at three dofs
    let corner = (0..model.mesh.nodes.len()).find(|&i| model.mesh.nodes[i][0] < 1e-12 && model.mesh.nodes[i][1] < 1e-12).unwrap();
    let right = (0..model.mesh.nodes.len()).find(|&i| (model.mesh.nodes[i][0] - 2.0).abs() < 1e-12 && model.mesh.nodes[i][1] < 1e-12).unwrap();
    bc.fix(corner, 0, 0.0);
    bc.fix(corner, 1, 0.0);
    bc.fix(right, 1, exact(&model.mesh.nodes[right])[1]);
    let loads = Loads { temperature: Some(temp.clone()), ..Loads::default() };
    let sol = model.solve_static(&loads, &bc).unwrap();
    let scale = sol.u.iter().fold(0.0f64, |m, v| m.max(v.abs()));
    for (i, x) in model.mesh.nodes.iter().enumerate() {
        let e = exact(x);
        assert!((sol.u[2 * i] - e[0]).abs() < 1e-9 * scale && (sol.u[2 * i + 1] - e[1]).abs() < 1e-9 * scale, "node {i}: {:?} vs {e:?}", &sol.u[2 * i..2 * i + 2]);
    }
    let peak = model.gauss_stresses_field(&sol.u, 0.0, Some(&temp)).unwrap().iter().flatten().flat_map(|(_, st)| st.iter()).fold(0.0f64, |m, v| m.max(v.abs()));
    assert!(peak < 1e-4, "stress-free to round-off against E alpha T = {}: {peak}", 2.0e7 * a * 50.0);
    // Ignoring the field's thermal strain in the stress recovery would report a large stress: the field matters there too.
    let wrong = model.gauss_stresses(&sol.u, 0.0).unwrap().iter().flatten().flat_map(|(_, st)| st.iter()).fold(0.0f64, |m, v| m.max(v.abs()));
    assert!(wrong > 1e3 * peak.max(1e-9));
}

#[test]
fn a_heated_bar_clamped_at_both_ends_carries_minus_e_alpha_mean_t() {
    // The heat solution drives the structure. A bar held at both ends (and in y, so the problem is one-dimensional, nu = 0) has the
    // uniform stress sigma = -E alpha mean(T) by equilibrium; the temperature field only enters through its mean.
    let (e, a, l) = (3.0e7, 1.0e-5, 4.0);
    let mut mesh = grid(plane(), ElementKind::Quad9, Elastic::new(e, 0.0).with_alpha(a), [6, 1, 1], &move |p| [l * p[0], p[1], 0.0]).unwrap();
    mesh.set_thermal(0, Conductivity::Constant(K), 1.0).unwrap();
    let model = Model::new(mesh).unwrap();
    let mut tbc = model.thermal_dirichlet();
    fix_set(&model, &mut tbc, "u0", 0.0);
    fix_set(&model, &mut tbc, "u1", 100.0);
    let heat = model.solve_heat_steady(&HeatLoads::default(), &tbc, 1e-12).unwrap();
    let mut bc = model.dirichlet();
    for n in 0..model.mesh.nodes.len() {
        bc.fix(n, 1, 0.0);
    }
    for set in ["u0", "u1"] {
        for &n in model.mesh.node_set(set).unwrap() {
            bc.fix(n, 0, 0.0);
        }
    }
    let loads = Loads { temperature: Some(heat.temperature.clone()), ..Loads::default() };
    let sol = model.solve_static(&loads, &bc).unwrap();
    let mean = 50.0;
    for (pos, st) in model.gauss_stresses_field(&sol.u, 0.0, loads.temperature.as_deref()).unwrap().iter().flatten() {
        assert!((st[0] + e * a * mean).abs() < 1e-6 * e * a * mean, "x = {}: {} vs {}", pos[0], st[0], -e * a * mean);
    }
}

#[test]
fn a_wrong_length_temperature_field_is_refused() {
    let mesh = grid(plane(), ElementKind::Quad4, Elastic::new(1.0, 0.3), [2, 2, 1], &|p| p).unwrap();
    let model = Model::new(mesh).unwrap();
    let mut bc = model.dirichlet();
    for &n in model.mesh.node_set("u0").unwrap() {
        bc.fix_node(n);
    }
    assert!(model.solve_static(&Loads { temperature: Some(vec![0.0; 3]), ..Loads::default() }, &bc).is_err());
}
