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
        ["vessel-geometry", "vessel-pressure-end-condition", "vessel-material", "vessel-analysis-options", "vessel-head-shell-code-check-ug-27", "vessel-line-run", "vessel-requirements", "vessel-allowables", "vessel-walls", "vessel-result", "vessel-checks", "vessel-thickness", "vessel-wall", "vessel-heads", "vessel-drawing"]
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
    assert_eq!(old.version, 5, "older readers must reject cases whose wall losses they cannot apply");
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

#[test]
fn the_code_check_matches_the_hand_value_follows_the_wall_and_round_trips() {
    use crate::toolboxes::pressure_vessel::model::HeadComponent;
    let mut s = session("heads");
    // R 24 in, S 20000 psi, E 1, P 300 psi, cylinder hoop: t = 7200 / 19820.
    act(&mut s, VesselAct::Row(FieldRow::ToggleHeadComponent));
    act(&mut s, VesselAct::Row(FieldRow::ToggleHeadComponent));
    act(&mut s, VesselAct::Row(FieldRow::ToggleHeadComponent));
    assert_eq!(s.model.head.component, HeadComponent::CylinderHoop);
    let t = s.model.heads.required.clone().unwrap();
    assert!((t - 7200.0 / 19_820.0).abs() < 1e-12, "{t}");
    // The provided thickness is the vessel's remaining wall: 1 in nominal, 12.5 % mill loss, 0.125 in corrosion.
    edit(&mut s, NumberTarget::CorrosionAllowance, "0.125");
    edit(&mut s, NumberTarget::MillUndertolerance, "12.5");
    assert!((s.model.heads.provided.unwrap() - 0.75).abs() < 1e-12);
    let m = s.model.heads.margin().unwrap();
    assert!((m - (0.75 / t - 1.0)).abs() < 1e-12 && m > 0.0);
    let text = texts(&s.inspector());
    assert!(text.contains("Required thickness at 300 psi  0.3633 in") && text.contains("Provided (remaining wall)  0.7500 in"), "{text}");
    assert!(text.contains("adequate") && text.contains("MAWP"), "{text}");
    // Undo reaches the component and the numbers; the case file keeps them.
    edit(&mut s, NumberTarget::HeadPressure, "500");
    assert_eq!(s.model.head.pressure, 500.0);
    act(&mut s, VesselAct::Save);
    let back = VesselSession::open(&s.path).unwrap();
    assert_eq!(back.model().head, s.model().head);
    act(&mut s, VesselAct::Undo);
    assert_eq!(s.model.head.pressure, 300.0);
    // The report states the check.
    let report = report::build_report_text(s.model());
    assert!(report.contains("Cylinder, hoop stress") && report.contains("Required thickness: 0.3633 in"), "{report}");
}

#[test]
fn a_case_outside_the_code_rules_shows_the_reason_not_a_number() {
    let mut s = session("heads-outside");
    act(&mut s, VesselAct::Row(FieldRow::ToggleHeadComponent)); // torispherical
    edit(&mut s, NumberTarget::HeadKnuckleRadius, "2");
    assert!(s.model.heads.required.is_err());
    let text = texts(&s.inspector());
    assert!(text.contains("Outside the code rules") && text.contains("16.67"), "{text}");
    assert!(!text.contains("Required thickness at"), "{text}");
    assert!(NumberTarget::HeadEfficiency.validation_hint(&s.model).is_none());
    edit(&mut s, NumberTarget::HeadEfficiency, "1.5");
    assert!(NumberTarget::HeadEfficiency.validation_hint(&s.model).is_some());
    assert!(texts(&s.inspector()).contains("Invalid input"));
}

#[test]
fn an_older_case_gets_the_default_code_check_and_a_newer_component_is_reported() {
    let mut s = session("heads-old");
    s.model.head.pressure = 999.0;
    let mut old = s.snapshot();
    old.version = 2;
    old.head_component = None;
    old.numbers.retain(|k, _| !k.starts_with("head "));
    old.apply(&mut s.model);
    assert_eq!(s.model.head, crate::toolboxes::pressure_vessel::model::HeadInputs::default());
    let mut f = s.snapshot();
    f.head_component = Some("Conical head".into());
    assert!(f.apply(&mut s.model).iter().any(|m| m.contains("Conical head")));
}

// ---------------------------------------------------------------------- minimum wall by requirement and the line run

