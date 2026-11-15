use super::*;
use crate::gpu_viewer::editor::authoring::Authoring;
use crate::gpu_viewer::editor::session::{Key, Mods};
use crate::gpu_viewer::ui::DockItem;

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("line-run-session-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("case.json")
}

fn session(name: &str) -> LineRunSession {
    LineRunSession::open(&temp(name)).unwrap()
}

/// OD 1, wall 0.05, 100 in on two supports, contents 2 lbf/in, weightless tube: peak moment 2500 in.lbf.
fn enter_beam(s: &mut LineRunSession) {
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
        type_value(s, f, &v.to_string());
    }
}

fn type_value(s: &mut LineRunSession, f: Field, text: &str) {
    s.act(EditorAction::LineRun(LineRunAct::Edit(f)));
    assert!(s.prompting());
    for c in text.chars() {
        s.key(Key::Char(c), Mods::default());
    }
    s.key(Key::Enter, Mods::default());
    assert!(!s.prompting());
}

fn act(s: &mut LineRunSession, a: LineRunAct) {
    s.act(EditorAction::LineRun(a));
}

fn cards(s: &LineRunSession) -> Vec<Card> {
    s.inspector()
}
fn card<'a>(cards: &'a [Card], id: &str) -> &'a Card {
    cards.iter().find(|c| c.id == id).unwrap_or_else(|| panic!("no card {id}: {:?}", cards.iter().map(|c| c.id.as_str()).collect::<Vec<_>>()))
}
fn text(c: &Card) -> String {
    c.items.iter().filter_map(|i| i.plain_text()).collect::<Vec<_>>().join("\n")
}

#[test]
fn a_new_session_says_what_is_missing_and_draws_nothing() {
    let mut s = session("new");
    assert!(s.status().contains("elastic modulus"), "{}", s.status());
    assert!(text(card(&cards(&s), "line-result")).contains("never defaulted"));
    assert!(!cards(&s).iter().any(|c| c.id == "line-margins"), "no margins without a result");
    let scene = s.scene().unwrap();
    assert_eq!(scene.positions.len(), 3, "the empty scene");
    assert_eq!(s.step_state("Inputs"), Some(crate::gpu_viewer::editor::authoring::StepState::Todo));
}

#[test]
fn typing_the_inputs_gives_the_closed_form_peak_moment_in_the_station_plot_and_the_picture() {
    let mut s = session("beam");
    enter_beam(&mut s);
    assert!(s.model.outcome.is_ok(), "{:?}", s.model.outcome);
    let c = cards(&s);
    let margins = card(&c, "line-margins");
    assert!(margins.items.iter().any(|i| matches!(i, DockItem::Bars { rows, .. } if rows.len() == 4)), "four margins on the bars");
    assert!(text(margins).contains("governing"), "{}", text(margins));
    // The moment plot peaks at wL^2/8 = 2500 in.lbf and marks both supports and the governing station.
    let stations = card(&c, "line-stations");
    let DockItem::Plot(moment) = &stations.items[0] else { panic!("moment plot first") };
    let top = moment.series[0].points.iter().map(|p| p[1]).fold(0.0, f64::max);
    assert!((top - 2500.0).abs() < 1e-6, "{top}");
    let marks: Vec<_> = moment.marks.iter().map(|m| m.text.as_str()).collect();
    assert!(marks.contains(&"S1 anchor") && marks.contains(&"S2") && marks.contains(&"governing"), "{marks:?}");
    let DockItem::Plot(axial) = &stations.items[1] else { panic!("axial plot second") };
    assert!(axial.y_label.contains("Axial force") && axial.series[0].dashed);
    // Reactions: wL/2 = 100 lbf up at each support.
    assert!(text(card(&c, "line-reactions")).contains("100"), "{}", text(card(&c, "line-reactions")));
    // The picture is coloured by the same moment.
    let scene = s.scene().unwrap();
    assert_eq!(scene.field_label, "Bending moment");
    assert!((scene.range[1] - 2500.0).abs() < 1e-3, "{:?}", scene.range);
    assert_eq!(scene.fields.iter().map(|f| f.label.as_str()).collect::<Vec<_>>(), drawing::FIELDS[1..]);
    assert_eq!(s.field_unit("Bending moment"), "in\u{b7}lbf");
    assert!(s.status().starts_with("PASS") || s.status().starts_with("REVIEW"));
}

#[test]
fn units_switch_changes_the_text_not_the_solve_and_is_one_undo_step() {
    let mut s = session("units");
    enter_beam(&mut s);
    let before = s.model.outcome.clone();
    act(&mut s, LineRunAct::ToggleUnits);
    assert_eq!(s.model.outcome, before, "display units never reach the solver");
    let c = cards(&s);
    assert!(text(card(&c, "line-tube")).contains("25.4 mm"), "{}", text(card(&c, "line-tube")));
    let DockItem::Plot(p) = &card(&c, "line-stations").items[0] else { panic!() };
    assert!(p.y_label.contains("N\u{b7}mm"), "{}", p.y_label);
    // A value typed now is in millimetres.
    type_value(&mut s, Field::Od, "12.7");
    assert!((s.model.inputs.od - 0.5).abs() < 1e-12);
    s.key(Key::Char('z'), Mods { command: true, shift: false, ..Mods::default() });
    s.key(Key::Char('z'), Mods { command: true, shift: false, ..Mods::default() });
    assert_eq!(s.model.units(), crate::toolboxes::line_run::model::Units::Us, "two undos: the diameter, then the units");
    assert!((s.model.inputs.od - 1.0).abs() < 1e-12);
}

