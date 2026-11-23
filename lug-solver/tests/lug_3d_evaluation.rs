//! Does a physically complete 3D pin-loaded lug merit replacing the 2D lug (`FeaLug` plane stress / plane strain)?
//! Evidence only (`docs/edge-distance-crosscheck.md`, "3D lug evaluation"); nothing here is a product feature.
//!
//! First case: NACA TN 1503 75S-T 1x2 bar (pin 0.5 in, plate 2 in wide, t = 0.25 in: t/D = 0.5), e/D 1.5.
//!
//! The 3D model (`run_3d`): one quarter of the plate (load-line plane `y = 0` and mid-plane `z = 0` symmetry; the
//! half-symmetry model of the request cut at `z = t/2` is the same model with the load-line symmetry used as well), the
//! 2D `FeaLug` mesh extruded into Hex27 layers over half the thickness; the lug is J2 plastic with a **true stress -
//! true plastic strain curve** (`Hardening::true_curve`: Ramberg-Osgood through Fty / Ftu, Swift beyond necking) in
//! **total-Lagrangian finite strain**; the pin is a **meshed deformable steel body** (Hex27, finite-strain elastic),
//! pressed in by prescribing the displacement of its core ring, in two-pass **frictional** contact with the bore.
//! The load is the sum of the core reactions; it is read at fixed pin travels by continuing the converged state from
//! stage to stage (`fea_core::fit::start_from`), so the whole load-travel curve and the state at each stage are
//! available. Limits that the kernel cannot do are listed in the doc, not simplified here.

use fea_core::contact::{ContactRule, ContactSpec};
use fea_core::fit::{start_from, Tuning, EPS_T_RATIO};
use fea_core::material::J2;
use fea_core::nonlinear::Stop;
use fea_core::sweep::extrude;
use fea_core::{Elastic, ElementKind, Loads, Mesh, Model, NlSolution};
use lug_solver::fea::FeaLug;
use lug_solver::*;
use mechanics_core::fracture::failure_strain;
use std::time::Instant;

const D: f64 = 0.5;
const T: f64 = 0.25;
const MAT: Material = Material { e_psi: 10.4e6, nu: 0.33 };
const STEEL: Material = Material { e_psi: 29.0e6, nu: 0.30 };

/// One tested bar: tensile properties, test bearing ultimates and (TN 1502 only) 2 % offset bearing yields at e/D 1.5 / 2.0.
#[derive(Clone, Copy)]
struct Bar {
    ftu: f64,
    fty: f64,
    /// Elongation in 2 in.
    elong: f64,
    fbru: [f64; 2],
    /// Bearing yield (2 % of D offset) at e/D 1.5 and 2.0; 0 = not in the repo's transcriptions.
    fbry: [f64; 2],
    /// Material library id (failure-strain family multiplier).
    id: &'static str,
}

impl Bar {
    fn test_ultimate(&self, ed: f64) -> f64 {
        if ed < 1.75 { self.fbru[0] } else { self.fbru[1] }
    }
    fn test_yield(&self, ed: f64) -> f64 {
        if ed < 1.75 { self.fbry[0] } else { self.fbry[1] }
    }
}

/// NACA TN 1503 Tables I / II, 75S-T 1x2 bar (no bearing yield transcribed).
const BAR_75S: Bar = Bar { ftu: 87_900.0, fty: 79_800.0, elong: 0.112, fbru: [115_500.0, 151_900.0], fbry: [0.0, 0.0], id: "al7075" };
/// NACA TN 1502 (Moore 1948) Tables I and III: 14S-T plate machined to 0.250 in, across grain (X) and with grain (W), pin 0.5 in.
/// Bearing yield = 2 % of D offset on the bearing stress - hole elongation curve (Table III, "Yield" columns).
const BAR_14ST_X: Bar = Bar { ftu: 70_600.0, fty: 62_800.0, elong: 0.083, fbru: [102_400.0, 141_000.0], fbry: [84_000.0, 100_700.0], id: "al2014" };
const BAR_14ST_W: Bar = Bar { ftu: 71_600.0, fty: 63_800.0, elong: 0.098, fbru: [98_900.0, 139_800.0], fbry: [84_400.0, 101_100.0], id: "al2014" };

