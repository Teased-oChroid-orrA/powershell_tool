//! Member problem: closed-form agreement, frame invariance, atomic edits, refusals.
use fea_problem::members::*;

fn rel(a: f64, b: f64) -> f64 {
    (a / b - 1.0).abs()
}

#[test]
fn tripod_template_matches_closed_form_and_balances() {
    let p = templates().remove(0);
    let out = p.solve().unwrap();
    let apex = p.nodes.len() - 1;
    let (r, h, e, a, load) = (20.0_f64, 15.0_f64, 1.0e7, 0.5, 1000.0);
    let l = r.hypot(h);
    let exact = -load * l.powi(3) / (3.0 * e * a * h * h);
    assert!(rel(out.displacement[apex][2], exact) < 1e-10, "{} vs {exact}", out.displacement[apex][2]);
    for m in &out.members {
        assert!(rel(m.axial_force, -load * l / (3.0 * h)) < 1e-10, "member force {}", m.axial_force);
        assert_eq!(m.max_bending_moment, 0.0);
    }
    assert!(out.acceptance.force_balance < 1e-12 && out.acceptance.moment_balance < 1e-12, "{:?}", out.acceptance);
}

fn cantilever(p: &MemberProblem) -> (u32, f64, SectionSpec) {
    let m = &p.members[0];
    let MemberKind::Frame { section, .. } = m.kind else { panic!() };
    (m.nodes[1], 50.0, section)
}

#[test]
fn cantilever_template_matches_timoshenko_tip_deflection() {
    let p = templates().remove(1);
    let (tip, l, s) = cantilever(&p);
    let out = p.solve().unwrap();
    let idx = p.nodes.iter().position(|n| n.id == tip).unwrap();
    let exact = 100.0 * l.powi(3) / (3.0 * s.e * s.iz) + 100.0 * l / (s.g * s.shear_areas.unwrap()[0]);
    assert!(rel(out.displacement[idx][1], exact) < 1e-10);
    // Root bending moment equals P L; reaction equals the load.
    assert!(rel(out.members[0].max_bending_moment, 100.0 * l) < 1e-9);
    assert!(rel(out.reaction[0][1], -100.0) < 1e-10);
    assert!(out.strain_energy > 0.0);
}

#[test]
fn uniform_load_cantilever_matches_closed_form_and_balances() {
    let mut p = templates().remove(1);
    let q = 2.0;
    let id = p.members[0].id;
    let MemberKind::Frame { section, reference, .. } = p.members[0].kind else { panic!() };
    p.apply(Edit::SetKind { id, kind: MemberKind::Frame { section, reference, uniform_load: [0.0, q, 0.0] } }).unwrap();
    p.apply(Edit::SetLoad { node: p.nodes[1].id, force: [0.0; 3], moment: [0.0; 3] }).unwrap();
    let out = p.solve().unwrap();
    let l = 50.0_f64;
    let exact = q * l.powi(4) / (8.0 * section.e * section.iz) + q * l * l / (2.0 * section.g * section.shear_areas.unwrap()[0]);
    assert!(rel(out.displacement[1][1], exact) < 1e-9, "{} vs {exact}", out.displacement[1][1]);
    assert!(rel(out.reaction[0][1], -q * l) < 1e-10);
    assert!(out.acceptance.force_balance < 1e-10 && out.acceptance.moment_balance < 1e-10);
}

