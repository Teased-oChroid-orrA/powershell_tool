//! `FeaLug::analyze` (the kernel) against `LugModel::solve` (the condensed solver): the same
//! `LugSolution`, the same mesh, two independent implementations.

use lug_solver::fea::FeaLug;
use lug_solver::*;

const AL: Material = Material { e_psi: 10.3e6, nu: 0.33 };
const STEEL: Material = Material { e_psi: 29.0e6, nu: 0.30 };

fn lug() -> LugGeometry {
    LugGeometry::round_head(0.5, 1.5, 0.25, 3.75)
}

fn show(tag: &str, k: &LugSolution, c: &LugSolution, ms: (f64, f64)) {
    eprintln!(
        "{tag}: hoop {:.0} vs {:.0} ({:+.2} %), p {:.0} vs {:.0} ({:+.2} %), travel {:.4e} vs {:.4e}, arc {:.1} vs {:.1}, vm {:.0} vs {:.0}, zz {:.3} vs {:.3}, {:.0} ms vs {:.1} ms",
        k.peak_hoop, c.peak_hoop, 100.0 * (k.peak_hoop / c.peak_hoop - 1.0), k.peak_pressure, c.peak_pressure, 100.0 * (k.peak_pressure / c.peak_pressure - 1.0), k.bearing_deflection, c.bearing_deflection, k.contact_arc_deg, c.contact_arc_deg, k.peak_von_mises, c.peak_von_mises, k.verification.discretisation_error, c.verification.discretisation_error, ms.0, ms.1
    );
    if let (Some(kb), Some(cb)) = (&k.bushing, &c.bushing) {
        eprintln!("   bushing: fit {:.0} vs {:.0}, mean iface {:.0} vs {:.0}, separated {:.2} vs {:.2}, bush hoop {:.0} vs {:.0}, bush vm {:.0} vs {:.0}", kb.fit_pressure_unloaded, cb.fit_pressure_unloaded, kb.interface_mean_pressure, cb.interface_mean_pressure, kb.separated_fraction, cb.separated_fraction, kb.peak_hoop, cb.peak_hoop, kb.peak_von_mises, cb.peak_von_mises);
    }
    eprintln!("   verification kernel {:?}", k.verification);
}

fn both(tag: &str, half: bool, around: usize, pin: PinSpec, case: LoadCase, bushing: Option<BushingSpec>) -> (LugSolution, LugSolution) {
    let spec = MeshSpec { elements_around: around, ..Default::default() };
    let cond = LugModel::build_bushed(&lug(), AL, spec, half, PlaneMode::Stress, bushing).unwrap();
    let t = std::time::Instant::now();
    let c = cond.solve(pin, case).unwrap();
    let cms = t.elapsed().as_secs_f64() * 1e3;
    let fe = FeaLug::build_bushed(&lug(), AL, spec, half, PlaneMode::Stress, bushing).unwrap();
    let t = std::time::Instant::now();
    let k = fe.analyze(pin, case).unwrap();
    show(tag, &k, &c, (t.elapsed().as_secs_f64() * 1e3, cms));
    (k, c)
}

#[test]
fn axial_rigid_pin_matches() {
    let (k, c) = both("axial", true, 48, PinSpec::new(0.4995, 0.0), LoadCase { load_lbf: 2000.0, angle_deg: 0.0 }, None);
    assert!((k.peak_hoop / c.peak_hoop - 1.0).abs() < 0.03);
    assert!((k.bearing_deflection / c.bearing_deflection - 1.0).abs() < 5e-3);
}

#[test]
fn oblique_frictional_pin_matches() {
    let (k, c) = both("oblique mu .15", false, 32, PinSpec::new(0.4995, 0.15), LoadCase { load_lbf: 1500.0, angle_deg: 45.0 }, None);
    assert!((k.peak_hoop / c.peak_hoop - 1.0).abs() < 0.12);
}

#[test]
fn bushed_axial_matches() {
    let b = BushingSpec { inner_dia: 0.40, material: STEEL, interference_dia: 0.001, friction: 0.15 };
    let (k, c) = both("bushed axial", true, 48, PinSpec::new(0.3995, 0.1), LoadCase { load_lbf: 1500.0, angle_deg: 0.0 }, Some(b));
    assert!(k.bushing.is_some() && c.bushing.is_some());
}