fn plate_geometry(ed: f64) -> LugGeometry {
    LugGeometry { hole_dia: D, width: 2.0, edge: ed * D, length: 5.0, thickness: T, head_corner_radius: 0.0, far_corner_radius: 0.0 }
}

/// The 75S-T bar's true curve, to necking and beyond (Swift), up to a plastic strain of 0.6.
fn true_law(bar: Bar) -> Hardening {
    Hardening::true_curve(bar.fty, bar.ftu, bar.elong, 0.6).unwrap()
}

/// The library's failure strain for the bar's alloy: 7075 m = 1.0, 2014 m = 2.0 (both validated on TN 1503 / TN 1502), `eps_f = m ln(1 + A)`.
fn eps_f(bar: Bar) -> f64 {
    let m = failure_strain(bar.id).map_or(1.0, |f| f.multiplier);
    m * (1.0 + bar.elong).ln()
}

/// `(travel, load, max equivalent plastic strain)` after every stage.
#[derive(Clone, Copy, Debug)]
struct Pt {
    travel: f64,
    load: f64,
    max_ep: f64,
}

struct Run3 {
    pts: Vec<Pt>,
    /// The last converged state (for the through-thickness read-out).
    last: Option<NlSolution>,
    model: Option<Model>,
    secs: f64,
    dofs: usize,
    /// Why the stages ended: reached the travel / failure strain / a stage did not converge.
    ended: String,
}

#[derive(Clone, Copy, Debug)]
struct Spec3 {
    ed: f64,
    /// Elements around the full bore circle of the 2D mesh.
    around: usize,
    /// Hex27 layers over half the thickness.
    layers: usize,
    /// Radial element rings in the pin.
    rings: usize,
    mu: f64,
    /// Final pin travel over D, and the travel per stage over D.
    end: f64,
    stage: f64,
    /// Stop when the peak equivalent plastic strain reaches this (`None`: run to `end`).
    fail_ep: Option<f64>,
    /// Angular element step (rad) over the loaded sector (half width 60 degrees) of the 2D mesh; `None`: uniform.
    fine: Option<f64>,
}

fn mesh_spec(around: usize, fine: Option<f64>) -> MeshSpec {
    let refine = fine.map(|f| Refinement { centre: std::f64::consts::PI, half_width: 60f64.to_radians(), fine_step: f });
    MeshSpec { elements_around: around, refine, ..MeshSpec::default() }
}

/// Half ring of Quad9 elements (the pin, `y >= 0`) matching `n_around` angular divisions; surface `rim` = outer edges.
fn ring(n_around: usize, r_pin: f64, r_core: f64, rings: usize, physics: fea_core::Physics) -> Mesh {
    let mut pm = Mesh::new(physics);
    let nr = 2 * rings + 1;
    let nc = 2 * n_around + 1;
    let id = |c: usize, r: usize| c * nr + r;
    for c in 0..nc {
        let th = std::f64::consts::PI * c as f64 / (nc - 1) as f64;
        for r in 0..nr {
            let rho = r_core + (r_pin - r_core) * r as f64 / (nr - 1) as f64;
            pm.add_node([rho * th.cos(), rho * th.sin(), 0.0]);
        }
    }
    let mut conn = Vec::new();
    for e in 0..n_around {
        for j in 0..rings {
            for (dc, dr) in [(0, 0), (0, 2), (2, 2), (2, 0), (0, 1), (1, 2), (2, 1), (1, 0), (1, 1)] {
                conn.push(id(2 * e + dc, 2 * j + dr));
            }
        }
    }
    pm.add_block(ElementKind::Quad9, conn, Elastic::new(STEEL.e_psi, STEEL.nu), "pin").unwrap();
    pm.surfaces.insert("rim".into(), (0..n_around).map(|e| vec![id(2 * e, nr - 1), id(2 * e + 2, nr - 1), id(2 * e + 1, nr - 1)]).collect());
    pm
}

