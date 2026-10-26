use super::*;
use crate::gpu_viewer::editor::authoring::Authoring;
use crate::gpu_viewer::editor::bushing::file::BushingFile;
use crate::gpu_viewer::editor::session::{EditorAction, Key, Mods};
use crate::toolboxes::eccentric_bushing::model::NumberTarget;
use std::time::{Duration, Instant};

fn dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("eccentric-session-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}
fn session(tag: &str) -> EccentricSession {
    EccentricSession::open(&dir(tag).join("ecc.json")).unwrap()
}
fn act(s: &mut EccentricSession, a: EccentricAct) {
    s.act(EditorAction::Eccentric(a));
}
fn type_text(s: &mut EccentricSession, text: &str) {
    for c in text.chars() {
        s.key(Key::Char(c), Mods::default());
    }
}
fn edit(s: &mut EccentricSession, t: NumberTarget, value: &str) {
    act(s, EccentricAct::Row(FieldRow::Number(t)));
    assert!(s.prompting());
    type_text(s, value);
    s.key(Key::Enter, Mods::default());
}
fn wait(s: &mut EccentricSession, what: &str, done: impl Fn(&EccentricSession) -> bool) {
    let t = Instant::now();
    while !done(s) {
        s.poll();
        // A hang guard, not a speed check: the full suite runs many FE solves at once and starves this one.
        assert!(t.elapsed() < Duration::from_secs(300), "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}
fn texts(s: &EccentricSession) -> String {
    s.inspector().iter().flat_map(|c| c.items.iter()).filter_map(|i| i.plain_text()).collect::<Vec<_>>().join("\n")
}

#[test]
fn it_opens_with_inputs_the_bushing_case_and_two_steps() {
    let s = session("open");
    assert_eq!(s.title(), "Eccentric bushing | ecc");
    assert!(s.benches().is_empty());
    assert_eq!(s.key_labels()[3], "Bushing");
    assert_eq!(s.ribbon().into_iter().map(|t| t.title).collect::<Vec<_>>(), ["Inputs", "Analysis"]);
    let t = texts(&s);
    assert!(t.contains("Bore Offset e  0.0200 in") && t.contains("Boss OD / Bore  2.50 x"), "{t}");
    assert!(t.contains("no Bushing case at") && t.contains("Bore  0.5000 in"), "defaults stand, and it says so: {t}");
    assert!(!t.contains("Analyse this offset  Enter"), "the run rows are ribbon buttons, not inputs");
    assert!(s.step_state("Analysis") == Some(crate::gpu_viewer::editor::authoring::StepState::Todo));
}

#[test]
fn a_typed_offset_reaches_the_inputs_and_the_picture_and_undo_returns() {
    let mut s = session("edit");
    wait(&mut s, "the mesh", |s| s.fea().preview_summary().is_ok());
    let before = s.fe_key.clone();
    edit(&mut s, NumberTarget::Offset, "0.035");
    assert_eq!(s.ui().offset, 0.035);
    assert!(s.dirty());
    assert_eq!(s.fe_key.as_ref().unwrap().bushings[0].offset, [0.035, 0.0]);
    assert_ne!(s.fe_key, before);
    // Typing what is already there is not an edit.
    edit(&mut s, NumberTarget::Offset, "0.035");
    act(&mut s, EccentricAct::Undo);
    assert_eq!(s.ui().offset, 0.02, "one undo step: the repeat was not an edit");
    act(&mut s, EccentricAct::Redo);
    assert_eq!(s.ui().offset, 0.035);
    // A range the field clamps: a negative offset is zero.
    edit(&mut s, NumberTarget::Offset, "-1");
    assert_eq!(s.ui().offset, 0.0);
    // Choices flip in one click and an offset the wall cannot take is refused with the solver's reason.
    act(&mut s, EccentricAct::Row(FieldRow::ToggleHousing));
    assert!(s.ui().edge_limited);
    edit(&mut s, NumberTarget::Offset, "0.5");
    assert!(s.status().contains("No picture") || s.status().to_lowercase().contains("wall") || s.status().to_lowercase().contains("offset"), "{}", s.status());
}

#[test]
fn the_inputs_come_from_the_bushing_case_file_and_reload_picks_up_a_change() {
    let d = dir("case");
    let mut m = BushingModel::default();
    m.commit_number(crate::toolboxes::bushing::model::NumberTarget::BoreDia, 0.625);
    m.commit_number(crate::toolboxes::bushing::model::NumberTarget::IdBushing, 0.5);
    std::fs::write(d.join("bushing-case.json"), BushingFile::capture(&m, "wing").to_json()).unwrap();
    let mut s = EccentricSession::open(&d.join("ecc.json")).unwrap();
    assert!(texts(&s).contains("Bore  0.6250 in") && texts(&s).contains("Bushing ID  0.5000 in"), "{}", texts(&s));
    assert_eq!(s.fe_key.as_ref().unwrap().bushings[0].inner_diameter, 0.5);
    m.commit_number(crate::toolboxes::bushing::model::NumberTarget::Load, 2222.0);
    std::fs::write(d.join("bushing-case.json"), BushingFile::capture(&m, "wing").to_json()).unwrap();
    assert!(texts(&s).contains("Pin load  1000 lbf"));
    act(&mut s, EccentricAct::ReloadBushing);
    assert!(texts(&s).contains("Pin load  2222 lbf"), "{}", texts(&s));
}

#[test]
fn an_analysis_runs_off_the_ui_thread_and_the_solved_housing_bushing_and_pin_are_drawn() {
    let mut s = session("analyse");
    wait(&mut s, "the mesh", |s| s.fea().preview_summary().is_ok());
    assert!(s.scene().is_ok() && !s.shows_result());
    act(&mut s, EccentricAct::Analyse);
    assert!(s.solving() && s.status().contains("field export"), "{}", s.status());
    wait(&mut s, "the analysis", |s| s.st.analysis.is_some() && !s.solving());
    let a = &s.st.analysis.as_ref().unwrap().1;
    assert!(a.margin.is_finite() && a.torque_required > 0.0);
    let t = texts(&s);
    assert!(t.contains("HOLDS") || t.contains("SPINS"), "{t}");
    assert!(t.contains("Capacity, fit alone") && t.contains("Design capacity"), "{t}");
    assert!(s.result_is_current() && s.analysis_is_current());
    assert_eq!(s.step_state("Analysis"), Some(crate::gpu_viewer::editor::authoring::StepState::Done));
    // The window rebuilds its scene: it is the solved model now, with the pin, in psi.
    assert!(s.needs_scene());
    let scene = s.scene().unwrap();
    assert!(s.shows_result() && !s.geometry_view());
    assert!(scene.range[1] > scene.range[0] && scene.range[1] > 1000.0, "von Mises in psi: {:?}", scene.range);
    assert_eq!(s.field_unit("von Mises stress"), "psi");
    assert_eq!(scene.gain, 1.0, "a fit result opens at its true size");
    // The picture of the inputs comes back on request, and a change makes the result stale.
    act(&mut s, EccentricAct::ToggleResult);
    s.scene().unwrap();
    assert!(!s.shows_result());
    act(&mut s, EccentricAct::ToggleResult);
    edit(&mut s, NumberTarget::Offset, "0.03");
    assert!(!s.result_is_current() && !s.analysis_is_current());
    assert!(texts(&s).contains("Inputs changed since this run"));
    s.scene().unwrap();
    assert!(!s.shows_result(), "a stale result is not drawn");
    // Exports of the (stale) run say so rather than writing a mismatch; the current kinds are written beside the case.
    act(&mut s, EccentricAct::ExportReport);
    assert!(s.status().contains("Nothing to export"), "{}", s.status());
}

#[test]
fn a_cancelled_run_stops_and_the_inputs_stay_editable() {
    let mut s = session("cancel");
    act(&mut s, EccentricAct::Analyse);
    assert!(s.solving());
    s.key(Key::Escape, Mods::default());
    wait(&mut s, "the cancelled run to end", |s| !s.solving());
    assert!(s.st.analysis.is_none());
    assert!(s.status().contains("cancelled") || s.st.error.as_deref().is_some_and(|e| e.contains("cancelled")) || s.status().is_empty(), "{}", s.status());
    edit(&mut s, NumberTarget::LoadAngle, "45");
    assert_eq!(s.ui().load_angle, 45.0);
}

#[test]
fn a_saved_case_reopens_with_its_inputs_and_the_report_and_fields_are_written_beside_it() {
    let mut s = session("save");
    edit(&mut s, NumberTarget::BossFactor, "3");
    act(&mut s, EccentricAct::Save);
    assert!(!s.dirty());
    assert_eq!(EccentricSession::open(s.path()).unwrap().ui().boss_factor, 3.0);
    act(&mut s, EccentricAct::ExportFields);
    wait(&mut s, "the analysis for the export", |s| s.st.analysis.is_some() && !s.solving());
    s.poll();
    let vtu = std::fs::read_to_string(s.path().with_extension("vtu")).expect("the fields were written");
    assert!(vtu.contains("von_mises"));
    act(&mut s, EccentricAct::ExportReport);
    assert!(std::fs::read_to_string(s.path().with_extension("report.txt")).unwrap().contains("ECCENTRIC BUSHING"));
    act(&mut s, EccentricAct::ExportCsv);
    assert!(std::fs::read_to_string(s.path().with_extension("csv")).unwrap().contains("angle_deg,fit_pressure_psi"));
}
