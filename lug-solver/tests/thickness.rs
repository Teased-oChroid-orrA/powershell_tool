//! Through-thickness pin bending (upgrade plan item 8).

use lug_solver::*;

const AL: Material = Material { e_psi: 10.3e6, nu: 0.33 };

fn setup() -> (LugModel, PinSpec, LoadCase) {
    let g = LugGeometry::round_head(0.5, 1.5, 0.25, 3.75);
    let m = LugModel::build_with(&g, AL, MeshSpec::default(), true, PlaneMode::Stress).unwrap();
    (m, PinSpec::new(0.4995, 0.15), LoadCase { load_lbf: 3000.0, angle_deg: 0.0 })
}

fn bending(e: f64, shear: Shear) -> PinBending {
    PinBending { e_psi: e, nu: 0.3, shear, slices: 9 }
}

#[test]
fn a_very_stiff_pin_spreads_the_load_evenly_and_carries_the_statics_moment() {
    let (m, pin, case) = setup();
    let c = 0.1;
    let r = m.through_thickness(pin, case, bending(1.0e12, Shear::Double { offset: c })).unwrap();
    let (t, p) = (0.25, case.load_lbf);
    assert!((r.peaking - 1.0).abs() < 0.01, "peaking {}", r.peaking);
    // Statics: the clevis reactions P/2 at t/2 + c, the lug load P/t over the thickness.
    let want = p * (t / 8.0 + c / 2.0);
    assert!((r.max_moment / want - 1.0).abs() < 0.02, "moment {} vs {want}", r.max_moment);
    let total: f64 = r.slice_load.iter().map(|q| q * t / r.z.len() as f64).sum();
    assert!((total / p - 1.0).abs() < 1e-6, "slice loads sum to the pin load: {total}");
    // Pin shear at the lug face is half the load.
    assert!((r.max_shear_force / (p / 2.0) - 1.0).abs() < 0.02, "{}", r.max_shear_force);
}

#[test]
fn a_flexible_pin_concentrates_bearing_at_the_lug_faces_next_to_the_clevis() {
    let (m, pin, case) = setup();
    let shear = Shear::Double { offset: 0.1 };
    let stiff = m.through_thickness(pin, case, bending(29.0e6 * 100.0, shear)).unwrap();
    let steel = m.through_thickness(pin, case, bending(29.0e6, shear)).unwrap();
    let soft = m.through_thickness(pin, case, bending(0.3e6, shear)).unwrap();
    eprintln!("THICK peaking stiff {:.3} steel {:.3} soft {:.3}; loads {:?}", stiff.peaking, steel.peaking, soft.peaking, soft.slice_load.iter().map(|v| *v as i64).collect::<Vec<_>>());
    assert!(stiff.peaking < steel.peaking && steel.peaking < soft.peaking);
    assert!(soft.peaking > 1.1, "{}", soft.peaking);
    // Double shear is symmetric about the middle and the faces carry the most.
    let n = soft.slice_load.len();
    for i in 0..n / 2 {
        assert!((soft.slice_load[i] / soft.slice_load[n - 1 - i] - 1.0).abs() < 1e-6);
    }
    assert!(soft.slice_load[0] > soft.slice_load[n / 2]);
    let total: f64 = soft.slice_load.iter().map(|q| q * 0.25 / n as f64).sum();
    assert!((total / case.load_lbf - 1.0).abs() < 1e-6);
    assert!(steel.pin_bending_stress > 0.0 && steel.pin_shear_stress > 0.0);
}


#[test]
fn a_thick_lug_on_a_slender_pin_peaks_more_than_a_thin_one() {
    let peak = |t: f64| {
        let g = LugGeometry::round_head(0.5, 1.5, t, 3.75);
        let m = LugModel::build_with(&g, AL, MeshSpec::default(), true, PlaneMode::Stress).unwrap();
        let case = LoadCase { load_lbf: 12_000.0 * t, angle_deg: 0.0 };
        m.through_thickness(PinSpec::new(0.4995, 0.15), case, bending(29.0e6, Shear::Double { offset: t / 2.0 })).unwrap().peaking
    };
    let (thin, thick) = (peak(0.2), peak(0.9));
    eprintln!("THICK peaking t 0.2: {thin:.3}, t 0.9: {thick:.3}");
    assert!(thick > thin && thick > 1.1, "{thin} {thick}");
}
