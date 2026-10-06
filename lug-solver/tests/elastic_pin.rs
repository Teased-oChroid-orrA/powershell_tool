//! The deformable pin (upgrade plan item 1) against exact solutions.

use lug_solver::contact::{ContactModel, ContactParams};
use lug_solver::fe::{Condensed, FarEnd};
use lug_solver::pin::PinDisc;
use lug_solver::*;
use std::sync::Arc;

const LUG: Material = Material { e_psi: 10.0e6, nu: 0.33 };
const STEEL: Material = Material { e_psi: 29.0e6, nu: 0.30 };

fn disc(a: f64, r: f64) -> LugGeometry {
    LugGeometry { hole_dia: 2.0 * a, width: 2.0 * r, edge: r, length: r, thickness: 1.0, head_corner_radius: r, far_corner_radius: r }
}

/// Mean fit pressure of an elastic pin of radius `a + delta` pressed into a disc: Lame for the
/// disc plus Lame for the (hollow, core ratio `core`) pin; the rigid pin is the same without the pin term.
fn lame(a: f64, r_out: f64, delta: f64, pin: Option<(Material, f64)>) -> f64 {
    let lug = ((r_out * r_out + a * a) / (r_out * r_out - a * a) + LUG.nu) / LUG.e_psi;
    let pin_term = pin.map_or(0.0, |(m, core)| {
        let ac = core * a;
        ((1.0 - m.nu) * a * a + (1.0 + m.nu) * ac * ac) / ((a * a - ac * ac) * m.e_psi)
    });
    delta / (a * (lug + pin_term))
}

fn fit_pressure(elastic: Option<Material>, a: f64, r: f64, delta: f64) -> f64 {
    let cond = Condensed::build(&disc(a, r), MeshSpec { elements_around: 48, ..Default::default() }, LUG, true, FarEnd::Soft).unwrap();
    let mut cm = ContactModel::new(&cond, a + delta, ContactParams::default(), [-1.0, 0.0]);
    if let Some(m) = elastic {
        let pin = PinDisc::build(&cond.mesh.theta, cond.mesh.periodic, a + delta, m, 6).unwrap();
        cm = cm.with_pin_compliance(Arc::new(pin.combine(&cond).unwrap()));
    }
    cm.free_perp = false;
    let mut st = cm.fresh_state();
    let res = cm.advance(&mut st, 0.0).unwrap();
    res.points.iter().map(|p| p.pressure).sum::<f64>() / res.points.len() as f64
}

#[test]
fn an_elastic_pin_press_fit_matches_the_two_cylinder_lame_solution() {
    let (a, r, delta) = (0.25, 1.0, 0.0005);
    let rigid = fit_pressure(None, a, r, delta);
    let soft = fit_pressure(Some(STEEL), a, r, delta);
    let (exact_rigid, exact_soft) = (lame(a, r, delta, None), lame(a, r, delta, Some((STEEL, 0.1))));
    eprintln!("ELPIN rigid {rigid:.0} (Lame {exact_rigid:.0}), steel pin {soft:.0} (Lame {exact_soft:.0})");
    assert!((rigid / exact_rigid - 1.0).abs() < 0.01);
    assert!((soft / exact_soft - 1.0).abs() < 0.015, "elastic pin {soft} vs Lame {exact_soft}");
    assert!(soft < 0.95 * rigid, "a deformable pin gives the interference away: {soft} vs {rigid}");
    // A pin 100 times stiffer than the lug is the rigid pin.
    let stiff = fit_pressure(Some(Material { e_psi: 1.0e9, nu: 0.30 }), a, r, delta);
    assert!((stiff / rigid - 1.0).abs() < 0.02, "{stiff} vs {rigid}");
}

#[test]
fn an_elastic_pin_in_a_lug_spreads_the_bearing_load_and_the_collapse_load_does_not_depend_on_it() {
    let g = LugGeometry::round_head(0.5, 1.5, 0.25, 3.75);
    let al = Material { e_psi: 10.3e6, nu: 0.33 };
    let m = LugModel::build_with(&g, al, MeshSpec::default(), true, PlaneMode::Stress).unwrap();
    let case = LoadCase { load_lbf: 3000.0, angle_deg: 0.0 };
    let rigid = m.solve(PinSpec::new(0.4995, 0.15), case).unwrap();
    let elastic = m.solve(PinSpec { body: PinBody::Elastic(STEEL), ..PinSpec::new(0.4995, 0.15) }, case).unwrap();
    eprintln!("ELPIN lug: peak pressure rigid {:.0} elastic {:.0}", rigid.peak_pressure, elastic.peak_pressure);
    assert!(elastic.peak_pressure < rigid.peak_pressure, "{} !< {}", elastic.peak_pressure, rigid.peak_pressure);
    assert!(elastic.contact_arc_deg >= rigid.contact_arc_deg - 1.0);
    assert!(elastic.verification.ok() && (elastic.peak_hoop / rigid.peak_hoop - 1.0).abs() < 0.05);
    // Limit analysis does not see elastic compliance: same collapse load (within the iteration tolerance).
    let pm = LugModel::build_with(&g, al, MeshSpec::default(), true, PlaneMode::Strain).unwrap();
    let a = pm.limit_load(PinSpec::new(0.4995, 0.0), case, 55_000.0).unwrap().limit_load_lbf;
    let b = pm.limit_load(PinSpec { body: PinBody::Elastic(STEEL), ..PinSpec::new(0.4995, 0.0) }, case, 55_000.0).unwrap().limit_load_lbf;
    assert!((b / a - 1.0).abs() < 0.03, "{a} vs {b}");
}

#[test]
fn an_invalid_pin_material_is_an_error() {
    let g = LugGeometry::round_head(0.5, 1.5, 0.25, 3.75);
    let m = LugModel::build_with(&g, LUG, MeshSpec::default(), true, PlaneMode::Stress).unwrap();
    let pin = PinSpec { body: PinBody::Elastic(Material { e_psi: -1.0, nu: 0.3 }), ..PinSpec::new(0.499, 0.0) };
    assert!(m.solve(pin, LoadCase { load_lbf: 1000.0, angle_deg: 0.0 }).is_err());
}
