//! Sketch mode and the feature history through the same entry points the window uses.
use super::*;
use crate::gpu_viewer::{controller::StandardView, editor::authoring::Authoring};
use crate::gpu_viewer::editor::sketcher::Tool;
use crate::gpu_viewer::ui::DockItem;
use fea_problem::{history::FeatureKind, Shape};

pub(super) const SIZE: [f32; 2] = [900.0, 600.0];
pub(super) fn cam() -> Camera {
    Camera { yaw: 0.0, pitch: 0.0, zoom: 1.6, aspect: 1.5, pan: [0.0, 0.0] }
}
pub(super) fn temp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("solid-sketching-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d.join("model.json")
}
pub(super) fn session(name: &str) -> SolidSession {
    let mut p = fea_problem::templates::templates().remove(0).1;
    p.mesh.size = 1.0;
    SolidSession::new(p, temp(name))
}
/// A new sketch on a legacy model: the user converts it to a history first (a sketch never converts silently).
pub(super) fn new_sketch(s: &mut SolidSession, plane: PlaneChoice) {
    if s.problem.history.is_none() {
        act(s, SketchAction::Convert);
    }
    act(s, SketchAction::NewSketch(plane));
}
/// Window pixel of a sketch-plane point (the inverse of the click mapping).
pub(super) fn px(s: &SolidSession, p: [f64; 2]) -> [f32; 2] {
    let v = s.sketching.as_ref().expect("sketching").view;
    let q = [((p[0] - v.center[0]) / v.span) as f32, ((p[1] - v.center[1]) / v.span) as f32, 0.0];
    let (ndc, _) = cam().project(q);
    Camera::pixel(ndc, SIZE[0], SIZE[1])
}
pub(super) fn click(s: &mut SolidSession, p: [f64; 2]) {
    let at = px(s, p);
    Authoring::hover(s, at, SIZE, &cam());
    Authoring::click(s, at, SIZE, &cam());
}
pub(super) fn act(s: &mut SolidSession, a: SketchAction) {
    s.do_action(SolidAction::Sketch(a));
}
pub(super) fn type_number(s: &mut SolidSession, text: &str) {
    for c in text.chars() {
        s.handle_key(Key::Char(c), Mods::default());
    }
    s.handle_key(Key::Enter, Mods::default());
}
pub(super) fn sketch_of(s: &SolidSession) -> fea_problem::sketch::Sketch {
    let st = s.sketching.as_ref().expect("sketching");
    match &s.problem.history.as_ref().unwrap().feature(st.feature).unwrap().kind {
        FeatureKind::Sketch { sketch, .. } => sketch.clone(),
        _ => panic!("not a sketch"),
    }
}

/// A new sketch with a 10 x 4 rectangle fixed at the origin, built by clicks and typed dimensions.
pub(super) fn drawn_rectangle(name: &str) -> SolidSession {
    let mut s = session(name);
    new_sketch(&mut s, PlaneChoice::Front);
    assert!(s.sketch_mode());
    act(&mut s, SketchAction::Tool(Tool::Rectangle));
    click(&mut s, [0.0, 0.0]);
    click(&mut s, [8.0, 3.0]);
    // Fix the first corner.
    act(&mut s, SketchAction::Tool(Tool::Select));
    click(&mut s, [0.0, 0.0]);
    act(&mut s, SketchAction::Fix);
    // Dimension the bottom edge, then the left edge.
    act(&mut s, SketchAction::Tool(Tool::Dimension));
    click(&mut s, [4.0, 0.0]);
    assert!(s.typing_active(), "a dimensionable selection opens the entry box");
    type_number(&mut s, "10");
    click(&mut s, [0.0, 1.5]);
    type_number(&mut s, "4");
    s
}

#[test]
fn entering_a_sketch_looks_at_the_plane_and_a_new_sketch_is_empty() {
    let mut s = session("enter");
    assert!(s.problem.history.is_none());
    new_sketch(&mut s, PlaneChoice::Top);
    assert_eq!(Authoring::take_view(&mut s), Some(StandardView::Front));
    assert_eq!(Authoring::take_view(&mut s), None, "asked once");
    assert_eq!(s.problem.history.as_ref().unwrap().features.len(), 2, "a plane problem converts to Sketch1 (no extrude), plus the new sketch");
}