fn disc(a: f64, r: f64) -> LugGeometry {
    LugGeometry { hole_dia: 2.0 * a, width: 2.0 * r, edge: r, length: r, thickness: 1.0, head_corner_radius: r, far_corner_radius: r }
}

#[test]
fn the_kernel_press_fit_matches_the_two_cylinder_lame_solution() {
    // Same disc as `bushing_validation.rs`: a bushing pressed into a round disc, exact Lame pressure.
    let (a, r, r_in, delta) = (0.25, 1.0, 0.15, 0.0005);
    let lug = Material { e_psi: 10.0e6, nu: 0.33 };
    let bush = Material { e_psi: 29.0e6, nu: 0.30 };
    let l = ((r * r + a * a) / (r * r - a * a) + lug.nu) / lug.e_psi;
    let b = ((a * a + r_in * r_in) / (a * a - r_in * r_in) - bush.nu) / bush.e_psi;
    let exact = delta / (a * (l + b));
    for friction in [0.0, 0.15] {
        let spec = BushingSpec { inner_dia: 2.0 * r_in, material: bush, interference_dia: 2.0 * delta, friction };
        let fe = FeaLug::build_bushed(&disc(a, r), lug, MeshSpec { elements_around: 48, ..Default::default() }, true, PlaneMode::Stress, Some(spec)).unwrap();
        let p = fe.fit_pressure(PinSpec::new(0.1, 0.0)).unwrap();
        eprintln!("kernel fit pressure {p:.1} vs Lame {exact:.1} ({:+.2} %), mu {friction}", 100.0 * (p / exact - 1.0));
        assert!((p / exact - 1.0).abs() < 0.02, "{p} vs {exact}");
    }
}

#[test]
#[ignore]
fn bushed_axial_tuning() {
    use lug_solver::fea::Tuning;
    let b = BushingSpec { inner_dia: 0.40, material: STEEL, interference_dia: 0.001, friction: 0.15 };
    let spec = MeshSpec { elements_around: 48, ..Default::default() };
    let pin = PinSpec::new(0.3995, 0.1);
    let case = LoadCase { load_lbf: 1500.0, angle_deg: 0.0 };
    let c = LugModel::build_bushed(&lug(), AL, spec, true, PlaneMode::Stress, Some(b)).unwrap().solve(pin, case).unwrap();
    eprintln!("condensed: hoop {:.0} p {:.0} travel {:.4e} fit {:.0}", c.peak_hoop, c.peak_pressure, c.bearing_deflection, c.bushing.as_ref().unwrap().fit_pressure_unloaded);
    let mut fe = FeaLug::build_bushed(&lug(), AL, spec, true, PlaneMode::Stress, Some(b)).unwrap();
    for (name, t) in [("friction", Tuning::friction()), ("reference", Tuning::reference()), ("frictionless", Tuning::frictionless())] {
        fe.tuning = Some(t);
        let t0 = std::time::Instant::now();
        let k = fe.analyze(pin, case).unwrap();
        eprintln!("{name}: hoop {:.0} p {:.0} travel {:.4e} arc {:.1} fit {:.0} ({:.0} ms)", k.peak_hoop, k.peak_pressure, k.bearing_deflection, k.contact_arc_deg, k.bushing.as_ref().unwrap().fit_pressure_unloaded, t0.elapsed().as_secs_f64() * 1e3);
    }
}

fn collapse_both(tag: &str, around: usize, angle: f64, bushing: Option<BushingSpec>, pin: PinSpec) -> (f64, f64) {
    let sf = 55_000.0;
    let half = angle == 0.0;
    let spec = MeshSpec { elements_around: around, ..Default::default() };
    let case = LoadCase { load_lbf: 1000.0, angle_deg: angle };
    let cond = LugModel::build_bushed(&lug(), AL, spec, half, PlaneMode::Strain, bushing).unwrap();
    let t = std::time::Instant::now();
    let c = cond.limit_load(pin, case, sf).unwrap();
    let cms = t.elapsed().as_secs_f64() * 1e3;
    let fe = FeaLug::build_bushed(&lug(), AL, spec, half, PlaneMode::Strain, bushing).unwrap();
    let t = std::time::Instant::now();
    let k = fe.limit_load(pin, case, sf, LimitOptions::default()).unwrap();
    eprintln!("{tag}: collapse kernel {:.0} lbf vs condensed {:.0} lbf ({:+.2} %), plateau {} vs {}, {:.0} ms vs {:.0} ms, {} steps", k.limit_load_lbf, c.limit_load_lbf, 100.0 * (k.limit_load_lbf / c.limit_load_lbf - 1.0), k.plateau, c.plateau, t.elapsed().as_secs_f64() * 1e3, cms, k.curve.len());
    (k.limit_load_lbf, c.limit_load_lbf)
}

