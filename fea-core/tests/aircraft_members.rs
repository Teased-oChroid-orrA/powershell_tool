//! Phase 16 scope selected by the user: aircraft frame and truss members.
//! These are independent representative load-path benchmarks, not a certified aircraft design.
use fea_core::fields::Field::{Rotation as R, Translation as U};
use fea_core::structural::*;
use std::f64::consts::PI;
#[test]
fn symmetric_aircraft_truss_tripod_matches_closed_form_load_sharing_and_work() {
    let radius = 2.0_f64;
    let height = 1.5_f64;
    let e = 1e7;
    let area = 0.02;
    let p = 100.0;
    let mut nodes = Vec::new();
    for i in 0..3 {
        let angle = 2.0 * PI * i as f64 / 3.0;
        nodes.push([radius * angle.cos(), radius * angle.sin(), 0.0]);
    }
    nodes.push([0.0, 0.0, height]);
    let model = MemberModel::new(
        nodes,
        (0..3)
            .map(|n| Member::Truss {
                nodes: [n, 3],
                e,
                area,
            })
            .collect(),
    )
    .unwrap();
    let mut bc = model.fields().constraints();
    for n in 0..3 {
        for axis in 0..3 {
            bc.prescribe(model.fields(), n, U(axis), 0.0).unwrap();
        }
    }
    let out = model.solve(&bc, &[(3, U(2), -p)], 1e-12).unwrap();
    let l = radius.hypot(height);
    let exact = -p * l.powi(3) / (3.0 * e * area * height * height);
    assert!((out.fields.value(3, U(2)).unwrap() / exact - 1.0).abs() < 1e-12);
    assert!(
        out.fields.value(3, U(0)).unwrap().abs() < 1e-12
            && out.fields.value(3, U(1)).unwrap().abs() < 1e-12
    );
    for member in &out.members {
        assert!((member.axial_stress / (-p * l / (3.0 * height * area)) - 1.0).abs() < 1e-12);
    }
    let support: f64 = (0..3)
        .map(|n| out.fields.reactions[model.fields().index(n, U(2)).unwrap()])
        .sum();
    assert!((support - p).abs() < 1e-12);
    assert!((2.0 * out.fields.quadratic_energy / (-p * exact) - 1.0).abs() < 1e-12);
}
#[test]
fn aircraft_longeron_combined_load_matches_beam_and_torsion_references() {
    let section = FrameSection {
        e: 1e7,
        g: 3.8e6,
        area: 0.3,
        iy: 0.008,
        iz: 0.012,
        torsion: 0.004,
        shear_areas: Some([0.25, 0.25]),
    };
    let l = 5.0;
    let p = 80.0;
    let torque = 12.0;
    let model = MemberModel::new(
        vec![[0.0; 3], [l, 0.0, 0.0]],
        vec![Member::Frame {
            nodes: [0, 1],
            section,
            reference: [0.0, 1.0, 0.0],
            uniform_load: [0.0; 3],
        }],
    )
    .unwrap();
    let mut bc = model.fields().constraints();
    for axis in 0..3 {
        bc.prescribe(model.fields(), 0, U(axis), 0.0).unwrap();
        bc.prescribe(model.fields(), 0, R(axis), 0.0).unwrap();
    }
    let out = model
        .solve(
            &bc,
            &[(1, U(1), p), (1, U(0), 20.0), (1, R(0), torque)],
            1e-12,
        )
        .unwrap();
    let expected = p * l.powi(3) / (3.0 * section.e * section.iz) + p * l / (section.g * 0.25);
    assert!((out.fields.value(1, U(1)).unwrap() / expected - 1.0).abs() < 1e-12);
    assert!(
        (out.fields.value(1, R(0)).unwrap() / (torque * l / (section.g * section.torsion)) - 1.0)
            .abs()
            < 1e-12
    );
    assert!((out.members[0].axial_stress - 20.0 / section.area).abs() < 1e-12);
    assert!((out.members[0].local_end_actions[5] + p * l).abs() < 1e-9);
}
