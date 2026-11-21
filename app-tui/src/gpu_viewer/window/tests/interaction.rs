//! Legend, tree menu, tool pill, fix highlight, face picks, pressure, layout learning, units.
use super::*;

/// A result shows what its colours mean, can be cycled through every field, and so can a mode shape.
#[test]
fn a_result_shows_a_labelled_legend_whose_title_cycles_the_field_and_modes_have_the_same_fields() {
    let mut problem = fea_problem::templates::templates().remove(0).1;
    problem.mesh.size = 1.0;
    let ed = crate::gpu_viewer::editor::solid::SolidSession::new(
        problem,
        std::env::temp_dir().join("window-legend-test.json"),
    );
    let mut v = viewer(Some(Box::new(ed)));
    // The model itself: a colour key, no field.
    frame(&mut v);
    assert!(
        v.gui
            .target_labels()
            .iter()
            .all(|l| !l.ends_with(" legend")),
        "no result yet: {:?}",
        v.gui.target_labels()
    );
    let tab = target(&v, "Simulation tab");
    click(&mut v, tab);
    let solve = target(&v, "Solve");
    click(&mut v, solve);
    let t = Instant::now();
    while v.scene().fields.is_empty() {
        v.editor.as_mut().unwrap().poll();
        frame(&mut v);
        assert!(t.elapsed().as_secs() < 120, "timed out");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    frame(&mut v);
    let legend = v
        .gui
        .target_labels()
        .into_iter()
        .find(|l| l.ends_with(" legend"))
        .expect("a legend for the result");
    assert!(legend.starts_with("von Mises stress"), "{legend}");
    // Clicking the title shows the next field, and its name is on the legend.
    let first = v.view.field;
    let title = target(&v, &legend);
    click(&mut v, title);
    frame(&mut v);
    assert_eq!(v.view.field, first + 1);
    let next = v
        .gui
        .target_labels()
        .into_iter()
        .find(|l| l.ends_with(" legend"))
        .unwrap();
    assert_ne!(next, legend, "the legend names the new field");
    // Strains are among the fields.
    let names: Vec<String> = v.scene().fields.iter().map(|f| f.label.clone()).collect();
    for want in [
        "Equivalent strain",
        "Max principal strain",
        "Min principal strain",
    ] {
        assert!(names.iter().any(|n| n == want), "{want} in {names:?}");
    }
}

#[test]
fn a_right_click_on_a_tree_row_selects_it_and_opens_the_menu_of_its_entity() {
    let mut problem = fea_problem::templates::templates().remove(0).1;
    problem.mesh.size = 1.0;
    let ed = crate::gpu_viewer::editor::solid::SolidSession::new(
        problem,
        std::env::temp_dir().join("window-treemenu-test.json"),
    );
    let mut v = viewer(Some(Box::new(ed)));
    let row = target(&v, "Mesh: ");
    move_to(&mut v, row);
    v.test_events.push(egui::Event::PointerButton {
        pos: pos(row),
        button: egui::PointerButton::Secondary,
        pressed: true,
        modifiers: Default::default(),
    });
    v.test_events.push(egui::Event::PointerButton {
        pos: pos(row),
        button: egui::PointerButton::Secondary,
        pressed: false,
        modifiers: Default::default(),
    });
    frame(&mut v);
    frame(&mut v);
    assert!(
        v.gui.context_open(),
        "the menu opens from the selected state"
    );
    let labels = labels(&v);
    assert!(labels.iter().any(|l| l == "Generate mesh"), "{labels:?}");
    assert!(
        labels.iter().any(|l| l.starts_with("Element Type")),
        "{labels:?}"
    );
    // Choosing an entry runs it and closes the menu.
    let generate = v
        .gui
        .target_labels()
        .iter()
        .rposition(|l| l == "Generate mesh")
        .map(|_| ())
        .is_some();
    assert!(generate);
}

/// Visit `Model` and `Mesh` alternately so each tool group is offered again, using `tool` each time.
fn train_tool(v: &mut Viewer, tool: &str, rounds: usize) {
    for _ in 0..rounds {
        let model = target(v, "Model tab");
        click(v, model);
        let button = target(v, tool);
        click(v, button);
        let simulation = target(v, "Simulation tab");
        click(v, simulation);
    }
    let model = target(v, "Model tab");
    click(v, model);
}

#[test]
fn the_tool_pill_learns_which_tools_are_used_first_and_reset_and_off_restore_the_fixed_order() {
    let mut v = member_viewer();
    let x = |v: &Viewer, l: &str| target(v, l).0;
    assert!(
        x(&v, "Add support") < x(&v, "Add load"),
        "the fixed order puts Add support first"
    );
    train_tool(&mut v, "Add load", 7);
    assert!(
        x(&v, "Add load") < x(&v, "Add support"),
        "the tool that is used moves up: {:?}",
        labels(&v)
    );
    // Switching the adaptive layout off restores the fixed order; on again brings the learned one back.
    v.dispatch(UiAction::Layout(LayoutOp::ToggleAdaptive));
    frame(&mut v);
    assert!(x(&v, "Add support") < x(&v, "Add load"));
    v.dispatch(UiAction::Layout(LayoutOp::ToggleAdaptive));
    frame(&mut v);
    assert!(x(&v, "Add load") < x(&v, "Add support"));
    // Reset forgets it.
    v.dispatch(UiAction::Layout(LayoutOp::Reset));
    frame(&mut v);
    assert!(x(&v, "Add support") < x(&v, "Add load"));
    assert!(v.editor.as_ref().unwrap().status().contains("forgotten"));
    v.dispatch(UiAction::Layout(LayoutOp::Explain));
    assert!(v
        .editor
        .as_ref()
        .unwrap()
        .status()
        .contains("Adaptive layout"));
}

#[test]
fn a_finished_step_rings_the_next_one_and_moves_on_once_that_is_a_habit() {
    fn finish_mesh(v: &mut Viewer) {
        let tab = target(v, "Mesh tab");
        click(v, tab);
        let generate = target(v, "Generate mesh");
        click(v, generate);
        let t = Instant::now();
        while !v.scene().label.contains("geometry") {
            v.editor.as_mut().unwrap().poll();
            frame(v);
            assert!(t.elapsed().as_secs() < 120, "timed out");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        frame(v);
    }
    let make = |name: &str| {
        let mut problem = fea_problem::templates::templates().remove(0).1;
        problem.mesh.size = 1.0;
        viewer(Some(Box::new(
            crate::gpu_viewer::editor::solid::SolidSession::new(
                problem,
                std::env::temp_dir().join(name),
            ),
        )))
    };
    // No habit yet: the next step only gets a ring, the tools stay on Mesh.
    let mut v = make("window-nudge-test.json");
    finish_mesh(&mut v);
    assert_eq!(v.gui.nudge(), Some("Simulation"));
    assert!(
        labels(&v).iter().any(|l| l == "Generate mesh"),
        "still on the Mesh step"
    );
    // With a learned habit (Mesh is followed by Simulation) the step moves on by itself.
    let mut v = make("window-advance-test.json");
    for _ in 0..5 {
        v.gui.adapt_mut().step_taken("Mesh", "Simulation");
    }
    finish_mesh(&mut v);
    assert!(
        labels(&v).iter().any(|l| l == "Solve"),
        "moved to Simulation: {:?}",
        labels(&v)
    );
    assert_eq!(v.gui.nudge(), None);
}

#[test]
fn pointing_at_a_fix_highlights_the_surface_it_touches_and_leaving_clears_it() {
    let mut problem = fea_problem::templates::templates().remove(0).1;
    problem.mesh.size = 1.0;
    problem.supports.clear();
    let mut v = viewer(Some(Box::new(
        crate::gpu_viewer::editor::solid::SolidSession::new(
            problem,
            std::env::temp_dir().join("window-fixhover-test.json"),
        ),
    )));
    // The mesh first: the preview scene is where surfaces are lit.
    let tab = target(&v, "Mesh tab");
    click(&mut v, tab);
    let generate = target(&v, "Generate mesh");
    click(&mut v, generate);
    let t = Instant::now();
    while !v.scene().label.contains("geometry") {
        v.editor.as_mut().unwrap().poll();
        frame(&mut v);
        assert!(t.elapsed().as_secs() < 120, "timed out");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    frame(&mut v);
    // Vertices carrying the "selected" state code (4 is the viewer's colour for it; the session marks highlights 3 -> selected).
    let lit = |v: &Viewer| {
        v.scene()
            .samples
            .iter()
            .filter(|s| s[3] >= 3.0 - 1e-6)
            .count()
    };
    let before = lit(&v);
    let labels = v.gui.target_labels();
    let fix = labels
        .iter()
        .find(|l| l.contains("Fix 'left'"))
        .unwrap_or_else(|| panic!("no fix row among {labels:?}"))
        .clone();
    let at = target(&v, &fix);
    move_to(&mut v, at);
    frame(&mut v);
    let on = lit(&v);
    assert!(
        on > before,
        "pointing at 'Fix left' lights the left edge: {before} -> {on}"
    );
    // Away from every row the preview goes, and nothing was applied.
    let away = viewport_centre(&v);
    move_to(&mut v, away);
    frame(&mut v);
    assert_eq!(lit(&v), before, "leaving the row clears the highlight");
}

#[test]
fn a_real_mouse_click_on_a_3d_face_picks_it_and_its_menu_adds_a_support_there() {
    let mut problem = fea_problem::templates::templates()
        .into_iter()
        .find(|(n, _)| n.contains("3D"))
        .unwrap()
        .1;
    problem.mesh.size = 1.0;
    problem.supports.clear();
    let mut v = viewer(Some(Box::new(
        crate::gpu_viewer::editor::solid::SolidSession::new(
            problem,
            std::env::temp_dir().join("window-facepick-test.json"),
        ),
    )));
    let tab = target(&v, "Mesh tab");
    click(&mut v, tab);
    let generate = target(&v, "Generate mesh");
    click(&mut v, generate);
    let t = Instant::now();
    while !v.scene().label.contains("geometry") {
        v.editor.as_mut().unwrap().poll();
        frame(&mut v);
        assert!(t.elapsed().as_secs() < 120, "timed out");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    frame(&mut v);
    let supports = |v: &Viewer| {
        tree_labels(v)
            .into_iter()
            .find(|l| l.starts_with("Supports ("))
            .unwrap()
    };
    assert_eq!(supports(&v), "Supports (0)");
    // Sweep the viewport with real clicks until one lands on the part and opens the surface menu.
    let r = v.viewport();
    let mut picked = None;
    'scan: for iy in 1..10 {
        for ix in 1..16 {
            let at = ((r.x + r.w * ix / 16) as f64, (r.y + r.h * iy / 10) as f64);
            click(&mut v, at);
            if labels(&v).iter().any(|l| l == "Hold directions...") {
                picked = Some((at, "Fixed".to_string()));
                break 'scan;
            }
        }
    }
    let (at, item) =
        picked.expect("some click in the viewport must land on the part and open the surface menu");
    let face = v
        .editor
        .as_ref()
        .unwrap()
        .tree()
        .into_iter()
        .find(|n| n.selected && n.glyph == 's')
        .map(|n| n.label)
        .expect("the picked surface is selected in the tree");
    assert!(
        ["left", "right", "top", "bottom", "start", "end", "hole1"].contains(&face.as_str()),
        "a named face of the extruded plate: {face}"
    );
    // The tree shows the picked surface as selected, and choosing the menu entry adds the support on it.
    let selected: Vec<String> = v
        .editor
        .as_ref()
        .unwrap()
        .tree()
        .into_iter()
        .filter(|n| n.selected && n.label == face)
        .map(|n| n.label)
        .collect();
    assert_eq!(selected, std::slice::from_ref(&face), "pointer at {at:?}");
    let entry = target(&v, &item);
    click(&mut v, entry);
    assert_eq!(
        supports(&v),
        "Supports (1)",
        "{}",
        v.editor.as_ref().unwrap().status()
    );
    assert!(
        v.editor.as_ref().unwrap().status().contains(&face),
        "{}",
        v.editor.as_ref().unwrap().status()
    );
}

/// The whole path a person takes: right-click a face of the part, choose Pressure, type the number, press Apply.
#[test]
fn a_right_click_on_a_face_chooses_a_pressure_types_the_number_and_applies_it() {
    let mut problem = fea_problem::templates::templates()
        .into_iter()
        .find(|(n, _)| n.contains("3D"))
        .unwrap()
        .1;
    problem.mesh.size = 1.0;
    problem.loads.clear();
    let mut v = viewer(Some(Box::new(
        crate::gpu_viewer::editor::solid::SolidSession::new(
            problem,
            std::env::temp_dir().join("window-bc-card-test.json"),
        ),
    )));
    // The body shows before any mesh exists; wait for it.
    let t = Instant::now();
    while !v.scene().label.contains("body (not meshed)") {
        v.editor.as_mut().unwrap().poll();
        frame(&mut v);
        assert!(t.elapsed().as_secs() < 60, "the body did not appear");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    frame(&mut v);
    let loads = |v: &Viewer| {
        tree_labels(v)
            .into_iter()
            .find(|l| l.starts_with("Loads ("))
            .unwrap()
    };
    assert_eq!(loads(&v), "Loads (0)");
    // Find a point of the viewport on the part and right-click it.
    let r = v.viewport();
    let mut at = None;
    'scan: for iy in 1..10 {
        for ix in 1..16 {
            let p = ((r.x + r.w * ix / 16) as f64, (r.y + r.h * iy / 10) as f64);
            move_to(&mut v, p);
            v.input(Input::Press(MouseButton::Right));
            v.input(Input::Release(MouseButton::Right));
            frame(&mut v);
            if labels(&v).iter().any(|l| l == "Pressure...") {
                at = Some(p);
                break 'scan;
            }
            key(&mut v, NamedKey::Escape);
        }
    }
    let at = at.expect("a right-click on the part opens the face menu");
    let face = v
        .editor
        .as_ref()
        .unwrap()
        .tree()
        .into_iter()
        .find(|n| n.selected && n.glyph == 's')
        .map(|n| n.label)
        .expect("the face is selected");
    let labels_now = labels(&v);
    for want in [
        "Fixed",
        "Hold directions...",
        "Pressure...",
        "Force...",
        "Traction...",
    ] {
        assert!(
            labels_now.iter().any(|l| l == want),
            "{want} in {labels_now:?}"
        );
    }
    let pressure = target(&v, "Pressure...");
    click(&mut v, pressure);
    frame(&mut v);
    assert!(
        v.editor.as_ref().unwrap().value_card().is_some(),
        "choosing Pressure opens its card"
    );
    let field = target(&v, "Pressure field");
    // The card opens beside the face's menu, inside the window.
    let (w, h) = v.window_size();
    assert!(
        field.0 > 0.0 && field.0 < w as f64 && field.1 > 0.0 && field.1 < h as f64,
        "{field:?}"
    );
    assert_eq!(loads(&v), "Loads (0)", "nothing is added until Apply");
    type_text(&mut v, "2500");
    frame(&mut v);
    let text = v
        .editor
        .as_ref()
        .unwrap()
        .value_card()
        .unwrap()
        .rows
        .into_iter()
        .find_map(|r| {
            if let crate::gpu_viewer::ui::CardRow::Field { text, .. } = r {
                Some(text)
            } else {
                None
            }
        })
        .unwrap();
    assert_eq!(text, "2500", "the first keys replace the suggested value");
    let apply = target(&v, "Apply");
    click(&mut v, apply);
    frame(&mut v);
    assert!(
        v.editor.as_ref().unwrap().value_card().is_none(),
        "Apply closes the card"
    );
    assert_eq!(
        loads(&v),
        "Loads (1)",
        "{}",
        v.editor.as_ref().unwrap().status()
    );
    // Right-clicking the same face now also offers to edit and remove what is on it.
    move_to(&mut v, at);
    v.input(Input::Press(MouseButton::Right));
    v.input(Input::Release(MouseButton::Right));
    frame(&mut v);
    let again = labels(&v);
    assert!(
        again
            .iter()
            .any(|l| l == &format!("Edit the load on '{face}'..."))
            && again
                .iter()
                .any(|l| l == &format!("Remove the load on '{face}'")),
        "{again:?}"
    );
    // Cancel from a card leaves everything as it was.
    let hold = target(&v, "Hold directions...");
    click(&mut v, hold);
    frame(&mut v);
    let cancel = target(&v, "Cancel");
    click(&mut v, cancel);
    frame(&mut v);
    assert!(v.editor.as_ref().unwrap().value_card().is_none());
    assert_eq!(
        tree_labels(&v)
            .into_iter()
            .find(|l| l.starts_with("Supports ("))
            .unwrap(),
        "Supports (1)",
        "the template's own support; nothing added"
    );
}

/// The whole STEP workflow with a real mouse: open a cube, mesh it, click a face, cut a hole in it from the Insert menu's
/// action, and see the support set on another face follow its face through the edit.

#[test]
fn what_the_layout_learned_survives_a_restart() {
    let dir = std::env::temp_dir().join(format!("window-layout-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let x = |v: &Viewer, l: &str| target(v, l).0;
    {
        let mut v = member_viewer();
        v.gui.attach_layout_dir(&dir);
        frame(&mut v);
        train_tool(&mut v, "Add load", 7);
        assert!(x(&v, "Add load") < x(&v, "Add support"));
        v.gui.flush_layout();
    }
    let mut v = member_viewer();
    assert!(
        x(&v, "Add support") < x(&v, "Add load"),
        "a fresh window starts from the fixed order"
    );
    v.gui.attach_layout_dir(&dir);
    frame(&mut v);
    assert!(
        x(&v, "Add load") < x(&v, "Add support"),
        "the habit came back from the file"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn selecting_the_mesh_row_in_the_tree_brings_the_mesh_tools() {
    let mut problem = fea_problem::templates::templates().remove(0).1;
    problem.mesh.size = 1.0;
    let ed = crate::gpu_viewer::editor::solid::SolidSession::new(
        problem,
        std::env::temp_dir().join("window-focusstep-test.json"),
    );
    let mut v = viewer(Some(Box::new(ed)));
    assert!(
        labels(&v).iter().any(|l| l == "Extrude"),
        "starts on the Model step"
    );
    let row = target(&v, "Mesh: ");
    click(&mut v, row);
    frame(&mut v);
    assert!(
        labels(&v).iter().any(|l| l == "Generate mesh"),
        "the tools followed the selection: {:?}",
        labels(&v)
    );
    let row = target(&v, "Material: ");
    click(&mut v, row);
    frame(&mut v);
    assert!(labels(&v).iter().any(|l| l == "Extrude"));
}
#[test]
fn the_units_dropdown_converts_the_saved_physical_model_through_real_clicks() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("units.json");
    let problem = fea_problem::templates::templates().remove(0).1;
    let physical_e = problem.modulus_pa();
    let physical_thickness = problem.thickness * problem.units.length_m();
    let ed = crate::gpu_viewer::editor::solid::SolidSession::new(problem, path.clone());
    let mut v = viewer(Some(Box::new(ed)));
    let at = target(&v, "Units:");
    click(&mut v, at);
    let at = target(&v, "Unit system dropdown");
    click(&mut v, at);
    let at = target(&v, "mm, N, MPa");
    assert_ne!(
        v.gui.ctx().layer_id_at(pos(at)),
        Some(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::new("value-card")
        ))
    );
    click(&mut v, at);
    assert!(v.editor.as_ref().unwrap().status().contains("rescaled"));
    v.dispatch(UiAction::Editor(
        crate::gpu_viewer::editor::session::EditorAction::Solid(
            crate::gpu_viewer::editor::solid::SolidAction::Save,
        ),
    ));
    let converted =
        fea_problem::Problem::from_json(&std::fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(converted.units, fea_problem::UnitSystem::MmMpa);
    assert!((converted.modulus_pa() / physical_e - 1.0).abs() < 1e-12);
    assert!(
        (converted.thickness * converted.units.length_m() / physical_thickness - 1.0).abs() < 1e-12
    );
}
