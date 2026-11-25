//! The dashboard window as a shell (ADR-024): workbenches open in the window, park with their state and are closed
//! from the nav strip, and unsaved edits are never dropped without a question.
use super::*;
use crate::gpu_viewer::{
    dashboard::{Dashboard, Recents},
    nav::NavAct,
};

fn shell() -> Viewer {
    let mut v = new_viewer(ViewerProject::single(blank_scene("dashboard")), None);
    v.size_override = Some((1600, 900));
    v.nav = Some(Nav::default());
    crate::gpu_viewer::editor::handoff::serve_shell();
    v.dashboard = Some(Dashboard::new(Recents::default(), None));
    frame(&mut v);
    v
}
fn case(dir: &tempfile::TempDir, name: &str) -> std::path::PathBuf {
    dir.path().join(name)
}
fn tabs(v: &Viewer) -> Vec<&'static str> {
    v.nav_strip().map(|s| s.tabs.iter().map(|t| t.id).collect()).unwrap_or_default()
}
fn active(v: &Viewer) -> Option<&'static str> {
    v.shown_workbench()
}
/// Open the member workbench on a fresh file and add one node, so it holds unsaved edits.
fn dirty_members(v: &mut Viewer, file: std::path::PathBuf) {
    v.open_workbench("members", Some(file));
    frame(v);
    let add = target(v, "Add node");
    click(v, add);
    let at = viewport_centre(v);
    click(v, at);
    assert!(v.editor.as_ref().unwrap().dirty());
}

#[test]
fn a_workbench_opens_in_the_window_and_home_returns_to_the_dashboard() {
    let dir = tempfile::tempdir().unwrap();
    let mut v = shell();
    assert!(v.dashboard.is_some() && v.nav_strip().is_none(), "no strip until something is open");
    v.open_workbench("members", Some(case(&dir, "a.json")));
    frame(&mut v);
    assert!(v.dashboard.is_none() && v.editor.is_some(), "the workbench replaced the dashboard in this window");
    assert_eq!((tabs(&v), active(&v)), (vec!["members"], Some("members")));
    let home = target(&v, "Toolbench home");
    click(&mut v, home);
    assert!(v.dashboard.is_some() && v.editor.is_none());
    assert_eq!((tabs(&v), active(&v)), (vec!["members"], None), "the workbench stays open behind the dashboard");
    assert!(v.gui.find_target("Toolbench home").is_some(), "the strip is on the dashboard too");
}

#[test]
fn thread_illustration_keeps_its_display_without_changing_the_shared_view() {
    use crate::gpu_viewer::display::{default_key_colours, DisplayStyle};
    let dir = tempfile::tempdir().unwrap();
    let mut v = shell();
    v.open_workbench("thread", Some(case(&dir, "thread.json")));
    assert_eq!(v.view.style, DisplayStyle::Shaded);
    assert_ne!(v.view.key_colours, default_key_colours());
    v.dispatch(UiAction::Nav(NavAct::Home));
    assert_eq!(v.view.style, DisplayStyle::Wireframe);
    assert_eq!(v.view.key_colours, default_key_colours());
    v.dispatch(UiAction::Nav(NavAct::Open("thread")));
    assert_eq!(v.view.style, DisplayStyle::Shaded);
    assert_ne!(v.view.key_colours, default_key_colours());
}

#[test]
fn a_parked_workbench_keeps_its_unsaved_model_and_opening_it_again_reuses_the_tab() {
    let dir = tempfile::tempdir().unwrap();
    let mut v = shell();
    dirty_members(&mut v, case(&dir, "a.json"));
    let nodes = tree_labels(&v);
    v.dispatch(UiAction::Nav(NavAct::Home));
    frame(&mut v);
    v.open_workbench("members", None);
    frame(&mut v);
    assert!(v.editor.as_ref().unwrap().dirty(), "the edit survived the round trip");
    assert_eq!(tree_labels(&v), nodes);
    v.open_workbench("members", None);
    assert_eq!(tabs(&v), vec!["members"], "one tab per workbench");
}

#[test]
fn a_tab_holding_edits_asks_before_it_closes_and_keep_open_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let mut v = shell();
    dirty_members(&mut v, case(&dir, "a.json"));
    let close = target(&v, "Close Frame and Truss");
    click(&mut v, close);
    assert_eq!(tabs(&v), vec!["members"], "the tab is still open");
    assert_eq!(v.nav_strip().unwrap().ask_close, Some("members"));
    frame(&mut v); // a popup's first frame only measures it
    let keep = target(&v, "Keep open");
    click(&mut v, keep);
    assert_eq!(v.nav_strip().unwrap().ask_close, None);
    assert!(v.editor.as_ref().unwrap().dirty());
    let close = target(&v, "Close Frame and Truss");
    click(&mut v, close);
    frame(&mut v);
    let discard = target(&v, "Discard and close");
    click(&mut v, discard);
    assert!(tabs(&v).is_empty(), "discarded on request");
    assert!(v.dashboard.is_some() && v.editor.is_none());
}

