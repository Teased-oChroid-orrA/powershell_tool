//! 3D bracket of the edge-distance contact FE: do the plane-strain and plane-stress bearing collapses of a pin near a
//! free edge bracket the three-dimensional answer, and by how much does plane strain over- or under-predict it?
//!
//! One driver (`run`) serves 2D and 3D so that the only difference between the numbers is the idealisation:
//!
//! * the 2D mesh is `FeaLug`'s (Q9, graded about the bore, bushing annulus when there is one);
//! * `Idealisation::Strain` / `Stress` solve that mesh as it is; `Idealisation::Solid` extrudes it into Hex27 layers
//!   (one quarter of the plate: symmetric about the load line `y = 0` and the mid-plane `z = 0`);
//! * the housing is elastic-perfectly-plastic J2 (the model's flow stress), the bushing elastic, the pin an analytic
//!   rigid circle / cylinder (uniform through the thickness) driven toward the edge by a prescribed travel;
//! * a bushing is installed first (interference by two-pass frictional contact, plastic if the housing yields), then
//!   the pin is made to fit the post-fit bore, as `edge-check`'s contact FE does;
//! * the load is read at a pin travel of `COLLAPSE_D_OVER_D` of the bore diameter, `edge-check`'s collapse rule.
//!
//! Nothing here is a product feature: it is evidence (`docs/edge-distance-crosscheck.md`, "3D bracket").
//!
//! Default suite: the driver is proved on limiting cases (3D with every `uz` locked equals plane strain; the 2D driver
//! equals `FeaLug`'s own limit-load run; a very thin plate approaches plane stress; the fit pressure equals
//! `edge-check`'s). The long runs are `--ignored` tests that print their tables.

use edge_check::models::contact_model::flow_stress;
use edge_check::types::Strengths;
use lug_solver::bracket::{try_run, Bush, Case, Housing, Idealisation, Out, D_OVER_D};
use fea_core::fit::Tuning;
use lug_solver::*;
use std::time::Instant;

/// Hole (bore) diameter of every case.
const D: f64 = 0.5;
/// Pin clearance of `kernel_naca.rs` and `validation_naca_tn1503.rs` (an unbushed pin: diameter `D - 0.0004`).
const NACA_PIN_CLEARANCE: f64 = 0.0004;
const NACA_PIN_DIA: f64 = D - NACA_PIN_CLEARANCE;

/// The driver lives in `lug_solver::bracket` (the Plate Edge Workbench runs it); these tests panic on a failed run.
fn run(case: Case, ideal: Idealisation) -> Out {
    try_run(case, ideal, None).unwrap()
}

/// NACA TN 1503 specimen: 2 in wide, hole centred, steel pin (rigid here), no bushing.
fn naca(ftu: f64, fty: f64, fbru_15: f64, fbru_20: f64, ed: f64, td: f64, around: usize) -> Case {
    let (e, nu) = (10.4e6, 0.33);
    let flow = flow_stress(&Strengths { e, nu, sy: fty, fsu: 0.6 * ftu, ftu, fbru: fbru_20, fbru_e15: fbru_15 });
    Case { dia: D, pin_clearance: NACA_PIN_CLEARANCE, pin_mu: 0.0, ed, td, width: 2.0, length: 5.0, around, housing: Housing { e, nu, flow, law: None }, bush: None }
}

/// The same specimen with the bar's own hardening: Ramberg-Osgood through `Fty` and `Ftu` at the uniform elongation
/// (engineering stress treated as true, flat at `Ftu` beyond it: a strength-level idealisation, not a necking model).
fn hardened(mut c: Case, ftu: f64, fty: f64, elong: f64) -> Case {
    c.housing.law = Some(Hardening::ramberg_osgood(fty, ftu, elong).unwrap());
    c
}

/// 75S-T 1x2 bar (the first NACA row) at the given geometry.
fn naca_75s(ed: f64, td: f64, around: usize) -> Case {
    naca(87_900.0, 79_800.0, 115_500.0, 151_900.0, ed, td, around)
}