#[test]
fn rotating_and_translating_the_model_preserves_the_physics() {
    let base = templates().remove(1);
    let reference = base.solve().unwrap();
    // Rotate 90 degrees about z and translate far from the origin; the load rotates with the member.
    let mut moved = MemberProblem::new("moved");
    let shift = [1000.0, -500.0, 200.0];
    let ids: Vec<u32> = base
        .nodes
        .iter()
        .map(|n| moved.apply(Edit::AddNode { pos: [-n.pos[1] + shift[0], n.pos[0] + shift[1], n.pos[2] + shift[2]] }).unwrap().unwrap())
        .collect();
    let MemberKind::Frame { section, .. } = base.members[0].kind else { panic!() };
    // Member now runs along +y; keep the reference direction rotated too: (0,1,0) -> (-1,0,0).
    moved.apply(Edit::AddMember { a: ids[0], b: ids[1], kind: MemberKind::Frame { section, reference: [-1.0, 0.0, 0.0], uniform_load: [0.0; 3] } }).unwrap();
    moved.apply(Edit::SetSupport { node: ids[0], fixed: [true; 6] }).unwrap();
    moved.apply(Edit::SetLoad { node: ids[1], force: [-100.0, 0.0, 0.0], moment: [0.0; 3] }).unwrap();
    let out = moved.solve().unwrap();
    assert!(rel(out.max_displacement(), reference.max_displacement()) < 1e-9);
    assert!(out.acceptance.force_balance < 1e-9 && out.acceptance.moment_balance < 1e-9, "{:?}", out.acceptance);
    assert!(rel(out.members[0].max_bending_moment, reference.members[0].max_bending_moment) < 1e-9);
}

#[test]
fn edits_are_atomic_and_cascade() {
    let mut p = templates().remove(0);
    let before = p.clone();
    // Invalid edits leave the problem untouched.
    assert!(p.apply(Edit::AddMember { a: 1, b: 1, kind: MemberKind::Truss { e: 1.0, area: 1.0 } }).is_err());
    assert!(p.apply(Edit::AddMember { a: 1, b: 99, kind: MemberKind::Truss { e: 1.0, area: 1.0 } }).is_err());
    assert!(p.apply(Edit::AddMember { a: 1, b: 4, kind: MemberKind::Truss { e: 1.0, area: 1.0 } }).is_err(), "duplicate pair");
    assert!(p.apply(Edit::AddMember { a: 1, b: 2, kind: MemberKind::Truss { e: -1.0, area: 1.0 } }).is_err());
    assert!(p.apply(Edit::AddNode { pos: [f64::NAN, 0.0, 0.0] }).is_err());
    assert!(p.apply(Edit::AddNode { pos: p.nodes[0].pos }).is_err(), "coincident node");
    assert!(p.apply(Edit::MoveNode { id: 1, pos: p.nodes[1].pos }).is_err());
    assert!(p.apply(Edit::RemoveNode { id: 12345 }).is_err());
    assert_eq!(p, before);
    // Removing a node removes its members, support and load; ids are never reused.
    let apex = p.nodes.last().unwrap().id;
    p.apply(Edit::RemoveNode { id: apex }).unwrap();
    assert!(p.members.is_empty() && p.loads.is_empty());
    assert_eq!(p.supports.len(), 3);
    let fresh = p.apply(Edit::AddNode { pos: [1.0, 2.0, 3.0] }).unwrap().unwrap();
    assert!(fresh > apex);
    p.validate().unwrap();
}

#[test]
fn rotational_actions_require_a_frame_member() {
    let mut p = templates().remove(0);
    let apex = p.nodes.last().unwrap().id;
    assert!(p.apply(Edit::SetSupport { node: apex, fixed: [false, false, false, true, false, false] }).is_err());
    assert!(p.apply(Edit::SetLoad { node: apex, force: [0.0; 3], moment: [0.0, 1.0, 0.0] }).is_err());
    // Replacing the only frame member by a truss drops rotational supports and moments instead of leaving an invalid model.
    let mut f = templates().remove(1);
    let (tip, ..) = cantilever(&f);
    f.apply(Edit::SetLoad { node: tip, force: [0.0, 1.0, 0.0], moment: [0.0, 0.0, 5.0] }).unwrap();
    let id = f.members[0].id;
    f.apply(Edit::SetKind { id, kind: MemberKind::Truss { e: 1e7, area: 1.0 } }).unwrap();
    assert!(f.supports[0].fixed[3..].iter().all(|v| !v));
    assert_eq!(f.load(tip).unwrap().moment, [0.0; 3]);
    f.validate().unwrap();
}

