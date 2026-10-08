//! Closed-form and equilibrium checks of the eccentric-bushing solver.

use eccentric_bushing::{analyze, Elasticity, Inputs};
use mechanics_core::lame::diametral_interference_compliance as compliance;
use std::f64::consts::PI;

fn base() -> Inputs {
    Inputs {
        bore_dia: 0.5,
        housing_od: 1.25,
        edge_distance: None,
        bushing_id: 0.38,
        offset: 0.0,
        interference_dia: 0.002,
        thickness: 0.5,
        housing: Elasticity::iso(10.4e6, 0.33),
        bushing: Elasticity::iso(29.0e6, 0.30),
        friction: 0.15,
        pin: Elasticity::iso(29.0e6, 0.30),
        pin_friction: 0.1,
        pin_clearance_dia: 0.001,
        credit_pin_load: true,
        load_lbf: 0.0,
        load_angle_deg: 90.0,
        direct_onset: false,
        min_wall: 0.0,
        plane_strain: false,
        mesh_size: None,
    }
}

/// Two-cylinder Lame fit pressure of the concentric limit (the formula `bushing-solver` uses).
fn lame_pressure(i: &Inputs) -> f64 {
    let (r, ri, ro) = (i.bore_dia / 2.0, i.bushing_id / 2.0, i.housing_od / 2.0);
    let ch = compliance(r, ro, r, 1.0, 0.0, i.housing.e_psi, i.housing.nu);
    let cb = compliance(ri, r, r, 0.0, 1.0, i.bushing.e_psi, i.bushing.nu);
    i.interference_dia / (ch + cb)
}

#[test]
fn concentric_fit_matches_lame_pressure_and_the_classical_torque() {
    let i = base();
    let a = analyze(&i).unwrap();
    let p = lame_pressure(&i);
    let rel = |x: f64, y: f64| (x - y).abs() / y;
    eprintln!("p_mean {} lame {} min {} max {} Tcap {} classical {}", a.fit_pressure_mean, p, a.fit_pressure_min, a.fit_pressure_max, a.torque_capacity_fit, 2.0 * PI * i.friction * p * (i.bore_dia / 2.0).powi(2) * i.thickness);
    assert!(rel(a.fit_pressure_mean, p) < 0.01, "mean pressure {} vs Lame {}", a.fit_pressure_mean, p);
    assert!(rel(a.fit_pressure_max, a.fit_pressure_min + 1e-9) < 0.05 || (a.fit_pressure_max - a.fit_pressure_min) / p < 0.05, "concentric pressure must be uniform");
    let classical = 2.0 * PI * i.friction * p * (i.bore_dia / 2.0).powi(2) * i.thickness;
    assert!(rel(a.torque_capacity_fit, classical) < 0.01, "T_cap {} vs 2 pi mu p R^2 L {}", a.torque_capacity_fit, classical);
}

fn coarse(i: Inputs) -> Inputs {
    Inputs { mesh_size: Some(0.05), ..i }
}

/// The conservative basis (fit alone): a loaded run is not needed to judge the margin, so searches stay fast.
fn fit_basis(i: Inputs) -> Inputs {
    Inputs { credit_pin_load: false, ..coarse(i) }
}

#[test]
fn global_equilibrium_holds_for_an_eccentric_bushing_under_load() {
    // The FE interface tractions must return the pin force and the pin's moment about the bushing centre: a check of the
    // solution that uses no formula (the friction torque is read from the contact state, not from `T_req`).
    let i = coarse(Inputs { offset: 0.04, load_lbf: 1500.0, ..base() });
    let a = analyze(&i).unwrap();
    assert!((a.net_force[1] - 1500.0).abs() < 0.01 * 1500.0, "net force {:?}", a.net_force);
    assert!(a.net_force[0].abs() < 0.01 * 1500.0, "net force {:?}", a.net_force);
    assert!((a.friction_torque - a.torque_required).abs() < 0.02 * a.torque_required, "friction torque {} vs F e {}", a.friction_torque, a.torque_required);
    assert!(a.slip_share < 1.0 && a.margin > 0.0);
    assert!(a.ground_leak < 2e-3 * 1500.0, "the springs that keep the bodies from floating leak {} lbf of 1500", a.ground_leak);
}

