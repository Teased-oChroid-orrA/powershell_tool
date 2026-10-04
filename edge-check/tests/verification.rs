//! Verification of the edge-check models against closed forms, equilibrium
//! identities and each other. See `docs/edge-distance-crosscheck.md`.

use edge_check::analytic::AnalyticField;
use edge_check::fem::UnitCase;
use edge_check::field::UnitFieldSource;
use edge_check::model::EdgeModel;
use edge_check::models::analytic_model::AnalyticModel;
use edge_check::models::default_models;
use edge_check::models::fem_model::FemModel;
use edge_check::runner::{run, EdgeConfig, EdgeInput, EdgeMin, TARGETS};
use edge_check::types::{Geometry, Loads, Mode, Strengths};
use mechanics_core::materials::get_material;

const A: f64 = 0.25;
const T: f64 = 0.5;

/// Production-sized plate (3 e or 10 bore radii).
fn geom(ed: f64) -> Geometry {
    let e = ed * 2.0 * A;
    let reach = (3.0 * e).max(10.0 * A);
    Geometry { bore_radius: A, edge: e, thickness: T, plate_far: reach, plate_half_height: reach, plane_angle_deg: 40.0 }
}

/// Plate so large it is a half plane for practical purposes - the analytic
/// model's own idealisation, so the two methods can be compared tightly -
/// and a mesh fine enough for such a plate.
fn geom_half_plane(ed: f64) -> Geometry {
    Geometry { plate_far: 25.0 * A, plate_half_height: 25.0 * A, ..geom(ed) }
}

fn fine_fe() -> FemModel {
    FemModel { mesh: edge_check::fem::MeshSpec { n_radial: 28, n_arc: [4, 6, 30], grade: 1.8 }, ..FemModel::default() }
}

fn al() -> Strengths {
    Strengths::from_material(get_material("al7075"))
}

fn margin(r: &dyn edge_check::model::Response, mode: Mode, loads: Loads) -> f64 {
    r.margins(&loads, 1.0).into_iter().find(|m| m.mode == mode).unwrap().margin
}

#[test]
fn free_edge_hoop_stress_of_a_pressurised_bore_matches_the_closed_form() {
    // Uniform bore pressure p, free-edge point nearest the bore: the
    // stress is exactly 4 p a^2 / (e^2 - a^2) (and sigma_xx = 0 there).
    for e_over_a in [1.5, 2.0, 3.0, 5.0, 8.0] {
        let f = AnalyticField::build(A, e_over_a * A, T, 0.33).unwrap();
        let s = f.stress(UnitCase::Fit, 0.0, 0.0).unwrap();
        let exact = 4.0 * A * A / ((e_over_a * A).powi(2) - A * A);
        assert!((s.yy - exact).abs() < 1e-6 * exact.max(1.0), "e/a={e_over_a}: {} vs {exact}", s.yy);
        assert!(s.xx.abs() < 1e-8, "free edge must be traction free: {s:?}");
    }
}

#[test]
fn the_bore_carries_exactly_the_prescribed_tractions() {
    for ed in [1.0, 1.5, 3.0] {
        let f = AnalyticField::build(A, ed * 2.0 * A, T, 0.33).unwrap();
        for k in 0..=24 {
            let phi = std::f64::consts::PI * k as f64 / 24.0;
            let (x, y) = (ed * 2.0 * A + A * phi.cos(), A * phi.sin());
            let s = f.stress(UnitCase::Fit, x, y).unwrap();
            let (c, sn) = (phi.cos(), phi.sin());
            let srr = s.xx * c * c + s.yy * sn * sn + 2.0 * s.xy * sn * c;
            let srp = (s.yy - s.xx) * sn * c + s.xy * (c * c - sn * sn);
            assert!((srr + 1.0).abs() < 1e-8 && srp.abs() < 1e-8, "e/D={ed} phi={phi}: srr={srr} srp={srp}");
        }
    }
}

#[test]
fn shear_out_equals_the_arc_force_over_twice_t_e_for_both_load_sharing_cases() {
    // Mean shear on the two tangent planes is fixed by x-equilibrium of the
    // strip between them: (force on the bore's loaded half-arc) / (2 t e).
    // Fit pressure p on that half-arc contributes p D t; the pin load
    // contributes P (contact lost on the back) or P/2 (contact retained).
    let mat = al();
    for ed in [1.25, 2.0, 3.0] {
        let g = geom(ed);
        let r = AnalyticModel.respond(&g, &mat, 0.0).unwrap();
        for (fit, pin, share) in [(5000.0, 1000.0, 0.5), (500.0, 4000.0, 1.0), (0.0, 2000.0, 1.0)] {
            let loads = Loads { fit_pressure: fit, pin_load: pin };
            let tau = (pin * share + fit * 2.0 * A * T) / (2.0 * T * g.edge);
            let expected = mat.fsu / tau - 1.0;
            let got = margin(&*r, Mode::ShearOut, loads);
            assert!((got - expected).abs() < 2e-3 * expected.abs().max(1.0), "e/D={ed} fit={fit} pin={pin}: {got} vs {expected}");
        }
    }
}

