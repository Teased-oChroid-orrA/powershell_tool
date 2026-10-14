//! Level 2 of the derivative suite: constitutive tangents. The consistent tangent of the *implemented stress-update algorithm* is
//! checked against the independent closed-form tangents of `material.rs`, against finite differences of the algorithm, and (for
//! the implicit route) against differentiating through converged iterations; unconverged local solves are refused.

use fea_core::derivative::constitutive::*;
use fea_core::material::{neo_hookean_update, small_strain_update, Moduli, M3};
use mechanics_core::hardening::Hardening;

const E: f64 = 29.0e6;
const NU: f64 = 0.3;

fn law() -> Hardening {
    Hardening::table(&[(0.0, 40_000.0), (0.002, 48_000.0), (0.01, 52_000.0), (0.05, 60_000.0)]).unwrap()
}

fn tensor(v: &[f64; 6]) -> M3 {
    [[v[0], v[3], v[5]], [v[3], v[1], v[4]], [v[5], v[4], v[2]]]
}

fn voigt(m: &M3) -> [f64; 6] {
    [m[0][0], m[1][1], m[2][2], m[0][1], m[1][2], m[2][0]]
}

/// The closed-form tangent of `small_strain_update` (`dS_ij / dH_kl`) as a Voigt `6 x 6` derivative with respect to tensor-shear strain.
fn closed_form_tangent(strain: &[f64; 6], old: &[f64; 7]) -> (Vec<f64>, [f64; 6], [f64; 7]) {
    let mo = Moduli::new(E, NU);
    let up = small_strain_update(mo, &law(), &tensor(strain), old);
    let rows = [(0, 0), (1, 1), (2, 2), (0, 1), (1, 2), (2, 0)];
    let mut t = vec![0.0; 36];
    for (r, &(i, j)) in rows.iter().enumerate() {
        for (c, &(k, l)) in rows.iter().enumerate() {
            let mut v = up.tangent[3 * i + j][3 * k + l];
            if k != l {
                v += up.tangent[3 * i + j][3 * l + k]; // the symmetric shear strain enters both H_kl and H_lk
            }
            t[r * 6 + c] = v;
        }
    }
    (t, voigt(&up.stress), up.state)
}

fn max_diff(a: &[f64], b: &[f64]) -> f64 {
    let scale = a.iter().chain(b).fold(0.0f64, |m, v| m.max(v.abs())).max(1e-300);
    a.iter().zip(b).fold(0.0f64, |m, (x, y)| m.max((x - y).abs())) / scale
}

fn central_difference_tangent<M: DifferentiableMaterial>(pt: &MaterialPoint<M>, strain: &[f64; 6], h: f64) -> Vec<f64> {
    let mut t = vec![0.0; 36];
    for c in 0..6 {
        let (mut sp, mut sm) = (*strain, *strain);
        sp[c] += h;
        sm[c] -= h;
        let (a, b) = (pt.trial(&sp).unwrap(), pt.trial(&sm).unwrap());
        for r in 0..6 {
            t[r * 6 + c] = (a.stress[r] - b.stress[r]) / (2.0 * h);
        }
    }
    t
}

#[test]
fn the_ad_tangent_of_hookes_law_is_the_constant_analytic_tangent_and_the_parameter_derivatives_are_exact() {
    let m = Hooke { e: E, nu: NU };
    let pt = MaterialPoint::new(m);
    let strain = [1.2e-3, -4e-4, 2e-4, 3e-4, -5e-4, 1e-4];
    let s = pt.sensitivities(&strain).unwrap();
    assert!(max_diff(&s.d_stress_d_strain, &m.analytic_tangent()) < 1e-14);
    // dsigma/dE = sigma / E exactly (the law is linear in E at fixed nu)
    let de: Vec<f64> = (0..6).map(|r| s.d_stress_d_params[r * 2]).collect();
    let want: Vec<f64> = s.stress.iter().map(|v| v / E).collect();
    assert!(max_diff(&de, &want) < 1e-13);
    assert_eq!(s.branch, Branch::Elastic);
    assert_eq!(s.new_state, [0.0; 7]);
}