#[test]
fn degenerate_geometry_and_references_are_refused() {
    let mut p = templates().remove(1);
    let m = p.members[0].clone();
    let MemberKind::Frame { section, .. } = m.kind else { panic!() };
    // reference parallel to the member
    assert!(p.apply(Edit::SetKind { id: m.id, kind: MemberKind::Frame { section, reference: [1.0, 0.0, 0.0], uniform_load: [0.0; 3] } }).unwrap_err().contains("parallel"));
    assert!(p.apply(Edit::SetKind { id: m.id, kind: MemberKind::Frame { section, reference: [0.0; 3], uniform_load: [0.0; 3] } }).is_err());
    assert!(p.apply(Edit::MoveNode { id: m.nodes[1], pos: [f64::INFINITY, 0.0, 0.0] }).is_err());
}

#[test]
fn mechanisms_and_missing_supports_are_rejected_not_solved() {
    let mut p = templates().remove(0);
    for s in p.supports.clone() {
        p.apply(Edit::SetSupport { node: s.node, fixed: [false; 6] }).unwrap();
    }
    assert!(p.solve().is_err(), "unsupported structure");
    let mut q = templates().remove(1);
    q.apply(Edit::SetSupport { node: q.nodes[0].id, fixed: [true, true, true, false, false, false] }).unwrap();
    assert!(q.solve().is_err(), "a pinned cantilever is a mechanism");
    assert!(MemberProblem::new("empty").solve().is_err());
}

#[test]
fn json_round_trip_and_version_gate() {
    let p = templates().remove(1);
    let text = serde_json::to_string(&p).unwrap();
    let back: MemberProblem = serde_json::from_str(&text).unwrap();
    assert_eq!(back, p);
    back.validate().unwrap();
    let mut bad = back;
    bad.version = 99;
    assert!(bad.validate().unwrap_err().contains("version"));
    // A hand-edited file with a dangling reference is refused by validate, not by a panic in solve.
    let mut dangling = p.clone();
    dangling.members[0].nodes[1] = 4242;
    assert!(dangling.validate().is_err() && dangling.solve().is_err());
}

fn two_bar() -> MemberProblem {
    let mut p = MemberProblem::new("two-bar");
    let a = p.apply(Edit::AddNode { pos: [-25.0, 0.0, 0.0] }).unwrap().unwrap();
    let b = p.apply(Edit::AddNode { pos: [25.0, 0.0, 0.0] }).unwrap().unwrap();
    let c = p.apply(Edit::AddNode { pos: [0.0, 30.0, 0.0] }).unwrap().unwrap();
    for n in [a, b] {
        p.apply(Edit::SetSupport { node: n, fixed: [true, true, true, false, false, false] }).unwrap();
        p.apply(Edit::AddMember { a: n, b: c, kind: MemberKind::Truss { e: 1.0e7, area: 1.0 } }).unwrap();
    }
    p.apply(Edit::SetLoad { node: c, force: [0.0, -100.0, 0.0], moment: [0.0; 3] }).unwrap();
    p
}

#[test]
fn planar_flag_removes_the_out_of_plane_mechanism_and_matches_the_closed_form() {
    let mut p = two_bar();
    // In 3D the pinned flat truss can still rotate about the line through its supports.
    let err = p.solve().unwrap_err();
    assert!(err.contains("rigid motions"), "{err}");
    p.apply(Edit::SetPlane { axis: Some(2) }).unwrap();
    let out = p.solve().unwrap();
    let theta = 30.0_f64.atan2(25.0);
    assert!(rel(out.members[0].axial_force.abs(), 100.0 / (2.0 * theta.sin())) < 1e-10);
    let l = 25.0_f64.hypot(30.0);
    let exact = 100.0 / (2.0 * theta.sin()) * l / (1.0e7 * 1.0) / theta.sin();
    assert!(rel(out.displacement[2][1].abs(), exact) < 1e-10, "{} vs {exact}", out.displacement[2][1]);
    assert_eq!(out.displacement[2][2], 0.0);
    assert!(out.acceptance.force_balance < 1e-10);
    // Back to 3D restores the refusal.
    p.apply(Edit::SetPlane { axis: None }).unwrap();
    assert!(p.solve().is_err());
}

