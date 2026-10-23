//! Whole-model commands: New, templates, Open, Clear, Delete feature. Nothing here may keep the old model's data.
use super::sketching_tests::{act, click, drawn_rectangle, temp};
use super::*;
use crate::gpu_viewer::editor::{authoring::Authoring, sketcher::Tool};

fn template_session(name: &str) -> SolidSession {
    SolidSession::new(templates().remove(0).1, temp(name))
}
fn tree_labels(s: &SolidSession) -> Vec<String> {
    s.tree().into_iter().map(|n| n.label).collect()
}

#[test]
fn new_part_replaces_the_template_with_an_empty_sketch_and_shows_it_in_the_tree_at_once() {
    let mut s = template_session("new-part");
    assert!(!s.problem.supports.is_empty() && s.problem.history.is_none());
    s.do_action(SolidAction::NewPart);
    assert!(s.sketch_mode(), "{}", s.status);
    let p = &s.problem;
    assert!(p.supports.is_empty() && p.loads.is_empty() && p.bushings.is_empty(), "nothing of the template survives");
    assert_eq!(p.history.as_ref().map(|h| h.features.len()), Some(1), "an empty Sketch1, not a converted template");
    let tree = tree_labels(&s);
    assert!(tree.iter().any(|l| l == "History (1)") && tree.iter().any(|l| l == "Sketch1 [editing]"), "{tree:?}");
    // It cannot be meshed or solved until the sketch is a closed profile, and says why.
    s.do_action(SolidAction::Sketch(SketchAction::Finish));
    assert!(s.sketch_mode(), "an empty sketch cannot be finished: {}", s.status);
    // One Undo brings the template back.
    s.do_action(SolidAction::Sketch(SketchAction::Cancel));
    s.do_action(SolidAction::Undo);
    assert!(!s.problem.supports.is_empty() && s.problem.history.is_none(), "undo restores the previous model");
}

#[test]
fn a_new_part_is_drawn_finished_meshed_and_solved_without_any_template_data() {
    let mut s = template_session("new-part-flow");
    s.do_action(SolidAction::NewPart);
    s.do_action(SolidAction::Sketch(SketchAction::Cancel));
    assert!(!s.sketch_mode());
    // Not meshable or solvable yet: the sketch is empty.
    assert!(s.mesh_availability().unwrap_err().contains("no valid body") || s.mesh_availability().is_err());
    assert!(s.solve_availability().is_err());
    // The history exists, so a new sketch is allowed without converting anything.
    act_new_sketch_and_rectangle(&mut s);
    assert!(s.mesh_availability().is_ok(), "{:?}", s.mesh_availability());
    assert!(s.problem.supports.is_empty(), "boundary conditions are the user's to add");
}

fn act_new_sketch_and_rectangle(s: &mut SolidSession) {
    // The empty Sketch1 of the new part is reopened and drawn as a 6 x 2 rectangle.
    let id = s.problem.history.as_ref().unwrap().features[0].id;
    act(s, SketchAction::Edit(id));
    act(s, SketchAction::Tool(Tool::Rectangle));
    click(s, [0.0, 0.0]);
    click(s, [6.0, 2.0]);
    act(s, SketchAction::Finish);
    assert!(!s.sketch_mode(), "{}", s.status);
}

#[test]
fn a_sketch_on_a_model_without_history_is_refused_not_silently_converted() {
    let mut s = template_session("no-convert");
    let before = s.problem.clone();
    act(&mut s, SketchAction::NewSketch(PlaneChoice::Front));
    assert!(!s.sketch_mode() && s.problem == before, "{}", s.status);
    assert!(s.status.contains("File > New part"), "{}", s.status);
}

