//! Step strip, hover, templates, inspector, cards, tree chip and the colour key.
use super::*;

#[test]
fn the_step_strip_follows_the_model_and_a_tab_click_swaps_the_tools() {
    let mut problem = fea_problem::templates::templates().remove(0).1;
    problem.mesh.size = 1.0;
    let ed = crate::gpu_viewer::editor::solid::SolidSession::new(
        problem,
        std::env::temp_dir().join("window-steps-test.json"),
    );
    let mut v = viewer(Some(Box::new(ed)));
    let labels = v.gui.target_labels();
    for step in ["Model tab", "Mesh tab", "Simulation tab", "View tab"] {
        assert!(labels.iter().any(|l| l == step), "{step} among {labels:?}");
    }
    assert!(
        labels.iter().any(|l| l == "Extrude"),
        "the Model step's tools are shown first: {labels:?}"
    );
    assert!(!labels.iter().any(|l| l == "Generate mesh"));
    let tab = target(&v, "Mesh tab");
    click(&mut v, tab);
    assert!(v.gui.target_labels().iter().any(|l| l == "Generate mesh"));
    assert!(
        !v.gui.target_labels().iter().any(|l| l == "Extrude"),
        "only the selected step's tools are shown"
    );
    // The Mesh step is still to do; the strip says so through the editor.
    use crate::gpu_viewer::editor::authoring::StepState;
    let ed = v.editor.as_ref().unwrap();
    assert_eq!(
        (
            ed.step_state("Model"),
            ed.step_state("Mesh"),
            ed.step_state("Simulation")
        ),
        (
            Some(StepState::Done),
            Some(StepState::Todo),
            Some(StepState::Todo)
        )
    );
    assert_eq!(ed.step_state("View"), None);
}

#[test]
fn hovering_a_tool_shows_what_it_does_in_the_status_bar() {
    let mut v = member_viewer();
    let button = target(&v, "Add node");
    move_to(&mut v, button);
    move_to(&mut v, (button.0 + 1.0, button.1));
    assert!(
        v.gui.hint().is_some_and(|h| h.starts_with("Add node")),
        "{:?}",
        v.gui.hint()
    );
    let centre = viewport_centre(&v);
    move_to(&mut v, centre);
    assert_eq!(v.gui.hint(), None);
}

#[test]
fn an_empty_model_offers_a_way_to_begin_and_a_template_click_starts_one() {
    use crate::gpu_viewer::editor::solid::{SolidAction, SolidSession};
    let (title, problem) = fea_problem::templates::templates().remove(0);
    let mut ed = SolidSession::new(problem, std::env::temp_dir().join("window-empty-test.json"));
    assert!(
        Authoring::start_options(&ed).is_empty(),
        "a model with geometry needs no starting card"
    );
    ed.act(session::EditorAction::Solid(SolidAction::ClearModel));
    assert_eq!(
        Authoring::start_options(&ed).len(),
        2 + fea_problem::templates::templates().len(),
        "new part, every template, open file"
    );
    let mut v = viewer(Some(Box::new(ed)));
    let labels = v.gui.target_labels();
    assert!(
        labels.iter().any(|l| l.starts_with("Open problem")) && labels.iter().any(|l| l == title),
        "{labels:?}"
    );
    let card = target(&v, title);
    click(&mut v, card);
    assert!(
        v.editor
            .as_ref()
            .unwrap()
            .status()
            .contains("Started from template"),
        "{}",
        v.editor.as_ref().unwrap().status()
    );
    assert!(
        !v.gui
            .target_labels()
            .iter()
            .any(|l| l.starts_with("Open problem")),
        "the card goes away once there is a model"
    );
}

#[test]
fn the_inspector_floats_with_what_matters_closes_for_a_selection_and_pins() {
    let mut v = member_viewer();
    // Nothing selected: no selection card, only the cards for the step.
    assert!(!labels(&v).iter().any(|l| l == "Node 1 card"));
    assert!(
        labels(&v).iter().any(|l| l == "Modelling card"),
        "{:?}",
        labels(&v)
    );
    let close = target(&v, "Close the inspector");
    click(&mut v, close);
    assert!(!labels(&v).iter().any(|l| l == "Modelling card"), "closed");
    // A new selection brings it back.
    let add = target(&v, "Add node");
    click(&mut v, add);
    let at = viewport_centre(&v);
    click(&mut v, at);
    let select = target(&v, "Select");
    click(&mut v, select);
    move_to(&mut v, at);
    v.input(Input::Press(MouseButton::Right));
    v.input(Input::Release(MouseButton::Right));
    frame(&mut v);
    key(&mut v, NamedKey::Escape);
    assert!(
        labels(&v)
            .iter()
            .any(|l| l.starts_with("Node ") && l.ends_with(" card")),
        "{:?}",
        labels(&v)
    );
    // Pinned: it stays when closed and the selection changes.
    let pin = target(&v, "Pin the inspector");
    click(&mut v, pin);
    let close = target(&v, "Close the inspector");
    click(&mut v, close);
    assert!(
        labels(&v).iter().any(|l| l.ends_with(" card")),
        "a pinned inspector ignores Close"
    );
}

