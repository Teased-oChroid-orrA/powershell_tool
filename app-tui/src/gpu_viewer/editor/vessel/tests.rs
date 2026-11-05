use super::*;
use crate::gpu_viewer::editor::authoring::Authoring;
use crate::gpu_viewer::editor::session::{EditorAction, Key, Mods};
use crate::gpu_viewer::ui::CardRow;
use crate::toolboxes::pressure_vessel::model::NumberTarget;

fn dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("vessel-session-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}
/// A session that never touches the person's own material library.
fn session(tag: &str) -> VesselSession {
    let d = dir(tag);
    let mut s = VesselSession::new(&VesselFile::capture(&PressureVesselModel::default(), "tank"), d.join("case.json"));
    s.library = Some(d.join("materials.json"));
    s
}
fn act(s: &mut VesselSession, a: VesselAct) {
    s.act(EditorAction::Vessel(a));
}
fn type_text(s: &mut VesselSession, text: &str) {
    for c in text.chars() {
        s.key(Key::Char(c), Mods::default());
    }
}
fn edit(s: &mut VesselSession, t: NumberTarget, value: &str) {
    act(s, VesselAct::Row(FieldRow::Number(t)));
    assert!(s.prompting(), "{t:?} opens a typed edit");
    type_text(s, value);
    s.key(Key::Enter, Mods::default());
    assert!(!s.prompting());
}
fn texts(cards: &[Card]) -> String {
    cards.iter().flat_map(|c| std::iter::once(c.title.clone()).chain(c.items.iter().filter_map(|i| i.plain_text()))).collect::<Vec<_>>().join("\n")
}
fn ids(cards: &[Card]) -> Vec<String> {
    cards.iter().map(|c| c.id.clone()).collect()
}
fn choices(card: &ValueCard) -> Vec<String> {
    card.rows.iter().filter_map(|r| if let CardRow::Choice { label, .. } = r { Some(label.clone()) } else { None }).collect()
}

#[test]
fn it_opens_on_the_models_inputs_with_the_checks_and_a_contour() {
    let mut s = session("open");
    assert_eq!(s.title(), "Pressure Vessel | tank");
    assert!(s.benches().is_empty());
    assert_eq!(s.ribbon().into_iter().map(|t| t.title).collect::<Vec<_>>(), ["Inputs", "Results", "Report"]);
    let cards = s.inspector();
    assert_eq!(
        ids(&cards),
        ["vessel-geometry", "vessel-pressure-end-condition", "vessel-material", "vessel-analysis-options", "vessel-result", "vessel-checks", "vessel-thickness", "vessel-wall", "vessel-drawing"]
    );
    let t = texts(&cards);
    // The rows are the model's labels and values; the checks are the solver's.
    assert!(t.contains("Outer Diameter  6.0000 in") && t.contains("End Condition  Closed ends"), "{t}");
    assert!(t.contains("Classification  Thick-wall") && t.contains("Inner diameter  4.0000 in"), "{t}");
    assert!(t.contains("Radial / hoop / axial  -5000 / 13000 / 4000 psi"), "the Shigley bore: {t}");
    assert!(s.status().contains("checks pass") && s.status().contains("governs"), "{}", s.status());
    assert!(s.shows_result() && !s.geometry_view());
    assert_eq!(s.field_unit("Hoop stress"), "psi");
    assert!(s.needs_scene());
    let scene = s.scene().unwrap();
    assert_eq!(scene.field_label, "Hoop stress");
    assert!(scene.positions.len() > 100);
    assert!(!s.needs_scene());
}

#[test]
fn a_typed_value_reaches_the_solver_redraws_and_undo_returns() {
    let mut s = session("edit");
    s.scene().unwrap();
    edit(&mut s, NumberTarget::WallThickness, "0.5");
    assert_eq!(s.model().wall_thickness, 0.5);
    assert!((s.model().geometry.unwrap().inner_radius - 2.5).abs() < 1e-12);
    assert!(s.dirty() && s.title().ends_with('*') && s.needs_scene());
    // Typing the same value again is not an edit; a non-number is refused with a reason.
    let depth = s.undo.len();
    edit(&mut s, NumberTarget::WallThickness, "0.5");
    assert_eq!(s.undo.len(), depth);
    edit(&mut s, NumberTarget::WallThickness, "-");
    assert!(s.status().contains("is not a number"), "{}", s.status());
    // Esc leaves the value alone.
    act(&mut s, VesselAct::Row(FieldRow::Number(NumberTarget::WallThickness)));
    type_text(&mut s, "9");
    s.key(Key::Escape, Mods::default());
    assert_eq!(s.model().wall_thickness, 0.5);
    // A wall thicker than the radius makes no vessel: said, and nothing drawn.
    edit(&mut s, NumberTarget::WallThickness, "4");
    assert!(s.status().starts_with("Invalid geometry"), "{}", s.status());
    assert!(texts(&s.inspector()).contains("Nothing to draw"));
    assert_eq!(s.scene().unwrap().positions.len(), 3);
    s.key(Key::Char('z'), Mods { command: true, ..Mods::default() });
    assert_eq!(s.model().wall_thickness, 0.5);
    s.key(Key::Char('z'), Mods { command: true, ..Mods::default() });
    assert_eq!(s.model().wall_thickness, 1.0);
    s.key(Key::Char('z'), Mods { command: true, shift: true });
    assert_eq!(s.model().wall_thickness, 0.5);
}

#[test]
fn the_end_condition_toggles_and_an_unloaded_vessel_passes() {
    let mut s = session("ends");
    act(&mut s, VesselAct::Row(FieldRow::ToggleEndCondition));
    assert!(!s.model().closed_ends);
    assert!(texts(&s.inspector()).contains("End Condition  Open ends"));
    // No pressure loads no check: every margin is unbounded, and that is a pass (the terminal read "REVIEW 0/4").
    edit(&mut s, NumberTarget::InternalPressure, "0");
    assert!(s.all_pass(), "{:?}", s.model().rows.iter().map(|r| r.margin).collect::<Vec<_>>());
    assert!(s.status().starts_with("PASS"), "{}", s.status());
}

#[test]
fn the_picker_searches_the_catalog_picks_and_adds_a_material_of_ones_own() {
    let mut s = session("pick");
    act(&mut s, VesselAct::Row(FieldRow::OpenMaterialPicker));
    let card = s.value_card().expect("the material picker");
    assert_eq!(card.title, "Material");
    assert_eq!(choices(&card)[0], s.model().material().name, "the current material leads the list");
    assert_eq!(choices(&card).last().unwrap(), "Add a material");
    // The search narrows the list; a pick selects and closes.
    s.card_field(0, "4130".into());
    let card = s.value_card().unwrap();
    let hits = choices(&card);
    // The shared filter (`material_detail::matches`): the best matches lead.
    assert!(hits.len() >= 2 && hits[0].contains("4130"), "{hits:?}");
    let i = s.model().material_catalog().iter().position(|m| m.name == hits[0]).unwrap();
    act(&mut s, VesselAct::PickMaterial(i));
    assert!(s.value_card().is_none());
    assert_eq!(s.model().material().name, hits[0]);
    assert_eq!(s.undo.len(), 1);
    // A material of one's own: a bad form says why, a good one is added, selected and kept in the library file.
    act(&mut s, VesselAct::Row(FieldRow::OpenMaterialPicker));
    act(&mut s, VesselAct::NewMaterial);
    assert_eq!(s.value_card().unwrap().title, "New material");
    act(&mut s, VesselAct::AddMaterial);
    assert!(s.value_card().unwrap().rows.iter().any(|r| matches!(r, CardRow::Text(t) if t.starts_with("Not added"))));
    for (k, v) in ["Test steel", "29000", "100", "120", "0.3", "6.5"].into_iter().enumerate() {
        s.card_field(k + 1, v.into());
    }
    act(&mut s, VesselAct::AddMaterial);
    assert!(s.value_card().is_none(), "{}", s.status());
    assert_eq!(s.model().material().name, "Test steel");
    assert!(texts(&s.inspector()).contains("Material  Test steel"));
    let kept = std::fs::read_to_string(s.library.as_ref().unwrap()).unwrap();
    assert!(kept.contains("Test steel"), "{kept}");
    // Esc leaves the form, then the picker.
    act(&mut s, VesselAct::Row(FieldRow::OpenMaterialPicker));
    act(&mut s, VesselAct::NewMaterial);
    s.key(Key::Escape, Mods::default());
    assert_eq!(s.value_card().unwrap().title, "Material");
    s.key(Key::Escape, Mods::default());
    assert!(s.value_card().is_none());
}

#[test]
fn save_and_reopen_round_trips_every_input_and_a_custom_material_and_export_writes_the_report() {
    let mut s = session("save");
    edit(&mut s, NumberTarget::CorrosionAllowance, "0.125");
    edit(&mut s, NumberTarget::MillUndertolerance, "12.5");
    edit(&mut s, NumberTarget::ExternalPressure, "100");
    edit(&mut s, NumberTarget::UnsupportedLength, "24");
    edit(&mut s, NumberTarget::TemperatureDifferential, "50");
    act(&mut s, VesselAct::Row(FieldRow::ToggleEndCondition));
    s.model.add_custom_material("Saved alloy".into(), 10000.0, 50.0, 60.0, 0.33, 12.0);
    act(&mut s, VesselAct::Save);
    assert!(!s.dirty(), "{}", s.status());
    let back = VesselSession::open(&s.path).unwrap();
    assert_eq!(VesselFile::capture(back.model(), "tank"), VesselFile::capture(s.model(), "tank"));
    assert_eq!(back.model().material().name, "Saved alloy", "the material travels with the case");
    act(&mut s, VesselAct::Export);
    let report = std::fs::read_to_string(s.path.with_extension("report.txt")).unwrap();
    assert_eq!(report, report::build_report_text(s.model()));
    assert!(report.contains("Corrosion allowance: 0.1250 in") && report.contains("Mill undertolerance: 12.50 %"), "{report}");
    assert!(report.contains("Remaining wall thickness: 0.7500 in") && report.contains("Minimum nominal wall thickness with allowances"), "{report}");
    // A file another program changed is not overwritten.
    crate::persistence::write(&s.path, "{}").unwrap();
    edit(&mut s, NumberTarget::ExternalPressure, "200");
    act(&mut s, VesselAct::Save);
    assert!(s.status().starts_with("Save conflict"), "{}", s.status());
}

#[test]
fn allowances_drive_the_checks_contour_minimum_wall_and_undo() {
    use pressure_vessel_solver::{failure::evaluate_failure_modes, geometry::CylinderGeometry, thickness::ThicknessSolverOutcome};
    let mut s = session("allowances");
    let before = s.scene().unwrap().range;
    let before_margin = s.model.rows[0].margin;
    edit(&mut s, NumberTarget::CorrosionAllowance, "0.125");
    edit(&mut s, NumberTarget::MillUndertolerance, "12.5");
    let g = s.model.geometry.unwrap();
    assert_eq!(g, CylinderGeometry::new(2.25, 3.0).unwrap());
    assert_eq!(s.model.rows, evaluate_failure_modes(&g, &s.model.pressure.unwrap(), s.model.material()));
    assert!(s.model.rows[0].margin < before_margin);
    assert!(s.needs_scene());
    let scene = s.scene().unwrap();
    assert!(scene.range[1] > before[1]);
    let expected_hoop = 5000.0 * (3.0_f64.powi(2) + 2.25_f64.powi(2)) / (3.0_f64.powi(2) - 2.25_f64.powi(2));
    assert!((scene.range[1] as f64 - expected_hoop).abs() < 0.5);
    let ThicknessSolverOutcome::Converged(sol) = s.model.thickness_outcome.as_ref().unwrap() else { panic!("minimum wall") };
    let nominal = s.model.wall_allowance().unwrap().nominal_thickness(sol.wall_thickness);
    assert!((nominal * 0.875 - 0.125 - sol.wall_thickness).abs() < 1e-12);
    assert!(sol.governing.margin >= s.model.required_ms);
    let mut candidate = PressureVesselModel::default();
    candidate.outer_diameter = 2.0 * sol.outer_radius;
    candidate.wall_thickness = nominal;
    candidate.corrosion_allowance = 0.125;
    candidate.mill_undertolerance = 12.5;
    candidate.recompute();
    assert!((candidate.geometry.unwrap().inner_radius - g.inner_radius).abs() < 1e-12);
    assert!(candidate.rows.iter().all(|r| r.margin >= s.model.required_ms));
    let text = texts(&s.inspector());
    assert!(text.contains("Remaining wall  0.7500 in") && text.contains("Minimum nominal wall"), "{text}");
    act(&mut s, VesselAct::Undo);
    assert_eq!(s.model.mill_undertolerance, 0.0);
    assert_eq!(s.model.geometry.unwrap().wall_thickness(), 0.875);
    act(&mut s, VesselAct::Undo);
    assert_eq!(s.model.corrosion_allowance, 0.0);
    assert_eq!(s.scene().unwrap().range, before);
    act(&mut s, VesselAct::Redo);
    assert_eq!(s.model.corrosion_allowance, 0.125);
}

#[test]
fn invalid_allowances_clear_results_and_old_cases_reset_losses_to_zero() {
    let mut s = session("invalid-allowances");
    let mut old = s.snapshot();
    assert_eq!(old.version, 2, "older readers must reject cases whose wall losses they cannot apply");
    old.version = 1;
    old.numbers.remove("corrosion allowance");
    old.numbers.remove("mill undertolerance percent");
    let old = VesselFile::from_json(&old.to_json()).unwrap();
    for (target, value) in [(NumberTarget::CorrosionAllowance, "-1"), (NumberTarget::CorrosionAllowance, "1"), (NumberTarget::MillUndertolerance, "-1"), (NumberTarget::MillUndertolerance, "100")] {
        old.apply(&mut s.model);
        edit(&mut s, target, value);
        assert!(s.model.geometry.is_err() && s.model.rows.is_empty() && s.model.thickness_outcome.is_none());
        assert_eq!(s.scene().unwrap().positions.len(), 3);
        assert!(target.validation_hint(&s.model).is_some());
        assert!(report::build_report_text(&s.model).contains("Invalid geometry"));
    }
    old.apply(&mut s.model);
    assert_eq!(s.model.corrosion_allowance, 0.0);
    assert_eq!(s.model.mill_undertolerance, 0.0);
    assert_eq!(s.model.geometry.unwrap().wall_thickness(), 1.0);
    assert!(!s.model.rows.is_empty());
}

#[test]
fn a_file_with_an_unknown_material_applies_the_rest_and_says_what_it_skipped() {
    let mut f = VesselFile::capture(&PressureVesselModel::default(), "x");
    f.material = "Unobtainium".into();
    f.numbers.insert("outer diameter".into(), 8.0);
    let s = VesselSession::new(&f, dir("skip").join("case.json"));
    assert_eq!(s.model().outer_diameter, 8.0);
    assert!(s.status().contains("material: Unobtainium"), "{}", s.status());
    assert!(VesselFile::from_json("{\"version\": 99}").unwrap_err().contains("newer version"));
}
