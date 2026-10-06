//! The kernel path of every lug feature the toolbox offers, against closed forms and the
//! condensed solver: thermal fit, second order, bushing, sweeps.

use lug_solver::fea::FeaLug;
use lug_solver::*;

const AL: Material = Material { e_psi: 10.3e6, nu: 0.33 };
const STEEL: Material = Material { e_psi: 29e6, nu: 0.30 };
const AL_CTE: f64 = 12.8e-6;
const ST_CTE: f64 = 6.5e-6;

fn lug() -> LugGeometry {
    LugGeometry::round_head(0.5, 1.5, 0.25, 3.75)
}

fn bushing(interference_dia: f64) -> BushingSpec {
    BushingSpec { inner_dia: 0.375, material: STEEL, interference_dia, friction: 0.2 }
}

fn unloaded_fit(interference: f64, thermal: Thermal) -> f64 {
    let m = FeaLug::build_bushed(&lug(), AL, MeshSpec::default(), true, PlaneMode::Stress, Some(bushing(interference))).unwrap();
    m.fit_pressure(PinSpec { thermal, ..PinSpec::new(0.374, 0.0) }).unwrap()
}

#[test]
fn heating_a_steel_bushing_in_aluminium_relaxes_the_fit_linearly() {
    let thermal = |dt: f64| Thermal { delta_t: dt, alpha_lug: AL_CTE, alpha_bushing: ST_CTE, alpha_pin: ST_CTE };
    let p0 = unloaded_fit(0.001, Thermal::default());
    let (hot, cold) = (unloaded_fit(0.001, thermal(100.0)), unloaded_fit(0.001, thermal(-100.0)));
    eprintln!("KERNEL THERMAL fit pressure: cold {cold:.0} nominal {p0:.0} hot {hot:.0}");
    assert!(hot < p0 && p0 < cold);
    let d_eff = |dt: f64| 0.001 + dt * 0.5 * (ST_CTE - AL_CTE);
    for (dt, p) in [(100.0, hot), (-100.0, cold)] {
        let want = p0 * d_eff(dt) / 0.001;
        assert!((p / want - 1.0).abs() < 0.02, "dT {dt}: {p:.0} vs linear {want:.0}");
    }
    let lost = unloaded_fit(0.001, thermal(400.0));
    assert!(lost < 0.05 * p0, "fit pressure {lost} after 400 F");
}

#[test]
fn the_kernel_fit_agrees_with_the_condensed_fit() {
    let c = LugModel::build_bushed(&lug(), AL, MeshSpec::default(), true, PlaneMode::Stress, Some(bushing(0.001))).unwrap();
    let pin = PinSpec::new(0.374, 0.0);
    let cond = c.solve(pin, LoadCase { load_lbf: 0.0, angle_deg: 0.0 }).unwrap().bushing.unwrap().fit_pressure_unloaded;
    let ker = unloaded_fit(0.001, Thermal::default());
    eprintln!("fit pressure: kernel {ker:.0} vs condensed {cond:.0}");
    assert!((ker / cond - 1.0).abs() < 0.04, "{ker} vs {cond}");
}

// ---- second order (the beam-column amplification of tests/second_order.rs) ----
const SOFT: Material = Material { e_psi: 5000.0, nu: 0.33 };
const W: f64 = 1.0;
const L: f64 = 3.0;

fn amplification(angle_deg: f64, load: f64) -> f64 {
    let g = LugGeometry::round_head(0.3, W, 0.25, L);
    let mut fe = FeaLug::build(&g, SOFT, MeshSpec::default(), false, PlaneMode::Stress).unwrap();
    let pin = PinSpec::new(0.2999, 0.0);
    let case = LoadCase { load_lbf: load, angle_deg };
    let first = fe.analyze(pin, case).unwrap();
    fe.set_geometric_nonlinearity(true);
    let second = fe.analyze(pin, case).unwrap();
    second.pin_centre[1] / first.pin_centre[1]
}

#[test]
fn tension_stiffens_and_compression_softens_a_lug_like_a_beam_column() {
    let theta = 3.0f64.to_radians();
    let ei = SOFT.e_psi * W.powi(3) / 12.0;
    let n_axial = 0.64 * ei / (L * L);
    let load = n_axial * 0.25 / theta.cos();
    let u = L * (n_axial / ei).sqrt();
    let want_tension = 3.0 * (u - u.tanh()) / u.powi(3);
    let want_compression = 3.0 * (u.tan() - u) / u.powi(3);
    let (tension, compression) = (amplification(3.0, load), amplification(177.0, load));
    eprintln!("KERNEL P-DELTA u {u:.3}: tension {tension:.3} (beam {want_tension:.3}), compression {compression:.3} (beam {want_compression:.3})");
    assert!(tension < 1.0 && compression > 1.0);
    assert!((tension / want_tension - 1.0).abs() < 0.10, "{tension} vs {want_tension}");
    assert!((compression / want_compression - 1.0).abs() < 0.10, "{compression} vs {want_compression}");
}

// ---- load-direction robustness (tests/angle_sweep.rs on the kernel) ----