#[test]
fn the_j2_return_map_and_its_ad_tangent_match_the_closed_form_at_every_stage_of_a_path() {
    let m = J2Table::new(E, NU, &law());
    let mut pt = MaterialPoint::new(m);
    let mut old = [0.0; 7];
    // a loading path: elastic, first yield, hardening across the table's segments, a reversal
    let path: [[f64; 6]; 8] = [
        [2.0e-4, -6e-5, -6e-5, 0.0, 0.0, 0.0],
        [1.0e-3, -3e-4, -3e-4, 2e-4, 0.0, 0.0],
        [2.0e-3, -6e-4, -6e-4, 5e-4, 1e-4, 0.0],
        [4.0e-3, -1.2e-3, -1.2e-3, 1e-3, 2e-4, 3e-4],
        [8.0e-3, -2.4e-3, -2.4e-3, 2e-3, 4e-4, 6e-4],
        [1.6e-2, -4.8e-3, -4.8e-3, 3e-3, 8e-4, 1e-3],
        [2.4e-2, -7.2e-3, -7.2e-3, 3e-3, 8e-4, 1e-3],
        [1.0e-2, -3.0e-3, -3.0e-3, 1e-3, 0.0, 5e-4],
    ];
    let mut yielded = 0;
    for (step, strain) in path.iter().enumerate() {
        let (want_t, want_s, want_state) = closed_form_tangent(strain, &old);
        let trial = pt.trial(strain).unwrap();
        let sens = pt.sensitivities(strain).unwrap();
        assert!(max_diff(&trial.stress, &want_s) < 1e-12, "step {step}: stress");
        assert!(max_diff(&trial.new_state, &want_state) < 1e-12, "step {step}: state");
        assert!(max_diff(&sens.d_stress_d_strain, &want_t) < 1e-9, "step {step}: AD tangent vs closed form {:e}", max_diff(&sens.d_stress_d_strain, &want_t));
        // the tangent is that of the algorithm: a difference of the algorithm itself agrees (to the difference quotient's accuracy)
        let fd = central_difference_tangent(&pt, strain, 1e-7);
        assert!(max_diff(&sens.d_stress_d_strain, &fd) < 1e-5, "step {step}: AD tangent vs finite differences {:e}", max_diff(&sens.d_stress_d_strain, &fd));
        yielded += usize::from(sens.branch == Branch::Plastic);
        pt.commit(&trial);
        old = want_state;
        assert!(max_diff(&pt.committed, &old) < 1e-14, "the committed state follows the closed-form state history");
    }
    assert!(yielded >= 5, "the path must go plastic: {yielded}");
    assert!(pt.committed[6] > 0.0);
}

#[test]
fn a_trial_update_does_not_touch_the_committed_state() {
    let pt = MaterialPoint::new(J2Table::new(E, NU, &law()));
    let before = pt.committed;
    let t = pt.trial(&[5e-3, -1.5e-3, -1.5e-3, 0.0, 0.0, 0.0]).unwrap();
    assert_eq!(pt.committed, before, "a trial is not a commit");
    assert!(t.new_state[6] > 0.0);
}

#[test]
fn the_history_and_parameter_derivatives_of_the_update_match_differences_of_the_algorithm() {
    let m = J2Table::new(E, NU, &law());
    let mut pt = MaterialPoint::new(m.clone());
    // pre-strain to put a plastic history on the point
    let first = pt.trial(&[6e-3, -1.8e-3, -1.8e-3, 1e-3, 0.0, 0.0]).unwrap();
    pt.commit(&first);
    let strain = [9e-3, -2.7e-3, -2.7e-3, 1.5e-3, 3e-4, 0.0];
    let s = pt.sensitivities(&strain).unwrap();
    assert_eq!(s.branch, Branch::Plastic);
    let eval = |p: &[f64], old: &[f64; 7]| {
        let q = m.solve_local(p, &strain, old).unwrap();
        m.update::<f64>(p, &strain, old, &q)
    };
    let h = 1e-6;
    // wrt the committed state (plastic strain components and ep)
    for c in 0..7 {
        let (mut op, mut om) = (pt.committed, pt.committed);
        let step = h * (pt.committed[c].abs().max(1e-3));
        op[c] += step;
        om[c] -= step;
        let (up, um) = (eval(&pt.params, &op), eval(&pt.params, &om));
        for r in 0..6 {
            let fd = (up.stress[r] - um.stress[r]) / (2.0 * step);
            let ad = s.d_stress_d_state[r * 7 + c];
            assert!((fd - ad).abs() <= 1e-4 * (1.0 + ad.abs().max(fd.abs())), "dsigma_{r}/dstate_{c}: AD {ad} vs FD {fd}");
        }
        for r in 0..7 {
            let fd = (up.new_state[r] - um.new_state[r]) / (2.0 * step);
            let ad = s.d_new_state_d_state[r * 7 + c];
            assert!((fd - ad).abs() <= 1e-4 * (1.0 + ad.abs().max(fd.abs())), "dstate_{r}/dstate_{c}: AD {ad} vs FD {fd}");
        }
    }
    // wrt the parameters E, nu, yield scale, hardening scale
    for c in 0..4 {
        let (mut pp, mut pm) = (pt.params.clone(), pt.params.clone());
        let step = h * pt.params[c].abs();
        pp[c] += step;
        pm[c] -= step;
        let (up, um) = (eval(&pp, &pt.committed), eval(&pm, &pt.committed));
        for r in 0..6 {
            let fd = (up.stress[r] - um.stress[r]) / (2.0 * step);
            let ad = s.d_stress_d_params[r * 4 + c];
            assert!((fd - ad).abs() <= 1e-4 * (1.0 + ad.abs().max(fd.abs())), "dsigma_{r}/dp_{c}: AD {ad} vs FD {fd}");
        }
    }
}

