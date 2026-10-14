//! Member compliance of a clamped stack against closed forms and bounds.

use fea_problem::joint::{member_compliance, member_compliance_sized, Layer};
use std::f64::consts::{FRAC_PI_4, PI};

fn layer(t: f64, e: f64, hole: f64, od: Option<f64>) -> Layer {
    Layer { thickness: t, e, nu: 0.3, hole_diameter: hole, outer_diameter: od }
}

fn area(od: f64, hole: f64) -> f64 {
    FRAC_PI_4 * (od * od - hole * hole)
}

#[test]
fn a_ring_loaded_over_its_whole_face_is_exactly_l_over_ea() {
    // Uniform axial compression of a free ring: the FE state is exact.
    let l = layer(0.5, 10.0e6, 0.25, Some(1.5));
    let c = member_compliance(&[l], 1.5, 1.5).unwrap().compliance;
    let want = 0.5 / (10.0e6 * area(1.5, 0.25));
    assert!((c / want - 1.0).abs() < 1e-9, "{c:e} vs {want:e}");
}

#[test]
fn members_in_series_add_their_compliances() {
    // Poisson's ratio zero: no lateral mismatch at the interfaces, so the stress state is uniform and the
    // compliances add exactly (with nu > 0 and different moduli the interfaces constrain each other).
    let mut stack = [layer(0.3, 10.0e6, 0.25, Some(1.5)), layer(0.2, 29.0e6, 0.25, Some(1.5)), layer(0.4, 16.0e6, 0.25, Some(1.5))];
    for l in &mut stack {
        l.nu = 0.0;
    }
    let c = member_compliance(&stack, 1.5, 1.5).unwrap().compliance;
    let want: f64 = stack.iter().map(|l| l.thickness / (l.e * area(1.5, 0.25))).sum();
    assert!((c / want - 1.0).abs() < 1e-9, "{c:e} vs {want:e}");
}

#[test]
fn a_narrow_bearing_on_a_wide_plate_sits_between_its_two_bounds() {
    // Load spreading: stiffer than the bearing annulus alone carrying it, softer than the whole plate carrying it.
    let (t, e, hole, bearing, od) = (1.0, 10.0e6, 0.5, 1.0, 6.0);
    let c = member_compliance(&[layer(t, e, hole, Some(od))], bearing, bearing).unwrap().compliance;
    let upper = t / (e * area(bearing, hole));
    let lower = t / (e * area(od, hole));
    assert!(c < upper && c > lower, "{lower:e} < {c:e} < {upper:e}");
    // A thick stack spreads the load more: its compliance per unit thickness is lower than the thin one's.
    let thick = member_compliance(&[layer(4.0, e, hole, Some(od))], bearing, bearing).unwrap().compliance;
    assert!(thick / 4.0 < c / t);
}

#[test]
fn the_fe_compliance_has_converged_with_the_default_mesh() {
    let stack = [layer(0.4, 10.0e6, 0.25, None), layer(0.3, 29.0e6, 0.27, Some(2.0))];
    let coarse = member_compliance(&stack, 0.9, 0.9).unwrap();
    let fine = member_compliance_sized(&stack, 0.9, 0.9, 0.06).unwrap();
    assert!(fine.elements > 2 * coarse.elements, "the refined mesh is larger");
    assert!((coarse.compliance / fine.compliance - 1.0).abs() < 0.01, "{:e} vs {:e}", coarse.compliance, fine.compliance);
}