#[test]
fn a_card_folds_and_unfolds_on_a_click() {
    let mut v = member_viewer();
    assert_eq!(
        v.gui.card_open("mode"),
        None,
        "no choice yet: the default (open) applies"
    );
    let card = target(&v, "Modelling card");
    click(&mut v, card);
    assert_eq!(v.gui.card_open("mode"), Some(false));
    let card = target(&v, "Modelling card");
    click(&mut v, card);
    assert_eq!(v.gui.card_open("mode"), Some(true));
}

#[test]
fn the_tree_folds_to_a_chip_and_opens_again_and_the_model_takes_the_space() {
    let mut v = member_viewer();
    let open = v.viewport();
    let fold = target(&v, "Fold the feature tree");
    click(&mut v, fold);
    assert!(!v.gui.tree_is_open());
    assert!(v.viewport().w > open.w, "the viewport widens");
    let chip = target(&v, "Feature tree");
    click(&mut v, chip);
    assert!(v.gui.tree_is_open() && v.viewport().w == open.w);
}

/// With the tree folded the chip that opens it must not sit under the tool pill (it used to overlap the analysis tools).
#[test]
fn the_folded_trees_chip_and_the_tool_pill_never_overlap_at_any_window_width() {
    for width in [1400u32, 1100, 900, 700] {
        let mut problem = fea_problem::templates::templates().remove(0).1;
        problem.mesh.size = 1.0;
        let ed = crate::gpu_viewer::editor::solid::SolidSession::new(
            problem,
            std::env::temp_dir().join("window-chip-test.json"),
        );
        let mut v = viewer(Some(Box::new(ed)));
        v.size_override = Some((width, 800));
        frame(&mut v);
        let fold = target(&v, "Fold the feature tree");
        click(&mut v, fold);
        frame(&mut v);
        let (chip, pill) = (
            v.gui.chip_rect().expect("the chip is shown"),
            v.gui.pill_rect().expect("the pill is shown"),
        );
        assert!(
            !chip.intersects(pill),
            "width {width}: chip {chip:?} overlaps the tool pill {pill:?}"
        );
        assert!(
            pill.left() >= chip.right(),
            "width {width}: the pill starts to the right of the chip"
        );
        let vp = v.gui.viewport_px((width, 800));
        assert!(
            pill.right() * v.gui.ctx().pixels_per_point() <= (vp.x + vp.w) as f32 + 1.0,
            "width {width}: the pill stays inside the viewport"
        );
    }
}

