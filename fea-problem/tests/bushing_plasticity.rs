//! Elastic-plastic materials with interference-fit bushings (#234). A concentric bushing in a round plate (plane strain) is the
//! thick-cylinder fit of `fea-core/tests/contact_plastic.rs` run through the problem layer: closed forms are the elastic Lame
//! pressure and Hill's elastic-perfectly-plastic relation (`p = Y' [ln(c/a) + (1 - c^2/b^2)/2]`, `Y' = 2 sigma_y / sqrt 3`,
//! fully plastic `p -> Y' ln(b/a)`). The solving tests are `#[ignore]`d (a plastic bushing solve is 70-215 s in release, against
//! about 10 s elastic): `cargo test --release -p fea-problem --test bushing_plasticity -- --ignored`.
use fea_problem::{solve, templates::templates, Analysis, Bushing, Geometry, MaterialSpec, MeshSpec, Problem, Shape, Solved, Support};

const E: f64 = 10.0e6;
const NU: f64 = 0.3;
const A: f64 = 1.0;
const B: f64 = 2.0;
const RI: f64 = 0.1;

fn material(yield_stress: Option<f64>) -> MaterialSpec {
    MaterialSpec { name: "test".into(), e: E, nu: NU, alpha: 0.0, yield_stress, density: 0.0, plastic: yield_stress.is_some(), hardening: 0.0 }
}

/// Round plate `A..B` (free outside) with a concentric bushing of bore `RI`, plane strain. The supports remove the rigid motions
/// only (uy at the two points on the x axis, ux at the two on the y axis: the radial solution has no reaction there).
fn fit(diametral: f64, plate_yield: Option<f64>, bushing_yield: Option<f64>) -> Problem {
    let mut p = templates().into_iter().find(|(n, _)| *n == "Lug with an eccentric bushing").unwrap().1;
    p.analysis = Analysis::PlaneStrain;
    p.thickness = 1.0;
    p.geometry = Geometry::Sketch { outer: Shape::Circle { cx: 0.0, cy: 0.0, r: B }, holes: vec![Shape::Circle { cx: 0.0, cy: 0.0, r: A }], depth: 1.0, layers: 1, regions: Vec::new() };
    p.material = material(plate_yield);
    p.mesh = MeshSpec { size: 0.3, hole_factor: 0.25, adapt_passes: 0, ..MeshSpec::default() };
    let pt = |x, y, ux, uy| Support::Point { x, y, z: 0.0, ux, uy, uz: None };
    p.supports = vec![pt(B, 0.0, None, Some(0.0)), pt(-B, 0.0, None, Some(0.0)), pt(0.0, B, Some(0.0), None), pt(0.0, -B, Some(0.0), None)];
    p.loads.clear();
    p.bushings = vec![Bushing { hole: 1, inner_diameter: 2.0 * RI, offset: [0.0, 0.0], interference: diametral, friction: 0.15, material: material(bushing_yield) }];
    p
}

/// Plane-strain Lame fit pressure of a hub `A..B` and a ring `RI..A` (both elastic).
fn lame(delta_radial: f64) -> f64 {
    let c = (1.0 + NU) * A / E * (((1.0 - 2.0 * NU) * A * A + B * B) / (B * B - A * A) + ((1.0 - 2.0 * NU) * A * A + RI * RI) / (A * A - RI * RI));
    delta_radial / c
}

fn hill(y: f64, c: f64) -> f64 {
    y * ((c / A).ln() + 0.5 * (1.0 - c * c / (B * B)))
}

/// The physical interface pressure from the force the interface transmits (`2 pi a` of unit thickness).
fn pressure(s: &Solved) -> f64 {
    s.summary.interfaces[0].normal_force / (2.0 * std::f64::consts::PI * A)
}

/// Largest radius of a node of the plate with plastic strain (the elastic-plastic boundary, to about an element).
fn plastic_radius(s: &Solved) -> f64 {
    let ep = s.plastic.as_ref().expect("a plastic result has its strain field");
    let bushing: std::collections::BTreeSet<usize> = s.model.mesh.blocks.last().unwrap().conn.iter().copied().collect();
    (0..ep.len()).filter(|n| !bushing.contains(n) && ep[*n] > 1e-12).map(|n| s.model.mesh.nodes[n][0].hypot(s.model.mesh.nodes[n][1])).fold(0.0, f64::max)
}

#[test]
#[ignore = "minutes in release: a plastic fit is 8 load steps of contact"]
fn a_yield_stress_far_above_any_stress_gives_the_elastic_fit_and_the_lame_pressure() {
    let delta = 0.004;
    let elastic = solve(&fit(delta, None, None), None).unwrap();
    let plastic = solve(&fit(delta, Some(1.0e12), Some(1.0e12)), None).unwrap();
    let (pe, pp, want) = (pressure(&elastic), pressure(&plastic), lame(0.5 * delta));
    eprintln!("elastic {pe:.2} plastic path {pp:.2} Lame {want:.2}");
    assert_eq!(plastic.summary.interfaces.len(), 1, "the plastic path keeps the interface results");
    assert!(plastic.plastic.as_ref().unwrap().iter().all(|e| *e == 0.0));
    // The elastic fit stops at the fit solver's contact tolerance (`Tuning::friction`, outer 2e-3: it reads 2.8e-4 under Lame
    // here); the plastic path takes 8 steps and lands on Lame to 5e-6. The bound is that tolerance, not the plastic path's.
    assert!((pp / pe - 1.0).abs() < 2e-3, "{pp} vs {pe}");
    assert!((pp / want - 1.0).abs() < 1e-4, "plastic path {pp} vs Lame {want}");
    assert!((pe / want - 1.0).abs() < 0.03, "elastic fit {pe} vs Lame {want}");
    for (a, b) in elastic.summary.interfaces.iter().zip(&plastic.summary.interfaces) {
        assert!((a.torque_capacity / b.torque_capacity - 1.0).abs() < 2e-3 && (a.open_arc_deg - b.open_arc_deg).abs() < 1e-9);
    }
}

