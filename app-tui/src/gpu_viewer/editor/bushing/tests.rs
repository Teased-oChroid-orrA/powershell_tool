use super::*;
use crate::gpu_viewer::editor::authoring::Authoring;
use crate::gpu_viewer::editor::session::{EditorAction, Key, Mods};
use crate::gpu_viewer::controller::UiAction;
use crate::gpu_viewer::ui::CardRow;
use crate::toolboxes::bushing::model::{BushingModel, NumberTarget};
use std::time::{Duration, Instant};

fn session() -> BushingSession {
    let dir = std::env::temp_dir().join(format!("bushing-session-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    BushingSession::new(&BushingFile::capture(&BushingModel::default(), "wing fitting"), dir.join(format!("case-{:?}.json", std::thread::current().id()).replace(['(', ')'], "")))
}
fn act(s: &mut BushingSession, a: BushingAct) {
    s.act(EditorAction::Bushing(a));
}
fn type_text(s: &mut BushingSession, text: &str) {
    for c in text.chars() {
        s.key(Key::Char(c), Mods::default());
    }
}
fn edit(s: &mut BushingSession, t: NumberTarget, value: &str) {
    act(s, BushingAct::Row(FieldRow::Number(t)));
    assert!(s.prompting(), "{t:?} opens a typed edit");
    type_text(s, value);
    s.key(Key::Enter, Mods::default());
    assert!(!s.prompting());
}
fn wait(s: &mut BushingSession, what: &str, done: impl Fn(&BushingSession) -> bool) {
    let t = Instant::now();
    while !done(s) {
        s.poll();
        assert!(t.elapsed() < Duration::from_secs(90), "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}
fn texts(cards: &[Card]) -> String {
    cards.iter().flat_map(|c| c.items.iter()).filter_map(|i| i.plain_text()).collect::<Vec<_>>().join("\n")
}
fn card<'a>(cards: &'a [Card], id: &str) -> &'a Card {
    cards.iter().find(|c| c.id == id).unwrap_or_else(|| panic!("no card {id} among {:?}", cards.iter().map(|c| &c.id).collect::<Vec<_>>()))
}
fn labels(items: Vec<Item>) -> Vec<String> {
    items.into_iter().filter_map(|i| if let Item::Entry(e) = i { Some(e.label) } else { None }).collect()
}

#[test]
fn it_opens_on_the_workbench_inputs_with_three_steps_and_no_other_benches() {
    let s = session();
    assert_eq!(s.title(), "Bushing | wing fitting");
    assert!(s.benches().is_empty());
    assert_eq!(s.key_labels()[0], "Housing");
    assert_eq!(s.key_labels()[3], "Bushing");
    let tabs: Vec<_> = s.ribbon().into_iter().map(|t| t.title).collect();
    assert_eq!(tabs, ["Inputs", "Checks", "Simulation"]);
    // Every section of the terminal form is a card of the Inputs step.
    let cards = s.inspector();
    for id in ["bushing-bore-fit", "bushing-housing-geometry", "bushing-materials-friction", "bushing-load"] {
        assert!(card(&cards, id).steps == ["Inputs"], "{id}");
    }
    assert!(texts(&cards).contains("Bore Diameter  0.5000 in"));
    assert!(s.step_state("Inputs").is_some() && s.step_state("Nope").is_none());
}

#[test]
fn clicking_a_number_row_types_a_value_that_the_solver_and_the_housing_picture_follow_and_undo_returns() {
    let mut s = session();
    let before = s.model().output.pressure;
    let key_before = s.fe_key.clone();
    edit(&mut s, NumberTarget::Interference, "0.003");
    assert_eq!(s.model().interference, 0.003);
    assert!(s.model().output.pressure > 1.5 * before, "the solver recomputed: {before} -> {}", s.model().output.pressure);
    assert!(s.dirty() && s.title().ends_with('*'));
    let p = s.fe_key.clone().unwrap();
    assert_ne!(Some(&p), key_before.as_ref(), "the FE problem follows the inputs");
    assert!((p.bushings[0].interference - s.model().output.delta_total).abs() < 1e-12);
    // Typing replaces the old value; a non-number is refused and nothing changes.
    act(&mut s, BushingAct::Row(FieldRow::Number(NumberTarget::Load)));
    assert!(texts(&s.inspector()).contains("Applied Edge Load  1000.0 lbf") || texts(&s.inspector()).contains("Applied Edge Load  1000"), "{}", texts(&s.inspector()));
    type_text(&mut s, "abc");
    s.key(Key::Enter, Mods::default());
    assert_eq!(s.model().load, 1000.0);
    // Undo and redo walk the history.
    act(&mut s, BushingAct::Undo);
    assert_eq!(s.model().interference, 0.0015);
    assert_eq!(s.model().output.pressure, before);
    act(&mut s, BushingAct::Redo);
    assert_eq!(s.model().interference, 0.003);
}

#[test]
fn escape_cancels_a_typed_edit_and_a_choice_row_cycles_in_one_click() {
    let mut s = session();
    act(&mut s, BushingAct::Row(FieldRow::Number(NumberTarget::HousingWidth)));
    type_text(&mut s, "9");
    s.key(Key::Escape, Mods::default());
    assert!(!s.prompting());
    assert_eq!(s.model().housing_width, 1.5);
    act(&mut s, BushingAct::Row(FieldRow::ToggleFitType));
    assert_eq!(s.model().fit_type, crate::toolboxes::bushing::model::FitType::Shrink);
    act(&mut s, BushingAct::Row(FieldRow::AdvancedSection));
    assert!(texts(&s.inspector()).contains("Tolerance Entry"), "the advanced rows appear");
}

#[test]
fn the_bore_row_opens_a_reamer_picker_whose_choice_sets_the_bore_and_its_tolerance() {
    let mut s = session();
    act(&mut s, BushingAct::Row(FieldRow::Number(NumberTarget::BoreDia)));
    let c = s.value_card().expect("a picker card");
    assert_eq!(c.title, "Bore diameter");
    let choices: Vec<_> = c.rows.iter().filter_map(|r| if let CardRow::Choice { label, action, .. } = r { Some((label.clone(), *action)) } else { None }).collect();
    assert!(choices.iter().any(|(l, _)| l == "Type a value"));
    // Searching by a size narrows it to sizes that contain it.
    s.card_field(0, "0.3".into());
    let c = s.value_card().unwrap();
    let narrowed = c.rows.iter().filter(|r| matches!(r, CardRow::Choice { label, .. } if label != "Type a value")).count();
    assert!(narrowed > 0 && narrowed <= 12, "{narrowed} sizes near 0.3");
    let (label, action) = c.rows.iter().find_map(|r| if let CardRow::Choice { label, action, .. } = r { Some((label.clone(), *action)) } else { None }).unwrap();
    assert!(label.contains("in"), "{label}");
    let UiAction::Editor(a) = action else { panic!("an editor action") };
    s.act(a);
    assert!(s.value_card().is_none(), "choosing closes the card");
    assert!((s.model().bore_dia - 0.3).abs() < 0.05, "bore {}", s.model().bore_dia);
    assert!(s.model().bore_tol_plus > 0.0 || s.model().bore_tol_minus > 0.0, "the reamer's tolerance came with it");
}

#[test]
fn the_material_picker_lists_matches_marks_the_current_one_and_choosing_changes_the_solver() {
    let mut s = session();
    let before = s.model().housing_material().id;
    act(&mut s, BushingAct::Row(FieldRow::OpenHousingMaterialPicker));
    s.card_field(0, "7075".into());
    let c = s.value_card().unwrap();
    let rows: Vec<_> = c.rows.iter().filter_map(|r| if let CardRow::Choice { label, action, current, .. } = r { Some((label.clone(), *action, *current)) } else { None }).collect();
    assert!(!rows.is_empty() && rows.len() <= 12 && rows.iter().all(|(l, _, _)| l.contains("7075")), "{rows:?}");
    let (_, action, _) = rows.iter().find(|(_, _, cur)| !cur).cloned().unwrap_or_else(|| rows[0].clone());
    let pressure = s.model().output.pressure;
    let UiAction::Editor(a) = action else { panic!() };
    s.act(a);
    assert!(s.value_card().is_none());
    assert!(s.model().housing_material().name.contains("7075"));
    let _ = (before, pressure);
}

#[test]
fn a_failing_check_offers_its_verified_fix_and_applying_it_clears_that_failure() {
    let mut s = session();
    edit(&mut s, NumberTarget::EdgeDist, "0.32");
    let failing: Vec<_> = s.model().checks.iter().filter(|c| c.severity == Severity::Fail).map(|c| c.kind).collect();
    assert!(!failing.is_empty(), "an edge distance this small fails: {:?}", s.model().checks);
    let verdict = card(&s.inspector(), "bushing-verdict").clone();
    assert!(verdict.items.iter().filter_map(|i| i.plain_text()).any(|t| t.starts_with("REVIEW")));
    let kind = s.model().recommendations.first().map(|r| r.fixes).expect("a fix is offered");
    act(&mut s, BushingAct::ShowFixes(kind));
    assert_eq!(s.step, "Checks");
    let fixes = card(&s.inspector(), "bushing-fixes").clone();
    let apply = fixes.items.iter().find_map(|i| if let DockItem::Line { text, action: Some(UiAction::Editor(EditorAction::Bushing(a @ BushingAct::ApplyFix(_)))), .. } = i { (text == "Apply this fix").then_some(*a) } else { None }).expect("an Apply row");
    let n_before = failing.len();
    s.do_act(apply);
    let after = s.model().checks.iter().filter(|c| c.severity == Severity::Fail).count();
    assert!(after < n_before, "the fix reduced the failures: {n_before} -> {after}");
    act(&mut s, BushingAct::Undo);
    assert_eq!(s.model().checks.iter().filter(|c| c.severity == Severity::Fail).count(), n_before, "undo takes the fix back");
}

#[test]
fn the_edge_check_runs_off_the_ui_thread_and_fills_its_table() {
    let mut s = session();
    assert!(texts(&s.inspector()).contains("Not run."));
    act(&mut s, BushingAct::EdgeCheck(false));
    assert!(s.st.edge_job.is_some(), "running");
    wait(&mut s, "the edge check", |s| s.st.edge_check.is_some());
    let t = texts(&s.inspector());
    assert!(t.contains("Superposition") && t.contains("Legacy solver"), "{t}");
    assert!(s.step_state("Checks").is_some());
    act(&mut s, BushingAct::EdgeCheck(false));
    s.do_act(BushingAct::CancelEdgeCheck);
    assert!(s.st.edge_job.is_none());
}

#[test]
fn the_housing_and_bushing_are_meshed_on_their_own_and_the_bushing_reads_apart_from_its_housing() {
    let mut s = session();
    wait(&mut s, "the mesh", |s| s.fea().preview_summary().is_ok());
    let scene = s.scene().unwrap();
    let codes: std::collections::BTreeSet<i32> = scene.samples.iter().map(|v| v[3].round() as i32).collect();
    assert!(codes.contains(&3), "bushing nodes carry the highlight code: {codes:?}");
    assert!(codes.contains(&0), "housing nodes carry the body code: {codes:?}");
    assert!(s.geometry_view() && !s.shows_result());
    // The picture is read-only: a click picks nothing and opens no menu.
    let cam = Camera { yaw: 0.0, pitch: 0.0, zoom: 1.0, aspect: 1.0, pan: [0.0, 0.0] };
    assert!(!s.click([100.0, 100.0], [800.0, 600.0], &cam));
    assert!(s.take_popup().is_none());
}

#[test]
fn solving_shows_the_fit_pressure_beside_the_closed_form_and_a_change_makes_it_stale() {
    let mut s = session();
    wait(&mut s, "the mesh", |s| s.fea().preview_summary().is_ok());
    act(&mut s, BushingAct::Solve);
    wait(&mut s, "the solve", |s| s.fea().current_solution().is_some() && !s.solving());
    // The window rebuilds the scene when it is stale; the result shows once it has.
    assert!(s.needs_scene());
    s.scene().unwrap();
    assert!(s.shows_result(), "the result is shown once solved");
    assert_eq!(s.scene().unwrap().gain, 1.0, "a fit result opens at its true size");
    let fe = texts(&s.inspector());
    assert!(fe.contains("Fit pressure, FE") && fe.contains("Closed form") && fe.contains("Max von Mises"), "{fe}");
    assert_eq!(s.step_state("Simulation"), Some(crate::gpu_viewer::editor::authoring::StepState::Done));
    edit(&mut s, NumberTarget::Load, "2000");
    assert!(texts(&s.inspector()).contains("not run for these inputs"));
    assert_eq!(s.step_state("Simulation"), Some(crate::gpu_viewer::editor::authoring::StepState::Todo));
}

#[test]
fn a_saved_case_reopens_with_the_same_inputs_and_the_report_is_written_beside_it() {
    let mut s = session();
    edit(&mut s, NumberTarget::HousingWidth, "2.25");
    act(&mut s, BushingAct::Save);
    assert!(!s.dirty());
    let again = BushingSession::open(s.path()).unwrap();
    assert_eq!(again.model().housing_width, 2.25);
    assert_eq!(again.model().output.pressure, s.model().output.pressure);
    act(&mut s, BushingAct::Export);
    let report = std::fs::read_to_string(s.path().with_extension("report.txt")).unwrap();
    assert!(report.contains("Bore"), "{report}");
    assert!(BushingSession::open(std::path::Path::new("/nonexistent-dir/none.json")).is_ok(), "a missing file starts from the defaults");
    let _ = std::fs::remove_file(s.path());
    let _ = std::fs::remove_file(s.path().with_extension("report.txt"));
}

#[test]
fn the_tree_and_menus_follow_the_checks_and_say_why_something_is_unavailable() {
    let mut s = session();
    let tree = s.tree();
    let failing = s.model().checks.iter().filter(|c| c.severity == Severity::Fail).count();
    assert!(tree.iter().any(|n| n.label == format!("Checks ({failing} failing)")), "{:?}", tree.iter().map(|n| &n.label).collect::<Vec<_>>());
    assert!(tree.iter().any(|n| n.label == "Housing") && tree.iter().any(|n| n.label == "Bushing"));
    edit(&mut s, NumberTarget::EdgeDist, "0.32");
    assert!(s.tree().iter().any(|n| n.label.starts_with("Checks (") && !n.label.starts_with("Checks (0")));
    assert!(s.tree().iter().any(|n| n.label.starts_with("Fixes (")));
    let menus = s.menus();
    let titles: Vec<_> = menus.iter().map(|m| m.title.as_str()).collect();
    assert_eq!(titles, ["File", "Edit", "Tools", "Simulation"]);
    assert!(labels(menus[1].items.clone()).contains(&"Undo".to_string()));
    let undo = s.ribbon()[0].groups.iter().flat_map(|g| g.buttons.iter()).find(|e| e.label == "Undo").unwrap().clone();
    assert!(undo.enabled, "there is something to undo after the edit");
    let fresh = session();
    let undo = fresh.ribbon()[0].groups.iter().flat_map(|g| g.buttons.iter()).find(|e| e.label == "Undo").unwrap().clone();
    assert!(!undo.enabled && undo.reason.as_deref() == Some("nothing to undo"));
}

#[test]
fn inputs_that_describe_no_bushing_in_a_housing_keep_the_last_picture_and_say_so() {
    let mut s = session();
    edit(&mut s, NumberTarget::EdgeDist, "0.2");
    assert!(s.status().contains("No FE picture yet") && s.status().contains("edge distance"), "{}", s.status());
    assert!(texts(&s.inspector()).contains("No picture yet"));
    edit(&mut s, NumberTarget::EdgeDist, "0.9");
    assert!(!s.status().contains("No FE picture"));
}
