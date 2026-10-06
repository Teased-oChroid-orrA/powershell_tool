//! End-to-end checks of the pin-loaded lug model.

use lug_solver::*;

const AL: Material = Material { e_psi: 10.0e6, nu: 0.33 };

fn lug() -> LugGeometry {
    LugGeometry::round_head(0.5, 1.5, 0.25, 3.75)
}

fn pin(clearance: f64) -> PinSpec {
    PinSpec::new(0.5 - clearance, 0.0)
}

#[test]
fn axial_load_is_carried_in_full_and_contact_force_equals_the_load() {
    let sol = solve_once(&lug(), AL, MeshSpec::default(), pin(0.0), LoadCase { load_lbf: 5000.0, angle_deg: 0.0 }).unwrap();
    assert!(sol.contact.iter().any(|p| p.pressure > 0.0));
    // Sum of contact force along the load (per unit thickness x t) is the load.
    let t = lug().thickness;
    // The axial case runs on the half lug (y >= 0): double its force for the whole lug.
    let (mut fx, mut fy) = (0.0, 0.0);
    for p in &sol.contact {
        let rel = [p.x[0] - sol.pin_centre[0], p.x[1] - sol.pin_centre[1]];
        let d = rel[0].hypot(rel[1]);
        let n = [rel[0] / d, rel[1] / d];
        let tau = [-n[1], n[0]];
        fx += p.w * (p.pressure * n[0] + p.shear * tau[0]);
        fy += p.w * (p.pressure * n[1] + p.shear * tau[1]);
    }
    assert!((2.0 * fx * t + 5000.0).abs() < 5000.0 * 1e-4, "2 fx {} should equal -P", 2.0 * fx * t);
    let _ = fy; // the upper half's y force is balanced by the mirrored lower half
}

#[test]
fn half_and_full_models_agree_for_an_axial_load() {
    let case = LoadCase { load_lbf: 4000.0, angle_deg: 0.0 };
    let spec = MeshSpec::default();
    let half = LugModel::build(&lug(), AL, spec, true).unwrap().solve(pin(0.001), case).unwrap();
    let full = LugModel::build(&lug(), AL, spec, false).unwrap().solve(pin(0.001), case).unwrap();
    let rel = |a: f64, b: f64| (a - b).abs() / b.abs();
    assert!(rel(half.peak_hoop, full.peak_hoop) < 2e-3, "{} vs {}", half.peak_hoop, full.peak_hoop);
    assert!(rel(half.peak_pressure, full.peak_pressure) < 2e-3);
    assert!(rel(half.bearing_deflection, full.bearing_deflection) < 2e-3);
}

#[test]
fn stresses_scale_linearly_with_load_once_the_contact_arc_is_fixed() {
    // Interference keeps the contact patch all round, so the response is linear in the extra load.
    let m = LugModel::build(&lug(), AL, MeshSpec::default(), true).unwrap();
    let a = m.solve(pin(-0.002), LoadCase { load_lbf: 300.0, angle_deg: 0.0 }).unwrap();
    let b = m.solve(pin(-0.002), LoadCase { load_lbf: 600.0, angle_deg: 0.0 }).unwrap();
    assert!(b.contact_arc_deg > 359.0, "the fit must keep the whole bore in contact (arc {})", b.contact_arc_deg);
    let z = m.solve(pin(-0.002), LoadCase { load_lbf: 0.0, angle_deg: 0.0 }).unwrap();
    let d1 = a.peak_hoop - z.peak_hoop;
    let d2 = b.peak_hoop - z.peak_hoop;
    assert!((d2 / d1 - 2.0).abs() < 0.02, "d1 {d1} d2 {d2}");
}

#[test]
fn contact_arc_widens_with_load_for_a_clearance_pin() {
    let m = LugModel::build(&lug(), AL, MeshSpec::default(), true).unwrap();
    let lo = m.solve(pin(0.003), LoadCase { load_lbf: 800.0, angle_deg: 0.0 }).unwrap();
    let hi = m.solve(pin(0.003), LoadCase { load_lbf: 8000.0, angle_deg: 0.0 }).unwrap();
    assert!(lo.contact_arc_deg > 0.0 && hi.contact_arc_deg > lo.contact_arc_deg, "{} -> {}", lo.contact_arc_deg, hi.contact_arc_deg);
    assert!(hi.contact_arc_deg < 180.0 + 20.0);
}

