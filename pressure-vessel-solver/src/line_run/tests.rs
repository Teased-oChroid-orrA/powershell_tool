use super::*;

const E: f64 = 10.0e6;
const ALPHA: f64 = 12.8e-6;

fn tube() -> Tube {
    Tube { outer_diameter: 1.0, wall: 0.05, e: E, alpha: ALPHA, weight_density: 0.0 }
}

fn ductile() -> MaterialClass {
    MaterialClass::Ductile { yield_strength: 40000.0, ultimate_strength: 60000.0, yield_factor_of_safety: 1.0, ultimate_factor_of_safety: 1.5 }
}

fn sup(x: f64) -> Support {
    Support { x, axial_anchor: false, rotation_fixed: false }
}

fn run(supports: Vec<Support>, x_end: f64, w: f64) -> LineRun {
    LineRun { tube: tube(), x_start: 0.0, x_end, supports, contents_weight_per_length: w, insulation_weight_per_length: 0.0, point_loads: vec![], subdivisions: 1 }
}

fn case(n: [f64; 3]) -> LoadCase {
    LoadCase { name: "c".into(), internal_pressure: 0.0, external_pressure: 0.0, pressure_end_load: false, delta_t: 0.0, load_factors: n, point_loads: vec![] }
}

fn close(a: f64, b: f64, rel: f64, what: &str) {
    assert!((a - b).abs() <= rel * b.abs().max(1e-300) + 1e-12, "{what}: got {a}, expected {b}");
}

fn extreme(r: &CaseResult, f: impl Fn(&Station) -> f64) -> (f64, f64) {
    let lo = r.stations.iter().map(&f).fold(f64::INFINITY, f64::min);
    let hi = r.stations.iter().map(&f).fold(f64::NEG_INFINITY, f64::max);
    (lo, hi)
}

fn one(run: &LineRun, case: LoadCase) -> CaseResult {
    analyze(run, &ductile(), &[case]).unwrap().cases.remove(0)
}

fn reaction_z(r: &CaseResult, x: f64) -> f64 {
    r.reactions.iter().find(|q| (q.x - x).abs() < 1e-9).unwrap().force[2]
}

// ---- beam closed forms --------------------------------------------------------------------

#[test]
fn simply_supported_udl_matches_wl2_over_8_and_wl_over_2() {
    let (l, w) = (100.0, 2.0);
    let r = one(&run(vec![sup(0.0), sup(l)], l, w), case([0.0, 0.0, 1.0]));
    let (lo, hi) = extreme(&r, |s| s.bend_z);
    close(lo, -w * l * l / 8.0, 1e-12, "midspan moment (load up, so negative in the EI w'' convention)");
    assert!(hi.abs() < 1e-9, "ends are moment-free");
    close(reaction_z(&r, 0.0), -w * l / 2.0, 1e-12, "left reaction");
    close(reaction_z(&r, l), -w * l / 2.0, 1e-12, "right reaction");
    close(extreme(&r, |s| s.shear_z.abs()).1, w * l / 2.0, 1e-12, "end shear");
    assert!(r.residual < 1e-12, "residual {}", r.residual);
}

#[test]
fn fixed_fixed_udl_matches_wl2_over_12_and_wl2_over_24() {
    let (l, w) = (80.0, 3.0);
    let fixed = |x| Support { x, axial_anchor: false, rotation_fixed: true };
    let r = one(&run(vec![fixed(0.0), fixed(l)], l, w), case([0.0, 0.0, 1.0]));
    let (lo, hi) = extreme(&r, |s| s.bend_z);
    close(hi, w * l * l / 12.0, 1e-11, "end moment magnitude");
    close(lo, -w * l * l / 24.0, 1e-11, "midspan moment");
    // The clamp reaction moment balances the end moment (about y: -conjugate).
    let m_react = r.reactions[0].moment[1].abs();
    close(m_react, w * l * l / 12.0, 1e-11, "clamp moment reaction");
}

#[test]
fn cantilever_tip_load_gives_root_moment_pl_and_reaction_p() {
    let (l, p) = (50.0, 120.0);
    let mut rn = run(vec![Support { x: 0.0, axial_anchor: false, rotation_fixed: true }], l, 0.0);
    rn.point_loads.push(PointLoad { x: l, force: [0.0, p, 0.0], ..Default::default() });
    let r = one(&rn, case([0.0; 3]));
    close(extreme(&r, |s| s.bend_y.abs()).1, p * l, 1e-12, "root moment");
    close(r.reactions[0].force[1], -p, 1e-12, "root reaction");
    close(r.reactions[0].moment[2].abs(), p * l, 1e-12, "root reaction moment");
}

