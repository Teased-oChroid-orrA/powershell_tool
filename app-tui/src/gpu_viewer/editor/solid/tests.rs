use super::*;
use crate::gpu_viewer::{
    controller::UiAction,
    editor::{authoring::Authoring, camera::Camera, session::EditorAction},
    ui::DockItem,
};
use fea_problem::Support;
use std::time::Instant;

fn temp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("solid-session-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d.join("model.json")
}
/// A coarse plate with a hole: fast to mesh and solve in a debug build.
fn plate() -> Problem {
    let mut p = templates().remove(0).1;
    p.mesh.size = 1.0;
    p
}
fn session(name: &str) -> SolidSession {
    SolidSession::new(plate(), temp(name))
}
/// Run `poll` until the preview mesh for the current inputs exists.
fn meshed(s: &mut SolidSession) {
    s.do_action(SolidAction::GenerateMesh);
    let t = Instant::now();
    while !s.preview_current() {
        s.poll();
        assert!(t.elapsed().as_secs() < 60, "preview did not finish: {:?}", s.preview_error);
        std::thread::sleep(Duration::from_millis(5));
    }
}
fn solved(s: &mut SolidSession) {
    s.solve();
    let t = Instant::now();
    while s.solving() {
        s.poll();
        assert!(t.elapsed().as_secs() < 120, "solve did not finish");
        std::thread::sleep(Duration::from_millis(5));
    }
    s.poll();
}
fn row_index(s: &SolidSession, row: FieldRow) -> usize {
    s.rows().iter().position(|r| *r == row).unwrap_or_else(|| panic!("row {row:?}"))
}
fn type_text(s: &mut SolidSession, text: &str) {
    for c in text.chars() {
        s.handle_key(Key::Char(c), Mods::default());
    }
    s.handle_key(Key::Enter, Mods::default());
}
fn texts(s: &SolidSession) -> Vec<String> {
    s.properties()
        .into_iter()
        .filter_map(|i| match i {
            DockItem::Heading(t) => Some(t),
            i @ (DockItem::Line { .. } | DockItem::Field { .. }) => i.plain_text(),
            _ => None,
        })
        .collect()
}

#[test]
fn the_preview_mesh_is_built_on_a_worker_and_colours_supports_loads_and_selection() {
    let mut s = session("preview");
    assert!(s.preview_summary().is_err(), "nothing meshed before the first poll");
    meshed(&mut s);
    let (nodes, elements) = s.preview_summary().unwrap();
    assert!(nodes > 20 && elements > 10);
    let scene = s.build_scene().unwrap();
    assert!(scene.label.contains("geometry"), "{}", scene.label);
    assert!(scene.positions.len() > nodes, "the mesh plus the hole's refinement ring");
    assert_eq!(scene.range, [0.0, 3.0]);
    let values: Vec<f32> = scene.samples.iter().map(|v| v[3]).collect();
    // The template has a roller on "left" and a traction on "right": both are coloured, the body is not.
    assert!(values.contains(&0.0) && values.contains(&1.0) && values.contains(&2.0), "{values:?}");
    assert!(!values.contains(&3.0), "nothing selected yet");
    s.do_action(SolidAction::SelectSurface(s.names().iter().position(|n| n == "top").unwrap()));
    assert!(s.needs_scene());
    let values: Vec<f32> = s.build_scene().unwrap().samples.iter().map(|v| v[3]).collect();
    assert!(values.contains(&3.0), "the selected surface is red");
    assert!(!s.needs_scene());
}

#[test]
fn an_invalid_geometry_shows_a_placeholder_and_the_reason_instead_of_failing() {
    let mut s = session("invalid");
    s.problem.thickness = -1.0;
    s.problem.mesh.size = 0.0;
    s.do_action(SolidAction::GenerateMesh);
    s.poll();
    let scene = s.build_scene().unwrap();
    assert!(scene.label.contains("no mesh"), "{}", scene.label);
    assert!(s.preview_summary().is_err());
}

#[test]
fn typing_a_number_edits_the_focused_row_and_undo_redo_walk_the_history() {
    let mut s = session("edit");
    let row = row_index(&s, FieldRow::MeshSize);
    s.focus = row;
    let before = s.problem.mesh.size;
    s.handle_key(Key::Char('2'), Mods::default());
    assert!(s.prompting(), "a digit starts the edit");
    type_text(&mut s, ".5");
    assert_eq!(s.problem.mesh.size, 2.5);
    assert!(s.dirty && s.can_undo());
    s.handle_key(Key::Char('z'), Mods { command: true, shift: false });
    assert_eq!(s.problem.mesh.size, before);
    assert!(s.can_redo());
    s.handle_key(Key::Char('z'), Mods { command: true, shift: true });
    assert_eq!(s.problem.mesh.size, 2.5);
    // Escape cancels an edit and leaves the value alone.
    s.handle_key(Key::Enter, Mods::default());
    assert!(s.prompting());
    for c in "99".chars() {
        s.handle_key(Key::Char(c), Mods::default());
    }
    s.handle_key(Key::Escape, Mods::default());
    assert!(!s.prompting() && s.problem.mesh.size == 2.5);
    // A no-op commit records nothing.
    let depth = s.undo.len();
    s.handle_key(Key::Enter, Mods::default());
    s.handle_key(Key::Enter, Mods::default());
    assert_eq!(s.undo.len(), depth, "re-entering the same value is not an edit");
}

#[test]
fn rows_cycle_add_and_remove_exactly_like_the_terminal_workbench() {
    let mut s = session("rows");
    let holes = |s: &SolidSession| match &s.problem.geometry {
        Geometry::Sketch { holes, .. } => holes.len(),
        _ => 0,
    };
    assert_eq!(holes(&s), 1);
    s.do_action(SolidAction::Activate(FieldRow::HoleAdd));
    assert_eq!(holes(&s), 2);
    // Focus the new hole and remove it.
    s.do_action(SolidAction::FocusFeature(Feature::Hole(1)));
    assert_eq!(s.focused_row().map(feature_of), Some(Feature::Hole(1)));
    s.do_action(SolidAction::RemoveFocused);
    assert_eq!(holes(&s), 1);
    // Left/Right cycle the analysis type through the same list.
    s.focus = row_index(&s, FieldRow::Analysis);
    let a = s.problem.analysis;
    s.handle_key(Key::Right, Mods::default());
    assert_ne!(s.problem.analysis, a);
    s.handle_key(Key::Left, Mods::default());
    assert_eq!(s.problem.analysis, a);
    // Removing needs a removable focus.
    s.focus = row_index(&s, FieldRow::MeshSize);
    assert!(s.remove_availability().is_err());
    s.do_action(SolidAction::RemoveFocused);
    assert!(s.status.contains("focus a row"));
}

#[test]
fn material_search_picks_the_first_catalog_match_and_reports_misses() {
    let mut s = session("material");
    s.focus = row_index(&s, FieldRow::Material);
    s.handle_key(Key::Enter, Mods::default());
    assert!(s.prompting() && texts(&s).iter().any(|t| t == "Material search"));
    type_text(&mut s, "7075 t6");
    assert!(s.problem.material.name.to_lowercase().contains("7075"), "{}", s.problem.material.name);
    assert!(s.problem.material.e > 1.0e6, "E comes from the catalog (psi)");
    assert!(s.status.starts_with("Material"));
    s.handle_key(Key::Enter, Mods::default());
    type_text(&mut s, "no such alloy");
    assert!(s.status.contains("No material matches"));
}

#[test]
fn surfaces_are_picked_by_name_and_clicks_assign_them_to_the_focused_edge() {
    let mut s = session("pick");
    meshed(&mut s);
    s.build_scene().unwrap();
    let cam = Camera { yaw: 0.0, pitch: 0.0, zoom: 1.0, aspect: 1.0, pan: [0.0, 0.0] };
    let size = [800.0, 800.0];
    let mesh = s.preview.as_ref().unwrap().mesh.clone();
    // Pixel of a node on each named edge, offset by 3 px away from the line.
    let positions = s.pick_positions.clone();
    let pixel_of = |name: &str, dx: f32| {
        // A middle node: corner nodes belong to two edges.
        let set = SolidSession::nodes_of(&mesh, name);
        let node = *set.iter().nth(set.len() / 2).unwrap();
        let (ndc, _) = cam.project(positions[node]);
        let p = Camera::pixel(ndc, size[0], size[1]);
        [p[0] + dx, p[1]]
    };
    for name in ["left", "right", "top", "bottom"] {
        assert_eq!(s.pick(&cam, pixel_of(name, 0.0), size).as_deref(), Some(name), "{name}");
    }
    assert_eq!(s.pick(&cam, [5.0, 5.0], size), None, "far from every surface");
    // A click selects; with a support focused it assigns the edge instead.
    assert!(s.click(pixel_of("top", 0.0), size, &cam));
    assert_eq!(s.selected.as_deref(), Some("top"));
    s.focus = row_index(&s, FieldRow::SupportEdge(0));
    let was = s.problem.supports[0].edge().map(str::to_string);
    assert!(s.click(pixel_of("bottom", 0.0), size, &cam));
    assert_eq!(s.problem.supports[0].edge(), Some("bottom"));
    assert_ne!(was.as_deref(), Some("bottom"));
    assert!(s.can_undo(), "assigning is an undoable edit");
    // Clicking the background clears the selection.
    s.click([5.0, 5.0], size, &cam);
    assert_eq!(s.selected, None);
}

