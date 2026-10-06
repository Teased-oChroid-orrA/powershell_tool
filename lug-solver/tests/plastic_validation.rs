//! Elastic-perfectly-plastic validation against an exact solution.
//!
//! A rigid pin forced into a plane-stress ring (inner radius a, outer R, free
//! outside) (modelled as a symmetric half so the free body cannot spin) collapses at the fully plastic pressure of the von Mises ring:
//! with the yield surface sr^2 - sr st + st^2 = sf^2 (tensile hoop branch),
//! equilibrium r dsr/dr = st - sr and sr(R) = 0, integrated inward;
//! p_L = -sr(a) - but a yielding bore can never carry a radial stress beyond
//! the extent of the yield ellipse, (2/sqrt 3) sf, so a thick ring saturates
//! there (the tensile-hoop branch ends before it reaches the bore).

use lug_solver::contact::{ContactModel, ContactParams};
use lug_solver::fe::{Condensed, FarEnd};
use lug_solver::plastic::{PlaneMode, PlasticModel, PlasticOptions};
use lug_solver::*;

const E: f64 = 10.0e6;
const MAT: Material = Material { e_psi: E, nu: 0.33 };

/// Fully plastic collapse pressure of the plane-stress von Mises ring.
fn ring_collapse_pressure(a: f64, r_out: f64, sf: f64) -> f64 {
    let cap = 2.0 * sf / 3f64.sqrt();
    let hoop = |sr: f64| 0.5 * (sr + (4.0 * sf * sf - 3.0 * sr * sr).max(0.0).sqrt());
    // d sr / d ln r = st - sr, RK4 from ln R down to ln a.
    let n = 20_000;
    let h = (a / r_out).ln() / n as f64;
    let f = |sr: f64| hoop(sr) - sr;
    let mut sr = 0.0;
    for _ in 0..n {
        let k1 = f(sr);
        let k2 = f(sr + 0.5 * h * k1);
        let k3 = f(sr + 0.5 * h * k2);
        let k4 = f(sr + h * k3);
        sr += h * (k1 + 2.0 * k2 + 2.0 * k3 + k4) / 6.0;
        if -sr >= cap {
            return cap;
        }
    }
    -sr
}

fn disc(a: f64, r: f64) -> LugGeometry {
    LugGeometry { hole_dia: 2.0 * a, width: 2.0 * r, edge: r, length: r, thickness: 1.0, head_corner_radius: r, far_corner_radius: r }
}

#[test]
fn the_ring_collapse_ode_matches_two_known_limits() {
    // Thin ring: p_L -> sf * (R - a) / a ... (membrane limit sf t / r) for R -> a.
    let (a, sf) = (1.0, 50_000.0);
    let thin = ring_collapse_pressure(a, 1.01, sf);
    assert!((thin - sf * 0.01).abs() / (sf * 0.01) < 0.02, "{thin}");
    // Monotone in thickness.
    assert!(ring_collapse_pressure(a, 2.0, sf) > ring_collapse_pressure(a, 1.5, sf));
}

/// Drive the pin in until the ring is fully plastic and return the converged mean pressure.
fn ring_collapse_by_fe(a: f64, r: f64, sf: f64, deltas: &[f64]) -> f64 {
    let cond = Condensed::build(&disc(a, r), MeshSpec { elements_around: 48, ..Default::default() }, MAT, true, FarEnd::Soft).unwrap();
    let pm = PlasticModel::new(&cond, PlaneMode::Stress, E, 0.33).unwrap();
    let mut last = 0.0;
    for &delta in deltas {
        let mut cm = ContactModel::new(&cond, a + delta, ContactParams::default(), [-1.0, 0.0]);
        cm.free_perp = false;
        let mut st = cm.fresh_state();
        let mut ps = pm.fresh_state();
        let res = pm.equilibrate(&cm, &mut st, &mut ps, 0.0, PlasticOptions::new(sf)).unwrap();
        let mean: f64 = res_points_mean(&st);
        eprintln!("RING a {a} R {r} delta {delta} p {mean:.0} iters {} plastic {:.2}", res.iterations, res.plastic_fraction);
        assert!(mean >= last * 0.999, "pressure must not fall as the pin goes in");
        last = mean;
    }
    last
}