#[test]
fn a_tighter_clearance_lowers_the_peak_pressure() {
    let m = LugModel::build(&lug(), AL, MeshSpec::default(), true).unwrap();
    let case = LoadCase { load_lbf: 5000.0, angle_deg: 0.0 };
    let tight = m.solve(pin(0.0005), case).unwrap();
    let loose = m.solve(pin(0.01), case).unwrap();
    assert!(loose.peak_pressure > tight.peak_pressure, "{} vs {}", loose.peak_pressure, tight.peak_pressure);
    assert!(loose.peak_hoop > tight.peak_hoop);
}

#[test]
fn transverse_load_runs_on_the_full_model_and_loads_the_side_of_the_bore() {
    let sol = solve_once(&lug(), AL, MeshSpec::default(), pin(0.001), LoadCase { load_lbf: 3000.0, angle_deg: 90.0 }).unwrap();
    // Pulled toward +y: the patch is centred about 90 degrees.
    let c = sol.contact_centre_deg.rem_euclid(360.0);
    assert!((c - 90.0).abs() < 25.0, "centre {c}");
    assert!(sol.load[1] > 2999.0 && sol.load[0].abs() < 1e-6);
}

#[test]
fn friction_changes_the_stress_state_but_never_exceeds_the_coulomb_limit() {
    let m = LugModel::build(&lug(), AL, MeshSpec::default(), true).unwrap();
    let case = LoadCase { load_lbf: 6000.0, angle_deg: 0.0 };
    let free = m.solve(PinSpec::new(0.499, 0.0), case).unwrap();
    let fric = m.solve(PinSpec::new(0.499, 0.3), case).unwrap();
    assert!(free.contact.iter().all(|p| p.shear == 0.0));
    for p in &fric.contact {
        assert!(p.shear.abs() <= 0.3 * p.pressure * (1.0 + 1e-9) + 1e-9, "|T| {} > mu p {}", p.shear, 0.3 * p.pressure);
    }
    assert!(fric.contact.iter().any(|p| p.shear != 0.0), "friction should carry some traction");
}

#[test]
fn zero_load_with_interference_gives_a_self_equilibrated_fit() {
    let sol = solve_once(&lug(), AL, MeshSpec::default(), pin(-0.001), LoadCase { load_lbf: 0.0, angle_deg: 0.0 }).unwrap();
    assert!(sol.peak_pressure > 0.0 && sol.contact_arc_deg > 359.0, "arc {}", sol.contact_arc_deg);
}

#[test]
fn invalid_inputs_are_errors_not_panics() {
    let m = LugModel::build(&lug(), AL, MeshSpec::default(), true).unwrap();
    assert!(m.solve(PinSpec::new(-1.0, 0.0), LoadCase { load_lbf: 1.0, angle_deg: 0.0 }).is_err());
    assert!(m.solve(PinSpec::new(0.5, 0.0), LoadCase { load_lbf: 1.0, angle_deg: 45.0 }).is_err(), "half model needs an axial load");
    assert!(m.solve(PinSpec::new(0.5, 0.0), LoadCase { load_lbf: f64::NAN, angle_deg: 0.0 }).is_err());
}

#[test]
fn reports_speed_for_the_default_mesh() {
    let case = LoadCase { load_lbf: 5000.0, angle_deg: 0.0 };
    let t = std::time::Instant::now();
    let model = LugModel::build(&lug(), AL, MeshSpec::default(), true).unwrap();
    let build = t.elapsed();
    let t = std::time::Instant::now();
    let sol = model.solve(pin(0.001), case).unwrap();
    let solve = t.elapsed();
    eprintln!("SPEED half: build {:?} solve {:?} dofs {} bw {} iters {} contact_ms {:.1} recover_ms {:.1} mesh {:?}", build, solve, sol.dofs, sol.bandwidth, sol.contact_iterations, sol.timings.contact_ms, sol.timings.recover_ms, sol.mesh);
    let t = std::time::Instant::now();
    let full = LugModel::build(&lug(), AL, MeshSpec::default(), false).unwrap();
    let build = t.elapsed();
    let t = std::time::Instant::now();
    let sol = full.solve(pin(0.001), LoadCase { load_lbf: 5000.0, angle_deg: 60.0 }).unwrap();
    eprintln!("SPEED full oblique: build {:?} solve {:?} iters {}  peak hoop {:.0}", build, t.elapsed(), sol.contact_iterations, sol.peak_hoop);
}



