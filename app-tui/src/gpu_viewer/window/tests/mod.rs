use super::*;
use crate::gpu_viewer::display::{Background, DisplayStyle};
use crate::gpu_viewer::editor::session::Session;

#[test]
fn winit_keys_map_to_editor_keys() {
    assert_eq!(
        editor_key(&Key::Named(NamedKey::Enter)),
        Some(session::Key::Enter)
    );
    assert_eq!(
        editor_key(&Key::Named(NamedKey::Backspace)),
        Some(session::Key::Backspace)
    );
    assert_eq!(
        editor_key(&Key::Named(NamedKey::ArrowUp)),
        Some(session::Key::Up)
    );
    assert_eq!(
        editor_key(&Key::Named(NamedKey::ArrowLeft)),
        Some(session::Key::Left)
    );
    assert_eq!(
        editor_key(&Key::Character("Z".into())),
        Some(session::Key::Char('Z'))
    );
    assert_eq!(
        editor_key(&Key::Character("-".into())),
        Some(session::Key::Char('-'))
    );
    assert_eq!(
        editor_key(&Key::Character("ab".into())),
        None,
        "multi-character input (IME) is not a shortcut"
    );
    assert_eq!(editor_key(&Key::Named(NamedKey::F5)), None);
}
#[test]
fn the_palette_opens_with_ctrl_or_cmd_k_and_f10_only() {
    let none = ModifiersState::default();
    assert!(opens_palette(&Key::Named(NamedKey::F10), none));
    assert!(opens_palette(
        &Key::Character("k".into()),
        ModifiersState::CONTROL
    ));
    assert!(opens_palette(
        &Key::Character("K".into()),
        ModifiersState::SUPER
    ));
    assert!(
        !opens_palette(&Key::Character("k".into()), none),
        "a plain letter belongs to the editor"
    );
    assert!(!opens_palette(
        &Key::Character("m".into()),
        ModifiersState::CONTROL
    ));
}

// ---- the whole input path without a window ------------------------------------------------

