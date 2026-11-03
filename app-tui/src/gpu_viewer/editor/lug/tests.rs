use super::*;
use crate::gpu_viewer::editor::authoring::Authoring;
use crate::gpu_viewer::editor::session::{EditorAction, Key, Mods};
use crate::gpu_viewer::ui::CardRow;
use crate::toolboxes::lug_analysis::model::{NumberTarget, SolverChoice};
use crate::toolboxes::material_lookup::model::catalog;

fn dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("lug-session-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}
/// The default lug, elastic only (the collapse runs are the solver's to test; they only slow these down).
fn session(tag: &str) -> LugSession {
    let mut m = LugUiModel::default();
    m.plastic = false;
    LugSession::new(&LugFile::capture(&m, "clevis lug"), dir(tag).join("case.json"))
}
fn act(s: &mut LugSession, a: LugAct) {
    s.act(EditorAction::Lug(a));
}
fn type_text(s: &mut LugSession, text: &str) {
    for c in text.chars() {
        s.key(Key::Char(c), Mods::default());
    }
}
fn edit(s: &mut LugSession, t: NumberTarget, value: &str) {
    act(s, LugAct::Row(FieldRow::Number(t)));
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
/// Poll until the analysis of the inputs shown is in, or has failed (the window polls every 250 ms).
fn wait_for_run(s: &mut LugSession) {
    let start = Instant::now();
    while s.current_run().is_none() && s.error().is_none() {
        assert!(start.elapsed() < Duration::from_secs(120), "the analysis never answered");
        s.poll();
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn it_opens_on_the_models_inputs_draws_the_parts_then_the_kernel_field() {
    let mut s = session("open");
    assert_eq!(s.title(), "Lug Analysis | clevis lug");
    assert!(s.benches().is_empty());
    assert_eq!(s.initial_view(), Some(StandardView::Front));
    assert_eq!(s.ribbon().into_iter().map(|t| t.title).collect::<Vec<_>>(), ["Inputs", "Results", "Report"]);
    // One card per section of the input rows, then the result and the drawing.
    assert_eq!(ids(&s.inspector()), ["lug-lug-geometry", "lug-material", "lug-bushing", "lug-pin", "lug-load", "lug-advanced-settings", "lug-result", "lug-drawing"]);
    let t = texts(&s.inspector());
    assert!(t.contains("Hole Diameter D") && t.contains("Head Shape  Round (e = W/2)") && t.contains("Pin Load P"), "{t}");
    assert!(t.contains("Advanced settings  Show, ") && t.contains("Bushing  Off (pin bears on the hole)"), "{t}");
    assert_eq!(s.field_unit("von Mises stress"), "psi");
    assert_eq!(s.field_unit("Displacement"), "in");
    // The first analysis starts at once; until it answers the picture is the parts in the colour key.
    assert!(s.running() || s.current_run().is_some());
    if s.current_run().is_none() {
        assert!(s.geometry_view() && !s.shows_result());
        assert_eq!(s.scene().unwrap().field_label, "part (see color key)");
        assert!(texts(&s.inspector()).contains("coloured by part"));
        assert_eq!(s.status(), "Analysing the lug...");
    }
    wait_for_run(&mut s);
    assert!(s.error().is_none(), "{:?}", s.error());
    assert!(s.shows_result() && !s.geometry_view());
    assert!(s.needs_scene(), "the answer redraws");
    assert_eq!(s.scene().unwrap().field_label, "von Mises stress");
    assert!(!s.needs_scene());
    let cards = s.inspector();
    for id in ["lug-contact", "lug-stress", "lug-bore", "lug-model"] {
        assert!(ids(&cards).contains(&id.to_string()), "{id} in {:?}", ids(&cards));
    }
    assert!(!ids(&cards).contains(&"lug-ultimate".to_string()), "no collapse run asked for");
    let t = texts(&cards);
    let run = s.current_run().unwrap();
    assert!(t.contains(&format!("Peak von Mises  {:.1} ksi", run.solution.peak_von_mises / 1000.0)), "{t}");
    assert!(t.contains("Solver  kernel (fea-core)") && t.contains("contoured by the kernel's nodal field"), "{t}");
    let verdict = s.status();
    assert!(verdict.starts_with("PASS") || verdict.starts_with("REVIEW") || verdict.starts_with("FAIL"), "{verdict}");
}

#[test]
fn a_typed_value_reaches_the_solver_and_undo_returns() {
    let mut s = session("edit");
    wait_for_run(&mut s);
    let before = s.current_run().unwrap().solution.peak_hoop;
    edit(&mut s, NumberTarget::Load, "6000");
    assert_eq!(s.model().load, 6000.0);
    assert!(s.dirty() && s.title().ends_with('*') && s.needs_scene());
    // The last answer stays on show (marked stale) until the new one is in.
    assert!(s.current_run().is_none() && s.run.is_some());
    assert!(texts(&s.inspector()).contains("the last completed analysis is shown") || texts(&s.inspector()).contains("the inputs have changed"));
    wait_for_run(&mut s);
    assert!(s.current_run().unwrap().solution.peak_hoop > before);
    // Typing the value it already has is not an edit; a non-number and an out-of-range value are refused with a reason.
    let depth = s.undo.len();
    edit(&mut s, NumberTarget::Load, "6000");
    assert_eq!(s.undo.len(), depth);
    edit(&mut s, NumberTarget::Load, "-");
    assert!(s.status().contains("is not a number"), "{}", s.status());
    edit(&mut s, NumberTarget::Friction, "-5");
    assert_eq!(s.status(), "Friction Coefficient not changed: -5 is out of range.");
    assert_eq!(s.undo.len(), depth);
    // Esc leaves the value alone.
    act(&mut s, LugAct::Row(FieldRow::Number(NumberTarget::Load)));
    type_text(&mut s, "9");
    s.key(Key::Escape, Mods::default());
    assert_eq!(s.model().load, 6000.0);
    // A hole wider than the lug makes no lug: said, and nothing drawn.
    edit(&mut s, NumberTarget::HoleDia, "1.9");
    assert!(s.status().starts_with("Cannot analyse"), "{}", s.status());
    assert!(texts(&s.inspector()).contains("Nothing to draw"));
    assert_eq!(s.scene().unwrap().positions.len(), 3);
    s.key(Key::Char('z'), Mods { command: true, ..Mods::default() });
    assert_eq!(s.model().hole_dia, 0.5);
    s.key(Key::Char('z'), Mods { command: true, ..Mods::default() });
    assert_eq!(s.model().load, 4000.0);
    s.key(Key::Char('z'), Mods { command: true, shift: true });
    assert_eq!(s.model().load, 6000.0);
    // Back to the inputs analysed first: their answer is still the one held, so it is current again only once rerun.
    act(&mut s, LugAct::Undo);
    wait_for_run(&mut s);
    assert_eq!(s.current_run().unwrap().solution.peak_hoop, before);
}

#[test]
fn switches_and_the_advanced_section_follow_the_model_rows() {
    let mut s = session("rows");
    // Showing the advanced rows is not an edit of the lug.
    act(&mut s, LugAct::Row(FieldRow::MeshSection));
    assert!(s.model().mesh_open && !s.dirty() && s.undo.is_empty());
    let t = texts(&s.inspector());
    assert!(t.contains("Advanced settings  Hide") && t.contains("Mesh Size Test  Not run: click runs a brief test") && t.contains("Solver  General kernel (fea-core)"), "{t}");
    assert!(ids(&s.inspector()).contains(&"lug-analysis".to_string()));
    act(&mut s, LugAct::Row(FieldRow::ToggleHeadShape));
    assert!(!s.model().head_round);
    assert!(texts(&s.inspector()).contains("Edge Distance e"));
    act(&mut s, LugAct::Row(FieldRow::ToggleBushing));
    assert!(s.model().bushing);
    assert!(texts(&s.inspector()).contains("Bushing Material"));
    // A preset replaces a typed element count.
    edit(&mut s, NumberTarget::ElementsAround, "40");
    assert_eq!(s.model().elements_around, Some(40));
    act(&mut s, LugAct::Row(FieldRow::ToggleMeshDensity));
    assert_eq!(s.model().elements_around, None);
    act(&mut s, LugAct::Row(FieldRow::ToggleSolver));
    assert_ne!(s.model().solver, SolverChoice::Kernel);
    assert_eq!(s.undo.len(), 5);
    act(&mut s, LugAct::Undo);
    assert_eq!(s.model().solver, SolverChoice::Kernel);
    assert_eq!(s.redo.len(), 1);
}

#[test]
fn the_mesh_test_row_applies_a_recommendation_for_these_inputs_and_starts_a_test_otherwise() {
    let mut s = session("mesh-test");
    act(&mut s, LugAct::Row(FieldRow::MeshSection));
    assert!(s.mesh_advice_text().starts_with("Not run"));
    // A recommendation made for the inputs shown is shown in the row, and a click applies it (one undo step).
    let sig = mesh_test::signature(&s.model().input().unwrap());
    s.mesh_advice = Some((sig, MeshAdvice { rows: vec![], recommended: 54, converged: true, collapse_reference: None, total_ms: 1.0 }));
    assert!(s.mesh_advice_text().contains("Recommended 54"), "{}", s.mesh_advice_text());
    act(&mut s, LugAct::Row(FieldRow::MeshAdvice));
    assert_eq!(s.model().elements_around, Some(54));
    assert_eq!(s.undo.len(), 1);
    // Changing the geometry makes it stale; a failure is shown in the row.
    edit(&mut s, NumberTarget::Thickness, "0.3");
    assert!(s.mesh_advice_text().starts_with("Inputs changed"), "{}", s.mesh_advice_text());
    s.mesh_error = Some("boom".into());
    assert_eq!(s.mesh_advice_text(), "Failed: boom");
    // A click on stale advice starts a test for the inputs shown; a second click while it runs starts no other.
    s.mesh_error = None;
    act(&mut s, LugAct::Row(FieldRow::MeshAdvice));
    let job = s.mesh_job.clone().expect("a mesh test runs");
    assert_eq!(job.1, mesh_test::signature(&s.model().input().unwrap()));
    assert!(s.mesh_advice_text().starts_with("Testing mesh sizes"));
    act(&mut s, LugAct::Row(FieldRow::MeshAdvice));
    assert_eq!(s.mesh_job, Some(job), "no second test while one runs");
}

#[test]
fn the_legacy_solver_gives_no_field_so_the_picture_stays_the_parts() {
    let mut s = session("legacy");
    while s.model().solver != SolverChoice::Legacy {
        act(&mut s, LugAct::Row(FieldRow::ToggleSolver));
    }
    wait_for_run(&mut s);
    assert!(s.error().is_none(), "{:?}", s.error());
    assert!(s.current_run().unwrap().field.is_none());
    assert!(!s.shows_result() && s.geometry_view());
    assert_eq!(s.scene().unwrap().field_label, "part (see color key)");
    assert!(texts(&s.inspector()).contains("the legacy solver gives no nodal field"));
}

#[test]
fn the_material_picker_searches_the_catalog_and_picks() {
    let mut s = session("pick");
    act(&mut s, LugAct::Row(FieldRow::OpenMaterialPicker));
    let card = s.value_card().expect("the material picker");
    assert_eq!(card.title, "Lug material");
    let current = catalog()[s.model().material_index].name;
    assert!(card.rows.iter().any(|r| matches!(r, CardRow::Choice { label, current: true, .. } if label == current)), "{:?}", choices(&card));
    assert!(choices(&card).len() <= 12);
    // The search narrows; a pick sets the material and closes.
    s.card_field(0, "7075".into());
    let hits = choices(&s.value_card().unwrap());
    assert!(!hits.is_empty() && hits.iter().all(|h| h.contains("7075")), "{hits:?}");
    let i = catalog().iter().position(|m| m.name == hits[0]).unwrap();
    act(&mut s, LugAct::PickMaterial(i));
    assert!(s.value_card().is_none());
    assert_eq!(s.model().material_index, i);
    assert_eq!(s.undo.len(), 1);
    // Picking the current one again is not an edit; nothing matches nonsense; Esc closes.
    act(&mut s, LugAct::Row(FieldRow::OpenMaterialPicker));
    act(&mut s, LugAct::PickMaterial(i));
    assert_eq!(s.undo.len(), 1);
    act(&mut s, LugAct::Row(FieldRow::OpenMaterialPicker));
    s.card_field(0, "zzzz-no-such".into());
    assert!(s.value_card().unwrap().rows.iter().any(|r| matches!(r, CardRow::Text(t) if t == "No material matches the search.")));
    s.key(Key::Escape, Mods::default());
    assert!(s.value_card().is_none());
    // The bushing material has its own picker.
    act(&mut s, LugAct::Row(FieldRow::ToggleBushing));
    act(&mut s, LugAct::Row(FieldRow::OpenBushingMaterialPicker));
    assert_eq!(s.value_card().unwrap().title, "Bushing material");
}

#[test]
fn save_and_reopen_round_trips_the_lug_and_export_writes_the_report() {
    let mut s = session("save");
    act(&mut s, LugAct::Row(FieldRow::ToggleHeadShape));
    edit(&mut s, NumberTarget::Edge, "0.9");
    edit(&mut s, NumberTarget::LoadAngle, "30");
    act(&mut s, LugAct::Export);
    assert!(s.status().starts_with("Nothing to export yet"), "{}", s.status());
    act(&mut s, LugAct::Save);
    assert!(!s.dirty(), "{}", s.status());
    let back = LugSession::open(&s.path).unwrap();
    assert_eq!(LugFile::capture(back.model(), "x"), LugFile::capture(s.model(), "x"));
    wait_for_run(&mut s);
    assert!(!s.current_run().unwrap().field.as_ref().unwrap().half, "an oblique load is a full model");
    act(&mut s, LugAct::Export);
    let report = std::fs::read_to_string(s.path.with_extension("report.txt")).unwrap();
    assert_eq!(report, model::report_text(s.current_run().unwrap()));
    // A file another program changed is not overwritten.
    crate::persistence::write(&s.path, "{}").unwrap();
    edit(&mut s, NumberTarget::Load, "4100");
    act(&mut s, LugAct::Save);
    assert!(s.status().starts_with("Save conflict"), "{}", s.status());
}
