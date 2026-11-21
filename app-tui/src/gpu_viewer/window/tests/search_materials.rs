//! The Search and Materials pages.
use super::*;

#[test]
fn the_search_page_does_not_run_when_an_input_loses_focus() {
    use crate::gpu_viewer::editor::search::SearchSession;
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("example.txt"), "do not run on blur").unwrap();
    let mut search = SearchSession::in_memory();
    search.document_field(100, dir.path().to_string_lossy().into_owned());
    let mut v = viewer(Some(Box::new(search)));
    replace_page_text(&mut v, "Find input", "do not run on blur");
    let folder = target(&v, "Folder input");
    click(&mut v, folder);
    assert!(!v.editor.as_ref().unwrap().solving());
    assert!(!labels(&v).contains(&"Stop".into()));
}

#[test]
fn a_question_typed_in_find_is_an_ordinary_find_that_needs_a_folder() {
    use crate::gpu_viewer::editor::search::SearchSession;

    let search = SearchSession::in_memory();
    let mut v = viewer(Some(Box::new(search)));
    assert!(
        v.gui.find_target("Ask").is_none(),
        "there is no separate Ask panel"
    );
    let find = target(&v, "Find input");
    click(&mut v, find);
    type_text(&mut v, "Where is the design load?");
    egui_key(&mut v, egui::Key::Enter);
    frame(&mut v);

    let editor = v.editor.as_ref().unwrap();
    assert!(!editor.solving(), "no search starts without a folder");
    assert!(
        editor.status().contains("search path"),
        "{}",
        editor.status()
    );
}

#[test]
fn the_search_page_takes_typed_input_runs_on_enter_and_a_row_click_shows_its_matches() {
    use crate::gpu_viewer::editor::search::SearchSession;
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("fitting.txt"),
        "intro\nverify bolt torque\n",
    )
    .unwrap();
    std::fs::write(dir.path().join("other.txt"), "bolt only\n").unwrap();
    let mut v = viewer(Some(Box::new(SearchSession::in_memory())));
    assert!(v.page_mode());
    let labels = v.gui.target_labels();
    assert!(
        !labels.iter().any(|l| l == "View tab"),
        "no 3D View tab on a page bench: {labels:?}"
    );
    let folder = target(&v, "Folder input");
    click(&mut v, folder);
    type_text(&mut v, &dir.path().to_string_lossy());
    let find = target(&v, "Find input");
    click(&mut v, find);
    type_text(&mut v, "torque");
    egui_key(&mut v, egui::Key::Enter);
    let t = Instant::now();
    while v.editor.as_ref().unwrap().solving() || v.gui.find_target("row fitting.txt").is_none() {
        assert!(
            t.elapsed().as_secs() < 30,
            "no result row; status {:?}",
            v.editor.as_ref().unwrap().status()
        );
        v.editor.as_mut().unwrap().poll();
        frame(&mut v);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(
        v.gui.find_target("row other.txt").is_none(),
        "only the file with the word is listed"
    );
    // Keys typed into the page never reach the viewer (no orbit, no mesh toggle), and a click on the row selects it.
    let row = target(&v, "row fitting.txt");
    click(&mut v, row);
    let doc = v.editor.as_ref().unwrap().document().unwrap();
    let detail = doc.detail.unwrap();
    assert!(detail.text.is_some() || detail.empty.contains("Loading"));
    let before = v.view.show_mesh;
    v.input(Input::Key(Key::Character("m".into())));
    assert_eq!(v.view.show_mesh, before, "viewer keys do nothing on a page");
}

#[test]
fn search_sidebar_resize_stays_after_pointer_moves() {
    use crate::gpu_viewer::editor::search::SearchSession;
    let mut v = viewer(Some(Box::new(SearchSession::in_memory())));
    v.size_override = Some((800, 900));
    frame(&mut v);
    let before = v.viewport().x;
    let from = (before as f64 - 8.0, 450.0);
    drag(&mut v, from, (from.0 + 120.0, from.1));
    move_to(&mut v, (from.0 + 220.0, from.1));
    let after = v.viewport().x;
    assert!(
        after > before + 80,
        "sidebar width reset: before={before}, after={after}"
    );
    move_to(&mut v, (from.0 + 300.0, from.1 + 120.0));
    let after_pointer_move = v.viewport().x;
    assert!(
        after_pointer_move > before + 80,
        "sidebar width reset after pointer moved: before={before}, after={after_pointer_move}"
    );
}