#[test]
fn simply_supported_point_load_matches_pab_over_l() {
    let (l, a, p) = (100.0, 30.0, 500.0);
    let mut rn = run(vec![sup(0.0), sup(l)], l, 0.0);
    rn.point_loads.push(PointLoad { x: a, force: [0.0, 0.0, -p], ..Default::default() });
    let r = one(&rn, case([0.0; 3]));
    close(extreme(&r, |s| s.bend_z.abs()).1, p * a * (l - a) / l, 1e-12, "moment under the load");
    close(reaction_z(&r, 0.0), p * (l - a) / l, 1e-12, "left reaction");
    close(reaction_z(&r, l), p * a / l, 1e-12, "right reaction");
}

#[test]
fn three_equal_spans_udl_match_the_three_moment_equation() {
    // Clapeyron: M_B = M_C = w L^2 / 10, R = 0.4, 1.1, 1.1, 0.4 w L, end-span peak 0.08 w L^2 at 0.4 L
    // (an interior extremum: subdivisions = 1 puts no node there, so this exercises the exact extremum station).
    let (l, w) = (60.0, 1.5);
    let r = one(&run(vec![sup(0.0), sup(l), sup(2.0 * l), sup(3.0 * l)], 3.0 * l, w), case([0.0, 0.0, 1.0]));
    let (lo, hi) = extreme(&r, |s| s.bend_z);
    close(hi, w * l * l / 10.0, 1e-11, "support moment");
    close(lo, -0.08 * w * l * l, 1e-11, "end-span peak");
    for (x, f) in [(0.0, 0.4), (l, 1.1), (2.0 * l, 1.1), (3.0 * l, 0.4)] {
        close(reaction_z(&r, x), -f * w * l, 1e-11, "reaction");
    }
    let at = r.stations.iter().find(|s| (s.bend_z + 0.08 * w * l * l).abs() < 1e-9 * w * l * l).unwrap();
    close(at.x, 0.4 * l, 1e-11, "peak location");
}

#[test]
fn two_equal_spans_udl_match_the_three_moment_equation() {
    let (l, w) = (40.0, 2.0);
    let r = one(&run(vec![sup(0.0), sup(l), sup(2.0 * l)], 2.0 * l, w), case([0.0, 0.0, 1.0]));
    close(extreme(&r, |s| s.bend_z).1, w * l * l / 8.0, 1e-11, "middle support moment");
    close(reaction_z(&r, l), -1.25 * w * l, 1e-11, "middle reaction");
    close(reaction_z(&r, 0.0), -0.375 * w * l, 1e-11, "end reaction");
}

#[test]
fn reactions_sum_to_total_load_with_valve_point_loads_overhang_and_load_factors() {
    let mut rn = run(vec![anchored(10.0, false), sup(70.0), sup(130.0)], 150.0, 0.8);
    rn.tube.weight_density = 0.1;
    rn.insulation_weight_per_length = 0.05;
    rn.point_loads.push(PointLoad { x: 35.0, weight: 12.0, offset: [3.0, 0.0, 0.0], ..Default::default() });
    let mut c = case([0.3, -0.4, -1.5]);
    c.point_loads.push(PointLoad { x: 100.0, force: [0.0, 20.0, -35.0], ..Default::default() });
    let r = one(&rn, c);
    let w = 0.1 * rn.tube.area() + 0.8 + 0.05;
    let n = [0.3, -0.4, -1.5];
    for k in [0, 1, 2] {
        let total = w * 150.0 * n[k] + 12.0 * n[k] + [0.0, 20.0, -35.0][k];
        let sum: f64 = r.reactions.iter().map(|q| q.force[k]).sum();
        close(sum, -total, 1e-11, "reaction sum balances the applied load");
    }
    assert!(r.residual < 1e-12);
}

