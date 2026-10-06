//! The bushing layer against the exact two-cylinder Lame solution of a press fit.
//!
//! A bushing (inner radius r_i, outer radius a, E_b, nu_b) pressed with a radial
//! overlap d into a disc (hole radius a, outer radius R, E_l, nu_l), plane stress,
//! free inner and outer surfaces: compatibility of the radial displacements at the
//! interface gives
//!   d = p a { [(R^2 + a^2)/(R^2 - a^2) + nu_l] / E_l + [(a^2 + r_i^2)/(a^2 - r_i^2) - nu_b] / E_b }.

use lug_solver::contact::{ContactModel, ContactParams, InterfaceSpec};
use lug_solver::fe::{Condensed, FarEnd};
use lug_solver::*;

const LUG: Material = Material { e_psi: 10.0e6, nu: 0.33 };
const BUSH: Material = Material { e_psi: 29.0e6, nu: 0.30 };

fn disc(a: f64, r: f64) -> LugGeometry {
    LugGeometry { hole_dia: 2.0 * a, width: 2.0 * r, edge: r, length: r, thickness: 1.0, head_corner_radius: r, far_corner_radius: r }
}

fn lame_fit_pressure(a: f64, r_in: f64, r_out: f64, delta: f64) -> f64 {
    let lug = ((r_out * r_out + a * a) / (r_out * r_out - a * a) + LUG.nu) / LUG.e_psi;
    let bush = ((a * a + r_in * r_in) / (a * a - r_in * r_in) - BUSH.nu) / BUSH.e_psi;
    delta / (a * (lug + bush))
}

fn build(a: f64, r: f64, r_in: f64, layers: usize, n: usize) -> Condensed {
    Condensed::build_with(&disc(a, r), MeshSpec { elements_around: n, ..Default::default() }, LUG, true, FarEnd::Soft, Some((BushingMesh { inner_radius: r_in, layers }, BUSH))).unwrap()
}

#[test]
fn the_press_fit_pressure_matches_the_two_cylinder_lame_solution() {
    let (a, r, r_in, delta) = (0.25, 1.0, 0.15, 0.0005);
    let exact = lame_fit_pressure(a, r_in, r, delta);
    for (layers, n) in [(2usize, 48usize), (3, 72)] {
        let cond = build(a, r, r_in, layers, n);
        let mut cm = ContactModel::with_interface(&cond, 0.05, ContactParams::default(), [-1.0, 0.0], Some(InterfaceSpec { interference: delta, friction: 0.0 }));
        cm.free_perp = false;
        let mut st = cm.fresh_state();
        let res = cm.advance(&mut st, 0.0).unwrap();
        assert!(res.points.iter().all(|p| p.pressure == 0.0), "the pin does not touch the bushing");
        let n_if = res.iface.len() as f64;
        let mean = res.iface.iter().map(|p| p.pressure).sum::<f64>() / n_if;
        let (lo, hi) = res.iface.iter().fold((f64::MAX, 0.0f64), |(l, h), p| (l.min(p.pressure), h.max(p.pressure)));
        eprintln!("BUSH layers {layers} n {n}: mean {mean:.1} exact {exact:.1} ({:+.3} %) spread {:.2} %", 100.0 * (mean / exact - 1.0), 100.0 * (hi - lo) / exact);
        assert!((mean - exact).abs() / exact < 0.01, "mean {mean} vs Lame {exact}");
        assert!((hi - lo) / exact < 0.03, "the fit pressure is uniform: {lo}..{hi}");
        assert!(res.iface.iter().all(|p| p.shear == 0.0));
    }
}

#[test]
fn zero_interference_carries_no_fit_pressure_and_a_clearance_separates() {
    let (a, r, r_in) = (0.25, 1.0, 0.15);
    let cond = build(a, r, r_in, 2, 48);
    for delta in [0.0, -0.0003] {
        let mut cm = ContactModel::with_interface(&cond, 0.05, ContactParams::default(), [-1.0, 0.0], Some(InterfaceSpec { interference: delta, friction: 0.0 }));
        cm.free_perp = false;
        let mut st = cm.fresh_state();
        let res = cm.advance(&mut st, 0.0).unwrap();
        let peak = res.iface.iter().fold(0.0f64, |m, p| m.max(p.pressure));
        assert!(peak < 1e-3 * lame_fit_pressure(a, r_in, r, 0.0005), "delta {delta}: pressure {peak}");
    }
}

