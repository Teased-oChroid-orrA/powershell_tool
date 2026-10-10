use fea_core::fields::Field::{Rotation as R, Translation as U};
use fea_core::plate::*;
use std::f64::consts::PI;
fn mesh(div: usize, t: f64) -> PlateModel {
    let mut nodes = Vec::new();
    for j in 0..=div {
        for i in 0..=div {
            nodes.push([i as f64 / div as f64, j as f64 / div as f64, 0.0]);
        }
    }
    let mut quads = Vec::new();
    for j in 0..div {
        for i in 0..div {
            let a = j * (div + 1) + i;
            quads.push([a, a + 1, a + div + 2, a + div + 1]);
        }
    }
    PlateModel::new(
        nodes,
        quads,
        PlateMaterial {
            e: 2e7,
            nu: 0.3,
            thickness: t,
            shear_correction: 5.0 / 6.0,
        },
    )
    .unwrap()
}
#[test]
fn flat_shell_membrane_and_bending_patch_recover_exact_resultants() {
    let model = mesh(2, 0.1);
    let mut bc = model.fields().constraints();
    let ex = 0.002;
    let ey = -0.001;
    let curvature = 0.03;
    for (n, x) in model.nodes().iter().enumerate() {
        bc.prescribe(model.fields(), n, U(0), ex * x[0]).unwrap();
        bc.prescribe(model.fields(), n, U(1), ey * x[1]).unwrap();
        bc.prescribe(model.fields(), n, U(2), -curvature * x[0] * x[0] / 2.0)
            .unwrap();
        bc.prescribe(model.fields(), n, R(0), 0.0).unwrap();
        bc.prescribe(model.fields(), n, R(1), curvature * x[0])
            .unwrap();
    }
    let out = model.solve(&bc, &[], &|_| 0.0, 1e-12).unwrap();
    let dm = 2e7 * 0.1 / (1.0 - 0.3 * 0.3);
    let db = 2e7 * 0.1_f64.powi(3) / (12.0 * (1.0 - 0.3 * 0.3));
    for center in out.centers {
        assert!((center.membrane[0] - dm * (ex + 0.3 * ey)).abs() < 1e-8);
        assert!((center.membrane[1] - dm * (ey + 0.3 * ex)).abs() < 1e-8);
        assert!((center.moments[0] - db * curvature).abs() < 1e-8);
        assert!((center.moments[1] - 0.3 * db * curvature).abs() < 1e-8);
        assert!(center.shear.iter().all(|v| v.abs() < 1e-8));
    }
}
#[test]
fn flat_shell_rigid_translation_and_rotation_have_zero_energy() {
    let model = mesh(2, 0.02);
    let mut bc = model.fields().constraints();
    for (n, x) in model.nodes().iter().enumerate() {
        bc.prescribe(model.fields(), n, U(0), 0.2 - 0.01 * x[1])
            .unwrap();
        bc.prescribe(model.fields(), n, U(1), 0.1 + 0.01 * x[0])
            .unwrap();
        bc.prescribe(model.fields(), n, U(2), 0.3 + 0.02 * x[1] - 0.03 * x[0])
            .unwrap();
        bc.prescribe(model.fields(), n, R(0), 0.02).unwrap();
        bc.prescribe(model.fields(), n, R(1), 0.03).unwrap();
    }
    let out = model.solve(&bc, &[], &|_| 0.0, 1e-12).unwrap();
    assert!(out.fields.quadratic_energy.abs() < 1e-8);
    assert!(out.centers.iter().all(|c| c
        .moments
        .iter()
        .chain(&c.membrane)
        .chain(&c.shear)
        .all(|v| v.abs() < 1e-8)));
}
#[test]
fn simply_supported_sinusoidal_plate_converges_without_thin_plate_locking() {
    // Navier/Mindlin single Fourier mode: w=P/(D k^4)+P/(S k²).
    // Hard simply supported: w=0, tangential rotation=0 along each edge.
    for thickness in [0.2_f64, 0.02, 0.002] {
        let d = 2e7 * thickness.powi(3) / (12.0 * (1.0 - 0.3 * 0.3));
        let shear = (5.0 / 6.0) * 2e7 * thickness / (2.0 * (1.0 + 0.3));
        let k2 = 2.0 * PI * PI;
        let exact = 1.0 / (d * k2 * k2) + 1.0 / (shear * k2);
        let mut errors = Vec::new();
        for div in [4, 8, 16] {
            let model = mesh(div, thickness);
            let mut bc = model.fields().constraints();
            for (n, x) in model.nodes().iter().enumerate() {
                // Isolate bending; membrane response is checked separately above.
                bc.prescribe(model.fields(), n, U(0), 0.0).unwrap();
                bc.prescribe(model.fields(), n, U(1), 0.0).unwrap();
                if x[0] == 0.0 || x[0] == 1.0 {
                    bc.prescribe(model.fields(), n, U(2), 0.0).unwrap();
                    bc.prescribe(model.fields(), n, R(0), 0.0).unwrap();
                }
                if x[1] == 0.0 || x[1] == 1.0 {
                    bc.prescribe(model.fields(), n, U(2), 0.0).unwrap();
                    bc.prescribe(model.fields(), n, R(1), 0.0).unwrap();
                }
            }
            let out = model
                .solve(&bc, &[], &|x| (PI * x[0]).sin() * (PI * x[1]).sin(), 1e-8)
                .unwrap();
            let center = (div / 2) * (div + 1) + div / 2;
            let got = out.fields.value(center, U(2)).unwrap();
            errors.push((got / exact - 1.0).abs());
        }
        assert!(
            errors[0] > errors[1] && errors[1] > errors[2],
            "t={thickness}: {errors:?}"
        );
        assert!(errors[2] < 0.01, "t={thickness}: {errors:?}");
        assert!(
            errors[0] / errors[1] > 3.0 && errors[1] / errors[2] > 3.0,
            "t={thickness}: {errors:?}"
        );
    }
}
#[test]
fn unsupported_distorted_geometry_is_an_error() {
    let material = PlateMaterial {
        e: 1.0,
        nu: 0.3,
        thickness: 0.1,
        shear_correction: 5.0 / 6.0,
    };
    assert!(PlateModel::new(
        vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.1, 1.0, 0.0],
            [0.0, 1.0, 0.0]
        ],
        vec![[0, 1, 2, 3]],
        material
    )
    .is_err());
}
