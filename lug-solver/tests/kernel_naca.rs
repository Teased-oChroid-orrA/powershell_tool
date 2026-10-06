//! The kernel's collapse loads against the twelve NACA TN 1503 pin-bearing tests, as
//! `validation_naca_tn1503.rs` does for the condensed solver. The kernel's perfectly plastic collapse
//! sits about 3 % above the condensed solver's (fully converged Newton against an initial-strain
//! iteration), so the statistics of the three flow rules shift up by about that much: this test
//! states them for the solver the toolbox now uses by default.

use edge_check::models::contact_model::flow_stress;
use edge_check::types::Strengths;
use lug_solver::fea::FeaLug;
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

// The transcription of `validation_naca_tn1503.rs` (equal to `edge-check`'s).
const CASES: [Case; 6] = [
    Case { alloy: "75S-T 1x2 bar", ftu: 87_900.0, fty: 79_800.0, fbru_15: 115_500.0, fbru_20: 151_900.0 },
    Case { alloy: "75S-T 2x2 bar", ftu: 86_300.0, fty: 69_000.0, fbru_15: 109_500.0, fbru_20: 140_400.0 },
    Case { alloy: "24S-T 1x2 bar", ftu: 67_800.0, fty: 48_400.0, fbru_15: 98_500.0, fbru_20: 123_000.0 },
    Case { alloy: "24S-T 2x2 bar", ftu: 65_400.0, fty: 46_500.0, fbru_15: 98_400.0, fbru_20: 123_400.0 },
    Case { alloy: "14S-T 1x2 bar", ftu: 69_300.0, fty: 63_200.0, fbru_15: 102_800.0, fbru_20: 129_500.0 },
    Case { alloy: "14S-T 2x2 bar", ftu: 68_700.0, fty: 61_100.0, fbru_15: 99_400.0, fbru_20: 124_200.0 },
];
/// Elongation in 2 in. of the six bars (Table I of NACA TN 1503), in `CASES` order.
const ELONG: [f64; 6] = [0.112, 0.114, 0.196, 0.188, 0.120, 0.116];
const MAT: Material = Material { e_psi: 10.4e6, nu: 0.33 };

fn flow(c: &Case) -> f64 {
    flow_stress(&Strengths { e: MAT.e_psi, nu: MAT.nu, sy: c.fty, fsu: 0.6 * c.ftu, ftu: c.ftu, fbru: c.fbru_20, fbru_e15: c.fbru_15 })
}

#[test]
fn the_kernel_reproduces_the_pin_bearing_tests_under_the_three_flow_rules() {
    let jobs: Vec<(usize, f64)> = (0..CASES.len()).flat_map(|i| [(i, 1.5), (i, 2.0)]).collect();
    let rows: Vec<[f64; 3]> = std::thread::scope(|s| {
        let hs: Vec<_> = jobs
            .iter()
            .map(|&(i, ed)| {
                s.spawn(move || {
                    let c = &CASES[i];
                    let test = if ed == 1.5 { c.fbru_15 } else { c.fbru_20 };
                    let g = LugGeometry { hole_dia: D, width: 2.0, edge: ed * D, length: 5.0, thickness: T, head_corner_radius: 0.0, far_corner_radius: 0.0 };
                    let fe = FeaLug::build(&g, MAT, MeshSpec::default(), true, PlaneMode::Strain).unwrap();
                    let at = |f: f64| {
                        let l = fe.limit_load(PinSpec::new(D - 0.0004, 0.0), LoadCase { load_lbf: 1000.0, angle_deg: 0.0 }, f, LimitOptions::default()).unwrap();
                        assert!(l.plateau, "{} e/D {ed}: the collapse did not flatten", c.alloy);
                        l.limit_load_lbf / (D * T) / test - 1.0
                    };
                    [at(flow(c)), at(c.ftu), at(c.ftu * (1.0 + ELONG[i]))]
                })
            })
            .collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });
    for (r, (i, ed)) in rows.iter().zip(&jobs) {
        eprintln!("KERNEL NACA {:<14} e/D {ed}: mean rule {:+.1} %, Ftu {:+.1} %, Ftu(1+e) {:+.1} %", CASES[*i].alloy, 100.0 * r[0], 100.0 * r[1], 100.0 * r[2]);
    }
    let mean = |k: usize| rows.iter().map(|r| r[k]).sum::<f64>() / rows.len() as f64;
    for k in 0..3 {
        let (l, h) = rows.iter().fold((f64::MAX, f64::MIN), |(l, h), r| (l.min(r[k]), h.max(r[k])));
        eprintln!("KERNEL NACA rule {k}: {:+.1} .. {:+.1} %, mean {:+.1} %", 100.0 * l, 100.0 * h, 100.0 * mean(k));
    }
    // The statistics the toolbox quotes for the kernel (`naca_statistics`): the mean rule stays conservative on
    // every point; Ftu is no longer "never high" (two 75S points are 1.5 and 2.3 % high); Ftu (1 + elongation) is
    // high for the 75S bars.
    assert!(rows.iter().all(|r| r[0] < 0.0 && r[0] > -0.26), "(Ftu + Fty)/2: 6 to 25 % low");
    assert!(rows.iter().all(|r| r[1] < 0.03 && r[1] > -0.14), "Ftu: from 13 % low to 2 % high");
    assert!(rows.iter().all(|r| r[2] > -0.03 && r[2] < 0.15) && rows.iter().any(|r| r[2] > 0.10), "Ftu (1 + elongation): -2 to +14 %");
    assert!((mean(0) + 0.151).abs() < 0.01 && (mean(1) + 0.071).abs() < 0.01 && (mean(2) - 0.054).abs() < 0.01, "means {} {} {}", mean(0), mean(1), mean(2));
}
