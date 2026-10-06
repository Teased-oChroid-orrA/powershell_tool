//! Validation against published test data: NACA TN 1503 (R. L. Moore, Alcoa,
//! 1948), pin-bearing tests on rolled bar - the same twelve points the
//! `edge-check` contact FE is validated against (`edge-check/tests/
//! validation_naca_tn1503.rs`; the transcription below is copied from there and
//! must stay equal to it). Pin 0.500 in, specimen thickness 0.250 in, width
//! 2 in with the hole centred, edge distance e/D = 1.5 and 2.0; ultimate
//! bearing stress = peak load / (D t). A plain steel pin in a reamed hole.
//!
//! Result (see `docs/lug-analysis.md`): all twelve points are predicted LOW, by 9 to 28 %
//! (mean about -18 %), about 8 points below the `edge-check` contact FE, whose deformable
//! steel pin spreads the bearing load; a rigid pin cannot. Never over-predicting is the
//! property that matters for a strength check, and the e/D 1.5 -> 2.0 strengthening is
//! reproduced within 12 %.
//!
//! Here the specimen is a lug with a square end and the pin is a rigid, frictionless
//! analytic circle; the plasticity is plane-strain elastic-perfectly-plastic at the
//! same `(Ftu + Fty) / 2` flow stress rule. Plane stress is NOT used: bearing crush
//! at the bore would cap the radial stress near 1.15 sigma_f and hide the edge
//! (`lug_limit_load_plane_stress_is_bearing_crush_limited` in `plastic_validation.rs`).

use edge_check::models::contact_model::flow_stress;
use edge_check::types::Strengths;
use lug_solver::*;

const D: f64 = 0.5;
const T: f64 = 0.25;

struct Case {
    alloy: &'static str,
    ftu: f64,
    fty: f64,
    fbru_15: f64,
    fbru_20: f64,
}

const CASES: [Case; 6] = [
    Case { alloy: "75S-T 1x2 bar", ftu: 87_900.0, fty: 79_800.0, fbru_15: 115_500.0, fbru_20: 151_900.0 },
    Case { alloy: "75S-T 2x2 bar", ftu: 86_300.0, fty: 69_000.0, fbru_15: 109_500.0, fbru_20: 140_400.0 },
    Case { alloy: "24S-T 1x2 bar", ftu: 67_800.0, fty: 48_400.0, fbru_15: 98_500.0, fbru_20: 123_000.0 },
    Case { alloy: "24S-T 2x2 bar", ftu: 65_400.0, fty: 46_500.0, fbru_15: 98_400.0, fbru_20: 123_400.0 },
    Case { alloy: "14S-T 1x2 bar", ftu: 69_300.0, fty: 63_200.0, fbru_15: 102_800.0, fbru_20: 129_500.0 },
    Case { alloy: "14S-T 2x2 bar", ftu: 68_700.0, fty: 61_100.0, fbru_15: 99_400.0, fbru_20: 124_200.0 },
];

const MAT: Material = Material { e_psi: 10.4e6, nu: 0.33 };

fn flow(c: &Case) -> f64 {
    flow_stress(&Strengths { e: MAT.e_psi, nu: MAT.nu, sy: c.fty, fsu: 0.6 * c.ftu, ftu: c.ftu, fbru: c.fbru_20, fbru_e15: c.fbru_15 })
}

/// Predicted ultimate bearing stress (peak load / (D t)) and whether the curve flattened.
fn predict(c: &Case, ed: f64, mesh: MeshSpec) -> (f64, bool) {
    let g = LugGeometry { hole_dia: D, width: 2.0, edge: ed * D, length: 5.0, thickness: T, head_corner_radius: 0.0, far_corner_radius: 0.0 };
    let m = LugModel::build_with(&g, MAT, mesh, true, PlaneMode::Strain).unwrap();
    let l = m.limit_load(PinSpec::new(D - 0.0004, 0.0), LoadCase { load_lbf: 1000.0, angle_deg: 0.0 }, flow(c)).unwrap();
    (l.limit_load_lbf / (D * T), l.plateau)
}

fn all(mesh: MeshSpec) -> Vec<(usize, f64, f64, f64, bool)> {
    let jobs: Vec<(usize, f64)> = (0..CASES.len()).flat_map(|i| [(i, 1.5), (i, 2.0)]).collect();
    std::thread::scope(|s| {
        let hs: Vec<_> = jobs
            .iter()
            .map(|&(i, ed)| {
                s.spawn(move || {
                    let c = &CASES[i];
                    let (pred, plateau) = predict(c, ed, mesh);
                    (i, ed, if ed == 1.5 { c.fbru_15 } else { c.fbru_20 }, pred, plateau)
                })
            })
            .collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    })
}

#[test]
fn the_lug_fe_reproduces_the_pin_bearing_tests() {
    let p = all(MeshSpec::default());
    for &(i, ed, test, pred, plateau) in &p {
        eprintln!("NACA {:<14} e/D {ed}: test {test:>8.0} predicted {pred:>8.0} ({:+.1} %) plateau {plateau}", CASES[i].alloy, 100.0 * (pred / test - 1.0));
    }
    let mut sum_abs = 0.0;
    for &(i, ed, test, pred, plateau) in &p {
        let err = pred / test - 1.0;
        assert!(plateau, "{} e/D {ed}: the collapse load did not flatten", CASES[i].alloy);
        assert!((-0.30..=0.0).contains(&err), "{} e/D {ed}: test {test:.0}, predicted {pred:.0} ({:+.1} %)", CASES[i].alloy, 100.0 * err);
        sum_abs += err.abs();
    }
    assert!(sum_abs / (p.len() as f64) < 0.20, "mean |error| {:.1} %", 100.0 * sum_abs / p.len() as f64);
    // The edge-distance strengthening Fbru(2.0)/Fbru(1.5) is what edge distance is about.
    for (i, case) in CASES.iter().enumerate() {
        let at = |ed: f64| p.iter().find(|r| r.0 == i && r.1 == ed).unwrap();
        let (tr, pr) = (at(2.0).2 / at(1.5).2, at(2.0).3 / at(1.5).3);
        assert!((pr / tr - 1.0).abs() < 0.12, "{}: Fbru(2.0)/Fbru(1.5) test {tr:.3}, predicted {pr:.3}", case.alloy);
    }
}


