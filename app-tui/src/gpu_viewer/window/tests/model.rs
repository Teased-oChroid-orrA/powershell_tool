//! Solid models, bench switching, closing with unsaved changes and box selection.
use super::*;

#[test]
fn a_solid_model_is_meshed_solved_and_viewed_through_the_chrome() {
    let mut problem = fea_problem::templates::templates().remove(0).1;
    problem.mesh.size = 1.0;
    let ed = crate::gpu_viewer::editor::solid::SolidSession::new(
        problem,
        std::env::temp_dir().join("window-solid-test.json"),
    );
    let mut v = viewer(Some(Box::new(ed)));
    let wait = |v: &mut Viewer, done: &dyn Fn(&Viewer) -> bool| {
        let t = Instant::now();
        while !done(v) {
            v.editor.as_mut().unwrap().poll();
            frame(v);
            assert!(t.elapsed().as_secs() < 120, "timed out");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    };
    // Nothing meshes on its own: the viewport says so until the Mesh step's Generate mesh is clicked.
    frame(&mut v);
    std::thread::sleep(std::time::Duration::from_millis(400));
    v.editor.as_mut().unwrap().poll();
    frame(&mut v);
    assert!(
        v.scene().label.contains("mesh not generated"),
        "{}",
        v.scene().label
    );
    let tab = target(&v, "Mesh tab");
    click(&mut v, tab);
    let generate = target(&v, "Generate mesh");
    click(&mut v, generate);
    // The mesh arrives on a worker and replaces the placeholder scene.
    wait(&mut v, &|v| v.scene().label.contains("geometry"));
    assert!(tree_labels(&v)
        .iter()
        .any(|l| l.starts_with("Mesh: ") && l.contains("nodes")));
    // Simulation step, Solve button.
    let tab = target(&v, "Simulation tab");
    click(&mut v, tab);
    let solve = target(&v, "Solve");
    click(&mut v, solve);
    wait(&mut v, &|v| !v.scene().fields.is_empty());
    assert_eq!(
        v.scene().fields.len(),
        13,
        "14 fields: the primary plus thirteen selectable ones (stresses, strains, displacements)"
    );
    frame(&mut v);
    assert!(
        v.gui.target_labels().iter().any(|l| l == "Result card"),
        "the Simulation step shows the result card: {:?}",
        v.gui.target_labels()
    );
    // V returns to the editable model.
    v.input(Input::Key(Key::Character("v".into())));
    frame(&mut v);
    assert!(v.scene().fields.is_empty() && v.scene().label.contains("geometry"));
}

fn sim_labels(v: &Viewer) -> Vec<String> {
    let ed = v.editor.as_deref().unwrap();
    let m = ed
        .menus()
        .into_iter()
        .find(|m| m.title == "Simulation")
        .unwrap();
    m.items
        .into_iter()
        .filter_map(|i| {
            if let crate::gpu_viewer::menu::Item::Entry(e) = i {
                Some(e.label)
            } else {
                None
            }
        })
        .collect()
}

#[test]
fn switching_benches_keeps_the_solid_model_and_shows_only_that_benchs_studies() {
    let dir = std::env::temp_dir().join(format!("window-bench-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let ed = crate::gpu_viewer::editor::open(Bench::Static, &dir.join("solid-model.json")).unwrap();
    let mut v = viewer(Some(ed));
    assert!(
        sim_labels(&v).contains(&"Solve".to_string())
            && !sim_labels(&v).iter().any(|l| l.contains("mode"))
    );
    v.dispatch(UiAction::SwitchBench(Bench::Modal));
    assert_eq!(v.editor.as_ref().unwrap().bench(), Bench::Modal);
    let labels = sim_labels(&v);
    assert!(
        labels.contains(&"Run study".to_string()) && !labels.contains(&"Solve".to_string()),
        "{labels:?}"
    );
    assert!(v.editor.as_ref().unwrap().title().starts_with("Modal"));
    // The Window menu lists every bench with the current one checked.
    let bar = menus::bar(&v.menu_context());
    let window = bar.iter().find(|m| m.title == "Window").unwrap();
    let checked: Vec<_> = window
        .items
        .iter()
        .filter_map(|i| {
            if let crate::gpu_viewer::menu::Item::Entry(e) = i {
                Some((e.label.clone(), e.checked))
            } else {
                None
            }
        })
        .filter(|(l, _)| l.starts_with("Bench:"))
        .collect();
    assert_eq!(checked.len(), Bench::ALL.len());
    assert_eq!(
        checked
            .iter()
            .filter(|(_, c)| *c == Some(true))
            .map(|(l, _)| l.as_str())
            .collect::<Vec<_>>(),
        ["Bench: Modal / vibration"]
    );
    // Crossing to the member model loads its own file; unsaved solid edits block the switch.
    v.editor.as_mut().unwrap().act(session::EditorAction::Solid(
        crate::gpu_viewer::editor::solid::SolidAction::NextTemplate,
    ));
    assert!(v.editor.as_ref().unwrap().dirty());
    v.dispatch(UiAction::SwitchBench(Bench::Members));
    assert_eq!(
        v.editor.as_ref().unwrap().bench(),
        Bench::Modal,
        "refused while unsaved"
    );
    assert!(v.editor.as_ref().unwrap().status().contains("Save"));
    v.editor.as_mut().unwrap().act(session::EditorAction::Solid(
        crate::gpu_viewer::editor::solid::SolidAction::Save,
    ));
    v.dispatch(UiAction::SwitchBench(Bench::Members));
    assert_eq!(v.editor.as_ref().unwrap().bench(), Bench::Members);
    assert!(v
        .editor
        .as_ref()
        .unwrap()
        .path()
        .ends_with("member-model.json"));
    // And back: the saved solid model comes back with its template.
    v.dispatch(UiAction::SwitchBench(Bench::Buckling));
    assert_eq!(v.editor.as_ref().unwrap().bench(), Bench::Buckling);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn closing_with_unsaved_changes_requires_an_explicit_discard() {
    let mut v = member_viewer();
    v.request_quit();
    assert!(v.quit, "a clean model closes at once");
    let mut v = member_viewer();
    let button = target(&v, "Add node");
    click(&mut v, button);
    let at = viewport_centre(&v);
    click(&mut v, at);
    assert!(v.editor.as_ref().unwrap().dirty());
    v.request_quit();
    assert!(!v.quit && v.editor.as_ref().unwrap().status().contains("Unsaved"));
    v.request_quit();
    assert!(!v.quit, "repeated close never discards implicitly");
    v.dispatch(UiAction::DiscardAndClose);
    assert!(v.quit);
}

#[test]
fn shift_drag_in_the_viewport_box_selects_and_delete_removes_the_group() {
    let mut v = member_viewer();
    let button = target(&v, "Add node");
    click(&mut v, button);
    let (cx, cy) = viewport_centre(&v);
    for dx in [-120.0, 0.0, 120.0] {
        click(&mut v, (cx + dx, cy));
    }
    assert!(
        tree_labels(&v).iter().any(|l| l == "Nodes (3)"),
        "{:?}",
        tree_labels(&v)
    );
    let button = target(&v, "Select");
    click(&mut v, button);
    // Shift-drag a box over the left and middle nodes.
    v.input(Input::Modifiers(ModifiersState::SHIFT));
    move_to(&mut v, (cx - 200.0, cy - 40.0));
    button_press(&mut v);
    move_to(&mut v, (cx + 60.0, cy + 40.0));
    assert!(
        v.pointer.box_drag().is_some(),
        "the drag is a box, not an orbit"
    );
    let yaw = v.view.yaw;
    frame(&mut v);
    button_release(&mut v);
    frame(&mut v);
    assert_eq!(v.view.yaw, yaw, "a box drag never orbits");
    assert!(
        property_text(&v).contains("+1 more selected"),
        "{}",
        property_text(&v)
    );
    v.input(Input::Modifiers(ModifiersState::default()));
    key(&mut v, NamedKey::Delete);
    assert!(
        tree_labels(&v).iter().any(|l| l == "Nodes (1)"),
        "{:?}",
        tree_labels(&v)
    );
}