#[test]
fn every_template_loads_by_name_and_replaces_everything() {
    let mut s = drawn_rectangle("templates");
    act(&mut s, SketchAction::Finish);
    for (i, (title, original)) in templates().into_iter().enumerate() {
        s.do_action(SolidAction::OpenTemplate(i));
        assert_eq!(s.problem, original, "template {title}");
        assert!(s.status.contains(title));
    }
    s.do_action(SolidAction::OpenTemplate(999));
    assert!(s.status.contains("No such template"));
    // The File menu lists them (a list to pick from, not a cycle).
    let file = s.menus().into_iter().find(|m| m.title == "File").unwrap();
    let labels: Vec<String> = file.items.iter().filter_map(|i| if let crate::gpu_viewer::menu::Item::Entry(e) = i { Some(e.label.clone()) } else { None }).collect();
    for (title, _) in templates() {
        assert!(labels.iter().any(|l| l == title), "{labels:?}");
    }
    assert!(labels.contains(&"New part".to_string()) && labels.contains(&"Clear model".to_string()));
}

#[test]
fn clear_model_and_deleting_the_last_feature_really_remove_the_geometry() {
    let mut s = template_session("clear");
    s.do_action(SolidAction::ClearModel);
    let p = &s.problem;
    assert!(p.history.as_ref().is_some_and(|h| h.features.is_empty()) && p.supports.is_empty() && p.loads.is_empty());
    assert!(p.validate_geometry().unwrap_err().contains("no geometry"));
    let scene = s.build_scene().unwrap();
    assert!(scene.label.contains("mesh not generated") || scene.label.contains("no mesh"), "{}", scene.label);
    // A sketch can be added to the cleared model with no conversion.
    act(&mut s, SketchAction::NewSketch(PlaneChoice::Top));
    assert!(s.sketch_mode(), "{}", s.status);

    // Deleting the last feature of a converted model empties it as well, boundary conditions included.
    let mut s = template_session("delete-last");
    act(&mut s, SketchAction::Convert);
    let ids: Vec<_> = s.problem.history.as_ref().unwrap().features.iter().map(|f| f.id).collect();
    assert!(!s.problem.supports.is_empty());
    for id in ids {
        act(&mut s, SketchAction::RemoveFeature(id));
    }
    let p = &s.problem;
    assert!(p.history.as_ref().is_some_and(|h| h.features.is_empty()), "history is empty, not None: {:?}", p.history);
    assert!(p.supports.is_empty() && p.loads.is_empty(), "the old geometry's boundary conditions are gone");
    assert!(p.validate_geometry().is_err());
    s.do_action(SolidAction::Undo);
    assert!(s.problem.history.as_ref().is_some_and(|h| h.features.len() == 1), "one undo step per delete");
}

#[test]
fn deleting_the_only_profile_sketch_is_allowed_and_leaves_an_unbuilt_model() {
    let mut s = template_session("delete-profile");
    act(&mut s, SketchAction::Convert);
    // A second, empty sketch: Sketch1 is the plate's profile.
    act(&mut s, SketchAction::NewSketch(PlaneChoice::Front));
    s.sketching = None;
    let first = s.problem.history.as_ref().unwrap().features[0].id;
    act(&mut s, SketchAction::RemoveFeature(first));
    let h = s.problem.history.as_ref().unwrap();
    assert_eq!(h.features.len(), 1, "the empty sketch remains");
    assert!(s.status.contains("deleted") || s.status.contains("Feature"), "{}", s.status);
    assert!(s.mesh_availability().is_err() && s.solve_availability().is_err());
}

#[test]
fn open_reads_a_saved_problem_or_a_mesh_and_reports_bad_paths() {
    let mut s = template_session("open-src");
    s.do_action(SolidAction::Save);
    let saved = s.path.clone();
    let mut other = template_session("open-dst");
    other.do_action(SolidAction::ClearModel);
    other.open_path(saved.to_str().unwrap());
    assert_eq!(other.problem, s.problem, "{}", other.status);
    assert_eq!(other.path, saved, "Save now writes back to the opened file");
    assert!(!other.dirty);

    other.open_path("/definitely/not/here.json");
    assert!(other.status.starts_with("Open failed"), "{}", other.status);
    other.open_path("/definitely/not/here.msh");
    assert!(other.status.contains("Could not read"), "{}", other.status);
    assert_eq!(other.problem, s.problem, "a failed open keeps the model");

    let mesh = saved.with_file_name("tiny.msh");
    std::fs::write(&mesh, "$MeshFormat\n2.2 0 8\n$EndMeshFormat\n").unwrap();
    other.open_path(mesh.to_str().unwrap());
    assert!(matches!(other.problem.geometry, Geometry::Imported { .. }) && other.problem.history.is_none(), "{}", other.status);
    assert!(other.problem.supports.is_empty());
    let _ = std::fs::remove_dir_all(saved.parent().unwrap());
}

