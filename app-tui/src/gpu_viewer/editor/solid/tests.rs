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
    let cam = Camera { yaw: 0.0, pitch: 0.0, zoom: 1.0, aspect: 1.0 };
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
    assert_eq!(scene.fields.len(), 10, "11 fields: the primary plus ten selectable ones");
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
    let cam = Camera { yaw: 0.0, pitch: 0.0, zoom: 1.0, aspect: 1.0 };
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
    assert!(labels.contains(&"Support on top".to_string()) && labels.contains(&"Solve".to_string()), "{labels:?}");
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
    let cam = Camera { yaw: 0.0, pitch: 0.0, zoom: 1.0, aspect: 1.0 };
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
    let cam = Camera { yaw: 0.0, pitch: 0.0, zoom: 1.0, aspect: 1.0 };
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
    let front = Camera { yaw: 0.0, pitch: 0.0, zoom: 1.0, aspect: 1.0 };
    let back = Camera { yaw: std::f32::consts::PI, pitch: 0.0, zoom: 1.0, aspect: 1.0 };
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