#[test]
fn valve_offset_equals_the_explicit_moment_and_the_pure_moment_reaction_is_m0_over_l() {
    let (l, f, d) = (100.0, 40.0, 3.0);
    let mut a = run(vec![sup(0.0), sup(l)], l, 0.0);
    let mut b = a.clone();
    a.point_loads.push(PointLoad { x: 30.0, force: [0.0, 0.0, f], offset: [d, 0.0, 0.0], ..Default::default() });
    b.point_loads.push(PointLoad { x: 30.0, force: [0.0, 0.0, f], moment: [0.0, -f * d, 0.0], ..Default::default() });
    let (ra, rb) = (one(&a, case([0.0; 3])), one(&b, case([0.0; 3])));
    assert_eq!(ra.stations.len(), rb.stations.len());
    for (s, t) in ra.stations.iter().zip(&rb.stations) {
        close(s.bend_z, t.bend_z, 1e-12, "offset vs explicit moment");
    }
    let mut m = run(vec![sup(0.0), sup(l)], l, 0.0);
    m.point_loads.push(PointLoad { x: 30.0, moment: [0.0, -50.0, 0.0], ..Default::default() });
    let r = one(&m, case([0.0; 3]));
    close(reaction_z(&r, 0.0).abs(), 50.0 / l, 1e-12, "couple reaction");
    close(reaction_z(&r, 0.0), -reaction_z(&r, l), 1e-12, "couple reactions are equal and opposite");
}

// ---- pressure ------------------------------------------------------------------------------

#[test]
fn pressure_only_state_is_exact_lame_and_approaches_pd_over_2t_and_pd_over_4t() {
    let mut rn = run(vec![sup(0.0), sup(100.0)], 100.0, 0.0);
    rn.tube = Tube { outer_diameter: 10.0, wall: 0.01, ..tube() };
    let mut c = case([0.0; 3]);
    c.internal_pressure = 100.0;
    c.pressure_end_load = true;
    let r = one(&rn, c);
    let g = rn.tube.geometry().unwrap();
    let lame = crate::stress::stress_at_inner_surface(&g, &PressureLoading::new(100.0, 0.0, EndCondition::Closed).unwrap());
    let s = r.margins[0].stress;
    close(s.sigma_x, lame.axial, 1e-12, "axial equals the exact Lame closed-end value");
    close(s.hoop, lame.hoop, 1e-12, "hoop equals the exact Lame inner value");
    assert_eq!(r.margins[0].location.surface, Surface::Inner);
    // Thin-wall membrane limits; the difference is O(t / r_m), so the tolerance is stated in that unit.
    let (rm, t) = (g.mean_radius(), 0.01);
    let tol = 1.5 * t / rm;
    close(s.hoop, 100.0 * rm / t, tol, "PD/2t");
    close(s.sigma_x, 100.0 * rm / (2.0 * t), tol, "PD/4t");
}

#[test]
fn open_end_line_has_no_pressure_axial_stress() {
    let mut c = case([0.0; 3]);
    c.internal_pressure = 3000.0;
    let r = one(&run(vec![sup(0.0), sup(50.0)], 50.0, 0.0), c);
    assert_eq!(r.margins[0].stress.sigma_x, 0.0);
}

// ---- thermal -------------------------------------------------------------------------------

fn anchored(x: f64, rot: bool) -> Support {
    Support { x, axial_anchor: true, rotation_fixed: rot }
}

#[test]
fn fully_restrained_thermal_stress_is_e_alpha_dt_and_reduces_to_uniaxial_yield() {
    let mut c = case([0.0; 3]);
    c.delta_t = 100.0;
    let r = one(&run(vec![anchored(0.0, false), anchored(100.0, false)], 100.0, 0.0), c);
    let sigma = E * ALPHA * 100.0;
    for s in &r.stations {
        close(s.axial_force, -sigma * tube().area(), 1e-11, "N = -E A alpha dT");
    }
    let vm = &r.margins[0];
    close(vm.stress.sigma_x, -sigma, 1e-11, "restrained stress");
    close(vm.applied, sigma, 1e-11, "von Mises equals the uniaxial stress");
    close(r.margins[1].applied, sigma, 1e-11, "Tresca equals the uniaxial stress");
    close(vm.margin, 40000.0 / sigma - 1.0, 1e-11, "margin = Fy / sigma - 1");
    close(r.margins[2].margin, 60000.0 / (1.5 * sigma) - 1.0, 1e-11, "ultimate margin with factor 1.5");
    assert_eq!(r.margins[3].criterion, Criterion::UltimateVonMises);
    close(r.margins[3].margin, r.margins[2].margin, 1e-11, "von Mises ultimate equals max-principal ultimate in uniaxial stress");
    close(r.reactions[0].force[0], sigma * tube().area(), 1e-11, "anchor reaction");
    close(r.reactions[0].force[0], -r.reactions[1].force[0], 1e-11, "anchor reactions balance");
}

