//! Real failures of a structural solve and what the user is told. These are the cases the guidance layer (ADR-020) is built
//! on, and each reason must stay specific: a rewrite of the message is a visible change here.
use fea_problem::*;

fn plate() -> Problem {
    templates::templates().into_iter().find(|(n, _)| *n == "Plate with a hole").unwrap().1
}
fn err(p: &Problem) -> String {
    solve(p, None).map(|_| "solved".to_string()).unwrap_err()
}

#[test]
fn a_model_without_supports_or_with_too_few_is_refused_with_how_many_motions_are_held() {
    let mut p = plate();
    p.supports.clear();
    assert!(err(&p).contains("free to move"));
    p.supports = vec![Support::roller("left", 0)];
    assert!(err(&p).contains("2 of 3 rigid-body motions are held"), "{}", err(&p));
}

#[test]
fn a_support_or_load_on_a_missing_edge_lists_the_names_that_exist() {
    let mut p = plate();
    p.loads = vec![Load::Pressure { edge: "nope".into(), p: 100.0 }];
    let e = err(&p);
    assert!(e.contains("'nope'") && e.contains("hole1") && e.contains("left"), "{e}");
    let mut p = plate();
    p.supports = vec![Support::fixed("nope")];
    assert!(err(&p).contains("'nope'"));
}

#[test]
fn a_mesh_that_is_far_too_fine_is_refused_with_the_element_forecast() {
    let mut p = plate();
    p.mesh.size = 1e-4;
    let e = err(&p);
    assert!(e.contains("too fine") && e.contains("limit 120000"), "{e}");
}

#[test]
fn a_non_finite_load_is_an_error_not_a_silent_zero_solution() {
    // Was `Ok` with zero displacement: the kernel's model check was never run by the problem layer.
    let mut p = plate();
    p.loads = vec![Load::Pressure { edge: "right".into(), p: f64::NAN }];
    assert!(err(&p).contains("not finite"), "{}", err(&p));
}

#[test]
fn nothing_applied_solves_but_says_the_solution_is_zero() {
    let mut p = plate();
    p.loads.clear();
    let s = solve(&p, None).unwrap();
    assert!(s.summary.notes.iter().any(|n| n.contains("the solution is zero")), "{:?}", s.summary.notes);
    // A loaded model carries no such caveat.
    assert!(!solve(&plate(), None).unwrap().summary.notes.iter().any(|n| n.contains("the solution is zero")));
}

// ---------------------------------------------------------------------------------------------- diagnosis (ADR-020)
use fea_problem::diagnose::{diagnose, FailureClass, RemedyKind};

/// The proof a remedy is right: apply it to the failing model and the solve that failed now succeeds.
fn solves_after(p: &Problem, edit: &fea_problem::diagnose::Edit) -> bool {
    let mut q = p.clone();
    edit.apply(&mut q).unwrap();
    solve(&q, None).is_ok()
}

#[test]
fn no_supports_is_diagnosed_before_the_solve_and_the_top_remedy_fixes_it() {
    let mut p = plate();
    p.supports.clear();
    let d = diagnose(&p, None);
    assert_eq!(d.len(), 1);
    assert_eq!(d[0].class, FailureClass::NoSupports);
    // The load is on the right: the fixed edge is the farthest one, the left. (Fixed alone constrains all motions.)
    assert!(d[0].remedies[0].label.contains("'left'"), "{}", d[0].remedies[0].label);
    assert_eq!(d[0].remedies[0].kind, RemedyKind::AddFixedSupport);
    assert!(solves_after(&p, &d[0].remedies[0].edit));
    // The question about intent offers every free outline surface, and each answer is a working support.
    let q = &d[0].questions[0];
    assert!(q.options.len() >= 3);
    assert!(q.options.iter().all(|(_, e)| solves_after(&p, e)));
}