#[test]
fn contact_retention_follows_the_fit_pressure_threshold() {
    let mat = al();
    let r = AnalyticModel.respond(&geom(2.0), &mat, 0.0).unwrap();
    let threshold = 1000.0 / (std::f64::consts::PI * A * T);
    assert_eq!(r.contact_retained(&Loads { fit_pressure: threshold * 1.01, pin_load: 1000.0 }), Some(true));
    assert_eq!(r.contact_retained(&Loads { fit_pressure: threshold * 0.99, pin_load: 1000.0 }), Some(false));
}

#[test]
fn fe_and_stress_superposition_agree_on_every_mode() {
    let mat = al();
    for ed in [1.25, 1.5, 2.0, 3.0] {
        let g = geom_half_plane(ed);
        let an = AnalyticModel.respond(&g, &mat, 0.0).unwrap();
        let fe = fine_fe().respond(&g, &mat, 0.0).unwrap();
        for loads in [Loads { fit_pressure: 5000.0, pin_load: 1000.0 }, Loads { fit_pressure: 0.0, pin_load: 3000.0 }, Loads { fit_pressure: 3000.0, pin_load: 20000.0 }] {
            for (mode, tol) in [(Mode::ShearOut, 0.01), (Mode::FirstYield, 0.02), (Mode::Splitting, 0.05)] {
                let (a, f) = (margin(&*an, mode, loads), margin(&*fe, mode, loads));
                // Compare the applied/allowable ratios, which are what the
                // two methods actually compute.
                let (ra, rf) = (1.0 / (a + 1.0), 1.0 / (f + 1.0));
                assert!((ra - rf).abs() <= tol * ra.max(rf).max(1e-9), "e/D={ed} {mode:?} {loads:?}: analytic {a} vs FE {f}");
            }
        }
    }
}

#[test]
fn the_finite_plate_changes_the_stresses_by_only_a_few_percent() {
    // Production FE plate (3 e) versus the half plane: the finite-plate
    // effect is real but small, and is what the FE check adds over the
    // analytic model.
    let mat = al();
    let loads = Loads { fit_pressure: 5000.0, pin_load: 3000.0 };
    for ed in [1.5, 2.0, 3.0] {
        let an = AnalyticModel.respond(&geom_half_plane(ed), &mat, 0.0).unwrap();
        let fe = FemModel::default().respond(&geom(ed), &mat, 0.0).unwrap();
        for mode in [Mode::ShearOut, Mode::FirstYield] {
            let (ra, rf) = (1.0 / (margin(&*an, mode, loads) + 1.0), 1.0 / (margin(&*fe, mode, loads) + 1.0));
            assert!((ra - rf).abs() < 0.05 * ra, "e/D={ed} {mode:?}: {ra} vs {rf}");
        }
    }
}

fn input(ed: f64, load: f64, fit: f64, scatter: f64) -> EdgeInput {
    EdgeInput { geom: geom(ed), strengths: al(), applied_load: load, fit_pressure: fit, fit_pressure_min: fit * (1.0 - scatter), fit_pressure_max: fit * (1.0 + scatter) }
}

#[test]
fn the_minimum_edge_distance_is_a_true_root_of_the_margin() {
    let cfg = EdgeConfig { mc_samples: 0, ..EdgeConfig::default() };
    let models: Vec<Box<dyn EdgeModel>> = vec![Box::new(AnalyticModel)];
    let inp = input(2.0, 1000.0, 5000.0, 0.0);
    let rep = run(&models, &inp, &cfg);
    let d = rep.bore_diameter;
    let target_idx = TARGETS.iter().position(|t| t.label.starts_with("Bearing")).unwrap();
    let EdgeMin::Value(e_min) = rep.models[0].targets[target_idx].as_ref().unwrap().e_min else { panic!("expected a root") };
    let t = &TARGETS[target_idx];
    let at = |e: f64| {
        let mut g = inp.geom;
        g.edge = e;
        let r = AnalyticModel.respond(&g, &inp.strengths, 0.0).unwrap();
        let loads = Loads { fit_pressure: inp.fit_pressure, pin_load: inp.strengths.fbru * d * g.thickness };
        r.margins(&loads, 1.0).into_iter().filter(|m| t.modes.contains(&m.mode)).map(|m| m.margin).fold(f64::INFINITY, f64::min)
    };
    assert!(at(e_min) >= -1e-9, "margin at the reported minimum must be non-negative: {}", at(e_min));
    assert!(at(e_min - 0.02 * d) < 0.0, "just below it the check must fail: {}", at(e_min - 0.02 * d));
}