fn res_points_mean(st: &lug_solver::contact::ContactState) -> f64 {
    st.lambda.iter().sum::<f64>() / st.lambda.len() as f64
}

#[test]
fn a_thick_ring_saturates_at_the_yield_ellipse_radial_stress() {
    let (a, r, sf) = (0.25, 1.0, 60_000.0);
    let exact = ring_collapse_pressure(a, r, sf);
    assert!((exact - 2.0 * sf / 3f64.sqrt()).abs() < 1.0, "this ring is thick enough to hit the cap");
    let p = ring_collapse_by_fe(a, r, sf, &[0.002, 0.005, 0.01, 0.02]);
    assert!((p - exact).abs() / exact < 0.03, "collapse pressure {p} vs exact {exact}");
}

#[test]
fn a_thin_ring_follows_the_full_plastic_ode() {
    let (a, r, sf) = (0.25, 0.35, 60_000.0);
    let exact = ring_collapse_pressure(a, r, sf);
    assert!(exact < 2.0 * sf / 3f64.sqrt() * 0.7, "the ODE governs here: {exact}");
    let p = ring_collapse_by_fe(a, r, sf, &[0.0005, 0.001, 0.002, 0.005]);
    assert!((p - exact).abs() / exact < 0.04, "collapse pressure {p} vs exact {exact}");
}

fn lug() -> LugGeometry {
    LugGeometry::round_head(0.5, 1.5, 0.25, 3.75)
}

#[test]
fn lug_limit_load_scales_with_the_flow_stress_and_plateaus() {
    let m = LugModel::build(&lug(), MAT, MeshSpec::default(), true).unwrap();
    let pin = PinSpec::new(0.499, 0.0);
    let case = LoadCase { load_lbf: 1000.0, angle_deg: 0.0 };
    let a = m.limit_load(pin, case, 60_000.0).unwrap();
    let b = m.limit_load(pin, case, 120_000.0).unwrap();
    eprintln!("LUG P_L(60 ksi) {:.0} lbf plateau {} steps {} iters {} {:.0} ms; P_L(120) {:.0}", a.limit_load_lbf, a.plateau, a.curve.len(), a.plastic_iterations, a.elapsed_ms, b.limit_load_lbf);
    assert!(a.plateau && b.plateau, "{:?} {:?}", a.note, b.note);
    let ratio = b.limit_load_lbf / a.limit_load_lbf;
    assert!((ratio - 2.0).abs() < 0.06, "elastic-perfectly-plastic limit load is proportional to the flow stress: {ratio}");
}

/// Plane-stress elastic-perfectly-plastic cannot carry a bearing stress beyond the extent of
/// its yield ellipse, (2/sqrt 3) sigma_f: the collapse load is bore crush, whatever the head
/// length, and it masks net-section and shear-out. This is why ultimate capacity uses plane
/// strain (`validation_naca_tn1503.rs`).
#[test]
fn lug_limit_load_plane_stress_is_bearing_crush_limited() {
    let sf = 60_000.0;
    let pin = PinSpec::new(0.499, 0.0);
    let case = LoadCase { load_lbf: 1000.0, angle_deg: 0.0 };
    let mut v = Vec::new();
    for edge in [1.5, 3.0] {
        let g = LugGeometry { hole_dia: 0.5, width: 1.5, edge, length: 3.75, thickness: 0.25, head_corner_radius: 0.75, far_corner_radius: 0.0 };
        let l = LugModel::build(&g, MAT, MeshSpec::default(), true).unwrap().limit_load(pin, case, sf).unwrap();
        let bearing_stress = l.limit_load_lbf / (0.5 * 0.25);
        assert!(bearing_stress < 1.25 * 2.0 / 3f64.sqrt() * sf, "bearing stress {bearing_stress} cannot exceed the yield-ellipse cap");
        assert!(bearing_stress > 0.8 * sf);
        v.push(l.limit_load_lbf);
    }
    assert!((v[0] - v[1]).abs() / v[1] < 0.03, "capacity does not depend on the head length when bore crush governs: {v:?}");
}