/// The default Bushing window case (bore 0.5, bronze bushing ID 0.375, 0.0015 diametral interference, 7075 housing,
/// friction 0.15, e/D = 1.5), on `edge-check`'s plate (reach `max(3 e, 10 a)` each way).
fn default_bushing(ed: f64, td: f64, around: usize) -> Case {
    let m = mechanics_core::materials::get_material("al7075");
    let s = Strengths::from_material(m);
    let reach = (3.0 * ed * D).max(10.0 * 0.5 * D);
    Case {
        dia: D,
        pin_clearance: NACA_PIN_CLEARANCE,
        pin_mu: 0.0,
        ed,
        td,
        width: 2.0 * reach,
        length: reach,
        around,
        housing: Housing { e: s.e, nu: s.nu, flow: flow_stress(&s), law: None },
        bush: Some(Bush { inner_dia: 0.375, e: 17.0e6, nu: 0.33, interference_dia: 0.0015, mu: 0.15 }),
    }
}
fn pct(a: f64, b: f64) -> f64 {
    100.0 * (a / b - 1.0)
}

// ------------------------------------------------------------------------------------- proof of the driver

/// With every `uz` locked a 3D model is plane strain: same mesh, same material, same pin, same steps, so the two must
/// agree to the quadrature difference of Hex27 and Quad9 (identical shape functions in the plane).
#[test]
fn a_3d_model_with_uz_locked_is_the_2d_plane_strain_model() {
    let c = naca_75s(1.5, 0.5, 36);
    let e2 = run(c, Idealisation::Strain);
    let e3 = run(c, Idealisation::Solid { layers: 1, lock_z: true });
    eprintln!("plane strain 2D {:.1} lbf, 3D with uz = 0 {:.1} lbf ({:+.3} %), {:.1} s", e2.at_limit(), e3.at_limit(), pct(e3.at_limit(), e2.at_limit()), e3.secs);
    for (a, b) in e2.pts.iter().zip(&e3.pts) {
        assert!((b.1 / a.1 - 1.0).abs() < 2e-3, "travel {:.4} D: 2D {:.1}, 3D {:.1}", a.0, a.1, b.1);
    }
}

/// The 2D driver reproduces `FeaLug`'s own displacement-driven limit load (same kernel, same tuning, same pin).
#[test]
fn the_2d_driver_equals_fealugs_limit_load_run() {
    let c = naca_75s(1.5, 0.5, 36);
    let mut fe = c.lug(PlaneMode::Strain);
    fe.set_plasticity(Hardening::linear(c.housing.flow, 0.0, 1.0).unwrap(), false).unwrap();
    let cap_over_a = 2.0 * D_OVER_D;
    let steps = ((cap_over_a / Tuning::collapse().step_fraction).ceil() as usize).clamp(6, 80);
    let lim = fe.limit_load_axial(PinSpec::new(NACA_PIN_DIA, 0.0), cap_over_a, steps).unwrap();
    let theirs = lim.curve.last().unwrap().1;
    let ours = run(c, Idealisation::Strain).at_limit();
    eprintln!("FeaLug limit_load_axial {theirs:.1} lbf, this driver {ours:.1} lbf ({:+.4} %)", pct(ours, theirs));
    assert!((ours / theirs - 1.0).abs() < 1e-3);
}

/// Pin-hole friction reaches the pin contact of an unbushed case: it raises the load at the same travel (the pin drags the
/// bore material along as well as pushing it), and `pin_mu = 0` is the frictionless run of every recorded number.
#[test]
fn pin_hole_friction_raises_the_collapse_load_and_zero_friction_is_unchanged() {
    let c = naca_75s(1.5, 0.5, 36);
    let free = run(c, Idealisation::Strain).at_limit();
    let rough = run(Case { pin_mu: 0.12, ..c }, Idealisation::Strain).at_limit();
    eprintln!("plane strain collapse: frictionless {free:.1} lbf, friction 0.12 {rough:.1} lbf ({:+.2} %)", pct(rough, free));
    assert!(rough > free * 1.01, "friction must carry load: {rough:.0} vs {free:.0}");
    assert!(rough < free * 1.15, "friction {rough:.0} vs {free:.0} is out of the expected few-percent range");
}

/// A thin plate has no through-thickness constraint: the 3D answer approaches plane stress as `t/D -> 0`.
#[test]
fn a_thin_3d_plate_approaches_the_plane_stress_model() {
    let thin = naca_75s(1.5, 0.05, 36);
    let s2 = run(thin, Idealisation::Stress).at_limit();
    let s3 = run(thin, Idealisation::Solid { layers: 2, lock_z: false }).at_limit();
    eprintln!("t/D 0.05: plane stress {s2:.1} lbf, 3D {s3:.1} lbf ({:+.2} %)", pct(s3, s2));
    assert!((s3 / s2 - 1.0).abs() < 0.04, "thin 3D {s3:.0} vs plane stress {s2:.0}");
}

