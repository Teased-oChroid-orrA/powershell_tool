//! The per-mode allowables of `edge_check::modes`: every closed form against its hand calculation, the fit pressure against
//! Lamé, and the rows that must say why they have no number.

use edge_check::modes::{analyse, fit_pressure, Group, ModesInput, ModesResult, PinSpec};
use edge_check::types::Strengths;
use mechanics_core::materials::get_material;

const D: f64 = 0.25;

fn plate() -> Strengths {
    Strengths::from_material(get_material("al7075"))
}

fn pin(delta: f64) -> PinSpec {
    PinSpec { dia: D + delta, e: 29.0e6, nu: 0.30, ftu: 160_000.0, fsu: 95_000.0, fty: Some(140_000.0), fbru: Some(240_000.0), planes: 2, span: 0.5 }
}

fn input(edge_over_d: f64, delta: f64) -> ModesInput {
    let mut plate = plate();
    plate.fbru_e15 = 100_000.0;
    ModesInput { bore_dia: D, edge: edge_over_d * D, thickness: 0.125, plate, fbry: 90_000.0, fbry_e15: 70_000.0, pin: pin(delta), delta, custom_pressure: None, friction: 0.0, plane_angle_deg: 40.0, deep: false }
}

fn row<'a>(r: &'a ModesResult, id: &str) -> &'a edge_check::modes::ModeRow {
    r.rows.iter().find(|x| x.id == id).unwrap_or_else(|| panic!("no row {id}"))
}

fn close(a: f64, b: f64) {
    assert!((a / b - 1.0).abs() < 1e-9, "{a} vs {b}");
}

#[test]
fn bearing_rows_follow_the_tabulated_edge_ratio_rule() {
    let i = input(2.0, 0.0);
    let r = analyse(&i).unwrap();
    close(row(&r, "bearing_onset").handbook.unwrap(), 90_000.0 * D * 0.125);
    close(row(&r, "bearing_ultimate").handbook.unwrap(), i.plate.fbru * D * 0.125);
    // Halfway between 1.5 and 2.0 the value is the mean of the two table points.
    let r = analyse(&input(1.75, 0.0)).unwrap();
    close(row(&r, "bearing_onset").handbook.unwrap(), 0.5 * (90_000.0 + 70_000.0) * D * 0.125);
    // Below e/D 1.5 the tables do not reach: a reason, no number.
    let r = analyse(&input(1.3, 0.0)).unwrap();
    let b = row(&r, "bearing_onset");
    assert!(b.handbook.is_none() && b.why.as_deref().unwrap().contains("tables stop at e/D 1.5"));
}

#[test]
fn no_tabulated_fbry_gives_a_reason_not_a_guess() {
    let mut i = input(2.0, 0.0);
    i.fbry = 0.0;
    i.fbry_e15 = 0.0;
    let r = analyse(&i).unwrap();
    let b = row(&r, "bearing_onset");
    assert!(b.handbook.is_none() && b.why.as_deref().unwrap().contains("Fbry"));
}

#[test]
fn shear_out_is_the_two_plane_rule_charged_with_the_fit_pressure() {
    let i = input(2.0, 0.0);
    let r = analyse(&i).unwrap();
    let th = 40f64.to_radians();
    let expect = 2.0 * i.plate.fsu * 0.125 * (i.edge - 0.5 * D * th.cos());
    close(row(&r, "shear_out").handbook.unwrap(), expect);
    // A press fit consumes p D t of it (the same strip equilibrium `AllowableModel` uses).
    let fit = input(2.0, 0.0008);
    let p = fit_pressure(D, 0.0008, &fit.plate, &fit.pin);
    let r = analyse(&fit).unwrap();
    close(r.fit_pressure, p);
    close(row(&r, "shear_out").handbook.unwrap(), expect - p * D * 0.125);
}

#[test]
fn fit_pressure_is_lame_for_a_solid_pin_and_zero_without_interference() {
    let (pl, pn) = (plate(), pin(0.001));
    let p = fit_pressure(D, 0.001, &pl, &pn);
    // Radial displacement of the bore wall plus the pin's: (delta / 2) = p a [(1 + nu_plate) / E_plate + (1 - nu_pin) / E_pin].
    let a = 0.5 * D;
    close(0.5 * 0.001, p * a * ((1.0 + pl.nu) / pl.e + (1.0 - pn.nu) / pn.e));
    assert!(p > 0.0);
    assert_eq!(fit_pressure(D, 0.0, &pl, &pn), 0.0);
    assert_eq!(fit_pressure(D, -0.001, &pl, &pn), 0.0, "a clearance has no pressure");
    // A stiffer pin raises the pressure for the same interference.
    let mut soft = pn;
    soft.e = 10.0e6;
    assert!(fit_pressure(D, 0.001, &pl, &soft) < p);
}