#[test]
fn the_kernel_solves_every_load_angle_symmetrically() {
    let fe = FeaLug::build(&lug(), AL, MeshSpec::default(), false, PlaneMode::Stress).unwrap();
    let pin = PinSpec::new(0.4995, 0.15);
    let angles: Vec<f64> = (-165..=180).step_by(15).map(|a| a as f64).collect();
    for load in [1000.0, 4000.0] {
        let t = std::time::Instant::now();
        let sweep = fe.analyze_sweep(pin, load, &angles);
        eprintln!("kernel angle sweep at {load} lbf: {:.1} s", t.elapsed().as_secs_f64());
        let hoop = |a: f64| -> f64 { sweep[angles.iter().position(|x| *x == a).unwrap()].as_ref().unwrap_or_else(|e| panic!("angle {a} load {load}: {e}")).peak_hoop };
        for &a in &angles {
            let h = hoop(a);
            assert!(h.is_finite() && h > 0.0, "angle {a}");
            if a > 0.0 && a < 180.0 {
                let n = hoop(-a);
                eprintln!("  +-{a}: {h:.0} vs {n:.0}");
                assert!((h / n - 1.0).abs() < 0.05, "load {load} angle +-{a}: {h} vs {n}");
            }
        }
    }
}

#[test]
fn the_kernel_collapse_sweep_and_envelope() {
    let fe = FeaLug::build(&lug(), AL, MeshSpec::default(), false, PlaneMode::Strain).unwrap();
    let t = std::time::Instant::now();
    let env = fe.collapse_sweep(PinSpec::new(0.4995, 0.15), 55_000.0, &[0.0, 45.0, -45.0, 90.0]);
    let caps: Vec<f64> = env.iter().map(|r| r.as_ref().unwrap().limit_load_lbf).collect();
    eprintln!("kernel collapse envelope {caps:?} in {:.1} s", t.elapsed().as_secs_f64());
    assert!(caps.iter().all(|c| *c > 1000.0), "{caps:?}");
    assert!((caps[1] / caps[2] - 1.0).abs() < 0.05, "+45 vs -45: {} {}", caps[1], caps[2]);
}

// ---- finite-strain collapse on the kernel against FiniteLug ----

#[test]
fn finite_strain_collapse_matches_finite_lug_axial_and_oblique() {
    let law = Hardening::true_curve(46_500.0, 65_400.0, 0.188, 1.5).unwrap();
    let mat = FsMaterial { e_psi: 10.4e6, nu: 0.33, law };
    let g = LugGeometry { hole_dia: 0.5, width: 2.0, edge: 1.0, length: 5.0, thickness: 0.25, head_corner_radius: 0.0, far_corner_radius: 0.0 };
    let spec = MeshSpec { elements_around: 32, ..Default::default() };
    for (angle, mu) in [(0.0, 0.0), (0.0, 0.15), (30.0, 0.0)] {
        let case = LoadCase { load_lbf: 1000.0, angle_deg: angle };
        let half = angle == 0.0;
        let fl = FiniteLug::build(&g, spec, mat, half).unwrap();
        let c = fl.collapse(case, FsOptions { strain_limit: Some(0.38), travel_cap_over_a: 1.5, ..FsOptions::new(0.4996, mu) }).unwrap();
        let fe = FeaLug::build(&g, Material { e_psi: 10.4e6, nu: 0.33 }, spec, half, PlaneMode::Strain).unwrap();
        let t = std::time::Instant::now();
        let k = fe.finite_collapse(PinSpec::new(0.4996, mu), case, law, Some(0.38), 1.5).unwrap();
        eprintln!("finite angle {angle} mu {mu}: kernel {:.0} lbf vs FiniteLug {:.0} lbf ({:+.2} %), {:.1} s vs {:.2} s, strain-limited {} vs {}", k.collapse_lbf, c.collapse_lbf, 100.0 * (k.collapse_lbf / c.collapse_lbf - 1.0), t.elapsed().as_secs_f64(), c.elapsed_ms / 1e3, k.strain_limited, c.strain_limited);
        assert!((k.collapse_lbf / c.collapse_lbf - 1.0).abs() < 0.04, "{} vs {}", k.collapse_lbf, c.collapse_lbf);
    }
}

// ---- a bushing whose interface has no friction is held by the weak grounding springs ----

#[test]
fn a_frictionless_bushing_interface_and_an_oblique_load_solve_on_the_kernel() {
    // The condensed solver does not support this (the bushing floats); the kernel grounds the bushing weakly.
    let b = BushingSpec { inner_dia: 0.375, material: STEEL, interference_dia: 0.001, friction: 0.0 };
    let fe = FeaLug::build_bushed(&lug(), AL, MeshSpec { elements_around: 36, ..Default::default() }, false, PlaneMode::Stress, Some(b)).unwrap();
    let s = fe.analyze(PinSpec::new(0.374, 0.0), LoadCase { load_lbf: 1200.0, angle_deg: 40.0 }).unwrap();
    let fit = s.bushing.as_ref().unwrap().fit_pressure_unloaded;
    assert!(fit > 3_000.0 && s.peak_hoop > 10_000.0 && s.peak_hoop.is_finite(), "fit {fit}, hoop {}", s.peak_hoop);
    assert!(s.verification.force_balance < 1e-2, "{:?}", s.verification);
}