#[test]
fn ultimate_margins_never_decrease_as_the_edge_moves_away() {
    let mat = al();
    let loads = Loads { fit_pressure: 4000.0, pin_load: 15000.0 };
    let mut prev = [f64::NEG_INFINITY; 2];
    for k in 0..16 {
        let ed = 1.0 + 0.2 * k as f64;
        let r = AnalyticModel.respond(&geom(ed), &mat, 0.0).unwrap();
        let cur = [margin(&*r, Mode::ShearOut, loads), margin(&*r, Mode::Splitting, loads)];
        for i in 0..2 {
            assert!(cur[i] >= prev[i] - 1e-9, "mode {i} fell at e/D={ed}: {} < {}", cur[i], prev[i]);
        }
        prev = cur;
    }
}

#[test]
fn monte_carlo_is_reproducible_and_consistent_with_the_deterministic_margin() {
    let models: Vec<Box<dyn EdgeModel>> = vec![Box::new(AnalyticModel)];
    let cfg = EdgeConfig { mc_samples: 400, ..EdgeConfig::default() };
    let seq = TARGETS.iter().position(|t| t.label.starts_with("Bearing")).unwrap();

    // Comfortably passing and comfortably failing cases.
    let pass = run(&models, &input(3.0, 1000.0, 5000.0, 0.4), &cfg);
    let fail = run(&models, &input(1.0, 1000.0, 5000.0, 0.4), &cfg);
    assert_eq!(pass.models[0].targets[seq].as_ref().unwrap().mc.as_ref().unwrap().p_fail, 0.0);
    assert_eq!(fail.models[0].targets[seq].as_ref().unwrap().mc.as_ref().unwrap().p_fail, 1.0);

    // Same seed, same result.
    let again = run(&models, &input(3.0, 1000.0, 5000.0, 0.4), &cfg);
    assert_eq!(pass.models[0].targets[seq].as_ref().unwrap().mc, again.models[0].targets[seq].as_ref().unwrap().mc);

    // Marginal: the fit-pressure band straddles the pass/fail boundary, so
    // the failure probability is strictly between 0 and 1 and equals the
    // fraction of the uniform band on the failing side.
    let mat = al();
    let ed = 1.5;
    let g = geom(ed);
    let r = AnalyticModel.respond(&g, &mat, 0.0).unwrap();
    let load = mat.fbru * 2.0 * A * T;
    let margin_at = |p: f64| margin(&*r, Mode::ShearOut, Loads { fit_pressure: p, pin_load: load }).min(margin(&*r, Mode::Splitting, Loads { fit_pressure: p, pin_load: load }));
    // Margin falls linearly with p (shear-out) - find where it crosses zero.
    let (m0, m1) = (margin_at(0.0), margin_at(40_000.0));
    assert!(m0 > 0.0 && m1 < 0.0, "fixture must straddle zero: {m0}, {m1}");
    let (mut lo, mut hi) = (0.0, 40_000.0);
    for _ in 0..60 {
        let mid = 0.5 * (lo + hi);
        if margin_at(mid) > 0.0 { lo = mid } else { hi = mid }
    }
    let p_star = lo;
    let rep = run(&models, &EdgeInput { geom: g, strengths: mat, applied_load: 1000.0, fit_pressure: 5000.0, fit_pressure_min: 0.0, fit_pressure_max: 40_000.0 }, &EdgeConfig { mc_samples: 4000, ..EdgeConfig::default() });
    let got = rep.models[0].targets[seq].as_ref().unwrap().mc.as_ref().unwrap().p_fail;
    let expected = 1.0 - p_star / 40_000.0;
    assert!((got - expected).abs() < 0.01, "MC p_fail {got} vs analytic {expected}");
}