#[test]
fn drawing_dimensioning_and_finishing_rebuilds_the_geometry() {
    let mut s = drawn_rectangle("rect");
    let sk = sketch_of(&s);
    assert_eq!(sk.entities.iter().filter(|e| matches!(e.v, fea_problem::sketch::Entity::Line { .. })).count(), 4);
    let report = s.sketching.as_ref().unwrap().edit.last_report.unwrap();
    assert_eq!(report.dof, 0, "fixed corner, width and height: fully defined");
    assert_eq!(report.status, fea_problem::sketch::solve::Status::FullyConstrained);
    // The new (empty-before) sketch is the last one: finishing makes it the profile of the plate.
    act(&mut s, SketchAction::Finish);
    assert!(!s.sketch_mode(), "{}", s.status);
    let Geometry::Sketch { outer, holes, .. } = &s.problem.geometry else { panic!() };
    assert_eq!(*outer, Shape::Rect { x0: 0.0, y0: 0.0, x1: 10.0, y1: 4.0 });
    assert!(holes.is_empty());
    // One undo returns to the geometry before the finish.
    s.do_action(SolidAction::Undo);
    let Geometry::Sketch { outer, .. } = &s.problem.geometry else { panic!() };
    assert_eq!(*outer, Shape::Rect { x0: 0.0, y0: 0.0, x1: 12.0, y1: 4.0 });
}

#[test]
fn an_empty_or_open_sketch_cannot_be_finished() {
    let mut s = session("open");
    new_sketch(&mut s, PlaneChoice::Front);
    act(&mut s, SketchAction::Finish);
    assert!(s.sketch_mode());
    assert!(s.status.contains("no closed profile"), "{}", s.status);
    act(&mut s, SketchAction::Tool(Tool::Line));
    click(&mut s, [0.0, 0.0]);
    click(&mut s, [5.0, 0.0]);
    act(&mut s, SketchAction::Finish);
    assert!(s.sketch_mode() && s.status.to_lowercase().contains("open"), "{}", s.status);
}

#[test]
fn contradicting_a_dimension_is_refused_in_the_ui_path() {
    let mut s = drawn_rectangle("contradict");
    act(&mut s, SketchAction::Tool(Tool::Dimension));
    let before = sketch_of(&s);
    click(&mut s, [5.0, 4.0]); // top edge: dimension it 12, but the bottom edge is 10 and the sides are fixed
    type_number(&mut s, "12");
    assert_eq!(sketch_of(&s), before, "rolled back");
    assert!(s.status.contains("contradicts"), "{}", s.status);
}

#[test]
fn dragging_a_corner_is_one_undo_step_and_keeps_the_rectangle() {
    let mut s = session("drag");
    new_sketch(&mut s, PlaneChoice::Front);
    act(&mut s, SketchAction::Tool(Tool::Rectangle));
    click(&mut s, [0.0, 0.0]);
    click(&mut s, [8.0, 3.0]);
    act(&mut s, SketchAction::Tool(Tool::Select));
    let undo_before = s.undo.len();
    assert!({ let at = px(&s, [8.0, 3.0]); Authoring::grab(&mut s, at, SIZE, &cam()) }, "pressing a corner grabs it");
    for t in [0.25, 0.5, 1.0] {
        let target = [8.0 + 3.0 * t, 3.0 + 2.0 * t];
        let at = px(&s, target);
        Authoring::drag(&mut s, at, SIZE, &cam());
    }
    Authoring::release_grab(&mut s);
    assert_eq!(s.undo.len(), undo_before + 1, "the drag is one undo step");
    let sk = sketch_of(&s);
    let prof = sk.profile().unwrap();
    let Shape::Rect { x1, y1, .. } = prof.outer else { panic!("still a rectangle") };
    assert!((x1 - 11.0).abs() < 0.2 && (y1 - 5.0).abs() < 0.2, "{x1} {y1}");
    // Pressing on empty space does not grab (the view orbits instead).
    assert!(!{ let at = px(&s, [3.0, 1.0]); Authoring::grab(&mut s, at, SIZE, &cam()) });
    s.do_action(SolidAction::Undo);
    let Shape::Rect { x1, .. } = sketch_of(&s).profile().unwrap().outer else { panic!() };
    assert!((x1 - 8.0).abs() < 1e-6);
}