/// With strain hardening (Ramberg-Osgood through Fty and Ftu, elongation 10 %) and the collapse
/// taken where the strain at the hole reaches 10 %, the same twelve points are still never
/// over-predicted (7 to 24 % low, mean about 16 %, against 9 to 28 % and 18 % perfectly plastic).
/// The pin's elasticity does not move a limit load (`tests/elastic_pin.rs`), so the remaining gap is
/// not a rigid-pin artefact. `docs/lug-analysis.md` records this.
#[test]
fn hardening_with_a_ten_percent_failure_strain_is_conservative_and_a_little_closer() {
    let predict_h = |c: &Case, ed: f64| {
        let g = LugGeometry { hole_dia: D, width: 2.0, edge: ed * D, length: 5.0, thickness: T, head_corner_radius: 0.0, far_corner_radius: 0.0 };
        let m = LugModel::build_with(&g, MAT, MeshSpec::default(), true, PlaneMode::Strain).unwrap();
        let h = Hardening::ramberg_osgood(c.fty, c.ftu, 0.10).unwrap();
        let options = LimitOptions { hardening: Some(h), strain_limit: Some(0.10), travel_cap_over_a: 1.0, ..LimitOptions::default() };
        let l = m.limit_load_with(PinSpec::new(D - 0.0004, 0.0), LoadCase { load_lbf: 1000.0, angle_deg: 0.0 }, c.fty, options).unwrap();
        assert!(l.strain_limited, "{}: the strain limit was not reached", c.alloy);
        l.limit_load_lbf / (D * T)
    };
    let jobs: Vec<(usize, f64)> = (0..CASES.len()).flat_map(|i| [(i, 1.5), (i, 2.0)]).collect();
    let errs: Vec<f64> = std::thread::scope(|s| {
        let hs: Vec<_> = jobs.iter().map(|&(i, ed)| {
            let predict_h = &predict_h;
            s.spawn(move || {
                let c = &CASES[i];
                predict_h(c, ed) / (if ed == 1.5 { c.fbru_15 } else { c.fbru_20 }) - 1.0
            })
        }).collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });
    for (e, (i, ed)) in errs.iter().zip(&jobs) {
        assert!((-0.30..=0.0).contains(e), "{} e/D {ed}: {:+.1} %", CASES[*i].alloy, 100.0 * e);
    }
    let mean = errs.iter().map(|e| e.abs()).sum::<f64>() / errs.len() as f64;
    assert!(mean < 0.18, "mean |error| {:.1} %", 100.0 * mean);
}

/// Elongation in 2 in. of the six bars (Table I of NACA TN 1503), in `CASES` order.
const ELONG: [f64; 6] = [0.112, 0.114, 0.196, 0.188, 0.120, 0.116];

/// The three flow-stress rules over the twelve points, as the toolbox reports them: `(Ftu + Fty)/2`
/// 9-28 % low (mean 18 %); `Ftu` 1-17 % low (mean 10 %, never high); `Ftu (1 + elongation)`
/// -5 to +10 % (mean 4.3 %), high for the 75S bars. Same solver, only the flow stress differs.
#[test]
fn the_flow_rules_reproduce_the_documented_naca_accuracy() {
    let jobs: Vec<(usize, f64)> = (0..CASES.len()).flat_map(|i| [(i, 1.5), (i, 2.0)]).collect();
    let rows: Vec<[f64; 3]> = std::thread::scope(|s| {
        let hs: Vec<_> = jobs.iter().map(|&(i, ed)| s.spawn(move || {
            let c = &CASES[i];
            let test = if ed == 1.5 { c.fbru_15 } else { c.fbru_20 };
            let g = LugGeometry { hole_dia: D, width: 2.0, edge: ed * D, length: 5.0, thickness: T, head_corner_radius: 0.0, far_corner_radius: 0.0 };
            let m = LugModel::build_with(&g, MAT, MeshSpec::default(), true, PlaneMode::Strain).unwrap();
            let at = |f: f64| m.limit_load(PinSpec::new(D - 0.0004, 0.0), LoadCase { load_lbf: 1000.0, angle_deg: 0.0 }, f).unwrap().limit_load_lbf / (D * T) / test - 1.0;
            [at(flow(c)), at(c.ftu), at(c.ftu * (1.0 + ELONG[i]))]
        })).collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let mean_abs = |k: usize| rows.iter().map(|r| r[k].abs()).sum::<f64>() / rows.len() as f64;
    assert!(rows.iter().all(|r| r[1] < 0.0 && r[1] > -0.20), "Ftu must never over-predict");
    assert!(rows.iter().any(|r| r[2] > 0.05), "the true-ultimate rule is high for the 75S bars");
    assert!(mean_abs(1) < 0.12 && mean_abs(2) < 0.06 && mean_abs(0) > 0.15, "{} {} {}", mean_abs(0), mean_abs(1), mean_abs(2));
}