#[test]
fn the_result_does_not_depend_on_the_mesh() {
    let i = Inputs { offset: 0.04, load_lbf: 1500.0, ..base() };
    let (a, b) = (analyze(&Inputs { mesh_size: Some(0.05), ..i }).unwrap(), analyze(&Inputs { mesh_size: Some(0.03), ..i }).unwrap());
    let rel = |x: f64, y: f64| (x - y).abs() / y;
    assert!(rel(a.torque_capacity_fit, b.torque_capacity_fit) < 0.01, "fit {} vs {}", a.torque_capacity_fit, b.torque_capacity_fit);
    assert!(rel(a.torque_capacity, b.torque_capacity) < 0.01, "loaded {} vs {}", a.torque_capacity, b.torque_capacity);
}

#[test]
fn an_offset_lowers_the_capacity_and_the_pressure_is_higher_on_the_thick_side() {
    let (c, e) = (analyze(&coarse(base())).unwrap(), analyze(&coarse(Inputs { offset: 0.04, ..base() })).unwrap());
    assert!(e.torque_capacity_fit < c.torque_capacity_fit, "eccentric {} vs concentric {}", e.torque_capacity_fit, c.torque_capacity_fit);
    // The bore centre is at +x, so the wall is thinnest at theta = 0 and thickest at 180 degrees.
    let at = |th: f64| e.profile.iter().min_by(|a, b| (a.angle_deg - th).abs().total_cmp(&(b.angle_deg - th).abs())).unwrap().fit;
    assert!(at(182.5) > 1.2 * at(2.5), "thick {} thin {}", at(182.5), at(2.5));
}

#[test]
fn reversing_the_load_gives_the_same_demand_and_capacity() {
    let i = coarse(Inputs { offset: 0.04, load_lbf: 1500.0, ..base() });
    let (up, down) = (analyze(&i).unwrap(), analyze(&Inputs { load_angle_deg: 270.0, ..i }).unwrap());
    assert!((up.torque_required - down.torque_required).abs() < 1e-9);
    assert!((up.torque_capacity - down.torque_capacity).abs() < 0.01 * up.torque_capacity, "{} vs {}", up.torque_capacity, down.torque_capacity);
}

#[test]
fn a_load_along_the_offset_line_has_no_spin_torque() {
    let a = analyze(&coarse(Inputs { offset: 0.04, load_lbf: 1500.0, load_angle_deg: 0.0, ..base() })).unwrap();
    assert!(a.torque_required.abs() < 1e-9 && a.margin.is_infinite());
}

#[test]
fn the_maximum_offset_is_where_the_margin_changes_sign() {
    use eccentric_bushing::max_offset;
    let i = fit_basis(Inputs { interference_dia: 0.0006, load_lbf: 3000.0, ..base() });
    let lim = max_offset(&i, 0.02).unwrap();
    assert!(!lim.bounded_by_wall && lim.value > 0.0);
    let below = analyze(&Inputs { offset: 0.95 * lim.value, ..i }).unwrap();
    let above = analyze(&Inputs { offset: 1.05 * lim.value, ..i }).unwrap();
    assert!(below.margin > 0.0 && above.margin < 0.0, "margins {} / {} around e_max {}", below.margin, above.margin, lim.value);
}

#[test]
fn the_maximum_load_is_where_the_margin_changes_sign() {
    use eccentric_bushing::max_load;
    let i = fit_basis(Inputs { interference_dia: 0.0006, offset: 0.03, load_lbf: 1000.0, ..base() });
    let lim = max_load(&i, 0.02).unwrap();
    let below = analyze(&Inputs { load_lbf: 0.95 * lim.value, ..i }).unwrap();
    let above = analyze(&Inputs { load_lbf: 1.05 * lim.value, ..i }).unwrap();
    assert!(below.margin > 0.0 && above.margin < 0.0, "margins {} / {} around F {}", below.margin, above.margin, lim.value);
}

#[test]
fn the_minimum_wall_is_reported_against_the_entered_requirement() {
    let i = fit_basis(Inputs { offset: 0.03, min_wall: 0.05, ..base() });
    let a = analyze(&i).unwrap();
    assert!((a.wall_thin - (0.06 - 0.03)).abs() < 1e-12 && !a.wall_ok, "{a:?}");
    assert!(analyze(&Inputs { min_wall: 0.02, ..i }).unwrap().wall_ok);
    let lim = eccentric_bushing::max_offset(&Inputs { interference_dia: 0.002, load_lbf: 100.0, ..i }, 0.05).unwrap();
    assert!((lim.wall_limit - (0.06 - 0.05)).abs() < 1e-12, "wall limit {}", lim.wall_limit);
    assert!(lim.bounded_by_wall, "100 lbf never spins this fit: {lim:?}");
}