#[test]
fn hover_shows_the_rubber_band_in_the_scene() {
    let mut s = session("hover");
    new_sketch(&mut s, PlaneChoice::Front);
    let empty = s.build_scene().unwrap().positions.len();
    act(&mut s, SketchAction::Tool(Tool::Line));
    click(&mut s, [1.0, 1.0]);
    let at = px(&s, [6.0, 3.0]);
    assert!(Authoring::hover(&mut s, at, SIZE, &cam()), "a new snap position asks for a redraw");
    let with_band = s.build_scene().unwrap();
    assert!(with_band.positions.len() > empty, "preview tube + cursor marker are drawn");
    assert!(with_band.validate().is_ok());
    assert!(!Authoring::hover(&mut s, at, SIZE, &cam()), "the same position does not");
}

#[test]
fn keys_choose_tools_and_escape_steps_back() {
    let mut s = session("keys");
    new_sketch(&mut s, PlaneChoice::Front);
    s.handle_key(Key::Char('l'), Mods::default());
    assert_eq!(s.sketching.as_ref().unwrap().edit.tool, Tool::Line);
    click(&mut s, [1.0, 1.0]);
    assert!(s.sketching.as_ref().unwrap().edit.busy());
    s.handle_key(Key::Escape, Mods::default());
    assert!(!s.sketching.as_ref().unwrap().edit.busy(), "first Esc cancels the gesture");
    s.handle_key(Key::Escape, Mods::default());
    assert_eq!(s.sketching.as_ref().unwrap().edit.tool, Tool::Select, "second Esc returns to Select");
    assert!(s.sketch_mode(), "Esc never leaves the sketch (Finish does)");
}

#[test]
fn suggestions_show_in_the_panel_and_apply_by_number_key() {
    let mut s = session("suggest");
    new_sketch(&mut s, PlaneChoice::Front);
    act(&mut s, SketchAction::Tool(Tool::Line));
    click(&mut s, [0.0, 0.0]);
    click(&mut s, [6.0, 0.0]);
    click(&mut s, [6.5, 5.0]);
    let l2 = {
        click(&mut s, [6.5, 0.5]);
        s.sketching.as_ref().unwrap().edit.suggestions.clone()
    };
    assert!(!l2.is_empty());
    let panel = s.properties();
    assert!(panel.iter().any(|i| matches!(i, DockItem::Heading(h) if h == "Suggested constraints")));
    let n = l2.len();
    s.handle_key(Key::Char('1'), Mods::default());
    assert_eq!(s.sketching.as_ref().unwrap().edit.suggestions.len(), n - 1);
}

#[test]
fn extrude_and_revolve_features_and_parameters() {
    let mut s = session("features");
    // A plate (plane problem) -> convert, then Extrude it.
    act(&mut s, SketchAction::Convert);
    assert_eq!(s.problem.history.as_ref().unwrap().features.len(), 1);
    act(&mut s, SketchAction::AddExtrude);
    assert_eq!(s.problem.analysis, fea_problem::Analysis::Solid, "{}", s.status);
    assert_eq!(Authoring::take_view(&mut s), Some(StandardView::Isometric));
    let ext = s.selected_feature.expect("the new feature is selected");
    // Depth by typing.
    act(&mut s, SketchAction::EditParam(ext, FeatureParam::Depth));
    assert!(s.prompting());
    type_number(&mut s, "2.5");
    let Geometry::Sketch { depth, .. } = &s.problem.geometry else { panic!() };
    assert_eq!(*depth, 2.5);
    // A second solid feature is refused with the reason.
    assert!(s.feature_availability(true).unwrap_err().contains("already has its solid feature"));
    // Suppress the extrude: back to a plane region; restore it.
    act(&mut s, SketchAction::Suppress(ext));
    assert!(s.history().unwrap().feature(ext).unwrap().suppressed);
    act(&mut s, SketchAction::Suppress(ext));
    assert!(!s.history().unwrap().feature(ext).unwrap().suppressed);
    // Bad parameter values report instead of corrupting the model.
    let before = s.problem.clone();
    act(&mut s, SketchAction::EditParam(ext, FeatureParam::Layers));
    type_number(&mut s, "2.5");
    assert_eq!(s.problem, before);
    assert!(s.status.contains("whole number"), "{}", s.status);
    // Rolling back to the sketch and forward again.
    let sk = s.history().unwrap().features[0].id;
    act(&mut s, SketchAction::RollbackTo(Some(sk)));
    assert!(s.history().unwrap().rollback.is_some());
    act(&mut s, SketchAction::RollbackTo(None));
    assert!(s.history().unwrap().rollback.is_none());
    // Deleting the sketch deletes its extrude too and leaves an empty model (no stale body, no dangling supports).
    act(&mut s, SketchAction::RemoveFeature(sk));
    assert!(s.history().unwrap().features.is_empty() && s.problem.supports.is_empty(), "{:?}", s.problem.history);
    assert!(s.problem.validate_geometry().is_err());
}