#[test]
fn planar_flag_is_validated_against_geometry_and_loads() {
    let mut p = two_bar();
    p.apply(Edit::SetPlane { axis: Some(2) }).unwrap();
    // an off-plane node, an out-of-plane force and a bad axis are refused atomically
    assert!(p.apply(Edit::AddNode { pos: [0.0, 0.0, 5.0] }).unwrap_err().contains("planar"));
    let c = p.nodes[2].id;
    assert!(p.apply(Edit::SetLoad { node: c, force: [0.0, -100.0, 3.0], moment: [0.0; 3] }).unwrap_err().contains("out-of-plane"));
    assert!(p.apply(Edit::SetPlane { axis: Some(7) }).is_err());
    assert_eq!(p.plane, Some(2));
    // switching to a plane the nodes are not in is refused too
    assert!(p.apply(Edit::SetPlane { axis: Some(0) }).is_err());
    p.apply(Edit::SetPlane { axis: Some(1) }).unwrap_err();
}

#[test]
fn planar_frame_matches_the_three_dimensional_solution() {
    let base = templates().remove(1);
    let reference = base.solve().unwrap();
    let mut planar = base.clone();
    planar.apply(Edit::SetPlane { axis: Some(2) }).unwrap();
    let out = planar.solve().unwrap();
    assert!(rel(out.max_displacement(), reference.max_displacement()) < 1e-12);
    // Old files without the field still load as 3D.
    let mut v: serde_json::Value = serde_json::to_value(&base).unwrap();
    v.as_object_mut().unwrap().remove("plane");
    let back: MemberProblem = serde_json::from_value(v).unwrap();
    assert_eq!(back.plane, None);
}

#[test]
fn unloaded_structure_solves_to_exact_zero_and_is_accepted() {
    let mut p = templates().remove(0);
    let apex = p.nodes.last().unwrap().id;
    p.apply(Edit::SetLoad { node: apex, force: [0.0; 3], moment: [0.0; 3] }).unwrap();
    let out = p.solve().unwrap();
    assert_eq!(out.max_displacement(), 0.0);
    assert!(out.members.iter().all(|m| m.axial_force == 0.0));
    assert_eq!((out.acceptance.force_balance, out.acceptance.moment_balance), (0.0, 0.0));
}

#[test]
fn tiny_and_huge_unit_systems_solve_with_the_same_relative_accuracy() {
    // SI-like (E = 2e11, m, N) and a mm / N / MPa-like model must both pass the scale-free acceptance gate.
    for (e, area, len, load) in [(2.0e11, 1.0e-4, 2.0, 5.0e3), (2.0e5, 100.0, 2000.0, 5.0e3)] {
        let mut p = MemberProblem::new("scale");
        let a = p.apply(Edit::AddNode { pos: [0.0; 3] }).unwrap().unwrap();
        let b = p.apply(Edit::AddNode { pos: [len, 0.0, 0.0] }).unwrap().unwrap();
        p.apply(Edit::AddMember { a, b, kind: MemberKind::Truss { e, area } }).unwrap();
        p.apply(Edit::SetPlane { axis: Some(2) }).unwrap();
        p.apply(Edit::SetSupport { node: a, fixed: [true, true, true, false, false, false] }).unwrap();
        p.apply(Edit::SetSupport { node: b, fixed: [false, true, true, false, false, false] }).unwrap();
        p.apply(Edit::SetLoad { node: b, force: [load, 0.0, 0.0], moment: [0.0; 3] }).unwrap();
        let out = p.solve().unwrap();
        assert!(rel(out.displacement[1][0], load * len / (e * area)) < 1e-9);
        assert!(rel(out.members[0].axial_stress, load / area) < 1e-9);
    }
}