#[test]
fn plane_strain_lets_the_bearing_zone_carry_more_than_plane_stress() {
    let sf = 60_000.0;
    let g = lug();
    let pin = PinSpec::new(0.499, 0.0);
    let case = LoadCase { load_lbf: 1000.0, angle_deg: 0.0 };
    let ps = LugModel::build(&g, MAT, MeshSpec::default(), true).unwrap().limit_load(pin, case, sf).unwrap();
    let pe = LugModel::build_with(&g, MAT, MeshSpec::default(), true, PlaneMode::Strain).unwrap().limit_load(pin, case, sf).unwrap();
    assert!(pe.limit_load_lbf > 1.05 * ps.limit_load_lbf, "plane strain {} vs plane stress {}", pe.limit_load_lbf, ps.limit_load_lbf);
    assert!(pe.plateau);
}

#[test]
fn the_limit_load_exceeds_the_first_yield_load_and_the_curve_is_monotone_until_the_plateau() {
    let m = LugModel::build(&lug(), MAT, MeshSpec::default(), true).unwrap();
    let pin = PinSpec::new(0.499, 0.0);
    let sf = 60_000.0;
    // First yield from the elastic solve: load at which the peak von Mises reaches sf (linear scaling).
    let probe = m.solve(pin, LoadCase { load_lbf: 2000.0, angle_deg: 0.0 }).unwrap();
    let p_yield = 2000.0 * sf / probe.peak_von_mises;
    let l = m.limit_load(pin, LoadCase { load_lbf: 1000.0, angle_deg: 0.0 }, sf).unwrap();
    assert!(l.limit_load_lbf > p_yield, "limit {} must exceed first yield {p_yield}", l.limit_load_lbf);
    for w in l.curve.windows(2).take(l.curve.len().saturating_sub(3)) {
        assert!(w[1].load_lbf >= w[0].load_lbf * 0.995, "load must not fall before the plateau: {w:?}");
    }
    assert!(l.curve.last().unwrap().plastic_fraction > l.curve[0].plastic_fraction);
}

#[test]
fn the_limit_load_is_mesh_converged() {
    let pin = PinSpec::new(0.499, 0.0);
    let case = LoadCase { load_lbf: 1000.0, angle_deg: 0.0 };
    let mut v = Vec::new();
    for n in [48usize, 72, 120] {
        let m = LugModel::build(&lug(), MAT, MeshSpec { elements_around: n, ..Default::default() }, true).unwrap();
        let l = m.limit_load(pin, case, 60_000.0).unwrap();
        eprintln!("LUGMESH n={n} P_L {:.0} plateau {} ms {:.0}", l.limit_load_lbf, l.plateau, l.elapsed_ms);
        v.push(l.limit_load_lbf);
    }
    assert!((v[1] - v[2]).abs() / v[2] < 0.04, "{v:?}");
}