#[test]
fn free_thermal_growth_with_one_anchor_is_stress_free() {
    let mut c = case([0.0; 3]);
    c.delta_t = 250.0;
    let r = one(&run(vec![anchored(0.0, false), sup(100.0)], 100.0, 0.0), c);
    assert!(r.stations.iter().all(|s| s.axial_force.abs() < 1e-6), "no restraint, no axial force");
    assert_eq!(r.margins[0].margin, f64::INFINITY);
}

#[test]
fn an_intermediate_guide_does_not_change_the_anchor_force() {
    // A transverse guide (no axial anchor) between two anchors leaves the restrained thermal force unchanged.
    let mut c = case([0.0; 3]);
    c.delta_t = 80.0;
    let a = one(&run(vec![anchored(0.0, false), sup(30.0), anchored(100.0, false)], 100.0, 0.0), c.clone());
    let b = one(&run(vec![anchored(0.0, false), anchored(100.0, false)], 100.0, 0.0), c);
    close(a.reactions[0].force[0], b.reactions[1].force[0].abs(), 1e-11, "same anchor force");
}

// ---- thin-wall shear, criteria, hand values --------------------------------------------------

#[test]
fn wall_shear_integrates_back_to_the_shear_force_and_peaks_at_2v_over_a() {
    let (rm, t, vy, vz) = (0.475, 0.05, 31.0, -17.0);
    let n = 20000;
    let (mut sy, mut sz) = (0.0, 0.0);
    for i in 0..n {
        let th = (i as f64 + 0.5) * 2.0 * PI / n as f64;
        let q = wall_shear_stress(vy, vz, th, rm, t) * t * rm * (2.0 * PI / n as f64);
        sy += q * -th.sin();
        sz += q * th.cos();
    }
    close(sy, vy, 1e-9, "Vy recovered");
    close(sz, vz, 1e-9, "Vz recovered");
    let area_thin = 2.0 * PI * rm * t;
    close(wall_shear_stress(0.0, 100.0, 0.0, rm, t), 2.0 * 100.0 / area_thin, 1e-12, "tau_max = 2V/A");
}

#[test]
fn hand_computed_multiaxial_state() {
    // sigma_x = 10, hoop = 20, radial = 0, tau = 5 (units arbitrary):
    // principals 15 +- sqrt(25 + 25) = 22.0711, 7.9289, 0; von Mises^2 = 100 - 200 + 400 + 3*25 = 375,
    // Tresca = 22.0711 - 0.
    let s = PointStress { sigma_x: 10.0, hoop: 20.0, radial: 0.0, tau: 5.0 };
    let p = s.principals();
    close(p[0], 15.0 + 50.0_f64.sqrt(), 1e-14, "sigma_1");
    close(p[1], 15.0 - 50.0_f64.sqrt(), 1e-14, "sigma_2");
    let class = MaterialClass::Ductile { yield_strength: 100.0, ultimate_strength: 200.0, yield_factor_of_safety: 1.0, ultimate_factor_of_safety: 1.0 };
    let (u_vm, vm) = utilization_of(&class, Criterion::VonMisesYield, &s, 1.0);
    close(vm, 375.0_f64.sqrt(), 1e-14, "von Mises");
    close(u_vm, 375.0_f64.sqrt() / 100.0, 1e-14, "utilization");
    close(utilization_of(&class, Criterion::TrescaYield, &s, 1.0).1, 15.0 + 50.0_f64.sqrt(), 1e-14, "Tresca");
}

