//! Recommendations, boundary-kind ranking and the opt-in learning loop through the solid session.
use super::sketching_tests::new_sketch;
use super::*;
use crate::gpu_viewer::{
    editor::{
        authoring::Authoring,
        rank::{Recorded, Store},
        sketcher::Tool,
    },
    menu::Item,
    ui::DockItem,
};
use fea_problem::{advice::BcKind, sketch::infer::Rule, Support};

fn temp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("solid-advising-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}
fn session(name: &str) -> SolidSession {
    let mut p = fea_problem::templates::templates().remove(0).1;
    p.supports.clear();
    p.loads.clear();
    p.mesh.size = 0.5;
    let mut s = SolidSession::new(p, temp(name).join("model.json"));
    s.advisors = advising::Advisors::in_dir(temp(&format!("{name}-advisor")));
    s
}
pub(super) fn wait_for_corpus(s: &mut SolidSession) {
    let until = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while s.advisors.constraints.bundle.is_none() || s.advisors.boundary.bundle.is_none() || s.advisors.refinement.bundle.is_none() {
        s.poll();
        assert!(std::time::Instant::now() < until, "corpus training did not finish");
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}
fn rows(s: &SolidSession) -> Vec<String> {
    s.properties().into_iter().filter_map(|i| match i {
        DockItem::Heading(t) => Some(t),
        i @ (DockItem::Line { .. } | DockItem::Field { .. }) => i.plain_text(),
        _ => None,
    }).collect()
}
fn select(s: &mut SolidSession, name: &str) {
    let i = s.names().iter().position(|n| n == name).unwrap();
    s.do_action(SolidAction::SelectSurface(i));
}
fn entries(items: &[Item]) -> Vec<String> {
    items.iter().filter_map(|i| if let Item::Entry(e) = i { Some(e.label.clone()) } else { None }).collect()
}

#[test]
fn recommendations_are_listed_applied_and_undone() {
    let mut s = session("recs");
    let text = rows(&s);
    assert!(text.iter().any(|t| t == "Recommendations"), "{text:?}");
    assert!(text.iter().any(|t| t.contains("Add a fixed support on left")), "{text:?}");
    s.do_action(SolidAction::Advice(0));
    assert!(s.problem.supports.contains(&Support::fixed("left")), "{}", s.status);
    s.do_action(SolidAction::Undo);
    assert!(s.problem.supports.is_empty());
    // Hidden by the menu toggle; never shown while sketching.
    s.do_action(SolidAction::Advisor(AdvisorOp::ToggleShow));
    assert!(!rows(&s).iter().any(|t| t == "Recommendations"));
    s.do_action(SolidAction::Advisor(AdvisorOp::ToggleShow));
    new_sketch(&mut s, PlaneChoice::Front);
    assert!(!rows(&s).iter().any(|t| t == "Recommendations"));
}

#[test]
fn a_stale_recommendation_index_is_refused_not_misapplied() {
    let mut s = session("stale");
    s.do_action(SolidAction::Advice(99));
    assert!(s.status.contains("no longer applies"));
    assert!(s.problem.supports.is_empty());
}

#[test]
fn the_surface_popup_offers_the_kinds_that_fit_in_the_rules_order() {
    let mut s = session("popup");
    select(&mut s, "left");
    s.popup = true;
    let first = Authoring::take_popup(&mut s).unwrap();
    assert_eq!(entries(&first)[0], "Fixed", "{:?}", entries(&first));
    assert!(entries(&first).contains(&"Hold directions...".to_string()) && entries(&first).contains(&"Pressure...".to_string()) && entries(&first).contains(&"Force...".to_string()), "{:?}", entries(&first));
    // With a support in place the same click offers a load first; a hole offers the pin load.
    s.do_action(SolidAction::BoundaryKind(BcKind::Fixed));
    assert_eq!(s.problem.supports.len(), 1);
    select(&mut s, "right");
    s.popup = true;
    assert_eq!(entries(&Authoring::take_popup(&mut s).unwrap())[0], "Pressure...");
    select(&mut s, "hole1");
    s.popup = true;
    assert_eq!(entries(&Authoring::take_popup(&mut s).unwrap())[0], "Pin load...");
    s.do_action(SolidAction::BoundaryKind(BcKind::Bearing));
    assert!(matches!(s.problem.loads.last(), Some(fea_problem::Load::Bearing { hole: 1, .. })));
    // A bearing load on an outline edge is refused with the reason and changes nothing.
    select(&mut s, "right");
    let before = s.problem.clone();
    s.do_action(SolidAction::BoundaryKind(BcKind::Bearing));
    assert_eq!(s.problem, before);
    assert!(s.status.contains("acts on a hole"), "{}", s.status);
}

#[test]
fn recording_is_off_until_enabled_and_content_free() {
    let mut s = session("rec");
    let dir = s.advisors.boundary.store.as_ref().unwrap().dir().to_path_buf();
    select(&mut s, "left");
    s.popup = true;
    let _ = Authoring::take_popup(&mut s);
    s.do_action(SolidAction::BoundaryKind(BcKind::Fixed));
    assert!(!dir.join("boundary.jsonl").exists(), "recording is off by default");
    s.do_action(SolidAction::Advisor(AdvisorOp::ToggleRecord));
    assert!(s.advisors.boundary.settings.record, "{}", s.status);
    select(&mut s, "right");
    s.popup = true;
    let _ = Authoring::take_popup(&mut s);
    s.do_action(SolidAction::BoundaryKind(BcKind::Pressure));
    let text = std::fs::read_to_string(dir.join("boundary.jsonl")).unwrap();
    assert_eq!(text.lines().count(), 1);
    for forbidden in ["left", "right", "hole", "1000", "name"] {
        assert!(!text.contains(forbidden), "{forbidden} in {text}");
    }
    s.do_action(SolidAction::Advisor(AdvisorOp::DeleteRecordings));
    assert!(!dir.join("boundary.jsonl").exists());
}

/// Synthetic population (not user data): everyone picks the LAST rule of the offer the rules put first.
#[test]
fn learned_constraint_order_needs_a_trained_gated_model_and_then_reorders() {
    let mut s = session("learn");
    let draw = |s: &mut SolidSession| -> Vec<Rule> {
        new_sketch(s, PlaneChoice::Front);
        let tool = |s: &mut SolidSession, t| s.do_action(SolidAction::Sketch(SketchAction::Tool(t)));
        tool(s, Tool::Line);
        let st = s.sketching.as_mut().unwrap();
        // Draw through the pure core (the pixel path is covered by sketching_tests).
        let feature = st.feature;
        let mut ed = std::mem::take(&mut st.edit);
        let (_, sk) = s.problem.history.as_mut().unwrap().sketch_mut(feature).unwrap();
        ed.pick = 0.2;
        ed.set_tool(Tool::Line);
        for p in [[0.0, 0.0], [10.0, 0.0], [5.0, 5.0], [5.0, 0.1]] {
            ed.click(sk, p).unwrap();
        }
        s.sketching.as_mut().unwrap().edit = ed;
        s.settle_constraint_advice();
        s.sketching.as_ref().unwrap().edit.offered_rules()
    };
    let rules = draw(&mut s);
    assert!(rules.len() >= 2, "{rules:?}");
    let sit = s.sketching.as_ref().unwrap().edit.situation().unwrap();
    let target = *rules.last().unwrap();
    // No model yet: the toggle explains why it is unavailable and the order stays the rules'.
    s.do_action(SolidAction::Advisor(AdvisorOp::ToggleLearned));
    assert!(s.status.contains("no trained model"), "{}", s.status);
    assert_eq!(s.sketching.as_ref().unwrap().edit.offered_rules(), rules);
    // Fill the store with a synthetic population and train.
    let dir = s.advisors.constraints.store.as_ref().unwrap().dir().to_path_buf();
    let store = Store::new(dir, "constraints");
    for session in 0..60u64 {
        for _ in 0..12 {
            store.append(&Recorded { session, situation: sit, offered: rules.clone(), taken: Some(target),
                }).unwrap();
        }
    }
    s.do_action(SolidAction::Advisor(AdvisorOp::Train));
    let until=std::time::Instant::now()+std::time::Duration::from_secs(10);
    while s.advisors.constraints.bundle.is_none() { s.poll(); assert!(std::time::Instant::now()<until); std::thread::sleep(std::time::Duration::from_millis(2)); }
    let report=s.advisors.constraints.bundle.as_ref().unwrap().report.summary("Constraints");
    assert!(report.contains("eligible to use"), "{report}");
    s.do_action(SolidAction::Advisor(AdvisorOp::ToggleLearned));
    assert!(s.advisors.constraints.settings.use_learned, "{}", s.status);
    // A new sketch with the same situation now shows the learned order first; the set is unchanged.
    s.do_action(SolidAction::Sketch(SketchAction::Finish)); // fails (open profile) and stays: leave via undo-free path
    let mut s2 = SolidSession::new(s.problem.clone(), temp("learn2").join("m.json"));
    s2.advisors = advising::Advisors::in_dir(s.advisors.constraints.store.as_ref().unwrap().dir().to_path_buf(),
    );
    let rules2 = draw(&mut s2);
    assert_eq!(rules2[0], target, "{rules2:?}");
    let mut a = rules.clone();
    let mut b = rules2.clone();
    a.sort();
    b.sort();
    assert_eq!(a, b, "only the order changed");
    assert!(s2.sketching.as_ref().unwrap().edit.learned_order);
}

#[test]
fn advisor_menu_states_what_is_possible() {
    let s = session("menu");
    let menu = s.menus().into_iter().find(|m| m.title == "Advisor").unwrap();
    let by = |label: &str| {
        menu.items.iter().find_map(|i| match i {
            Item::Entry(e) if e.label.starts_with(label) => Some(e.clone()),
            _ => None,
        }).unwrap_or_else(|| panic!("{label}"))
    };
    assert!(by("Show recommendations").enabled && by("Show recommendations").checked == Some(true));
    assert!(by("Record my").enabled && by("Record my").checked == Some(false));
    assert!(!by("Use learned").enabled && by("Use learned").reason.as_deref().unwrap().contains("no trained model"));
    assert!(!by("Train").enabled && by("Train").reason.as_deref().unwrap().contains("nothing recorded"));
    assert!(!by("Delete").enabled);
}

#[test]
fn building_the_corpus_trains_both_rankers_and_the_learned_order_is_opt_in_and_labelled() {
    let mut s = session("corpus-e2e");
    // An axisymmetric section: the rules offer a fixed support first, a stress analyst restrains it with rollers.
    let (_, mut cyl) = fea_problem::templates::templates().into_iter().find(|(t, _)| t.contains("cylinder")).unwrap();
    cyl.supports.clear();
    cyl.loads.clear();
    s.problem = cyl;
    let rules = s.boundary_offer("bottom");
    assert_eq!(rules[0], BcKind::Fixed);

    assert!(entries(&s.advisor_items()).contains(&"Build training corpus (simulated prior)".to_string()));
    s.do_action(SolidAction::Advisor(AdvisorOp::BuildCorpus));
    assert!(s.status.contains("simulated") && s.status.contains("background"), "{}", s.status);
    wait_for_corpus(&mut s);
    assert!(s.advisors.boundary.bundle.as_ref().unwrap().report.summary("Boundary").contains("SIMULATED"));
    assert!(s.advisors.boundary.bundle.is_some() && s.advisors.constraints.bundle.is_some());
    // Nothing is switched on by building it.
    assert!(!s.advisors.boundary.settings.use_learned && s.boundary_offer("bottom") == rules);
    s.do_action(SolidAction::Advisor(AdvisorOp::ToggleLearned));
    assert!(s.advisors.boundary.settings.use_learned, "{}", s.status);
    let learned = s.boundary_offer("bottom");
    assert_eq!(learned[0], BcKind::Roller, "{learned:?}");
    let mut sorted = learned.clone();
    sorted.sort();
    let mut expected = rules.clone();
    expected.sort();
    assert_eq!(sorted, expected, "the learned model only reorders what the rules offered");
    // The panel says what the order is based on.
    let text = rows(&s);
    assert!(text.iter().any(|t| t.contains("simulated prior")), "{text:?}");
}

#[test]
fn your_own_recordings_keep_refining_the_model_while_you_work() {
    let mut s = session("corpus-online");
    s.do_action(SolidAction::Advisor(AdvisorOp::BuildCorpus));
    wait_for_corpus(&mut s);
    s.do_action(SolidAction::Advisor(AdvisorOp::ToggleRecord));
    assert!(s.advisors.boundary.settings.record && s.advisors.boundary.settings.auto_train);
    let before = s.advisors.boundary.bundle.as_ref().unwrap().report.real_sessions;
    // Fifty boundary choices in the real flow: popup, pick, undo (so the model stays comparable).
    for _ in 0..60 {
        select(&mut s, "left");
        let _ = s.boundary_items();
        s.do_action(SolidAction::BoundaryKind(BcKind::Fixed));
        s.do_action(SolidAction::Undo);
    }
    let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while s
        .advisors
        .boundary
        .bundle
        .as_ref()
        .unwrap()
        .report
        .real_sessions
        <= before
    {
        s.poll();
        assert!(std::time::Instant::now() < until);
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    let b = s.advisors.boundary.bundle.as_ref().unwrap();
    assert!(b.report.real_sessions > before, "refitted from the recordings: {}", b.report.summary("x"));
    assert!(s.advisors.last_fit.as_ref().is_some_and(|f| f.contains("Boundary")), "{:?}", s.advisors.last_fit);
}
