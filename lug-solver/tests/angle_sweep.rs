//! Load-direction robustness: every pin load angle must solve, not only the axial one.
//! 45 degrees once failed ("contact did not stiffen": secant-predictor extrapolation, then a
//! neutral sideways pin DOF at first touch); the cases below pin those failures down.

use lug_solver::*;

const AL: Material = Material { e_psi: 10.3e6, nu: 0.33 };

fn lug() -> LugGeometry {
    LugGeometry::round_head(0.5, 1.5, 0.25, 3.75)
}

#[test]
fn plain_lug_solves_every_load_angle() {
    let m = LugModel::build_with(&lug(), AL, MeshSpec::default(), false, PlaneMode::Stress).unwrap();
    for load in [1000.0, 4000.0] {
        for ang in (-165..=180).step_by(15) {
            let s = m
                .solve(PinSpec::new(0.4995, 0.15), LoadCase { load_lbf: load, angle_deg: ang as f64 })
                .unwrap_or_else(|e| panic!("angle {ang} load {load}: {e}"));
            assert!(s.peak_hoop.is_finite() && s.peak_hoop > 0.0);
            if ang > 0 && ang < 180 {
                // The lug is symmetric about its axis: +angle mirrors -angle.
                let n = m
                    .solve(PinSpec::new(0.4995, 0.15), LoadCase { load_lbf: load, angle_deg: -ang as f64 })
                    .unwrap_or_else(|e| panic!("angle {} load {load}: {e}", -ang));
                assert!((s.peak_hoop / n.peak_hoop - 1.0).abs() < 0.03, "load {load} angle +-{ang}: {} vs {}", s.peak_hoop, n.peak_hoop);
            }
        }
    }
}

#[test]
fn bushed_lug_solves_oblique_loads() {
    let b = BushingSpec { inner_dia: 0.375, material: Material { e_psi: 17e6, nu: 0.34 }, interference_dia: 0.001, friction: 0.2 };
    let m = LugModel::build_bushed(&lug(), AL, MeshSpec::default(), false, PlaneMode::Stress, Some(b)).unwrap();
    for (ang, load) in [(0.0, 1500.0), (45.0, 1500.0), (90.0, 800.0)] {
        let s = m
            .solve(PinSpec::new(0.374, 0.15), LoadCase { load_lbf: load, angle_deg: ang })
            .unwrap_or_else(|e| panic!("bushed angle {ang}: {e}"));
        assert!(s.peak_hoop > 10_000.0 && s.peak_hoop < 60_000.0, "angle {ang}: hoop {}", s.peak_hoop);
    }
}

#[test]
fn collapse_load_is_continuous_and_symmetric_in_angle() {
    let m = LugModel::build_with(&lug(), AL, MeshSpec::default(), false, PlaneMode::Strain).unwrap();
    let cap = |ang: f64| {
        m.limit_load(PinSpec::new(0.4995, 0.15), LoadCase { load_lbf: 1000.0, angle_deg: ang }, 55_000.0)
            .unwrap_or_else(|e| panic!("collapse at {ang}: {e}"))
            .limit_load_lbf
    };
    let (c0, c45, cm45) = (cap(0.0), cap(45.0), cap(-45.0));
    assert!(c0 > 1000.0 && c45 > 1000.0, "collapse {c0} {c45}");
    assert!((c45 / cm45 - 1.0).abs() < 0.05, "+45 vs -45: {c45} {cm45}");
}

#[test]
fn sweeps_match_single_solves_and_run_in_parallel() {
    let m = LugModel::build_with(&lug(), AL, MeshSpec::default(), false, PlaneMode::Stress).unwrap();
    let angles = [0.0, 30.0, 60.0, 90.0, 150.0];
    let pin = PinSpec::new(0.4995, 0.15);
    let sweep = m.solve_sweep(pin, 2500.0, &angles);
    for (a, r) in angles.iter().zip(&sweep) {
        let single = m.solve(pin, LoadCase { load_lbf: 2500.0, angle_deg: *a }).unwrap();
        let r = r.as_ref().unwrap_or_else(|e| panic!("angle {a}: {e}"));
        assert!((r.peak_hoop / single.peak_hoop - 1.0).abs() < 1e-9, "angle {a}");
    }
    let pm = LugModel::build_with(&lug(), AL, MeshSpec::default(), false, PlaneMode::Strain).unwrap();
    let env = pm.collapse_sweep(pin, 55_000.0, &[0.0, 45.0, 90.0]);
    let caps: Vec<f64> = env.iter().map(|r| r.as_ref().unwrap().limit_load_lbf).collect();
    assert!(caps.iter().all(|c| *c > 1000.0), "{caps:?}");
}
