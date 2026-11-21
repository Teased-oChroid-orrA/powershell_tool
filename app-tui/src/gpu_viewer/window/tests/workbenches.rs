//! Bushing, Fastener Hole, Pressure Vessel, Preload and Lug workbenches by click and typing.
use super::*;

fn bushing_workbench_text(v: &Viewer) -> String {
    let workbench = v.editor.as_ref().unwrap().workbench().unwrap();
    workbench
        .inputs
        .iter()
        .chain(&workbench.results)
        .flat_map(|card| card.items.iter().filter_map(|item| item.plain_text()))
        .collect::<Vec<_>>()
        .join("\n")
}

fn click_bushing_target(v: &mut Viewer, label: &str) {
    for _ in 0..8 {
        let at = target(v, label);
        move_to(v, at);
        let after = target(v, label);
        if (at.1 - after.1).abs() < 1.0 {
            button(v, MouseButton::Left, true);
            button(v, MouseButton::Left, false);
            frame(v);
            return;
        }
    }
    panic!("{label} kept moving while the pointer approached it");
}

/// The Bushing workbench through the real window: an input click reaches the solver and updates the verdict.
#[test]
fn the_bushing_workbench_edits_an_input_by_click_and_typing_and_the_verdict_follows() {
    let dir = std::env::temp_dir().join(format!("window-bushing-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let session =
        crate::gpu_viewer::editor::bushing::BushingSession::open(&dir.join("case.json")).unwrap();
    let mut v = viewer(Some(Box::new(session)));
    frame(&mut v);
    // The default case already fails the edge-distance check, so that input carries the marker.
    assert!(
        bushing_workbench_text(&v).contains("! Edge Distance  0.7500 in"),
        "{}",
        bushing_workbench_text(&v)
    );
    let at = target(&v, "Edge Distance");
    click(&mut v, at);
    assert!(v.editor.as_ref().unwrap().prompting());
    for c in "0.3".chars() {
        v.input(Input::Key(Key::Character(c.to_string().into())));
    }
    v.input(Input::Key(Key::Named(NamedKey::Enter)));
    frame(&mut v);
    assert!(!v.editor.as_ref().unwrap().prompting());
    let text = bushing_workbench_text(&v);
    assert!(text.contains("Edge Distance  0.3000 in"), "{text}");
    assert!(
        text.contains("REVIEW") && text.contains("margin -"),
        "a 0.3 in edge distance fails: {text}"
    );
    assert!(text.contains("Contact pressure"), "{text}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn bushing_fix_action_reopens_a_manually_closed_card() {
    let path =
        std::env::temp_dir().join(format!("window-bushing-fixes-{}.json", std::process::id()));
    let session = crate::gpu_viewer::editor::bushing::BushingSession::open(&path).unwrap();
    let mut v = viewer(Some(Box::new(session)));
    v.size_override = Some((1600, 1800));
    frame(&mut v);
    v.gui.set_card_open("bushing-fixes", false);
    assert_eq!(v.gui.card_open("bushing-fixes"), Some(false));
    let fix = v
        .gui
        .target_labels()
        .into_iter()
        .find(|label| label.starts_with("Fixes for "))
        .expect("verdict fix action");
    let fix = target(&v, &fix);
    click(&mut v, fix);
    assert_eq!(v.gui.card_open("bushing-fixes"), Some(true));
}

#[test]
fn bushing_uncertainty_edits_by_click_and_typing() {
    let dir = std::env::temp_dir().join(format!("window-bushing-cv-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let session =
        crate::gpu_viewer::editor::bushing::BushingSession::open(&dir.join("case.json")).unwrap();
    let mut v = viewer(Some(Box::new(session)));
    v.size_override = Some((1600, 1800));
    frame(&mut v);
    // The ways to fix are open whenever something fails and would push the cross-check out of view.
    v.gui.set_card_open("bushing-fixes", false);
    v.gui.set_card_open("bushing-edge-check", true);
    frame(&mut v);
    frame(&mut v);
    for label in [
        "Strength CV (%)",
        "Stress superposition model CV (%)",
        "Contact FE (bushing + housing, elastic-plastic) model CV (%)",
    ] {
        click_bushing_target(&mut v, label);
        assert!(v.editor.as_ref().unwrap().prompting(), "{label}");
        for c in "12.5".chars() {
            v.input(Input::Key(Key::Character(c.to_string().into())));
        }
        v.input(Input::Key(Key::Named(NamedKey::Enter)));
        frame(&mut v);
        assert!(!v.editor.as_ref().unwrap().prompting());
        assert!(
            bushing_workbench_text(&v).contains(&format!("{label}  12.5")),
            "{}",
            bushing_workbench_text(&v)
        );
    }
    let _ = std::fs::remove_dir_all(dir);
}

/// The Fastener Hole workbench through the real window: a click on a dimension starts a typed edit, the number reaches
/// the fit, a click on a choice switches to the countersink, and the drawing follows.
#[test]
fn the_fastener_hole_workbench_edits_by_click_and_typing_and_switches_hole_type() {
    let dir = std::env::temp_dir().join(format!("window-fastener-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let session =
        crate::gpu_viewer::editor::fastener::FastenerSession::open(&dir.join("case.json")).unwrap();
    let mut v = viewer(Some(Box::new(session)));
    frame(&mut v);
    assert!(
        property_text(&v).contains("TRANSITION"),
        "{}",
        property_text(&v)
    );
    let at = target(&v, "Hole 2 Diameter Nominal");
    click(&mut v, at);
    assert!(v.editor.as_ref().unwrap().prompting());
    for c in "0.26".chars() {
        v.input(Input::Key(Key::Character(c.to_string().into())));
    }
    v.input(Input::Key(Key::Named(NamedKey::Enter)));
    frame(&mut v);
    assert!(
        property_text(&v).contains("Fit  0.0100 -0.0000/+0.0000 in")
            && property_text(&v).contains("CLEARANCE"),
        "{}",
        property_text(&v)
    );
    let regular = v.scene().positions.len();
    let at = target(&v, "Hole Type");
    click(&mut v, at);
    frame(&mut v);
    assert!(
        property_text(&v).contains("Angle (calculated)  90.000"),
        "{}",
        property_text(&v)
    );
    assert_ne!(
        v.scene().positions.len(),
        regular,
        "the drawing follows the hole type"
    );
    assert!(
        tree_labels(&v).iter().any(|l| l == "Inputs (Countersunk)"),
        "{:?}",
        tree_labels(&v)
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// The Pressure Vessel window: a click on an input types it and the checks and the contour follow; Material opens
/// the picker card.
#[test]
fn the_pressure_vessel_workbench_edits_by_click_and_typing_and_opens_the_material_picker() {
    let dir = std::env::temp_dir().join(format!("window-vessel-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let session =
        crate::gpu_viewer::editor::vessel::VesselSession::open(&dir.join("case.json")).unwrap();
    let mut v = viewer(Some(Box::new(session)));
    frame(&mut v);
    assert!(
        property_text(&v).contains("Inner diameter  4.0000 in"),
        "{}",
        property_text(&v)
    );
    let hoop = v.scene().range;
    let at = target(&v, "Wall Thickness");
    click(&mut v, at);
    assert!(v.editor.as_ref().unwrap().prompting());
    for c in "0.5".chars() {
        v.input(Input::Key(Key::Character(c.to_string().into())));
    }
    v.input(Input::Key(Key::Named(NamedKey::Enter)));
    frame(&mut v);
    assert!(
        property_text(&v).contains("Inner diameter  5.0000 in"),
        "{}",
        property_text(&v)
    );
    assert_ne!(v.scene().range, hoop, "the contour follows the wall");
    let hoop = v.scene().range;
    for (label, text) in [
        ("Corrosion Allowance", "0.125"),
        ("Mill Undertolerance", "12.5"),
    ] {
        let at = target(&v, label);
        click(&mut v, at);
        assert!(v.editor.as_ref().unwrap().prompting());
        for c in text.chars() {
            v.input(Input::Key(Key::Character(c.to_string().into())));
        }
        v.input(Input::Key(Key::Named(NamedKey::Enter)));
        frame(&mut v);
    }
    assert!(
        property_text(&v).contains("Remaining wall  0.3125 in"),
        "{}",
        property_text(&v)
    );
    assert!(
        property_text(&v).contains("Inner diameter  5.3750 in"),
        "{}",
        property_text(&v)
    );
    assert!(
        v.scene().range[1] > hoop[1],
        "both allowances reach the contour through native input"
    );
    // The inspector's Material row (the tree and the ribbon have one too).
    let at = v
        .gui
        .find_target_after("Material card", "Material")
        .unwrap();
    click(&mut v, at);
    frame(&mut v);
    assert!(v
        .editor
        .as_ref()
        .unwrap()
        .value_card()
        .is_some_and(|c| c.title == "Material"));
    let _ = std::fs::remove_dir_all(dir);
}

/// The Pressure Vessel line run: the switch opens the page over the viewport, its table takes clicks and typing, and
/// the wall solve answers through the worker.
#[test]
fn the_pressure_vessel_line_run_page_takes_clicks_in_its_table() {
    let dir = std::env::temp_dir().join(format!("window-vessel-line-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let session =
        crate::gpu_viewer::editor::vessel::VesselSession::open(&dir.join("case.json")).unwrap();
    let mut v = viewer(Some(Box::new(session)));
    frame(&mut v);
    assert!(v.editor.as_ref().unwrap().page().is_empty());
    let at = target(&v, "Supported line run");
    click(&mut v, at);
    let t = Instant::now();
    while v.editor.as_ref().unwrap().solving() {
        v.editor.as_mut().unwrap().poll();
        frame(&mut v);
        assert!(t.elapsed().as_secs() < 60, "timed out");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    v.editor.as_mut().unwrap().poll();
    frame(&mut v);
    let page = |v: &Viewer| {
        v.editor
            .as_ref()
            .unwrap()
            .page()
            .iter()
            .filter_map(|i| i.plain_text())
            .collect::<Vec<_>>()
            .join("\n")
    };
    assert!(
        page(&v).contains("Sum of reactions"),
        "{} / {:?} / {:?}",
        page(&v),
        v.editor.as_ref().unwrap().status(),
        v.gui.target_labels()
    );
    // The table's add row and a label cell are clickable controls of the page.
    let at = target(&v, "Add requirement");
    click(&mut v, at);
    frame(&mut v);
    let label = v
        .editor
        .as_ref()
        .unwrap()
        .page()
        .iter()
        .find_map(|i| i.plain_text().filter(|t| t.contains("Requirement ")));
    assert!(label.is_some(), "{}", page(&v));
    let name = target_label_with(&v, "Requirement ");
    let at = target(&v, &name);
    click(&mut v, at);
    assert!(
        v.editor.as_ref().unwrap().prompting(),
        "a click on a label cell types into it"
    );
    v.input(Input::Key(Key::Character("x".into())));
    v.input(Input::Key(Key::Named(NamedKey::Enter)));
    frame(&mut v);
    assert!(page(&v).contains(&format!("{name}x")), "{}", page(&v));
    let _ = std::fs::remove_dir_all(dir);
}
/// Segment mode by click: the switch, an end's condition, the bound and a valve at the end, then the page shows both
/// bounds and says the cut is an idealisation.
#[test]
fn the_pressure_vessel_segment_card_takes_clicks_and_the_page_shows_both_bounds() {
    let dir = std::env::temp_dir().join(format!("window-vessel-segment-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let session =
        crate::gpu_viewer::editor::vessel::VesselSession::open(&dir.join("case.json")).unwrap();
    let mut v = viewer(Some(Box::new(session)));
    frame(&mut v);
    let settle = |v: &mut Viewer| {
        let t = Instant::now();
        while v.editor.as_ref().unwrap().solving() {
            v.editor.as_mut().unwrap().poll();
            frame(v);
            assert!(t.elapsed().as_secs() < 60, "timed out");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        v.editor.as_mut().unwrap().poll();
        frame(v);
    };
    let page = |v: &Viewer| {
        v.editor
            .as_ref()
            .unwrap()
            .page()
            .iter()
            .filter_map(|i| i.plain_text())
            .collect::<Vec<_>>()
            .join("\n")
    };
    // A row below the fold of the inspector is scrolled into view like a person would, then clicked.
    let press = |v: &mut Viewer, label: &str| {
        if target(v, label).1 > 800.0 {
            let tab = target(v, "Supported line run");
            move_to(v, (tab.0, tab.1 - 100.0));
            v.test_events.push(egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, -400.0),
                phase: egui::TouchPhase::Move,
                modifiers: Default::default(),
            });
            for _ in 0..12 {
                frame(v);
            }
        }
        let at = target(v, label);
        click(v, at);
        settle(v);
    };
    press(&mut v, "Supported line run");
    press(&mut v, "Segment between two supports");
    assert!(
        page(&v).contains("A cut segment is an idealisation"),
        "{}",
        page(&v)
    );
    // The bound switch runs both.
    press(&mut v, "Bound (pinned and clamped)");
    assert!(
        page(&v).contains("(pinned)"),
        "the bound table names the worse bound: {}",
        page(&v)
    );
    press(&mut v, "  + Valve or fitting at this end");
    assert!(
        v.gui.target_labels().iter().any(|l| l == "Load 1 label"),
        "a valve was added: {:?}",
        v.gui.target_labels()
    );
    let _ = std::fs::remove_dir_all(dir);
}
fn target_label_with(v: &Viewer, prefix: &str) -> String {
    v.gui
        .target_labels()
        .into_iter()
        .find(|l| l.starts_with(prefix))
        .unwrap_or_else(|| panic!("no control starting {prefix:?}"))
}

/// The Preload Analysis window: a click on Applied Torque types it and the preload follows; the Bolt row opens the
/// bolt picker card.
#[test]
fn the_preload_workbench_edits_the_torque_by_click_and_typing_and_opens_the_bolt_picker() {
    let dir = std::env::temp_dir().join(format!("window-preload-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let session =
        crate::gpu_viewer::editor::preload::PreloadSession::open(&dir.join("case.json")).unwrap();
    let mut v = viewer(Some(Box::new(session)));
    frame(&mut v);
    let before = property_text(&v);
    assert!(before.contains("Applied Torque  360.00 in-lbf"), "{before}");
    let at = target(&v, "Applied Torque");
    click(&mut v, at);
    assert!(v.editor.as_ref().unwrap().prompting());
    for c in "180".chars() {
        v.input(Input::Key(Key::Character(c.to_string().into())));
    }
    v.input(Input::Key(Key::Named(NamedKey::Enter)));
    frame(&mut v);
    assert!(!v.editor.as_ref().unwrap().prompting());
    let after = property_text(&v);
    assert!(after.contains("Applied Torque  180.00 in-lbf"), "{after}");
    let preload = |t: &str| {
        t.lines()
            .find(|l| l.starts_with("Preload  "))
            .map(str::to_string)
    };
    assert!(
        preload(&after).is_some() && preload(&after) != preload(&before),
        "half the torque, another preload: {:?} -> {:?}",
        preload(&before),
        preload(&after)
    );
    let at = target(&v, "Bolt (AN Standard)");
    click(&mut v, at);
    frame(&mut v);
    assert!(v
        .editor
        .as_ref()
        .unwrap()
        .value_card()
        .is_some_and(|c| c.title == "Bolt"));
    let _ = std::fs::remove_dir_all(dir);
}

/// The Lug Analysis window: a click on Pin Load P types it; the Lug Material row opens the material picker card.
#[test]
fn the_lug_workbench_edits_the_load_by_click_and_typing_and_opens_the_material_picker() {
    let dir = std::env::temp_dir().join(format!("window-lug-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let session = crate::gpu_viewer::editor::lug::LugSession::open(&dir.join("case.json")).unwrap();
    let mut v = viewer(Some(Box::new(session)));
    frame(&mut v);
    let before = property_text(&v);
    assert!(before.contains("Pin Load P  4000.0 lbf"), "{before}");
    let at = target(&v, "Pin Load P");
    click(&mut v, at);
    assert!(v.editor.as_ref().unwrap().prompting());
    for c in "5200".chars() {
        v.input(Input::Key(Key::Character(c.to_string().into())));
    }
    v.input(Input::Key(Key::Named(NamedKey::Enter)));
    frame(&mut v);
    assert!(!v.editor.as_ref().unwrap().prompting());
    let after = property_text(&v);
    assert!(after.contains("Pin Load P  5200.0 lbf"), "{after}");
    assert!(v.editor.as_ref().unwrap().dirty());
    let at = target(&v, "Lug Material");
    click(&mut v, at);
    frame(&mut v);
    assert!(v
        .editor
        .as_ref()
        .unwrap()
        .value_card()
        .is_some_and(|c| c.title == "Lug material"));
    let _ = std::fs::remove_dir_all(dir);
}
