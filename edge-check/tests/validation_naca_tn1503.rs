//! Validation against test data: NACA TN 1503 (R. L. Moore, Alcoa, 1948),
//! pin-bearing tests on rolled bar. Pin 0.500 in diameter (steel), specimen
//! thickness 0.250 in (t/D = 0.5), width 2 in (the hole is centred, so the
//! side distance is 1 in = 2 D), edge distance e/D = 1.5 and 2.0, ultimate
//! bearing stress = peak load / (D t). Tensile properties are the averages
//! of Table I; bearing ultimates the averages of Table II (both transcribed
//! from the report's page images and checked against its Table III ratios).
//!
//! The pin in these tests is a plain steel pin in a reamed hole: no bushing
//! and no interference. They therefore validate the contact FE's edge /
//! bearing capacity, plasticity and contact, but not the fit (see
//! `docs/edge-distance-crosscheck.md`).

use edge_check::contact::ContactMesh;
use edge_check::fem::MeshSpec;
use edge_check::models::contact_model::flow_stress;
use edge_check::types::{BushingSpec, Geometry, Strengths};

const D: f64 = 0.5;
const T: f64 = 0.25;

struct Case {
    alloy: &'static str,
    /// Tensile ultimate and 0.2 % yield (psi), Table I averages.
    ftu: f64,
    fty: f64,
    /// Bearing ultimate (psi) at e/D = 1.5 and 2.0, Table II averages.
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

/// Predicted ultimate bearing stress for edge distance `ed` (in D).
fn predict(sigma0: f64, ed: f64, friction: f64) -> f64 {
    let a = D / 2.0;
    let e = ed * D;
    // Specimen: 18 in long, hole near one end, 2 in wide.
    let geom = Geometry { bore_radius: a, edge: e, thickness: T, plate_far: 5.0, plate_half_height: 1.0, plane_angle_deg: 40.0 };
    // A steel pin modelled as a thick steel cylinder loaded through a small
    // rigid core (the load then spreads through the elastic steel like a pin's).
    let pin = BushingSpec { inner_radius: 0.4 * a, interference: 0.0, e: 29.0e6, nu: 0.30, friction };
    let mesh = MeshSpec { n_radial: 10, n_arc: [3, 4, 10], grade: 2.0 };
    let m = ContactMesh::build(&geom, &pin, 10.4e6, 0.33, mesh, 3, true).unwrap();
    m.collapse(0.0, sigma0, T).unwrap().collapse / (D * T)
}

/// The flow stress the contact model uses for these properties.
fn model_flow_stress(c: &Case) -> f64 {
    let m = Strengths { e: 10.4e6, nu: 0.33, sy: c.fty, fsu: 0.6 * c.ftu, ftu: c.ftu, fbru: c.fbru_20, fbru_e15: c.fbru_15 };
    flow_stress(&m)
}

/// Predicted ultimate bearing stress for every case at e/D 1.5 and 2.0 (all
/// in parallel): `(case, e/D, friction, test, predicted)`.
fn predictions(mu: f64, sigma0: impl Fn(&Case) -> f64 + Sync) -> Vec<(usize, f64, f64, f64)> {
    let jobs: Vec<(usize, f64)> = (0..CASES.len()).flat_map(|i| [(i, 1.5), (i, 2.0)]).collect();
    let mut out = Vec::new();
    for chunk in jobs.chunks(8) {
        std::thread::scope(|sc| {
            let sigma0 = &sigma0;
            let hs: Vec<_> = chunk
                .iter()
                .map(|&(i, ed)| {
                    sc.spawn(move || {
                        let c = &CASES[i];
                        (i, ed, if ed == 1.5 { c.fbru_15 } else { c.fbru_20 }, predict(sigma0(c), ed, mu))
                    })
                })
                .collect();
            out.extend(hs.into_iter().map(|h| h.join().unwrap()));
        });
    }
    out
}

/// The contact model, with its own flow-stress rule and a frictionless pin,
/// against all twelve test points: no over-prediction beyond 8 %, no
/// under-prediction beyond 20 %, mean absolute error under 10 %, and the
/// e/D 1.5 -> 2.0 strengthening (the quantity edge distance is about) within
/// 10 % of the tests.
#[test]
fn the_contact_fe_reproduces_the_pin_bearing_tests() {
    let p = predictions(0.0, model_flow_stress);
    let mut sum_abs = 0.0;
    for &(i, ed, test, pred) in &p {
        let err = pred / test - 1.0;
        assert!((-0.20..=0.08).contains(&err), "{} e/D {ed}: test {test:.0}, predicted {pred:.0} ({:+.1} %)", CASES[i].alloy, 100.0 * err);
        sum_abs += err.abs();
    }
    assert!(sum_abs / p.len() as f64 > 0.0 && sum_abs / (p.len() as f64) < 0.10, "mean |error| {:.1} %", 100.0 * sum_abs / p.len() as f64);
    for i in 0..CASES.len() {
        let at = |ed: f64| p.iter().find(|r| r.0 == i && r.1 == ed).unwrap();
        let (test_ratio, pred_ratio) = (at(2.0).2 / at(1.5).2, at(2.0).3 / at(1.5).3);
        assert!((pred_ratio / test_ratio - 1.0).abs() < 0.10, "{}: Fbru(2.0)/Fbru(1.5) test {test_ratio:.3}, predicted {pred_ratio:.3}", CASES[i].alloy);
    }
}

#[test]
#[ignore = "prints the validation table; run with --ignored --nocapture"]
fn report() {
    for (label, mu, rule) in [
        ("flow stress (Ftu+Fty)/2 (model), frictionless pin", 0.0, 0usize),
        ("flow stress Ftu, frictionless pin", 0.0, 1),
        ("flow stress (Ftu+Fty)/2, pin-hole friction 0.2", 0.2, 0),
    ] {
        println!("{label}");
        let p = predictions(mu, move |c: &Case| if rule == 0 { model_flow_stress(c) } else { c.ftu });
        for (i, ed, test, pred) in p {
            println!("  {:<14} e/D {ed}: test {test:>8.0} predicted {pred:>8.0} ({:+.1}%)", CASES[i].alloy, 100.0 * (pred / test - 1.0));
        }
    }
}
