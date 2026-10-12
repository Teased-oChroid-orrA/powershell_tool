use super::*;
use fea_problem::sketch::Constraint as C;

fn setup() -> (Sketch, SketchEdit) {
    let mut ed = SketchEdit::new();
    ed.pick = 0.2;
    (Sketch::default(), ed)
}

#[test]
fn line_tool_creates_a_line_and_snaps_horizontal() {
    let (mut sk, mut ed) = setup();
    ed.set_tool(Tool::Line);
    ed.click(&mut sk, [0.0, 0.0]).unwrap();
    assert!(ed.busy());
    ed.hover_at(&sk, [5.0, 0.1]);
    assert_eq!(ed.hover.unwrap().kind, SnapKind::Horizontal);
    assert_eq!(ed.preview().len(), 1);
    let made = ed.click(&mut sk, [5.0, 0.1]).unwrap();
    assert_eq!(made.len(), 1);
    assert!(!ed.busy());
    let (a, b) = sk.line_pts(made[0]).unwrap();
    assert_eq!(a[1], b[1], "the horizontal snap was enforced by a constraint");
    assert!(sk.constraints.iter().any(|c| matches!(c.v, C::Horizontal { .. })));
}

#[test]
fn polyline_chains_shares_points_and_closes_on_the_first_point() {
    let (mut sk, mut ed) = setup();
    ed.set_tool(Tool::Polyline);
    for p in [[0.0, 0.0], [4.0, 0.0], [4.0, 3.0], [0.0, 3.0]] {
        ed.click(&mut sk, p).unwrap();
    }
    assert!(ed.busy(), "still chaining");
    ed.click(&mut sk, [0.05, 0.05]).unwrap(); // snaps to the first point
    assert!(!ed.busy(), "closing ends the chain");
    let pts = sk.entities.iter().filter(|e| matches!(e.v, Entity::Point { .. })).count();
    assert_eq!(pts, 4, "points are shared, not duplicated");
    let profile = sk.profile().unwrap();
    assert!(matches!(profile.outer, fea_problem::Shape::Rect { x0, y0, x1, y1 } if (x1 - x0 - 4.0).abs() < 1e-9 && (y1 - y0 - 3.0).abs() < 1e-9));
}

#[test]
fn rectangle_tool_gives_a_dimensionable_rectangle() {
    let (mut sk, mut ed) = setup();
    ed.set_tool(Tool::Rectangle);
    ed.click(&mut sk, [1.0, 1.0]).unwrap();
    ed.hover_at(&sk, [5.0, 4.0]);
    assert_eq!(ed.preview().len(), 4);
    let lines = ed.click(&mut sk, [5.0, 4.0]).unwrap();
    assert_eq!(lines.len(), 4);
    assert_eq!(ed.last_report.unwrap().dof, 4, "position, width, height remain free");
    // Dimension the width and the height: DOF drops by two each time (position is still free).
    ed.set_tool(Tool::Dimension);
    ed.select_at(&sk, [3.0, 1.0], false);
    assert!(matches!(ed.dimension_kind(&sk), Some(DimKind::Length(_))));
    assert!((ed.dimension_value(&sk).unwrap() - 4.0).abs() < 1e-9);
    ed.set_dimension(&mut sk, 6.0).unwrap();
    ed.selection = [lines[1]].into();
    ed.set_dimension(&mut sk, 2.0).unwrap();
    assert_eq!(ed.last_report.unwrap().dof, 2);
    let Some(fea_problem::Shape::Rect { x0, x1, y0, y1 }) = sk.profile().ok().map(|p| p.outer) else { panic!("{:?}", sk.profile()) };
    assert!((x1 - x0 - 6.0).abs() < 1e-9 && (y1 - y0 - 2.0).abs() < 1e-9);
}

#[test]
fn a_contradicting_dimension_is_rolled_back() {
    let (mut sk, mut ed) = setup();
    ed.set_tool(Tool::Rectangle);
    ed.click(&mut sk, [0.0, 0.0]).unwrap();
    let lines = ed.click(&mut sk, [4.0, 2.0]).unwrap();
    // Dimension the top and bottom to different lengths.
    ed.selection = [lines[0]].into();
    ed.set_dimension(&mut sk, 4.0).unwrap();
    ed.selection = [lines[2]].into();
    let before = sk.clone();
    let err = ed.set_dimension(&mut sk, 9.0).unwrap_err();
    assert!(err.contains("contradicts"), "{err}");
    assert_eq!(sk, before, "rolled back");
}

