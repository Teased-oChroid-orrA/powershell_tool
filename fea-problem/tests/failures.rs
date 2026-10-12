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

// ------------------------------------------------------------------------------------------ inverted elements of an imported mesh

const SQUARE_MSH: &str = "$MeshFormat\n2.2 0 8\n$EndMeshFormat\n$PhysicalNames\n2\n1 1 \"left\"\n1 2 \"right\"\n$EndPhysicalNames\n$Nodes\n4\n1 0 0 0\n2 1 0 0\n3 1 1 0\n4 0 1 0\n$EndNodes\n$Elements\n4\n1 1 2 1 1 4 1\n2 1 2 2 2 2 3\n3 2 2 0 1 1 2 3\n4 2 2 0 1 1 3 4\n$EndElements\n";
/// The same square with the second triangle written clockwise.
const SQUARE_BAD_MSH: &str = "$MeshFormat\n2.2 0 8\n$EndMeshFormat\n$PhysicalNames\n2\n1 1 \"left\"\n1 2 \"right\"\n$EndPhysicalNames\n$Nodes\n4\n1 0 0 0\n2 1 0 0\n3 1 1 0\n4 0 1 0\n$EndNodes\n$Elements\n4\n1 1 2 1 1 4 1\n2 1 2 2 2 2 3\n3 2 2 0 1 1 2 3\n4 2 2 0 1 1 4 3\n$EndElements\n";

fn imported_square() -> fea_problem::Problem {
    let mut p = fea_problem::templates::imported("square.msh");
    p.analysis = fea_problem::Analysis::PlaneStress;
    p.thickness = 1.0;
    p.supports = vec![fea_problem::Support::fixed("left")];
    p.loads = vec![fea_problem::Load::Traction { edge: "right".into(), tx: 10.0, ty: 0.0, tz: 0.0 }];
    p
}

#[test]
fn a_reversed_triangle_in_an_imported_mesh_is_diagnosed_and_the_fix_gives_the_answer_of_the_good_mesh() {
    use fea_problem::diagnose::{diagnose, FailureClass, RemedyKind};
    let mut p = imported_square();
    let good = fea_problem::solve(&p, Some(SQUARE_MSH)).unwrap();
    // The failure, as the user meets it.
    let e = fea_problem::solve(&p, Some(SQUARE_BAD_MSH)).unwrap_err();
    assert!(e.contains("inverted or degenerate"), "{e}");
    let found = diagnose(&p, Some(&e));
    let d = found.iter().find(|d| d.class == FailureClass::InvertedElements).expect("diagnosed");
    assert_eq!(d.remedies[0].kind, RemedyKind::ReorientMesh);
    assert!(d.evidence[0].contains("(0, 1)"), "the element is named: {:?}", d.evidence);
    // The top remedy fixes it, and the result is the good mesh's, to rounding.
    d.remedies[0].edit.apply(&mut p).unwrap();
    assert!(p.mesh.reorient);
    let fixed = fea_problem::solve(&p, Some(SQUARE_BAD_MSH)).unwrap();
    let (a, b) = (good.summary.strain_energy, fixed.summary.strain_energy);
    assert!((a - b).abs() < 1e-12 * a.abs(), "strain energy {a} vs {b}");
    assert!((good.summary.max_displacement.value - fixed.summary.max_displacement.value).abs() < 1e-12);
    // A good mesh is not touched by the option, and the option survives a save.
    let again = fea_problem::solve(&p, Some(SQUARE_MSH)).unwrap();
    assert!((again.summary.strain_energy - a).abs() < 1e-12 * a.abs());
    assert!(Problem::from_json(&p.to_json().unwrap()).unwrap().mesh.reorient);
    // A collapsed element is beyond the repair: the model says so, and offers no fix that cannot work.
    let collapsed = SQUARE_BAD_MSH.replace("4 2 2 0 1 1 4 3", "4 2 2 0 1 1 3 3");
    let e = fea_problem::solve(&p, Some(&collapsed)).unwrap_err();
    assert!(e.contains("cannot be turned right side out"), "{e}");
    let d = diagnose(&p, Some(&e)).into_iter().find(|d| d.class == FailureClass::InvertedElements).unwrap();
    assert!(d.remedies.is_empty() && d.summary.contains("collapsed or folded"), "{d:?}");
}