#[test]
fn search_sidebar_and_controls_reflow_when_window_shrinks() {
    use crate::gpu_viewer::editor::search::SearchSession;
    let mut v = viewer(Some(Box::new(SearchSession::in_memory())));
    v.size_override = Some((725, 740));
    frame(&mut v);
    let before = v.viewport().x;
    drag(
        &mut v,
        (before as f64 - 8.0, 400.0),
        (before as f64 + 72.0, 400.0),
    );
    let widened = v.viewport().x;
    assert!(
        widened > before,
        "sidebar did not resize: before={before}, widened={widened}"
    );
    v.size_override = Some((600, 700));
    frame(&mut v);
    let after = v.viewport().x;
    assert!(
        after <= 240,
        "sidebar did not adapt to narrower window: {after}px"
    );
    for control in [
        "Folder input",
        "Find input",
        "Options panel",
        "Report panel",
        "Help and shortcuts",
    ] {
        let (x, y) = target(&v, control);
        assert!(
            x < 600.0 && y < 700.0,
            "{control} escaped resized window at ({x}, {y})"
        );
    }
}

#[test]
fn the_search_page_exclude_row_opens_and_results_keep_the_approved_layout() {
    use crate::gpu_viewer::editor::search::SearchSession;
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("fitting.txt"), "verify bolt torque\n").unwrap();
    let mut v = viewer(Some(Box::new(SearchSession::in_memory())));
    assert!(
        v.gui.find_target("Exclude input").is_none(),
        "hidden until switched on"
    );
    let chip = target(&v, "Exclude");
    click(&mut v, chip);
    frame(&mut v);
    assert!(
        v.gui.find_target("Exclude input").is_some(),
        "the switch shows the row"
    );
    // The new row has the keyboard: typing goes straight into it.
    type_text(&mut v, "draft");
    let doc = |v: &Viewer| v.editor.as_ref().unwrap().document().unwrap();
    assert_eq!(doc(&v).extra_inputs[0].text, "draft");
    let whole = target(&v, "Whole file");
    click(&mut v, whole);
    assert!(
        doc(&v).extra_inputs[0].choices[0].checked == Some(true),
        "the scope is a choice beside the input"
    );
    let s = v.editor.as_ref().unwrap();
    assert!(s.document().unwrap().extra_inputs[0]
        .note
        .as_deref()
        .unwrap()
        .contains("every file"));
    // A result, so both cards are shown.
    let folder = target(&v, "Folder input");
    click(&mut v, folder);
    type_text(&mut v, &dir.path().to_string_lossy());
    let find = target(&v, "Find input");
    click(&mut v, find);
    type_text(&mut v, "torque");
    egui_key(&mut v, egui::Key::Enter);
    let t = Instant::now();
    while v.editor.as_ref().unwrap().solving() || v.gui.find_target("row fitting.txt").is_none() {
        assert!(
            t.elapsed().as_secs() < 30,
            "no detail card; status {:?}",
            v.editor.as_ref().unwrap().status()
        );
        v.editor.as_mut().unwrap().poll();
        frame(&mut v);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(
        v.view.page_layout, None,
        "Search keeps artifact's results-left, preview-right layout"
    );
    assert!(
        v.gui.find_target("Move the results").is_none(),
        "approved layout has no draggable card chrome"
    );
    assert!(
        v.gui.find_target("Move the matches").is_none(),
        "approved layout has no draggable card chrome"
    );
    assert!(
        v.gui.find_target("Resize the cards").is_none(),
        "approved layout has no splitter chrome"
    );
}