#[test]
fn the_yield_surface_is_detected_and_both_one_sided_tangents_are_reported() {
    let pt = MaterialPoint::new(J2Table::new(E, NU, &law()));
    // uniaxial-like strain scaled to land exactly on the surface: q = sigma_y  <=>  strain scale
    let dir = [1.0, -0.3, -0.3, 0.0, 0.0, 0.0];
    let q_of = |a: f64| {
        let e: [f64; 6] = std::array::from_fn(|i| a * dir[i]);
        let tr = e[0] + e[1] + e[2];
        let dev = [e[0] - tr / 3.0, e[1] - tr / 3.0, e[2] - tr / 3.0];
        1.5f64.sqrt() * 2.0 * (E / (2.0 * (1.0 + NU))) * dev.iter().map(|v| v * v).sum::<f64>().sqrt()
    };
    let a = 40_000.0 / q_of(1.0);
    let strain: [f64; 6] = std::array::from_fn(|i| a * dir[i]);
    let on = pt.sensitivities(&strain).unwrap();
    assert_eq!(on.branch, Branch::Transition);
    let (elastic, plastic) = pt.one_sided(&strain).unwrap().expect("a yield surface");
    assert_eq!((elastic.branch, plastic.branch), (Branch::Elastic, Branch::Plastic));
    // the elastic side is Hooke, the plastic side is softer in the flow direction: the derivative genuinely jumps
    let hooke = Hooke { e: E, nu: NU }.analytic_tangent();
    assert!(max_diff(&elastic.d_stress_d_strain, &hooke) < 1e-12);
    assert!(max_diff(&plastic.d_stress_d_strain, &hooke) > 1e-2);
    // a point well inside either side is not a transition
    let inside: [f64; 6] = std::array::from_fn(|i| 0.5 * strain[i]);
    assert_eq!(pt.sensitivities(&inside).unwrap().branch, Branch::Elastic);
    let beyond: [f64; 6] = std::array::from_fn(|i| 2.0 * strain[i]);
    assert_eq!(pt.sensitivities(&beyond).unwrap().branch, Branch::Plastic);
}

fn voce() -> J2Voce {
    J2Voce::new(E, NU, 35_000.0, 2.0e5, 25_000.0, 400.0)
}