#[test]
fn collapse_axial_matches() {
    let (k, c) = collapse_both("axial", 32, 0.0, None, PinSpec::new(0.4995, 0.0));
    assert!((k / c - 1.0).abs() < 0.05);
}

#[test]
fn collapse_oblique_matches() {
    for angle in [30.0, 60.0] {
        let (k, c) = collapse_both(&format!("oblique {angle}"), 32, angle, None, PinSpec::new(0.4995, 0.0));
        assert!((k / c - 1.0).abs() < 0.08, "{k} vs {c}");
    }
}

#[test]
fn collapse_bushed_matches() {
    let b = BushingSpec { inner_dia: 0.40, material: STEEL, interference_dia: 0.001, friction: 0.15 };
    let (k, c) = collapse_both("bushed axial", 32, 0.0, Some(b), PinSpec::new(0.3995, 0.0));
    assert!((k / c - 1.0).abs() < 0.08, "{k} vs {c}");
}

#[test]
fn elastic_pin_matches() {
    let steel_pin = PinBody::Elastic(STEEL);
    for (half, angle, mu, around) in [(true, 0.0, 0.0, 48usize), (true, 0.0, 0.15, 48), (false, 45.0, 0.0, 32)] {
        let pin = PinSpec { body: steel_pin, ..PinSpec::new(0.4995, mu) };
        let (k, c) = both(&format!("elastic pin angle {angle}"), half, around, pin, LoadCase { load_lbf: 1500.0, angle_deg: angle }, None);
        assert!((k.peak_hoop / c.peak_hoop - 1.0).abs() < 0.08, "{} vs {}", k.peak_hoop, c.peak_hoop);
    }
}

#[test]
#[ignore]
fn elastic_pin_oblique_probe() {
    let pin_body = PinBody::Elastic(STEEL);
    for (mu, around) in [(0.0, 32usize), (0.15, 32), (0.0, 48)] {
        let pin = PinSpec { body: pin_body, ..PinSpec::new(0.4995, mu) };
        let spec = MeshSpec { elements_around: around, ..Default::default() };
        let fe = FeaLug::build(&lug(), AL, spec, false, PlaneMode::Stress).unwrap();
        let t = std::time::Instant::now();
        match fe.analyze(pin, LoadCase { load_lbf: 1500.0, angle_deg: 45.0 }) {
            Ok(k) => eprintln!("oblique elastic pin mu {mu} around {around}: hoop {:.0} p {:.0} travel {:.4e} ({:.0} ms)", k.peak_hoop, k.peak_pressure, k.bearing_deflection, t.elapsed().as_secs_f64() * 1e3),
            Err(e) => eprintln!("oblique elastic pin mu {mu} around {around}: FAILED {e}"),
        }
    }
}

#[test]
#[ignore]
fn elastic_pin_friction_probe() {
    use lug_solver::fea::Tuning;
    let pin = PinSpec { body: PinBody::Elastic(STEEL), ..PinSpec::new(0.4995, 0.15) };
    let spec = MeshSpec { elements_around: 32, ..Default::default() };
    let mut fe = FeaLug::build(&lug(), AL, spec, true, PlaneMode::Stress).unwrap();
    for (name, t) in [("reference", Tuning::reference()), ("friction", Tuning::friction()), ("pf30 outer12", Tuning { penalty_factor: 30.0, ..Tuning::reference() }), ("pf10", Tuning { penalty_factor: 10.0, max_outer: 12, outer_tol: 2e-3, chord_iters: 0, newton_tol: 1e-6, step_fraction: 0.015, stall_tol: 0.0, first_step: 1.0, stick_slip_guard: false })] {
        fe.tuning = Some(t);
        let t0 = std::time::Instant::now();
        match fe.analyze(pin, LoadCase { load_lbf: 1500.0, angle_deg: 0.0 }) {
            Ok(k) => eprintln!("{name}: hoop {:.0} p {:.0} travel {:.4e} ({:.0} ms)", k.peak_hoop, k.peak_pressure, k.bearing_deflection, t0.elapsed().as_secs_f64() * 1e3),
            Err(e) => eprintln!("{name}: FAILED {e}"),
        }
    }
}

