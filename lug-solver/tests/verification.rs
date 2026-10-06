//! The solution self-checks and the ZZ error estimate (items 13 and 5 of the upgrade plan).

use lug_solver::*;

const AL: Material = Material { e_psi: 10.3e6, nu: 0.33 };

fn lug() -> LugGeometry {
    LugGeometry::round_head(0.5, 1.5, 0.25, 3.75)
}

#[test]
fn self_checks_pass_for_axial_oblique_bushed_and_friction_cases() {
    let plain = LugModel::build_with(&lug(), AL, MeshSpec::default(), false, PlaneMode::Stress).unwrap();
    let half = LugModel::build_with(&lug(), AL, MeshSpec::default(), true, PlaneMode::Stress).unwrap();
    let b = BushingSpec { inner_dia: 0.375, material: Material { e_psi: 17e6, nu: 0.34 }, interference_dia: 0.001, friction: 0.2 };
    let bushed = LugModel::build_bushed(&lug(), AL, MeshSpec::default(), false, PlaneMode::Stress, Some(b)).unwrap();
    let cases: Vec<(&str, LugSolution)> = vec![
        ("half axial", half.solve(PinSpec::new(0.4995, 0.15), LoadCase { load_lbf: 3000.0, angle_deg: 0.0 }).unwrap()),
        ("full 45", plain.solve(PinSpec::new(0.4995, 0.15), LoadCase { load_lbf: 3000.0, angle_deg: 45.0 }).unwrap()),
        ("full 90 frictionless", plain.solve(PinSpec::new(0.4995, 0.0), LoadCase { load_lbf: 2000.0, angle_deg: 90.0 }).unwrap()),
        ("bushed 30", bushed.solve(PinSpec::new(0.374, 0.15), LoadCase { load_lbf: 1500.0, angle_deg: 30.0 }).unwrap()),
    ];
    for (name, s) in &cases {
        eprintln!("{name}: {:?}", s.verification);
        assert!(s.verification.ok(), "{name}: {:?}", s.verification);
        assert!(s.verification.discretisation_error > 0.0 && s.verification.discretisation_error < 0.3, "{name}: {:?}", s.verification);
    }
}

#[test]
fn zz_error_estimate_falls_when_the_mesh_is_refined() {
    let err = |n: usize| {
        let mesh = MeshSpec { elements_around: n, ..MeshSpec::default() };
        let m = LugModel::build_with(&lug(), AL, mesh, false, PlaneMode::Stress).unwrap();
        m.solve(PinSpec::new(0.4995, 0.0), LoadCase { load_lbf: 3000.0, angle_deg: 0.0 }).unwrap().verification.discretisation_error
    };
    let (coarse, fine) = (err(36), err(72));
    eprintln!("ZZ error 36: {coarse:.4}, 72: {fine:.4}");
    assert!(fine < coarse, "{fine} !< {coarse}");
}

/// Interface pressure must be smooth along the arc (item 4). Gauss-point constraints on quadratic
/// edges produced a 3-point sawtooth of +-60 % (roughness 0.37); nodal collocation gives < 0.02.
#[test]
fn the_bushing_interface_pressure_is_smooth_not_a_sawtooth() {
    let b = BushingSpec { inner_dia: 0.375, material: Material { e_psi: 17e6, nu: 0.34 }, interference_dia: 0.001, friction: 0.2 };
    let m = LugModel::build_bushed(&lug(), AL, MeshSpec::default(), true, PlaneMode::Stress, Some(b)).unwrap();
    for load in [0.0, 1500.0, 4000.0] {
        let s = m.solve(PinSpec::new(0.374, 0.15), LoadCase { load_lbf: load, angle_deg: 0.0 }).unwrap();
        let mut pts = s.bushing.unwrap().interface;
        pts.sort_by(|a, c| a.angle_deg.total_cmp(&c.angle_deg));
        let p: Vec<f64> = pts.iter().map(|q| q.pressure).collect();
        let (mut rough, mut n) = (0.0f64, 0.0f64);
        for i in 1..p.len() - 1 {
            if p[i - 1] > 0.0 && p[i] > 0.0 && p[i + 1] > 0.0 {
                rough += (p[i - 1] - 2.0 * p[i] + p[i + 1]).abs();
                n += 1.0;
            }
        }
        let mean = p.iter().sum::<f64>() / p.len() as f64;
        assert!(rough / n.max(1.0) / mean < 0.03, "load {load}: roughness {:.3}", rough / n.max(1.0) / mean);
    }
}

/// The clamped far end of a full (oblique) model has corner singularities that are not the lug:
/// they must not set the peak von Mises (they did: a 45 degree load reported 116 ksi at the clamp
/// corner). The mesh error estimate looks at the region around the pin, and falls with refinement.
#[test]
fn the_clamped_end_does_not_set_the_peak_stress_and_the_oblique_mesh_error_converges() {
    let at = |n: usize| {
        let m = LugModel::build_with(&lug(), AL, MeshSpec { elements_around: n, ..MeshSpec::default() }, false, PlaneMode::Stress).unwrap();
        m.solve(PinSpec::new(0.4995, 0.15), LoadCase { load_lbf: 4000.0, angle_deg: 45.0 }).unwrap()
    };
    let (coarse, fine) = (at(72), at(144));
    assert!(coarse.peak_von_mises_at[0] < 3.75 - 0.75 && fine.peak_von_mises_at[0] < 3.75 - 0.75, "{:?} {:?}", coarse.peak_von_mises_at, fine.peak_von_mises_at);
    assert!(fine.verification.discretisation_error < 0.5 * coarse.verification.discretisation_error, "{} {}", coarse.verification.discretisation_error, fine.verification.discretisation_error);
    // The hoop stress at the bore is what the estimate is for: it agrees across the two meshes.
    assert!((coarse.peak_hoop / fine.peak_hoop - 1.0).abs() < 0.04, "{} vs {}", coarse.peak_hoop, fine.peak_hoop);
}