#[test]
fn an_under_supported_model_is_diagnosed_from_the_failure_and_the_remedy_solves_it() {
    let mut p = plate();
    p.supports = vec![Support::roller("left", 0)];
    let e = err(&p);
    assert!(diagnose(&p, None).is_empty(), "the state alone does not show a missing motion: the prevention pass stays quiet");
    let d = diagnose(&p, Some(&e));
    assert_eq!(d[0].class, FailureClass::UnderConstrained);
    assert!(d[0].evidence[0].contains("left"), "{:?}", d[0].evidence);
    assert!(solves_after(&p, &d[0].remedies[0].edit), "{}", d[0].remedies[0].label);
}

#[test]
fn a_slip_in_a_surface_name_is_retargeted_and_an_unknown_one_can_be_dropped() {
    let mut p = plate();
    p.loads = vec![Load::Traction { edge: "rigth".into(), tx: 10000.0, ty: 0.0, tz: 0.0 }];
    let d = diagnose(&p, Some(&err(&p)));
    assert_eq!(d[0].class, FailureClass::UnknownSurface);
    assert_eq!(d[0].remedies[0].kind, RemedyKind::RetargetSurface);
    assert!(d[0].remedies[0].label.contains("'right'"));
    assert!(solves_after(&p, &d[0].remedies[0].edit));
    assert_eq!(d[0].remedies[1].kind, RemedyKind::DropReference);
    // Something unrelated to any surface has no retarget, only the drop.
    p.loads = vec![Load::Traction { edge: "zzzzzzzz".into(), tx: 1.0, ty: 0.0, tz: 0.0 }];
    let d = diagnose(&p, None);
    assert_eq!(d[0].remedies.len(), 1);
    assert_eq!(d[0].remedies[0].kind, RemedyKind::DropReference);
}

#[test]
fn a_mesh_over_the_limit_is_diagnosed_with_a_size_that_fits() {
    let mut p = plate();
    p.mesh.size = 1e-3;
    let d = diagnose(&p, Some(&err(&p)));
    assert_eq!(d[0].class, FailureClass::MeshTooFine);
    assert_eq!(d[0].remedies[0].kind, RemedyKind::CoarsenMesh);
    assert!(solves_after(&p, &d[0].remedies[0].edit));
}

#[test]
fn a_non_finite_load_is_diagnosed_in_the_prevention_pass() {
    let mut p = plate();
    p.loads = vec![Load::Pressure { edge: "right".into(), p: f64::NAN }];
    let d = diagnose(&p, None);
    assert_eq!(d[0].class, FailureClass::NonFiniteInput);
    // Removing the only load leaves a model that solves (to zero, with the "no load" caveat).
    assert!(solves_after(&p, &d[0].remedies[0].edit));
}

#[test]
fn a_healthy_model_has_no_findings_and_an_unrecognised_failure_keeps_its_message() {
    assert!(diagnose(&plate(), None).is_empty());
    let d = diagnose(&plate(), Some("the pivot went wrong in some way nobody wrote a rule for"));
    assert_eq!(d.len(), 1);
    assert_eq!(d[0].class, FailureClass::Other);
    assert!(d[0].remedies.is_empty() && d[0].summary.contains("nobody wrote a rule"));
    // Every template is healthy.
    for (n, t) in fea_problem::templates::templates() {
        assert!(diagnose(&t, None).is_empty(), "{n}: {:?}", diagnose(&t, None).iter().map(|d| d.summary.clone()).collect::<Vec<_>>());
    }
}