#[test]
fn tree_ribbon_and_menus_follow_the_mode() {
    let mut s = session("chrome");
    let tabs: Vec<String> = s.ribbon().into_iter().map(|t| t.title).collect();
    assert!(tabs.contains(&"Features".to_string()) && !tabs.contains(&"Sketch".to_string()));
    act(&mut s, SketchAction::Convert);
    assert!(s.tree().iter().any(|n| n.label.starts_with("History")));
    assert!(s.tree().iter().any(|n| n.label.starts_with("Sketch1")));
    new_sketch(&mut s, PlaneChoice::Front);
    let tabs: Vec<String> = s.ribbon().into_iter().map(|t| t.title).collect();
    assert_eq!(tabs, vec!["Sketch".to_string()]);
    let insert = s.menus().into_iter().find(|m| m.title == "Insert").unwrap();
    assert!(insert.items.iter().any(|i| matches!(i, crate::gpu_viewer::menu::Item::Entry(e) if e.label == "Finish sketch")));
    // Constraint entries explain why they are disabled with nothing selected.
    let ribbon = s.ribbon();
    let horizontal = ribbon[0].groups.iter().flat_map(|g| g.buttons.iter()).find(|b| b.label == "Horizontal").unwrap();
    assert!(!horizontal.enabled && horizontal.reason.as_deref().unwrap().contains("select"), "{:?}", horizontal.reason);
}

#[test]
fn a_saved_history_reopens_and_regenerates_the_geometry() {
    let mut s = drawn_rectangle("reopen");
    act(&mut s, SketchAction::Finish);
    let text = s.problem.to_json().unwrap();
    // Simulate a file whose stored geometry is stale: the history is authoritative.
    let mut stale = fea_problem::Problem::from_json(&text).unwrap();
    stale.geometry = Geometry::Sketch { outer: Shape::Rect { x0: 0.0, y0: 0.0, x1: 1.0, y1: 1.0 }, holes: vec![], depth: 1.0, layers: 1, regions: Vec::new() };
    let reopened = SolidSession::new(stale, temp("reopen2"));
    let Geometry::Sketch { outer, .. } = &reopened.problem.geometry else { panic!() };
    assert_eq!(*outer, Shape::Rect { x0: 0.0, y0: 0.0, x1: 10.0, y1: 4.0 });
}

#[test]
fn boundary_conditions_apply_to_selected_sketch_entities() {
    let mut s = drawn_rectangle("bc");
    act(&mut s, SketchAction::Tool(Tool::Select));
    // Click the left edge: it is `left` once built.
    click(&mut s, [0.0, 2.0]);
    assert_eq!(s.selected.as_deref(), Some("left"));
    assert!(s.properties().iter().any(|i| matches!(i, DockItem::Line { text, .. } if text.contains("surfaces: left"))));
    let ribbon = s.ribbon();
    let support = ribbon[0].groups.iter().flat_map(|g| &g.buttons).find(|b| b.label == "Support on selection").unwrap();
    assert!(support.enabled);
    let n = s.problem.supports.len();
    s.do_action(SolidAction::SupportOnSelected);
    assert_eq!(s.problem.supports.len(), n + 1);
    assert_eq!(s.problem.supports.last().unwrap().edge(), Some("left"));
    // Shift-click the top edge too: both named; a point selects no surface.
    let at = px(&s, [5.0, 4.0]);
    Authoring::click_with(&mut s, at, SIZE, &cam(), true);
    assert_eq!(s.all_selected().len(), 2);
    click(&mut s, [0.0, 0.0]); // the fixed corner point
    assert!(s.all_selected().is_empty());
    let ribbon = s.ribbon();
    let load = ribbon[0].groups.iter().flat_map(|g| &g.buttons).find(|b| b.label == "Load on selection").unwrap();
    assert!(!load.enabled && load.reason.as_deref().unwrap().contains("select an edge"));
}