// ------------------------------------------------------------------------------------------ contact that does not converge

fn bushed_lug() -> fea_problem::Problem {
    let mut p = fea_problem::templates::templates().into_iter().find(|(n, _)| n.contains("bushing")).unwrap().1;
    p.mesh.size = 0.8;
    p.mesh.hole_factor = 0.5;
    p
}

/// The failure as the solver reports it (the exact text a real stalled solve gave, see the ignored test below), the diagnosis,
/// and the remedy applied: fast enough for every run.
#[test]
fn a_bushing_with_a_units_slip_in_its_modulus_stalls_the_contact_and_is_diagnosed() {
    use fea_problem::diagnose::{diagnose, FailureClass, RemedyKind};
    let mut p = bushed_lug();
    p.bushings[0].material.e = 1.0; // 1 where the plate is 1.03e7: a units slip
    let e = "the loaded bushing could not be solved: NoConvergence(\"stagnated (residual 2.083e-5, best 2.098e-5)\")";
    let found = diagnose(&p, Some(e));
    let d = found.iter().find(|d| d.class == FailureClass::ContactNotConverged).expect("diagnosed");
    assert!(d.evidence[0].contains("ratio 9.7e-8"), "{:?}", d.evidence);
    assert_eq!(d.remedies[0].kind, RemedyKind::MatchBushingModulus);
    assert_eq!(d.remedies[0].edit.surfaces(&p), ["hole1"]);
    d.remedies[0].edit.apply(&mut p).unwrap();
    assert_eq!(p.bushings[0].material.e, p.material.e);
    // A stall with a sensible modulus gets the evidence and no invented remedy.
    let normal = bushed_lug();
    let d = diagnose(&normal, Some("the loaded bushing could not be solved: NoConvergence(\"stagnated\")")).into_iter().find(|d| d.class == FailureClass::ContactNotConverged).unwrap();
    assert!(d.remedies.is_empty() && !d.evidence.is_empty());
    // And a plate without a bushing is not a contact finding at all.
    let mut plain = normal.clone();
    plain.bushings.clear();
    assert!(diagnose(&plain, Some("could not be solved")).iter().all(|d| d.class != FailureClass::ContactNotConverged));
}

/// The remedy fixes the failure: solving the repaired model installs the fit. A contact solve takes minutes in a debug
/// build: `cargo test --release -p fea-problem --test failures -- --ignored`.
#[test]
#[ignore = "slow in a debug build: run with --release --ignored"]
fn a_real_stalled_contact_solve_is_diagnosed_and_applying_the_top_fix_makes_the_model_solve() {
    use fea_problem::diagnose::{diagnose, FailureClass};
    let mut p = bushed_lug();
    p.bushings[0].material.e = 1.0;
    let e = fea_problem::solve(&p, None).unwrap_err();
    assert!(e.contains("could not be solved"), "{e}");
    let d = diagnose(&p, Some(&e)).into_iter().find(|d| d.class == FailureClass::ContactNotConverged).unwrap();
    d.remedies[0].edit.apply(&mut p).unwrap();
    let s = fea_problem::solve(&p, None).unwrap();
    assert!(!s.summary.interfaces.is_empty() && s.summary.interfaces[0].mean_pressure > 0.0, "the fit is installed");
}

// ------------------------------------------------------------------------------------------ a feature too thin to mesh

fn plate_with_a_hair_slot() -> fea_problem::Problem {
    let mut p = fea_problem::templates::templates().remove(0).1;
    if let Geometry::Sketch { holes, .. } = &mut p.geometry {
        *holes = vec![Shape::Slot { cx: 6.0, cy: 2.0, length: 2.0, width: 1e-5, angle: 0.0 }];
    }
    // The adaptive passes (on by default) are what chase the sliver: without them the same mesh solves.
    assert!(p.mesh.adapt_passes > 0);
    p
}