#[test]
fn brittle_criteria_match_hand_values_and_coincide_with_rankine_in_uniaxial_tension() {
    let class = MaterialClass::Brittle { ultimate_tension: 30.0, ultimate_compression: 100.0, factor_of_safety: 1.0 };
    // sigma = (15, 0, -10): Coulomb-Mohr u = 15/30 + 10/100 = 0.6; Rankine u = max(0.5, 0.1) = 0.5.
    let s = PointStress { sigma_x: 15.0, hoop: 0.0, radial: -10.0, tau: 0.0 };
    close(utilization_of(&class, Criterion::CoulombMohr, &s, 1.0).0, 0.6, 1e-14, "Coulomb-Mohr");
    close(utilization_of(&class, Criterion::Rankine, &s, 1.0).0, 0.5, 1e-14, "Rankine");
    let uni = PointStress { sigma_x: 24.0, hoop: 0.0, radial: 0.0, tau: 0.0 };
    close(utilization_of(&class, Criterion::CoulombMohr, &uni, 1.0).0, 0.8, 1e-14, "uniaxial tension, Coulomb-Mohr");
    close(utilization_of(&class, Criterion::Rankine, &uni, 1.0).0, 0.8, 1e-14, "uniaxial tension, Rankine");
    // Pure compression: governed by Suc in both.
    let comp = PointStress { sigma_x: -50.0, hoop: 0.0, radial: 0.0, tau: 0.0 };
    close(utilization_of(&class, Criterion::CoulombMohr, &comp, 1.0).0, 0.5, 1e-14, "compression, Coulomb-Mohr");
    close(utilization_of(&class, Criterion::Rankine, &comp, 1.0).0, 0.5, 1e-14, "compression, Rankine");
    // Equal tension and compression strength: Coulomb-Mohr reduces to Tresca (sigma_1 - sigma_3 = S).
    let eq = MaterialClass::Brittle { ultimate_tension: 40.0, ultimate_compression: 40.0, factor_of_safety: 1.0 };
    let m = PointStress { sigma_x: 15.0, hoop: 0.0, radial: -10.0, tau: 0.0 };
    close(utilization_of(&eq, Criterion::CoulombMohr, &m, 1.0).0, 25.0 / 40.0, 1e-14, "equal strengths");
}

#[test]
fn bending_run_margin_and_worst_angle_match_m_c_over_i() {
    let (l, w) = (100.0, 2.0);
    let rn = run(vec![sup(0.0), sup(l)], l, w);
    let r = one(&rn, case([0.0, 0.0, 1.0]));
    let sigma = (w * l * l / 8.0) * 0.5 / rn.tube.inertia();
    let vm = &r.margins[0];
    close(vm.margin, 40000.0 / sigma - 1.0, 1e-9, "von Mises margin at midspan outer fibre");
    close(vm.location.x, l / 2.0, 1e-12, "governing station");
    assert_eq!(vm.location.surface, Surface::Outer);
    assert!((vm.location.theta.sin().abs() - 1.0).abs() < 1e-9, "worst fibre is at +-z, got theta {}", vm.location.theta);
}

#[test]
fn von_mises_ultimate_is_stricter_than_max_principal_in_shear_and_equal_in_equibiaxial_tension() {
    let class = MaterialClass::Ductile { yield_strength: 100.0, ultimate_strength: 200.0, yield_factor_of_safety: 1.0, ultimate_factor_of_safety: 1.5 };
    // Pure shear tau = 40: principals +-40, von Mises = sqrt(3) * 40, max |principal| = 40.
    let shear = PointStress { sigma_x: 0.0, hoop: 0.0, radial: 0.0, tau: 40.0 };
    let (u_vm, a_vm) = utilization_of(&class, Criterion::UltimateVonMises, &shear, 1.0);
    close(a_vm, 3.0_f64.sqrt() * 40.0, 1e-14, "von Mises in pure shear");
    close(u_vm, 1.5 * 3.0_f64.sqrt() * 40.0 / 200.0, 1e-14, "utilization with FS 1.5");
    close(utilization_of(&class, Criterion::UltimateMaxPrincipal, &shear, 1.0).0, 1.5 * 40.0 / 200.0, 1e-14, "max principal");
    let biax = PointStress { sigma_x: 50.0, hoop: 50.0, radial: 0.0, tau: 0.0 };
    close(utilization_of(&class, Criterion::UltimateVonMises, &biax, 1.0).0, utilization_of(&class, Criterion::UltimateMaxPrincipal, &biax, 1.0).0, 1e-14, "equibiaxial");
}