#[test]
fn it_is_close_to_the_cone_estimate_for_a_typical_joint() {
    // The VDI-style 30 degree pressure cone for two 0.5 in aluminium plates under a 0.75 in head.
    let (t, e, hole, dc) = (0.5, 10.3e6, 0.3125, 0.75);
    let stack = [layer(t, e, hole, None), layer(t, e, hole, None)];
    let fe = member_compliance(&stack, dc, dc).unwrap().compliance;
    // Cone: each half (thickness t) under a cone growing at tan(30 deg) from the contact diameter.
    let tan = (30.0f64).to_radians().tan();
    let n = 2000;
    let mut cone = 0.0;
    for i in 0..n {
        let z = (i as f64 + 0.5) / n as f64 * t;
        cone += 2.0 * (t / n as f64) / (e * area(dc + 2.0 * z * tan, hole));
    }
    let ratio = fe / cone;
    eprintln!("FE {fe:.4e}, cone {cone:.4e}, ratio {ratio:.3}");
    assert!(ratio > 0.6 && ratio < 1.6, "FE and cone estimates differ by more than the model scatter: {ratio}");
    let _ = PI;
}

#[test]
fn bad_stacks_are_refused_with_a_reason() {
    assert!(member_compliance(&[], 1.0, 1.0).unwrap_err().contains("no members"));
    assert!(member_compliance(&[layer(0.5, 1.0e7, 0.5, Some(0.4))], 1.0, 1.0).is_err());
    assert!(member_compliance(&[layer(0.0, 1.0e7, 0.25, None)], 1.0, 1.0).unwrap_err().contains("invalid"));
    // A bearing diameter no larger than the hole has no annulus.
    assert!(member_compliance(&[layer(0.5, 1.0e7, 0.5, None)], 0.5, 1.0).unwrap_err().contains("no width"));
}

#[test]
fn compliance_sensitivities_are_exact_for_uncoupled_members_and_match_differences_for_coupled_ones() {
    use fea_problem::joint::member_compliance_sensitivities;
    // nu = 0: members in series, C = sum t_k / (E_k A), so the derivative with respect to a modulus multiplier is -C_k
    let mut stack = [layer(0.3, 10.0e6, 0.25, Some(1.5)), layer(0.2, 29.0e6, 0.25, Some(1.5)), layer(0.4, 16.0e6, 0.25, Some(1.5))];
    for l in &mut stack {
        l.nu = 0.0;
    }
    let s = member_compliance_sensitivities(&stack, 1.5, 1.5).unwrap();
    for (k, l) in stack.iter().enumerate() {
        let ck = l.thickness / (l.e * area(1.5, 0.25));
        assert!((s.modulus_scale[k] / -ck - 1.0).abs() < 1e-8, "member {k}: {:e} vs {:e}", s.modulus_scale[k], -ck);
    }
    // a Poisson-coupled stack with a narrow bearing on a wide plate: against central differences of the full FE compliance
    let stack = [layer(0.5, 10.0e6, 0.4, Some(5.0)), layer(0.4, 29.0e6, 0.4, Some(5.0))];
    let s = member_compliance_sensitivities(&stack, 1.0, 1.0).unwrap();
    let c = |st: &[Layer; 2]| member_compliance(st, 1.0, 1.0).unwrap().compliance;
    for k in 0..2 {
        let (h_e, h_nu) = (1e-3, 1e-4);
        let (mut p, mut m) = (stack, stack);
        p[k].e *= 1.0 + h_e;
        m[k].e *= 1.0 - h_e;
        let fd_e = (c(&p) - c(&m)) / (2.0 * h_e);
        assert!((s.modulus_scale[k] - fd_e).abs() < 1e-6 * fd_e.abs(), "modulus {k}: {:e} vs {fd_e:e}", s.modulus_scale[k]);
        let (mut p, mut m) = (stack, stack);
        p[k].nu += h_nu;
        m[k].nu -= h_nu;
        let fd_nu = (c(&p) - c(&m)) / (2.0 * h_nu);
        assert!((s.poisson[k] - fd_nu).abs() < 1e-6 * fd_nu.abs().max(1e-30), "poisson {k}: {:e} vs {fd_nu:e}", s.poisson[k]);
    }
    assert!((s.compliance - member_compliance(&stack, 1.0, 1.0).unwrap().compliance).abs() < 1e-12 * s.compliance);
}