#[test]
fn invalid_inputs_are_errors_not_panics() {
    assert!(analyze(&Inputs { offset: 0.06, ..base() }).is_err(), "offset eats the wall");
    assert!(analyze(&Inputs { offset: 0.055, ..base() }).is_err(), "below the numerical wall floor");
    assert!(analyze(&Inputs { interference_dia: 0.0, ..base() }).is_err());
    assert!(analyze(&Inputs { friction: 0.0, ..base() }).is_err());
    assert!(analyze(&Inputs { bushing_id: 0.6, ..base() }).is_err());
}

#[test]
fn the_pin_is_a_real_body_whose_pressure_spreads_with_its_stiffness_and_clearance() {
    let i = coarse(Inputs { offset: 0.04, load_lbf: 1500.0, ..base() });
    let tight = analyze(&Inputs { pin_clearance_dia: 0.0005, ..i }).unwrap();
    let loose = analyze(&Inputs { pin_clearance_dia: 0.004, ..i }).unwrap();
    // A looser pin bears on a narrower arc at a higher peak pressure (conformity), and the mean it exerts on the
    // bushing is what raises the loaded capacity above the fit's.
    assert!(loose.pin_arc_deg < tight.pin_arc_deg && loose.pin_peak_pressure > tight.pin_peak_pressure, "arc {} vs {}, peak {} vs {}", loose.pin_arc_deg, tight.pin_arc_deg, loose.pin_peak_pressure, tight.pin_peak_pressure);
    for a in [&tight, &loose] {
        assert!(a.torque_capacity > a.torque_capacity_fit, "the pin's pressure squeezes the bushing: {} vs {}", a.torque_capacity, a.torque_capacity_fit);
        assert!((a.net_force[1] - 1500.0).abs() < 0.01 * 1500.0, "the interface returns the pin load: {:?}", a.net_force);
    }
    // The capacity hardly depends on the pin's clearance (the squeeze is set by the load, not the contact shape).
    assert!((tight.torque_capacity / loose.torque_capacity - 1.0).abs() < 0.05, "{} vs {}", tight.torque_capacity, loose.torque_capacity);
    // A soft pin (aluminium) is solved just as well, and the interface still returns the load.
    let soft = analyze(&Inputs { pin: Elasticity::iso(10.4e6, 0.33), ..i }).unwrap();
    assert!((soft.net_force[1] - 1500.0).abs() < 0.01 * 1500.0, "{:?}", soft.net_force);
}

#[test]
fn the_pin_is_free_to_rotate_so_its_friction_does_not_change_the_torque_the_bushing_must_return() {
    let i = coarse(Inputs { offset: 0.04, load_lbf: 1500.0, ..base() });
    let (smooth, rough) = (analyze(&Inputs { pin_friction: 0.0, ..i }).unwrap(), analyze(&Inputs { pin_friction: 0.4, ..i }).unwrap());
    for a in [&smooth, &rough] {
        assert!((a.friction_torque - a.torque_required).abs() < 0.02 * a.torque_required, "the moment about the bushing is F e sin(a) whatever the pin friction: {} vs {}", a.friction_torque, a.torque_required);
    }
}

#[test]
fn a_light_pin_load_is_solved_and_checked_too() {
    let a = analyze(&coarse(Inputs { offset: 0.02, load_lbf: 100.0, ..base() })).unwrap();
    assert!((a.friction_torque - a.torque_required).abs() < 0.05 * a.torque_required + 0.2, "{} vs {}", a.friction_torque, a.torque_required);
}

/// Diagnostic (slow): the graded automatic mesh against uniform fine meshes on a thin-wall case.
/// `cargo test -p eccentric-bushing --release --test verification -- --ignored --nocapture thin_wall`
#[test]
#[ignore]
fn thin_wall_mesh_convergence() {
    let i = Inputs { bore_dia: 0.5, bushing_id: 0.375, interference_dia: 0.0015, offset: 0.05, load_lbf: 1000.0, ..base() };
    for h in [None, Some(0.02), Some(0.0125)] {
        let t = std::time::Instant::now();
        match analyze(&Inputs { mesh_size: h, ..i }) {
            Ok(a) => eprintln!("h {h:?}: Tfit {:.3} Tcap {:.3} Tfric {:.3} Treq {:.3} dofs {} {:.1}s", a.torque_capacity_fit, a.torque_capacity, a.friction_torque, a.torque_required, a.dofs, t.elapsed().as_secs_f64()),
            Err(m) => eprintln!("h {h:?}: ERR {m}"),
        }
    }
}