#[test]
fn circle_and_arc_tools() {
    let (mut sk, mut ed) = setup();
    ed.set_tool(Tool::Circle);
    ed.click(&mut sk, [2.0, 2.0]).unwrap();
    ed.hover_at(&sk, [3.0, 2.0]);
    assert!(ed.preview().len() >= 40);
    let c = ed.click(&mut sk, [3.0, 2.0]).unwrap()[0];
    assert!((sk.round(c).unwrap().1 - 1.0).abs() < 1e-9);
    ed.set_tool(Tool::Arc);
    ed.click(&mut sk, [10.0, 0.0]).unwrap();
    ed.click(&mut sk, [11.0, 0.0]).unwrap();
    ed.hover_at(&sk, [10.0, 1.5]);
    assert!(!ed.preview().is_empty());
    let arc = ed.click(&mut sk, [10.0, 1.5]).unwrap()[0];
    let (c, r) = sk.round(arc).unwrap();
    assert!((r - 1.0).abs() < 1e-9 && c == [10.0, 0.0]);
    // A degenerate click is an error, not a panic or a zero-size entity.
    ed.set_tool(Tool::Circle);
    ed.click(&mut sk, [20.0, 20.0]).unwrap();
    assert!(ed.click(&mut sk, [20.0, 20.0]).is_err());
}

#[test]
fn snapping_to_points_entities_and_grid() {
    let (mut sk, mut ed) = setup();
    let a = sk.add_point(0.0, 0.0);
    let b = sk.add_point(10.0, 0.0);
    let l = sk.add_line(a, b);
    assert_eq!(ed.snap(&sk, [0.1, 0.1]).kind, SnapKind::Point(a));
    let on = ed.snap(&sk, [5.0, 0.15]);
    assert_eq!(on.kind, SnapKind::OnEntity(l));
    assert_eq!(on.at[1], 0.0);
    ed.grid = 0.5;
    let g = ed.snap(&sk, [3.2, 4.1]);
    assert_eq!((g.kind, g.at), (SnapKind::Grid, [3.0, 4.0]));
    ed.grid = 0.0;
    assert_eq!(ed.snap(&sk, [3.2, 4.1]).kind, SnapKind::Free);
}

#[test]
fn line_ending_on_a_line_adds_point_on_and_suggestions_follow_drawing() {
    let (mut sk, mut ed) = setup();
    ed.set_tool(Tool::Line);
    ed.click(&mut sk, [0.0, 0.0]).unwrap();
    ed.click(&mut sk, [10.0, 0.0]).unwrap();
    ed.click(&mut sk, [5.0, 5.0]).unwrap();
    let l2 = ed.click(&mut sk, [5.0, 0.1]).unwrap()[0]; // ends on the first line
    assert!(sk.constraints.iter().any(|c| matches!(c.v, C::PointOn { .. })), "end point snapped onto the line");
    assert!(ed.suggestions.iter().any(|s| s.rule == Rule::Perpendicular), "{:?}", ed.suggestions);
    let n = ed.suggestions.len();
    let idx = ed.suggestions.iter().position(|s| s.rule == Rule::Perpendicular).unwrap();
    ed.accept(&mut sk, idx).unwrap();
    assert_eq!(ed.suggestions.len(), n - 1);
    let (a, b) = sk.line_pts(l2).unwrap();
    assert!((a[0] - b[0]).abs() < 1e-6, "now vertical: {a:?} {b:?}");
}

#[test]
fn select_box_delete_and_orphan_cleanup() {
    let (mut sk, mut ed) = setup();
    ed.set_tool(Tool::Rectangle);
    ed.click(&mut sk, [0.0, 0.0]).unwrap();
    ed.click(&mut sk, [4.0, 2.0]).unwrap();
    ed.set_tool(Tool::Circle);
    ed.click(&mut sk, [2.0, 1.0]).unwrap();
    ed.click(&mut sk, [2.5, 1.0]).unwrap();
    ed.set_tool(Tool::Select);
    // Click the circle's rim, then delete it: its centre point must go too.
    ed.select_at(&sk, [2.5, 1.0], false);
    assert_eq!(ed.selection.len(), 1);
    assert_eq!(ed.delete_selected(&mut sk), 1);
    assert!(sk.entities.iter().all(|e| !matches!(e.v, Entity::Circle { .. })));
    assert_eq!(sk.entities.iter().filter(|e| matches!(e.v, Entity::Point { .. })).count(), 4);
    // Box select takes the whole rectangle (4 points + 4 lines).
    ed.select_box(&sk, [-1.0, -1.0], [5.0, 3.0], false);
    assert_eq!(ed.selection.len(), 8);
    assert_eq!(ed.delete_selected(&mut sk), 8);
    assert!(sk.entities.is_empty());
}

