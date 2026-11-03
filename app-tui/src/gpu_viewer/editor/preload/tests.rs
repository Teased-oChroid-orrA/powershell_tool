use super::*;
use crate::gpu_viewer::editor::authoring::Authoring;
use crate::gpu_viewer::editor::session::{EditorAction, Key, Mods};
use crate::gpu_viewer::ui::CardRow;
use crate::toolboxes::preload_analysis::{joint_templates, model::NumberTarget};

fn dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("preload-session-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}
fn session(tag: &str) -> PreloadSession {
    PreloadSession::new(&PreloadFile::capture(&PreloadModel::default(), "lap joint"), dir(tag).join("case.json"))
}
fn act(s: &mut PreloadSession, a: PreloadAct) {
    s.act(EditorAction::Preload(a));
}
fn type_text(s: &mut PreloadSession, text: &str) {
    for c in text.chars() {
        s.key(Key::Char(c), Mods::default());
    }
}
fn edit(s: &mut PreloadSession, t: NumberTarget, value: &str) {
    act(s, PreloadAct::Row(FieldRow::Number(t)));
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
/// Poll until the member FE has answered for the joint shown (the window polls every 250 ms).
fn wait_for_fe(s: &mut PreloadSession) {
    let start = Instant::now();
    while s.fe_current().is_none() {
        assert!(start.elapsed() < Duration::from_secs(60), "the member FE never answered");
        s.poll();
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn it_opens_on_the_terminal_forms_inputs_draws_the_parts_then_the_fe_stress() {
    let mut s = session("open");
    assert_eq!(s.title(), "Preload Analysis | lap joint");
    assert!(s.benches().is_empty());
    assert_eq!(s.ribbon().into_iter().map(|t| t.title).collect::<Vec<_>>(), ["Inputs", "Results", "Report"]);
    let cards = s.inspector();
    assert_eq!(
        ids(&cards),
        [
            "preload-tightening",
            "preload-friction",
            "preload-bearing-geometry",
            "preload-fastener",
            "preload-joint-stack",
            "preload-service-load-slip",
            "preload-material-strength-limits",
            "preload-advanced-settings",
            "preload-joint",
            "preload-result",
            "preload-torque",
            "preload-stress",
            "preload-deformation",
            "preload-rotation",
            "preload-fe",
            "preload-drawing"
        ]
    );
    let t = texts(&cards);
    // The rows are the terminal form's labels and values, worded for a click; the results are the solver's.
    assert!(t.contains("Analysis Mode  Torque Controlled") && t.contains("Bolt (AN Standard)  AN6"), "{t}");
    assert!(t.contains("Joint Template  1 member") && t.contains("Add Joint Member  Add (1/6)") && t.contains("Advanced settings  Show"), "{t}");
    let preload = s.model().output.as_ref().unwrap().preload;
    assert!(t.contains(&format!("Preload  {preload:.0} lbf")) && t.contains("Solver  CONVERGED"), "{t}");
    // No strength limit, service load or slip check is on, so there is no margin to count.
    assert!(t.contains("PASS  no margins checked") && !t.contains("0 / 0"), "{t}");
    assert!(s.status().starts_with("PASS: preload"), "{}", s.status());
    assert_eq!(s.field_unit("Axial stress"), "psi");
    // The first member solve starts at once; until it answers the picture is the parts in the colour key.
    assert!(s.fe_running() || s.fe_current().is_some());
    if s.fe_current().is_none() {
        assert!(s.geometry_view() && !s.shows_result());
        assert_eq!(s.scene().unwrap().field_label, "part (see color key)");
        assert!(texts(&s.inspector()).contains("coloured by part"));
    }
    wait_for_fe(&mut s);
    assert!(s.shows_result() && !s.geometry_view());
    assert!(s.needs_scene(), "the answer redraws");
    let scene = s.scene().unwrap();
    assert_eq!(scene.field_label, "Axial stress");
    assert!(!s.needs_scene());
    let t = texts(&s.inspector());
    assert!(t.contains("FE C_m") && t.contains("Equivalent cone") && t.contains("as the finite-element model solved them"), "{t}");
}

#[test]
fn a_typed_value_reaches_the_solver_and_undo_returns() {
    let mut s = session("edit");
    s.scene().unwrap();
    let before = s.model().output.as_ref().unwrap().preload;
    edit(&mut s, NumberTarget::AppliedTorque, "480");
    assert_eq!(s.model().applied_torque, 480.0);
    assert!(s.model().output.as_ref().unwrap().preload > before);
    assert!(s.dirty() && s.title().ends_with('*') && s.needs_scene());
    // Typing the same value again is not an edit; a non-number is refused with a reason.
    let depth = s.undo.len();
    edit(&mut s, NumberTarget::AppliedTorque, "480");
    assert_eq!(s.undo.len(), depth);
    edit(&mut s, NumberTarget::AppliedTorque, "-");
    assert!(s.status().contains("is not a number"), "{}", s.status());
    // Esc leaves the value alone.
    act(&mut s, PreloadAct::Row(FieldRow::Number(NumberTarget::AppliedTorque)));
    type_text(&mut s, "9");
    s.key(Key::Escape, Mods::default());
    assert_eq!(s.model().applied_torque, 480.0);
    // A bearing face inside the hole makes no joint: said, and nothing drawn.
    edit(&mut s, NumberTarget::BearingOuterRadius, "0");
    assert!(s.status().starts_with("Invalid input"), "{}", s.status());
    assert!(texts(&s.inspector()).contains("Nothing to draw"));
    assert_eq!(s.scene().unwrap().positions.len(), 3);
    s.key(Key::Char('z'), Mods { command: true, ..Mods::default() });
    s.key(Key::Char('z'), Mods { command: true, ..Mods::default() });
    assert_eq!(s.model().applied_torque, 360.0);
    s.key(Key::Char('z'), Mods { command: true, shift: true });
    assert_eq!(s.model().applied_torque, 480.0);
}

#[test]
fn switches_members_and_the_advanced_section_follow_the_terminal_rows() {
    let mut s = session("rows");
    // Showing the advanced rows is not an edit of the joint.
    act(&mut s, PreloadAct::Row(FieldRow::AdvancedSection));
    assert!(s.model().advanced_open && !s.dirty() && s.undo.is_empty());
    let t = texts(&s.inspector());
    assert!(t.contains("Advanced settings  Hide") && t.contains("Member Stiffness  Pressure Cone"), "{t}");
    act(&mut s, PreloadAct::Row(FieldRow::ToggleAddMember));
    assert_eq!(s.model().members.len(), 2);
    assert!(texts(&s.inspector()).contains("Remove Last Joint Member  Remove"));
    act(&mut s, PreloadAct::Row(FieldRow::ToggleRemoveMember));
    assert_eq!(s.model().members.len(), 1);
    act(&mut s, PreloadAct::Row(FieldRow::ToggleExternalLoadEnabled));
    assert!(ids(&s.inspector()).contains(&"preload-service".to_string()));
    act(&mut s, PreloadAct::Row(FieldRow::ToggleSlipEnabled));
    assert!(ids(&s.inspector()).contains(&"preload-slip".to_string()));
    assert_eq!(s.undo.len(), 4);
    // Finite Element stiffness: pending until the member model answers, then the solver uses its cone angle.
    act(&mut s, PreloadAct::Row(FieldRow::ToggleMemberStiffness));
    wait_for_fe(&mut s);
    let angle = s.fe_current().unwrap().as_ref().unwrap().0.equivalent_angle_deg.unwrap();
    assert!(s.model().uses_fe_stiffness());
    assert_eq!(s.model().solver_cone_angle_deg(), angle);
    assert!(texts(&s.inspector()).contains(&format!("Finite Element (cone {angle:.1} deg)")));
}

#[test]
fn the_pickers_pick_a_bolt_apply_a_template_and_draw_random_joints() {
    let mut s = session("pick");
    act(&mut s, PreloadAct::Row(FieldRow::OpenBoltPicker));
    let card = s.value_card().expect("the bolt picker");
    assert_eq!(card.title, "Bolt");
    // The default joint is an AN6; it is listed and marked current.
    assert!(choices(&card).iter().any(|c| c.starts_with("AN6 ")), "{:?}", choices(&card));
    assert!(card.rows.iter().any(|r| matches!(r, CardRow::Choice { label, current: true, .. } if label.starts_with("AN6 "))));
    assert!(card.rows.iter().any(|r| matches!(r, CardRow::Text(t) if t.contains("more: type to narrow"))));
    // The search narrows by designation or section; a pick fills the thread and closes.
    s.card_field(0, "hi-lok".into());
    let hits = choices(&s.value_card().unwrap());
    assert!(!hits.is_empty() && hits.iter().all(|h| h.starts_with("HL")), "{hits:?}");
    s.card_field(0, "AN8".into());
    assert_eq!(choices(&s.value_card().unwrap()), ["AN8 (.5000-20 UNF)"]);
    let (sec, e) = model::BOLT_SECTIONS.iter().enumerate().find_map(|(k, (_, es))| es.iter().position(|b| b.designation.starts_with("AN8 ")).map(|e| (k, e))).unwrap();
    act(&mut s, PreloadAct::PickBolt(sec, e));
    assert!(s.value_card().is_none());
    assert!(s.model().matching_bolt().unwrap().designation.starts_with("AN8 "));
    assert_eq!(s.undo.len(), 1);
    // A Hi-Lok fills the thread only and says so.
    let (sec, e) = (4, 0);
    act(&mut s, PreloadAct::PickBolt(sec, e));
    assert_eq!(s.status(), model::HI_LOK_NOTE);
    // A preset applies at once and closes; a random joint applies and keeps the picker open for another.
    act(&mut s, PreloadAct::Row(FieldRow::OpenTemplatePicker));
    let card = s.value_card().unwrap();
    assert_eq!(choices(&card).len(), joint_templates::presets().len() + 1);
    act(&mut s, PreloadAct::ApplyTemplate(0));
    assert!(s.value_card().is_none());
    assert_eq!(s.model().members.len(), joint_templates::presets()[0].layers.len());
    assert!(s.status().starts_with("Joint template applied"), "{}", s.status());
    act(&mut s, PreloadAct::Row(FieldRow::OpenTemplatePicker));
    act(&mut s, PreloadAct::RandomJoint);
    let first = PreloadFile::capture(s.model(), "");
    assert!(s.value_card().unwrap().rows.iter().any(|r| matches!(r, CardRow::Text(t) if t.starts_with("Applied random joint #1"))));
    act(&mut s, PreloadAct::RandomJoint);
    assert_ne!(PreloadFile::capture(s.model(), ""), first, "the next random joint is another one");
    act(&mut s, PreloadAct::Undo);
    assert_eq!(PreloadFile::capture(s.model(), ""), first);
    // Esc closes a picker.
    act(&mut s, PreloadAct::Row(FieldRow::OpenBoltPicker));
    s.key(Key::Escape, Mods::default());
    assert!(s.value_card().is_none());
}

#[test]
fn save_and_reopen_round_trips_the_joint_and_export_writes_the_report_with_the_fe_check() {
    let mut s = session("save");
    s.model.apply_template(&joint_templates::presets()[0]);
    act(&mut s, PreloadAct::Row(FieldRow::ToggleSlipEnabled));
    act(&mut s, PreloadAct::Row(FieldRow::ToggleMode));
    edit(&mut s, NumberTarget::TargetPreload, "3000");
    act(&mut s, PreloadAct::Save);
    assert!(!s.dirty(), "{}", s.status());
    let back = PreloadSession::open(&s.path).unwrap();
    assert_eq!(PreloadFile::capture(back.model(), "x"), PreloadFile::capture(s.model(), "x"));
    wait_for_fe(&mut s);
    act(&mut s, PreloadAct::Export);
    let report = std::fs::read_to_string(s.path.with_extension("report.txt")).unwrap();
    assert_eq!(report, report::build_report_text(s.model(), s.fe_check_current().as_ref()));
    assert!(report.contains("Member compliance cross-check (FE"), "{report}");
    // A file another program changed is not overwritten.
    crate::persistence::write(&s.path, "{}").unwrap();
    edit(&mut s, NumberTarget::TargetPreload, "3100");
    act(&mut s, PreloadAct::Save);
    assert!(s.status().starts_with("Save conflict"), "{}", s.status());
}