#[test]
fn the_materials_page_searches_sorts_marks_on_double_click_and_compares() {
    use crate::gpu_viewer::editor::materials::MaterialsSession;
    use crate::toolboxes::material_lookup::model;
    let mut v = viewer(Some(Box::new(MaterialsSession::with_report(None))));
    assert!(v.page_mode());
    let doc = |v: &Viewer| v.editor.as_ref().unwrap().document().unwrap();
    let input = target(&v, "Search input");
    click(&mut v, input);
    type_text(&mut v, "7075");
    let want = model::filter_sort(&model::Query {
        text: "7075".into(),
        ..model::Query::default()
    });
    assert!(!want.is_empty());
    assert_eq!(
        doc(&v).table.rows.len(),
        want.len(),
        "typing filters with the toolbox's own query"
    );
    // A header click sorts by that column, largest first.
    let fty = target(&v, "Fty ksi column");
    click(&mut v, fty);
    let d = doc(&v);
    assert_eq!(
        d.table
            .columns
            .iter()
            .find(|c| c.title == "Fty ksi")
            .unwrap()
            .sorted,
        Some(false)
    );
    // Double-click two rows on screen (the sort keeps the highlighted material in view, and a click on a row cut by
    // the list's edge scrolls it in, so look again each time and skip the edge row): both are marked, in order.
    let mut marked: Vec<String> = Vec::new();
    for _ in 0..2 {
        let shown: Vec<String> = labels(&v)
            .into_iter()
            .filter_map(|l| l.strip_prefix("row ").map(str::to_string))
            .collect();
        let name = shown
            .iter()
            .skip(1)
            .find(|n| !marked.contains(n))
            .unwrap_or_else(|| panic!("{shown:?}"))
            .clone();
        let row = target(&v, &format!("row {name}"));
        double_click(&mut v, row);
        // The clicked row is the one highlighted, and a row in view does not move (egui's row positions match the
        // painted ones).
        assert_eq!(
            target(&v, &format!("row {name}")),
            row,
            "a click on a row in view does not scroll the list"
        );
        assert!(
            doc(&v)
                .table
                .rows
                .iter()
                .any(|r| r.selected && r.cells[0] == name),
            "the clicked row is highlighted"
        );
        marked.push(name);
    }
    let d = doc(&v);
    let badge = |name: &str| {
        d.table
            .rows
            .iter()
            .find(|r| r.cells[0] == name)
            .and_then(|r| r.badge.clone())
    };
    assert_eq!(
        (badge(&marked[0]), badge(&marked[1])),
        (Some("#1".into()), Some("#2".into()))
    );
    assert_eq!(d.chips[0].label, "Compare (2)");
    let compare = target(&v, "Compare (2)");
    click(&mut v, compare);
    let detail = doc(&v).detail.unwrap();
    assert!(detail.title.starts_with("Compare"), "{}", detail.title);
    assert_eq!(
        detail.grid.unwrap().columns,
        ["Property", "Unit", "#1", "#2"]
    );
    // Keys typed into the page never reach the viewer.
    let before = v.view.show_mesh;
    v.input(Input::Key(Key::Character("m".into())));
    assert_eq!(v.view.show_mesh, before);
}