#[test]
fn every_default_model_evaluates_a_typical_case_without_error() {
    let cfg = EdgeConfig { mc_samples: 200, ..EdgeConfig::default() };
    let rep = run(&default_models(&cfg), &input(1.5, 1000.0, 8000.0, 0.3), &cfg);
    for m in &rep.models {
        assert!(m.error.is_none(), "{}: {:?}", m.label, m.error);
    }
    // The two stress models agree on the minimum edge distance.
    for (i, target) in TARGETS.iter().enumerate() {
        if let Some(agree) = rep.e_min_agree(i, 0.10) {
            assert!(agree, "{}: {:?}", target.label, rep.e_min_spread(i));
        }
    }
}

#[test]
fn degenerate_inputs_are_reported_not_panicked() {
    let cfg = EdgeConfig { mc_samples: 10, ..EdgeConfig::default() };
    let mut bad = input(1.5, 1000.0, 5000.0, 0.1);
    bad.geom.edge = bad.geom.bore_radius * 0.9; // edge inside the bore
    let rep = run(&default_models(&cfg), &bad, &cfg);
    assert!(rep.models.iter().filter(|m| m.field_model).all(|m| m.error.is_some()));
    // Zero load: nothing can fail, nothing may panic.
    let zero = run(&default_models(&cfg), &input(1.5, 0.0, 0.0, 0.0), &cfg);
    assert!(zero.models.iter().all(|m| m.error.is_none()));
}

#[test]
fn the_plastic_limit_load_is_more_conservative_than_the_mean_shear_assumption() {
    // The stress models take mean tangent-plane shear = Fsu (full
    // redistribution). The elastic-plastic solution computes the failure
    // instead; it must not find the edge *stronger* than that upper-bound
    // style assumption, and its capacity must stay below the kinematic bound.
    let cfg = EdgeConfig { mc_samples: 0, include_plastic: true, ..EdgeConfig::default() };
    let rep = run(&default_models(&cfg), &input(1.5, 1000.0, 5000.0, 0.0), &cfg);
    let seq = TARGETS.iter().position(|t| t.label.starts_with("Bearing")).unwrap();
    let e_min = |id: &str| match rep.models.iter().find(|m| m.id == id).unwrap().targets[seq].as_ref().unwrap().e_min {
        EdgeMin::Value(v) => v,
        other => panic!("{id}: {other:?}"),
    };
    let (shear, plastic) = (e_min("analytic"), e_min("plastic"));
    assert!(plastic >= shear * 0.99, "plastic {plastic} must not be below the mean-shear limit {shear}");
    assert!(plastic <= 3.0 * rep.bore_diameter, "plastic {plastic} implausibly large");
}

#[test]
fn reported_capacity_is_the_load_where_the_margin_crosses_zero_and_the_fit_costs_capacity() {
    let models: Vec<Box<dyn EdgeModel>> = vec![Box::new(AnalyticModel), Box::new(edge_check::models::allowable::AllowableModel::default())];
    let rep = run(&models, &input(2.0, 1000.0, 8000.0, 0.0), &EdgeConfig { mc_samples: 0, ..EdgeConfig::default() });
    let si = TARGETS.iter().position(|t| t.label.starts_with("Strength")).unwrap();
    // Stress model: margin at capacity is 0, the fit removes capacity.
    let t = rep.models[0].targets[si].as_ref().unwrap();
    let r = AnalyticModel.respond(&geom(2.0), &al(), 8000.0).unwrap();
    let at = |load: f64| TARGETS[si].modes.iter().filter_map(|m| r.margins(&Loads { fit_pressure: 8000.0, pin_load: load }, 1.0).into_iter().find(|x| x.mode == *m).map(|x| x.margin)).fold(f64::INFINITY, f64::min);
    assert!(at(t.capacity_lbf * 0.999) > 0.0 && at(t.capacity_lbf * 1.001) < 0.0, "capacity {}", t.capacity_lbf);
    assert!(t.capacity_no_fit_lbf > t.capacity_lbf);
    // Tabulated shear-out: capacity is exactly 2 Fsu t L - p D t (bearing is far higher here).
    let a = rep.models[1].targets[si].as_ref().unwrap();
    let l = 1.0 - A * 40f64.to_radians().cos();
    let shear = 2.0 * al().fsu * T * l;
    let bearing = al().fbru * 2.0 * A * T;
    let expect = (shear - 8000.0 * 2.0 * A * T).min(bearing);
    assert!((a.capacity_lbf - expect).abs() < 1e-3 * expect, "{} vs {expect}", a.capacity_lbf);
    assert!((a.capacity_no_fit_lbf - shear.min(bearing)).abs() < 1e-3 * shear);
}