#[test]
fn a_stiffer_bushing_carries_more_of_the_fit() {
    // Same overlap, stiffer bushing: more fit pressure (exact formula monotone in E_b).
    let (a, r, r_in, delta) = (0.25, 1.0, 0.15, 0.0005);
    let soft = Material { e_psi: 10.0e6, nu: 0.30 };
    let run = |bush: Material| {
        let cond = Condensed::build_with(&disc(a, r), MeshSpec { elements_around: 48, ..Default::default() }, LUG, true, FarEnd::Soft, Some((BushingMesh { inner_radius: r_in, layers: 2 }, bush))).unwrap();
        let mut cm = ContactModel::with_interface(&cond, 0.05, ContactParams::default(), [-1.0, 0.0], Some(InterfaceSpec { interference: delta, friction: 0.0 }));
        cm.free_perp = false;
        let mut st = cm.fresh_state();
        let res = cm.advance(&mut st, 0.0).unwrap();
        res.iface.iter().map(|p| p.pressure).sum::<f64>() / res.iface.len() as f64
    };
    assert!(run(BUSH) > run(soft));
}

fn lug() -> LugGeometry {
    LugGeometry::round_head(0.5, 1.5, 0.25, 3.75)
}

fn bushing(interference_dia: f64, friction: f64) -> BushingSpec {
    BushingSpec { inner_dia: 0.375, material: BUSH, interference_dia, friction }
}

fn pin() -> PinSpec {
    PinSpec::new(0.374, 0.1)
}

#[test]
fn a_bushed_lug_solves_and_reports_the_fit_the_bushing_and_the_lug_separately() {
    let m = LugModel::build_bushed(&lug(), LUG, MeshSpec::default(), true, PlaneMode::Stress, Some(bushing(0.001, 0.2))).unwrap();
    let s = m.solve(pin(), LoadCase { load_lbf: 3000.0, angle_deg: 0.0 }).unwrap();
    let b = s.bushing.as_ref().expect("bushing results");
    eprintln!("BL fit {:.0} mean {:.0} peak {:.0} sep {:.2} bushing hoop {:.0} vm {:.0} lug hoop {:.0} patch {:.0} nodes {}", b.fit_pressure_unloaded, b.interface_mean_pressure, b.interface_peak_pressure, b.separated_fraction, b.peak_hoop, b.peak_von_mises, s.peak_hoop, s.contact_arc_deg, s.mesh.nodes);
    assert!(b.fit_pressure_unloaded > 1000.0, "an interference fit presses the bushing");
    assert!(b.peak_von_mises > 0.0 && s.peak_von_mises > 0.0 && s.peak_hoop > 0.0);
    assert_eq!(s.lug_bore.len(), s.bore.len());
    // The pin bears on the bushing's inner diameter.
    assert!((b.bearing_stress - 3000.0 / (0.375 * 0.25)).abs() < 1e-6);
    // Equilibrium of the pin contact (half model: double the half force).
    let t = 0.25;
    let mut fx = 0.0;
    for p in &s.contact {
        let rel = [p.x[0] - s.pin_centre[0], p.x[1] - s.pin_centre[1]];
        let d = rel[0].hypot(rel[1]);
        let n = [rel[0] / d, rel[1] / d];
        let tau = [-n[1], n[0]];
        fx += p.w * (p.pressure * n[0] + p.shear * tau[0]);
    }
    assert!((2.0 * fx * t + 3000.0).abs() < 3000.0 * 2e-4, "pin contact carries the load: {}", 2.0 * fx * t);
}

