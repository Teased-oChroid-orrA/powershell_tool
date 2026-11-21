//! Clicks, drags, wheel, palette and window menu against the viewer.
use super::*;

#[test]
fn a_tool_click_picks_a_tool_and_a_viewport_click_then_uses_it() {
    let mut v = member_viewer();
    assert!(tree_labels(&v).iter().any(|l| l == "Nodes (0)"));
    // Tool pill: Add node. The press is taken by the chrome and never reaches the model.
    let button = target(&v, "Add node");
    click(&mut v, button);
    assert!(
        property_text(&v).contains("tool Add node"),
        "{}",
        property_text(&v)
    );
    assert!(
        tree_labels(&v).iter().any(|l| l == "Nodes (0)"),
        "clicking the chrome placed nothing"
    );
    // Viewport: the node tool places a node.
    let at = viewport_centre(&v);
    click(&mut v, at);
    assert!(
        tree_labels(&v).iter().any(|l| l == "Nodes (1)"),
        "{:?}",
        tree_labels(&v)
    );
    // Clicking a dock's empty space does nothing to the model either.
    let vp = v.viewport();
    click(&mut v, (vp.x as f64 - 6.0, (vp.y + vp.h) as f64 - 4.0));
    assert!(tree_labels(&v).iter().any(|l| l == "Nodes (1)"));
}

#[test]
fn right_click_selects_under_the_cursor_and_opens_a_menu_that_closes_on_escape() {
    let mut v = member_viewer();
    let button = target(&v, "Add node");
    click(&mut v, button);
    let at = viewport_centre(&v);
    click(&mut v, at);
    let button = target(&v, "Select");
    click(&mut v, button);
    move_to(&mut v, at);
    v.input(Input::Press(MouseButton::Right));
    v.input(Input::Release(MouseButton::Right));
    frame(&mut v);
    assert!(v.gui.context_open(), "right-click opens the context menu");
    assert!(
        property_text(&v).contains("x "),
        "the node under the cursor was selected: {}",
        property_text(&v)
    );
    // Escape closes the menu and does not quit.
    key(&mut v, NamedKey::Escape);
    assert!(!v.gui.context_open() && !v.quit);
    // A right-click on a dock opens no context menu.
    let dock = target(&v, "Nodes");
    move_to(&mut v, dock);
    v.input(Input::Press(MouseButton::Right));
    v.input(Input::Release(MouseButton::Right));
    assert!(!v.gui.context_open());
}

/// The real window gives egui the same secondary press and release the viewer sees; a menu opened on the
/// press was closed by that press ("pressed outside") when its first frame was not under the cursor.
#[test]
fn a_right_click_the_egui_also_sees_leaves_the_menu_open_across_frames_and_positions() {
    for (fx, fy) in [
        (0.5, 0.5),
        (0.95, 0.5),
        (0.5, 0.95),
        (0.98, 0.98),
        (0.3, 0.2),
    ] {
        let mut v = member_viewer();
        let vp = v.viewport();
        move_to(
            &mut v,
            (
                vp.x as f64 + fx * vp.w as f64,
                vp.y as f64 + fy * vp.h as f64,
            ),
        );
        button(&mut v, MouseButton::Right, true);
        frame(&mut v);
        button(&mut v, MouseButton::Right, false);
        for _ in 0..3 {
            frame(&mut v);
        }
        assert!(
            v.gui.context_open(),
            "menu closed after a right click at {fx},{fy}"
        );
    }
}