#[test]
fn an_implicit_return_map_gets_its_tangent_from_the_converged_local_system() {
    let mut pt = MaterialPoint::new(voce());
    // some history first
    let t = pt.trial(&[4e-3, -1.2e-3, -1.2e-3, 8e-4, 0.0, 0.0]).unwrap();
    pt.commit(&t);
    let strain = [9e-3, -2.6e-3, -2.6e-3, 2e-3, 5e-4, 1e-3];
    let s = pt.sensitivities(&strain).unwrap();
    assert_eq!(s.branch, Branch::Plastic);
    // against differences of the converged algorithm
    let fd = central_difference_tangent(&pt, &strain, 1e-7);
    assert!(max_diff(&s.d_stress_d_strain, &fd) < 1e-5, "implicit tangent vs finite differences {:e}", max_diff(&s.d_stress_d_strain, &fd));
    // against differentiating through converged iterations
    let through = voce_tangent_through_iterations(&voce(), &strain, &pt.committed, 60);
    assert!(max_diff(&s.d_stress_d_strain, &through) < 1e-10, "{:e}", max_diff(&s.d_stress_d_strain, &through));
    // ...which, truncated, is wrong: unconverged iterations are not the constitutive map
    let truncated = voce_tangent_through_iterations(&voce(), &strain, &pt.committed, 1);
    assert!(max_diff(&s.d_stress_d_strain, &truncated) > 1e-3, "{:e}", max_diff(&s.d_stress_d_strain, &truncated));
    // parameter and history derivatives through the implicit system
    let np = 6;
    for c in 0..np {
        let (mut pp, mut pm) = (pt.params.clone(), pt.params.clone());
        let step = 1e-6 * pt.params[c].abs();
        pp[c] += step;
        pm[c] -= step;
        let eval = |p: &[f64]| {
            let q = pt.model.solve_local(p, &strain, &pt.committed).unwrap();
            pt.model.update::<f64>(p, &strain, &pt.committed, &q)
        };
        let (up, um) = (eval(&pp), eval(&pm));
        for r in 0..6 {
            let fd = (up.stress[r] - um.stress[r]) / (2.0 * step);
            let ad = s.d_stress_d_params[r * np + c];
            assert!((fd - ad).abs() <= 1e-5 * (1.0 + ad.abs().max(fd.abs())), "dsigma_{r}/dp_{c}: {ad} vs {fd}");
        }
    }
    for c in 0..7 {
        let (mut op, mut om) = (pt.committed, pt.committed);
        let step = 1e-6 * pt.committed[c].abs().max(1e-3);
        op[c] += step;
        om[c] -= step;
        let eval = |old: &[f64; 7]| {
            let q = pt.model.solve_local(&pt.params, &strain, old).unwrap();
            pt.model.update::<f64>(&pt.params, &strain, old, &q)
        };
        let (up, um) = (eval(&op), eval(&om));
        for r in 0..7 {
            let fd = (up.new_state[r] - um.new_state[r]) / (2.0 * step);
            let ad = s.d_new_state_d_state[r * 7 + c];
            assert!((fd - ad).abs() <= 1e-5 * (1.0 + ad.abs().max(fd.abs())), "dstate_{r}/dstate_{c}: {ad} vs {fd}");
        }
    }
}

#[test]
fn a_local_solve_that_did_not_converge_has_no_derivative() {
    let mut m = voce();
    m.max_iter = 1;
    // a strain far enough past yield that one Newton step from d = 0 cannot converge
    let pt = MaterialPoint::new(m);
    let err = pt.sensitivities(&[3e-2, -9e-3, -9e-3, 0.0, 0.0, 0.0]).unwrap_err();
    assert!(matches!(err, ConstitutiveError::LocalSolveFailed(_)), "{err}");
    assert!(err.to_string().contains("derivative does not exist"));
    // an elastic point needs no local solve and differentiates fine whatever the iteration budget
    assert!(pt.sensitivities(&[1e-4, 0.0, 0.0, 0.0, 0.0, 0.0]).is_ok());
}

#[test]
fn the_neo_hookean_ad_tangent_matches_the_closed_form_hyperelastic_tangent() {
    use fea_core::derivative::dual::Dual;
    use fea_core::derivative::scalar::Scalar;
    let mo = Moduli::new(E / 1000.0, 0.35);
    let lam = mo.k - 2.0 * mo.g / 3.0;
    let h: M3 = [[0.12, 0.03, -0.02], [0.01, -0.08, 0.04], [0.02, 0.0, 0.05]];
    let up = neo_hookean_update(mo, &h, &[0.0; 7]).unwrap();
    // AD: 9 derivative directions
    let hd: [Dual<f64, 9>; 9] = std::array::from_fn(|n| Dual::var(h[n / 3][n % 3], n));
    let pk = neo_hookean_pk1(Dual::<f64, 9>::cst(mo.g), Dual::<f64, 9>::cst(lam), &hd);
    let mut worst = 0.0f64;
    let scale = up.tangent.iter().flatten().fold(0.0f64, |m, v| m.max(v.abs()));
    for n in 0..9 {
        assert!((pk[n].value() - up.stress[n / 3][n % 3]).abs() < 1e-9 * scale, "stress {n}");
        for c in 0..9 {
            worst = worst.max((pk[n].d[c] - up.tangent[n][c]).abs() / scale);
        }
    }
    assert!(worst < 1e-11, "neo-Hookean tangent differs from the closed form by {worst:e}");
}