/// `edge-check`'s plane-strain contact FE on the default case (an independent solver): the plate is `max(3 e, 10 a)`
/// in every direction, as the Bushing window builds it.
fn edge_mesh(ed: f64) -> edge_check::contact::ContactMesh {
    use edge_check::contact::ContactMesh;
    use edge_check::fem::MeshSpec as EMesh;
    use edge_check::types::{BushingSpec as EBush, Geometry};
    let a = 0.5 * D;
    let reach = (3.0 * ed * D).max(10.0 * a);
    let geom = Geometry { bore_radius: a, edge: ed * D, thickness: 0.5, plate_far: reach, plate_half_height: reach, plane_angle_deg: 40.0 };
    let b = EBush { inner_radius: 0.1875, interference: 0.00075, e: 17.0e6, nu: 0.33, friction: 0.15 };
    let h = mechanics_core::materials::get_material("al7075");
    ContactMesh::build(&geom, &b, h.e_ksi * 1000.0, h.nu, EMesh { n_radial: 10, n_arc: [3, 4, 10], grade: 2.0 }, 3, true).unwrap()
}

/// The kernel's fit of the default case equals `edge-check`'s own contact FE (an independent solver).
#[test]
fn the_fit_pressure_of_the_default_bushing_equals_edge_checks() {
    let ours = run(default_bushing(1.5, 1.0, 36), Idealisation::Strain).fit_pressure.unwrap();
    let theirs = edge_mesh(1.5).fit_only(0.00075).unwrap().0;
    eprintln!("fit pressure: kernel 2D {ours:.0} psi, edge-check contact FE {theirs:.0} psi ({:+.2} %)", pct(ours, theirs));
    assert!((ours / theirs - 1.0).abs() < 0.03);
}

// ------------------------------------------------------------------------------------- evidence tables

#[test]
#[ignore = "exploration"]
fn explore() {
    let c = naca_75s(1.5, 0.5, 36);
    for (name, ideal) in [("2D strain", Idealisation::Strain), ("2D stress", Idealisation::Stress), ("3D uz=0", Idealisation::Solid { layers: 1, lock_z: true }), ("3D 2 layers", Idealisation::Solid { layers: 2, lock_z: false })] {
        let o = run(c, ideal);
        println!("{name:<12} {:.0} lbf at limit, peak {:.0}, {:.1} s, {} dof", o.at_limit(), o.peak(), o.secs, o.dofs);
    }
}

#[test]
#[ignore = "exploration of the bushed default case: edge-check's own 2D contact FE (the reference)"]
fn explore_bushed_edge_check() {
    let t0 = Instant::now();
    let m = edge_mesh(1.5);
    println!("built {:.1} s, {} nodes", t0.elapsed().as_secs_f64(), m.node_count());
    println!("fit {:.0} psi ({:.1} s)", m.fit_only(0.00075).unwrap().0, t0.elapsed().as_secs_f64());
    let h = mechanics_core::materials::get_material("al7075");
    let flow = flow_stress(&Strengths::from_material(h));
    println!("flow {flow:.0}");
    let c = m.collapse(0.00075, flow, 0.5).unwrap();
    println!("collapse {:.0} lbf, first yield {:.0}, plateau {} ({:.1} s)", c.collapse, c.first_yield, c.plateau, t0.elapsed().as_secs_f64());
}

#[test]
#[ignore = "exploration of the bushed default case: the kernel driver in 2D"]
fn explore_bushed() {
    let c = default_bushing(1.5, 1.0, 36);
    for (name, ideal) in [("2D strain", Idealisation::Strain), ("2D stress", Idealisation::Stress)] {
        let o = run(c, ideal);
        println!("{name:<12} {:.0} lbf at limit, peak {:.0}, fit {:.0} psi, {:.1} s, {} dof", o.at_limit(), o.peak(), o.fit_pressure.unwrap(), o.secs, o.dofs);
    }
}

#[test]
#[ignore = "mesh convergence of the bracket; minutes"]
fn convergence() {
    for around in [36, 54, 72] {
        let c = naca_75s(1.5, 0.5, around);
        let e = run(c, Idealisation::Strain);
        let s = run(c, Idealisation::Stress);
        println!("around {around:>3}: 2D strain {:.0}  2D stress {:.0}  ({} dof)", e.at_limit(), s.at_limit(), e.dofs);
    }
    for (around, layers) in [(36, 2), (36, 3), (36, 4), (54, 2)] {
        let o = run(naca_75s(1.5, 0.5, around), Idealisation::Solid { layers, lock_z: false });
        println!("3D around {around:>3} layers {layers}: {:.0} lbf at limit, peak {:.0}, {:.0} s, {} dof", o.at_limit(), o.peak(), o.secs, o.dofs);
    }
}