#[test]
fn a_clean_tab_closes_at_once() {
    let dir = tempfile::tempdir().unwrap();
    let mut v = shell();
    v.open_workbench("members", Some(case(&dir, "a.json")));
    frame(&mut v);
    let close = target(&v, "Close Frame and Truss");
    click(&mut v, close);
    assert!(tabs(&v).is_empty() && v.dashboard.is_some());
}

#[test]
fn quitting_finds_unsaved_edits_in_a_parked_workbench_and_asks_about_them() {
    let dir = tempfile::tempdir().unwrap();
    let mut v = shell();
    dirty_members(&mut v, case(&dir, "a.json"));
    v.open_workbench("material-lookup", None);
    frame(&mut v);
    assert_eq!(active(&v), Some("material-lookup"));
    v.request_quit();
    assert!(!v.quit, "unsaved edits in a parked workbench stop the quit");
    assert_eq!(active(&v), Some("members"), "the workbench with the edits is brought forward");
    assert!(v.gui.confirm_close);
    v.request_quit();
    assert!(!v.quit, "asking again never discards");
    v.dispatch(UiAction::DiscardAndClose);
    assert!(v.quit, "nothing else holds edits");
}

#[test]
fn discarding_one_workbench_at_quit_moves_on_to_the_next_with_edits() {
    let dir = tempfile::tempdir().unwrap();
    let mut v = shell();
    dirty_members(&mut v, case(&dir, "a.json"));
    // A second workbench with edits: type a new pin load into Lug Analysis.
    v.open_workbench("lug", Some(case(&dir, "l.json")));
    frame(&mut v);
    let load = target(&v, "Pin Load P");
    click(&mut v, load);
    for c in "5200".chars() {
        v.input(Input::Key(Key::Character(c.to_string().into())));
    }
    v.input(Input::Key(Key::Named(NamedKey::Enter)));
    frame(&mut v);
    assert!(v.editor.as_ref().unwrap().dirty());
    v.request_quit();
    assert!(!v.quit);
    let first = active(&v);
    v.dispatch(UiAction::DiscardAndClose);
    assert!(!v.quit, "the other workbench still has edits");
    assert_ne!(active(&v), first);
    assert!(v.gui.confirm_close);
    v.dispatch(UiAction::DiscardAndClose);
    assert!(v.quit);
}

#[test]
fn a_workbench_that_fails_to_open_changes_nothing_and_says_why() {
    let dir = tempfile::tempdir().unwrap();
    let bad = case(&dir, "bad.json");
    std::fs::write(&bad, "{ not a model").unwrap();
    let mut v = shell();
    v.open_workbench("members", Some(bad));
    assert!(v.dashboard.is_some() && v.editor.is_none() && tabs(&v).is_empty());
    assert!(v.dashboard.as_ref().unwrap().message.as_deref().is_some_and(|m| m.contains("Could not open Frame and Truss")));
}

#[test]
fn another_file_replaces_a_clean_tab_but_never_a_dirty_one() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (case(&dir, "a.json"), case(&dir, "b.json"));
    let mut v = shell();
    v.open_workbench("members", Some(a.clone()));
    v.open_workbench("members", Some(b.clone()));
    assert_eq!(tabs(&v), vec!["members"]);
    assert_eq!(v.editor.as_ref().unwrap().path(), b, "a clean tab shows the newly requested file");
    dirty_members(&mut v, b.clone());
    v.open_workbench("members", Some(a));
    assert_eq!(v.editor.as_ref().unwrap().path(), b, "unsaved edits are not replaced");
    assert!(v.editor.as_ref().unwrap().dirty());
    assert!(v.editor.as_ref().unwrap().status().contains("unsaved edits"));
}

#[test]
fn escape_on_the_dashboard_keeps_parked_workbenches_and_the_window() {
    let dir = tempfile::tempdir().unwrap();
    let mut v = shell();
    v.input(Input::Key(Key::Named(NamedKey::Escape)));
    assert!(v.quit, "an empty dashboard closes on Escape as before");
    let mut v = shell();
    v.open_workbench("members", Some(case(&dir, "a.json")));
    v.dispatch(UiAction::Nav(NavAct::Home));
    v.input(Input::Key(Key::Named(NamedKey::Escape)));
    assert!(!v.quit);
}

#[test]
fn the_switcher_opens_with_ctrl_j_filters_and_enter_opens_the_first_match() {
    let mut v = shell();
    v.input(Input::Modifiers(ModifiersState::CONTROL));
    v.input(Input::Key(Key::Character("j".into())));
    frame(&mut v);
    assert!(v.gui.find_target("Switch to Search").is_some(), "{:?}", v.gui.target_labels());
    type_text(&mut v, "material");
    egui_key(&mut v, egui::Key::Enter);
    frame(&mut v);
    assert_eq!(active(&v), Some("material-lookup"));
    assert!(v.gui.find_target("Switch to Search").is_none(), "the switcher closed");
}

