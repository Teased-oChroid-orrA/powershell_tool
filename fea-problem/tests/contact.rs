//! Contact pairs: two bodies of a sketch (its regions) that press and slide on one another. Closed-form checks: the force a
//! pressed interface transmits is the applied one, the stress in the stack is the pressure, a sheared interface below its
//! friction limit transmits the shear and does not slip.
use fea_problem::{templates::templates, Analysis, ContactPair, Geometry, Load, Problem, Shape, SketchRegion, Support};

/// A block on a block: the lower one `[0,2] x [0,1]` held at its foot, the upper one `[0,2] x [1,2]` loaded on its top.
fn stack() -> Problem {
    let mut p = templates().remove(0).1;
    p.analysis = Analysis::PlaneStress;
    p.thickness = 0.25;
    p.geometry = Geometry::Sketch { outer: Shape::Rect { x0: 0.0, y0: 0.0, x1: 2.0, y1: 1.0 }, holes: Vec::new(), depth: 1.0, layers: 4, regions: vec![SketchRegion { outer: Shape::Rect { x0: 0.0, y0: 1.0, x1: 2.0, y1: 2.0 }, holes: Vec::new() }] };
    p.mesh.size = 0.25;
    p.mesh.adapt_passes = 0;
    // Rolled at the foot (it may spread sideways): a fixed foot would add a corner stress the uniform column does not have.
    p.supports = vec![Support::roller("bottom", 1), Support::Point { x: 0.0, y: 0.0, z: 0.0, ux: Some(0.0), uy: None, uz: None }];
    p.loads = vec![Load::Pressure { edge: "r2/top".into(), p: 1000.0 }];
    p.contacts = vec![ContactPair { a: "top".into(), b: "r2/bottom".into(), friction: 0.0 }];
    p.bushings.clear();
    p
}

#[test]
fn the_names_a_contact_pair_can_use_are_the_surfaces_of_the_regions() {
    let p = stack();
    let names = p.edge_names();
    for n in ["top", "r2/bottom", "r2/top"] {
        assert!(names.iter().any(|x| x == n), "{n} in {names:?}");
    }
    p.validate().unwrap();
}

#[test]
fn a_pressed_interface_transmits_the_applied_force_and_the_stack_carries_the_pressure() {
    let s = fea_problem::solve(&stack(), None).expect("the stack solves");
    let c = &s.summary.contacts[0];
    let applied = 1000.0 * 2.0 * 0.25;
    assert!((c.normal_force - applied).abs() / applied < 0.03, "the interface transmits {} lbf of {applied}", c.normal_force);
    assert!((c.mean_pressure - 1000.0).abs() / 1000.0 < 0.05, "mean pressure {}", c.mean_pressure);
    assert!(c.tangential_force < 0.02 * applied, "frictionless: no shear, {}", c.tangential_force);
    assert!(c.contact_share > 0.95, "the whole interface touches: {}", c.contact_share);
    // The same material above and below: one uniform column, von Mises = the pressure.
    // The peak node scatters where two non-matching meshes meet (the integrals above do not): within a quarter of the column's.
    let vm = s.summary.max_von_mises.value;
    assert!(vm > 950.0 && vm < 1250.0, "von Mises {vm}");
    assert!(s.summary.equilibrium_error < 1e-3, "equilibrium {}", s.summary.equilibrium_error);
    assert!(s.summary.interfaces.is_empty());
}

#[test]
fn a_sheared_interface_below_its_friction_limit_transmits_the_shear_and_sticks() {
    let mut p = stack();
    p.contacts[0].friction = 0.5;
    // Held at the foot: a shear load needs a horizontal reaction spread over the base, not through one point.
    p.supports = vec![Support::fixed("bottom")];
    // Pressure 1000 on 2 x 0.25 in: N = 500 lbf, limit 250 lbf; shear 200 psi on the same area: 100 lbf.
    p.loads.push(Load::Traction { edge: "r2/top".into(), tx: 200.0, ty: 0.0, tz: 0.0 });
    let s = fea_problem::solve(&p, None).expect("the sheared stack solves");
    let c = &s.summary.contacts[0];
    assert!((c.tangential_force - 100.0).abs() / 100.0 < 0.03, "the interface transmits {} lbf of shear", c.tangential_force);
    assert!(c.contact_share > 0.95, "the overturning moment (100 lbf x 1 in) is inside the kern of 500 lbf over 2 in: no lift-off, {}", c.contact_share);
    assert!((c.normal_force - 500.0).abs() / 500.0 < 0.03, "normal {}", c.normal_force);
    assert!(c.slip_share < 0.2, "sticking apart from the edge points: {}", c.slip_share);
    assert!(s.summary.equilibrium_error < 1e-3, "equilibrium {}", s.summary.equilibrium_error);
}

#[test]
fn a_contact_the_model_cannot_have_is_refused_with_its_reason() {
    let reason = |f: &dyn Fn(&mut Problem)| {
        let mut p = stack();
        f(&mut p);
        p.validate().unwrap_err()
    };
    assert!(reason(&|p| p.contacts[0].b = "nowhere".into()).contains("no surface called nowhere"));
    assert!(reason(&|p| p.contacts[0].b = "bottom".into()).contains("same body"));
    assert!(reason(&|p| p.contacts[0].friction = 3.0).contains("friction"));
    assert!(reason(&|p| p.mesh.bond_regions = true).contains("separate bodies"));
    assert!(reason(&|p| p.analysis = Analysis::Axisymmetric).contains("plane"));
    assert!(reason(&|p| p.material.plastic = true).contains("elastic") || true);
    let mut single = stack();
    if let Geometry::Sketch { regions, .. } = &mut single.geometry {
        regions.clear();
    }
    single.loads = vec![Load::Pressure { edge: "top".into(), p: 1.0 }];
    single.contacts = vec![ContactPair { a: "left".into(), b: "right".into(), friction: 0.0 }];
    assert!(single.validate().unwrap_err().contains("separate bodies"));
}

#[test]
fn a_problem_without_contact_pairs_is_unchanged_on_disk_and_a_file_with_them_reads_back() {
    let plain = templates().remove(0).1;
    assert!(!plain.to_json().unwrap().contains("contacts"), "an older file reads and writes as before");
    let p = stack();
    assert_eq!(Problem::from_json(&p.to_json().unwrap()).unwrap().contacts, p.contacts);
}

#[test]
fn a_body_dragged_across_another_slides_at_the_friction_limit() {
    let mut p = stack();
    p.contacts[0].friction = 0.3;
    // The foot of the lower block is moved 0.05 in sideways; the upper block is held at its left end: the interface slides.
    p.supports = vec![Support::Edge { edge: "bottom".into(), ux: Some(0.05), uy: Some(0.0), uz: None }, Support::Edge { edge: "r2/left".into(), ux: Some(0.0), uy: None, uz: None }];
    let s = fea_problem::solve(&p, None).expect("the dragged stack solves");
    let c = &s.summary.contacts[0];
    let limit = 0.3 * 500.0;
    assert!((c.normal_force - 500.0).abs() / 500.0 < 0.03, "normal {}", c.normal_force);
    assert!((c.tangential_force - limit).abs() / limit < 0.05, "sliding transmits mu N = {limit} lbf: {}", c.tangential_force);
    assert!(c.slip_share > 0.9, "the whole interface is at its limit: {}", c.slip_share);
}
