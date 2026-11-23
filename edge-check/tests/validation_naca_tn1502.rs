//! Validation against test data, second set: NACA TN 1502 (R. L. Moore, Alcoa, 1948; NTRS 19930082185),
//! "Bearing Tests of 14S Sheet and Plate". Same method as `validation_naca_tn1503.rs` (2 in wide specimens, hole
//! centred, steel pin, ultimate bearing stress = peak load / (D t), e/D 1.5 and 2.0), but two thicknesses:
//!
//! * 0.064 in sheet on a 1/4 in pin: t/D = 0.256 (the t/D range MMPDS covers is 0.25-0.50);
//! * 0.250 in plate (machined from 0.750 in plate, or Alclad sheet) on a 1/2 in pin: t/D = 0.5.
//!
//! Each row is one alloy / temper / grain direction: with grain (`W`) and across grain (`X`) are separate tests, each
//! predicted from its own direction's tensile properties. Values are the averages of two tests (Table III, bearing
//! ultimate) and of two tensile tests (Table I), transcribed by hand from the report's page images and checked
//! against the report's own Table IV ratios `bearing ultimate / tensile ultimate` (`table_iv_ratios_match`).
//! Temper names: 14S-W is now 14S-T3 / T4, 14S-T is now 14S-T6. Alclad rows are the clad sheet as tested.
//!
//! Like TN 1503 these are plain steel pins in reamed holes: no bushing and no interference. They validate the
//! contact FE's edge / bearing capacity, plasticity and contact at a second t/D, not the fit.

use edge_check::contact::ContactMesh;
use edge_check::fem::MeshSpec;
use edge_check::models::contact_model::flow_stress;
use edge_check::types::{BushingSpec, Geometry, Strengths};

struct Row {
    alloy: &'static str,
    /// Pin diameter (in): 0.25 for the 0.064 in sheet, 0.5 for the 0.250 in plate.
    d: f64,
    t: f64,
    /// Direction of test: `W` with grain, `X` across grain.
    dir: char,
    /// Tensile ultimate and 0.2 % yield (psi), Table I.
    ftu: f64,
    fty: f64,
    /// Bearing ultimate (psi) at e/D 1.5 and 2.0, Table III averages.
    fbru_15: f64,
    fbru_20: f64,
    /// The report's own ratios (Table IV) `bearing ultimate / tensile ultimate` at e/D 1.5 and 2.0.
    iv_15: f64,
    iv_20: f64,
}

const ROWS: [Row; 12] = [
    Row { alloy: "Alclad 14S-W", d: 0.25, t: 0.064, dir: 'X', ftu: 64_800.0, fty: 40_400.0, fbru_15: 103_800.0, fbru_20: 131_300.0, iv_15: 1.60, iv_20: 2.02 },
    Row { alloy: "Alclad 14S-W", d: 0.25, t: 0.064, dir: 'W', ftu: 65_800.0, fty: 47_400.0, fbru_15: 105_900.0, fbru_20: 135_900.0, iv_15: 1.61, iv_20: 2.06 },
    Row { alloy: "Alclad 14S-T", d: 0.25, t: 0.064, dir: 'X', ftu: 68_200.0, fty: 60_700.0, fbru_15: 110_100.0, fbru_20: 147_100.0, iv_15: 1.61, iv_20: 2.16 },
    Row { alloy: "Alclad 14S-T", d: 0.25, t: 0.064, dir: 'W', ftu: 68_800.0, fty: 62_000.0, fbru_15: 109_600.0, fbru_20: 142_200.0, iv_15: 1.59, iv_20: 2.07 },
    Row { alloy: "Alclad 14S-W", d: 0.5, t: 0.25, dir: 'X', ftu: 64_500.0, fty: 39_800.0, fbru_15: 100_000.0, fbru_20: 129_600.0, iv_15: 1.55, iv_20: 2.01 },
    Row { alloy: "Alclad 14S-W", d: 0.5, t: 0.25, dir: 'W', ftu: 65_200.0, fty: 47_600.0, fbru_15: 97_200.0, fbru_20: 130_000.0, iv_15: 1.49, iv_20: 2.00 },
    Row { alloy: "Alclad 14S-T", d: 0.5, t: 0.25, dir: 'X', ftu: 68_000.0, fty: 59_700.0, fbru_15: 104_100.0, fbru_20: 137_800.0, iv_15: 1.53, iv_20: 2.02 },
    Row { alloy: "Alclad 14S-T", d: 0.5, t: 0.25, dir: 'W', ftu: 69_600.0, fty: 63_800.0, fbru_15: 101_000.0, fbru_20: 133_500.0, iv_15: 1.45, iv_20: 1.92 },
    Row { alloy: "14S-W plate", d: 0.5, t: 0.25, dir: 'X', ftu: 66_500.0, fty: 43_900.0, fbru_15: 97_300.0, fbru_20: 131_700.0, iv_15: 1.46, iv_20: 1.98 },
    Row { alloy: "14S-W plate", d: 0.5, t: 0.25, dir: 'W', ftu: 69_600.0, fty: 48_300.0, fbru_15: 99_800.0, fbru_20: 127_200.0, iv_15: 1.44, iv_20: 1.83 },
    Row { alloy: "14S-T plate", d: 0.5, t: 0.25, dir: 'X', ftu: 70_600.0, fty: 62_800.0, fbru_15: 102_400.0, fbru_20: 141_000.0, iv_15: 1.45, iv_20: 2.00 },
    Row { alloy: "14S-T plate", d: 0.5, t: 0.25, dir: 'W', ftu: 71_600.0, fty: 63_800.0, fbru_15: 98_900.0, fbru_20: 139_800.0, iv_15: 1.38, iv_20: 1.95 },
];