#[test]
fn a_standalone_workbench_window_has_no_strip() {
    let mut v = member_viewer();
    assert!(v.nav.is_none() && v.nav_strip().is_none());
    assert!(v.gui.find_target("Toolbench home").is_none());
    v.open_workbench("members", None);
    assert!(v.editor.is_some(), "nothing happens without a shell");
}

#[test]
fn a_workbench_gets_its_tab_back_after_another_one_was_shown() {
    let dir = tempfile::tempdir().unwrap();
    let mut v = shell();
    v.open_workbench("fea", Some(case(&dir, "a.fea.json")));
    frame(&mut v);
    let mesh = target(&v, "Mesh tab");
    click(&mut v, mesh);
    let chosen = v.gui.tab_index();
    assert_ne!(chosen, 0, "the Mesh tab is not the first");
    v.dispatch(UiAction::Nav(NavAct::Open("material-lookup")));
    frame(&mut v);
    assert_eq!(v.gui.tab_index(), 0, "another workbench starts on its own first tab");
    v.dispatch(UiAction::Nav(NavAct::Open("fea")));
    frame(&mut v);
    assert_eq!(v.gui.tab_index(), chosen, "the tab came back with the workbench");
    v.dispatch(UiAction::Nav(NavAct::Home));
    frame(&mut v);
    v.dispatch(UiAction::Nav(NavAct::Open("fea")));
    frame(&mut v);
    assert_eq!(v.gui.tab_index(), chosen, "and after a visit to the dashboard");
}

#[test]
fn a_handoff_launch_opens_the_workbench_in_this_window_not_in_a_process() {
    use crate::gpu_viewer::{editor::handoff, EditorKind};
    let dir = tempfile::tempdir().unwrap();
    let mut v = shell();
    let file = case(&dir, "b.fea.json");
    handoff::LAUNCHED.with(|l| l.borrow_mut().clear());
    handoff::launch(EditorKind::Solid, &file).unwrap();
    assert!(handoff::LAUNCHED.with(|l| l.borrow().is_empty()), "no process was asked for");
    assert!(v.serve_asked(), "the window took the request");
    frame(&mut v);
    assert_eq!((tabs(&v), active(&v)), (vec!["fea"], Some("fea")));
    assert_eq!(v.editor.as_ref().unwrap().path(), file);
    assert!(!v.serve_asked(), "a request is served once");
}

#[test]
fn a_standalone_window_still_starts_a_process_for_a_handoff() {
    use crate::gpu_viewer::{editor::handoff, EditorKind};
    // Not `shell()`: this thread does not serve a dashboard window.
    std::thread::spawn(|| {
        handoff::LAUNCHED.with(|l| l.borrow_mut().clear());
        handoff::launch(EditorKind::Bushing, std::path::Path::new("c.json")).unwrap();
        assert_eq!(handoff::LAUNCHED.with(|l| l.borrow().len()), 1);
        assert!(handoff::take_asked().is_empty());
    })
    .join()
    .unwrap();
}

#[test]
fn new_window_moves_a_clean_workbench_to_a_process_and_refuses_one_with_edits() {
    use crate::gpu_viewer::dashboard::SPAWNED;
    let dir = tempfile::tempdir().unwrap();
    let mut v = shell();
    let file = case(&dir, "a.json");
    SPAWNED.with(|s| s.borrow_mut().clear());
    dirty_members(&mut v, file.clone());
    v.dispatch(UiAction::Nav(NavAct::PopOut("members")));
    assert!(SPAWNED.with(|s| s.borrow().is_empty()), "unsaved edits stay in this window");
    assert_eq!(tabs(&v), vec!["members"]);
    v.editor.as_mut().unwrap().act(session::EditorAction::Run(crate::gpu_viewer::editor::caps::Operation::Save));
    v.dispatch(UiAction::Nav(NavAct::PopOut("members")));
    assert_eq!(SPAWNED.with(|s| s.borrow().clone()), vec![("members", Some(file))], "the process is asked for the same file");
    assert!(tabs(&v).is_empty() && v.dashboard.is_some(), "the tab is gone and the dashboard is shown");
}

#[test]
fn the_window_grows_to_hold_a_workbench_and_never_shrinks() {
    use super::super::nav::{room_for_workbench, WORKBENCH_SIZE};
    assert_eq!(room_for_workbench((800.0, 600.0)), Some(WORKBENCH_SIZE));
    assert_eq!(room_for_workbench((1500.0, 700.0)), Some((1500.0, WORKBENCH_SIZE.1)));
    assert_eq!(room_for_workbench((1600.0, 900.0)), None);
}
