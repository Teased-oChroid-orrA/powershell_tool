//! Second-order (P-delta) analysis (upgrade plan item 7) against the beam-column amplification
//! of a slender cantilever: tip load `F` with an axial force `N` at the free end,
//! `delta / delta_1 = 3 (tan u - u) / u^3` in compression and `3 (u - tanh u) / u^3` in tension,
//! `u = L sqrt(N / EI)`.

use lug_solver::*;

// A very soft material makes the geometric stiffness comparable to the elastic one at stresses
// far inside the linear range (the ratio is what matters), and a lug of ordinary proportions keeps
// the star-shaped mesh valid.
const SOFT: Material = Material { e_psi: 5000.0, nu: 0.33 };
const AL: Material = Material { e_psi: 10.0e6, nu: 0.33 };
const W: f64 = 1.0;
const L: f64 = 3.0;

fn slender() -> LugGeometry {
    LugGeometry::round_head(0.3, W, 0.25, L)
}

fn amplification(angle_deg: f64, load: f64) -> (f64, f64) {
    let m = LugModel::build_with(&slender(), SOFT, MeshSpec::default(), false, PlaneMode::Stress).unwrap();
    let pin = PinSpec::new(0.2999, 0.0);
    let case = LoadCase { load_lbf: load, angle_deg };
    let first = m.solve(pin, case).unwrap();
    let second = m.solve_second_order(pin, case).unwrap();
    (second.pin_centre[1] / first.pin_centre[1], second.verification.energy_balance)
}

#[test]
fn tension_stiffens_and_compression_softens_a_lug_like_a_beam_column() {
    let theta = 3.0f64.to_radians();
    let ei = SOFT.e_psi * W.powi(3) / 12.0; // per unit thickness
    // Axial force giving u = L sqrt(N / EI) = 0.8.
    let n_axial = 0.64 * ei / (L * L);
    let load = n_axial * 0.25 / theta.cos();
    let u = L * (n_axial / ei).sqrt();
    let want_tension = 3.0 * (u - u.tanh()) / u.powi(3);
    let want_compression = 3.0 * (u.tan() - u) / u.powi(3);
    let (tension, e_t) = amplification(3.0, load);
    let (compression, e_c) = amplification(177.0, load);
    eprintln!("P-DELTA u {u:.3}: tension {tension:.3} (beam {want_tension:.3}), compression {compression:.3} (beam {want_compression:.3})");
    assert!(tension < 1.0 && compression > 1.0);
    assert!((tension / want_tension - 1.0).abs() < 0.08, "{tension} vs {want_tension}");
    assert!((compression / want_compression - 1.0).abs() < 0.08, "{compression} vs {want_compression}");
    // The energy identity includes the geometric term.
    assert!(e_t < 1e-6 && e_c < 1e-6, "{e_t} {e_c}");
}

#[test]
fn an_axial_load_in_a_stocky_lug_barely_changes() {
    let g = LugGeometry::round_head(0.5, 1.5, 0.25, 3.75);
    let m = LugModel::build_with(&g, AL, MeshSpec::default(), false, PlaneMode::Stress).unwrap();
    let case = LoadCase { load_lbf: 2000.0, angle_deg: 0.0 };
    let a = m.solve(PinSpec::new(0.4995, 0.15), case).unwrap();
    let b = m.solve_second_order(PinSpec::new(0.4995, 0.15), case).unwrap();
    assert!((b.peak_hoop / a.peak_hoop - 1.0).abs() < 0.02, "{} vs {}", a.peak_hoop, b.peak_hoop);
}