#[test]
fn a_model_with_a_history_does_not_offer_row_editing_of_its_geometry() {
    let mut s = session("rows");
    let before = s.rows();
    assert!(before.contains(&FieldRow::HoleAdd) && before.contains(&FieldRow::OuterKind));
    assert!(s.hole_availability().is_ok());
    act(&mut s, SketchAction::Convert);
    let rows = s.rows();
    for gone in [FieldRow::Source, FieldRow::OuterKind, FieldRow::HoleAdd, FieldRow::Outer(0), FieldRow::Hole(0, 0)] {
        assert!(!rows.contains(&gone), "{gone:?} would be silently undone by the next rebuild");
    }
    assert!(rows.contains(&FieldRow::MeshSize) && rows.contains(&FieldRow::SupportAdd), "everything else stays editable");
    assert!(s.hole_availability().unwrap_err().contains("feature history"));
    assert!(!s.tree().iter().any(|n| n.label.starts_with("Outline") || n.label.starts_with("Hole 1")));
    // Deleting the only feature empties the model; geometry stays history-driven (templates restore row editing).
    let sk = s.history().unwrap().features[0].id;
    act(&mut s, SketchAction::RemoveFeature(sk));
    assert!(s.history().unwrap().features.is_empty() && !s.rows().contains(&FieldRow::HoleAdd));
    s.do_action(SolidAction::OpenTemplate(0));
    assert!(s.rows().contains(&FieldRow::HoleAdd), "a template brings the row editing back");
}

#[test]
fn replacing_the_model_while_sketching_leaves_sketch_mode_cleanly() {
    let mut s = session("replace");
    new_sketch(&mut s, PlaneChoice::Front);
    assert!(s.sketch_mode());
    // Ctrl+N cycles templates: the whole problem (and its history) is replaced.
    s.do_action(SolidAction::NextTemplate);
    assert!(!s.sketch_mode(), "the sketch that was open no longer exists");
    assert!(s.build_scene().is_ok(), "the scene must not error about a missing sketch");
    // Reload from file has the same effect.
    new_sketch(&mut s, PlaneChoice::Front);
    assert!(s.sketch_mode());
    s.do_action(SolidAction::Save);
    act(&mut s, SketchAction::Convert); // refused (already has a history): no-op, still sketching
    s.do_action(SolidAction::Load);
    assert!(!s.sketch_mode() || s.history().is_some_and(|h| s.sketching.as_ref().is_some_and(|st| h.feature(st.feature).is_some())));
    assert!(s.build_scene().is_ok());
}

#[test]
fn solving_is_refused_while_a_sketch_is_open() {
    let mut s = session("solve-while-sketching");
    s.do_action(SolidAction::Advice(0)); // a support, so the model is otherwise solvable
    assert!(s.solve_availability().is_ok(), "{:?}", s.solve_availability());
    new_sketch(&mut s, PlaneChoice::Front);
    let err = s.solve_availability().unwrap_err();
    assert!(err.contains("finish the sketch"), "{err}");
    assert!(s.study_availability(StudyKind::Modal).unwrap_err().contains("finish the sketch"));
}

#[test]
fn cancelling_a_sketch_restores_the_model_and_is_undoable() {
    let mut s = session("cancel");
    act(&mut s, SketchAction::Convert);
    let original = s.problem.clone();
    new_sketch(&mut s, PlaneChoice::Front);
    act(&mut s, SketchAction::Tool(Tool::Rectangle));
    click(&mut s, [0.0, 0.0]);
    click(&mut s, [4.0, 3.0]);
    assert!(s.problem.history.is_some());
    act(&mut s, SketchAction::Cancel);
    assert!(!s.sketch_mode());
    assert_eq!(s.problem, original, "the new sketch is gone");
    assert!(s.status.contains("cancelled"), "{}", s.status);
    // Undo brings the cancelled sketch back (as model data).
    s.do_action(SolidAction::Undo);
    assert!(s.problem.history.is_some());
}