#[test]
fn support_and_load_can_be_added_on_the_selected_surface() {
    let mut s = session("onsel");
    assert!(s.selected_ok().is_err());
    s.do_action(SolidAction::SupportOnSelected);
    assert!(s.status.contains("Pick a surface"));
    let (sup, loads) = (s.problem.supports.len(), s.problem.loads.len());
    s.selected = Some("top".into());
    s.do_action(SolidAction::SupportOnSelected);
    assert_eq!(s.problem.supports.len(), sup + 1);
    assert_eq!(s.problem.supports.last().and_then(Support::edge), Some("top"));
    assert!(matches!(s.focused_row(), Some(FieldRow::SupportKind(i)) if i == sup), "{:?}", s.focused_row());
    s.do_action(SolidAction::LoadOnSelected);
    assert_eq!(s.problem.loads.len(), loads + 1);
    assert_eq!(s.problem.loads.last().and_then(|l| l.edge()), Some("top"));
}

#[test]
fn solving_runs_on_a_worker_and_the_results_carry_every_field() {
    let mut s = session("solve");
    assert!(s.solve_availability().is_ok());
    assert!(s.results_availability().is_err());
    solved(&mut s);
    assert!(s.current_solution().is_some() && s.show_results, "{}", s.status);
    assert!(s.status.starts_with("Solved"));
    assert!(s.solve_availability().unwrap_err().contains("already current"));
    let scene = s.build_scene().unwrap();
    assert_eq!(scene.fields.len(), 13, "14 fields: the primary plus thirteen selectable ones (stresses, strains, displacements)");
    assert!(texts(&s).iter().any(|t| t == "Result") && texts(&s).iter().any(|t| t.starts_with("max von Mises")));
    // Editing makes the result stale and returns to the model.
    s.focus = row_index(&s, FieldRow::MeshSize);
    s.handle_key(Key::Char('2'), Mods::default());
    type_text(&mut s, "");
    assert!(s.current_solution().is_none());
    assert!(texts(&s).iter().any(|t| t.contains("stale")));
    // While results are shown a click does not edit the model.
    s.show_results = true;
    s.solved = Some(Arc::new(fea_problem::solve(&s.problem, None).unwrap()));
    let cam = Camera { yaw: 0.0, pitch: 0.0, zoom: 1.0, aspect: 1.0, pan: [0.0, 0.0] };
    let depth = s.undo.len();
    assert!(s.click([100.0, 100.0], [800.0, 800.0], &cam));
    assert!(s.status.contains("press V"));
    assert_eq!(s.undo.len(), depth);
}

#[test]
fn an_invalid_model_cannot_be_solved_and_says_why() {
    let mut s = session("badsolve");
    s.problem.supports.clear();
    let why = s.solve_availability().unwrap_err();
    assert!(!why.is_empty(), "the validation message is shown");
    s.do_action(SolidAction::Solve);
    assert_eq!(s.status, why);
    assert!(!s.solving());
}