#[test]
fn the_open_prompt_collects_a_path_and_replacing_is_refused_while_sketching() {
    let mut s = template_session("open-prompt");
    s.do_action(SolidAction::OpenFile);
    assert!(s.prompting());
    s.handle_key(Key::Escape, Mods::default());
    assert!(!s.prompting());

    s.do_action(SolidAction::NewPart);
    assert!(s.sketch_mode());
    for a in [SolidAction::NewPart, SolidAction::ClearModel, SolidAction::OpenTemplate(0), SolidAction::OpenFile] {
        let before = s.problem.clone();
        s.do_action(a);
        assert!(s.sketch_mode() && s.problem == before && !s.prompting(), "{a:?}: {}", s.status);
    }
    let _ = Authoring::take_view(&mut s);
}

#[test]
fn open_browser_navigates_and_uses_existing_open_validation() {
    let mut source = template_session("browse-source");
    source.do_action(SolidAction::Save);
    let dir = source.path.parent().unwrap().to_path_buf();
    let nested = dir.join("nested");
    std::fs::create_dir(&nested).unwrap();
    let saved = nested.join("part.fea.json");
    std::fs::copy(&source.path, &saved).unwrap();
    std::fs::write(nested.join("bad.fea.json"), "not json").unwrap();

    let mut s = template_session("browse-destination");
    s.path = dir.join("destination.fea.json");
    s.do_action(SolidAction::OpenFile);
    assert!(s.inspector_cards().iter().any(|c| c.id == "file-browser"), "browser must be visible in the native inspector");
    let index = s.browser.as_ref().unwrap().entries.iter().position(|p| p == &nested).unwrap();
    s.do_action(SolidAction::BrowseEntry(index));
    assert_eq!(s.browser.as_ref().unwrap().dir, nested);
    let index = s.browser.as_ref().unwrap().entries.iter().position(|p| p == &saved).unwrap();
    s.do_action(SolidAction::BrowseEntry(index));
    assert_eq!(s.problem, source.problem);
    assert_eq!(s.path, saved);
    assert!(!s.prompting());

    s.do_action(SolidAction::OpenFile);
    s.do_action(SolidAction::BrowseParent);
    assert_eq!(s.browser.as_ref().unwrap().dir, dir);
    let before = s.problem.clone();
    let index = s.browser.as_ref().unwrap().entries.iter().position(|p| p == &nested).unwrap();
    s.do_action(SolidAction::BrowseEntry(index));
    let index = s.browser.as_ref().unwrap().entries.iter().position(|p| p.ends_with("bad.fea.json")).unwrap();
    s.do_action(SolidAction::BrowseEntry(index));
    assert!(s.status.starts_with("Open failed"));
    assert_eq!(s.problem, before);
    #[cfg(not(feature = "occt"))]
    {
        let step = nested.join("cube.step");
        std::fs::write(&step, solid_kernel::native_brep::NativeBrep::box_from_corners([0.0, 0.0, 0.0], [1.0, 1.0, 1.0]).unwrap().to_step_text()).unwrap();
        s.do_action(SolidAction::OpenFile);
        let index = s.browser.as_ref().unwrap().entries.iter().position(|p| p == &step).unwrap();
        s.do_action(SolidAction::BrowseEntry(index));
        assert!(matches!(s.problem.geometry, Geometry::Solid { .. }), "{}", s.status);
    }
    s.do_action(SolidAction::OpenFile);
    s.handle_key(Key::Escape, Mods::default());
    assert!(s.browser.is_none() && !s.prompting());
    std::fs::remove_dir_all(dir).unwrap();
}