use crate::gpu_viewer::page::Cell;
use crate::toolboxes::pressure_vessel::edit::{Edit, LineField, ReqField};

/// Run the worker to its answer (the window's `poll` does this each frame).
fn settle(s: &mut VesselSession) {
    let t = std::time::Instant::now();
    while s.model.walls.is_none() && s.model.line.enabled && s.model.geometry.is_ok() {
        s.poll();
        assert!(t.elapsed().as_secs() < 60, "the wall solve did not finish");
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}
fn table(s: &VesselSession) -> crate::gpu_viewer::page::DockTable {
    match s.requirements_table(false) {
        DockItem::Table(t) => *t,
        _ => unreachable!(),
    }
}
fn cell_text(c: &Cell) -> String {
    match c {
        Cell::Text { text, .. } | Cell::Wall { text, .. } | Cell::Chip { text, .. } => text.clone(),
        Cell::Button { .. } => String::new(),
    }
}

#[test]
fn the_cylinder_table_is_seeded_neutral_and_a_typed_factor_moves_the_wall_and_undoes() {
    let mut s = session("walls");
    let t = table(&s);
    assert_eq!(t.rows.len(), s.model.requirements.len());
    assert!(t.hint.contains("enter your program's factors"), "{}", t.hint);
    assert!(t.rows.iter().filter(|r| r.governing).count() == 1);
    let before = t.rows[0].cells[4].clone();
    act(&mut s, VesselAct::Edit(Edit::Req(0, ReqField::Factor)));
    assert!(s.prompting());
    type_text(&mut s, "2");
    s.key(Key::Enter, Mods::default());
    assert_eq!(s.model.requirements[0].factor, 2.0);
    assert!(s.model.requirements[0].edited && s.dirty());
    let after = table(&s).rows[0].cells[4].clone();
    assert_ne!(before, after, "the minimum wall follows the factor at once");
    assert!(table(&s).footer.contains("Governing wall"), "{}", table(&s).footer);
    s.undo();
    assert_eq!(table(&s).rows[0].cells[4], before);
}

#[test]
fn a_label_is_typed_as_text_and_rows_are_added_cycled_and_removed_with_undo() {
    let mut s = session("rows");
    let n = s.model.requirements.len();
    act(&mut s, VesselAct::AddRequirement);
    assert_eq!(s.model.requirements.len(), n + 1);
    act(&mut s, VesselAct::Edit(Edit::Req(n, ReqField::Label)));
    type_text(&mut s, " proof");
    s.key(Key::Enter, Mods::default());
    assert!(s.model.requirements[n].label.ends_with(" proof"), "{}", s.model.requirements[n].label);
    let first = s.model.requirements[n].criterion;
    act(&mut s, VesselAct::CycleRequirement(n));
    assert_ne!(s.model.requirements[n].criterion, first);
    act(&mut s, VesselAct::RemoveRequirement(n));
    assert_eq!(s.model.requirements.len(), n);
    for _ in 0..4 {
        s.undo();
    }
    assert_eq!(s.model.requirements.len(), n);
    assert!(s.undo.is_empty());
}

#[test]
fn a_changed_material_reseeds_only_the_rows_the_person_left_alone() {
    let mut s = session("reseed");
    act(&mut s, VesselAct::Edit(Edit::Req(0, ReqField::RequiredMs)));
    type_text(&mut s, "0.25");
    s.key(Key::Enter, Mods::default());
    let name = s.model.requirements[0].label.clone();
    let other = (0..s.model.material_catalog().len()).find(|&i| i != s.model.material_index).unwrap();
    act(&mut s, VesselAct::PickMaterial(other));
    let row = s.model.requirements.iter().find(|r| r.label == name).expect("an edited row survives");
    assert_eq!(row.required_ms, 0.25);
}

#[test]
fn the_case_file_keeps_rows_and_the_line_run_and_an_older_file_loads_seeded() {
    let mut s = session("persist");
    act(&mut s, VesselAct::AddRequirement);
    act(&mut s, VesselAct::ToggleLineRun);
    act(&mut s, VesselAct::AddLoad);
    settle(&mut s);
    let file = s.snapshot();
    assert_eq!(file.version, 5);
    let reopened = VesselFile::from_json(&file.to_json()).unwrap();
    let mut m = PressureVesselModel::default();
    assert!(reopened.apply(&mut m).is_empty());
    assert_eq!(m.requirements, s.model.requirements);
    assert_eq!(m.line, s.model.line);
    assert!(m.line.enabled && m.line.loads.len() == 1);
    // A format 3 case has none of it: seeds and the default run, also over an edited model.
    let old = VesselFile::from_json(r#"{"version":3,"name":"old","numbers":{"wall thickness":0.5},"material":""}"#).unwrap();
    old.apply(&mut m);
    assert_eq!(m.wall_thickness, 0.5);
    assert!(!m.line.enabled && m.line.loads.is_empty());
    assert_eq!(m.requirements, crate::toolboxes::pressure_vessel::requirements::seeds(m.kind()));
}

#[test]
fn the_line_run_page_shows_the_drawing_the_table_and_a_balanced_chip_once_the_walls_arrive() {
    let mut s = session("line");
    assert!(s.page().is_empty(), "the cylinder keeps the 3D picture");
    act(&mut s, VesselAct::ToggleLineRun);
    assert!(s.solving() || s.model.walls.is_some());
    settle(&mut s);
    let page = s.page();
    let text = page.iter().filter_map(|i| i.plain_text()).collect::<Vec<_>>().join("\n");
    assert!(text.contains("Sum of reactions") && text.contains("Case 1") && text.contains("Export report"), "{text}");
    assert!(page.iter().any(|i| matches!(i, DockItem::Run(d) if d.curve.len() > 2 && d.worst.is_some())));
    let t = table(&s);
    assert!(t.rows.iter().any(|r| matches!(&r.cells[4], Cell::Wall { .. })), "{:?}", t.rows.iter().map(|r| cell_text(&r.cells[4])).collect::<Vec<_>>());
    assert!(t.footer.contains("Governing wall"));
    let cards = texts(&s.inspector());
    assert!(cards.contains("Reactions") && cards.contains("Governing") && cards.contains("Tube"), "{cards}");
    // Moving a support re-solves: the stale answer is dropped and the new one arrives.
    act(&mut s, VesselAct::Edit(Edit::Line(LineField::SupportX(1))));
    type_text(&mut s, "40");
    s.key(Key::Enter, Mods::default());
    assert!(s.model.walls.is_none(), "the wall solve for the new run is pending");
    settle(&mut s);
    assert!(s.model.walls.is_some());
    // A point load shows on the drawing and the reactions still balance it.
    act(&mut s, VesselAct::AddLoad);
    settle(&mut s);
    assert!(s.model.equilibrium().unwrap().balanced());
}

fn page_text(s: &VesselSession) -> String {
    // Headings and the drawing's note (where a refused run says why) carry no plain text of their own; the page tests
    // read them too.
    s.page().iter().filter_map(|i| match i {
        DockItem::Heading(t) => Some(t.clone()),
        DockItem::Run(d) => Some(d.note.clone()),
        i => i.plain_text(),
    }).collect::<Vec<_>>().join("\n")
}

#[test]
fn segment_mode_takes_end_conditions_a_valve_and_the_bound_and_shows_both_results() {
    let mut s = session("segment");
    act(&mut s, VesselAct::ToggleLineRun);
    act(&mut s, VesselAct::ToggleSegment);
    act(&mut s, VesselAct::AddLoad);
    act(&mut s, VesselAct::Edit(Edit::Line(LineField::LoadWeight(0))));
    type_text(&mut s, "150");
    s.key(Key::Enter, Mods::default());
    settle(&mut s);
    let cards = texts(&s.inspector());
    assert!(cards.contains("Segment between two supports") && cards.contains("Left end (x = 0)") && cards.contains("Condition"), "{cards}");
    assert!(page_text(&s).contains("A cut segment is an idealisation"), "the page says the cut is an idealisation");
    assert!(!page_text(&s).contains("Segment bound"), "no bound until asked");
    // Bound on: both results are on the page, with the worse one named.
    act(&mut s, VesselAct::ToggleBound);
    settle(&mut s);
    let page = page_text(&s);
    assert!(page.contains("Segment bound: pinned and clamped"), "{page}");
    let bound = s.page().into_iter().find_map(|i| match i {
        DockItem::Table(t) if t.columns.iter().any(|c| c.title == "Clamped") => Some(t),
        _ => None,
    });
    let bound = bound.expect("the bound table");
    assert_eq!(bound.rows.len(), 3);
    assert!(bound.rows.iter().all(|r| cell_text(&r.cells[3]).contains("pinned") || cell_text(&r.cells[3]).contains("clamped")));
    assert!(texts(&s.inspector()).contains("Segment bound"), "the Results card");
    // A free end has no bound to bracket: the page says why instead of showing a stale result.
    act(&mut s, VesselAct::CycleFixity(1)); // Pinned -> Spring
    act(&mut s, VesselAct::CycleFixity(1)); // -> Clamped
    act(&mut s, VesselAct::CycleFixity(1)); // -> Free
    settle(&mut s);
    assert!(page_text(&s).contains("The bound needs both ends held"), "{}", page_text(&s));
    assert!(s.model.bounds.is_none());
    // The valve at the free end, then the bound off: a clamp-and-valve segment that solves.
    act(&mut s, VesselAct::ToggleBound);
    act(&mut s, VesselAct::CycleFixity(0)); // Pinned -> Spring
    act(&mut s, VesselAct::CycleFixity(0)); // -> Clamped
    act(&mut s, VesselAct::AddEndLoad(1));
    assert_eq!(s.model.line.loads.last().map(|l| l.x), Some(s.model.line.length));
    settle(&mut s);
    assert!(matches!(s.model.line_result, Some(Ok(_))), "{:?}", s.model.line_result);
    // The valve and each end condition are one undo step each.
    let (loads, end) = (s.model.line.loads.len(), s.model.line.segment.ends[0].fixity);
    act(&mut s, VesselAct::Undo);
    assert_eq!(s.model.line.loads.len(), loads - 1);
    act(&mut s, VesselAct::Undo);
    assert_ne!(s.model.line.segment.ends[0].fixity, end);
}

#[test]
fn the_case_file_keeps_the_segment_and_a_format_4_case_loads_without_one() {
    let mut s = session("segment-file");
    act(&mut s, VesselAct::ToggleLineRun);
    act(&mut s, VesselAct::ToggleSegment);
    act(&mut s, VesselAct::CycleFixity(0));
    act(&mut s, VesselAct::CycleFixity(0)); // Pinned -> Spring -> Clamped
    act(&mut s, VesselAct::CycleFixity(1)); // Pinned -> Spring
    act(&mut s, VesselAct::Edit(Edit::Line(LineField::EndSpring(1))));
    type_text(&mut s, "250000");
    s.key(Key::Enter, Mods::default());
    act(&mut s, VesselAct::ToggleBound);
    settle(&mut s);
    let file = s.snapshot();
    let json = file.to_json();
    let mut m = PressureVesselModel::default();
    assert!(VesselFile::from_json(&json).unwrap().apply(&mut m).is_empty());
    assert_eq!(m.line.segment, s.model.line.segment);
    assert_eq!((m.line.segment.ends[0].fixity, m.line.segment.ends[1].spring, m.line.segment.bound), (crate::toolboxes::pressure_vessel::line::FixityIn::Clamped, 250000.0, true));
    // A format 4 case (a line run, no segment key) opens as the whole run.
    let mut v4 = serde_json::from_str::<serde_json::Value>(&json).unwrap();
    v4["version"] = 4.into();
    v4["line_run"].as_object_mut().unwrap().remove("segment");
    let old = VesselFile::from_json(&v4.to_string()).unwrap();
    old.apply(&mut m);
    assert!(m.line.enabled && !m.line.segment.enabled && !m.line.segment.bound);
    assert_eq!(m.line.segment, crate::toolboxes::pressure_vessel::line::SegmentIn::default());
    // A case from a newer build is refused, not misread.
    let mut v6 = serde_json::from_str::<serde_json::Value>(&json).unwrap();
    v6["version"] = 6.into();
    assert!(VesselFile::from_json(&v6.to_string()).is_err());
}

#[test]
fn the_report_states_the_segment_and_both_bounds() {
    let mut s = session("segment-report");
    act(&mut s, VesselAct::ToggleLineRun);
    act(&mut s, VesselAct::ToggleSegment);
    act(&mut s, VesselAct::ToggleBound);
    settle(&mut s);
    let text = crate::toolboxes::pressure_vessel::report::build_report_text(&s.model);
    assert!(text.contains("Segment between two supports") && text.contains("idealisation") && text.contains("worse margin per requirement"), "{text}");
    assert!(text.contains("pinned") && text.contains("clamped") && text.contains("worse"), "{text}");
}
