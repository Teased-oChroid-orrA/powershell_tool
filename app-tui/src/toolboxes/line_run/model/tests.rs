use super::*;

/// OD 1, wall 0.05, 100 in simply supported, contents 2 lbf/in, weightless tube, one g down.
fn beam(units: Units) -> LineRunModel {
    let mut m = LineRunModel::default();
    m.inputs.units = units;
    for (f, v) in [
        (Field::Od, 1.0),
        (Field::Wall, 0.05),
        (Field::XEnd, 100.0),
        (Field::SupportX(1), 100.0),
        (Field::Contents, 2.0),
        (Field::E, 10.0e6),
        (Field::Alpha, 12.8e-6),
        (Field::Density, 0.0),
        (Field::Fty, 40000.0),
        (Field::Ftu, 60000.0),
        (Field::FsYield, 1.0),
        (Field::FsUltimate, 1.5),
    ] {
        // Typed in the shown units: convert the US numbers to what the box would show.
        assert!(m.set(f, v * units.factor(f.qty())));
    }
    m
}

fn peak_moment(m: &LineRunModel) -> f64 {
    m.outcome.as_ref().unwrap().cases[0].stations.iter().map(|s| s.resultant_moment()).fold(0.0, f64::max)
}

#[test]
fn a_new_model_has_no_result_until_the_material_constants_and_allowables_are_entered() {
    let m = LineRunModel::default();
    let why = m.outcome.as_ref().unwrap_err();
    assert!(why.contains("elastic modulus") && why.contains("never defaulted"), "{why}");
    assert!(m.inputs.fty.is_none() && m.inputs.ftu.is_none() && m.inputs.fs_yield.is_none() && m.inputs.fs_ultimate.is_none());
}

#[test]
fn the_window_inputs_reproduce_the_simply_supported_closed_form() {
    // wL^2/8 = 2 * 100^2 / 8 = 2500 in.lbf at midspan.
    let m = beam(Units::Us);
    let peak = peak_moment(&m);
    assert!((peak - 2500.0).abs() < 1e-6, "{peak}");
    assert!(m.outcome.as_ref().unwrap().cases[0].residual < 1e-9);
}

#[test]
fn si_entry_converts_once_and_gives_the_same_solve() {
    let (us, si) = (beam(Units::Us), beam(Units::Si));
    let rel = |a: f64, b: f64| (a - b).abs() / b.abs();
    assert!(rel(si.inputs.od, us.inputs.od) < 1e-12 && rel(si.inputs.e.unwrap(), 10.0e6) < 1e-12 && rel(si.inputs.contents, 2.0) < 1e-12);
    assert!(rel(peak_moment(&si), peak_moment(&us)) < 1e-9);
    // Shown back in SI: 1 in is 25.4 mm, 10e6 psi is 68947.57 MPa, 2 lbf/in is 0.35025 N/mm.
    assert!((si.shown(Field::Od).unwrap() - 25.4).abs() < 1e-9);
    assert!((si.shown(Field::E).unwrap() - 68947.572_931_68).abs() < 1e-4);
    assert!((si.shown(Field::Contents).unwrap() - 0.350_25).abs() < 1e-4);
    assert_eq!(Units::Si.text(Qty::Length, 0.5), "12.7 mm");
}

#[test]
fn typing_the_current_value_is_not_an_edit_and_junk_is_refused() {
    let mut m = beam(Units::Us);
    assert!(m.same(Field::Od, 1.0) && !m.same(Field::Od, 1.1));
    assert!(m.same(Field::Fcu, 1.0) == false, "an unset constant has no current value");
    assert!(!m.set(Field::Od, f64::NAN) && !m.set(Field::SupportX(9), 3.0));
    assert_eq!(m.inputs.od, 1.0);
}

#[test]
fn a_brittle_class_needs_its_own_allowables() {
    let mut m = beam(Units::Us);
    m.toggle_class();
    assert!(m.outcome.as_ref().unwrap_err().contains("compressive ultimate"));
    m.set(Field::Fcu, 80000.0);
    m.set(Field::FsBrittle, 2.0);
    let r = m.outcome.as_ref().unwrap();
    assert_eq!(r.cases[0].margins.len(), 2, "Rankine and Coulomb-Mohr");
}

#[test]
fn removing_a_case_drops_its_loads_and_renumbers_the_rest() {
    let mut m = beam(Units::Us);
    m.add_case();
    m.add_case();
    for c in [None, Some(0), Some(1), Some(2)] {
        m.add_load();
        m.inputs.loads.last_mut().unwrap().case = c;
    }
    m.remove_case(1);
    let scopes: Vec<_> = m.inputs.loads.iter().map(|l| l.case).collect();
    assert_eq!(scopes, vec![None, Some(0), Some(1)]);
    assert_eq!(m.inputs.cases.len(), 2);
    assert!(m.inputs.selected < 2);
    m.remove_case(0);
    m.remove_case(0);
    assert_eq!(m.inputs.cases.len(), 1, "the last case stays");
}

#[test]
fn a_case_load_acts_only_in_its_case() {
    // 100 lbf down at midspan in case 2 only: its peak moment is wL^2/8 + PL/4, case 1 stays wL^2/8.
    let mut m = beam(Units::Us);
    m.add_case();
    m.add_load();
    let l = m.inputs.loads.last_mut().unwrap();
    l.force = [0.0, 0.0, -100.0];
    l.case = Some(1);
    m.recompute();
    let r = m.outcome.as_ref().unwrap();
    let peak = |k: usize| r.cases[k].stations.iter().map(|s| s.resultant_moment()).fold(0.0, f64::max);
    assert!((peak(0) - 2500.0).abs() < 1e-6, "{}", peak(0));
    assert!((peak(1) - (2500.0 + 100.0 * 100.0 / 4.0)).abs() < 1e-6, "{}", peak(1));
}

#[test]
fn a_mechanism_is_reported_as_the_reason_not_a_number() {
    let mut m = beam(Units::Us);
    m.remove_support(1);
    m.remove_support(0);
    assert!(m.outcome.is_err());
}

#[test]
fn the_report_and_the_station_table_use_the_shown_units() {
    let si = beam(Units::Si);
    let text = crate::toolboxes::line_run::report::build_report_text(&si);
    assert!(text.contains("Units: mm, N, MPa") && text.contains("OD 25.4 mm") && text.contains("Governing:"), "{text}");
    let csv = crate::toolboxes::line_run::report::stations_csv(&si).unwrap();
    let head = csv.lines().next().unwrap();
    assert!(head.contains("x (mm)") && head.contains("moment (N\u{b7}mm)"), "{head}");
    let peak: f64 = csv.lines().skip(1).map(|l| l.split(',').nth(4).unwrap().parse::<f64>().unwrap()).fold(0.0, f64::max);
    assert!((peak - 2500.0 * 4.448_221_615_260_5 * 25.4).abs() < 1.0, "{peak}");
    assert!(crate::toolboxes::line_run::report::stations_csv(&LineRunModel::default()).is_err());
}

#[test]
fn inputs_round_trip_through_json_and_an_old_file_keeps_unset_constants_unset() {
    let m = beam(Units::Si);
    let back: Inputs = serde_json::from_str(&serde_json::to_string(&m.inputs).unwrap()).unwrap();
    assert_eq!(back, m.inputs);
    let sparse: Inputs = serde_json::from_str(r#"{"od": 0.75}"#).unwrap();
    assert_eq!(sparse.od, 0.75);
    assert!(sparse.fty.is_none() && sparse.e.is_none());
}