fn plain_scene() -> Scene {
    Scene {
        label: "t".into(),
        positions: vec![[-0.3, -0.3, 0.0], [0.3, -0.3, 0.0], [0.0, 0.3, 0.0]],
        triangles: vec![0, 1, 2],
        samples: vec![[0.0; 4]; 3],
        times: vec![0.0],
        range: [0.0, 1.0],
        gain: 1.0,
        harmonic_period: None,
        field_label: String::new(),
        fields: Vec::new(),
    }
}
fn viewer(editor: Option<Box<dyn Authoring>>) -> Viewer {
    let mut v = new_viewer(ViewerProject::single(plain_scene()), editor);
    v.size_override = Some((1600, 900));
    frame(&mut v);
    v
}
fn member_viewer() -> Viewer {
    let ed = Session::new(
        fea_problem::members::blank_planar(),
        std::env::temp_dir().join("window-input-test.json"),
    );
    viewer(Some(Box::new(ed)))
}
/// One displayed frame: sync the scene and run the chrome twice (the second pass has the measured sizes of the
/// first, as it does a moment later in the real window), with the queued egui events.
fn frame(v: &mut Viewer) {
    v.sync_editor_scene();
    for _ in 0..2 {
        let (w, h) = v.window_size();
        v.test_time += 0.016;
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(w as f32, h as f32),
            )),
            events: std::mem::take(&mut v.test_events),
            time: Some(v.test_time),
            ..Default::default()
        };
        v.run_gui(raw).drop_without_applying_deltas();
    }
}
fn pos(at: (f64, f64)) -> egui::Pos2 {
    egui::pos2(at.0 as f32, at.1 as f32)
}
fn move_to(v: &mut Viewer, at: (f64, f64)) {
    v.test_events.push(egui::Event::PointerMoved(pos(at)));
    v.input(Input::Cursor(at));
    frame(v);
}
fn button(v: &mut Viewer, b: MouseButton, down: bool) {
    let at = v.pointer.cursor.expect("cursor placed");
    let pb = if b == MouseButton::Right {
        egui::PointerButton::Secondary
    } else {
        egui::PointerButton::Primary
    };
    v.test_events.push(egui::Event::PointerButton {
        pos: pos(at),
        button: pb,
        pressed: down,
        modifiers: Default::default(),
    });
    v.input(if down {
        Input::Press(b)
    } else {
        Input::Release(b)
    });
}
fn click(v: &mut Viewer, at: (f64, f64)) {
    move_to(v, at);
    button(v, MouseButton::Left, true);
    button(v, MouseButton::Left, false);
    frame(v);
}
/// Two clicks after a pause: egui counts a click within 0.6 s of the click before last as a triple click, and
/// `double_clicked()` only answers a count of exactly two.
fn double_click(v: &mut Viewer, at: (f64, f64)) {
    v.test_time += 1.0;
    click(v, at);
    click(v, at);
}
fn key(v: &mut Viewer, k: NamedKey) {
    v.input(Input::Key(Key::Named(k)));
    frame(v);
}
/// A key the palette's text field also sees.
fn egui_key(v: &mut Viewer, k: egui::Key) {
    v.test_events.push(egui::Event::Key {
        key: k,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Default::default(),
    });
    v.test_events.push(egui::Event::Key {
        key: k,
        physical_key: None,
        pressed: false,
        repeat: false,
        modifiers: Default::default(),
    });
    frame(v);
}
fn type_text(v: &mut Viewer, text: &str) {
    v.test_events.push(egui::Event::Text(text.into()));
    frame(v);
}
fn target(v: &Viewer, label: &str) -> (f64, f64) {
    v.gui
        .find_target(label)
        .unwrap_or_else(|| panic!("no control {label:?} among {:?}", v.gui.target_labels()))
}
fn viewport_centre(v: &Viewer) -> (f64, f64) {
    let r = v.viewport();
    ((r.x + r.w / 2) as f64, (r.y + r.h / 2) as f64)
}
fn tree_labels(v: &Viewer) -> Vec<String> {
    v.editor
        .as_ref()
        .unwrap()
        .tree()
        .into_iter()
        .map(|n| n.label)
        .collect()
}
fn property_text(v: &Viewer) -> String {
    v.editor
        .as_ref()
        .unwrap()
        .properties()
        .into_iter()
        .filter_map(|i| i.plain_text())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Press at `from`, move to `to` in steps, release there.
fn drag(v: &mut Viewer, from: (f64, f64), to: (f64, f64)) {
    move_to(v, from);
    v.test_events.push(egui::Event::PointerButton {
        pos: pos(from),
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: Default::default(),
    });
    frame(v);
    for k in 1..=6 {
        let f = k as f64 / 6.0;
        move_to(
            v,
            (from.0 + (to.0 - from.0) * f, from.1 + (to.1 - from.1) * f),
        );
    }
    v.test_events.push(egui::Event::PointerButton {
        pos: pos(to),
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: Default::default(),
    });
    frame(v);
    frame(v);
}

fn labels(v: &Viewer) -> Vec<String> {
    v.gui.target_labels()
}

fn button_press(v: &mut Viewer) {
    button(v, MouseButton::Left, true);
}

fn button_release(v: &mut Viewer) {
    button(v, MouseButton::Left, false);
}

fn replace_page_text(v: &mut Viewer, label: &str, text: &str) {
    let at = target(v, label);
    click(v, at);
    v.test_events.push(egui::Event::Key {
        key: egui::Key::A,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers {
            ctrl: true,
            command: true,
            ..Default::default()
        },
    });
    frame(v);
    type_text(v, text);
}

mod chrome;
mod input;
mod interaction;
mod model;
mod search_materials;
mod stackup;
mod workbenches;