#[test]
fn typing_the_same_value_is_not_an_edit_and_junk_is_reported() {
    let mut s = session("same");
    enter_beam(&mut s);
    let depth = s.undo.len();
    type_value(&mut s, Field::Od, "1");
    assert_eq!(s.undo.len(), depth);
    type_value(&mut s, Field::Od, "1e");
    assert_eq!(s.undo.len(), depth);
    assert!(s.status().contains("not a number"), "{}", s.status());
    s.act(EditorAction::LineRun(LineRunAct::Edit(Field::Od)));
    s.key(Key::Escape, Mods::default());
    assert!(!s.prompting());
}

#[test]
fn cases_supports_and_loads_are_edited_through_the_inspector_and_every_stale_row_is_harmless() {
    let mut s = session("tables");
    enter_beam(&mut s);
    act(&mut s, LineRunAct::AddCase);
    act(&mut s, LineRunAct::AddLoad);
    act(&mut s, LineRunAct::Scope(0));
    type_value(&mut s, Field::LoadForce(0, 2), "-100");
    assert_eq!(s.model.inputs.loads[0].case, Some(0));
    let r = s.model.outcome.as_ref().unwrap();
    assert_eq!(r.cases.len(), 2);
    // Case 1 (index 0) carries the 100 lbf load, case 2 does not.
    let peak = |k: usize| r.cases[k].stations.iter().map(|x| x.resultant_moment()).fold(0.0, f64::max);
    assert!(peak(0) > peak(1) + 1000.0, "{} {}", peak(0), peak(1));
    // Selecting the case redraws and the tree lists both.
    act(&mut s, LineRunAct::SelectCase(1));
    assert!(s.needs_scene());
    assert_eq!(s.tree().iter().filter(|n| n.label.contains("MS")).count(), 2);
    // A case name is typed as text.
    act(&mut s, LineRunAct::RenameCase);
    for c in "Crash 9 g".chars() {
        s.key(Key::Char(c), Mods::default());
    }
    s.key(Key::Enter, Mods::default());
    assert_eq!(s.model.selected_case().name, "Crash 9 g");
    // Stale indices (a row removed after the card was drawn) do nothing.
    for a in [LineRunAct::Anchor(9), LineRunAct::Clamp(9), LineRunAct::RemoveSupport(9), LineRunAct::RemoveLoad(9), LineRunAct::Thrust(9), LineRunAct::Scope(9), LineRunAct::SelectCase(9), LineRunAct::Edit(Field::LoadX(9))] {
        act(&mut s, a);
        s.key(Key::Escape, Mods::default());
    }
    act(&mut s, LineRunAct::RemoveCase);
    act(&mut s, LineRunAct::RemoveCase);
    assert_eq!(s.model.inputs.cases.len(), 1);
}

#[test]
fn a_mechanism_shows_its_reason_and_nothing_is_exported_as_a_table() {
    let mut s = session("mechanism");
    enter_beam(&mut s);
    act(&mut s, LineRunAct::RemoveSupport(1));
    act(&mut s, LineRunAct::RemoveSupport(0));
    assert!(s.model.outcome.is_err());
    assert!(!s.status().is_empty());
    act(&mut s, LineRunAct::Export);
    assert!(s.status().contains("no station table"), "{}", s.status());
    assert!(s.path.with_extension("report.txt").exists() && !s.path.with_extension("stations.csv").exists());
    let ribbon = s.ribbon();
    let export = ribbon.iter().find(|t| t.title == "Results").unwrap().groups[0].buttons[0].clone();
    assert!(!export.enabled);
}

#[test]
fn save_reopen_and_export_round_trip() {
    let mut s = session("files");
    enter_beam(&mut s);
    act(&mut s, LineRunAct::Save);
    assert!(!s.dirty());
    let again = LineRunSession::open(&s.path).unwrap();
    assert_eq!(again.model().inputs, s.model().inputs);
    assert_eq!(again.model().outcome, s.model().outcome);
    act(&mut s, LineRunAct::Export);
    let report = std::fs::read_to_string(s.path.with_extension("report.txt")).unwrap();
    assert!(report.contains("LINE RUN REPORT") && report.contains("Governing:"), "{report}");
    let csv = std::fs::read_to_string(s.path.with_extension("stations.csv")).unwrap();
    assert!(csv.lines().count() > 10 && csv.starts_with("case,x (in)"), "{csv}");
    // An unset constant stays unset across the file.
    let blank = session("blank");
    let reread = LineRunFile::from_json(&blank.snapshot().to_json()).unwrap();
    assert!(reread.inputs.fty.is_none());
    assert!(LineRunFile::from_json(r#"{"version": 99}"#).is_err());
}

#[test]
fn every_ribbon_entry_and_menu_item_acts_and_the_tree_follows_the_step() {
    let mut s = session("menus");
    enter_beam(&mut s);
    let tabs = s.ribbon();
    assert_eq!(tabs.iter().map(|t| t.title.as_str()).collect::<Vec<_>>(), ["Inputs", "Results", "Report"]);
    for m in s.menus() {
        assert!(!m.items.is_empty(), "{} menu is empty", m.title);
    }
    act(&mut s, LineRunAct::Step("Results"));
    assert_eq!(s.focus_step(), Some("Results"));
    assert!(s.tree().iter().any(|n| n.label.starts_with("Results (")));
}