#[test]
fn cancel_after_editing_an_existing_sketch_reverts_only_that_edit() {
    let mut s = drawn_rectangle("cancel2");
    act(&mut s, SketchAction::Finish);
    let id = s.history().unwrap().features.iter().rev().find(|f| matches!(f.kind, FeatureKind::Sketch { .. })).unwrap().id;
    let before = s.problem.clone();
    act(&mut s, SketchAction::Edit(id));
    act(&mut s, SketchAction::Tool(Tool::Circle));
    click(&mut s, [5.0, 2.0]);
    click(&mut s, [6.0, 2.0]);
    assert_ne!(s.problem, before);
    act(&mut s, SketchAction::Cancel);
    assert_eq!(s.problem, before);
    assert!(s.history().is_some());
}

#[test]
fn after_finishing_the_sketch_is_drawn_even_before_any_mesh_exists() {
    let mut s = drawn_rectangle("visible");
    act(&mut s, SketchAction::Finish);
    assert!(!s.sketch_mode() && !s.preview_current(), "no mesh yet");
    let scene = s.build_scene().unwrap();
    assert!(scene.label.contains("sketch outline"), "{}", scene.label);
    // Four edges of the rectangle plus the earlier (template) sketch: far more than the 3-vertex placeholder.
    assert!(scene.triangles.len() > 4 * 8 * 3, "{}", scene.triangles.len());
    assert!(scene.validate().is_ok());
    let tree: Vec<String> = s.tree().into_iter().map(|n| n.label).collect();
    assert!(tree.iter().any(|l| l.starts_with("History (")) && tree.iter().any(|l| l == "Sketch2"), "{tree:?}");
}

#[test]
fn dimensions_and_constraint_glyphs_are_placed_on_the_geometry_in_the_viewport() {
    use crate::gpu_viewer::editor::authoring::OverlayKind;
    let s = drawn_rectangle("overlay");
    let labels = Authoring::overlay(&s, &cam(), SIZE);
    let dims: Vec<&str> = labels.iter().filter(|o| o.kind == OverlayKind::Dimension).map(|o| o.text.as_str()).collect();
    assert!(dims.contains(&"10") && dims.contains(&"4"), "the typed dimensions are on the sketch: {labels:?}");
    let glyphs: Vec<&str> = labels.iter().filter(|o| o.kind == OverlayKind::Constraint).map(|o| o.text.as_str()).collect();
    assert!(glyphs.contains(&"H") && glyphs.contains(&"V"), "horizontal and vertical constraints: {glyphs:?}");
    for o in &labels {
        assert!(o.px[0] >= 0.0 && o.px[0] <= SIZE[0] && o.px[1] >= 0.0 && o.px[1] <= SIZE[1], "inside the viewport: {o:?}");
    }
    // The 10-long dimension sits over the middle of the bottom edge (y = 0), a little above it on screen.
    let bottom_mid = px(&s, [5.0, 0.0]);
    let ten = labels.iter().find(|o| o.text == "10").unwrap();
    assert!((ten.px[0] - bottom_mid[0]).abs() < 8.0 && (ten.px[1] - bottom_mid[1]).abs() < 40.0, "{:?} vs {bottom_mid:?}", ten.px);
    // No two labels sit on top of each other.
    for (i, a) in labels.iter().enumerate() {
        for b in &labels[i + 1..] {
            assert!((a.px[0] - b.px[0]).abs() >= 28.0 || (a.px[1] - b.px[1]).abs() >= 14.0, "{a:?} overlaps {b:?}");
        }
    }
    // Outside sketch mode there is nothing to place.
    let plain = session("overlay-none");
    assert!(Authoring::overlay(&plain, &cam(), SIZE).is_empty());
}

#[test]
fn a_sketch_can_be_placed_on_a_flat_face_of_the_extruded_body() {
    use fea_problem::history::Plane;
    let mut s = drawn_rectangle("on-face");
    act(&mut s, SketchAction::Finish);
    act(&mut s, SketchAction::AddExtrude);
    let faces = s.face_choices();
    assert_eq!(faces.len(), 6, "start, end and the four sides of the rectangle");
    let end = faces.iter().position(|f| f.name == "end").unwrap();
    // The Insert menu and the Features ribbon list the faces.
    let label = "Sketch on face 'end'";
    assert!(s.features_ribbon().groups.iter().flat_map(|g| g.buttons.iter()).any(|e| e.label == label), "ribbon entry");
    act(&mut s, SketchAction::NewSketchOnFace(end));
    assert!(s.sketch_mode(), "{}", s.status);
    let feature = s.sketching.as_ref().unwrap().feature;
    let FeatureKind::Sketch { plane, .. } = &s.problem.history.as_ref().unwrap().feature(feature).unwrap().kind else { panic!() };
    let Plane::Datum { origin, normal, .. } = plane else { panic!("{plane:?}") };
    assert!(origin[2] > 0.0, "the end cap lies at the extrusion depth: {origin:?}");
    assert_eq!(*normal, [0.0, 0.0, 1.0]);
    // Drawing works in the face's own coordinates.
    act(&mut s, SketchAction::Tool(Tool::Circle));
    click(&mut s, [2.0, 2.0]);
    click(&mut s, [3.0, 2.0]);
    assert!(sketch_of(&s).entities.iter().any(|e| matches!(e.v, fea_problem::sketch::Entity::Circle { .. })));
    // A stale index is refused with a message, never a panic.
    act(&mut s, SketchAction::Finish);
    act(&mut s, SketchAction::NewSketchOnFace(99));
    assert!(s.status.contains("no longer exists"), "{}", s.status);
}

