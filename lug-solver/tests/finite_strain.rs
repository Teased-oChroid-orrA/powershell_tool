//! The finite-strain collapse solver against the small-strain collapse, mesh convergence and the
//! published pin-bearing tests (NACA TN 1503).
//!
//! With a *true* stress - strain curve and a failure strain the finite-strain solver finds the
//! collapse load the perfectly plastic rules only approximate: the failure strain it needs is a
//! consistent material constant (75S about 0.11, 14S 0.28, 24S 0.38, from the inverse calibration
//! on the twelve points), which is the signature of a model that has the right structure.

use lug_solver::*;

const AL: Material = Material { e_psi: 10.3e6, nu: 0.33 };

fn lug() -> LugGeometry {
    LugGeometry::round_head(0.5, 1.5, 0.25, 3.75)
}

fn epp(sigma_y: f64, mat: Material) -> FsMaterial {
    FsMaterial { e_psi: mat.e_psi, nu: mat.nu, law: Hardening::linear(sigma_y, 1.0, 5.0).unwrap() }
}

fn mesh(n: usize) -> MeshSpec {
    MeshSpec { elements_around: n, ..MeshSpec::default() }
}

const AXIAL: LoadCase = LoadCase { load_lbf: 1000.0, angle_deg: 0.0 };

#[test]
fn perfectly_plastic_finite_strain_agrees_with_the_small_strain_collapse() {
    let fs = FiniteLug::build(&lug(), mesh(32), epp(60_000.0, AL), true).unwrap();
    let (nnz, madds) = fs.factor_size();
    eprintln!("FS factor nnz {nnz} madds {madds:.2e}");
    let r = fs.collapse(AXIAL, FsOptions::new(0.4995, 0.0)).unwrap();
    let m = LugModel::build_with(&lug(), AL, MeshSpec::default(), true, PlaneMode::Strain).unwrap();
    let small = m.limit_load(PinSpec::new(0.4995, 0.0), AXIAL, 60_000.0).unwrap().limit_load_lbf;
    eprintln!("FS EPP collapse {:.0} vs small strain {small:.0}, peak {}, {:.2} s", r.collapse_lbf, r.peak_reached, r.elapsed_ms / 1e3);
    assert!(r.peak_reached, "{:?}", r.note);
    // The same limit analysis; the finite-strain geometry (the lug stretches and the load path
    // shortens) adds a few percent.
    assert!((r.collapse_lbf / small - 1.0).abs() < 0.07, "{} vs {small}", r.collapse_lbf);
    assert!(r.factorisations < 80, "factorisations {}", r.factorisations);
}

#[test]
fn the_collapse_is_mesh_converged() {
    let law = Hardening::true_curve(46_500.0, 65_400.0, 0.188, 1.5).unwrap();
    let mat = FsMaterial { e_psi: 10.4e6, nu: 0.33, law };
    let g = LugGeometry { hole_dia: 0.5, width: 2.0, edge: 1.0, length: 5.0, thickness: 0.25, head_corner_radius: 0.0, far_corner_radius: 0.0 };
    let at = |n: usize| {
        FiniteLug::build(&g, mesh(n), mat, true).unwrap().collapse(AXIAL, FsOptions { strain_limit: Some(0.38), travel_cap_over_a: 1.5, ..FsOptions::new(0.4996, 0.0) }).unwrap().collapse_lbf
    };
    let (coarse, mid, fine) = (at(24), at(32), at(64));
    assert!((mid / fine - 1.0).abs() < 0.02 && (coarse / fine - 1.0).abs() < 0.05, "{coarse} {mid} {fine}");
}

#[test]
fn a_lower_failure_strain_gives_a_lower_collapse_and_friction_and_interference_run() {
    let law = Hardening::true_curve(69_000.0, 78_000.0, 0.10, 1.5).unwrap();
    let mat = FsMaterial { e_psi: 10.3e6, nu: 0.33, law };
    let fl = FiniteLug::build(&lug(), mesh(24), mat, true).unwrap();
    let at = |limit: f64, pin: f64, mu: f64| fl.collapse(AXIAL, FsOptions { strain_limit: Some(limit), ..FsOptions::new(pin, mu) }).unwrap();
    let (a, b) = (at(0.05, 0.4995, 0.0), at(0.20, 0.4995, 0.0));
    assert!(a.strain_limited && b.strain_limited && a.collapse_lbf < b.collapse_lbf, "{} {}", a.collapse_lbf, b.collapse_lbf);
    let friction = at(0.10, 0.4995, 0.15);
    let frictionless = at(0.10, 0.4995, 0.0);
    assert!((friction.collapse_lbf / frictionless.collapse_lbf - 1.0).abs() < 0.10, "{} {}", friction.collapse_lbf, frictionless.collapse_lbf);
    // An interference fit (pin larger than the hole) settles first and still carries load.
    let fit = at(0.10, 0.5006, 0.0);
    assert!(fit.collapse_lbf > 0.5 * frictionless.collapse_lbf, "{}", fit.collapse_lbf);
}