/// The colour key can be dragged anywhere the docks leave (over the model or beside the inspector's cards), each swatch
/// is a colour picker whose choice is kept, and the glyphs and the model use it.
#[test]
fn the_colour_key_moves_and_its_colours_change_and_persist() {
    let path = std::env::temp_dir()
        .join(format!("window-key-{}", std::process::id()))
        .join("display.json");
    let _ = std::fs::remove_file(&path);
    let mut v = member_viewer();
    v.attach_display_store(Some(path.clone()));
    frame(&mut v);
    let id = egui::Id::new("colour-key");
    let area = |v: &Viewer| {
        v.gui
            .ctx()
            .memory(|m| m.area_rect(id))
            .expect("the colour key is shown")
    };
    for name in [
        "Body colour",
        "Support colour",
        "Load colour",
        "Selected colour",
    ] {
        assert!(
            v.gui.target_labels().iter().any(|l| l == name),
            "{name} in {:?}",
            v.gui.target_labels()
        );
    }
    // Drag it by its heading: the key follows the pointer and stays inside the viewport.
    let before = area(&v);
    let grab = before.min + egui::vec2(24.0, 14.0);
    v.test_events.push(egui::Event::PointerMoved(grab));
    frame(&mut v);
    v.test_events.push(egui::Event::PointerButton {
        pos: grab,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: Default::default(),
    });
    frame(&mut v);
    for step in 1..=5 {
        v.test_events.push(egui::Event::PointerMoved(
            grab - egui::vec2(24.0 * step as f32, 0.0) + egui::vec2(0.0, 18.0 * step as f32),
        ));
        frame(&mut v);
    }
    v.test_events.push(egui::Event::PointerButton {
        pos: grab - egui::vec2(120.0, -90.0),
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: Default::default(),
    });
    frame(&mut v);
    frame(&mut v);
    let after = area(&v);
    assert!(
        (after.min.x - (before.min.x - 120.0)).abs() < 3.0
            && (after.min.y - (before.min.y + 90.0)).abs() < 3.0,
        "moved from {before:?} to {after:?}"
    );
    let vp = v.gui.viewport_px(v.window_size());
    let ppp = v.gui.ctx().pixels_per_point();
    assert!(
        after.min.x * ppp >= vp.x as f32 - 1.0 && after.max.x * ppp <= (vp.x + vp.w) as f32 + 1.0,
        "inside the viewport"
    );
    // The inspector's column does not fence it in: it can be dragged past the viewport's right edge, beside the cards.
    let room = v.gui.room().expect("the room is laid out");
    let vp_right = (vp.x + vp.w) as f32 / ppp;
    assert!(
        room.right() > vp_right + 200.0,
        "the inspector is shown: room {room:?}, viewport right {vp_right}"
    );
    let grab = after.min + egui::vec2(24.0, 14.0);
    let to = egui::pos2(vp_right + 150.0, grab.y);
    v.test_events.push(egui::Event::PointerMoved(grab));
    frame(&mut v);
    v.test_events.push(egui::Event::PointerButton {
        pos: grab,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: Default::default(),
    });
    frame(&mut v);
    for step in 1..=6 {
        v.test_events.push(egui::Event::PointerMoved(
            grab + (to - grab) * (step as f32 / 6.0),
        ));
        frame(&mut v);
    }
    v.test_events.push(egui::Event::PointerButton {
        pos: to,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: Default::default(),
    });
    frame(&mut v);
    frame(&mut v);
    let after = area(&v);
    assert!(
        (after.min.x - (to.x - 24.0)).abs() < 3.0 && after.max.x > vp_right + 100.0,
        "dragged past the viewport to {after:?}"
    );
    assert!(
        room.contains_rect(after),
        "still inside what the docks leave: {after:?} in {room:?}"
    );
    // Where it was left is kept as a fraction of that room, and a new session starts it there.
    let frac = v.view.panel_pos[0].expect("the drag's end is recorded");
    assert!(
        (room.left() + frac[0] * room.width() - after.min.x).abs() < 3.0
            && (room.top() + frac[1] * room.height() - after.min.y).abs() < 3.0,
        "fraction {frac:?} matches {after:?}"
    );
    assert_eq!(DisplaySettings::load_from(&path).panel_pos[0], Some(frac));
    let mut again = member_viewer();
    again.attach_display_store(Some(path.clone()));
    // The test viewer already drew a frame before the store was attached; the real window attaches it before the first.
    again.gui.ctx().memory_mut(|m| m.reset_areas());
    frame(&mut again);
    let restored = again
        .gui
        .ctx()
        .memory(|m| m.area_rect(id))
        .expect("the key is shown");
    assert!(
        (restored.min.x - after.min.x).abs() < 3.0 && (restored.min.y - after.min.y).abs() < 3.0,
        "restored {restored:?} vs {after:?}"
    );
    // A swatch is a colour: choosing one sets it, the view keeps it, and the file keeps it for the next session.
    v.dispatch(UiAction::Command(Command::SetKeyColour(1, [250, 10, 20])));
    assert_eq!(v.view.key_colours[1], [250, 10, 20]);
    let mut next = member_viewer();
    next.attach_display_store(Some(path.clone()));
    assert_eq!(next.view.key_colours[1], [250, 10, 20]);
    frame(&mut v);
    assert!(
        v.gui.target_labels().iter().any(|l| l == "Reset colours"),
        "a changed key offers its reset"
    );
    let reset = target(&v, "Reset colours");
    click(&mut v, reset);
    frame(&mut v);
    assert_eq!(
        v.view.key_colours,
        crate::gpu_viewer::display::default_key_colours()
    );
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}