/// Direct simulation of the spin: a pure torque on the bore, raised under arc-length control until the interface slips
/// all round. This is slow (a minute or two) and is run on demand: `cargo test -p eccentric-bushing --release --test
/// verification -- --ignored --nocapture spin_onset`.
#[test]
#[ignore]
fn the_integral_capacity_matches_the_direct_spin_onset_concentric_and_overstates_it_when_the_wall_varies() {
    let concentric = coarse(Inputs { offset: 0.0, ..base() });
    let cap = analyze(&concentric).unwrap().torque_capacity_fit;
    let direct = eccentric_bushing::spin_onset_torque(&concentric).unwrap().expect("the onset is within 2.5 x the capacity");
    eprintln!("concentric: integral {cap:.2}, direct onset {direct:.2} ({:+.2} %)", 100.0 * (direct / cap - 1.0));
    assert!((direct / cap - 1.0).abs() < 0.02, "concentric: {direct} vs {cap}");
    let eccentric = coarse(Inputs { offset: 0.04, ..base() });
    let cap_e = analyze(&eccentric).unwrap().torque_capacity_fit;
    let direct_e = eccentric_bushing::spin_onset_torque(&eccentric).unwrap().expect("the onset is within 2.5 x the capacity");
    eprintln!("e = 0.04: integral {cap_e:.2}, direct onset {direct_e:.2} ({:+.2} %)", 100.0 * (direct_e / cap_e - 1.0));
    // Measured: the all-slip integral over the fit pressure is 3-4 % above the true onset when the wall varies by 3:1
    // (the torque redistributes the pressure before the interface lets go); never below it.
    assert!(direct_e <= 1.01 * cap_e && direct_e >= 0.90 * cap_e, "eccentric: {direct_e} vs {cap_e}");
}

/// The pointwise pressure of a concentric fit is uniform to a few hundredths of a percent with the reduced collocation
/// rule the interface uses (measured: reduced 0.02 % standard deviation over the angular bins, full Gauss 0.03 %, nodal
/// collocation does not solve the deformable interface); the scatter on non-matching meshes is what the integral outputs
/// (capacity, net force, friction moment) are used to avoid.
#[test]
fn the_concentric_fit_pressure_is_uniform_pointwise() {
    let a = analyze(&coarse(Inputs { offset: 0.0, ..base() })).unwrap();
    let mean = a.fit_pressure_mean;
    let sd = (a.profile.iter().map(|b| (b.fit - mean).powi(2)).sum::<f64>() / a.profile.len() as f64).sqrt();
    assert!(sd < 2e-3 * mean, "sd {sd} of mean {mean}");
}

#[test]
fn an_edge_limited_housing_lowers_the_pressure_on_the_edge_side_and_the_capacity() {
    let boss = analyze(&coarse(Inputs { housing_od: 5.0, ..base() })).unwrap(); // a boss of 10 bore diameters: practically infinite
    let plate = analyze(&coarse(Inputs { edge_distance: Some(0.4), ..base() })).unwrap(); // 0.15 in of ligament beside the bore
    // The free edge is on the -x side (theta = 180 deg): the soft ligament there carries a lower fit pressure than the far side.
    let at = |a: &eccentric_bushing::Analysis, th: f64| a.profile.iter().min_by(|x, y| (x.angle_deg - th).abs().total_cmp(&(y.angle_deg - th).abs())).unwrap().fit;
    assert!(at(&plate, 182.5) < 0.9 * at(&plate, 2.5), "edge side {} vs far side {}", at(&plate, 182.5), at(&plate, 2.5));
    assert!(plate.torque_capacity_fit < boss.torque_capacity_fit, "plate {} vs boss {}", plate.torque_capacity_fit, boss.torque_capacity_fit);
    assert!(analyze(&coarse(Inputs { edge_distance: Some(0.2), ..base() })).is_err(), "no ligament left");
}