#[test]
fn the_display_style_and_background_persist_between_sessions_and_the_picker_toggles() {
    let path = std::env::temp_dir()
        .join(format!("window-display-{}", std::process::id()))
        .join("display.json");
    let _ = std::fs::remove_file(&path);
    let mut v = member_viewer();
    v.attach_display_store(Some(path.clone()));
    assert_eq!(v.view.style, DisplayStyle::Wireframe);
    v.dispatch(UiAction::Command(Command::SetStyle(
        DisplayStyle::HiddenLinesRemoved,
    )));
    v.dispatch(UiAction::Command(Command::SetBackground(
        Background::BlackToGrey,
    )));
    let mut next = member_viewer();
    next.attach_display_store(Some(path.clone()));
    assert_eq!(
        (next.view.style, next.view.background),
        (DisplayStyle::HiddenLinesRemoved, Background::BlackToGrey)
    );
    // Another command leaves the stored choice alone (and a viewer without a store never writes).
    let mut free = member_viewer();
    free.dispatch(UiAction::Command(Command::SetStyle(DisplayStyle::Shaded)));
    assert_eq!(
        DisplaySettings::load_from(&path).style,
        DisplayStyle::HiddenLinesRemoved
    );
    assert!(!v.gui.background_picker_open());
    v.dispatch(UiAction::BackgroundPicker);
    frame(&mut v);
    assert!(v.gui.background_picker_open());
    v.dispatch(UiAction::BackgroundPicker);
    assert!(!v.gui.background_picker_open());
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

/// The right-click list is as large as its entries, not as large as the room the window leaves it.
#[test]
fn the_context_menu_is_sized_to_its_entries() {
    let mut v = member_viewer();
    let vp = v.viewport();
    move_to(
        &mut v,
        (
            vp.x as f64 + 0.3 * vp.w as f64,
            vp.y as f64 + 0.3 * vp.h as f64,
        ),
    );
    v.input(Input::Press(MouseButton::Right));
    v.input(Input::Release(MouseButton::Right));
    for _ in 0..3 {
        frame(&mut v);
    }
    let area = v.gui.floating_rect().expect("menu open");
    let (rows, widest) = v.gui.target_rects_inside(area);
    eprintln!("area {area:?} rows {rows} widest {widest}");
    assert!(rows > 0);
    assert!(
        area.width() < 380.0 && area.width() <= widest + 24.0,
        "menu {} wide for entries {} wide",
        area.width(),
        widest
    );
}

#[test]
fn a_middle_or_right_drag_pans_a_right_click_does_not_and_the_wheel_zooms_about_the_cursor() {
    let mut v = member_viewer();
    let vp = v.viewport();
    let start = (
        vp.x as f64 + 0.5 * vp.w as f64,
        vp.y as f64 + 0.5 * vp.h as f64,
    );
    // Middle drag: the view follows the cursor (200 px right = 400/width NDC), y is flipped.
    move_to(&mut v, start);
    button(&mut v, MouseButton::Middle, true);
    move_to(&mut v, (start.0 + 100.0, start.1 + 50.0));
    move_to(&mut v, (start.0 + 200.0, start.1 + 100.0));
    button(&mut v, MouseButton::Middle, false);
    let want = [(400.0 / vp.w as f64) as f32, (-200.0 / vp.h as f64) as f32];
    assert!(
        (v.view.pan[0] - want[0]).abs() < 1e-4 && (v.view.pan[1] - want[1]).abs() < 1e-4,
        "{:?} vs {want:?}",
        v.view.pan
    );
    assert!(!v.gui.context_open());
    // Right drag pans from the press point (not from where the 4 px threshold was crossed) and opens no menu.
    v.command(Command::ResetCamera);
    move_to(&mut v, start);
    button(&mut v, MouseButton::Right, true);
    move_to(&mut v, (start.0 + 40.0, start.1));
    button(&mut v, MouseButton::Right, false);
    assert!(
        (v.view.pan[0] - 80.0 / vp.w as f32).abs() < 1e-4,
        "{:?}",
        v.view.pan
    );
    assert!(!v.gui.context_open(), "a drag is not a click");
    // A jitter of 2 px is still a click.
    move_to(&mut v, start);
    button(&mut v, MouseButton::Right, true);
    move_to(&mut v, (start.0 + 2.0, start.1));
    button(&mut v, MouseButton::Right, false);
    frame(&mut v);
    assert!(v.gui.context_open());
    v.gui.close_floating();
    // Wheel zoom keeps the model point under the cursor fixed: ndc = s * zoom + pan.
    v.command(Command::ResetCamera);
    let at = (
        vp.x as f64 + 0.8 * vp.w as f64,
        vp.y as f64 + 0.3 * vp.h as f64,
    );
    move_to(&mut v, at);
    let c = [(2.0 * 0.8 - 1.0) as f32, (1.0 - 2.0 * 0.3) as f32];
    let before = [c[0] / v.view.zoom, c[1] / v.view.zoom];
    v.input(Input::Wheel(5.0));
    assert!(v.view.zoom > crate::gpu_viewer::controller::DEFAULT_ZOOM);
    let after = [
        (c[0] - v.view.pan[0]) / v.view.zoom,
        (c[1] - v.view.pan[1]) / v.view.zoom,
    ];
    assert!(
        (before[0] - after[0]).abs() < 1e-4 && (before[1] - after[1]).abs() < 1e-4,
        "{before:?} {after:?}"
    );
    // R resets zoom and pan.
    v.input(Input::Key(Key::Character("r".into())));
    assert_eq!(v.view.pan, [0.0, 0.0]);
}

#[test]
fn the_palette_finds_a_command_anywhere_runs_it_and_explains_a_disabled_one() {
    let mut v = viewer(None);
    assert!(!v.view.show_mesh);
    v.input(Input::Modifiers(ModifiersState::CONTROL));
    v.input(Input::Key(Key::Character("k".into())));
    v.input(Input::Modifiers(ModifiersState::default()));
    frame(&mut v);
    assert!(v.gui.palette_open());
    // Typed text reaches the query, never the viewer's own shortcuts (M would toggle the mesh).
    type_text(&mut v, "mesh edg");
    assert!(!v.view.show_mesh);
    egui_key(&mut v, egui::Key::Enter);
    assert!(v.view.show_mesh, "Enter ran View: Mesh edges");
    assert!(
        !v.gui.palette_open(),
        "choosing a command closes the palette"
    );
    // A disabled command stays findable and says why; the palette stays open.
    v.input(Input::Key(Key::Named(NamedKey::F10)));
    frame(&mut v);
    type_text(&mut v, "next field");
    egui_key(&mut v, egui::Key::Enter);
    assert!(v.gui.palette_open());
    assert_eq!(v.error.as_deref(), Some("this result has a single field"));
}

#[test]
fn escape_closes_the_palette_first_then_quits_a_plain_viewer() {
    let mut v = viewer(None);
    v.input(Input::Key(Key::Named(NamedKey::F10)));
    frame(&mut v);
    assert!(v.gui.palette_open());
    key(&mut v, NamedKey::Escape);
    assert!(!v.gui.palette_open() && !v.quit);
    key(&mut v, NamedKey::Escape);
    assert!(v.quit);
}

#[test]
fn typing_a_number_is_never_hijacked_by_the_palette_or_menu_keys() {
    let mut v = member_viewer();
    let button = target(&v, "Add node");
    click(&mut v, button);
    let at = viewport_centre(&v);
    click(&mut v, at);
    let select = target(&v, "Select");
    click(&mut v, select);
    // Right-click selects the node under the cursor; its first property is then edited with `e`.
    move_to(&mut v, at);
    v.input(Input::Press(MouseButton::Right));
    v.input(Input::Release(MouseButton::Right));
    key(&mut v, NamedKey::Escape);
    v.input(Input::Key(Key::Character("e".into())));
    assert!(v.editor.as_ref().unwrap().prompting());
    // While a number is being typed F10 / Ctrl+K / arrows / Enter belong to the prompt.
    key(&mut v, NamedKey::F10);
    assert!(!v.gui.palette_open());
    v.input(Input::Modifiers(ModifiersState::CONTROL));
    v.input(Input::Key(Key::Character("k".into())));
    v.input(Input::Modifiers(ModifiersState::default()));
    assert!(!v.gui.palette_open());
    v.input(Input::Key(Key::Character("7".into())));
    key(&mut v, NamedKey::Enter);
    assert!(!v.editor.as_ref().unwrap().prompting());
    assert!(
        property_text(&v).contains("7.000000e0"),
        "{}",
        property_text(&v)
    );
}

#[test]
fn the_wheel_zooms_over_the_viewport_and_not_over_a_dock() {
    let mut v = member_viewer();
    let zoom = v.view.zoom;
    let centre = viewport_centre(&v);
    move_to(&mut v, centre);
    v.input(Input::Wheel(2.0));
    assert!(v.view.zoom > zoom);
    let zoom = v.view.zoom;
    let vp = v.viewport();
    move_to(&mut v, (vp.x as f64 - 20.0, (vp.y + 40) as f64)); // the feature tree
    v.input(Input::Wheel(2.0));
    assert_eq!(v.view.zoom, zoom, "a dock consumes the wheel");
}

#[test]
fn dragging_orbits_only_when_the_press_started_in_the_viewport() {
    let mut v = member_viewer();
    let start = viewport_centre(&v);
    move_to(&mut v, start);
    button(&mut v, MouseButton::Left, true);
    move_to(&mut v, (start.0 + 60.0, start.1 + 20.0));
    button(&mut v, MouseButton::Left, false);
    assert!(v.view.yaw != 0.0, "a viewport drag orbits");
    let yaw = v.view.yaw;
    let vp = v.viewport();
    let on_dock = (vp.x as f64 - 20.0, (vp.y + 60) as f64); // the feature tree
    move_to(&mut v, on_dock);
    button(&mut v, MouseButton::Left, true);
    move_to(&mut v, (on_dock.0 - 100.0, on_dock.1 + 40.0));
    button(&mut v, MouseButton::Left, false);
    assert_eq!(
        v.view.yaw, yaw,
        "a drag that starts on a dock does not orbit the model"
    );
}

#[test]
fn window_menu_toggles_docks_and_the_viewport_follows() {
    let mut v = viewer(None);
    let before = v.viewport();
    v.dispatch(UiAction::TogglePanel(
        crate::gpu_viewer::controller::Panel::Tree,
    ));
    v.dispatch(UiAction::TogglePanel(
        crate::gpu_viewer::controller::Panel::Properties,
    ));
    frame(&mut v);
    let after = v.viewport();
    assert!(after.w > before.w);
    assert_eq!(
        v.camera().aspect,
        after.w as f32 / after.h as f32,
        "the camera uses the viewport aspect"
    );
}
