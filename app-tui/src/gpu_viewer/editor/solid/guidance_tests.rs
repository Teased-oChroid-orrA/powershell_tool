//! Failure guidance through the solid session (ADR-020): the card, the status-bar fix, applying and undoing a fix,
//! recording the choice, and refusing a stale index.
use super::*;
use crate::gpu_viewer::editor::authoring::{Authoring, Relevance};
use fea_problem::Support;

fn temp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("solid-guidance-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}
/// The plate template with a load and without supports, advisors in a private folder.
fn session(name: &str) -> SolidSession {
    let mut p = fea_problem::templates::templates().remove(0).1;
    p.supports.clear();
    p.mesh.size = 0.5;
    let mut s = SolidSession::new(p, temp(name).join("model.json"));
    s.advisors = advising::Advisors::in_dir(temp(&format!("{name}-advisor")));
    s
}
fn card(s: &SolidSession) -> Option<crate::gpu_viewer::editor::authoring::Card> {
    Authoring::inspector(s).into_iter().find(|c| c.id == "guidance")
}
fn text(c: &crate::gpu_viewer::editor::authoring::Card) -> String {
    c.items.iter().filter_map(|i| i.plain_text()).collect::<Vec<_>>().join("\n")
}
fn wait_failed(s: &mut SolidSession) {
    let t = std::time::Instant::now();
    while s.solve_worker.is_busy() || s.failure.is_none() && s.solved.is_none() {
        s.poll();
        assert!(t.elapsed().as_secs() < 120, "{}", s.status);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    s.poll();
}

#[test]
fn a_model_without_supports_gets_a_quiet_check_card_and_the_fix_is_one_undoable_edit() {
    let mut s = session("prevent");
    let c = card(&s).expect("a model without supports is flagged while it is built");
    assert_eq!((c.title.as_str(), c.relevance), ("Check", Relevance::Context));
    assert!(text(&c).contains("Fix 'left'") && text(&c).contains("order: rules"), "{}", text(&c));
    assert!(s.status_fix_action().is_none(), "no red failure, no button");
    s.do_action(SolidAction::Remedy(0, 0));
    assert!(s.problem.supports.contains(&Support::fixed("left")), "{}", s.status);
    assert!(card(&s).is_none(), "the finding is gone once it is fixed");
    s.do_action(SolidAction::Undo);
    assert!(s.problem.supports.is_empty() && card(&s).is_some());
}

#[test]
fn a_failed_solve_makes_the_card_primary_and_offers_the_best_fix_in_the_status_bar() {
    let mut s = session("failed");
    s.problem.supports = vec![Support::roller("left", 0)];
    s.do_action(SolidAction::Solve);
    wait_failed(&mut s);
    assert!(s.failure.as_deref().is_some_and(|f| f.contains("rigid-body")), "{:?} {}", s.failure, s.status);
    let c = card(&s).unwrap();
    assert_eq!((c.title.as_str(), c.relevance), ("Solve failed", Relevance::Primary));
    assert!(text(&c).contains("Which surface is held"), "{}", text(&c));
    let (label, action) = s.status_fix_action().expect("a fix for the status bar");
    assert!(label.starts_with("Fix '"), "{label}");
    // Run the status-bar action itself, then solve again: the failure is gone.
    let crate::gpu_viewer::controller::UiAction::Editor(crate::gpu_viewer::editor::session::EditorAction::Solid(a)) = action else { panic!("{action:?}") };
    s.do_action(a);
    assert!(s.failure.is_none() && s.status.contains("Solve again"), "{}", s.status);
    s.do_action(SolidAction::Solve);
    wait_failed(&mut s);
    assert!(s.failure.is_none() && s.solved.is_some(), "{}", s.status);
}

#[test]
fn applying_a_fix_records_the_choice_only_when_recording_is_on_and_a_stale_index_is_refused() {
    let mut s = session("record");
    s.do_action(SolidAction::Remedy(5, 0));
    assert!(s.status.contains("no longer applies") && s.problem.supports.is_empty());
    s.do_action(SolidAction::Remedy(0, 0));
    assert!(!s.advisors.guidance.store.as_ref().unwrap().has_recordings(), "off by default");
    s.do_action(SolidAction::Undo);
    s.advisors.guidance.set_record(true).unwrap();
    s.do_action(SolidAction::Remedy(0, 0));
    assert!(s.advisors.guidance.store.as_ref().unwrap().has_recordings());
    // An answer to a question is an edit too.
    s.do_action(SolidAction::Undo);
    s.do_action(SolidAction::Answer(0, 0, 1));
    assert_eq!(s.problem.supports.len(), 1, "{}", s.status);
}

#[test]
fn nothing_is_flagged_for_an_empty_part_or_while_sketching() {
    let mut s = session("empty");
    s.do_action(SolidAction::NewPart);
    assert!(card(&s).is_none(), "{:?}", card(&s).map(|c| text(&c)));
}

/// STEP workflow: open, edit by menu action, pick a face with either CAD kernel.
#[test]
fn a_step_solid_opens_gets_a_hole_and_a_dangling_support_is_diagnosed() {
    let mut s = session("step");
    let dir = temp("step-files");
    std::fs::create_dir_all(&dir).unwrap();
    #[cfg(not(feature = "occt"))]
    {
        use super::stepops::PromptOp;
        // A file that is no solid is refused with the reason, and no edit is offered on the unchanged model.
        let path = dir.join("part.step");
        std::fs::write(&path, "ISO-10303-21;").unwrap();
        s.open_path(path.to_str().unwrap());
        assert!(s.status.contains("Could not open"), "{}", s.status);
        assert!(s.solid_op_availability(PromptOp::Fillet).unwrap_err().contains("not a STEP solid"));
        // The native pipeline opens a cube, cuts a hole in its top face, and undo restores it.
        let step = solid_kernel::native_brep::NativeBrep::box_from_corners([0.0, 0.0, 0.0], [10.0, 10.0, 10.0]).unwrap().to_step_text();
        let path = dir.join("cube.step");
        std::fs::write(&path, step).unwrap();
        s.open_path(path.to_str().unwrap());
        assert!(matches!(s.problem.geometry, Geometry::Solid { .. }) && s.status.contains("6 faces"), "{}", s.status);
        let top = (1..=6).find(|f| fea_problem::solid::flat_face(match &s.problem.geometry { Geometry::Solid { cache: Some(c), .. } => c, _ => unreachable!() }, *f).is_ok_and(|(c, _)| (c[2] - 10.0).abs() < 1e-6)).unwrap();
        assert!(s.solid_op_availability(PromptOp::Hole).is_err(), "no face picked yet");
        s.selected = Some(format!("face{top}"));
        s.begin_solid_op(PromptOp::Hole);
        assert!(s.prompt.is_some(), "{}", s.status);
        s.prompt.as_mut().unwrap().text = "2".into();
        s.commit_prompt();
        assert_eq!(s.problem.edge_names().len(), 7, "{}", s.status);
        s.do_action(SolidAction::Undo);
        assert_eq!(s.problem.edge_names().len(), 6);
    }
    #[cfg(feature = "occt")]
    {
        use super::stepops::PromptOp;
        let step = solid_kernel::occt::Brep::box_from_corners([0.0, 0.0, 0.0], [10.0, 10.0, 10.0]).to_step_text().unwrap();
        let path = dir.join("cube.step");
        std::fs::write(&path, step).unwrap();
        s.open_path(path.to_str().unwrap());
        assert!(matches!(s.problem.geometry, Geometry::Solid { .. }) && s.status.contains("6 faces"), "{}", s.status);
        // A flat face is required for a hole; pick the top (the one whose middle is at z = 10).
        let top = (1..=6).find(|f| fea_problem::solid::flat_face(match &s.problem.geometry { Geometry::Solid { cache: Some(c), .. } => c, _ => unreachable!() }, *f).is_ok_and(|(c, _)| (c[2] - 10.0).abs() < 1e-6)).unwrap();
        // The solid meshes for the viewport: its faces are the pickable surfaces face1..face6.
        s.do_action(SolidAction::GenerateMesh);
        let t = std::time::Instant::now();
        while s.preview.is_none() {
            s.poll();
            assert!(t.elapsed().as_secs() < 120, "{}", s.status);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let preview = s.preview.as_ref().unwrap();
        assert!(preview.mesh.n_elems() > 50, "{}", s.status);
        assert_eq!(s.names(), (1..=6).map(|i| format!("face{i}")).collect::<Vec<_>>());
        assert!(s.solid_op_availability(PromptOp::Hole).is_err(), "no face picked yet");
        s.selected = Some(format!("face{top}"));
        s.begin_solid_op(PromptOp::Hole);
        assert!(s.prompt.is_some());
        s.prompt.as_mut().unwrap().text = "2".into();
        s.commit_prompt();
        assert_eq!(s.problem.edge_names().len(), 7, "{}", s.status);
        s.do_action(SolidAction::Undo);
        assert_eq!(s.problem.edge_names().len(), 6);
        // A support on a face number that a fillet removes is found by the diagnosis, with the retarget offered.
        s.problem.supports = vec![Support::fixed("face30")];
        let kinds: Vec<_> = s.findings().iter().map(|d| d.class).collect();
        assert!(kinds.contains(&fea_problem::diagnose::FailureClass::UnknownSurface), "{kinds:?}");
    }
}

#[test]
fn answering_a_question_is_recorded_as_a_choice_against_the_fixes_and_the_rules_put_the_fixes_first() {
    use fea_problem::diagnose::{FailureClass, RemedyKind};
    let mut s = session("ask");
    // Rules: fixes first, then the question.
    let t = text(&card(&s).unwrap());
    assert!(t.find("Fixes").unwrap() < t.find("Which surface is held").unwrap(), "{t}");
    s.advisors.guidance.set_record(true).unwrap();
    s.do_action(SolidAction::Answer(0, 0, 1));
    s.do_action(SolidAction::Undo);
    s.do_action(SolidAction::Remedy(0, 0));
    let rec = s.advisors.guidance.store.as_ref().unwrap().read::<FailureClass, RemedyKind>().unwrap();
    assert_eq!(rec.len(), 2);
    for r in &rec {
        assert_eq!(r.situation, FailureClass::NoSupports);
        assert!(r.offered.contains(&RemedyKind::AnswerQuestion) && r.offered.contains(&RemedyKind::AddFixedSupport), "{:?}", r.offered);
    }
    assert_eq!(rec[0].taken, Some(RemedyKind::AnswerQuestion), "the answer");
    assert_eq!(rec[1].taken, Some(RemedyKind::AddFixedSupport), "the fix");
    // The decision is a function of the order: asking leads only when it ranks above the first fix.
    assert!(super::guidance::ask_first_in(&[RemedyKind::AnswerQuestion, RemedyKind::AddFixedSupport]));
    assert!(!super::guidance::ask_first_in(&[RemedyKind::AddFixedSupport, RemedyKind::AnswerQuestion]));
    assert!(!super::guidance::ask_first_in(&[RemedyKind::AnswerQuestion]), "nothing to compare with");
    assert!(!super::guidance::ask_first_in(&[RemedyKind::AddFixedSupport]));
}

#[test]
fn pointing_at_a_fix_marks_the_surfaces_it_touches_without_applying_it() {
    let mut s = session("hover");
    let before = s.problem.clone();
    assert!(s.fix_preview_surfaces().is_empty());
    s.do_action(SolidAction::PreviewFix(Some(FixRef::Remedy(0, 0))));
    assert_eq!(s.fix_preview_surfaces(), ["left"]);
    s.do_action(SolidAction::PreviewFix(Some(FixRef::Answer(0, 0, 1))));
    assert_eq!(s.fix_preview_surfaces().len(), 1);
    // A stale reference marks nothing; leaving clears; nothing was applied.
    s.do_action(SolidAction::PreviewFix(Some(FixRef::Remedy(7, 7))));
    assert!(s.fix_preview_surfaces().is_empty());
    s.do_action(SolidAction::PreviewFix(None));
    assert_eq!(s.problem, before);
    // Applying a fix clears the preview.
    s.do_action(SolidAction::PreviewFix(Some(FixRef::Remedy(0, 0))));
    s.do_action(SolidAction::Remedy(0, 0));
    assert!(s.fix_preview.is_none());
}

#[test]
fn a_solved_model_that_deforms_absurdly_gets_the_units_question_in_the_check_card() {
    let mut s = session("scale");
    s.problem.supports = vec![Support::fixed("left")];
    s.problem.material.e /= 1000.0;
    s.problem.mesh.size = 1.0;
    s.do_action(SolidAction::Solve);
    let t = std::time::Instant::now();
    while s.solve_worker.is_busy() || s.solved.is_none() {
        s.poll();
        assert!(t.elapsed().as_secs() < 120, "{}", s.status);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    s.poll();
    let c = card(&s).expect("a finding from the result");
    assert!(text(&c).contains("Implausibly large deformation") && text(&c).contains("different unit"), "{}", text(&c));
    // Answering restores the modulus in one undoable edit, and the result is no longer current.
    s.do_action(SolidAction::Answer(0, 0, 0));
    assert!((s.problem.material.e / (plate_modulus()) - 1.0).abs() < 1e-12, "{}", s.problem.material.e);
    assert!(card(&s).is_none() || !text(&card(&s).unwrap()).contains("Implausibly"));
}
fn plate_modulus() -> f64 {
    fea_problem::templates::templates().remove(0).1.material.e
}