#[test]
fn oblique_loads_run_on_the_full_model() {
    let law = Hardening::true_curve(69_000.0, 78_000.0, 0.10, 1.5).unwrap();
    let mat = FsMaterial { e_psi: 10.3e6, nu: 0.33, law };
    let fl = FiniteLug::build(&lug(), mesh(24), mat, false).unwrap();
    let r = fl.collapse(LoadCase { load_lbf: 1000.0, angle_deg: 45.0 }, FsOptions { travel_cap_over_a: 0.3, ..FsOptions::new(0.4995, 0.15) }).unwrap();
    assert!(r.collapse_lbf > 1000.0 && r.curve.len() > 5, "{r:?}");
    let half = FiniteLug::build(&lug(), mesh(24), mat, true).unwrap();
    assert!(half.collapse(LoadCase { load_lbf: 1000.0, angle_deg: 45.0 }, FsOptions::new(0.4995, 0.0)).is_err(), "a half model is axial only");
}

// ---- NACA TN 1503 ------------------------------------------------------------------------

struct Case {
    alloy: &'static str,
    ftu: f64,
    fty: f64,
    elong: f64,
    fbru_15: f64,
    fbru_20: f64,
    /// Failure strain from the inverse calibration (equivalent plastic strain at failure).
    eps_f: f64,
}

const CASES: [Case; 6] = [
    Case { alloy: "75S-T 1x2", ftu: 87_900.0, fty: 79_800.0, elong: 0.112, fbru_15: 115_500.0, fbru_20: 151_900.0, eps_f: 0.11 },
    Case { alloy: "75S-T 2x2", ftu: 86_300.0, fty: 69_000.0, elong: 0.114, fbru_15: 109_500.0, fbru_20: 140_400.0, eps_f: 0.11 },
    Case { alloy: "24S-T 1x2", ftu: 67_800.0, fty: 48_400.0, elong: 0.196, fbru_15: 98_500.0, fbru_20: 123_000.0, eps_f: 0.38 },
    Case { alloy: "24S-T 2x2", ftu: 65_400.0, fty: 46_500.0, elong: 0.188, fbru_15: 98_400.0, fbru_20: 123_400.0, eps_f: 0.38 },
    Case { alloy: "14S-T 1x2", ftu: 69_300.0, fty: 63_200.0, elong: 0.120, fbru_15: 102_800.0, fbru_20: 129_500.0, eps_f: 0.28 },
    Case { alloy: "14S-T 2x2", ftu: 68_700.0, fty: 61_100.0, elong: 0.116, fbru_15: 99_400.0, fbru_20: 124_200.0, eps_f: 0.28 },
];

fn naca(c: &Case, ed: f64, eps_f: f64) -> f64 {
    let (d, t) = (0.5, 0.25);
    let g = LugGeometry { hole_dia: d, width: 2.0, edge: ed * d, length: 5.0, thickness: t, head_corner_radius: 0.0, far_corner_radius: 0.0 };
    let law = Hardening::true_curve(c.fty, c.ftu, c.elong, 1.5).unwrap();
    let fl = FiniteLug::build(&g, mesh(32), FsMaterial { e_psi: 10.4e6, nu: 0.33, law }, true).unwrap();
    let r = fl.collapse(AXIAL, FsOptions { strain_limit: Some(eps_f), travel_cap_over_a: 1.5, ..FsOptions::new(d - 0.0004, 0.0) }).unwrap();
    r.collapse_lbf / (d * t)
}

fn run_all(eps: impl Fn(&Case) -> f64 + Sync) -> Vec<(usize, f64, f64)> {
    let jobs: Vec<(usize, f64)> = (0..CASES.len()).flat_map(|i| [(i, 1.5), (i, 2.0)]).collect();
    std::thread::scope(|s| {
        let eps = &eps;
        let hs: Vec<_> = jobs.iter().map(|&(i, ed)| s.spawn(move || {
            let c = &CASES[i];
            (i, ed, naca(c, ed, eps(c)) / (if ed == 1.5 { c.fbru_15 } else { c.fbru_20 }) - 1.0)
        })).collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    })
}

/// With each alloy's calibrated failure strain the twelve points are reproduced within +-8 %
/// (mean about 3 %), at both edge distances and both sizes: one material constant explains the
/// edge-distance effect, which no perfectly plastic flow-stress rule does.
#[test]
fn naca_is_reproduced_with_one_failure_strain_per_alloy() {
    let res = run_all(|c| c.eps_f);
    for (i, ed, e) in &res {
        eprintln!("NACA FS {:<10} e/D {ed}: {:+.1} %", CASES[*i].alloy, 100.0 * e);
    }
    let mean = res.iter().map(|r| r.2.abs()).sum::<f64>() / res.len() as f64;
    assert!(res.iter().all(|r| r.2.abs() < 0.09), "{res:?}");
    assert!(mean < 0.05, "mean |error| {:.1} %", 100.0 * mean);
}

/// With one common failure strain of 0.10 (no alloy data) the prediction never exceeds the tests
/// by more than 2 % and is 0 to 20 % low: conservative for the tougher alloys, on par with the Ftu rule.
#[test]
fn a_common_failure_strain_of_ten_percent_is_conservative() {
    let res = run_all(|_| 0.10);
    assert!(res.iter().all(|r| r.2 < 0.03 && r.2 > -0.25), "{res:?}");
}