#[test]
fn a_sweep_takes_the_selected_sketch_as_profile_and_the_last_as_path() {
    let mut s = drawn_rectangle("sweep");
    let rect = s.sketching.as_ref().unwrap().feature;
    act(&mut s, SketchAction::Finish);
    // The converted template's own sketch comes first; only the drawn rectangle is the profile here.
    // The path: a straight line leaving the rectangle's plane (the Right plane's sketch y is world z).
    act(&mut s, SketchAction::NewSketch(PlaneChoice::Right));
    act(&mut s, SketchAction::Tool(Tool::Line));
    click(&mut s, [0.0, 0.0]);
    click(&mut s, [0.0, 3.0]);
    act(&mut s, SketchAction::Tool(Tool::Select));
    act(&mut s, SketchAction::Finish);
    assert!(s.sketch_mode(), "an open line is no profile: {}", s.status);
    act(&mut s, SketchAction::FinishPath);
    assert!(!s.sketch_mode(), "{}", s.status);
    act(&mut s, SketchAction::SelectFeature(rect));
    let (profile, path) = s.sweep_sketches().unwrap();
    assert_eq!(profile, rect);
    assert!(path > rect);
    act(&mut s, SketchAction::AddSweep);
    let body = s.body_feature().expect("the sweep exists").clone();
    let FeatureKind::Sweep { profile: p, path: q, layers } = body.kind else { panic!("{:?} / {}", body.kind, s.status) };
    assert_eq!((p, q), (profile, path));
    assert!(layers > 0);
    assert_eq!(s.problem.analysis, fea_problem::Analysis::Solid);
    assert!(s.status.starts_with("Swept"), "{}", s.status);
    // The tree lists it, and a second solid feature is refused.
    assert!(s.history_nodes().iter().any(|n| n.label.starts_with("Sweep1")));
    assert!(s.sweep_sketches().is_err());
}

#[test]
fn a_spline_is_drawn_through_clicked_points_and_closes_a_profile() {
    use fea_problem::sketch::Entity;
    let mut s = session("spline");
    new_sketch(&mut s, PlaneChoice::Front);
    act(&mut s, SketchAction::Tool(Tool::Spline));
    click(&mut s, [0.0, 0.0]);
    click(&mut s, [2.0, 1.5]);
    assert!(s.sketching.as_ref().unwrap().edit.busy());
    click(&mut s, [5.0, 1.0]);
    s.handle_key(Key::Enter, Mods::default());
    let sk = sketch_of(&s);
    let spline = sk.entities.iter().find_map(|e| if let Entity::Spline { pts } = &e.v { Some(pts.clone()) } else { None }).expect("a spline was made");
    assert_eq!(spline.len(), 3, "{:?}", sk.entities);
    assert!(!s.sketching.as_ref().unwrap().edit.busy(), "{}", s.status);
    // Close it with a line between its ends: the profile is the area under the curve.
    act(&mut s, SketchAction::Tool(Tool::Line));
    click(&mut s, [5.0, 1.0]);
    click(&mut s, [0.0, 0.0]);
    act(&mut s, SketchAction::Tool(Tool::Select));
    act(&mut s, SketchAction::Finish);
    assert!(!s.sketch_mode(), "{}", s.status);
    let Geometry::Sketch { outer, .. } = &s.problem.geometry else { panic!() };
    assert!(matches!(outer, Shape::Outline { segs } if segs.len() == 2 && matches!(segs[0], fea_problem::Seg::Spline { .. } | fea_problem::Seg::Line { .. })), "{outer:?}");
    // Clicking the last point again also ends a spline.
    new_sketch(&mut s, PlaneChoice::Front);
    act(&mut s, SketchAction::Tool(Tool::Spline));
    click(&mut s, [1.0, 1.0]);
    click(&mut s, [3.0, 2.0]);
    click(&mut s, [3.0, 2.0]);
    assert!(sketch_of(&s).entities.iter().any(|e| matches!(&e.v, Entity::Spline { pts } if pts.len() == 2)), "{}", s.status);
    // One point is not a spline.
    act(&mut s, SketchAction::Tool(Tool::Spline));
    click(&mut s, [8.0, 8.0]);
    s.handle_key(Key::Enter, Mods::default());
    assert!(s.status.contains("at least two"), "{}", s.status);
}