// ------------------------------------------------------------------------------------- NACA TN 1503 table

/// Table I (tensile) and II (bearing ultimate) averages of NACA TN 1503, as in `validation_naca_tn1503.rs`:
/// `(alloy, Ftu, Fty, Fbru(1.5), Fbru(2.0))`, psi.
const NACA_BARS: [(&str, f64, f64, f64, f64); 6] = [
    ("75S-T 1x2 bar", 87_900.0, 79_800.0, 115_500.0, 151_900.0),
    ("75S-T 2x2 bar", 86_300.0, 69_000.0, 109_500.0, 140_400.0),
    ("24S-T 1x2 bar", 67_800.0, 48_400.0, 98_500.0, 123_000.0),
    ("24S-T 2x2 bar", 65_400.0, 46_500.0, 98_400.0, 123_400.0),
    ("14S-T 1x2 bar", 69_300.0, 63_200.0, 102_800.0, 129_500.0),
    ("14S-T 2x2 bar", 68_700.0, 61_100.0, 99_400.0, 124_200.0),
];

/// All twelve NACA points in plane strain, plane stress and 3D (t/D 0.5, 36 elements around, 2 layers over half the
/// thickness): the bearing stress `P/(D t)` of each against the test and the plane-strain / plane-stress ratios to 3D.
#[test]
#[ignore = "prints the NACA table in 2D and 3D; ~15-30 min"]
fn naca_table() {
    println!("{:<14} e/D {:>8} | {:>8} {:>8} {:>8} | vs test: {:>7} {:>7} {:>7} | strain/3D stress/3D", "bar", "test", "strain", "stress", "3D", "strain", "stress", "3D");
    let (mut sums, mut n) = ([0.0f64; 3], 0.0);
    for (alloy, ftu, fty, f15, f20) in NACA_BARS {
        for (ed, test) in [(1.5, f15), (2.0, f20)] {
            let c = naca(ftu, fty, f15, f20, ed, 0.5, 36);
            let area = D * c.thickness();
            let s = run(c, Idealisation::Strain).at_limit() / area;
            let p = run(c, Idealisation::Stress).at_limit() / area;
            let t = run(c, Idealisation::Solid { layers: 2, lock_z: false }).at_limit() / area;
            println!("{alloy:<14} {ed:>3} {test:>8.0} | {s:>8.0} {p:>8.0} {t:>8.0} | vs test: {:>+6.1}% {:>+6.1}% {:>+6.1}% | {:>+6.1}% {:>+6.1}%", pct(s, test), pct(p, test), pct(t, test), pct(s, t), pct(p, t));
            for (k, v) in [s, p, t].iter().enumerate() {
                sums[k] += pct(*v, test);
            }
            n += 1.0;
        }
    }
    println!("mean error vs test: plane strain {:+.1} %, plane stress {:+.1} %, 3D {:+.1} %", sums[0] / n, sums[1] / n, sums[2] / n);
}

/// Thickness sweep of the 75S-T 1x2 case: where plane stress < 3D < plane strain, and how fast plane strain converges
/// to the mid-plane of a thick plate. Model against model (no test data at these t/D), e/D 1.5 and 2.0.
#[test]
#[ignore = "prints the t/D sweep; ~20-40 min"]
fn thickness_sweep() {
    println!("{:>5} {:>5} | {:>8} {:>8} {:>8} | strain/3D stress/3D | s", "e/D", "t/D", "strain", "stress", "3D");
    for ed in [1.5, 2.0] {
        for td in [0.1, 0.25, 0.5, 1.0, 2.0] {
            let c = naca_75s(ed, td, 36);
            let layers = if td <= 0.5 { 2 } else { 3 };
            let area = D * c.thickness();
            let s = run(c, Idealisation::Strain).at_limit() / area;
            let p = run(c, Idealisation::Stress).at_limit() / area;
            let o3 = run(c, Idealisation::Solid { layers, lock_z: false });
            let t = o3.at_limit() / area;
            println!("{ed:>5} {td:>5} | {s:>8.0} {p:>8.0} {t:>8.0} | {:>+6.1}% {:>+6.1}% | {:.0}", pct(s, t), pct(p, t), o3.secs);
        }
    }
}