#[test]
fn a_cancelled_solve_is_ignored_when_it_finishes() {
    let mut s = session("cancel");
    s.solve();
    assert!(s.solving());
    s.handle_key(Key::Escape, Mods::default());
    assert!(!s.solving() && s.status.contains("cancelled"));
    let t = Instant::now();
    while s.solve_worker.is_busy() {
        s.poll();
        assert!(t.elapsed().as_secs() < 120);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(s.solved.is_none(), "the cancelled answer was dropped");
}

#[test]
fn save_and_load_use_the_workbench_json_and_stay_compatible() {
    let mut s = session("files");
    let path = s.path.clone();
    s.problem.name = "native solid".into();
    s.dirty = true;
    assert!(s.save_availability().is_ok());
    s.do_action(SolidAction::Save);
    assert!(!s.dirty && s.status.starts_with("Saved"));
    // The terminal Workbench reads the same file with `Problem::from_json`.
    let back = Problem::from_json(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(back, s.problem);
    assert!(s.save_availability().is_err());
    s.problem.name = "changed".into();
    s.do_action(SolidAction::Load);
    assert_eq!(s.problem.name, "native solid");
    assert!(!s.dirty);
    std::fs::write(&path, "garbage").unwrap();
    s.do_action(SolidAction::Load);
    assert!(s.status.starts_with("Load failed"));
    assert_eq!(s.problem.name, "native solid", "a failed load keeps the model");
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn imported_meshes_need_a_file_and_report_unreadable_paths() {
    let mut s = session("import");
    s.do_action(SolidAction::Activate(FieldRow::Source));
    assert!(matches!(s.problem.geometry, Geometry::Imported { .. }));
    assert!(s.solve_availability().is_err());
    s.do_action(SolidAction::GenerateMesh);
    s.poll();
    let why = s.preview_summary().unwrap_err();
    assert!(why.to_lowercase().contains("mesh") && !why.starts_with("not generated"), "{why}");
    s.focus = row_index(&s, FieldRow::ImportPath);
    s.handle_key(Key::Enter, Mods::default());
    type_text(&mut s, "/definitely/not/here.msh");
    assert!(s.status.contains("Could not read"), "{}", s.status);
    // Toggling back restores the sketch that was there.
    s.do_action(SolidAction::Activate(FieldRow::Source));
    assert!(matches!(s.problem.geometry, Geometry::Sketch { .. }));
}

#[test]
fn tree_follows_the_focus_and_clicking_features_and_surfaces_acts() {
    let mut s = session("tree");
    meshed(&mut s);
    let tree = Authoring::tree(&s);
    let find = |t: &[crate::gpu_viewer::ui::TreeNode], prefix: &str| t.iter().position(|n| n.label.starts_with(prefix)).unwrap_or_else(|| panic!("{prefix}: {:?}", t.iter().map(|n| &n.label).collect::<Vec<_>>()));
    for prefix in ["Analysis:", "Material:", "Outline:", "Hole 1", "Mesh:", "Supports (", "Loads (", "Surfaces ("] {
        find(&tree, prefix);
    }
    s.do_action(SolidAction::FocusFeature(Feature::Mesh));
    let tree = Authoring::tree(&s);
    assert!(tree[find(&tree, "Mesh:")].selected && !tree[find(&tree, "Material:")].selected);
    // Surface rows select (and highlight) the named surface.
    let i = find(&tree, "left");
    assert_eq!(tree[i].action, Some(UiAction::Editor(EditorAction::Solid(SolidAction::SelectSurface(s.names().iter().position(|n| n == "left").unwrap())))));
    s.act(EditorAction::Solid(SolidAction::SelectSurface(s.names().iter().position(|n| n == "left").unwrap())));
    let tree = Authoring::tree(&s);
    assert!(tree[find(&tree, "left")].selected);
}

#[test]
fn ribbon_menus_and_context_menu_explain_what_is_unavailable() {
    let s = session("ribbon");
    let ribbon = Authoring::ribbon(&s);
    assert_eq!(ribbon.iter().map(|t| t.title.as_str()).collect::<Vec<_>>(), ["Model", "Features", "Mesh", "Simulation"]);
    let buttons: Vec<_> = ribbon.iter().flat_map(|t| &t.groups).flat_map(|g| &g.buttons).collect();
    let by = |l: &str| buttons.iter().find(|b| b.label == l).unwrap_or_else(|| panic!("{l}"));
    assert!(by("Solve").enabled && !by("Cancel solve").enabled);
    assert!(by("Cancel solve").reason.as_deref().is_some_and(|r| r.contains("no solve")));
    assert!(!by("Show results").enabled && by("Show results").reason.as_deref() == Some("solve the model first"));
    assert!(!by("Import mesh").enabled && by("Import mesh").reason.as_deref().is_some_and(|r| r.contains("Imported")));
    assert!(!by("Support on surface").enabled);
    assert!(!by("Save").enabled && !by("Undo").enabled);
    // Menus use the same rules.
    let titles: Vec<_> = Authoring::menus(&s).into_iter().map(|m| m.title).collect();
    assert_eq!(titles, ["File", "Edit", "Insert", "Tools", "Simulation", "Advisor"]);
    // Right-click menu with a selected surface offers to use it.
    let mut s = s;
    s.selected = Some("top".into());
    let labels: Vec<String> = Authoring::context_menu(&s).into_iter().filter_map(|i| if let crate::gpu_viewer::menu::Item::Entry(e) = i { Some(e.label) } else { None }).collect();
    assert!(labels.contains(&"Fixed".to_string()) && labels.contains(&"Solve".to_string()), "{labels:?}");
}

#[test]
fn the_inspector_shows_only_what_the_selection_owns_and_the_choices_are_the_tree_rows_menu() {
    use crate::gpu_viewer::editor::authoring::Relevance;
    let mut s = session("dock");
    let cards = Authoring::inspector(&s);
    let primary: Vec<&crate::gpu_viewer::editor::authoring::Card> = cards.iter().filter(|c| c.relevance == Relevance::Primary).collect();
    assert_eq!(primary.len(), 1, "one primary card for the focused feature");
    let lines = texts(&s);
    // The problem's own values are shown; another feature's values and every choice are not repeated as rows.
    assert!(lines.iter().any(|l| l.starts_with("Thickness")), "{lines:?}");
    for other in ["Young", "Poisson", "Mesh Size", "Element Type", "Add Support", "Template", "Analysis"] {
        assert!(!lines.iter().any(|l| l.starts_with(other)), "{other} must not be listed: {lines:?}");
    }
    // Focusing a feature (a tree click) swaps the card.
    s.do_action(SolidAction::FocusFeature(Feature::Material));
    let lines = texts(&s);
    assert!(lines.iter().any(|l| l.starts_with("Young")) && !lines.iter().any(|l| l.starts_with("Thickness")), "{lines:?}");
    // Value rows still start editing on click.
    let i = row_index(&s, FieldRow::Young);
    s.act(EditorAction::Solid(SolidAction::Focus(i)));
    assert!(s.prompting(), "a number row starts editing on click");
    s.handle_key(Key::Escape, Mods::default());
    // The choices live on the tree row: right click Analysis, run the entry, the analysis changes.
    let tree = Authoring::tree(&s);
    let problem_node = tree.iter().find(|n| n.label.starts_with("Analysis:")).expect("analysis row").id;
    let menu = Authoring::tree_menu(&s, problem_node);
    let entry = menu.iter().find_map(|i| match i { crate::gpu_viewer::menu::Item::Entry(e) if e.label.starts_with("Analysis") => Some(e.clone()), _ => None }).expect("an Analysis entry");
    let before = s.problem.analysis;
    if let UiAction::Editor(EditorAction::Solid(a)) = entry.action {
        s.do_action(a);
    }
    assert_ne!(s.problem.analysis, before);
}

#[test]
fn every_tree_row_maps_to_the_feature_it_focuses_and_the_mesh_row_menu_offers_the_mesh_commands() {
    let s = session("nodes");
    for n in Authoring::tree(&s) {
        if let Some(UiAction::Editor(EditorAction::Solid(SolidAction::FocusFeature(f)))) = n.action {
            assert_eq!(super::inspect::node_feature(n.id), Some(f), "row {:?}", n.label);
        }
    }
    let mesh = Authoring::tree(&s).into_iter().find(|n| n.label.starts_with("Mesh")).unwrap().id;
    let labels: Vec<String> = Authoring::tree_menu(&s, mesh).into_iter().filter_map(|i| if let crate::gpu_viewer::menu::Item::Entry(e) = i { Some(e.label) } else { None }).collect();
    assert!(labels.iter().any(|l| l.starts_with("Element Type")) && labels.contains(&"Generate mesh".to_string()), "{labels:?}");
    // Support rows offer their kind, edge and removal; folders offer the add row.
    let sup = Authoring::tree(&s).into_iter().find(|n| n.label.contains(" on left")).expect("support row").id;
    let labels: Vec<String> = Authoring::tree_menu(&s, sup).into_iter().filter_map(|i| if let crate::gpu_viewer::menu::Item::Entry(e) = i { Some(e.label) } else { None }).collect();
    assert!(labels.iter().any(|l| l.starts_with("Remove")), "{labels:?}");
}

#[test]
fn feature_mapping_covers_every_row_and_removals_target_their_own_entry() {
    let mut p = plate();
    p.bushings.push(model::new_bushing(&p).unwrap());
    for row in model::field_rows(&p) {
        let _ = feature_of(row);
    }
    assert_eq!(removal_of(FieldRow::HoleKind(2)), Some(FieldRow::HoleRemove(2)));
    assert_eq!(removal_of(FieldRow::SupportComp(1, 0)), Some(FieldRow::SupportRemove(1)));
    assert_eq!(removal_of(FieldRow::LoadParam(0, 1)), Some(FieldRow::LoadRemove(0)));
    assert_eq!(removal_of(FieldRow::BushingParam(0, 0)), Some(FieldRow::BushingRemove(0)));
    assert_eq!(removal_of(FieldRow::SupportAdd), None, "add rows are not entries");
    assert_eq!(removal_of(FieldRow::MeshSize), None);
}

fn studied(s: &mut SolidSession, kind: StudyKind) {
    s.run_study(kind);
    let t = Instant::now();
    while s.study_worker.is_busy() {
        s.poll();
        assert!(t.elapsed().as_secs() < 180, "study did not finish: {}", s.status);
        std::thread::sleep(Duration::from_millis(5));
    }
    s.poll();
}

#[test]
fn a_modal_study_runs_in_the_editor_cycles_modes_and_goes_stale_with_the_model() {
    let mut s = session("modal");
    assert!(s.show_study_availability().is_err());
    studied(&mut s, StudyKind::Modal);
    let n = s.current_study().expect("modal result").scenes.len();
    assert!(n >= 2, "{}", s.status);
    assert!(s.show_study && s.study_scene().unwrap().duration() > 0.0, "harmonic modes animate");
    let first = s.study_scene().unwrap().label.clone();
    s.study_step(true);
    assert_ne!(s.study_scene().unwrap().label, first);
    s.study_step(false);
    assert_eq!(s.study_scene().unwrap().label, first);
    // The scene the window draws is the mode, not the static preview.
    assert_eq!(Authoring::scene(&mut s).unwrap().label, first);
    // Editing the model discards the study.
    s.problem.material.density *= 2.0;
    assert!(s.current_study().is_none() && s.study_scene().is_none());
    assert!(s.show_study_availability().is_err());
    // A material without mass density explains why frequencies cannot be computed.
    s.problem.material.density = 0.0;
    assert!(s.study_availability(StudyKind::Modal).unwrap_err().contains("density"));
}

#[test]
fn the_hole_tool_places_a_hole_where_the_front_face_is_clicked() {
    use crate::gpu_viewer::editor::camera::Camera;
    let mut s = session("holetool");
    meshed(&mut s);
    let holes = |s: &SolidSession| match &s.problem.geometry {
        Geometry::Sketch { holes, .. } => holes.clone(),
        _ => unreachable!(),
    };
    let before = holes(&s).len();
    // Front view, centred on the part: the viewport centre is the part's centre.
    let cam = Camera { yaw: 0.0, pitch: 0.0, zoom: 1.0, aspect: 1.0, pan: [0.0, 0.0] };
    let size = [800.0, 800.0];
    // Without the tool a click only selects.
    Authoring::click(&mut s, [400.0, 400.0], size, &cam);
    assert_eq!(holes(&s).len(), before);
    s.set_tool(SolidTool::Hole);
    assert_eq!(s.tool, SolidTool::Hole);
    Authoring::click(&mut s, [400.0, 400.0], size, &cam);
    let h = holes(&s);
    assert_eq!(h.len(), before + 1, "{}", s.status);
    let Geometry::Sketch { outer, .. } = &s.problem.geometry else { unreachable!() };
    let (lo, hi) = outer.bounds();
    let fea_problem::Shape::Circle { cx, cy, .. } = h[before] else { panic!() };
    assert!((cx - 0.5 * (lo[0] + hi[0])).abs() < 0.02 * (hi[0] - lo[0]), "{cx}");
    assert!((cy - 0.5 * (lo[1] + hi[1])).abs() < 0.02 * (hi[1] - lo[1]), "{cy}");
    // It is one undoable edit and focuses the new hole's rows.
    assert!(matches!(s.focused_row().map(feature_of), Some(Feature::Hole(i)) if i == before));
    s.undo();
    assert_eq!(holes(&s).len(), before);
    // Esc leaves the tool; an imported mesh has no holes to add.
    s.handle_key(Key::Escape, Mods::default());
    assert_eq!(s.tool, SolidTool::Select);
    s.problem.geometry = Geometry::Imported { path: "x.msh".into() };
    assert!(s.hole_availability().is_err());
}

#[test]
fn shift_click_and_box_select_gather_surfaces_and_one_action_covers_them_all() {
    let mut s = session("multisel");
    meshed(&mut s);
    s.build_scene().unwrap();
    let cam = Camera { yaw: 0.0, pitch: 0.0, zoom: 1.0, aspect: 1.0, pan: [0.0, 0.0] };
    let size = [800.0, 800.0];
    let mesh = s.preview.as_ref().unwrap().mesh.clone();
    let positions = s.pick_positions.clone();
    let pixel_of = |name: &str| {
        let set = SolidSession::nodes_of(&mesh, name);
        let node = *set.iter().nth(set.len() / 2).unwrap();
        let (ndc, _) = cam.project(positions[node]);
        Camera::pixel(ndc, size[0], size[1])
    };
    // A plain click selects one surface and asks for the applicable list; Shift-click toggles more in.
    assert!(s.click_with(pixel_of("top"), size, &cam, false));
    assert!(s.take_popup_flag());
    s.click_with(pixel_of("bottom"), size, &cam, true);
    assert_eq!(s.all_selected().len(), 2, "{:?}", s.all_selected());
    assert!(!s.take_popup_flag(), "adding to a selection does not reopen the list");
    s.click_with(pixel_of("bottom"), size, &cam, true);
    assert_eq!(s.all_selected(), ["top"], "Shift-click on a selected surface removes it");
    // A box over the whole view selects every named surface; a second action covers all of them.
    s.box_select([0.0, 0.0], [800.0, 800.0], size, &cam, false);
    let all = s.all_selected();
    assert!(all.len() >= 5, "{all:?}");
    let (sup, undo) = (s.problem.supports.len(), s.undo.len());
    s.do_action(SolidAction::RollerOnSelected(1));
    assert_eq!(s.problem.supports.len(), sup + all.len());
    assert_eq!(s.undo.len(), undo + 1, "one undo step for the whole group");
    assert!(s.problem.supports.last().is_some_and(|sup| matches!(sup, Support::Edge { ux: None, uy: Some(_), uz: None, .. })), "the roller holds y only");
    // A box that holds nothing replaces the selection with nothing; Additive keeps what was there.
    s.box_select([0.0, 0.0], [2.0, 2.0], size, &cam, true);
    assert_eq!(s.all_selected().len(), all.len());
    s.box_select([0.0, 0.0], [2.0, 2.0], size, &cam, false);
    assert!(s.all_selected().is_empty());
    // Undo drops the selection with the model change.
    s.undo();
    assert!(s.selected.is_none() && s.multi.is_empty());
}

#[test]
fn solid_faces_are_picked_front_most_at_the_pixel() {
    let mut p = plate();
    p.analysis = fea_problem::Analysis::Solid;
    p.mesh.size = 2.0;
    let mut s = SolidSession::new(p, temp("faces3d"));
    meshed(&mut s);
    s.build_scene().unwrap();
    let mesh = s.preview.as_ref().unwrap().mesh.clone();
    let size = [800.0, 800.0];
    let z_of = |name: &str| {
        let nodes = SolidSession::nodes_of(&mesh, name);
        let zs: Vec<f64> = nodes.iter().map(|n| mesh.nodes[*n][2]).collect();
        (zs.iter().cloned().fold(f64::INFINITY, f64::min), zs.iter().cloned().fold(f64::NEG_INFINITY, f64::max))
    };
    let (zlo, zhi) = mesh.nodes.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), n| (a.min(n[2]), b.max(n[2])));
    let front = Camera { yaw: 0.0, pitch: 0.0, zoom: 1.0, aspect: 1.0, pan: [0.0, 0.0] };
    let back = Camera { yaw: std::f32::consts::PI, pitch: 0.0, zoom: 1.0, aspect: 1.0, pan: [0.0, 0.0] };
    // A node of a face lying wholly in the front plane: the front view must pick a front-plane face there
    // (the back face is behind the same pixel), the mirrored back view a back-plane face.
    let flat_at = |z: f64| mesh.surfaces.keys().find(|n| { let (lo, hi) = z_of(n); (lo - z).abs() < 1e-9 && (hi - z).abs() < 1e-9 }).cloned();
    let (front_face, back_face) = (flat_at(zhi).expect("a face in the front plane"), flat_at(zlo).expect("a face in the back plane"));
    let node = *SolidSession::nodes_of(&mesh, &front_face).iter().nth(7).unwrap();
    let (ndc, _) = front.project(s.pick_positions[node]);
    let px = Camera::pixel(ndc, size[0], size[1]);
    let a = s.pick(&front, px, size).expect("a face is hit from the front");
    let b = s.pick(&back, [size[0] - px[0], px[1]], size).expect("a face is hit from the back");
    let (a_lo, a_hi) = z_of(&a);
    let (b_lo, b_hi) = z_of(&b);
    let at_extreme = |lo: f64, hi: f64| (lo - zlo).abs() < 1e-9 && (hi - zlo).abs() < 1e-9 || (lo - zhi).abs() < 1e-9 && (hi - zhi).abs() < 1e-9;
    assert!(at_extreme(a_lo, a_hi) && at_extreme(b_lo, b_hi), "{a} z {a_lo}..{a_hi}, {b} z {b_lo}..{b_hi}, part {zlo}..{zhi}");
    assert!(a_lo != b_lo, "opposite faces: {a} vs {b} ({front_face}/{back_face})");
}

#[test]
fn adaptive_refinement_is_the_default_and_the_result_panel_reports_what_it_did() {
    assert_eq!(fea_problem::MeshSpec::default().adapt_passes, 2, "new models refine adaptively");
    let mut s = session("amr");
    s.problem.mesh.size = 1.0;
    solved(&mut s);
    let text: Vec<String> = s.properties().into_iter().filter_map(|i| i.plain_text()).collect();
    let line = text.iter().find(|t| t.contains("adaptive")).unwrap_or_else(|| panic!("{text:?}"));
    assert!(line.contains("passes") || line.contains("target"), "{line}");
    // 3D solids are not refined yet and the panel says so instead of implying they were.
    let mut b = session("amr-3d");
    b.do_action(SolidAction::OpenTemplate(5));
    b.problem.mesh.size = 1.0;
    solved(&mut b);
    let text: Vec<String> = b.properties().into_iter().filter_map(|i| i.plain_text()).collect();
    assert!(text.iter().any(|t| t.contains("fixed mesh") || t.contains("no refinement")), "{text:?}");
}

#[test]
fn a_solved_model_adds_a_result_card_for_the_simulation_step_only() {
    use crate::gpu_viewer::editor::authoring::Relevance;
    let mut s = session("resultcard");
    s.problem.mesh.size = 1.0;
    s.do_action(SolidAction::Solve);
    let t = std::time::Instant::now();
    while !s.poll() || s.solving() {
        assert!(t.elapsed().as_secs() < 120);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(s.current_solution().is_some(), "{}", s.status);
    let cards = Authoring::inspector(&s);
    let result = cards.iter().find(|c| c.id == "result").unwrap_or_else(|| panic!("{:?}", cards.iter().map(|c| &c.id).collect::<Vec<_>>()));
    assert_eq!((result.relevance, result.steps.clone()), (Relevance::Info, vec!["Simulation"]));
    assert!(result.items.iter().any(|i| i.plain_text().is_some_and(|t| t.starts_with("max von Mises"))));
}

#[test]
fn a_thermal_study_is_set_up_solved_and_shown_as_a_temperature_field() {
    use fea_problem::{Field, Study};
    let mut s = session("thermal");
    // Switch the study: the supports / loads sections give way to the thermal ones.
    s.do_action(SolidAction::Activate(FieldRow::StudyKind));
    assert_eq!(s.problem.study, Study::Thermal, "Structural -> Thermal");
    let rows = s.rows();
    assert!(rows.contains(&FieldRow::Conductivity) && rows.contains(&FieldRow::HeatSupportAdd) && rows.contains(&FieldRow::HeatLoadAdd));
    assert!(!rows.contains(&FieldRow::SupportAdd) && !rows.contains(&FieldRow::LoadAdd));
    // Without a fixed temperature the solve is refused with the reason.
    assert!(s.solve_availability().unwrap_err().contains("fixed temperature"), "{:?}", s.solve_availability());
    // Quick commands on picked surfaces: Support -> fixed temperature, Load -> heat flux.
    s.do_action(SolidAction::SelectSurface(s.names().iter().position(|n| n == "left").unwrap()));
    s.do_action(SolidAction::SupportOnSelected);
    s.do_action(SolidAction::SelectSurface(s.names().iter().position(|n| n == "right").unwrap()));
    s.do_action(SolidAction::LoadOnSelected);
    assert_eq!(s.problem.thermal.supports.len(), 1);
    assert_eq!(s.problem.thermal.loads.len(), 1);
    assert!(s.problem.supports.len() == plate().supports.len(), "no structural support was added");
    // Edit the fixed temperature and the flux through their rows, then solve.
    let t_row = row_index(&s, FieldRow::HeatSupportTemp(0));
    s.do_action(SolidAction::Focus(t_row));
    type_text(&mut s, "100");
    s.handle_key(Key::Enter, Mods::default());
    assert_eq!(s.problem.thermal.supports[0].temperature(), 100.0);
    s.problem.thermal.loads.clear();
    s.problem.thermal.loads.push(fea_problem::HeatLoad::Convection { edge: "right".into(), h: 5.0, ambient: 20.0 });
    s.problem.thermal.conductivity = 2.0;
    solved(&mut s);
    let sol = s.current_solution().expect("solved").clone();
    assert_eq!(sol.default_field(), Field::Temperature);
    let h = sol.summary.heat.as_ref().unwrap();
    assert!(h.t_max.value <= 100.0 + 1e-6 && h.t_min.value >= 20.0 - 1e-6 && h.t_max.value > h.t_min.value, "{h:?}");
    // The scene the window draws is the temperature field, with no stress fields offered.
    let scene = crate::gpu_viewer::scene::SceneSource::from_solved(&sol, Field::VonMises);
    assert_eq!(scene.field_label, "Temperature");
    assert!(scene.fields.is_empty(), "a thermal-only result offers just the temperature: {:?}", scene.fields.iter().map(|f| &f.0).collect::<Vec<_>>());
    // Back to a structural study: the results no longer match.
    s.do_action(SolidAction::Activate(FieldRow::StudyKind));
    s.do_action(SolidAction::Activate(FieldRow::StudyKind));
    assert_eq!(s.problem.study, Study::Structural);
}

/// Run `poll` until the un-meshed body of the current geometry exists.
fn body_ready(s: &mut SolidSession) {
    let t = Instant::now();
    while !s.body_current() {
        s.poll();
        assert!(t.elapsed().as_secs() < 60, "the body did not finish");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn a_3d_model_shows_its_body_before_it_is_meshed_and_the_mesh_then_replaces_the_stand_in() {
    let mut p = plate();
    p.analysis = fea_problem::Analysis::Solid;
    let mut s = SolidSession::new(p, temp("body"));
    assert!(s.preview.is_none() && !s.body_current());
    body_ready(&mut s);
    let scene = s.build_scene().unwrap();
    assert!(scene.label.contains("body (not meshed)"), "{}", scene.label);
    assert!(scene.triangles.len() > 12 * 3, "a real surface, not the placeholder triangle or the sketch outline tubes");
    assert!(s.preview.is_none(), "the body is not the analysis mesh");
    // The body is a surface to pick on: a click on it selects a named face.
    let cam = Camera { yaw: 0.6, pitch: -0.5, zoom: 1.2, aspect: 1.0, pan: [0.0, 0.0] };
    let size = [800.0, 800.0];
    assert!(s.pick(&cam, [400.0, 400.0], size).is_some(), "the body can be picked before it is meshed");
    // Changing the mesh settings does not rebuild the body; changing the geometry does.
    let key = body_key(&s.problem);
    s.problem.mesh.size = 0.7;
    assert_eq!(key, body_key(&s.problem));
    // Meshing replaces the stand-in with the analysis mesh.
    meshed(&mut s);
    let scene = s.build_scene().unwrap();
    assert!(scene.label.contains("geometry:") && !scene.label.contains("not meshed"), "{}", scene.label);
}

#[test]
fn a_plane_model_never_builds_a_body_and_a_failing_body_is_not_retried_every_frame() {
    let mut s = session("nobody");
    for _ in 0..5 {
        s.poll();
    }
    assert!(s.body.is_none() && !s.body_worker.is_busy());
    let mut p = plate();
    p.analysis = fea_problem::Analysis::Solid;
    let mut s = SolidSession::new(p, temp("body-fail"));
    let failed = body_key(&s.problem);
    s.body_failed = Some(failed);
    for _ in 0..5 {
        s.poll();
    }
    assert!(s.body.is_none() && !s.body_worker.is_busy(), "a geometry whose body failed is left alone");
}

#[test]
fn the_mesh_overlay_is_suppressed_while_only_the_body_is_shown_and_returns_with_the_mesh() {
    let mut p = plate();
    p.analysis = fea_problem::Analysis::Solid;
    let mut s = SolidSession::new(p, temp("body-lines"));
    assert!(!Authoring::shows_body_only(&s), "nothing built yet");
    body_ready(&mut s);
    s.build_scene().unwrap();
    assert!(Authoring::shows_body_only(&s));
    meshed(&mut s);
    s.build_scene().unwrap();
    assert!(!Authoring::shows_body_only(&s), "the analysis mesh is shown: its lines may be drawn");
}

#[test]
fn picking_ignores_a_stale_mesh_after_the_geometry_changes() {
    let mut p = plate();
    p.analysis = fea_problem::Analysis::Solid;
    let mut s = SolidSession::new(p, temp("stale-pick"));
    meshed(&mut s);
    s.build_scene().unwrap();
    s.problem.mesh.size = 0.9;
    if let fea_problem::Geometry::Sketch { depth, .. } = &mut s.problem.geometry {
        *depth += 0.5;
    }
    s.touch();
    s.build_scene().unwrap();
    let cam = Camera { yaw: 0.6, pitch: -0.5, zoom: 1.2, aspect: 1.0, pan: [0.0, 0.0] };
    // Neither the stale analysis mesh nor a body that is not built yet may be picked on (and nothing may panic).
    assert!(!s.preview_current() && !s.body_current());
    assert!(s.pick(&cam, [400.0, 400.0], [800.0, 800.0]).is_none());
}

fn entry_labels(items: Vec<crate::gpu_viewer::menu::Item>) -> Vec<String> {
    items.into_iter().filter_map(|i| if let crate::gpu_viewer::menu::Item::Entry(e) = i { Some(e.label) } else { None }).collect()
}
fn card_fields(s: &SolidSession) -> Vec<(String, String)> {
    s.card_view().unwrap().rows.into_iter().filter_map(|r| if let crate::gpu_viewer::ui::CardRow::Field { label, text, .. } = r { Some((label, text)) } else { None }).collect()
}
fn set_field(s: &mut SolidSession, i: usize, text: &str) {
    s.card_set_field(i, text.into());
}

#[test]
fn the_face_menu_lists_constraints_then_loads_then_what_is_applied() {
    let mut s = session("face-menu");
    s.selected = Some("top".into());
    let labels = entry_labels(s.surface_menu());
    for want in ["Fixed", "Hold directions...", "Slides on the face", "Pressure...", "Force...", "Traction..."] {
        assert!(labels.iter().any(|l| l == want), "{want} in {labels:?}");
    }
    assert!(!labels.iter().any(|l| l.starts_with("Remove") || l.starts_with("Edit the load") || l.starts_with("Change directions")), "nothing applied on 'top' yet: {labels:?}");
    // The same list from a tree row, and a hole adds the pin load.
    assert_eq!(entry_labels(Authoring::tree_menu(&s, ui::T_SURFACE_BASE)).into_iter().filter(|l| l != "Assign to focused row").collect::<Vec<_>>(), labels);
    s.selected = Some("hole1".into());
    assert!(entry_labels(s.surface_menu()).iter().any(|l| l == "Pin load..."));
}

#[test]
fn a_pressure_card_adds_nothing_until_apply_and_a_bad_number_keeps_it_open() {
    let mut s = session("pressure-card");
    s.selected = Some("top".into());
    let loads = s.problem.loads.len();
    s.do_action(SolidAction::OpenCard(CardForm::Pressure));
    assert!(s.prompting());
    assert_eq!(card_fields(&s), [("Pressure".to_string(), "1000".to_string())]);
    assert_eq!(s.problem.loads.len(), loads, "opening the card changes nothing");
    set_field(&mut s, 0, "12x");
    s.do_action(SolidAction::CardApply);
    assert!(s.card_view().is_some() && s.status.contains("not a number"), "{}", s.status);
    assert_eq!(s.problem.loads.len(), loads);
    set_field(&mut s, 0, "2500");
    s.do_action(SolidAction::CardApply);
    assert!(s.card_view().is_none());
    assert!(matches!(s.problem.loads.last(), Some(fea_problem::Load::Pressure { edge, p }) if edge == "top" && *p == 2500.0), "{:?}", s.problem.loads.last());
    // One undo step removes it again.
    s.undo();
    assert_eq!(s.problem.loads.len(), loads);
    // Undo drops the selection, so pick the face again. Cancel leaves the model alone; Esc is the same.
    s.selected = Some("top".into());
    s.do_action(SolidAction::OpenCard(CardForm::Pressure));
    s.handle_key(Key::Escape, Mods::default());
    assert!(s.card_view().is_none() && s.problem.loads.len() == loads);
    // Enter applies the open card.
    s.do_action(SolidAction::OpenCard(CardForm::Pressure));
    s.handle_key(Key::Enter, Mods::default());
    assert_eq!(s.problem.loads.len(), loads + 1);
}

#[test]
fn force_traction_and_pin_cards_write_their_loads_and_the_menu_then_edits_them() {
    use fea_problem::Load;
    let mut s = session("load-cards");
    s.selected = Some("right".into());
    s.do_action(SolidAction::OpenCard(CardForm::Force));
    assert_eq!(card_fields(&s).len(), 2, "a plane model has two components");
    set_field(&mut s, 0, "300");
    set_field(&mut s, 1, "-50");
    s.do_action(SolidAction::CardApply);
    assert!(matches!(s.problem.loads.last(), Some(Load::Force { edge, fx, fy, .. }) if edge == "right" && *fx == 300.0 && *fy == -50.0), "{:?}", s.problem.loads.last());
    s.do_action(SolidAction::OpenCard(CardForm::Traction));
    set_field(&mut s, 0, "42");
    s.do_action(SolidAction::CardApply);
    assert!(matches!(s.problem.loads.last(), Some(Load::Traction { tx, .. }) if *tx == 42.0));
    s.selected = Some("hole1".into());
    s.do_action(SolidAction::OpenCard(CardForm::Bearing));
    set_field(&mut s, 0, "800");
    set_field(&mut s, 1, "200");
    s.do_action(SolidAction::CardApply);
    assert!(matches!(s.problem.loads.last(), Some(Load::Bearing { hole: 1, fx, fy }) if *fx == 800.0 && *fy == 200.0), "{:?}", s.problem.loads.last());
    // A pin load is for a hole.
    s.selected = Some("right".into());
    let n = s.problem.loads.len();
    s.do_action(SolidAction::OpenCard(CardForm::Bearing));
    assert!(s.card_view().is_none() && s.status.contains("pick a hole"), "{}", s.status);
    assert_eq!(s.problem.loads.len(), n);
    // The menu of 'right' offers to edit and remove what is on it; editing writes the new numbers in place.
    let labels = entry_labels(s.surface_menu());
    assert!(labels.iter().any(|l| l == "Edit the load on 'right'...") && labels.iter().any(|l| l == "Remove the load on 'right'"), "{labels:?}");
    let i = s.problem.loads.iter().position(|l| matches!(l, Load::Force { .. })).unwrap();
    s.do_action(SolidAction::OpenCard(CardForm::EditLoad(i)));
    assert_eq!(card_fields(&s)[0], ("Force x".to_string(), "300".to_string()));
    set_field(&mut s, 0, "310");
    s.do_action(SolidAction::CardApply);
    assert!(matches!(&s.problem.loads[i], Load::Force { fx, fy, .. } if *fx == 310.0 && *fy == -50.0));
}

#[test]
fn the_directions_card_holds_chosen_directions_with_a_displacement_and_can_be_changed() {
    let mut s = session("hold-card");
    s.selected = Some("bottom".into());
    s.do_action(SolidAction::OpenCard(CardForm::Hold));
    let rows = s.card_view().unwrap().rows;
    assert!(rows.iter().any(|r| matches!(r, crate::gpu_viewer::ui::CardRow::Toggle { label, on: true, .. } if label == "Hold y")));
    s.do_action(SolidAction::CardToggle(0));
    s.do_action(SolidAction::CardApply);
    let sup = s.problem.supports.last().unwrap().clone();
    assert_eq!((sup.edge(), sup.comps()), (Some("bottom"), [None, Some(0.0), None]), "only y is held");
    assert!(s.card_view().is_none());
    // Holding nothing is refused.
    s.do_action(SolidAction::OpenCard(CardForm::Hold));
    s.do_action(SolidAction::CardToggle(0));
    s.do_action(SolidAction::CardToggle(1));
    s.do_action(SolidAction::CardApply);
    assert!(s.card_view().is_some() && s.status.contains("at least one"));
    s.do_action(SolidAction::CardCancel);
    // Change directions: x held and moved by 0.5, y released.
    let i = s.problem.supports.len() - 1;
    s.do_action(SolidAction::OpenCard(CardForm::EditHold(i)));
    s.do_action(SolidAction::CardToggle(0));
    s.do_action(SolidAction::CardToggle(1));
    set_field(&mut s, 0, "0.5");
    s.do_action(SolidAction::CardApply);
    assert_eq!(s.problem.supports[i].comps(), [Some(0.5), None, None]);
}

#[test]
fn gravity_is_one_body_force_that_the_card_sets_changes_and_removes() {
    use fea_problem::Load;
    let mut s = session("gravity-card");
    let body = |s: &SolidSession| s.problem.loads.iter().filter(|l| matches!(l, Load::Body { .. })).count();
    assert_eq!(body(&s), 0);
    s.do_action(SolidAction::OpenCard(CardForm::Gravity));
    assert!(card_fields(&s)[1].1.starts_with('-'), "the suggestion pulls down: {:?}", card_fields(&s));
    s.do_action(SolidAction::CardApply);
    assert_eq!(body(&s), 1);
    s.do_action(SolidAction::OpenCard(CardForm::Gravity));
    set_field(&mut s, 1, "-0.25");
    s.do_action(SolidAction::CardApply);
    assert_eq!(body(&s), 1, "the card edits the one body force");
    assert!(s.problem.loads.iter().any(|l| matches!(l, Load::Body { by, .. } if *by == -0.25)));
    s.do_action(SolidAction::OpenCard(CardForm::Gravity));
    set_field(&mut s, 0, "0");
    set_field(&mut s, 1, "0");
    s.do_action(SolidAction::CardApply);
    assert_eq!(body(&s), 0, "zero removes it");
}

#[test]
fn load_cases_are_made_edited_activated_and_decide_what_a_solve_uses() {
    let mut s = session("cases");
    s.problem.loads.push(fea_problem::Load::Pressure { edge: "top".into(), p: 500.0 });
    assert!(entry_labels(s.simulation_items_for_test()).iter().any(|l| l == "New load case"));
    assert!(s.problem.cases.is_empty());
    s.do_action(SolidAction::NewCase);
    assert_eq!(s.problem.cases.len(), 2, "the first case keeps what the problem solved before");
    let card = s.card_view().unwrap();
    assert_eq!(card.title, "Load case 2");
    // Name it and switch the traction off in it.
    set_field(&mut s, 0, "Pressure only");
    set_field(&mut s, 1, "0");
    s.do_action(SolidAction::CardApply);
    assert_eq!(s.problem.cases[1].name, "Pressure only");
    assert_eq!(s.problem.resolved().loads.len(), 1);
    // The tree lists the cases; clicking one makes it active, and the result goes stale because it is another case.
    let labels: Vec<String> = Authoring::tree(&s).into_iter().map(|n| n.label).collect();
    assert!(labels.iter().any(|l| l == "Load cases (2)") && labels.iter().any(|l| l == "Pressure only (active)"), "{labels:?}");
    meshed(&mut s);
    solved(&mut s);
    assert!(s.current_solution().is_some());
    let applied = s.current_solution().unwrap().summary.applied;
    assert!(applied[0].abs() < 1e-6 && applied[1].abs() > 1.0, "only the pressure acted: {applied:?}");
    s.do_action(SolidAction::SetCase(0));
    assert!(s.current_solution().is_none(), "another case has no result yet");
    solved(&mut s);
    assert!(s.current_solution().unwrap().summary.applied[0].abs() > 1.0, "case 1 has the traction");
    // The tree row's menu edits and removes; removing down to one case is the plain model again.
    let node = ui::T_CASE_BASE + 1;
    assert!(entry_labels(Authoring::tree_menu(&s, node)).iter().any(|l| l == "Remove this load case"));
    s.do_action(SolidAction::RemoveCase(1));
    assert!(s.problem.cases.is_empty());
}

#[test]
fn supports_and_loads_are_drawn_on_the_model_as_glyphs_that_follow_the_camera_and_the_case() {
    use crate::gpu_viewer::editor::authoring::OverlayKind;
    let mut s = session("glyphs");
    s.problem.loads.push(fea_problem::Load::Pressure { edge: "top".into(), p: 500.0 });
    meshed(&mut s);
    s.build_scene().unwrap();
    let cam = Camera { yaw: 0.0, pitch: 0.0, zoom: 1.0, aspect: 1.0, pan: [0.0, 0.0] };
    let size = [800.0, 800.0];
    let glyphs = Authoring::overlay(&s, &cam, size);
    let loads: Vec<_> = glyphs.iter().filter(|g| g.kind == OverlayKind::Load).collect();
    let supports: Vec<_> = glyphs.iter().filter(|g| g.kind == OverlayKind::Support).collect();
    // The roller on the left face and the point support at its centre land on one spot: one glyph names both directions.
    assert_eq!((loads.len(), supports.len()), (2, 1), "a traction, a pressure and the two supports merged: {glyphs:?}");
    // The traction pulls the part to the right (+x leaves the right face); the pressure on the top pushes down into it.
    let traction = loads.iter().find(|g| g.text == "10000").expect("traction label");
    assert!(traction.tip.unwrap()[0] > traction.px[0] + 20.0, "{traction:?}");
    let pressure = loads.iter().find(|g| g.text == "500").expect("pressure label");
    assert!(pressure.tip.unwrap()[1] > pressure.px[1] + 20.0, "pushes down: tail above tip on the screen: {pressure:?}");
    // The roller on the left holds x only; the support glyph says so and points out of the face (to the left).
    let roller = supports.iter().find(|g| g.text == "xy").expect("the merged support holding x and y");
    assert!(roller.tip.unwrap()[0] < -0.5, "{roller:?}");
    // Turning the camera moves the glyphs; a load case that switches the pressure off removes its arrow.
    let turned = Authoring::overlay(&s, &Camera { yaw: 0.8, ..cam }, size);
    assert_ne!(turned.iter().map(|g| g.px[0] as i32).collect::<Vec<_>>(), glyphs.iter().map(|g| g.px[0] as i32).collect::<Vec<_>>());
    s.problem.add_case("Pull only");
    let last = s.problem.cases.len() - 1;
    s.problem.cases[last].set_factor(1, 0.0);
    let case_glyphs = Authoring::overlay(&s, &cam, size);
    assert_eq!(case_glyphs.iter().filter(|g| g.kind == OverlayKind::Load).count(), 1);
}

#[test]
fn the_material_library_searches_filters_and_applies_with_its_density() {
    use crate::gpu_viewer::ui::CardRow;
    let mut s = session("materials");
    let before = s.problem.material.clone();
    s.do_action(SolidAction::OpenCard(CardForm::Materials));
    let card = s.card_view().unwrap();
    assert_eq!(card.title, "Material library");
    let choices = |s: &SolidSession| -> Vec<(String, SolidAction)> {
        s.card_view().unwrap().rows.into_iter().filter_map(|r| match r {
            CardRow::Choice { label, action: UiAction::Editor(EditorAction::Solid(a)), .. } if !label.starts_with("Group:") && !label.starts_with("Sort by:") => Some((label, a)),
            _ => None,
        }).collect()
    };
    let all = choices(&s);
    assert!(all.len() == 12, "twelve rows are shown of the whole catalog: {}", all.len());
    // Searching narrows the list live; every hit contains every word.
    set_field(&mut s, 0, "7075 t6");
    let hits = choices(&s);
    assert!(!hits.is_empty() && hits.len() < all.len() + 1 && hits.iter().all(|(l, _)| l.contains("7075")), "{hits:?}");
    // The group filter and the sort key cycle.
    let titles = |s: &SolidSession| s.card_view().unwrap().rows.into_iter().filter_map(|r| if let CardRow::Choice { label, .. } = r { Some(label) } else { None }).take(2).collect::<Vec<_>>();
    let g0 = titles(&s);
    s.do_action(SolidAction::CardMaterialGroup);
    s.do_action(SolidAction::CardMaterialSort);
    assert_ne!(titles(&s), g0, "group and sort changed the rows");
    // Clearing the group, pick a curated aluminium: E, nu, expansion, yield and the DENSITY follow.
    for _ in 0..crate::toolboxes::material_lookup::model::groups().len() {
        s.do_action(SolidAction::CardMaterialGroup);
        if titles(&s)[0].starts_with("Group: All") {
            break;
        }
    }
    set_field(&mut s, 0, "7075 typical");
    let (_, apply) = choices(&s).into_iter().find(|(l, _)| l == "Al 7075-T6 (typical)").expect("the curated material");
    s.do_action(apply);
    assert!(s.card_view().is_none());
    let m = &s.problem.material;
    assert_eq!(m.name, "Al 7075-T6 (typical)");
    assert!((m.e - 10.3e6).abs() < 1.0 && (m.nu - 0.33).abs() < 1e-12 && m.yield_stress == Some(70_000.0));
    assert!((m.density * 386.089 - 0.101).abs() < 1e-9, "weight density 0.101 lb/in3, not the old {}: {}", before.density * 386.089, m.density * 386.089);
    // One undo returns the old material; Enter in the search takes the first match.
    s.undo();
    assert_eq!(s.problem.material, before);
    s.do_action(SolidAction::OpenCard(CardForm::Materials));
    set_field(&mut s, 0, "Ti-6Al-4V");
    s.handle_key(Key::Enter, Mods::default());
    assert!(s.problem.material.name.contains("Ti-6Al-4V"), "{}", s.problem.material.name);
    // A handbook material carries its own density.
    s.do_action(SolidAction::OpenCard(CardForm::Materials));
    set_field(&mut s, 0, "2024");
    let first = choices(&s)[0].1;
    s.do_action(first);
    assert!(s.problem.material.density > 0.0);
}

#[test]
fn the_check_card_warns_before_the_solve_and_a_certain_failure_is_not_started() {
    use fea_problem::{diagnose::FailureClass, Support};
    // Modulus in ksi beside loads in psi: forecast before any solve, one click repairs it.
    let mut s = session("precheck-scale");
    s.problem.material.e = 29_000.0;
    let found = s.findings();
    let scale = found.iter().find(|d| d.class == FailureClass::ScaleSuspect).expect("a forecast");
    assert!(scale.summary.starts_with("Before solving"), "{}", scale.summary);
    let card = Authoring::inspector(&s).into_iter().find(|c| c.id == "guidance").expect("the check card");
    assert_eq!(card.title, "Check before solving");
    assert_eq!(card.relevance, crate::gpu_viewer::editor::authoring::Relevance::Primary);
    s.do_action(SolidAction::Answer(found.iter().position(|d| d.class == FailureClass::ScaleSuspect).unwrap(), 0, 0));
    assert!((s.problem.material.e - 29.0e6).abs() < 1.0);
    assert!(!s.findings().iter().any(|d| d.class == FailureClass::ScaleSuspect));

    // A roller alone: once the mesh exists the check sees the free motion and Solve says why it will not run.
    let mut s = session("precheck-rigid");
    s.problem.supports = vec![Support::roller("left", 0)];
    assert!(s.solve_availability().is_ok(), "no mesh yet: nothing to check against");
    meshed(&mut s);
    let err = s.solve_availability().unwrap_err();
    assert!(err.starts_with("Before solving") && err.contains("Check card"), "{err}");
    s.do_action(SolidAction::Solve);
    assert!(!s.solving() && s.status.starts_with("Before solving"), "{}", s.status);
    // Supporting the part properly lifts the block.
    s.problem.supports.push(Support::fixed("right"));
    s.touch();
    assert!(s.solve_availability().is_ok(), "{:?}", s.solve_availability());
}

#[test]
fn the_simulation_tab_offers_every_kind_of_analysis_in_one_click() {
    use crate::gpu_viewer::bench::Bench;
    let mut s = session("analysis");
    let tab = Authoring::ribbon(&s).into_iter().find(|t| t.title == "Simulation").unwrap();
    let group = tab.groups.iter().find(|g| g.title == "Analysis").expect("an Analysis group");
    let labels: Vec<(&str, bool)> = group.buttons.iter().map(|b| (b.label.as_str(), b.checked == Some(true))).collect();
    for want in ["Static stress", "Modal / vibration", "Buckling", "Transient", "Structural", "Thermal (steady)", "Thermal stress"] {
        assert!(labels.iter().any(|(l, _)| *l == want), "{want} in {labels:?}");
    }
    assert!(labels.contains(&("Static stress", true)) && labels.contains(&("Structural", true)));
    // The benches are plain actions the window handles; the physics is a model edit.
    let buckling = group.buttons.iter().find(|b| b.label == "Buckling").unwrap();
    assert_eq!(buckling.action, Bench::Buckling.action());
    s.do_action(SolidAction::SetStudy(fea_problem::Study::Thermal));
    assert_eq!(s.problem.study, fea_problem::Study::Thermal);
    s.undo();
    assert_eq!(s.problem.study, fea_problem::Study::Structural);
    // The menu lists them under a heading, ahead of the run commands.
    let menu = Authoring::menus(&s).into_iter().find(|m| m.title == "Simulation").unwrap();
    assert!(matches!(menu.items.first(), Some(crate::gpu_viewer::menu::Item::Heading(h)) if h == "Analysis"));
}

/// Run `poll` until the current study is of `kind`.
fn study_done(s: &mut SolidSession, kind: StudyKind) {
    let t = Instant::now();
    while s.current_study().map(|c| c.kind) != Some(kind) {
        s.poll();
        assert!(!s.status.starts_with("Study failed"), "{}", s.status);
        assert!(t.elapsed().as_secs() < 120, "the study did not finish: {}", s.status);
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn the_dynamic_studies_run_from_the_window_and_their_scenes_move_by_themselves() {
    let mut s = session("studies");
    // Natural frequencies: harmonic mode shapes (the shader swings them), with a frequency in each label.
    s.do_action(SolidAction::RunStudy(StudyKind::Modal));
    study_done(&mut s, StudyKind::Modal);
    let study = s.current_study().expect("modal result");
    assert!(study.scenes.len() >= 2, "several modes");
    assert!(study.scenes.iter().all(|sc| sc.harmonic_period.is_some_and(|p| p > 0.0)), "every mode shape moves on its own");
    assert!(study.scenes[0].label.to_lowercase().contains("hz") || study.scenes[0].label.to_lowercase().contains("mode"), "{}", study.scenes[0].label);
    // The load-release transient is an animation with a duration.
    s.do_action(SolidAction::RunStudy(StudyKind::Transient));
    study_done(&mut s, StudyKind::Transient);
    let transient = s.current_study().expect("transient result");
    assert!(transient.scenes[0].duration() > 0.0 && transient.scenes[0].times.len() > 2, "frames over time");
}

#[test]
fn a_result_is_exported_as_report_paraview_and_csv_next_to_the_model() {
    let mut s = session("export");
    // Nothing to export before a solve, and the File menu says so.
    s.do_action(SolidAction::Export(ExportKind::Csv));
    assert!(s.status.contains("Solve the model first"), "{}", s.status);
    let file = Authoring::menus(&s).into_iter().find(|m| m.title == "File").unwrap();
    let csv = file.items.iter().find_map(|i| if let crate::gpu_viewer::menu::Item::Entry(e) = i { (e.label == ExportKind::Csv.label()).then_some(e.clone()) } else { None }).expect("an export entry");
    assert!(!csv.enabled);
    meshed(&mut s);
    solved(&mut s);
    for kind in ExportKind::ALL {
        s.do_action(SolidAction::Export(kind));
        assert!(s.status.starts_with("Wrote "), "{}", s.status);
    }
    let dir = s.path.parent().unwrap().to_path_buf();
    let read = |ext: &str| std::fs::read_to_string(dir.join(format!("model-results.{ext}"))).unwrap();
    assert!(read("vtu").contains("<VTKFile"), "a ParaView file");
    let csv = read("csv");
    assert!(csv.starts_with("node,x,y") && csv.lines().count() > 100, "one row per node");
    assert!(!read("txt").is_empty());
}

#[test]
fn a_mode_shape_offers_the_same_fields_as_a_static_result_and_each_has_real_values() {
    let mut s = session("mode-fields");
    s.do_action(SolidAction::RunStudy(StudyKind::Modal));
    study_done(&mut s, StudyKind::Modal);
    let scene = &s.current_study().unwrap().scenes[0];
    let names: Vec<&str> = scene.fields.iter().map(|f| f.label.as_str()).collect();
    for want in ["von Mises stress (relative)", "Max principal stress (relative)", "Equivalent strain (relative)", "Max principal strain (relative)", "Min principal strain (relative)", "Stress xx (relative)", "Displacement x (relative)"] {
        assert!(names.contains(&want), "{want} in {names:?}");
    }
    let vm = scene.fields.iter().find(|f| f.label.starts_with("von Mises")).unwrap();
    assert!(vm.range[1] > vm.range[0] && vm.values.iter().all(|v| v.is_finite() && *v >= 0.0), "{:?}", vm.range);
    // Hooke's law ties the strain field to the stress field: equivalent strain = 2 (1 + nu) vM / (3 E).
    let eq = scene.fields.iter().find(|f| f.label.starts_with("Equivalent strain")).unwrap();
    let (e, nu) = (s.problem.material.e as f32, s.problem.material.nu as f32);
    let k = 2.0 * (1.0 + nu) / (3.0 * e);
    for (a, b) in vm.values.iter().zip(&eq.values) {
        assert!((a * k - b).abs() <= 1e-3 * b.abs() + 1e-12, "{a} {b}");
    }
    // The viewer cycles through them like a static result's.
    let mut v = crate::gpu_viewer::controller::ViewState::new(scene);
    for _ in 0..3 {
        v.apply(crate::gpu_viewer::controller::Command::CycleField { forward: true }, scene);
    }
    assert_eq!(v.field, 3);
    assert_eq!(v.field_name(scene), scene.fields[2].label);
}

#[test]
fn the_unit_system_is_converted_or_declared_from_a_card_and_every_card_and_the_library_follow_it() {
    use crate::gpu_viewer::ui::CardRow;
    use fea_problem::UnitSystem;
    let mut s = session("units");
    assert_eq!(s.problem.units, UnitSystem::InchPsi);
    assert!(Authoring::tree(&s).iter().any(|n| n.label == "Units: inch, lbf, psi"));
    s.do_action(SolidAction::OpenCard(CardForm::Units));
    let labels: Vec<String> = s.card_view().unwrap().rows.into_iter().filter_map(|r| if let CardRow::Choice { label, .. } = r { Some(label) } else { None }).collect();
    for want in ["Convert to mm, N, MPa", "Convert to m, N, Pa", "Declare as mm, N, MPa", "Declare as m, N, Pa"] {
        assert!(labels.iter().any(|l| l == want), "{want} in {labels:?}");
    }
    let e_before = s.problem.material.e;
    s.do_action(SolidAction::ConvertUnits(UnitSystem::MmMpa));
    assert!(s.card_view().is_none());
    assert_eq!(s.problem.units, UnitSystem::MmMpa);
    assert!((s.problem.material.e / e_before - 6.894_757e-3).abs() < 1e-6, "29e6 psi is about 2e5 MPa: {}", s.problem.material.e);
    assert!(s.status.contains("rescaled"), "{}", s.status);
    assert!(Authoring::tree(&s).iter().any(|n| n.label == "Units: mm, N, MPa"));
    // The cards speak the new units, and so does the legend.
    s.selected = Some("top".into());
    s.do_action(SolidAction::OpenCard(CardForm::Pressure));
    let unit = s.card_view().unwrap().rows.into_iter().find_map(|r| if let CardRow::Field { unit, .. } = r { Some(unit) } else { None }).unwrap();
    assert_eq!(unit, "MPa");
    s.do_action(SolidAction::CardCancel);
    assert_eq!(Authoring::field_unit(&s, "von Mises stress"), "MPa");
    assert_eq!(Authoring::field_unit(&s, "Displacement magnitude"), "mm");
    assert_eq!(Authoring::field_unit(&s, "Equivalent strain"), "strain");
    // A library material arrives in the model's units: 7075-T6 is 10 300 ksi = 71 016 MPa, with its mass density in tonne/mm3 units.
    s.do_action(SolidAction::OpenCard(CardForm::Materials));
    set_field(&mut s, 0, "7075 typical");
    let first = s.card_view().unwrap().rows.into_iter().find_map(|r| if let CardRow::Choice { label, action: UiAction::Editor(EditorAction::Solid(a)), .. } = r { (label == "Al 7075-T6 (typical)").then_some(a) } else { None }).unwrap();
    s.do_action(first);
    assert!((s.problem.material.e - 71_016.0).abs() < 5.0, "{}", s.problem.material.e);
    let rho_kg_m3 = s.problem.material.density * UnitSystem::MmMpa.density_kg_m3();
    assert!((rho_kg_m3 - 2796.0).abs() < 5.0, "{rho_kg_m3}");
    // Declaring rescales nothing.
    let e = s.problem.material.e;
    s.do_action(SolidAction::DeclareUnits(UnitSystem::MeterPa));
    assert_eq!((s.problem.units, s.problem.material.e), (UnitSystem::MeterPa, e));
    // A geometry that cannot be rescaled refuses a conversion and says why.
    let mut imported = SolidSession::new(fea_problem::templates::imported("x.msh"), temp("units-imported"));
    imported.do_action(SolidAction::ConvertUnits(UnitSystem::MmMpa));
    assert!(imported.status.contains("declare its units instead"), "{}", imported.status);
    assert_eq!(imported.problem.units, UnitSystem::InchPsi);
}

#[test]
fn a_support_or_load_changes_kind_from_its_menu_and_a_point_force_becomes_a_pressure_on_the_picked_face() {
    use fea_problem::{Load, Support};
    let mut s = session("convert");
    s.problem.loads = vec![Load::Traction { edge: "right".into(), tx: 10_000.0, ty: 0.0, tz: 0.0 }, Load::Point { x: 6.0, y: 4.0, z: 0.0, fx: 0.0, fy: -300.0, fz: 0.0 }];
    s.problem.supports = vec![Support::roller("left", 0), Support::Point { x: 0.0, y: 2.0, z: 0.0, ux: None, uy: Some(0.0), uz: None }];
    // The tree row of a load lists what it can become (not what it is), and an edit entry.
    let load_node = ui::T_FEATURE_BASE + 2_000;
    let menu = entry_labels(Authoring::tree_menu(&s, load_node));
    assert!(menu.iter().any(|l| l == "Edit values..."), "{menu:?}");
    for want in ["Pressure", "Force", "Point force"] {
        assert!(menu.iter().any(|l| l == want), "{want} in {menu:?}");
    }
    assert!(!menu.iter().any(|l| l == "Traction"), "its own kind is not offered");
    // The face menu of 'right' offers it too.
    s.selected = Some("right".into());
    assert!(entry_labels(s.surface_menu()).iter().any(|l| l == "Force"));
    s.do_action(SolidAction::ConvertLoad(0, fea_problem::conversion::LoadKind::Force));
    assert!(matches!(&s.problem.loads[0], Load::Force { fx, .. } if (fx - 10_000.0).abs() < 1e-6), "{:?}", s.problem.loads[0]);
    assert!(s.status.starts_with("Converted to a force"), "{}", s.status);
    // One undo returns the traction.
    s.undo();
    assert!(matches!(&s.problem.loads[0], Load::Traction { .. }));
    // A point force (no surface of its own) becomes a pressure on the picked face; the pull along it is not a pressure.
    s.selected = Some("top".into());
    s.do_action(SolidAction::ConvertLoad(1, fea_problem::conversion::LoadKind::Pressure));
    assert!(matches!(&s.problem.loads[1], Load::Pressure { edge, p } if edge == "top" && (p - 100.0).abs() < 1e-6), "300 down into a top face of area 3: pressure 100: {:?}", s.problem.loads[1]);
    // Supports: the point support becomes a fixed face; the roller becomes a point.
    s.selected = Some("bottom".into());
    let node = ui::T_FEATURE_BASE + 1_000 + 1;
    let menu = entry_labels(Authoring::tree_menu(&s, node));
    assert!(menu.iter().any(|l| l == "Change directions..."), "{menu:?}");
    assert!(menu.iter().any(|l| l == "Fixed") && menu.iter().any(|l| l == "Slides on the face"), "{menu:?}");
    s.do_action(SolidAction::ConvertSupport(1, fea_problem::conversion::SupportKind::Fixed));
    assert!(matches!(&s.problem.supports[1], Support::Edge { edge, uy: Some(_), ux: Some(_), .. } if edge == "bottom"), "{:?}", s.problem.supports[1]);
    s.do_action(SolidAction::ConvertSupport(0, fea_problem::conversion::SupportKind::Point));
    assert!(matches!(&s.problem.supports[0], Support::Point { x, .. } if x.abs() < 1e-9), "{:?}", s.problem.supports[0]);
    // A refused conversion changes nothing and says why.
    let before = s.problem.clone();
    s.do_action(SolidAction::ConvertLoad(0, fea_problem::conversion::LoadKind::Bearing));
    assert_eq!(s.problem, before);
    assert!(s.status.contains("hole"), "{}", s.status);
}

#[test]
fn an_imported_mesh_is_forecast_from_its_mesh_before_a_solve_and_the_units_slip_is_named() {
    use fea_problem::diagnose::FailureClass;
    let inp = "*NODE\n1,0,0\n2,1,0\n3,2,0\n4,0,1\n5,1,1\n6,2,1\n*ELEMENT, TYPE=CPS4\n1,1,2,5,4\n2,2,3,6,5\n*NSET, NSET=LEFT\n1,4\n*NSET, NSET=RIGHT\n3,6\n";
    let mut p = fea_problem::templates::templates().into_iter().find(|(n, _)| *n == "Cantilever beam").unwrap().1;
    p.geometry = fea_problem::Geometry::Imported { path: "bar.inp".into() };
    p.thickness = 1.0;
    p.supports = vec![Support::fixed("LEFT")];
    p.loads = vec![fea_problem::Load::Force { edge: "RIGHT".into(), fx: 100.0, fy: 0.0, fz: 0.0 }];
    p.material.e = 290.0;
    let mut s = SolidSession::new(p, temp("imported-forecast"));
    s.import = Some(("bar.inp".into(), inp.into()));
    assert!(!s.findings().iter().any(|d| d.summary.contains("would move this part")), "no mesh yet: nothing to read the size from");
    meshed(&mut s);
    let found = s.findings();
    let scale = found.iter().find(|d| d.class == FailureClass::ScaleSuspect && d.summary.contains("would move this part")).expect("a forecast from the imported mesh");
    assert!(scale.questions[0].options.len() >= 2);
    // A sound modulus raises nothing.
    s.problem.material.e = 29.0e6;
    s.touch();
    assert!(!s.findings().iter().any(|d| d.class == FailureClass::ScaleSuspect));
}