#[test]
#[ignore]
fn elastic_pin_convergence_probe() {
    let pin = PinSpec { body: PinBody::Elastic(STEEL), ..PinSpec::new(0.4995, 0.0) };
    for around in [24usize, 32, 48, 72] {
        let spec = MeshSpec { elements_around: around, ..Default::default() };
        let case = LoadCase { load_lbf: 1500.0, angle_deg: 0.0 };
        let c = LugModel::build_with(&lug(), AL, spec, true, PlaneMode::Stress).unwrap().solve(pin, case).unwrap();
        let k = FeaLug::build(&lug(), AL, spec, true, PlaneMode::Stress).unwrap().analyze(pin, case).unwrap();
        eprintln!("around {around}: kernel hoop {:.0} p {:.0} | condensed hoop {:.0} p {:.0}", k.peak_hoop, k.peak_pressure, c.peak_hoop, c.peak_pressure);
    }
}

#[test]
#[ignore]
fn oblique_default_probe() {
    let spec = MeshSpec { elements_around: 72, ..Default::default() };
    let case = LoadCase { load_lbf: 4000.0, angle_deg: 45.0 };
    let fe = FeaLug::build(&lug(), AL, spec, false, PlaneMode::Stress).unwrap();
    for (mu, around) in [(0.0, 72usize), (0.15, 72)] {
        let _ = around;
        let t = std::time::Instant::now();
        match fe.analyze(PinSpec::new(0.499, mu), case) {
            Ok(k) => eprintln!("oblique 4000 lbf mu {mu}: hoop {:.0} p {:.0} ({:.1} s)", k.peak_hoop, k.peak_pressure, t.elapsed().as_secs_f64()),
            Err(e) => eprintln!("oblique mu {mu}: FAILED {e} ({:.1} s)", t.elapsed().as_secs_f64()),
        }
    }
}

#[test]
#[ignore]
fn oblique_pieces_probe() {
    let case = LoadCase { load_lbf: 4000.0, angle_deg: 45.0 };
    for around in [32usize, 72] {
        let spec = MeshSpec { elements_around: around, ..Default::default() };
        let fe = FeaLug::build(&lug(), AL, spec, false, PlaneMode::Stress).unwrap();
        let t = std::time::Instant::now();
        let r = fe.analyze(PinSpec::new(0.499, 0.15), case).map(|k| k.peak_hoop);
        eprintln!("around {around}: elastic mu .15 4000 lbf: {r:?} ({:.1} s)", t.elapsed().as_secs_f64());
        let fp = FeaLug::build(&lug(), AL, spec, false, PlaneMode::Strain).unwrap();
        for mu in [0.0, 0.15] {
            let t = std::time::Instant::now();
            let r = fp.limit_load(PinSpec::new(0.499, mu), case, 60_000.0, LimitOptions::default()).map(|l| (l.limit_load_lbf, l.curve.len(), l.plateau, l.plastic_iterations));
            eprintln!("around {around}: collapse mu {mu}: {r:?} ({:.1} s)", t.elapsed().as_secs_f64());
        }
    }
}

#[test]
#[ignore]
fn refined_oblique_probe() {
    let case = LoadCase { load_lbf: 4000.0, angle_deg: 45.0 };
    let pin = PinSpec::new(0.499, 0.15);
    let refine = auto_refinement_for(&lug(), AL, pin, case, 72, 0.25);
    eprintln!("refine {refine:?}");
    let spec = MeshSpec { elements_around: 72, refine, ..Default::default() };
    let fe = FeaLug::build(&lug(), AL, spec, false, PlaneMode::Stress).unwrap();
    eprintln!("nodes {}", fe.mesh.nodes.len());
    for mu in [0.0, 0.15] {
        let t = std::time::Instant::now();
        let r = fe.analyze(PinSpec::new(0.499, mu), case);
        eprintln!("refined 72 mu {mu}: {:?} ({:.1} s)", r.as_ref().map(|k| (k.peak_hoop, k.contact_iterations, k.contact_arc_deg)), t.elapsed().as_secs_f64());
    }
}