#[test]
fn half_and_full_bushed_models_agree() {
    let case = LoadCase { load_lbf: 2500.0, angle_deg: 0.0 };
    let spec = Some(bushing(0.0008, 0.15));
    let half = LugModel::build_bushed(&lug(), LUG, MeshSpec::default(), true, PlaneMode::Stress, spec).unwrap().solve(pin(), case).unwrap();
    let full = LugModel::build_bushed(&lug(), LUG, MeshSpec::default(), false, PlaneMode::Stress, spec).unwrap().solve(pin(), case).unwrap();
    let rel = |a: f64, b: f64| (a - b).abs() / b.abs();
    let (bh, bf) = (half.bushing.as_ref().unwrap(), full.bushing.as_ref().unwrap());
    assert!(rel(half.peak_hoop, full.peak_hoop) < 3e-3, "{} vs {}", half.peak_hoop, full.peak_hoop);
    assert!(rel(bh.fit_pressure_unloaded, bf.fit_pressure_unloaded) < 3e-3);
    // A local nodal peak near the contact-arc end, with a path-dependent friction history: 1 %.
    assert!(rel(bh.peak_von_mises, bf.peak_von_mises) < 1e-2, "{} vs {}", bh.peak_von_mises, bf.peak_von_mises);
}

#[test]
fn more_interference_means_more_fit_pressure_and_a_light_fit_loses_contact_under_load() {
    let case = LoadCase { load_lbf: 6000.0, angle_deg: 0.0 };
    let solve = |interference: f64| LugModel::build_bushed(&lug(), LUG, MeshSpec::default(), true, PlaneMode::Stress, Some(bushing(interference, 0.2))).unwrap().solve(pin(), case).unwrap().bushing.unwrap();
    let light = solve(0.0002);
    let heavy = solve(0.002);
    assert!(heavy.fit_pressure_unloaded > 3.0 * light.fit_pressure_unloaded, "{} vs {}", heavy.fit_pressure_unloaded, light.fit_pressure_unloaded);
    assert!(light.separated_fraction > 0.02, "a light fit loses contact on the unloaded side under 6 kip: {}", light.separated_fraction);
    assert!(heavy.separated_fraction < light.separated_fraction, "a heavy fit keeps it");
}

#[test]
fn a_clearance_bushing_still_solves_and_carries_no_fit() {
    let s = LugModel::build_bushed(&lug(), LUG, MeshSpec::default(), true, PlaneMode::Stress, Some(bushing(-0.0004, 0.0))).unwrap().solve(pin(), LoadCase { load_lbf: 2000.0, angle_deg: 0.0 }).unwrap();
    let b = s.bushing.unwrap();
    assert!(b.fit_pressure_unloaded < 1.0, "{}", b.fit_pressure_unloaded);
    assert!(b.interface_peak_pressure > 0.0, "the loaded side still bears");
}

#[test]
fn the_plastic_collapse_runs_with_a_bushing_which_stays_elastic() {
    let m = LugModel::build_bushed(&lug(), LUG, MeshSpec { elements_around: 48, ..Default::default() }, true, PlaneMode::Strain, Some(bushing(0.001, 0.2))).unwrap();
    let l = m.limit_load(pin(), LoadCase { load_lbf: 1000.0, angle_deg: 0.0 }, 60_000.0).unwrap();
    eprintln!("BL collapse {:.0} lbf plateau {} steps {} {:.0} ms", l.limit_load_lbf, l.plateau, l.curve.len(), l.elapsed_ms);
    assert!(l.plateau && l.limit_load_lbf > 3000.0);
}

#[test]
fn invalid_bushings_are_errors() {
    let bad = |b: BushingSpec| LugModel::build_bushed(&lug(), LUG, MeshSpec::default(), true, PlaneMode::Stress, Some(b)).is_err();
    assert!(bad(BushingSpec { inner_dia: 0.5, ..bushing(0.0, 0.0) }), "inner diameter must be smaller than the hole");
    assert!(bad(BushingSpec { inner_dia: -0.1, ..bushing(0.0, 0.0) }));
    assert!(bad(BushingSpec { material: Material { e_psi: 0.0, nu: 0.3 }, ..bushing(0.0, 0.0) }));
    assert!(bad(BushingSpec { friction: 5.0, ..bushing(0.0, 0.0) }));
}