#[test]
fn an_orthotropic_housing_is_the_isotropic_one_when_its_constants_are_isotropic_and_turns_with_its_axes() {
    use eccentric_bushing::Orthotropy;
    let iso = base().housing;
    let e = iso.e_psi;
    let as_ortho = |angle: f64, e2: f64| Elasticity { ortho: Some(Orthotropy { e2_psi: e2, g12_psi: e / (2.0 * (1.0 + iso.nu)), nu12: iso.nu, angle_deg: angle }), ..iso };
    let reference = analyze(&coarse(Inputs { offset: 0.0, ..base() })).unwrap();
    let same = analyze(&coarse(Inputs { offset: 0.0, housing: as_ortho(0.0, e), ..base() })).unwrap();
    assert!((same.torque_capacity_fit / reference.torque_capacity_fit - 1.0).abs() < 1e-4, "{} vs {}", same.torque_capacity_fit, reference.torque_capacity_fit);
    // Stiffer along its axis 1 than along 2: the fit pressure follows the housing's stiffness around the bore, and
    // turning the axes by 90 degrees turns the pressure pattern with them.
    let along_x = analyze(&coarse(Inputs { offset: 0.0, housing: as_ortho(0.0, 0.4 * e), ..base() })).unwrap();
    let along_y = analyze(&coarse(Inputs { offset: 0.0, housing: as_ortho(90.0, 0.4 * e), ..base() })).unwrap();
    let at = |a: &eccentric_bushing::Analysis, th: f64| a.profile.iter().min_by(|x, y| (x.angle_deg - th).abs().total_cmp(&(y.angle_deg - th).abs())).unwrap().fit;
    assert!((at(&along_x, 2.5) / at(&along_x, 92.5) - 1.0).abs() > 0.03, "the pressure must vary with the housing's stiffness: {} vs {}", at(&along_x, 2.5), at(&along_x, 92.5));
    for th in [2.5, 47.5, 92.5, 137.5] {
        let turned = at(&along_y, (th + 90.0_f64).rem_euclid(360.0));
        assert!((at(&along_x, th) / turned - 1.0).abs() < 0.01, "pattern at {th} deg: {} vs turned {}", at(&along_x, th), turned);
    }
}

/// Diagnostic (slow, minutes): the default mesh against a fine uniform one on the Bushing Workbench's default case:
/// capacity 268.03 vs 268.05 lbf in at e = 0.02 (9.8k vs 56k dofs), 252.73 vs 252.52 at e = 0.04.
#[test]
#[ignore]
fn default_mesh_convergence() {
    let i = Inputs { bore_dia: 0.5, housing_od: 1.25, bushing_id: 0.375, interference_dia: 0.0015, thickness: 0.5, housing: Elasticity::iso(10.3e6, 0.33), bushing: Elasticity::iso(17.0e6, 0.34), friction: 0.15, pin: Elasticity::iso(29.0e6, 0.3), pin_friction: 0.1, pin_clearance_dia: 0.001, credit_pin_load: true, load_lbf: 1000.0, load_angle_deg: 90.0, min_wall: 0.0, ..base() };
    for e in [0.02, 0.04, 0.05] {
        for h in [None, Some(0.015)] {
            let t = std::time::Instant::now();
            match analyze(&Inputs { offset: e, mesh_size: h, ..i }) {
                Ok(a) => eprintln!("e {e} h {h:?}: Tfit {:.2} Tcap {:.2} dofs {} {:.1}s", a.torque_capacity_fit, a.torque_capacity, a.dofs, t.elapsed().as_secs_f64()),
                Err(m) => eprintln!("e {e} h {h:?}: ERR {m}"),
            }
        }
    }
}

#[test]
fn a_search_that_is_interrupted_returns_what_it_verified_instead_of_failing() {
    use eccentric_bushing::{max_load_with, max_offset_with, Control, Interrupt};
    use std::sync::{atomic::AtomicBool, Arc};
    let i = fit_basis(Inputs { interference_dia: 0.0006, load_lbf: 3000.0, ..base() });
    // A cancel raised before the search starts: no candidate completes, the whole range is still open.
    let cancelled: Control = Interrupt { deadline: None, cancel: Some(Arc::new(AtomicBool::new(true))) }.into();
    let started = std::time::Instant::now();
    let lim = max_offset_with(&i, 0.02, &cancelled).unwrap();
    assert!(started.elapsed().as_secs_f64() < 5.0, "an interrupted search must return at once");
    assert_eq!(lim.halted.as_deref(), Some("cancelled"));
    assert_eq!(lim.value, 0.0);
    assert!(lim.upper.is_some_and(|u| u > 0.0), "the unresolved bracket is reported: {lim:?}");
    // The same through a deadline that has already passed.
    let late: Control = Interrupt { deadline: Some(std::time::Instant::now()), cancel: None }.into();
    assert_eq!(max_offset_with(&i, 0.02, &late).unwrap().halted.as_deref(), Some("time budget used up"));
    // max_load has nothing to report before its first (closed-form) solve finishes: a clear error, not a hang.
    let e = max_load_with(&Inputs { offset: 0.04, ..i }, 0.02, &cancelled).unwrap_err();
    assert!(e.contains("cancelled"), "{e}");
}