#[test]
fn a_hair_thin_slot_fails_fast_with_the_reason_and_each_fix_makes_the_model_solve() {
    use fea_problem::diagnose::{diagnose, FailureClass, RemedyKind};
    let p = plate_with_a_hair_slot();
    let t = std::time::Instant::now();
    let e = fea_problem::solve(&p, None).unwrap_err();
    assert!(e.contains("mesh exceeds"), "{e}");
    assert!(t.elapsed().as_secs() < 120, "the mesher's budget ends the refinement in seconds, not minutes: {:?}", t.elapsed());
    let found = diagnose(&p, Some(&e));
    let d = found.iter().find(|d| d.class == FailureClass::ThinFeature).expect("diagnosed");
    assert!(d.evidence[0].contains("hole 1"), "{:?}", d.evidence);
    assert_eq!(d.remedies.iter().map(|r| r.kind).collect::<Vec<_>>(), [RemedyKind::WidenHole, RemedyKind::RemoveHole]);
    for r in &d.remedies {
        let mut fixed = p.clone();
        r.edit.apply(&mut fixed).unwrap();
        assert!(fea_problem::solve(&fixed, None).is_ok(), "{}", r.label);
    }
    // A model whose mesh is merely big is not blamed on a feature.
    let mut big = fea_problem::templates::templates().remove(0).1;
    big.mesh.size = 0.001;
    assert!(diagnose(&big, Some("mesh exceeds 250000 vertices (size function too small?)")).iter().all(|d| d.class != FailureClass::ThinFeature));
}

// ------------------------------------------------------------------------------------------ a modulus in the wrong unit

#[test]
fn a_modulus_a_thousand_times_too_small_solves_but_is_flagged_and_the_answer_restores_it() {
    use fea_problem::diagnose::{diagnose_result, FailureClass};
    let good = fea_problem::templates::templates().remove(0).1;
    let solved_good = fea_problem::solve(&good, None).unwrap();
    assert!(diagnose_result(&solved_good).is_empty(), "a sound design is not flagged");
    // ksi typed where psi is meant: the plate is a thousand times too soft and stretches by far more than its own size.
    let mut slip = good.clone();
    slip.material.e /= 1000.0;
    let solved = fea_problem::solve(&slip, None).unwrap();
    let found = diagnose_result(&solved);
    let d = found.iter().find(|d| d.class == FailureClass::ScaleSuspect).expect("flagged");
    assert!(d.summary.contains("% of the part's size") && d.remedies.is_empty());
    let (label, edit) = &d.questions[0].options[0];
    assert!(label.contains("1000 times too small"), "{label}");
    edit.apply(&mut slip).unwrap();
    assert!((slip.material.e / good.material.e - 1.0).abs() < 1e-12);
    let again = fea_problem::solve(&slip, None).unwrap();
    assert!(diagnose_result(&again).is_empty() && (again.summary.max_displacement.value / solved_good.summary.max_displacement.value - 1.0).abs() < 1e-9);
    // A thermal-only study has no displacements to judge.
    let mut th = good.clone();
    th.study = fea_problem::Study::Thermal;
    th.thermal.supports = vec![fea_problem::HeatSupport::Temperature { edge: "left".into(), t: 0.0 }];
    assert!(diagnose_result(&fea_problem::solve(&th, None).unwrap()).is_empty());
}

#[test]
fn a_bushed_model_gets_the_kernels_model_check_before_the_contact_iteration() {
    // A NaN load used to be discovered (or not) deep in the contact iteration; the check now refuses it at once, with its reason.
    let mut p = bushed_lug();
    p.loads = vec![fea_problem::Load::Traction { edge: "right".into(), tx: f64::NAN, ty: 0.0, tz: 0.0 }];
    let t = std::time::Instant::now();
    let e = fea_problem::solve(&p, None).unwrap_err();
    assert!(e.contains("not finite"), "{e}");
    assert!(t.elapsed().as_secs() < 30, "refused before any contact work: {:?}", t.elapsed());
    // The held-by-contact bushing and a fit without a load are not findings.
    let mut ok = bushed_lug();
    ok.loads.clear();
    ok.bushings[0].material.e = ok.material.e;
    assert!(fea_problem::solve(&ok, None).is_ok());
}