#[test]
fn pressure_thrust_load_with_pressure_end_load_is_refused_and_accepted_without() {
    let mut rn = run(vec![anchored(0.0, false), anchored(100.0, false)], 100.0, 0.0);
    rn.point_loads.push(PointLoad { x: 40.0, force: [70.0, 0.0, 0.0], pressure_thrust: true, ..Default::default() });
    let mut c = case([0.0; 3]);
    c.internal_pressure = 100.0;
    c.pressure_end_load = true;
    let e = analyze(&rn, &ductile(), &[c.clone()]).unwrap_err();
    assert!(matches!(&e, LineRunError::InvalidInput(s) if s.contains("pressure_end_load")), "{e}");
    c.pressure_end_load = false;
    assert!(analyze(&rn, &ductile(), &[c]).is_ok());
}

#[test]
fn brittle_run_reports_rankine_and_coulomb_mohr() {
    let rn = run(vec![sup(0.0), sup(100.0)], 100.0, 2.0);
    let class = MaterialClass::Brittle { ultimate_tension: 30000.0, ultimate_compression: 90000.0, factor_of_safety: 1.0 };
    let r = analyze(&rn, &class, &[case([0.0, 0.0, 1.0])]).unwrap().cases.remove(0);
    let sigma = (2.0 * 100.0 * 100.0 / 8.0) * 0.5 / rn.tube.inertia();
    assert_eq!(r.margins.iter().map(|m| m.criterion).collect::<Vec<_>>(), vec![Criterion::Rankine, Criterion::CoulombMohr]);
    for m in &r.margins {
        close(m.margin, 30000.0 / sigma - 1.0, 1e-9, "uniaxial bending: tension governs both");
    }
}

#[test]
fn governing_picks_the_lowest_margin_over_cases() {
    let rn = run(vec![sup(0.0), sup(100.0)], 100.0, 2.0);
    let mut heavy = case([0.0, 0.0, 3.0]);
    heavy.name = "3g".into();
    let res = analyze(&rn, &ductile(), &[case([0.0, 0.0, 1.0]), heavy]).unwrap();
    assert_eq!(res.governing().location.case, "3g");
}

// ---- flow thrust ----------------------------------------------------------------------------

fn leg(d: [f64; 3], inflow: bool, mdot: f64) -> FlowLeg {
    FlowLeg { direction: d, inflow, gauge_pressure: 100.0, area: 0.5, mass_flow: mdot, velocity: 10.0 }
}

#[test]
fn bend_thrust_is_p_a_plus_mdot_v_times_d_in_minus_d_out() {
    let f = flow_thrust(&[leg([1.0, 0.0, 0.0], true, 2.0), leg([0.0, 1.0, 0.0], false, 2.0)]).unwrap();
    let mag = 100.0 * 0.5 + 2.0 * 10.0;
    assert_eq!(f, [mag, -mag, 0.0]);
}

#[test]
fn symmetric_tee_thrust_is_along_the_inlet_only() {
    let f = flow_thrust(&[leg([1.0, 0.0, 0.0], true, 2.0), leg([0.0, 1.0, 0.0], false, 1.0), leg([0.0, -1.0, 0.0], false, 1.0)]).unwrap();
    // inlet: pA + mdot v = 70; the two outlets cancel in y (each pA + (mdot/2) v = 60).
    close(f[0], 70.0, 1e-14, "x");
    close(f[1], 0.0, 1e-14, "y");
}

#[test]
fn straight_pipe_with_uniform_section_has_no_thrust() {
    let f = flow_thrust(&[leg([1.0, 0.0, 0.0], true, 2.0), leg([1.0, 0.0, 0.0], false, 2.0)]).unwrap();
    assert_eq!(f, [0.0; 3]);
}

#[test]
fn a_thrust_load_at_a_bend_shows_up_in_the_axial_reactions_of_an_anchored_run() {
    let mut rn = run(vec![anchored(0.0, false), anchored(100.0, false)], 100.0, 0.0);
    rn.point_loads.push(PointLoad { x: 40.0, force: [70.0, 0.0, 0.0], ..Default::default() });
    let r = one(&rn, case([0.0; 3]));
    close(r.reactions[0].force[0] + r.reactions[1].force[0], -70.0, 1e-11, "axial balance");
    close(r.reactions[0].force[0], -42.0, 1e-11, "uniform bar: the anchor at 0 takes 60 % of a load at 40 (the far anchor is 60 away)");
}

// ---- refusals and validation ----------------------------------------------------------------