#[test]
#[ignore]
fn collapse_mesh_convergence_probe() {
    for angle in [0.0, 45.0] {
        let case = LoadCase { load_lbf: 1000.0, angle_deg: angle };
        for around in [24usize, 32, 48, 72, 120] {
            let spec = MeshSpec { elements_around: around, ..Default::default() };
            let fp = FeaLug::build(&lug(), AL, spec, angle == 0.0, PlaneMode::Strain).unwrap();
            let t = std::time::Instant::now();
            let l = fp.limit_load(PinSpec::new(0.4995, 0.0), case, 60_000.0, LimitOptions::default()).unwrap();
            let c = LugModel::build_with(&lug(), AL, spec, angle == 0.0, PlaneMode::Strain).unwrap().limit_load(PinSpec::new(0.4995, 0.0), case, 60_000.0).unwrap();
            eprintln!("angle {angle} around {around}: kernel collapse {:.0} ({:.1} s), condensed {:.0}", l.limit_load_lbf, t.elapsed().as_secs_f64(), c.limit_load_lbf);
        }
    }
}

#[test]
#[ignore]
fn collapse_speed_probe() {
    use lug_solver::fea::Tuning;
    let case = LoadCase { load_lbf: 1000.0, angle_deg: 45.0 };
    let spec = MeshSpec { elements_around: 54, ..Default::default() };
    let mut fp = FeaLug::build(&lug(), AL, spec, false, PlaneMode::Strain).unwrap();
    let mut base = None;
    for (name, t) in [
        ("collapse preset", Tuning::collapse()),
        ("chord 2", Tuning { chord_iters: 2, ..Tuning::collapse() }),
        ("chord 4", Tuning { chord_iters: 4, ..Tuning::collapse() }),
        ("chord 8", Tuning { chord_iters: 8, ..Tuning::collapse() }),
    ] {
        fp.tuning = Some(t);
        let t0 = std::time::Instant::now();
        let l = fp.limit_load(PinSpec::new(0.4995, 0.15), case, 60_000.0, LimitOptions::default()).unwrap();
        let b = *base.get_or_insert(l.limit_load_lbf);
        eprintln!("{name}: {:.0} lbf ({:+.2} %), {:.1} s, {} its", l.limit_load_lbf, 100.0 * (l.limit_load_lbf / b - 1.0), t0.elapsed().as_secs_f64(), l.plastic_iterations);
    }
}

#[test]
#[ignore]
fn feature_pieces_probe() {
    let b = BushingSpec { inner_dia: 0.375, material: STEEL, interference_dia: 0.001, friction: 0.2 };
    let case = LoadCase { load_lbf: 1500.0, angle_deg: 20.0 };
    let spec = MeshSpec { elements_around: 72, ..Default::default() };
    let pin = PinSpec { thermal: Thermal { delta_t: 60.0, alpha_lug: 12.8e-6, alpha_bushing: 6.5e-6, alpha_pin: 6.5e-6 }, ..PinSpec::new(0.374, 0.15) };
    for geo in [false, true] {
        let mut fe = FeaLug::build_bushed(&lug(), AL, spec, false, PlaneMode::Stress, Some(b)).unwrap();
        fe.set_geometric_nonlinearity(geo);
        let t = std::time::Instant::now();
        let r = fe.analyze(pin, case);
        eprintln!("bushed oblique 72, geometric {geo}: {:?} ({:.1} s)", r.as_ref().map(|k| (k.peak_hoop, k.contact_iterations, k.mesh.nodes)), t.elapsed().as_secs_f64());
    }
    let fp = FeaLug::build_bushed(&lug(), AL, spec, false, PlaneMode::Strain, Some(b)).unwrap();
    let t = std::time::Instant::now();
    let r = fp.limit_load(pin, case, 60_000.0, LimitOptions::default()).map(|l| (l.limit_load_lbf, l.curve.len()));
    eprintln!("bushed oblique 72 collapse: {r:?} ({:.1} s)", t.elapsed().as_secs_f64());
}