/// The 3D model run from zero to `spec.end` D of pin travel in stages of `spec.stage` D.
fn run_3d(spec: Spec3, bar: Bar) -> Run3 {
    let t0 = Instant::now();
    let fe = FeaLug::build(&plate_geometry(spec.ed), MAT, mesh_spec(spec.around, spec.fine), true, PlaneMode::Strain).unwrap();
    let a = 0.5 * D;
    let half_t = 0.5 * T;
    let levels: Vec<f64> = (0..=spec.layers).map(|i| half_t * i as f64 / spec.layers as f64).collect();
    let mut mesh = extrude(&fe.model.mesh, &levels).unwrap();
    let (r_pin, r_core) = (a - 0.0002, 0.15 * a);
    let pin2 = ring(fe.bore_faces.len(), r_pin, r_core, spec.rings, fe.model.mesh.physics);
    let pin3 = extrude(&pin2, &levels).unwrap();
    let core: Vec<usize> = pin3.nodes.iter().enumerate().filter(|(_, p)| (p[0].hypot(p[1]) - r_core).abs() < 1e-9).map(|(i, _)| i).collect();
    let off = mesh.append(&pin3, "pin/").unwrap();
    // The pin touches the loaded (-x) side of the bore.
    for p in mesh.nodes[off..].iter_mut() {
        p[0] -= a - r_pin;
    }
    mesh.set_plasticity(0, J2::finite(true_law(bar))).unwrap();
    mesh.set_plasticity(1, J2::elastic_finite()).unwrap();
    let rim: Vec<Vec<usize>> = mesh.surfaces["pin/rim"].clone();
    let bore: Vec<Vec<usize>> = mesh.surfaces["bore"].clone();
    let model = Model::new(mesh).unwrap();
    let dofs = model.mesh.n_dofs();

    let (e_lug, tune) = (MAT.e_psi, Tuning { penalty_factor: 10.0, max_outer: 3, outer_tol: 1e-2, chord_iters: 0, newton_tol: 1e-4, step_fraction: 0.015, stall_tol: 1e-3, first_step: 1.0, stick_slip_guard: true });
    let eps_n = tune.eps_n(e_lug, a);
    let specs: Vec<ContactSpec> = ContactSpec::two_pass("pin", bore, rim, eps_n).map(|s| s.with_rule(ContactRule::Reduced).with_friction(spec.mu, EPS_T_RATIO * eps_n).with_margin(0.08)).to_vec();

    let n_stage = (spec.end / spec.stage).round() as usize;
    let mut pts: Vec<Pt> = Vec::new();
    let mut prev: Option<NlSolution> = None;
    let mut ended = String::from("reached the end travel");
    for k in 1..=n_stage {
        let cap = spec.stage * D * k as f64;
        let prev_cap = spec.stage * D * (k - 1) as f64;
        let mut bc = model.dirichlet();
        for &n in model.mesh.node_set("far").unwrap() {
            bc.fix(n, 0, 0.0);
            bc.fix(n, 1, 0.0);
        }
        for (i, p) in model.mesh.nodes.iter().enumerate() {
            if p[1].abs() < 1e-12 {
                bc.fix(i, 1, 0.0);
            }
            if p[2].abs() < 1e-12 {
                bc.fix(i, 2, 0.0);
            }
        }
        for &c in &core {
            bc.fix(off + c, 0, -cap);
        }
        let mut opts = tune.options(3);
        opts.failure_ep = spec.fail_ep;
        let start = prev.as_ref().map(|p| start_from(p, prev_cap / cap));
        let nl = match model.solve_nonlinear_contact_from(&Loads::default(), &bc, specs.clone(), &opts, start.as_ref()) {
            Ok(nl) => nl,
            Err(e) => {
                ended = format!("stage {k} (travel {:.4} D) failed: {e}", cap / D);
                break;
            }
        };
        let hit_failure = nl.stop == Stop::FailureStrain;
        if !nl.complete() && !hit_failure {
            ended = format!("stage {k} (travel {:.4} D) stopped: {:?}", cap / D, nl.stop);
            break;
        }
        // The pin core is held by the prescribed displacement: the reaction is the pin load (x4: quarter model).
        let load: f64 = 4.0 * core.iter().map(|&c| nl.reactions[3 * (off + c)]).sum::<f64>().abs();
        let pt = Pt { travel: cap / D, load, max_ep: nl.state.max_ep() };
        eprintln!("[{:>6.0}s] stage {k:>2} travel {:.4} D load {:>8.1} lbf ({:>7.0} psi) max_ep {:.4} factorisations {}", t0.elapsed().as_secs_f64(), pt.travel, load, load / (D * T), pt.max_ep, nl.factorisations);
        pts.push(pt);
        prev = Some(nl);
        if hit_failure {
            ended = format!("peak plastic strain reached the failure strain at travel {:.4} D", cap / D);
            break;
        }
    }
    Run3 { pts, last: prev, model: Some(model), secs: t0.elapsed().as_secs_f64(), dofs, ended }
}