#[test]
fn hardening_reduces_to_perfect_plasticity_and_raises_capacity() {
    let m = LugModel::build_with(&lug(), MAT, MeshSpec::default(), true, PlaneMode::Strain).unwrap();
    let pin = PinSpec::new(0.499, 0.0);
    let case = LoadCase { load_lbf: 1000.0, angle_deg: 0.0 };
    let epp = m.limit_load(pin, case, 60_000.0).unwrap();
    // Vanishing hardening slope: same collapse as the perfectly plastic run.
    let flat = Hardening::linear(60_000.0, 1.0, 0.2).unwrap();
    let h0 = m.limit_load_with(pin, case, 60_000.0, LimitOptions { hardening: Some(flat), strain_limit: Some(0.05), ..LimitOptions::default() }).unwrap();
    eprintln!("HARD epp {:.0} flat-hardening {:.0} strain_limited {}", epp.limit_load_lbf, h0.limit_load_lbf, h0.strain_limited);
    assert!((h0.limit_load_lbf / epp.limit_load_lbf - 1.0).abs() < 0.05, "{} vs {}", h0.limit_load_lbf, epp.limit_load_lbf);
    // A real curve starting at the same yield carries more by the ductility limit, and a larger
    // strain limit carries more still.
    let ro = Hardening::ramberg_osgood(60_000.0, 75_000.0, 0.08).unwrap();
    let cap = |limit: f64| m.limit_load_with(pin, case, 60_000.0, LimitOptions { hardening: Some(ro), strain_limit: Some(limit), ..LimitOptions::default() }).unwrap();
    let (lo, hi) = (cap(0.01), cap(0.04));
    eprintln!("HARD RO strain 1% {:.0} 4% {:.0} limited {} {}", lo.limit_load_lbf, hi.limit_load_lbf, lo.strain_limited, hi.strain_limited);
    assert!(lo.strain_limited && hi.strain_limited);
    assert!(hi.limit_load_lbf > lo.limit_load_lbf * 1.02 && hi.limit_load_lbf > epp.limit_load_lbf, "{} {} {}", lo.limit_load_lbf, hi.limit_load_lbf, epp.limit_load_lbf);
    assert!(hi.limit_load_lbf < 1.6 * epp.limit_load_lbf, "capacity must stay bounded by Ftu/Fty-ish: {}", hi.limit_load_lbf);
}

#[test]
fn hardening_in_plane_stress_is_refused() {
    let m = LugModel::build(&lug(), MAT, MeshSpec::default(), true).unwrap();
    let ro = Hardening::ramberg_osgood(60_000.0, 75_000.0, 0.08).unwrap();
    let r = m.limit_load_with(PinSpec::new(0.499, 0.0), LoadCase { load_lbf: 1000.0, angle_deg: 0.0 }, 60_000.0, LimitOptions { hardening: Some(ro), ..LimitOptions::default() });
    assert!(r.is_err());
}

/// Volumetric-locking patch test (upgrade plan item 6). Plastic flow is incompressible, so a Q9
/// mesh that locks over-predicts the collapse of a plane-strain ring. The fully plastic
/// pressure of a von Mises plane-strain ring is exact: `p_L = (2 / sqrt 3) sf ln(R / a)`.
#[test]
fn a_plane_strain_ring_collapses_at_the_exact_incompressible_pressure_so_the_mesh_does_not_lock() {
    let (a, r, sf): (f64, f64, f64) = (0.25, 0.35, 60_000.0);
    let exact = 2.0 / 3f64.sqrt() * sf * (r / a).ln();
    let (ee, en) = PlaneMode::Strain.effective(E, 0.33);
    let cond = Condensed::build(&disc(a, r), MeshSpec { elements_around: 48, ..Default::default() }, Material { e_psi: ee, nu: en }, true, FarEnd::Soft).unwrap();
    let pm = PlasticModel::new(&cond, PlaneMode::Strain, E, 0.33).unwrap();
    let mut last = 0.0;
    for delta in [0.0005, 0.001, 0.002, 0.005] {
        let mut cm = ContactModel::new(&cond, a + delta, ContactParams::default(), [-1.0, 0.0]);
        cm.free_perp = false;
        let (mut st, mut ps) = (cm.fresh_state(), pm.fresh_state());
        pm.equilibrate(&cm, &mut st, &mut ps, 0.0, PlasticOptions::new(sf)).unwrap();
        last = res_points_mean(&st);
        eprintln!("RING-PE delta {delta} p {last:.0} exact {exact:.0} ({:+.1} %)", 100.0 * (last / exact - 1.0));
    }
    assert!((last / exact - 1.0).abs() < 0.04, "plane-strain ring collapse {last} vs exact {exact}");
}
