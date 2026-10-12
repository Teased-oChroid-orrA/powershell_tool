//! Thermal studies: steady conduction against closed forms, and the one-way thermal stress.
use fea_problem::{solve, templates, Analysis, Geometry, HeatLoad, HeatSupport, Load, MeshSpec, Problem, Shape, Study, Support};

/// A bar 10 long and 1 high (plane stress, thickness 1), meshed finely along x.
fn bar() -> Problem {
    let mut p = templates::blank();
    p.history = None;
    p.geometry = Geometry::Sketch { outer: Shape::Rect { x0: 0.0, y0: 0.0, x1: 10.0, y1: 1.0 }, holes: vec![], depth: 1.0, layers: 1, regions: vec![] };
    p.analysis = Analysis::PlaneStress;
    p.thickness = 1.0;
    p.mesh = MeshSpec { size: 0.5, ..MeshSpec::default() };
    p.study = Study::Thermal;
    p.thermal.conductivity = 2.0;
    p
}

fn nodal_t(s: &fea_problem::Solved) -> Vec<([f64; 3], f64)> {
    let t = s.temperature.as_ref().expect("a thermal result has a temperature");
    s.model.mesh.nodes.iter().copied().zip(t.iter().copied()).collect()
}

#[test]
fn a_bar_between_two_temperatures_is_linear_and_conserves_heat() {
    let mut p = bar();
    p.thermal.supports = vec![HeatSupport::Temperature { edge: "left".into(), t: 100.0 }, HeatSupport::Temperature { edge: "right".into(), t: 20.0 }];
    let s = solve(&p, None).unwrap();
    for (x, t) in nodal_t(&s) {
        assert!((t - (100.0 - 8.0 * x[0])).abs() < 1e-8, "T({}) = {t}", x[0]);
    }
    let h = s.summary.heat.as_ref().unwrap();
    assert!((h.t_max.value - 100.0).abs() < 1e-9 && (h.t_min.value - 20.0).abs() < 1e-9);
    // What enters at one end leaves at the other: the net over the fixed temperatures is zero, and the balance holds.
    assert!(h.heat_in == 0.0 && h.heat_out.abs() < 1e-8 && h.balance_error < 1e-9, "{h:?}");
    assert!(s.fields()[0] == fea_problem::Field::Temperature && s.fields().len() == 1, "a thermal-only result offers just the temperature");
    assert!(s.u.iter().all(|u| *u == 0.0));
}

#[test]
fn flux_convection_and_generation_match_their_closed_forms() {
    // Flux q into the left end, convection h to an ambient at the right: steady heat balance fixes the right end
    // temperature, T(L) = ambient + q / h, and the profile is linear with slope q / k.
    let mut p = bar();
    p.thermal.supports.clear();
    p.thermal.loads = vec![HeatLoad::Flux { edge: "left".into(), q: 30.0 }, HeatLoad::Convection { edge: "right".into(), h: 6.0, ambient: 15.0 }];
    let s = solve(&p, None).unwrap();
    for (x, t) in nodal_t(&s) {
        let want = 15.0 + 30.0 / 6.0 + 30.0 / 2.0 * (10.0 - x[0]);
        assert!((t - want).abs() < 1e-7, "T({}) = {t}, want {want}", x[0]);
    }
    // 30 per unit area enters on the unit-high end, and the convection face returns h (T - ambient) = 6 x 5 = 30.
    let h = s.summary.heat.as_ref().unwrap();
    assert!((h.heat_in - 30.0).abs() < 1e-8 && (h.heat_out - 30.0).abs() < 1e-7 && h.balance_error < 1e-9, "{h:?}");
    // Uniform generation g in a bar held at 0 on both ends: T = g x (L - x) / (2 k), peak g L^2 / (8 k) = 6.25 g.
    let mut q = bar();
    q.thermal.source = 4.0;
    q.thermal.supports = vec![HeatSupport::Temperature { edge: "left".into(), t: 0.0 }, HeatSupport::Temperature { edge: "right".into(), t: 0.0 }];
    q.mesh.size = 0.25;
    let s = solve(&q, None).unwrap();
    let peak = s.summary.heat.as_ref().unwrap().t_max.value;
    assert!((peak - 4.0 * 100.0 / 16.0).abs() < 1e-3 * 25.0, "{peak}");
    let h = s.summary.heat.as_ref().unwrap();
    assert!((h.heat_in - 4.0 * 10.0).abs() < 1e-8 && h.balance_error < 1e-9, "{h:?}");
}