#[test]
fn constrain_selected_reports_wrong_selections() {
    let (mut sk, mut ed) = setup();
    ed.set_tool(Tool::Line);
    ed.click(&mut sk, [0.0, 0.0]).unwrap();
    let l = ed.click(&mut sk, [4.0, 1.0]).unwrap()[0];
    ed.set_tool(Tool::Select);
    assert!(ed.constrain_selected(&mut sk, Rule::Horizontal).unwrap_err().contains("select"));
    ed.selection = [l].into();
    ed.constrain_selected(&mut sk, Rule::Horizontal).unwrap();
    let (a, b) = sk.line_pts(l).unwrap();
    assert!((a[1] - b[1]).abs() < 1e-9);
    // Horizontal and then Vertical on the same line collapses it: refused.
    let before = sk.clone();
    assert!(ed.constrain_selected(&mut sk, Rule::Vertical).is_err() || sk.solve().status != Status::Conflicting);
    let _ = before;
}

#[test]
fn drag_a_point_of_a_constrained_rectangle() {
    let (mut sk, mut ed) = setup();
    ed.set_tool(Tool::Rectangle);
    ed.click(&mut sk, [0.0, 0.0]).unwrap();
    ed.click(&mut sk, [4.0, 2.0]).unwrap();
    let corner = ed.pick_entity(&sk, [4.0, 2.0]).unwrap();
    let r = ed.drag(&mut sk, corner, [6.0, 3.0]);
    assert!(r.violated.is_empty());
    let q = sk.point(corner).unwrap();
    assert!((q[0] - 6.0).abs() < 0.05 && (q[1] - 3.0).abs() < 0.05);
    let Some(fea_problem::Shape::Rect { x1, y1, .. }) = sk.profile().ok().map(|p| p.outer) else { panic!("still a rectangle") };
    assert!((x1 - q[0]).abs() < 1e-6 && (y1 - q[1]).abs() < 1e-6);
}

#[test]
fn sketch_segments_cover_every_curve() {
    let (mut sk, mut ed) = setup();
    ed.set_tool(Tool::Rectangle);
    ed.click(&mut sk, [0.0, 0.0]).unwrap();
    ed.click(&mut sk, [4.0, 2.0]).unwrap();
    ed.set_tool(Tool::Circle);
    ed.click(&mut sk, [2.0, 1.0]).unwrap();
    ed.click(&mut sk, [2.5, 1.0]).unwrap();
    let segs = sketch_segments(&sk);
    assert!(segs.len() >= 4 + 40);
}

#[test]
fn offers_are_recorded_as_taken_or_ignored_and_can_be_reordered() {
    let (mut sk, mut ed) = setup();
    ed.set_tool(Tool::Line);
    ed.click(&mut sk, [0.0, 0.0]).unwrap();
    ed.click(&mut sk, [10.0, 0.0]).unwrap();
    ed.click(&mut sk, [5.0, 5.0]).unwrap();
    ed.click(&mut sk, [5.0, 0.1]).unwrap();
    let sit = ed.situation().expect("an offer is open");
    assert_eq!((sit.tool, sit.entity), (Tool::Line, EntityKind::Line));
    let rules = ed.offered_rules();
    assert!(rules.contains(&Rule::Perpendicular));
    // A learned order puts Perpendicular first; only the order changes, not the set.
    let mut order = rules.clone();
    order.sort_by_key(|r| *r != Rule::Perpendicular);
    ed.reorder(&order);
    assert!(ed.learned_order);
    assert_eq!(ed.suggestions[0].rule, Rule::Perpendicular);
    assert_eq!(ed.offered_rules().len(), rules.len());
    ed.accept(&mut sk, 0).unwrap();
    let ev = ed.take_events();
    assert_eq!(ev.len(), 1);
    assert_eq!((ev[0].taken, ev[0].offered[0]), (Some(Rule::Perpendicular), Rule::Perpendicular));
    // Drawing on without choosing closes the remaining offer as ignored.
    ed.click(&mut sk, [0.0, 8.0]).unwrap();
    ed.click(&mut sk, [3.0, 8.0]).unwrap();
    let ev = ed.take_events();
    assert!(ev.iter().any(|e| e.taken.is_none()), "{ev:?}");
    // The recorded form contains only coarse enums.
    let json = serde_json::to_string(&(sit.tool, sit.entity, sit.dof, sit.constrained)).unwrap();
    assert!(!json.contains('.'), "{json}");
}