#[test]
fn composite_and_other_classes_are_refused_with_a_reason() {
    let rn = run(vec![sup(0.0), sup(100.0)], 100.0, 1.0);
    let e = analyze(&rn, &MaterialClass::Composite, &[case([0.0; 3])]).unwrap_err();
    assert!(matches!(&e, LineRunError::Refused(s) if s.contains("ply-level")), "{e}");
    let e = analyze(&rn, &MaterialClass::Other("PTFE".into()), &[case([0.0; 3])]).unwrap_err();
    assert!(matches!(&e, LineRunError::Refused(s) if s.contains("PTFE")));
}

#[test]
fn torque_about_the_line_axis_is_refused() {
    let mut rn = run(vec![sup(0.0), sup(100.0)], 100.0, 0.0);
    rn.point_loads.push(PointLoad { x: 50.0, weight: 10.0, offset: [0.0, 2.0, 0.0], ..Default::default() });
    let e = analyze(&rn, &ductile(), &[case([0.0, 0.0, -1.0])]).unwrap_err();
    assert!(matches!(&e, LineRunError::Refused(s) if s.contains("torque")), "{e}");
}

#[test]
fn a_single_pin_is_a_mechanism_and_an_axial_load_without_an_anchor_is_unstable() {
    let rn = run(vec![sup(0.0)], 100.0, 1.0);
    assert!(matches!(analyze(&rn, &ductile(), &[case([0.0, 0.0, 1.0])]).unwrap_err(), LineRunError::Unstable(_)));
    let rn = run(vec![sup(0.0), sup(100.0)], 100.0, 1.0);
    assert!(matches!(analyze(&rn, &ductile(), &[case([1.0, 0.0, 0.0])]).unwrap_err(), LineRunError::Unstable(_)));
}

#[test]
fn allowables_are_never_defaulted_and_bad_inputs_are_rejected() {
    let rn = run(vec![sup(0.0), sup(100.0)], 100.0, 1.0);
    let zero = MaterialClass::Ductile { yield_strength: 0.0, ultimate_strength: 60000.0, yield_factor_of_safety: 1.0, ultimate_factor_of_safety: 1.5 };
    assert!(matches!(analyze(&rn, &zero, &[case([0.0; 3])]).unwrap_err(), LineRunError::InvalidInput(_)));
    let mut bad = rn.clone();
    bad.tube.wall = 0.6;
    assert!(matches!(analyze(&bad, &ductile(), &[case([0.0; 3])]).unwrap_err(), LineRunError::InvalidInput(_)));
    let mut out = rn.clone();
    out.supports.push(sup(200.0));
    assert!(matches!(analyze(&out, &ductile(), &[case([0.0; 3])]).unwrap_err(), LineRunError::InvalidInput(_)));
    let mut neg = case([0.0; 3]);
    neg.internal_pressure = -1.0;
    assert!(matches!(analyze(&rn, &ductile(), &[neg]).unwrap_err(), LineRunError::InvalidInput(_)));
    assert!(matches!(analyze(&rn, &ductile(), &[]).unwrap_err(), LineRunError::InvalidInput(_)));
}

#[test]
fn stations_converge_independent_of_subdivisions() {
    let (l, w) = (60.0, 1.5);
    let mk = |sub| {
        let mut rn = run(vec![sup(0.0), sup(l), sup(2.0 * l)], 2.0 * l, w);
        rn.subdivisions = sub;
        one(&rn, case([0.0, 0.0, 1.0]))
    };
    let (a, b) = (mk(1), mk(16));
    close(a.margins[0].margin, b.margins[0].margin, 1e-12, "margin does not depend on the station count");
}

#[test]
fn station_margins_minimum_equals_the_case_margins() {
    let rn = run(vec![sup(0.0), sup(100.0)], 100.0, 1.0);
    let res = analyze(&rn, &ductile(), &[case([0.0, 0.0, -1.0])]).unwrap();
    let c = &res.cases[0];
    assert_eq!(c.station_margins.len(), c.stations.len());
    for m in &c.margins {
        let lowest = c.station_margins.iter().flat_map(|s| s.iter()).filter(|(k, _)| *k == m.criterion).map(|(_, v)| *v).fold(f64::INFINITY, f64::min);
        assert!((lowest - m.margin).abs() <= 1e-9 * (1.0 + m.margin.abs()), "{:?}: {lowest} vs {}", m.criterion, m.margin);
    }
}