/// The load at which the curve meets the line of the initial slope offset by `off` (travel, same unit as the curve):
/// the 2 % bearing-yield rule of TN 1502 ("offset of 2 percent of pin diameter on bearing stress - hole elongation curves").
fn offset_load(curve: &[(f64, f64)], off: f64) -> Option<f64> {
    let (t0, p0) = *curve.first()?;
    let k0 = p0 / t0;
    let mut prev = curve[0];
    for &(t, p) in &curve[1..] {
        let (f0, f1) = (prev.1 - k0 * (prev.0 - off), p - k0 * (t - off));
        if f0 > 0.0 && f1 <= 0.0 {
            let w = f0 / (f0 - f1);
            return Some(prev.1 + w * (p - prev.1));
        }
        prev = (t, p);
    }
    None
}

/// The load at which the peak equivalent plastic strain reaches `eps` (linear in between), if it does.
fn load_at_strain(pts: &[Pt], eps: f64) -> Option<f64> {
    let mut prev: Option<Pt> = None;
    for &p in pts {
        if p.max_ep >= eps {
            let q = prev.unwrap_or(Pt { travel: 0.0, load: 0.0, max_ep: 0.0 });
            let w = (eps - q.max_ep) / (p.max_ep - q.max_ep).max(1e-300);
            return Some(q.load + w * (p.load - q.load));
        }
        prev = Some(p);
    }
    None
}

/// 2D baseline on the kernel (`FeaLug`, rigid pin, the same true law in finite strain): the load-travel curve (travel
/// from first touch, whole-lug load) and the load at the failure strain.
fn run_2d(ed: f64, mode: PlaneMode, mu: f64, bar: Bar, around: usize, fine: Option<f64>) -> (Vec<(f64, f64)>, f64, f64) {
    let t0 = Instant::now();
    let mut fe = FeaLug::build(&plate_geometry(ed), MAT, mesh_spec(around, fine), true, mode).unwrap();
    let law = true_law(bar);
    fe.set_plasticity(law, true).unwrap();
    let case = LoadCase { load_lbf: 1000.0, angle_deg: 0.0 };
    let r = fe.finite_collapse(PinSpec::new(D - 0.0004, mu), case, law, Some(eps_f(bar)), 0.5).unwrap();
    (r.curve.iter().map(|p| (p.travel, p.load_lbf)).collect(), r.collapse_lbf, t0.elapsed().as_secs_f64())
}

#[test]
#[ignore = "first deliverable: small-mesh frictional finite-strain deformable-pin 3D run; minutes to hours"]
fn small_3d_run() {
    let spec = Spec3 { ed: 1.5, around: 24, layers: 2, rings: 3, mu: 0.12, end: 0.12, stage: 0.005, fail_ep: Some(eps_f(BAR_75S)), fine: None };
    let r = run_3d(spec, BAR_75S);
    let curve: Vec<(f64, f64)> = r.pts.iter().map(|p| (p.travel * D, p.load)).collect();
    println!("3D ({} dof, {:.0} s): {}", r.dofs, r.secs, r.ended);
    println!("  2 % offset load {:?}, load at failure strain {:?}", offset_load(&curve, 0.02 * D), load_at_strain(&r.pts, eps_f(BAR_75S)));
    let _ = (&r.last, &r.model);
}

