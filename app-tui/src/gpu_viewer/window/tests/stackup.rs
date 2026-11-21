//! The Stackup workbench inspector, selection and drags.
use super::*;

fn stackup_viewer() -> Viewer {
    use crate::gpu_viewer::editor::stackup::StackupSession;
    viewer(Some(Box::new(
        StackupSession::open(&std::env::temp_dir().join("never-written-stackup-interaction.json"))
            .unwrap(),
    )))
}
fn scroll_stackup_inspector(v: &mut Viewer, delta: f32) {
    let tab = target(v, "Member settings");
    move_to(v, (tab.0, tab.1 + 100.0));
    v.test_events.push(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, delta),
        phase: egui::TouchPhase::Move,
        modifiers: Default::default(),
    });
    for _ in 0..12 {
        frame(v);
    }
}
#[test]
fn the_stackup_inspector_commits_enter_and_blur_without_duplicate_fields() {
    let mut v = stackup_viewer();
    assert_eq!(
        labels(&v)
            .iter()
            .filter(|label| label.as_str() == "Current hole nominal (in) input")
            .count(),
        1
    );
    assert!(!labels(&v)
        .iter()
        .any(|label| label == "Selected member" || label == "Stack configuration"));
    replace_page_text(&mut v, "Current hole nominal (in) input", "0.5");
    let page = v
        .editor
        .as_ref()
        .unwrap()
        .document()
        .unwrap()
        .precision
        .unwrap();
    assert!(!page.current);
    egui_key(&mut v, egui::Key::Enter);
    let page = v
        .editor
        .as_ref()
        .unwrap()
        .document()
        .unwrap()
        .precision
        .unwrap();
    assert!(page.current);
    assert_eq!(page.inputs.members[0].hole.nominal, 0.5);
    assert!(page.result.unwrap().layers[0].cleanup_diameter > 0.5);
    replace_page_text(&mut v, "Current hole nominal (in) input", "0.6");
    let thickness = target(&v, "Thickness along stack z (in) input");
    click(&mut v, thickness);
    let page = v
        .editor
        .as_ref()
        .unwrap()
        .document()
        .unwrap()
        .precision
        .unwrap();
    assert!(page.current);
    assert_eq!(page.inputs.members[0].hole.nominal, 0.6);
    replace_page_text(&mut v, "Current hole nominal (in) input", "0.65");
    let stack = target(&v, "Stack setup");
    click(&mut v, stack);
    assert!(
        v.editor
            .as_ref()
            .unwrap()
            .document()
            .unwrap()
            .precision
            .unwrap()
            .current
    );
    let member = target(&v, "Member settings");
    click(&mut v, member);
    scroll_stackup_inspector(&mut v, -400.0);
    let datum = target(&v, "Datum direction");
    click(&mut v, datum);
    for _ in 0..8 {
        frame(&mut v);
    }
    scroll_stackup_inspector(&mut v, -220.0);
    replace_page_text(&mut v, "Datum normal angle (deg) input", "0.5");
    assert!(
        !v.editor
            .as_ref()
            .unwrap()
            .document()
            .unwrap()
            .precision
            .unwrap()
            .current
    );
    let datum = target(&v, "Datum direction");
    click(&mut v, datum);
    let page = v
        .editor
        .as_ref()
        .unwrap()
        .document()
        .unwrap()
        .precision
        .unwrap();
    assert!(page.current);
    assert_eq!(page.inputs.members[0].reference_normal_deg, 0.5);
    scroll_stackup_inspector(&mut v, 600.0);
    replace_page_text(&mut v, "Current hole nominal (in) input", "unfinished");
    egui_key(&mut v, egui::Key::Enter);
    let page = v
        .editor
        .as_ref()
        .unwrap()
        .document()
        .unwrap()
        .precision
        .unwrap();
    assert!(!page.current && page.result.is_none());
}
#[test]
fn the_stackup_member_selection_keeps_edits_with_their_member() {
    let mut v = stackup_viewer();
    replace_page_text(&mut v, "Current hole nominal (in) input", "0.45");
    let second = target(&v, "Member 2");
    click(&mut v, second);
    let page = v
        .editor
        .as_ref()
        .unwrap()
        .document()
        .unwrap()
        .precision
        .unwrap();
    assert_eq!(page.selected, 1);
    assert!(page.current);
    assert_eq!(page.inputs.members[0].hole.nominal, 0.45);
    assert_eq!(page.inputs.members[1].hole.nominal, 0.3);
    replace_page_text(&mut v, "Current hole nominal (in) input", "0.55");
    egui_key(&mut v, egui::Key::Enter);
    let page = v
        .editor
        .as_ref()
        .unwrap()
        .document()
        .unwrap()
        .precision
        .unwrap();
    assert!(page.current);
    assert_eq!(page.inputs.members[0].hole.nominal, 0.45);
    assert_eq!(page.inputs.members[1].hole.nominal, 0.55);
}
#[test]
fn the_compact_stackup_inspector_can_edit_pilot_registration() {
    let mut v = stackup_viewer();
    v.size_override = Some((780, 900));
    frame(&mut v);
    move_to(&mut v, (750.0, 450.0));
    v.test_events.push(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, -600.0),
        phase: egui::TouchPhase::Move,
        modifiers: Default::default(),
    });
    for _ in 0..12 {
        frame(&mut v);
    }
    let setup = target(&v, "Stack setup");
    click(&mut v, setup);
    replace_page_text(
        &mut v,
        "Pilot position from stack entrance (in) input",
        "0.5",
    );
    egui_key(&mut v, egui::Key::Enter);
    replace_page_text(
        &mut v,
        "Pilot center offset limit (in, radial) input",
        "0.001",
    );
    egui_key(&mut v, egui::Key::Enter);
    let page = v
        .editor
        .as_ref()
        .unwrap()
        .document()
        .unwrap()
        .precision
        .unwrap();
    assert!(page.current);
    assert_eq!(page.inputs.pilot_z, 0.5);
    assert_eq!(page.inputs.pilot_offset, 0.001);
}
fn stackup_bushing_viewer() -> Viewer {
    use crate::gpu_viewer::editor::stackup::{StackupAct, StackupSession};
    let mut session = StackupSession::open(
        &std::env::temp_dir().join("never-written-stackup-component-test.json"),
    )
    .unwrap();
    session.inputs.bushing_enabled = true;
    session.act(session::EditorAction::Stackup(StackupAct::Recalculate));
    viewer(Some(Box::new(session)))
}
#[test]
fn the_stackup_drags_only_the_grabbed_member_or_bushing() {
    let mut v = stackup_bushing_viewer();
    let before = v
        .editor
        .as_ref()
        .unwrap()
        .document()
        .unwrap()
        .precision
        .unwrap();
    let dirty = v.editor.as_ref().unwrap().dirty();
    assert!(labels(&v).contains(&"Finished axis".into()));
    assert!(labels(&v).contains(&"Original axis".into()));
    let start = target(&v, "Drag member 1");
    drag(&mut v, start, (start.0, start.1 - 120.0));
    let moved = v.gui.stackup_component_offsets();
    assert!(moved[0].1[0] < 0.0);
    assert_eq!(moved[0].1[1], 0.0);
    assert_eq!(moved[1].1, [0.0, 0.0]);
    let start = target(&v, "Drag member 1");
    drag(&mut v, start, (start.0, start.1 + 60.0));
    assert!(v.gui.stackup_component_offsets()[0].1[0] > moved[0].1[0]);
    let fitted = target(&v, "Fitted");
    click(&mut v, fitted);
    assert!(v
        .gui
        .stackup_component_offsets()
        .iter()
        .all(|(_, offset)| *offset == [0.0, 0.0]));
    let start = target(&v, "Drag bushing 1");
    drag(&mut v, start, (start.0, start.1 - 100.0));
    let moved = v.gui.stackup_component_offsets();
    assert_eq!(moved[0].1[0], 0.0);
    assert!(moved[0].1[1] < 0.0);
    assert_eq!(moved[1].1, [0.0, 0.0]);
    let canvas = target(&v, "Stack geometry drag");
    let blank = (canvas.0 - 450.0, canvas.1 - 180.0);
    drag(&mut v, blank, (blank.0, blank.1 - 80.0));
    assert_eq!(v.gui.stackup_component_offsets(), moved);
    let after = v
        .editor
        .as_ref()
        .unwrap()
        .document()
        .unwrap()
        .precision
        .unwrap();
    assert_eq!(before.inputs, after.inputs);
    assert_eq!(before.result, after.result);
    assert_eq!(v.editor.as_ref().unwrap().dirty(), dirty);
    let all = target(&v, "Explode all");
    click(&mut v, all);
    assert!(v
        .gui
        .stackup_component_offsets()
        .iter()
        .any(|(_, offset)| offset[0] > 0.0));
    let fitted = target(&v, "Fitted");
    click(&mut v, fitted);
    assert!(v
        .gui
        .stackup_component_offsets()
        .iter()
        .all(|(_, offset)| *offset == [0.0, 0.0]));
}
#[test]
fn component_poses_survive_names_but_reset_for_geometry_changes() {
    let mut v = stackup_viewer();
    let start = target(&v, "Drag member 1");
    drag(&mut v, start, (start.0, start.1 - 100.0));
    let moved = v.gui.stackup_component_offsets();
    assert!(moved[0].1[0] < 0.0);
    replace_page_text(&mut v, "Member name input", "Renamed member");
    egui_key(&mut v, egui::Key::Enter);
    assert_eq!(v.gui.stackup_component_offsets(), moved);
    replace_page_text(&mut v, "Current hole nominal (in) input", "0.4");
    egui_key(&mut v, egui::Key::Enter);
    assert!(v
        .gui
        .stackup_component_offsets()
        .iter()
        .all(|(_, offset)| *offset == [0.0, 0.0]));
}