#[test]
fn pin_rows_are_closed_forms_and_say_what_is_missing() {
    let i = input(2.0, 0.0);
    let r = analyse(&i).unwrap();
    close(row(&r, "pin_shear").handbook.unwrap(), 2.0 * 95_000.0 * std::f64::consts::PI * D * D / 4.0);
    close(row(&r, "pin_bearing").handbook.unwrap(), 240_000.0 * D * 0.125);
    // Double shear: M = P L / 8 against Fty pi D^3 / 32.
    close(row(&r, "pin_bending").handbook.unwrap(), 140_000.0 * std::f64::consts::PI * D.powi(3) / 32.0 / (0.5 / 8.0));
    for id in ["pin_shear", "pin_bearing", "pin_bending"] {
        assert_eq!(row(&r, id).group, Group::Pin);
    }
    // A bolt's specification gives no yield or bearing strength: those rows are unavailable, not invented.
    let mut bolt = input(2.0, 0.0);
    bolt.pin.fty = None;
    bolt.pin.fbru = None;
    let r = analyse(&bolt).unwrap();
    assert!(row(&r, "pin_bearing").handbook.is_none() && row(&r, "pin_bearing").why.is_some());
    assert!(row(&r, "pin_bending").handbook.is_none() && row(&r, "pin_bending").why.is_some());
    assert!(row(&r, "pin_shear").handbook.is_some());
    // Single shear carries half the planes.
    let mut single = input(2.0, 0.0);
    single.pin.planes = 1;
    let r1 = analyse(&single).unwrap();
    close(row(&r1, "pin_shear").handbook.unwrap(), 0.5 * row(&r, "pin_shear").handbook.unwrap());
}

#[test]
fn the_elastic_field_gives_shear_out_tear_out_and_first_yield_and_the_lowest_row_governs() {
    let r = analyse(&input(2.0, 0.0)).unwrap();
    let so = row(&r, "shear_out");
    // The elastic mean-shear criterion and the classical planes agree to within the idealisation (both are the
    // Fsu limit of the same strip), so neither is wildly off the other.
    let (h, e) = (so.handbook.unwrap(), so.elastic.unwrap());
    assert!((e / h - 1.0).abs() < 0.5, "handbook {h:.0} vs elastic {e:.0}");
    assert!(row(&r, "first_yield").elastic.unwrap() > 0.0);
    assert!(row(&r, "tear_out").elastic.is_some());
    let g = r.governing.expect("a governing row");
    let lowest = r.rows.iter().filter(|x| x.governs).filter_map(|x| x.lowest()).filter(|v| v.is_finite()).fold(f64::INFINITY, f64::min);
    close(r.rows[g].lowest().unwrap(), lowest);
    assert!(r.rows[g].governs, "informational rows never govern");
}

#[test]
fn the_contact_fe_adds_collapse_and_first_yield_when_asked() {
    let mut i = input(2.0, 0.0);
    i.deep = true;
    let r = analyse(&i).unwrap();
    let c = row(&r, "collapse");
    assert!(c.contact.is_some() && c.why.is_none());
    let fy = row(&r, "first_yield");
    assert!(fy.contact.unwrap() <= c.contact.unwrap(), "first yield comes before collapse");
    // Without the FE the collapse row says how to get it.
    let r = analyse(&input(2.0, 0.0)).unwrap();
    assert!(row(&r, "collapse").contact.is_none() && row(&r, "collapse").why.is_some());
}

#[test]
fn an_interference_pin_lowers_shear_out_and_a_clearance_does_not() {
    let clr = analyse(&input(2.0, -0.001)).unwrap();
    let line = analyse(&input(2.0, 0.0)).unwrap();
    let prs = analyse(&input(2.0, 0.001)).unwrap();
    close(row(&clr, "shear_out").handbook.unwrap(), row(&line, "shear_out").handbook.unwrap());
    assert!(row(&prs, "shear_out").handbook.unwrap() < row(&line, "shear_out").handbook.unwrap());
    assert!(prs.fit_pressure > 0.0 && clr.fit_pressure == 0.0);
}

#[test]
fn bad_inputs_are_refused() {
    let mut i = input(2.0, 0.0);
    i.edge = 0.5 * D;
    assert!(analyse(&i).unwrap_err().contains("edge distance"));
    let mut i = input(2.0, 0.0);
    i.pin.dia = D + 0.01;
    assert!(analyse(&i).unwrap_err().contains("pin diameter"));
    let mut i = input(2.0, 0.0);
    i.pin.planes = 3;
    assert!(analyse(&i).is_err());
    let mut i = input(2.0, 0.0);
    i.thickness = f64::NAN;
    assert!(analyse(&i).is_err());
}