/// Closed-form 2D Hertz contact of a loose rigid pin in a bore (plane stress, rigid pin):
/// half-width `b = sqrt(4 P' R* / (pi E))`, peak `p0 = 2 P' / (pi b)`, `1/R* = 1/r_pin - 1/a`.
fn hertz(a: f64, r_pin: f64, load: f64, t: f64) -> (f64, f64) {
    let pp = load / t;
    let rstar = 1.0 / (1.0 / r_pin - 1.0 / a);
    let b = (4.0 * pp * rstar / (std::f64::consts::PI * AL.e_psi)).sqrt();
    (b, 2.0 * pp / (std::f64::consts::PI * b))
}

/// Element-averaged peak pressure (integrates each 3-point bore edge).
fn edge_averaged_peak(s: &LugSolution) -> f64 {
    s.contact
        .chunks(3)
        .map(|ch| {
            let (f, w) = ch.iter().fold((0.0, 0.0), |(f, w), p| (f + p.pressure * p.w, w + p.w));
            f / w
        })
        .fold(0.0, f64::max)
}

#[test]
fn a_loose_pin_reproduces_the_hertz_contact_patch_with_automatic_refinement() {
    // R_pin = 0.8 a in a big plate: a narrow patch (14.6 degrees wide).
    let g = LugGeometry::round_head(0.5, 3.0, 0.25, 6.0);
    let (a, r_pin, load) = (0.25, 0.2, 2000.0);
    let (b, p0) = hertz(a, r_pin, load, 0.25);
    let exact_arc = 2.0 * (b / a).to_degrees();
    let pin = PinSpec::new(2.0 * r_pin, 0.0);
    let case = LoadCase { load_lbf: load, angle_deg: 0.0 };
    let m = LugModel::build_for_pin(&g, AL, MeshSpec::default(), pin, case).unwrap();
    let s = m.solve(pin, case).unwrap();
    let peak = edge_averaged_peak(&s);
    eprintln!("HERTZ auto-refined: nodes {} arc {:.2} (exact {:.2}) peak {:.0} (exact {:.0}) err {:.1}% build {:.0}ms contact {:.0}ms", s.mesh.nodes, s.contact_arc_deg, exact_arc, peak, p0, 100.0 * (peak - p0) / p0, s.timings.build.assemble_ms + s.timings.build.factor_ms + s.timings.build.condense_ms, s.timings.contact_ms);
    assert!((peak - p0).abs() / p0 < 0.04, "peak {peak} vs Hertz {p0}");
    assert!((s.contact_arc_deg - exact_arc).abs() / exact_arc < 0.12, "arc {} vs {exact_arc}", s.contact_arc_deg);
    assert!(s.mesh.nodes < 5000, "refinement must keep the model small ({} nodes)", s.mesh.nodes);
}

#[test]
fn the_uniform_coarse_mesh_underestimates_a_narrow_hertz_peak_which_is_why_refinement_exists() {
    let g = LugGeometry::round_head(0.5, 3.0, 0.25, 6.0);
    let (a, r_pin, load) = (0.25, 0.2, 2000.0);
    let (_, p0) = hertz(a, r_pin, load, 0.25);
    let case = LoadCase { load_lbf: load, angle_deg: 0.0 };
    let coarse = LugModel::build(&g, AL, MeshSpec::default(), true).unwrap().solve(PinSpec::new(2.0 * r_pin, 0.0), case).unwrap();
    assert!(edge_averaged_peak(&coarse) < 0.85 * p0, "the 72-element uniform mesh cannot resolve a 14-degree patch");
}

#[test]
fn refinement_is_skipped_for_wide_contact_and_follows_the_load_direction() {
    let g = lug();
    let tight = auto_refinement(&g, AL, PinSpec::new(0.4995, 0.0), LoadCase { load_lbf: 5000.0, angle_deg: 0.0 }, 72);
    assert!(tight.is_none(), "a tight fit loads a wide arc");
    assert!(auto_refinement(&g, AL, PinSpec::new(0.502, 0.0), LoadCase { load_lbf: 5000.0, angle_deg: 0.0 }, 72).is_none(), "interference");
    let loose = auto_refinement(&g, AL, PinSpec::new(0.40, 0.0), LoadCase { load_lbf: 1000.0, angle_deg: 90.0 }, 72).expect("a loose pin");
    assert!((loose.centre - std::f64::consts::FRAC_PI_2).abs() < 1e-12);
    assert!(loose.fine_step < 2.0 * std::f64::consts::PI / 72.0);
}