/// Slow (about 70 s): the Bushing Workbench defaults at 0.04 in offset and 1500 lbf on the earlier default mesh (0.031 in
/// elements). The first attempt of the loaded stage falls into a stick-slip limit cycle and fails; the second (with step
/// memory) converges, to the margin every other mesh and attempt gives (5.92).
#[test]
#[ignore = "slow: run with --ignored"]
fn the_fine_mesh_default_case_at_1500_lbf_converges_through_the_second_attempt() {
    let a = analyze(&Inputs { offset: 0.04, load_lbf: 1500.0, mesh_size: Some(0.031), ..base() }).unwrap_or_else(|e| panic!("{e}"));
    assert!(a.loaded_failure.is_none() && (a.margin - 5.921).abs() < 0.03, "margin {} {:?}", a.margin, a.loaded_failure);
}

/// The bushing the user reported (bore 0.5, ID 0.1875, length 0.125 in, interference 0.0025, friction 0.15, 1000 lbf,
/// boss 1.25): `max_load` probes 8856 lbf, where the rigid pressing-in stage used up its prescribed travel before the pin
/// carried a quarter of the load ("the pin could not be pressed in to 2214 lbf (Completed)"). The travel is extended.
fn reported_bushing() -> Inputs {
    Inputs {
        bore_dia: 0.5,
        housing_od: 1.25,
        edge_distance: None,
        bushing_id: 0.1875,
        offset: 0.02,
        interference_dia: 0.0025,
        thickness: 0.125,
        housing: Elasticity::iso(10.4e6, 0.33),
        bushing: Elasticity::iso(10.8e6, 0.30),
        friction: 0.15,
        pin: Elasticity::iso(29.0e6, 0.30),
        pin_friction: 0.1,
        pin_clearance_dia: 0.001,
        credit_pin_load: true,
        load_lbf: 1000.0,
        load_angle_deg: 90.0,
        direct_onset: false,
        min_wall: 0.0,
        plane_strain: false,
        mesh_size: None,
    }
}

#[test]
fn the_reported_8856_lbf_case_presses_the_pin_in_and_converges() {
    let a = analyze(&Inputs { load_lbf: 8856.0, ..reported_bushing() }).unwrap_or_else(|e| panic!("{e}"));
    assert!(a.loaded_failure.is_none(), "{:?}", a.loaded_failure);
    assert!((a.margin - 1.442).abs() < 0.01, "margin {}", a.margin);
    assert!((a.net_force[1].abs() - 8856.0).abs() < 0.03 * 8856.0, "net force {:?}", a.net_force);
    // The margin must fall with the load (the squeeze grows slower than the demand).
    let light = analyze(&reported_bushing()).unwrap();
    assert!(light.margin > a.margin, "{} vs {}", light.margin, a.margin);
}

#[test]
fn a_small_bushing_whose_friction_contact_chatters_still_converges() {
    // Bore 0.25 in, 113 lbf: with the strict tolerance the loaded stage crawled through 24 steps and 657 factorisations
    // (100 s) to margin 51.361; accepting the stalled residual (below 0.3 % of the force scale) takes seconds.
    let i = Inputs {
        bore_dia: 0.253539395644151,
        housing_od: 0.6394422267443205,
        edge_distance: None,
        bushing_id: 0.17230006325942043,
        offset: 0.01143373248833315,
        interference_dia: 0.001887110271978934,
        thickness: 0.16344989404120458,
        housing: Elasticity::iso(16.5e6, 0.3376056751374146),
        bushing: Elasticity::iso(15.0e6, 0.28855486018848214),
        friction: 0.14385111882242554,
        pin: Elasticity::iso(16.5e6, 0.30),
        pin_friction: 0.18818329575543752,
        pin_clearance_dia: 0.0015056058644689848,
        credit_pin_load: true,
        load_lbf: 113.23032109379717,
        load_angle_deg: 90.0,
        direct_onset: false,
        min_wall: 0.0,
        plane_strain: false,
        mesh_size: Some(0.031692424455518876),
    };
    let a = analyze(&i).unwrap();
    assert!(a.loaded_failure.is_none() && (a.margin / 51.36 - 1.0).abs() < 0.005, "margin {} {:?}", a.margin, a.loaded_failure);
}