#[test]
#[ignore = "2D baselines"]
fn baselines_2d() {
    for (name, mode) in [("plane strain", PlaneMode::Strain), ("plane stress", PlaneMode::Stress)] {
        for mu in [0.0, 0.12] {
            let (curve, at_fail, secs) = run_2d(1.5, mode, mu, BAR_75S, 36, None);
            println!("{name} mu {mu}: 2 % offset {:?} lbf, at failure strain {at_fail:.0} lbf ({:.0} psi), {secs:.1} s, curve {} pts", offset_load(&curve, 0.02 * D), at_fail / (D * T), curve.len());
        }
    }
}


/// Through-thickness read-out of the last converged state, per element layer from the mid-plane (`z = 0`) to the free face
/// (`z = t/2`): the mean bearing pressure on the bore and the peak equivalent plastic strain in the lug elements within
/// 1.4 a of the hole centre. The kernel gives no Cauchy stress of a finite-strain state (`nl_gauss_stresses` refuses it),
/// so stress and triaxiality profiles are not available here (a limit, see the doc).
fn through_thickness(r: &Run3, mu: f64, layers: usize) -> String {
    let (nl, model) = (r.last.as_ref().unwrap(), r.model.as_ref().unwrap());
    let half_t = 0.5 * T;
    let band = |z: f64| (((z / half_t) * layers as f64).floor() as usize).min(layers - 1);
    let (mut pw, mut w) = (vec![0.0; layers], vec![0.0; layers]);
    for t in nl.contact_tractions(0, mu) {
        let b = band(t.x[2].abs());
        pw[b] += t.pressure * t.weight;
        w[b] += t.weight;
    }
    let blk = &model.mesh.blocks[0];
    let nn = blk.kind.n_nodes();
    let ngp = blk.kind.table().ngp;
    let mut ep = vec![0.0f64; layers];
    for (e, conn) in blk.conn.chunks_exact(nn).enumerate() {
        let c = conn.iter().fold([0.0; 3], |a, &n| [a[0] + model.mesh.nodes[n][0], a[1] + model.mesh.nodes[n][1], a[2] + model.mesh.nodes[n][2]]);
        let c = [c[0] / nn as f64, c[1] / nn as f64, c[2] / nn as f64];
        if c[0].hypot(c[1]) > 1.4 * 0.5 * D {
            continue;
        }
        let b = band(c[2]);
        for g in 0..ngp {
            ep[b] = ep[b].max(nl.state.gp[0][e * ngp + g][6]);
        }
    }
    let mut out = String::from("  layer (z/(t/2), mid-plane to free face): mean bearing pressure psi | peak equivalent plastic strain\n");
    for b in 0..layers {
        out += &format!("   {:.2}-{:.2}: {:>8.0} | {:.4}\n", b as f64 / layers as f64, (b + 1) as f64 / layers as f64, if w[b] > 0.0 { pw[b] / w[b] } else { 0.0 }, ep[b]);
    }
    out
}

fn report(label: &str, spec: Spec3, r: &Run3, mu: f64, bar: Bar) {
    let curve: Vec<(f64, f64)> = r.pts.iter().map(|p| (p.travel * D, p.load)).collect();
    let (tu, ty) = (bar.test_ultimate(spec.ed), bar.test_yield(spec.ed));
    let at_fail = load_at_strain(&r.pts, eps_f(bar));
    let off = offset_load(&curve, 0.02 * D);
    let psi = |l: f64| l / (D * T);
    let vs = |l: Option<f64>, test: f64| l.map_or("-".into(), |l| format!("{l:.0} lbf = {:.0} psi{}", psi(l), if test > 0.0 { format!(" ({:+.1} %)", 100.0 * (psi(l) / test - 1.0)) } else { String::new() }));
    println!("{label}: {} dof, {:.0} s, mu {mu}; {}", r.dofs, r.secs, r.ended);
    println!("  load at failure strain {} vs test ultimate {tu:.0} psi | 2 % offset load {} vs test yield {ty:.0} psi", vs(at_fail, tu), vs(off, ty));
    if r.last.is_some() {
        println!("{}", through_thickness(r, mu, spec.layers));
    }
}