#[test]
#[ignore = "minutes in release: a plastic fit is 8 load steps of contact"]
fn a_plate_yielding_past_the_elastic_limit_follows_hills_thick_cylinder_relation() {
    // The elastic Lame pressure of this fit is twice the one at first yield of the bore: Y' (1 - a^2 / b^2) / 2.
    let delta = 0.004;
    let sy = lame(0.5 * delta) / 2.0 / ((1.0 - A * A / (B * B)) / 2.0) * 3.0f64.sqrt() / 2.0;
    let y = 2.0 * sy / 3.0f64.sqrt();
    let s = solve(&fit(delta, Some(sy), None), None).unwrap();
    let (p, c, elastic) = (pressure(&s), plastic_radius(&s), lame(0.5 * delta));
    eprintln!("p {p:.1} (Lame {elastic:.1}), plastic radius {c:.3}, Hill p(c) {:.1}", hill(y, c));
    assert!(c > 1.15 * A && c < 0.95 * B, "the plastic zone reaches into the wall: {c}");
    assert!(p < 0.9 * elastic, "yielding relieves the pressure: {p} vs elastic {elastic}");
    // c is a nodal radius, known to about one element (the plate's size near the hole is size * hole_factor times a few).
    let h = 0.1;
    let (lo, hi) = (hill(y, c - h), hill(y, c + h));
    assert!(p > lo * 0.97 && p < hi * 1.03, "p {p} vs Hill {lo:.1}..{hi:.1} at c = {c}");
    assert!(s.summary.equilibrium_error < 1e-3, "equilibrium {:e}", s.summary.equilibrium_error);
}

#[test]
#[ignore = "minutes in release: a plastic fit is 8 load steps of contact"]
fn a_large_interference_approaches_the_fully_plastic_limit() {
    let sy = 1.0e4;
    let limit = 2.0 * sy / 3.0f64.sqrt() * (B / A).ln();
    let p = pressure(&solve(&fit(0.1, Some(sy), None), None).unwrap());
    eprintln!("limit {limit:.1}: p {p:.1}");
    assert!(p < 1.03 * limit && p > 0.93 * limit, "{p} vs {limit}");
}

#[test]
#[ignore = "minutes in release: a plastic fit is 8 load steps of contact"]
fn the_bushings_own_law_yields_the_bushing_and_not_the_plate() {
    // A soft bushing (yield far below the fit pressure) in an elastic plate: only the bushing's nodes have plastic strain.
    let s = solve(&fit(0.004, None, Some(2000.0)), None).unwrap();
    let ep = s.plastic.as_ref().unwrap();
    let bushing: std::collections::BTreeSet<usize> = s.model.mesh.blocks.last().unwrap().conn.iter().copied().collect();
    let max_in = bushing.iter().map(|n| ep[*n]).fold(0.0, f64::max);
    let max_out = (0..ep.len()).filter(|n| !bushing.contains(n)).map(|n| ep[n]).fold(0.0, f64::max);
    eprintln!("plastic strain: bushing {max_in:.3e}, plate {max_out:.3e}");
    assert!(max_in > 1e-4, "the bushing yields: {max_in}");
    assert_eq!(max_out, 0.0, "the elastic plate does not");
    // A soft bushing carries less pressure than an elastic one.
    assert!(pressure(&s) < pressure(&solve(&fit(0.004, None, None), None).unwrap()));
}

#[test]
fn a_plastic_bushing_needs_a_yield_stress_and_plastic_bushings_are_accepted() {
    let mut p = fit(0.004, None, Some(1.0e12));
    p.validate().unwrap();
    assert!(p.any_plastic());
    p.bushings[0].material.yield_stress = None;
    assert!(p.validate().unwrap_err().contains("bushing 1"), "{:?}", p.validate());
    p.bushings[0].material.yield_stress = Some(-1.0);
    assert!(p.validate().is_err());
}

#[test]
#[ignore = "about a minute in release"]
fn a_pin_load_runs_through_the_plastic_fit() {
    // The loaded stage continues from the plastic fit state: a pin load on the bore is reacted by the supports' symmetric pairs.
    let mut p = fit(0.004, Some(1.0e12), Some(1.0e12));
    p.loads = vec![fea_problem::Load::Bearing { hole: 1, fx: -500.0, fy: 0.0 }];
    p.supports = vec![Support::Point { x: B, y: 0.0, z: 0.0, ux: Some(0.0), uy: Some(0.0), uz: None }, Support::Point { x: -B, y: 0.0, z: 0.0, ux: Some(0.0), uy: Some(0.0), uz: None }];
    let s = solve(&p, None).unwrap();
    assert!((s.summary.applied[0].abs() - 500.0).abs() < 1e-6 * 500.0 + 1e-3, "applied {:?}", s.summary.applied);
    assert_eq!(s.summary.interfaces.len(), 1);
}