/// Slow (about 60 s): a concentric, thick-walled, stiff bushing with the load along x. The default mesh fails every
/// attempt of the loaded stage (the friction contact cycles between stick and slip); a mesh 1.4 x coarser solves it, and
/// the margin (no spin torque here) is unaffected: the mesh route, not the fit-alone fallback, answers.
#[test]
#[ignore = "slow: run with --ignored"]
fn a_pin_load_the_default_mesh_cannot_solve_is_solved_on_another_mesh() {
    let i = Inputs {
        bore_dia: 0.7170741524845823,
        housing_od: 2.010949715936335,
        edge_distance: None,
        bushing_id: 0.6279361151428063,
        offset: 0.0,
        interference_dia: 0.005020770756775639,
        thickness: 0.739000798272336,
        housing: Elasticity::iso(15.0e6, 0.2895727566169047),
        bushing: Elasticity::iso(15.0e6, 0.2918196667044108),
        friction: 0.2255222816316399,
        pin: Elasticity::iso(16.5e6, 0.30),
        pin_friction: 0.07911088556462952,
        pin_clearance_dia: 0.0015331617664861002,
        credit_pin_load: true,
        load_lbf: 2720.1117949243553,
        load_angle_deg: 0.0,
        direct_onset: false,
        min_wall: 0.0,
        plane_strain: false,
        mesh_size: Some(0.08963426906057279),
    };
    let a = analyze(&i).unwrap();
    assert!(a.loaded_failure.is_none() && a.mesh_scale != 1.0 && a.pin_peak_pressure > 0.0, "{:?} scale {}", a.loaded_failure, a.mesh_scale);
}

/// Dimensional similarity: a bushing twice as large in every length (and the load 4 x, force goes as E l^2) is the same
/// problem, so the margin and the pressures must agree. The solve runs in units of the bore radius, so they do to
/// round-off, not merely to a tolerance.
#[test]
fn a_geometrically_similar_bushing_has_the_same_margin_and_pressures() {
    let a = Inputs { mesh_size: Some(0.07), offset: 0.03, load_lbf: 1500.0, ..base() };
    let k = 2.0;
    let b = Inputs {
        bore_dia: a.bore_dia * k,
        housing_od: a.housing_od * k,
        bushing_id: a.bushing_id * k,
        offset: a.offset * k,
        interference_dia: a.interference_dia * k,
        thickness: a.thickness * k,
        pin_clearance_dia: a.pin_clearance_dia * k,
        load_lbf: a.load_lbf * k * k,
        mesh_size: a.mesh_size.map(|m| m * k),
        ..a
    };
    let (ra, rb) = (analyze(&a).unwrap(), analyze(&b).unwrap());
    assert!((ra.margin / rb.margin - 1.0).abs() < 1e-9, "margins {} vs {}", ra.margin, rb.margin);
    assert!((ra.fit_pressure_max / rb.fit_pressure_max - 1.0).abs() < 1e-9 && (ra.pin_peak_pressure / rb.pin_peak_pressure - 1.0).abs() < 1e-9);
    // Torque goes as E l^3 (load x length): capacity 8 x.
    assert!((rb.torque_capacity / ra.torque_capacity / 8.0 - 1.0).abs() < 1e-9, "{} vs {}", rb.torque_capacity, ra.torque_capacity);
    assert!((rb.wall_thin / ra.wall_thin - k).abs() < 1e-9);
}

/// The mesh error estimate is reported and small on the default mesh of the Bushing Workbench defaults.
#[test]
fn the_default_mesh_reports_a_small_error_estimate() {
    let a = analyze(&Inputs { offset: 0.03, load_lbf: 1000.0, ..base() }).unwrap();
    assert!(a.mesh_error.is_finite() && a.mesh_error < 0.10 && a.mesh_scale == 1.0, "mesh error {} scale {}", a.mesh_error, a.mesh_scale);
}
