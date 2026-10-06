//! The library's failure-strain rule `eps_f = m ln(1 + A)` against pin-bearing tests.
//!
//! Data: R. L. Moore, NACA TN 1503 (1948), Tables I and III (rolled bar 75S-T, 24S-T, 14S-T, pin 0.5 in,
//! 0.25 x 2.0 in specimens), and NACA TN 1502 (1948), Tables I and III (14S-T / 14S-W plate machined to
//! 0.25 in and Alclad 14S 0.25 in, the same pin and width; with-grain values). `A` is the tensile
//! elongation in 2 in. of the same specimens' material. The TN 1502 points were NOT used to set `m`.
//!
//! Result: all twenty points within -12 % to +4 % (conservative on average), for tempers that differ
//! by a factor of two in elongation (T: 10-12 %, W: 17-21 %), which is the check on the elongation
//! scaling itself.

use lug_solver::*;
use mechanics_core::fracture::{failure_strain, Basis};

struct Case {
    name: &'static str,
    ftu: f64,
    fty: f64,
    a: f64,
    fbru_15: f64,
    fbru_20: f64,
    /// Library id of a material whose rule this case exercises (multiplier check).
    m: f64,
}

const CASES: [Case; 10] = [
    Case { name: "75S-T 1x2 bar", ftu: 87_900.0, fty: 79_800.0, a: 0.112, fbru_15: 115_500.0, fbru_20: 151_900.0, m: 1.0 },
    Case { name: "75S-T 2x2 bar", ftu: 86_300.0, fty: 69_000.0, a: 0.114, fbru_15: 109_500.0, fbru_20: 140_400.0, m: 1.0 },
    Case { name: "24S-T 1x2 bar", ftu: 67_800.0, fty: 48_400.0, a: 0.196, fbru_15: 98_500.0, fbru_20: 123_000.0, m: 2.0 },
    Case { name: "24S-T 2x2 bar", ftu: 65_400.0, fty: 46_500.0, a: 0.188, fbru_15: 98_400.0, fbru_20: 123_400.0, m: 2.0 },
    Case { name: "14S-T 1x2 bar", ftu: 69_300.0, fty: 63_200.0, a: 0.120, fbru_15: 102_800.0, fbru_20: 129_500.0, m: 2.0 },
    Case { name: "14S-T 2x2 bar", ftu: 68_700.0, fty: 61_100.0, a: 0.116, fbru_15: 99_400.0, fbru_20: 124_200.0, m: 2.0 },
    Case { name: "TN1502 14S-T plate", ftu: 71_600.0, fty: 63_800.0, a: 0.098, fbru_15: 98_900.0, fbru_20: 139_800.0, m: 2.0 },
    Case { name: "TN1502 14S-W plate", ftu: 69_600.0, fty: 48_300.0, a: 0.173, fbru_15: 99_800.0, fbru_20: 127_200.0, m: 2.0 },
    Case { name: "TN1502 Alclad 14S-T", ftu: 69_600.0, fty: 63_800.0, a: 0.115, fbru_15: 101_000.0, fbru_20: 133_500.0, m: 2.0 },
    Case { name: "TN1502 Alclad 14S-W", ftu: 65_200.0, fty: 47_600.0, a: 0.210, fbru_15: 97_200.0, fbru_20: 130_000.0, m: 2.0 },
];

fn predict(c: &Case, ed: f64) -> f64 {
    let (d, t) = (0.5, 0.25);
    let g = LugGeometry { hole_dia: d, width: 2.0, edge: ed * d, length: 5.0, thickness: t, head_corner_radius: 0.0, far_corner_radius: 0.0 };
    let law = Hardening::true_curve(c.fty, c.ftu, c.a, 1.5).unwrap();
    let fl = FiniteLug::build(&g, MeshSpec { elements_around: 32, ..MeshSpec::default() }, FsMaterial { e_psi: 10.4e6, nu: 0.33, law }, true).unwrap();
    let eps = (c.m * (1.0 + c.a).ln()).clamp(0.03, 0.5);
    fl.collapse(LoadCase { load_lbf: 1000.0, angle_deg: 0.0 }, FsOptions { strain_limit: Some(eps), travel_cap_over_a: 1.5, ..FsOptions::new(d - 0.0004, 0.0) }).unwrap().collapse_lbf / (d * t)
}

#[test]
fn the_failure_strain_rule_reproduces_twenty_pin_bearing_points() {
    let rows: Vec<(String, f64, f64)> = std::thread::scope(|s| {
        let hs: Vec<_> = CASES.iter().map(|c| s.spawn(move || (c.name.to_string(), predict(c, 1.5) / c.fbru_15 - 1.0, predict(c, 2.0) / c.fbru_20 - 1.0))).collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let mut sum = 0.0;
    for (name, e15, e20) in &rows {
        eprintln!("RULE {name}: e/D 1.5 {:+.1} %, e/D 2.0 {:+.1} %", 100.0 * e15, 100.0 * e20);
        assert!((-0.12..=0.05).contains(e15) && (-0.12..=0.05).contains(e20), "{name}: {e15} {e20}");
        sum += e15.abs() + e20.abs();
    }
    assert!(sum / 20.0 < 0.035, "mean |error| {:.1} %", 100.0 * sum / 20.0);
}

/// The library multipliers are the ones the test above validates.
#[test]
fn the_library_uses_the_validated_multipliers_for_these_alloys() {
    let m = |id: &str| failure_strain(id).unwrap();
    assert_eq!((m("al7075").multiplier, m("al7075").basis), (1.0, Basis::Typical));
    assert_eq!(m("al2024").multiplier, 2.0);
    // A handbook 2024-T3 sheet condition with an elongation.
    let hb = mechanics_core::materials::builtin_catalog().find(|x| x.extra.is_some_and(|e| e.alloy.starts_with("2024") && e.elong.l > 0.0)).unwrap();
    let f = failure_strain(hb.id).unwrap();
    assert!((f.multiplier, f.basis) == (2.0, Basis::Validated), "{f:?}");
}