#[test]
fn a_constrained_bar_heated_uniformly_has_the_textbook_thermal_stress() {
    // Plane strain bar fixed in x at both ends and heated 40 degrees above the stress-free state: sigma_xx = -E alpha dT /
    // (1 - 2 nu) for the fully constrained block; for a bar free in y/z but held in x, sigma_xx = -E alpha dT (plane stress).
    let mut p = bar();
    p.study = Study::ThermalStress;
    p.thermal.reference = 20.0;
    p.thermal.supports = vec![HeatSupport::Temperature { edge: "bottom".into(), t: 60.0 }, HeatSupport::Temperature { edge: "top".into(), t: 60.0 }];
    p.supports = vec![Support::roller("left", 0), Support::roller("right", 0), Support::Point { x: 0.0, y: 0.0, z: 0.0, ux: None, uy: Some(0.0), uz: None }];
    let s = solve(&p, None).unwrap();
    let (e, alpha) = (p.material.e, p.material.alpha);
    let want = -e * alpha * 40.0;
    let mid: Vec<f64> = s.model.mesh.nodes.iter().enumerate().filter(|(_, x)| (x[0] - 5.0).abs() < 1.0).map(|(i, _)| s.nodal[i][0]).collect();
    let mean = mid.iter().sum::<f64>() / mid.len() as f64;
    assert!((mean - want).abs() < 1e-3 * want.abs(), "{mean} vs {want}");
    assert!(s.fields().contains(&fea_problem::Field::Temperature) && s.fields().contains(&fea_problem::Field::VonMises));
    assert!(s.summary.equilibrium_error < 1e-6);
}

#[test]
fn thermal_problems_say_what_is_missing_and_round_trip_through_json() {
    let mut p = bar();
    let e = p.validate().unwrap_err();
    assert!(e.contains("fixed temperature or a convection"), "{e}");
    p.thermal.supports = vec![HeatSupport::Temperature { edge: "left".into(), t: 5.0 }];
    p.validate().unwrap();
    p.thermal.conductivity = -1.0;
    assert!(p.validate().unwrap_err().contains("conductivity"));
    p.thermal.conductivity = 1.0;
    p.thermal.supports.push(HeatSupport::Temperature { edge: "nowhere".into(), t: 1.0 });
    assert!(solve(&p, None).unwrap_err().contains("nowhere"));
    // A structural problem saved before the thermal fields existed loads as structural; a thermal one round-trips.
    let old: Problem = Problem::from_json(&templates::templates().remove(0).1.to_json().unwrap()).unwrap();
    assert_eq!(old.study, Study::Structural);
    assert!(!old.to_json().unwrap().contains("thermal"), "default thermal data is not written");
    p.thermal.supports.pop();
    p.thermal.loads.push(HeatLoad::Point { x: 1.0, y: 0.5, z: 0.0, q: 3.0 });
    let back = Problem::from_json(&p.to_json().unwrap()).unwrap();
    assert_eq!(back, p);
    let _ = Load::Body { bx: 0.0, by: 0.0, bz: 0.0 };
}

#[test]
fn a_thermal_study_without_an_anchor_is_diagnosed_and_the_top_remedy_makes_it_solve() {
    use fea_problem::diagnose::{diagnose, FailureClass};
    let mut p = bar();
    p.thermal.loads = vec![HeatLoad::Flux { edge: "left".into(), q: 5.0 }];
    let found = diagnose(&p, Some("a steady conduction problem needs a fixed temperature or a convection boundary"));
    let d = found.iter().find(|d| d.class == FailureClass::NoThermalAnchor).expect("the missing anchor is found");
    let top = &d.remedies[0];
    assert!(top.label.contains("fixed temperature") && !top.label.contains("'left'"), "not the heated surface: {}", top.label);
    top.edit.apply(&mut p).unwrap();
    assert_eq!(p.thermal.supports.len(), 1);
    let s = solve(&p, None).unwrap();
    assert!(s.summary.heat.is_some());
    // A heat support on a surface the geometry does not have is reported with the nearest name.
    p.thermal.supports = vec![HeatSupport::Temperature { edge: "lefft".into(), t: 0.0 }];
    let found = diagnose(&p, None);
    let u = found.iter().find(|d| d.class == FailureClass::UnknownSurface).expect("unknown surface");
    assert!(u.remedies.iter().any(|r| r.label.contains("'left'")), "{:?}", u.remedies.iter().map(|r| &r.label).collect::<Vec<_>>());
    u.remedies[0].edit.apply(&mut p).unwrap();
    assert!(diagnose(&p, None).iter().all(|d| d.class != FailureClass::UnknownSurface));
    // A structural study ignores thermal data entirely.
    p.study = Study::Structural;
    assert!(diagnose(&p, None).iter().all(|d| d.class != FailureClass::NoThermalAnchor));
}