/// e/D beyond the tested 1.5-2.0 (model against model), t/D 0.5.
#[test]
#[ignore = "prints the e/D extension; ~10 min"]
fn edge_distance_extension() {
    println!("{:>5} | {:>8} {:>8} {:>8} | strain/3D stress/3D", "e/D", "strain", "stress", "3D");
    for ed in [1.0, 1.25, 3.0] {
        let c = naca_75s(ed, 0.5, 36);
        let area = D * c.thickness();
        let s = run(c, Idealisation::Strain).at_limit() / area;
        let p = run(c, Idealisation::Stress).at_limit() / area;
        let t = run(c, Idealisation::Solid { layers: 2, lock_z: false }).at_limit() / area;
        println!("{ed:>5} | {s:>8.0} {p:>8.0} {t:>8.0} | {:>+6.1}% {:>+6.1}%", pct(s, t), pct(p, t));
    }
}

/// Hardening: does the plane-strain / 3D gap survive a hardening law? Two bars (Table I elongations of TN 1503: 75S-T
/// 0.112, 24S-T 0.196), plane strain and 3D, perfectly plastic at the model flow stress and with Ramberg-Osgood
/// hardening to `Ftu`; the load is the bearing stress `P/(D t)` against the test.
#[test]
#[ignore = "prints the hardened comparison; ~20 min"]
fn hardened_comparison() {
    println!("{:<14} e/D {:>8} | {:>9} {:>9} | {:>9} {:>9} | strain/3D (EPP, hardened)", "bar", "test", "strain EPP", "3D EPP", "strain RO", "3D RO");
    for (i, (alloy, ftu, fty, f15, f20)) in NACA_BARS.iter().enumerate() {
        if i != 0 && i != 2 {
            continue;
        }
        let elong = [0.112, 0.114, 0.196, 0.188, 0.120, 0.116][i];
        for (ed, test) in [(1.5, *f15), (2.0, *f20)] {
            let c = naca(*ftu, *fty, *f15, *f20, ed, 0.5, 36);
            let ch = hardened(c, *ftu, *fty, elong);
            let area = D * c.thickness();
            let l = Idealisation::Solid { layers: 2, lock_z: false };
            let (se, te) = (run(c, Idealisation::Strain).at_limit() / area, run(c, l).at_limit() / area);
            let (sh, th) = (run(ch, Idealisation::Strain).at_limit() / area, run(ch, l).at_limit() / area);
            println!("{alloy:<14} {ed:>3} {test:>8.0} | {se:>9.0} {te:>9.0} | {sh:>9.0} {th:>9.0} | {:>+6.1}% {:>+6.1}% | vs test: 3D EPP {:>+6.1}% 3D RO {:>+6.1}% strain RO {:>+6.1}%", pct(se, te), pct(sh, th), pct(te, test), pct(th, test), pct(sh, test));
        }
    }
}

/// The default Bushing case in 3D (bronze bushing, interference, friction 0.15, 7075 housing, t/D 1.0, e/D 1.5),
/// against plane strain on the same mesh and `edge-check`'s own 2D contact FE. Plane stress is not run: it stalled in
/// the plastic pin stage (see the doc).
#[test]
#[ignore = "bushed 3D: minutes to hours"]
fn bushed_3d() {
    let c = default_bushing(1.5, 1.0, 36);
    let area = D * c.thickness();
    let strain = run(c, Idealisation::Strain);
    println!("2D strain {:.0} lbf ({:.0} psi bearing), fit {:.0} psi, {:.1} s", strain.at_limit(), strain.at_limit() / area, strain.fit_pressure.unwrap(), strain.secs);
    for layers in [2usize] {
        let o = run(c, Idealisation::Solid { layers, lock_z: false });
        println!("3D {layers} layers {:.0} lbf ({:+.1} % vs plane strain), fit {:.0} psi, {:.0} s, {} dof", o.at_limit(), pct(o.at_limit(), strain.at_limit()), o.fit_pressure.unwrap(), o.secs, o.dofs);
    }
}

// ------------------------------------------------------------------------------------- bearing yield and convergence

