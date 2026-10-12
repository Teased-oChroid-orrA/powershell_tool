//! Mesh-density hints through the solid session: drawn in the viewport, applied from the panel, recorded and ranked.
use super::*;
use crate::gpu_viewer::{editor::{authoring::Authoring, rank::Store}, ui::DockItem};
use super::refining::{thousands, zone_outline};
use fea_problem::{advice::RegionKind, Load};

fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("solid-refining-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}
fn session(name: &str) -> SolidSession {
    let mut p = fea_problem::templates::templates().remove(0).1;
    p.supports.clear();
    p.loads.clear();
    p.mesh.size = 1.0;
    let mut s = SolidSession::new(p, dir(name).join("model.json"));
    s.advisors = advising::Advisors::in_dir(dir(&format!("{name}-advisor")));
    s
}
fn lines(s: &SolidSession) -> Vec<String> {
    s.properties().into_iter().filter_map(|i| match i {
        DockItem::Heading(t) => Some(t),
        i @ (DockItem::Line { .. } | DockItem::Field { .. }) => i.plain_text(),
        _ => None,
    }).collect()
}
fn mesh_nodes_near(s: &SolidSession, c: [f64; 2], r: f64) -> usize {
    s.preview.as_ref().unwrap().mesh.nodes.iter().filter(|n| (n[0] - c[0]).hypot(n[1] - c[1]) <= r).count()
}

#[test]
fn the_panel_lists_the_regions_with_the_recommended_level_and_alternatives() {
    let s = session("panel");
    let t = lines(&s);
    assert!(t.iter().any(|l| l == "Mesh density"), "{t:?}");
    assert!(t.iter().any(|l| l.contains("Refine hole at level 2 (x0.25)")), "{t:?}");
    assert!(t.iter().any(|l| l.contains("or level 3") ) && t.iter().any(|l| l.contains("or level 1")), "{t:?}");
    assert!(t.iter().any(|l| l.contains("thicker = finer")));
    // Not while sketching or when there is no valid body.
    let mut c = session("panel-clear");
    c.do_action(SolidAction::ClearModel);
    assert!(!lines(&c).iter().any(|l| l == "Mesh density"));
}

#[test]
fn applying_a_level_adds_one_undoable_zone_and_the_next_mesh_is_finer_there() {
    let mut s = session("apply");
    // The template already grades the mesh towards the hole (factor 0.1); without that the zone is the only refinement.
    s.problem.mesh.hole_factor = 1.0;
    s.do_action(SolidAction::GenerateMesh);
    let t = std::time::Instant::now();
    while !s.preview_current() {
        s.poll();
        assert!(t.elapsed().as_secs() < 60);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    // The hole zone is 1.25 radii (0.625) at full level, blending out to 1.0 from the centre of the hole.
    let before = mesh_nodes_near(&s, [6.0, 2.0], 0.8);
    let undo_depth = s.undo.len();
    s.do_action(SolidAction::Refine(0, 3));
    assert_eq!(s.problem.mesh.refine.len(), 1, "{}", s.status);
    assert_eq!((s.problem.mesh.refine[0].level, s.problem.mesh.refine[0].centre), (3, [6.0, 2.0]));
    assert_eq!(s.undo.len(), undo_depth + 1, "one undo step");
    assert!(!s.preview_current(), "the mesh is stale until it is generated again: nothing meshes on its own");
    s.do_action(SolidAction::GenerateMesh);
    let t = std::time::Instant::now();
    while !s.preview_current() {
        s.poll();
        assert!(t.elapsed().as_secs() < 60);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(mesh_nodes_near(&s, [6.0, 2.0], 0.8) > 2 * before, "{before} -> {}", mesh_nodes_near(&s, [6.0, 2.0], 0.8));
    // The panel now offers to remove it; removing and undo both work.
    assert!(lines(&s).iter().any(|l| l.contains("level 3") && l.contains("click to remove")), "{:?}", lines(&s));
    s.do_action(SolidAction::Refine(0, 0));
    assert!(s.problem.mesh.refine.is_empty());
    s.do_action(SolidAction::Undo);
    assert_eq!(s.problem.mesh.refine.len(), 1);
    s.do_action(SolidAction::ClearRefine);
    assert!(s.problem.mesh.refine.is_empty());
    s.do_action(SolidAction::Refine(9, 2));
    assert!(s.status.contains("no longer applies"), "{}", s.status);
}

#[test]
fn rings_are_drawn_before_and_after_the_mesh_exists_and_can_be_hidden() {
    let mut s = session("rings");
    s.do_action(SolidAction::Sketch(SketchAction::Convert));
    let plain = |s: &mut SolidSession| {
        s.do_action(SolidAction::ToggleRefineShow);
        let n = s.build_scene().unwrap().positions.len();
        s.do_action(SolidAction::ToggleRefineShow);
        n
    };
    // No mesh: the outline plus the ring of the hole hint.
    let without = plain(&mut s);
    let with = s.build_scene().unwrap();
    assert!(with.positions.len() > without && with.validate().is_ok(), "{} vs {without}", with.positions.len());
    // With a mesh: the mesh scene plus the ring, valid and with picking still addressing only the mesh.
    s.do_action(SolidAction::GenerateMesh);
    let t = std::time::Instant::now();
    while !s.preview_current() {
        s.poll();
        assert!(t.elapsed().as_secs() < 60);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let nodes = s.preview.as_ref().unwrap().mesh.nodes.len();
    let scene = s.build_scene().unwrap();
    assert!(scene.positions.len() > nodes && scene.validate().is_ok(), "{} nodes, {} vertices", nodes, scene.positions.len());
    assert_eq!(scene.samples.len(), scene.positions.len());
    assert_eq!(s.pick_positions.len(), nodes, "rings are not pickable surfaces");
    // Thicker for a finer level (both levels satisfy the hint, so each shows exactly its own ring).
    s.do_action(SolidAction::Refine(0, 2));
    let level1 = s.build_scene().unwrap();
    s.do_action(SolidAction::Refine(0, 0));
    s.do_action(SolidAction::Refine(0, 3));
    let level3 = s.build_scene().unwrap();
    assert_eq!(level1.positions.len(), level3.positions.len());
    let spread = |sc: &crate::gpu_viewer::Scene| sc.positions.iter().map(|p| p[2].abs()).fold(0.0_f32, f32::max);
    assert!(spread(&level3) > spread(&level1), "level 3 rings are thicker than level 2");
}

#[test]
fn applying_a_level_records_the_choice_and_the_ranker_keeps_learning() {
    let mut s = session("record");
    let store = Store::new(dir("record-advisor"), "refinement");
    s.do_action(SolidAction::Advisor(AdvisorOp::ToggleRecord));
    s.do_action(SolidAction::Refine(0, 3));
    let events = store.read::<refining::RefineSituation, refining::RefineLevel>().unwrap();
    assert_eq!(events.len(), 1);
    let e = &events[0];
    assert_eq!((e.situation.kind, e.situation.loaded, e.taken), (RegionKind::Hole, false, Some(refining::RefineLevel::Three)));
    assert_eq!(e.offered, vec![refining::RefineLevel::Two, refining::RefineLevel::Three, refining::RefineLevel::One], "the rules' order");
    let json = serde_json::to_string(e).unwrap();
    for forbidden in ["hole1", "centre", "radius", "x\":", "name"] {
        assert!(!json.contains(forbidden), "{forbidden} in {json}");
    }
    // Removing a zone is not a level choice and is not recorded.
    s.do_action(SolidAction::Refine(0, 0));
    assert_eq!(store.read::<refining::RefineSituation, refining::RefineLevel>().unwrap().len(), 1);
}

#[test]
fn with_the_simulated_prior_the_hole_hint_on_a_coarse_mesh_leads_with_level_3_only_when_enabled() {
    let mut s = session("prior");
    let first = |s: &SolidSession| lines(s).into_iter().find(|l| l.starts_with("Refine hole at level")).unwrap();
    assert!(first(&s).contains("level 2"), "rules first: {}", first(&s));
    s.do_action(SolidAction::Advisor(AdvisorOp::BuildCorpus));
    assert!(first(&s).contains("level 2"), "building the prior changes nothing by itself");
    s.do_action(SolidAction::Advisor(AdvisorOp::ToggleLearned));
    assert!(s.advisors.refinement.settings.use_learned, "{}", s.status);
    // Mesh size 1.0 against hole radius 0.5 is a barely resolved hole: the prior's analysts take level 3.
    assert!(first(&s).contains("level 3"), "{}", first(&s));
    // Only the order moved: all three levels are still offered.
    let t = lines(&s);
    assert!(t.iter().any(|l| l.contains("or level 2")) && t.iter().any(|l| l.contains("or level 1")), "{t:?}");
}

#[test]
fn refine_everywhere_applies_the_recommended_levels_in_one_step_and_reports_when_done() {
    let mut s = session("all");
    s.problem.loads.push(Load::Pressure { edge: "right".into(), p: 10.0 });
    s.problem.supports.push(fea_problem::Support::fixed("left"));
    assert!(s.refine_availability().is_ok());
    let depth = s.undo.len();
    s.do_action(SolidAction::RefineAll);
    assert_eq!(s.problem.mesh.refine.len(), 3, "hole, support and load: {}", s.status);
    assert_eq!(s.undo.len(), depth + 1);
    assert!(s.refine_availability().unwrap_err().contains("already refined"));
    s.do_action(SolidAction::RefineAll);
    assert!(s.status.contains("Nothing left"), "{}", s.status);
    let menu = s.menus().into_iter().find(|m| m.title == "Tools").unwrap();
    assert!(menu.items.iter().any(|i| matches!(i, crate::gpu_viewer::menu::Item::Entry(e) if e.label == "Refine where recommended")));
}

fn wait_solved(s: &mut SolidSession) {
    let t = std::time::Instant::now();
    while s.solve_worker.is_busy() || s.solved.is_none() {
        s.poll();
        assert!(t.elapsed().as_secs() < 120, "{}", s.status);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    s.poll();
}

#[test]
fn the_panel_shows_a_budget_and_per_zone_element_counts_and_bands_are_drawn_as_capsules() {
    let mut s = session("budget");
    s.problem.supports.push(fea_problem::Support::fixed("left"));
    s.do_action(SolidAction::RefineAll);
    let t = lines(&s);
    assert!(t.iter().any(|l| l.starts_with("budget: about ") && l.contains("at full refinement")), "{t:?}");
    assert!(t.iter().any(|l| l.contains("about ") && l.contains("elements, click to remove")), "{t:?}");
    // The left band's outline stays within one radius of its edge and spans it.
    let z = s.problem.mesh.refine.iter().find(|z| z.to.is_some()).expect("a band");
    let pts = zone_outline(z);
    let (lo, hi) = pts.iter().fold(([f64::MAX; 2], [f64::MIN; 2]), |(l, h), p| ([l[0].min(p[0]), l[1].min(p[1])], [h[0].max(p[0]), h[1].max(p[1])]));
    assert!((hi[0] - lo[0] - 2.0 * z.radius).abs() < 1e-6 && (hi[1] - lo[1] - (4.0 + 2.0 * z.radius)).abs() < 1e-6, "{lo:?} {hi:?}");
    assert_eq!(thousands(1234567.4), "1,234,567");
    assert_eq!(thousands(12.0), "12");
}

#[test]
fn the_error_estimate_of_a_solve_is_folded_into_the_model_at_the_next_mesh_not_before() {
    let mut s = session("zz");
    s.problem.supports.push(fea_problem::Support::fixed("left"));
    s.problem.loads.push(Load::Pressure { edge: "right".into(), p: 10.0 });
    s.problem.mesh.adapt_passes = 0;
    s.problem.mesh.size = 0.8;
    s.problem.mesh.hole_factor = 1.0;
    s.problem.mesh.target_error = 0.005;
    s.do_action(SolidAction::Solve);
    wait_solved(&mut s);
    assert!(!s.error_zones.is_empty(), "{}", s.status);
    assert!(s.status.contains("error estimate adds"), "{}", s.status);
    assert!(s.current_solution().is_some() && s.problem.mesh.refine.is_empty(), "the result just shown stays current");
    let (n, depth) = (s.error_zones.len(), s.undo.len());
    assert!(lines(&s).iter().any(|l| l.contains("from the last solve's error estimate")));
    s.do_action(SolidAction::GenerateMesh);
    assert_eq!(s.problem.mesh.refine.len(), n);
    assert_eq!(s.undo.len(), depth + 1, "one undoable edit");
    assert!(s.error_zones.is_empty() && s.status.contains("from the error estimate"), "{}", s.status);
}
