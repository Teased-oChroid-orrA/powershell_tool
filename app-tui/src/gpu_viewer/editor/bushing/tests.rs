use super::*;
use crate::gpu_viewer::controller::UiAction;
use crate::gpu_viewer::editor::authoring::Authoring;
use crate::gpu_viewer::editor::session::{EditorAction, Key, Mods};
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
        // A hang guard, not a speed check: the full suite runs many FE solves at once and starves this one.
        assert!(t.elapsed() < Duration::from_secs(300), "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}
fn texts(cards: &[Card]) -> String {
    cards.iter().flat_map(|c| c.items.iter()).filter_map(|i| i.plain_text()).collect::<Vec<_>>().join("\n")
}
fn inputs(s: &BushingSession) -> String {
    texts(&s.input_cards())
}
fn results(s: &BushingSession) -> String {
    texts(&s.result_cards())
}
fn card(cards: &[Card], id: &str) -> Card {
    cards.iter().find(|c| c.id == id).cloned().unwrap_or_else(|| panic!("no card {id} among {:?}", cards.iter().map(|c| &c.id).collect::<Vec<_>>()))
}
fn labels(items: Vec<Item>) -> Vec<String> {
    items.into_iter().filter_map(|i| if let Item::Entry(e) = i { Some(e.label) } else { None }).collect()
}
fn verdict_actions(s: &BushingSession) -> Vec<String> {
    card(&s.result_cards(), "bushing-verdict").items.iter().flat_map(|i| if let DockItem::Verdict { actions, .. } = i { actions.iter().map(|e| e.label.clone()).collect() } else { Vec::new() }).collect()
}

#[test]
fn it_opens_on_three_panes_with_install_force_first_and_no_tree_ribbon_or_inspector() {
    let s = session();
    assert_eq!(s.title(), "Bushing | wing fitting");
    assert!(s.benches().is_empty() && s.tree().is_empty() && s.ribbon().is_empty() && s.inspector().is_empty());
    assert!(s.has_workbench());
    let w = s.workbench().expect("the three panes");
    assert_eq!(w.chips[0].0, cards::UNITS_CHIP);
    assert!(w.chips.iter().any(|(c, _)| c.starts_with("REVIEW")), "the default case fails edge sequencing: {:?}", w.chips);
    // Every section of the input form is a card of the inputs pane.
    for id in ["bushing-bore-fit", "bushing-housing-geometry", "bushing-materials-friction", "bushing-load"] {
        card(&w.inputs, id);
    }
    assert!(texts(&w.inputs).contains("Bore Diameter  0.5000 in"));
    // Results: the verdict, then install force.
    let ids: Vec<_> = w.results.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(&ids[..2], ["bushing-verdict", "bushing-install"]);
    assert_eq!(w.sketches.len(), 2, "the axial section and the cross-section");
    assert_eq!(w.show, WorkbenchView::Split);
    assert!(w.dialog.is_none());
    assert_eq!(w.actions.last().unwrap().label, "Send joint to FEM...", "the main action is last");
    // Nothing is meshed while the inputs are only shown.
    assert!(!s.fea().meshing() && s.fea().preview_summary().is_err());
    let titles: Vec<_> = s.menus().into_iter().map(|m| m.title).collect();
    assert_eq!(titles, ["File", "Edit", "View", "Tools"]);
}

#[test]
fn the_default_install_force_is_shown_grouped_with_the_formula_that_reproduces_it() {
    let s = session();
    let out = &s.model().output;
    let f = s.model().friction * out.install_pressure * std::f64::consts::PI * s.model().bore_dia * s.model().housing_len;
    assert!((f - out.install_force).abs() / out.install_force < 0.005, "the formula reproduces the solver: {f} vs {}", out.install_force);
    let install = texts(&[card(&s.result_cards(), "bushing-install")]);
    assert!(install.contains("Install force  1,036 lbf"), "{install}");
    assert!(install.contains("F = \u{3bc} \u{b7} p \u{b7} \u{3c0} \u{b7} D \u{b7} L = 0.15 \u{b7} ") && install.contains("= 1,036.0 lbf"), "{install}");
}

#[test]
fn grouped_puts_commas_in_the_thousands_and_keeps_the_sign() {
    assert_eq!(cards::grouped(1036.04, 1), "1,036.0");
    assert_eq!(cards::grouped(1036.04, 0), "1,036");
    assert_eq!(cards::grouped(999.0, 0), "999");
    assert_eq!(cards::grouped(1_234_567.891, 2), "1,234,567.89");
    assert_eq!(cards::grouped(-12_500.0, 0), "-12,500");
    assert_eq!(cards::grouped(-0.0001, 1), "0.0", "a negative that rounds to zero has no sign");
}

#[test]
fn clicking_a_number_row_types_a_value_that_the_solver_and_the_fe_model_follow_and_undo_returns() {
    let mut s = session();
    let before = s.model().output.pressure;
    let key_before = s.fe_key.clone();
    edit(&mut s, NumberTarget::Interference, "0.003");
    assert_eq!(s.model().interference, 0.003);
    assert!(s.model().output.pressure > 1.5 * before, "the solver recomputed: {before} -> {}", s.model().output.pressure);
    assert!(s.dirty() && s.title().ends_with('*'));
    assert!(s.needs_scene(), "the 3D picture follows the inputs");
    let p = s.fe_key.clone().unwrap();
    assert_ne!(Some(&p), key_before.as_ref(), "the FE problem follows the inputs");
    assert!((p.bushings[0].interference - s.model().output.delta_total).abs() < 1e-12);
    assert!(s.fea().preview_summary().is_err(), "an edit meshes nothing");
    // Typing replaces the old value; a non-number is refused and nothing changes.
    act(&mut s, BushingAct::Row(FieldRow::Number(NumberTarget::Load)));
    assert!(inputs(&s).contains("Applied Edge Load  1000"), "{}", inputs(&s));
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
    assert!(inputs(&s).contains("Tolerance Entry"), "the advanced rows appear");
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
fn the_material_picker_lists_matches_and_choosing_changes_the_solver() {
    let mut s = session();
    act(&mut s, BushingAct::Row(FieldRow::OpenHousingMaterialPicker));
    s.card_field(0, "7075".into());
    let c = s.value_card().unwrap();
    let rows: Vec<_> = c.rows.iter().filter_map(|r| if let CardRow::Choice { label, action, current, .. } = r { Some((label.clone(), *action, *current)) } else { None }).collect();
    assert!(!rows.is_empty() && rows.len() <= 12 && rows.iter().all(|(l, _, _)| l.contains("7075")), "{rows:?}");
    let (_, action, _) = rows.iter().find(|(_, _, cur)| !cur).cloned().unwrap_or_else(|| rows[0].clone());
    let UiAction::Editor(a) = action else { panic!() };
    s.act(a);
    assert!(s.value_card().is_none());
    assert!(s.model().housing_material().name.contains("7075"));
}

#[test]
fn why_lists_the_checks_behind_the_verdict_and_hides_them_again() {
    let mut s = session();
    let failing: Vec<_> = s.model().checks.iter().filter(|c| c.severity != Severity::Pass).map(|c| c.kind.label().to_string()).collect();
    assert!(!failing.is_empty(), "the default case needs attention");
    let rows = |s: &BushingSession| card(&s.result_cards(), "bushing-verdict").items.len();
    assert_eq!(rows(&s), 1, "the verdict alone");
    assert!(verdict_actions(&s).contains(&"Why?".to_string()));
    act(&mut s, BushingAct::Why);
    let verdict = texts(&[card(&s.result_cards(), "bushing-verdict")]);
    for f in &failing {
        assert!(verdict.contains(f.as_str()), "{f} listed: {verdict}");
    }
    assert!(verdict_actions(&s).contains(&"Hide checks".to_string()));
    act(&mut s, BushingAct::Why);
    assert_eq!(rows(&s), 1);
}

#[test]
fn a_fixes_link_unfolds_the_fixes_card_and_applying_a_fix_clears_that_failure() {
    let mut s = session();
    edit(&mut s, NumberTarget::EdgeDist, "0.32");
    let n_before = s.failing();
    assert!(n_before > 0, "an edge distance this small fails: {:?}", s.model().checks);
    assert!(texts(&[card(&s.result_cards(), "bushing-verdict")]).starts_with("REVIEW"));
    assert!(card(&s.result_cards(), "bushing-fixes").folded, "folded until asked for");
    let kind = s.model().recommendations.first().map(|r| r.fixes).expect("a fix is offered");
    act(&mut s, BushingAct::ShowFixes(kind));
    let fixes = card(&s.result_cards(), "bushing-fixes");
    assert!(!fixes.folded && fixes.title == format!("Fixes for {}", kind.label()));
    let apply = fixes.items.iter().find_map(|i| if let DockItem::Line { text, action: Some(UiAction::Editor(EditorAction::Bushing(a @ BushingAct::ApplyFix(_)))), .. } = i { (text == "Apply this fix").then_some(*a) } else { None }).expect("an Apply row");
    s.do_act(apply);
    assert!(s.failing() < n_before, "the fix reduced the failures: {n_before} -> {}", s.failing());
    assert!(results(&s).contains("ok Applied:"));
    act(&mut s, BushingAct::Undo);
    assert_eq!(s.failing(), n_before, "undo takes the fix back");
}

#[test]
fn the_edge_check_runs_off_the_ui_thread_and_fills_the_folded_cross_checks_card() {
    let mut s = session();
    let c = card(&s.result_cards(), "bushing-edge-check");
    assert!(c.folded && c.tag == "not run" && texts(&[c]).contains("Not run."));
    act(&mut s, BushingAct::EdgeCheck(false));
    assert!(s.st.edge_job.is_some(), "running");
    assert_eq!(card(&s.result_cards(), "bushing-edge-check").tag, "running");
    wait(&mut s, "the edge check", |s| s.st.edge_check.is_some());
    let c = card(&s.result_cards(), "bushing-edge-check");
    let t = texts(&[c.clone()]);
    assert!(t.contains("Superposition") && t.contains("Legacy solver"), "{t}");
    assert_eq!(c.tag, "current");
    act(&mut s, BushingAct::EdgeCheck(false));
    s.do_act(BushingAct::CancelEdgeCheck);
    assert!(s.st.edge_job.is_none());
}

#[test]
fn quick_fe_meshes_and_solves_on_demand_beside_the_closed_form_and_an_edit_makes_it_stale() {
    let mut s = session();
    assert!(card(&s.result_cards(), "bushing-quick-fe").folded, "folded until run");
    act(&mut s, BushingAct::QuickFe);
    assert!(s.solving(), "the solve starts at once, meshing what it needs");
    wait(&mut s, "the solve", |s| s.fea().current_solution().is_some() && !s.solving());
    let c = card(&s.result_cards(), "bushing-quick-fe");
    assert!(!c.folded && c.tag == "current");
    let fe = texts(&[c]);
    assert!(fe.contains("FE mean / peak") && fe.contains("Lam\u{e9} closed form") && fe.contains("FE minus Lam\u{e9}") && fe.contains("Max von Mises"), "{fe}");
    edit(&mut s, NumberTarget::Load, "2000");
    let c = card(&s.result_cards(), "bushing-quick-fe");
    assert_eq!(c.tag, "stale");
    assert!(texts(&[c]).contains("Inputs changed since this solve"));
}

#[test]
fn the_send_dialog_shows_what_goes_and_closes_on_escape() {
    let mut s = session();
    act(&mut s, BushingAct::SendDialog);
    let d = s.workbench().unwrap().dialog.expect("the dialog is open");
    assert!(d.rows.iter().any(|(l, _)| l == "Housing") && d.rows.iter().any(|(l, _)| l == "Load"));
    let opt = |label: &str| d.options.iter().find(|o| o.label == label).unwrap_or_else(|| panic!("no option {label}"));
    let solid = opt("3D solid");
    assert!(!solid.enabled && !solid.on && solid.detail.contains("#241"), "{}", solid.detail);
    let linked = opt("Keep linked");
    assert!(linked.on && !linked.enabled, "the link is how the FEA Send finds the case");
    assert!(d.confirm.enabled);
    // The dialog is modal: keys go to it, and Esc closes it.
    assert_eq!(s.key(Key::Char('z'), Mods { command: true, ..Mods::default() }), KeyResult::Handled);
    s.key(Key::Escape, Mods::default());
    assert!(s.workbench().unwrap().dialog.is_none());
    act(&mut s, BushingAct::SendDialog);
    act(&mut s, BushingAct::CloseDialog);
    assert!(s.workbench().unwrap().dialog.is_none());
}

#[test]
fn inputs_that_describe_no_bushing_in_a_housing_block_send_and_quick_fe_and_say_why() {
    let mut s = session();
    edit(&mut s, NumberTarget::EdgeDist, "0.2");
    assert!(s.fe_note.is_some());
    assert!(s.status().contains("Nothing to draw yet") && s.status().contains("edge distance"), "{}", s.status());
    let send = s.workbench().unwrap().actions.into_iter().find(|e| e.label == "Send joint to FEM...").unwrap();
    assert!(!send.enabled && send.reason.as_deref().is_some_and(|r| r.contains("edge distance")));
    act(&mut s, BushingAct::SendDialog);
    assert!(s.workbench().unwrap().dialog.is_none(), "no dialog without an FE model");
    assert!(s.status().starts_with("Cannot send to the FEA Workbench"), "{}", s.status());
    act(&mut s, BushingAct::QuickFe);
    assert!(!s.solving() && s.status().starts_with("Quick FE check not run"), "{}", s.status());
    edit(&mut s, NumberTarget::EdgeDist, "0.9");
    assert!(s.fe_note.is_none() && s.send_availability().is_ok());
}

#[test]
fn the_view_controls_choose_panes_colouring_and_the_standard_view() {
    let mut s = session();
    s.scene().unwrap();
    assert!(!s.needs_scene() && !s.geometry_view() && s.shows_result() && s.field_unit("") == "psi");
    act(&mut s, BushingAct::Field(drawing::Field::Parts));
    assert!(s.needs_scene() && s.geometry_view() && !s.shows_result() && s.field_unit("").is_empty());
    s.scene().unwrap();
    act(&mut s, BushingAct::QuarterCut);
    assert!(s.needs_scene() && !s.quarter_cut);
    assert_eq!(s.workbench().unwrap().model_title, "whole joint");
    act(&mut s, BushingAct::View(WorkbenchView::Sketch(1)));
    assert_eq!(s.workbench().unwrap().show, WorkbenchView::Sketch(1));
    assert!(s.take_view().is_none());
    act(&mut s, BushingAct::ResetView);
    assert_eq!(s.take_view(), Some(StandardView::Isometric));
    assert!(s.take_view().is_none(), "taken once");
    // The picture is read-only: a click picks nothing.
    let cam = Camera { yaw: 0.0, pitch: 0.0, zoom: 1.0, aspect: 1.0, pan: [0.0, 0.0] };
    assert!(!s.click([100.0, 100.0], [800.0, 600.0], &cam));
    assert!(s.take_popup().is_none());
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
fn the_menus_say_why_something_is_unavailable() {
    let mut s = session();
    let undo = |s: &BushingSession| s.menus()[1].items.iter().find_map(|i| if let Item::Entry(e) = i { (e.label == "Undo").then(|| e.clone()) } else { None }).unwrap();
    assert!(!undo(&s).enabled && undo(&s).reason.as_deref() == Some("nothing to undo"));
    edit(&mut s, NumberTarget::HousingWidth, "2.0");
    assert!(undo(&s).enabled, "there is something to undo after the edit");
    let tools = labels(s.menus()[3].items.clone());
    assert!(tools.contains(&"Quick FE check".to_string()) && tools.contains(&"Edge check".to_string()));
    let view = labels(s.context_menu());
    assert!(view.contains(&"Reset view".to_string()) && view.contains(&"Parts".to_string()));
}

#[test]
fn uncertainty_rows_edit_both_cvs_without_changing_the_case_inputs() {
    let mut s = session();
    let inputs = BushingFile::capture(&s.st.model, "test");
    let rows = s.st.edge_settings.rows(&s.st.model);
    let actions: Vec<_> = card(&s.result_cards(), "bushing-edge-check")
        .items
        .iter()
        .filter_map(|item| if let DockItem::Field { action: Some(UiAction::Editor(EditorAction::Bushing(BushingAct::EdgeCv(id)))), .. } = item { Some(*id) } else { None })
        .collect();
    assert_eq!(actions, rows.iter().map(|(id, _, _)| *id).collect::<Vec<_>>());
    for (id, _, _) in &rows {
        act(&mut s, BushingAct::EdgeCv(*id));
        assert!(s.prompting());
        type_text(&mut s, "12.5");
        s.key(Key::Enter, Mods::default());
        assert!(!s.prompting());
        assert_eq!(s.st.edge_settings.rows(&s.st.model).iter().find(|(row, _, _)| row == id).unwrap().2, 0.125);
    }
    act(&mut s, BushingAct::EdgeCv(None));
    type_text(&mut s, "101");
    s.key(Key::Enter, Mods::default());
    assert!(s.status.contains("between 0 and 100"));
    assert_eq!(s.st.edge_settings.strength_cv, 0.125);
    act(&mut s, BushingAct::EdgeCv(None));
    type_text(&mut s, "3");
    s.key(Key::Escape, Mods::default());
    assert_eq!(s.st.edge_settings.strength_cv, 0.125);
    assert_eq!(serde_json::to_value(BushingFile::capture(&s.st.model, "test")).unwrap(), serde_json::to_value(inputs).unwrap());
    assert!(s.undo.is_empty(), "uncertainty does not edit the engineering case");
    s.poll();
    assert!(s.st.edge_job.is_none(), "debounce waits");
    wait(&mut s, "updated uncertainty", |s| s.st.edge_check.as_ref().is_some_and(|r| r.report.strength_cv == 0.125));
    assert!(s.st.report_text().contains("Strength CV 12.5 %"));
}