/// The 2 % offset load is on the curve between the first step and the collapse load, and a curve that never leaves its
/// initial line has none.
#[test]
fn the_offset_load_lies_between_the_first_step_and_the_collapse_load() {
    let o = run(naca_75s(1.5, 0.5, 36), Idealisation::Strain);
    let onset = o.offset_load(lug_solver::bracket::OFFSET_FRACTION).expect("a plastic plate yields within 0.1 D");
    eprintln!("2D strain: bearing yield {onset:.0} lbf, collapse {:.0} lbf", o.at_limit());
    assert!(onset > o.pts[0].1 && onset <= o.at_limit() * 1.0001, "{onset}");
    // A straight line through the origin never crosses its own offset.
    let linear = Out { pts: (1..=10).map(|i| (0.01 * i as f64, 1000.0 * i as f64)).collect(), fit_pressure: None, secs: 0.0, dofs: 0, fields: None };
    assert_eq!(linear.offset_load(0.02), None);
    // An ideal elastic-perfectly-plastic curve with k = 100000 and yield 1000 crosses its offset at 1000 (a flat top).
    let epp = Out { pts: (1..=10).map(|i| (0.01 * i as f64, (100_000.0 * 0.01 * i as f64).min(1000.0))).collect(), fit_pressure: None, secs: 0.0, dofs: 0, fields: None };
    let x = epp.offset_load(0.02).unwrap();
    assert!((x - 1000.0).abs() < 1e-6 * 1000.0, "{x}");
}

/// The departure load is where the secant stiffness has lost 5 %: exactly the yield of an elastic-perfectly-plastic curve
/// shifted by that loss, before the collapse load, and none for a straight line.
#[test]
fn the_departure_load_is_where_the_stiffness_falls() {
    // k = 100000, yield 1000 at travel 0.01: the secant is 0.95 k where P = 1000 and t = 1000 / (0.95 k).
    let epp = Out { pts: (1..=40).map(|i| (0.0025 * i as f64, (100_000.0 * 0.0025 * i as f64).min(1000.0))).collect(), fit_pressure: None, secs: 0.0, dofs: 0, fields: None };
    let d = epp.departure_load(0.05).unwrap();
    assert!((d - 1000.0).abs() < 1.0, "{d}");
    let linear = Out { pts: (1..=10).map(|i| (0.01 * i as f64, 1000.0 * i as f64)).collect(), fit_pressure: None, secs: 0.0, dofs: 0, fields: None };
    assert_eq!(linear.departure_load(0.05), None);
    let o = run(naca_75s(1.5, 0.5, 36), Idealisation::Strain);
    let (dep, off) = (o.departure_load(0.05).unwrap(), o.offset_load(0.02).unwrap());
    eprintln!("2D strain: departure {dep:.0}, 2 % offset {off:.0}, collapse {:.0}", o.at_limit());
    assert!(dep < off && off <= o.at_limit() * 1.0001);
}

/// The convergence loop stops on the measured change: the three 2D meshes of the recorded study (36, 54, 72 around) change the
/// collapse load by -0.6 % and -0.15 %, so a 0.5 % tolerance stops at the third, and a loose one at the second.
#[test]
fn the_convergence_loop_stops_when_the_load_stops_moving() {
    let mut seen = Vec::new();
    let c = lug_solver::bracket::converge(naca_75s(1.5, 0.5, 36), Idealisation::Strain, 0.005, 5, None, &mut |r| seen.push(r.clone())).unwrap();
    for r in &c.steps {
        eprintln!("around {:>3}: collapse {:.0} ({:?}), onset {:?} ({:?}), {} dof", r.around, r.collapse, r.collapse_change, r.onset, r.onset_change, r.dofs);
    }
    assert!(c.converged && (2..=3).contains(&c.steps.len()), "{} meshes", c.steps.len());
    assert_eq!(seen.len(), c.steps.len(), "progress is reported after every mesh");
    assert!(c.steps[0].collapse_change.is_none() && c.last().collapse_change.unwrap() < 0.005);
    assert!(c.steps.windows(2).all(|w| w[1].around > w[0].around && w[1].dofs > w[0].dofs), "each mesh is finer");
    // A tolerance no mesh can meet runs out the meshes and says it did not converge.
    let tight = lug_solver::bracket::converge(naca_75s(1.5, 0.5, 36), Idealisation::Strain, 1e-9, 2, None, &mut |_| {}).unwrap();
    assert!(!tight.converged && tight.steps.len() == 2);
}

/// A cancel request stops a run with an error instead of a result.
#[test]
fn a_cancel_request_stops_the_run() {
    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let err = try_run(naca_75s(1.5, 0.5, 36), Idealisation::Strain, Some(cancel)).err().expect("cancelled");
    eprintln!("cancelled run: {err}");
    assert!(!err.is_empty());
}