/// One row of the comparison: the bar at `ed`, 2D plane strain / plane stress (rigid pin, mu 0 and 0.12) and the small-mesh
/// 3D model (deformable pin, mu 0.12), each at both limits (2 % offset and the library failure strain), against the tests.
fn compare_row(name: &str, bar: Bar, ed: f64) {
    let (tu, ty) = (bar.test_ultimate(ed), bar.test_yield(ed));
    let psi = |l: f64| l / (D * T);
    println!("{name} e/D {ed}: test ultimate {tu:.0} psi, test 2 % yield {ty:.0} psi, failure strain {:.4}", eps_f(bar));
    for (label, mode) in [("plane strain", PlaneMode::Strain), ("plane stress", PlaneMode::Stress)] {
        for mu in [0.0, 0.12] {
            let (curve, at_fail, _) = run_2d(ed, mode, mu, bar, 36, None);
            let off = offset_load(&curve, 0.02 * D);
            println!("  2D {label:<12} mu {mu:<4}: failure strain {:>7.0} psi ({:+.1} %) | 2 % offset {}", psi(at_fail), 100.0 * (psi(at_fail) / tu - 1.0), off.map_or("none".into(), |l| format!("{:.0} psi{}", psi(l), if ty > 0.0 { format!(" ({:+.1} %)", 100.0 * (psi(l) / ty - 1.0)) } else { String::new() })));
        }
    }
    let spec = Spec3 { ed, around: 24, layers: 2, rings: 3, mu: 0.12, end: 0.12, stage: 0.005, fail_ep: Some(eps_f(bar)), fine: None };
    for mu in [0.0, 0.12] {
        let s = Spec3 { mu, ..spec };
        let r = run_3d(s, bar);
        report(&format!("  3D small mesh {name} e/D {ed}"), s, &r, mu, bar);
    }
}

#[test]
#[ignore = "the comparison table on three bars at e/D 1.5 and 2.0; ~30-60 min"]
fn comparison_table() {
    for (name, bar) in [("75S-T 1x2 (TN 1503)", BAR_75S), ("14S-T plate X (TN 1502)", BAR_14ST_X), ("14S-T plate W (TN 1502)", BAR_14ST_W)] {
        for ed in [1.5, 2.0] {
            compare_row(name, bar, ed);
        }
    }
}

#[test]
#[ignore = "3D at the requested resolution and the friction / mesh sensitivity; hours"]
fn sensitivity_3d() {
    let ef = Some(eps_f(BAR_75S));
    let coarse = Spec3 { ed: 1.5, around: 24, layers: 2, rings: 3, mu: 0.12, end: 0.12, stage: 0.005, fail_ep: ef, fine: None };
    // Friction at the small mesh (cheap): 0 (comparison), 0.10, 0.12, 0.15.
    for mu in [0.0, 0.10, 0.12, 0.15] {
        let s = Spec3 { mu, ..coarse };
        let r = run_3d(s, BAR_75S);
        report(&format!("small mesh (around 24, 2 layers) mu {mu}"), s, &r, mu, BAR_75S);
    }
    // Requested resolution: 4 layers over the half thickness, bearing arc element <= D/40 (0.05 rad on a = D/2 is 0.0125 in).
    // (6 layers was started and stopped after hours: more than 100k dof; 4 layers took 54 min, 73k dof, 0.78 GB.)
    for (name, around, layers, fine) in [("layers 4, around 36, arc D/40", 36, 4, Some(0.05))] {
        let s = Spec3 { around, layers, fine, ..coarse };
        let r = run_3d(s, BAR_75S);
        report(name, s, &r, 0.12, BAR_75S);
    }
}