#[test]
#[ignore]
fn bushed_oblique_collapse_probe() {
    use lug_solver::fea::Tuning;
    let b = BushingSpec { inner_dia: 0.375, material: STEEL, interference_dia: 0.001, friction: 0.2 };
    let case = LoadCase { load_lbf: 1500.0, angle_deg: 20.0 };
    let pin = PinSpec::new(0.374, 0.15);
    for around in [32usize, 48] {
        let spec = MeshSpec { elements_around: around, ..Default::default() };
        let mut fp = FeaLug::build_bushed(&lug(), AL, spec, false, PlaneMode::Strain, Some(b)).unwrap();
        for (name, t) in [
            ("collapse preset", Tuning::collapse()),
            ("outer 2 ntol 1e-6", Tuning { max_outer: 2, newton_tol: 1e-6, ..Tuning::collapse() }),
            ("pf 30", Tuning { penalty_factor: 30.0, ..Tuning::collapse() }),
            ("pf 30 outer 3 ntol 1e-6", Tuning { penalty_factor: 30.0, max_outer: 3, newton_tol: 1e-6, ..Tuning::collapse() }),
            ("pf 100 outer 3 ntol 1e-6", Tuning { penalty_factor: 100.0, max_outer: 3, newton_tol: 1e-6, ..Tuning::collapse() }),
        ] {
            fp.tuning = Some(t);
            let t0 = std::time::Instant::now();
            let r = fp.limit_load(pin, case, 60_000.0, LimitOptions::default()).map(|l| (l.limit_load_lbf, l.curve.len(), l.plateau));
            eprintln!("around {around} {name}: {r:?} ({:.1} s)", t0.elapsed().as_secs_f64());
        }
    }
}

#[test]
#[ignore]
fn bushed_oblique_collapse_one() {
    use lug_solver::fea::Tuning;
    let b = BushingSpec { inner_dia: 0.375, material: STEEL, interference_dia: 0.001, friction: 0.2 };
    let case = LoadCase { load_lbf: 1500.0, angle_deg: 20.0 };
    let pin = PinSpec::new(0.374, 0.15);
    let spec = MeshSpec { elements_around: 48, ..Default::default() };
    let mut fp = FeaLug::build_bushed(&lug(), AL, spec, false, PlaneMode::Strain, Some(b)).unwrap();
    fp.tuning = Some(Tuning { penalty_factor: 100.0, max_outer: 3, newton_tol: 1e-6, ..Tuning::collapse() });
    let l = fp.limit_load(pin, case, 60_000.0, LimitOptions::default()).unwrap();
    for c in &l.curve {
        eprintln!("travel {:.5} load {:.0} plastic {:.3} ep {:.4}", c.travel, c.load_lbf, c.plastic_fraction, c.max_equivalent_strain);
    }
}

#[test]
#[ignore]
fn condensed_plateau_probe() {
    let spec = MeshSpec { elements_around: 32, ..Default::default() };
    let pin = PinSpec::new(0.4995, 0.0);
    let case = LoadCase { load_lbf: 1000.0, angle_deg: 0.0 };
    let m = LugModel::build_with(&lug(), AL, spec, true, PlaneMode::Strain).unwrap();
    for stop in [true, false] {
        let l = m.limit_load_with(pin, case, 55_000.0, LimitOptions { stop_on_plateau: stop, travel_cap_over_a: 0.3, ..LimitOptions::default() }).unwrap();
        let peak = l.curve.iter().map(|c| c.load_lbf).fold(0.0, f64::max);
        eprintln!("condensed stop_on_plateau {stop}: limit {:.0} peak {peak:.0} plateau {} steps {}", l.limit_load_lbf, l.plateau, l.curve.len());
        for c in l.curve.iter().step_by(2) {
            eprintln!("   travel {:.5} load {:.0}", c.travel, c.load_lbf);
        }
    }
    let fe = FeaLug::build(&lug(), AL, spec, true, PlaneMode::Strain).unwrap();
    let k = fe.limit_load(pin, case, 55_000.0, LimitOptions { travel_cap_over_a: 0.3, ..LimitOptions::default() }).unwrap();
    eprintln!("kernel limit {:.0}", k.limit_load_lbf);
    for c in k.curve.iter().step_by(2) {
        eprintln!("   travel {:.5} load {:.0}", c.travel, c.load_lbf);
    }
}