#[test]
fn a_selected_line_becomes_construction_geometry_and_leaves_the_profile() {
    let mut s = drawn_rectangle("construction");
    // A diagonal through two corners: a branch in the outline until it is construction.
    act(&mut s, SketchAction::Tool(Tool::Line));
    click(&mut s, [0.0, 0.0]);
    click(&mut s, [10.0, 4.0]);
    act(&mut s, SketchAction::Tool(Tool::Select));
    act(&mut s, SketchAction::Finish);
    assert!(s.sketch_mode() && s.status.to_lowercase().contains("more than two"), "{}", s.status);
    // Nothing selected: the command says what it needs. Then select the diagonal and press X.
    act(&mut s, SketchAction::ToggleConstruction);
    assert!(s.status.contains("select a line"), "{}", s.status);
    click(&mut s, [5.0, 2.0]);
    s.handle_key(Key::Char('x'), Mods::default());
    assert!(s.status.contains("construction geometry"), "{}", s.status);
    assert_eq!(sketch_of(&s).construction.len(), 1);
    act(&mut s, SketchAction::Finish);
    assert!(!s.sketch_mode(), "the profile is the rectangle again: {}", s.status);
    // Back in the sketch the toggle undoes it, and the outline is refused again.
    let feature = s.history().unwrap().features.iter().rev().find(|f| matches!(f.kind, FeatureKind::Sketch { .. })).unwrap().id;
    act(&mut s, SketchAction::Edit(feature));
    click(&mut s, [5.0, 2.0]);
    act(&mut s, SketchAction::ToggleConstruction);
    assert!(sketch_of(&s).construction.is_empty(), "{}", s.status);
    act(&mut s, SketchAction::Finish);
    assert!(s.sketch_mode(), "the diagonal is real geometry again");
}

/// Sketch lines keep their width in pixels: world-space tubes scaled with the zoom, so a thin profile (two edges
/// 0.125 apart on a 10 long sketch) stayed one blurred line however far the user zoomed in.
#[test]
fn sketch_lines_keep_their_pixel_width_at_every_zoom() {
    let mut s = drawn_rectangle("pixelwidth");
    let axis_width_px = |s: &SolidSession, zoom: f32| {
        let scene = s.sketch_scene().unwrap();
        // The first eight vertices are the horizontal axis tube; its y extent is the line width.
        let (lo, hi) = scene.positions[..8].iter().fold((f32::MAX, f32::MIN), |(lo, hi), p| (lo.min(p[1]), hi.max(p[1])));
        (hi - lo) * zoom * SIZE[1] / 2.0
    };
    let mut seen = Vec::new();
    for zoom in [1.6, 16.0, 160.0, 0.4] {
        let c = Camera { zoom, ..cam() };
        s.set_sketch_pixel(&c, SIZE);
        seen.push(axis_width_px(&s, zoom));
    }
    for w in &seen {
        assert!((w - 1.4).abs() < 0.05, "axis line width in pixels {seen:?}");
    }
    // A zoom change makes the viewer rebuild the scene; the same zoom does not.
    let c = Camera { zoom: 3.0, ..cam() };
    s.set_sketch_pixel(&c, SIZE);
    s.build_scene().unwrap();
    assert!(!s.needs_scene());
    s.set_sketch_pixel(&c, SIZE);
    assert!(!s.needs_scene(), "an unchanged zoom keeps the built scene");
    s.set_sketch_pixel(&Camera { zoom: 6.0, ..cam() }, SIZE);
    assert!(s.needs_scene());
}