/// The transcription equals the report's own ratios (Table IV, printed to two decimals; the average of two rounded
/// bearing values over a rounded tensile strength agrees to 0.5 %).
#[test]
fn table_iv_ratios_match() {
    for r in &ROWS {
        for (b, iv, e) in [(r.fbru_15, r.iv_15, 1.5), (r.fbru_20, r.iv_20, 2.0)] {
            assert!((b / r.ftu / iv - 1.0).abs() < 0.006, "{} {} e/D {e}: {:.3} vs Table IV {iv}", r.alloy, r.dir, b / r.ftu);
        }
    }
}

/// Predicted ultimate bearing stress for edge distance `ed` (in D) on a 2 in wide specimen.
fn predict(r: &Row, ed: f64, friction: f64) -> f64 {
    let a = r.d / 2.0;
    let geom = Geometry { bore_radius: a, edge: ed * r.d, thickness: r.t, plate_far: 5.0, plate_half_height: 1.0, plane_angle_deg: 40.0 };
    let pin = BushingSpec { inner_radius: 0.4 * a, interference: 0.0, e: 29.0e6, nu: 0.30, friction };
    let mesh = MeshSpec { n_radial: 10, n_arc: [3, 4, 10], grade: 2.0 };
    let m = ContactMesh::build(&geom, &pin, 10.4e6, 0.33, mesh, 3, true).unwrap();
    m.collapse(0.0, flow(r), r.t).unwrap().collapse / (r.d * r.t)
}

fn flow(r: &Row) -> f64 {
    flow_stress(&Strengths { e: 10.4e6, nu: 0.33, sy: r.fty, fsu: 0.6 * r.ftu, ftu: r.ftu, fbru: r.fbru_20, fbru_e15: r.fbru_15 })
}

/// `(row, e/D, test, predicted)` for every row at e/D 1.5 and 2.0, run in parallel.
fn predictions(mu: f64) -> Vec<(usize, f64, f64, f64)> {
    let jobs: Vec<(usize, f64)> = (0..ROWS.len()).flat_map(|i| [(i, 1.5), (i, 2.0)]).collect();
    let mut out = Vec::new();
    for chunk in jobs.chunks(8) {
        std::thread::scope(|sc| {
            let hs: Vec<_> = chunk.iter().map(|&(i, ed)| sc.spawn(move || (i, ed, if ed == 1.5 { ROWS[i].fbru_15 } else { ROWS[i].fbru_20 }, predict(&ROWS[i], ed, mu)))).collect();
            out.extend(hs.into_iter().map(|h| h.join().unwrap()));
        });
    }
    out
}

/// The contact model, with its own flow-stress rule and a frictionless pin, against all 24 points: none over-predicted
/// (the property that matters for a check that could become governing), none under-predicted by more than 31 %, mean
/// absolute error under 22 % (measured: -28.6 % .. -5.0 %, mean -18.9 %, sd 6.5 %), and the e/D 1.5 -> 2.0
/// strengthening within 10 % of the tests. Bounds sit just outside the measured spread; they are not fitted.
#[test]
fn the_contact_fe_is_conservative_on_the_tn1502_tests() {
    let p = predictions(0.0);
    let mut sum_abs = 0.0;
    for &(i, ed, test, pred) in &p {
        let err = pred / test - 1.0;
        assert!((-0.31..0.0).contains(&err), "{} {} e/D {ed}: test {test:.0}, predicted {pred:.0} ({:+.1} %)", ROWS[i].alloy, ROWS[i].dir, 100.0 * err);
        sum_abs += err.abs();
    }
    assert!(sum_abs / (p.len() as f64) < 0.22, "mean |error| {:.1} %", 100.0 * sum_abs / p.len() as f64);
    for (i, row) in ROWS.iter().enumerate() {
        let at = |ed: f64| p.iter().find(|r| r.0 == i && r.1 == ed).unwrap();
        let (test_ratio, pred_ratio) = (at(2.0).2 / at(1.5).2, at(2.0).3 / at(1.5).3);
        assert!((pred_ratio / test_ratio - 1.0).abs() < 0.10, "{} {}: Fbru(2.0)/Fbru(1.5) test {test_ratio:.3}, predicted {pred_ratio:.3}", row.alloy, row.dir);
    }
}

#[test]
#[ignore = "exploration: prints the table"]
fn report() {
    let p = predictions(0.0);
    let mut errs = Vec::new();
    for &(i, ed, test, pred) in &p {
        let r = &ROWS[i];
        errs.push(pred / test - 1.0);
        println!("  {:<14} {} t/D {:.3} e/D {ed}: test {test:>8.0} predicted {pred:>8.0} ({:+.1}%)", r.alloy, r.dir, r.t / r.d, 100.0 * (pred / test - 1.0));
    }
    let n = errs.len() as f64;
    let mean = errs.iter().sum::<f64>() / n;
    let sd = (errs.iter().map(|e| (e - mean).powi(2)).sum::<f64>() / (n - 1.0)).sqrt();
    println!("mean {:+.1} %, sd {:.1} %, min {:+.1} %, max {:+.1} %", 100.0 * mean, 100.0 * sd, 100.0 * errs.iter().cloned().fold(f64::MAX, f64::min), 100.0 * errs.iter().cloned().fold(f64::MIN, f64::max));
}