#[test]
fn a_hole_outside_the_outline_is_named_by_its_own_number_and_removing_it_fixes_the_solve() {
    let mut p = plate();
    p.geometry = Geometry::Sketch { outer: Shape::Rect { x0: 0.0, y0: 0.0, x1: 12.0, y1: 4.0 }, holes: vec![Shape::Circle { cx: 6.0, cy: 2.0, r: 0.5 }, Shape::Circle { cx: 20.0, cy: 2.0, r: 0.5 }], depth: 1.0, layers: 1, regions: Vec::new() };
    let e = err(&p);
    assert!(e.contains("hole 2 is not inside"), "numbered like the hole2 surface, not from zero: {e}");
    let d = diagnose(&p, Some(&e));
    assert_eq!(d[0].class, FailureClass::HoleOutsideOutline);
    assert!(d[0].summary.contains("Hole 2"));
    assert!(solves_after(&p, &d[0].remedies[0].edit));
    // Overlapping holes: the later one is named.
    p.geometry = Geometry::Sketch { outer: Shape::Rect { x0: 0.0, y0: 0.0, x1: 12.0, y1: 4.0 }, holes: vec![Shape::Circle { cx: 6.0, cy: 2.0, r: 0.8 }, Shape::Circle { cx: 6.5, cy: 2.0, r: 0.8 }], depth: 1.0, layers: 1, regions: Vec::new() };
    let d = diagnose(&p, None);
    assert_eq!(d[0].class, FailureClass::HoleOutsideOutline);
    assert!(d[0].summary.contains("Hole 2") && d[0].evidence[0].contains("overlaps hole 1"), "{:?}", d[0]);
    assert!(solves_after(&p, &d[0].remedies[0].edit));
}

#[test]
fn removing_a_hole_renumbers_what_referred_to_the_later_ones() {
    let mut p = templates::templates().into_iter().find(|(n, _)| n.contains("bushing")).unwrap().1;
    let Geometry::Sketch { holes, .. } = &mut p.geometry else { panic!() };
    let first = holes[0].clone();
    holes.insert(0, Shape::Circle { cx: 1.0, cy: 1.0, r: 0.1 });
    for b in &mut p.bushings {
        b.hole += 1;
    }
    for l in &mut p.loads {
        if let Load::Bearing { hole, .. } = l {
            *hole += 1;
        }
    }
    fea_problem::diagnose::Edit::RemoveHole(1).apply(&mut p).unwrap();
    let Geometry::Sketch { holes, .. } = &p.geometry else { panic!() };
    assert_eq!(holes[0], first);
    assert_eq!(p.bushings[0].hole, 1);
    assert!(p.loads.iter().any(|l| matches!(l, Load::Bearing { hole: 1, .. })));
}

#[test]
fn a_mesh_coarser_than_the_part_is_flagged_before_it_silently_gives_a_meaningless_answer() {
    let mut p = plate();
    p.mesh.size = 1e6;
    // It used to solve without a word.
    let d = diagnose(&p, None);
    assert_eq!(d[0].class, FailureClass::MeshTooCoarse);
    assert!(solves_after(&p, &d[0].remedies[0].edit));
    let mut q = p.clone();
    d[0].remedies[0].edit.apply(&mut q).unwrap();
    assert!(diagnose(&q, None).is_empty());
}

#[test]
fn an_edit_names_the_surfaces_it_would_touch_for_the_viewer_to_highlight() {
    use fea_problem::diagnose::{AdviceEdit, Edit, Target};
    let mut p = fea_problem::templates::templates().remove(0).1;
    p.supports = vec![fea_problem::Support::fixed("left")];
    p.loads = vec![fea_problem::Load::Pressure { edge: "right".into(), p: 1.0 }];
    let add = Edit::Advice(AdviceEdit::Boundary { surface: "top".into(), kind: fea_problem::advice::BcKind::Fixed });
    assert_eq!(add.surfaces(&p), ["top"]);
    assert_eq!(Edit::Drop(Target::Support(0)).surfaces(&p), ["left"]);
    assert_eq!(Edit::Drop(Target::Load(0)).surfaces(&p), ["right"]);
    assert_eq!(Edit::Retarget { target: Target::Load(0), to: "bottom".into() }.surfaces(&p), ["right", "bottom"], "from and to");
    assert_eq!(Edit::RemoveHole(1).surfaces(&p), ["hole1"]);
    assert!(Edit::Advice(AdviceEdit::MeshSize { size: 0.1 }).surfaces(&p).is_empty(), "the whole model, no surface");
    assert!(Edit::Drop(Target::Support(9)).surfaces(&p).is_empty(), "a stale index names nothing");
    // Previewing changes nothing.
    let before = p.clone();
    let _ = add.surfaces(&p);
    assert_eq!(p, before);
}
