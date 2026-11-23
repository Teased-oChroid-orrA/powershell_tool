//! The shared edge-distance estimate (`edge_check::estimate`): a bare pin in a hole, a bushing, advisory versus governing.

use edge_check::contact::ContactMesh;
use edge_check::estimate::{estimate, pin_spec, prepare, BoreFit, EstimateInput, EstimateOptions, LoadCase};
use edge_check::fem::MeshSpec;
use edge_check::matrix::{LegacyEdge, Role, BEST_ESTIMATE_MODEL};
use edge_check::models::contact_model::flow_stress;
use edge_check::runner::TARGET_APPLIED;
use edge_check::types::{BushingSpec, Strengths};
use mechanics_core::materials::get_material;

fn input(fit: BoreFit, legacy: Option<LegacyEdge>) -> EstimateInput {
    let a = 0.25;
    EstimateInput {
        bore_radius: a,
        edge: 2.0 * 2.0 * a,
        thickness: 0.25,
        plane_angle_deg: 40.0,
        material: Strengths::from_material(get_material("al7075")),
        fit,
        load: LoadCase::PinTowardEdge { load: 1000.0 },
        fit_pressure: 8800.0,
        fit_pressure_min: 6000.0,
        fit_pressure_max: 11000.0,
        legacy,
    }
}

#[test]
fn the_bare_pin_is_the_configuration_the_naca_validation_runs() {
    let s = pin_spec(0.25, 0.0);
    assert_eq!(s, BushingSpec { inner_radius: 0.4 * 0.25, interference: 0.0, e: 29.0e6, nu: 0.30, friction: 0.0 }, "the NACA TN 1503 tests use exactly this pin");
    assert_eq!(pin_spec(0.25, -1.0).friction, 0.0, "a negative friction is clamped");
}

#[test]
fn a_bare_pin_has_no_fit_pressure_whatever_the_caller_passes() {
    let (i, cfg) = prepare(&input(BoreFit::Pin { friction: 0.0 }, None), &EstimateOptions { deep: true, ..Default::default() }).unwrap();
    assert_eq!((i.fit_pressure, i.fit_pressure_min, i.fit_pressure_max), (0.0, 0.0, 0.0));
    assert_eq!(cfg.bushing, Some(pin_spec(0.25, 0.0)));
    assert!(cfg.include_plastic);
    // A described bushing keeps its pressure band; "pressure only" has no bushing to mesh.
    let (b, cfgb) = prepare(&input(BoreFit::PressureOnly, None), &EstimateOptions::default()).unwrap();
    assert_eq!((b.fit_pressure, b.fit_pressure_max), (8800.0, 11000.0));
    assert_eq!(cfgb.bushing, None);
}

#[test]
fn bad_inputs_are_refused_with_the_reason() {
    let mut i = input(BoreFit::Pin { friction: 0.0 }, None);
    i.edge = 0.2;
    assert!(prepare(&i, &EstimateOptions::default()).unwrap_err().contains("edge distance must exceed the bore radius"));
    let mut i = input(BoreFit::Pin { friction: 0.0 }, None);
    i.thickness = 0.0;
    assert!(prepare(&i, &EstimateOptions::default()).unwrap_err().contains("housing length"));
    let mut i = input(BoreFit::Pin { friction: 0.0 }, None);
    i.load = LoadCase::PinTowardEdge { load: f64::NAN };
    assert!(prepare(&i, &EstimateOptions::default()).is_err());
}

#[test]
fn without_a_legacy_check_the_estimate_is_advisory() {
    let e = estimate(&input(BoreFit::Pin { friction: 0.0 }, None), &EstimateOptions::default()).unwrap();
    assert!(e.matrix.is_advisory() && e.matrix.governing().is_none());
    assert!(e.matrix.rows.iter().all(|r| r.role == Role::Comparison), "no deep run and no legacy: nothing is governing or best estimate");
    let with = estimate(&input(BoreFit::Pin { friction: 0.0 }, Some(LegacyEdge { applied: 0.5, bearing: 1.9 })), &EstimateOptions::default()).unwrap();
    assert!(!with.matrix.is_advisory());
    assert_eq!(with.matrix.governing().unwrap().role, Role::Governing);
}

/// The bare-pin estimate at a grid edge distance (e/D 2.0) is the validated contact solve itself: same pin, same plate sizing.
#[test]
fn the_bare_pin_estimate_reproduces_the_validated_contact_solve() {
    let i = input(BoreFit::Pin { friction: 0.0 }, None);
    let e = estimate(&i, &EstimateOptions { deep: true, strength_cv: Some(0.0), ..Default::default() }).unwrap();
    let best = e.matrix.best_estimate().expect("the contact FE ran on the pin");
    assert_eq!(best.id, BEST_ESTIMATE_MODEL);
    let contact = e.report.models.iter().find(|m| m.id == BEST_ESTIMATE_MODEL).unwrap();
    let capacity = contact.targets[TARGET_APPLIED].as_ref().unwrap().capacity_lbf;
    // The same solve, built by hand the way tests/validation_naca_tn1503.rs does it.
    let (edge_input, _) = prepare(&i, &EstimateOptions::default()).unwrap();
    let mesh = MeshSpec { n_radial: 10, n_arc: [3, 4, 10], grade: 2.0 };
    let m = ContactMesh::build(&edge_input.geom, &pin_spec(0.25, 0.0), i.material.e, i.material.nu, mesh, 3, true).unwrap();
    let direct = m.collapse(0.0, flow_stress(&i.material), i.thickness).unwrap().collapse;
    assert!((capacity / direct - 1.0).abs() < 0.01, "estimate {capacity:.0} lbf vs the validated solve {direct:.0} lbf");
    assert!(capacity > 1000.0, "a 0.5 in pin at e/D 2 in 7075 carries far more than 1000 lbf");
}