#[test]
fn the_materials_rail_and_basket_answer_clicks() {
    use crate::gpu_viewer::editor::materials::MaterialsSession;
    let mut v = viewer(Some(Box::new(MaterialsSession::with_report(None))));
    let doc = |v: &Viewer| v.editor.as_ref().unwrap().document().unwrap();
    let all = doc(&v).table.rows.len();
    // A form checkbox in the rail narrows the table and puts a chip over it; the chip takes it back.
    let plate = target(&v, "FORM: Plate");
    click(&mut v, plate);
    let narrowed = doc(&v).table.rows.len();
    assert!(narrowed > 0 && narrowed < all);
    assert_eq!(doc(&v).table.chips.len(), 1);
    // Counts changed, so the rail may have been laid out again: look the control up afresh.
    let plate = target(&v, "FORM: Plate");
    click(&mut v, plate);
    assert_eq!(doc(&v).table.rows.len(), all);
    // A basis pill and Clear all.
    let basis = labels(&v)
        .into_iter()
        .find(|l| l.starts_with("BASIS: A-basis"))
        .expect("the A-basis pill");
    let pill = target(&v, &basis);
    click(&mut v, pill);
    assert!(doc(&v).table.rows.len() < all);
    let clear = target(&v, "Clear narrowing");
    click(&mut v, clear);
    assert_eq!(doc(&v).table.rows.len(), all);
    // A row's box fills a basket slot; the slot's cross empties it. Neither moves the highlight.
    // The restored list scrolls the highlighted row into view on the next frame: let it settle before aiming.
    frame(&mut v);
    // (Skip the first: the list's edge may cut it, and a click there lands on the header.)
    let first = labels(&v)
        .into_iter()
        .filter_map(|l| l.strip_prefix("Basket ").map(str::to_string))
        .nth(1)
        .expect("a row box in view");
    let at = |v: &Viewer| {
        doc(v)
            .table
            .rows
            .iter()
            .position(|r| r.cells[0] == first)
            .unwrap()
    };
    let highlighted = doc(&v).table.rows.iter().position(|r| r.selected);
    let boxed = target(&v, &format!("Basket {first}"));
    click(&mut v, boxed);
    let d = doc(&v);
    assert_eq!(
        d.basket
            .as_ref()
            .unwrap()
            .slots
            .iter()
            .flatten()
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>(),
        [first.as_str()]
    );
    assert!(d.table.rows[at(&v)].mark.unwrap().on);
    // ToggleMark moves the cursor onto the row it marks: the row's box and its highlight agree.
    assert!(d.table.rows[at(&v)].selected && highlighted.is_some());
    let remove = target(&v, &format!("Remove {first} from the basket"));
    click(&mut v, remove);
    assert!(doc(&v).basket.unwrap().slots.iter().all(Option::is_none));
}

#[test]
fn the_search_page_has_no_docks_and_its_guide_and_options_open_as_popovers() {
    use crate::gpu_viewer::editor::search::SearchSession;
    let mut v = viewer(Some(Box::new(SearchSession::in_memory())));
    let labels = v.gui.target_labels();
    assert!(
        !labels
            .iter()
            .any(|l| l.ends_with(" tab") || l == "Close the inspector"),
        "no step strip, tool pill or inspector: {labels:?}"
    );
    let vp = v.viewport();
    assert!(
        vp.w > 1600 - 260 - 20,
        "the page has the whole width beside the tree: {vp:?}"
    );
    // The guide: open it, click an example, it fills Find and closes.
    let link = target(&v, "How to write a search");
    click(&mut v, link);
    let example = target(&v, "Example bolt, torque [near 3]");
    click(&mut v, example);
    assert_eq!(
        v.editor.as_ref().unwrap().document().unwrap().inputs[1].text,
        "bolt, torque [near 3]"
    );
    assert!(
        v.gui.find_target("Example bolt").is_none(),
        "the guide closed"
    );
    // Options: a popover with the settings; a switch flips in place and the popover stays.
    let options = target(&v, "Options panel");
    click(&mut v, options);
    let regex = target(&v, "Use regex");
    click(&mut v, regex);
    assert!(v
        .editor
        .as_ref()
        .unwrap()
        .document()
        .unwrap()
        .chips
        .iter()
        .any(|c| c.label == "Regex" && c.checked == Some(true)));
    assert!(
        v.gui.find_target("Whole word").is_some(),
        "still open after a change"
    );
    key(&mut v, NamedKey::Escape);
    assert!(
        v.gui.find_target("Exclude folders").is_none(),
        "Esc closes it"
    );
    // Report: the output folder is an input with its own Browse, as the folder to search is.
    let report = target(&v, "Report panel");
    click(&mut v, report);
    let out = target(&v, "Output folder input");
    click(&mut v, out);
    type_text(&mut v, "/reports");
    assert_eq!(
        v.editor
            .as_ref()
            .unwrap()
            .document()
            .unwrap()
            .panels
            .iter()
            .find(|panel| panel.label == "Report")
            .unwrap()
            .inputs[0]
            .text,
        "/reports"
    );
    let labels = v.gui.target_labels();
    assert_eq!(
        labels.iter().filter(|l| *l == "Browse").count(),
        2,
        "Browse beside both folders: {labels:?}"
    );
    assert!(!v.quit, "Esc closed the popover, not the window");
}
