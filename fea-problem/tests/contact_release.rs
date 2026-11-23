//! The release of a contact under a pull that a bolt holds (Phase C follow-up 4's unsolved case): a preloaded bolt through two unit
//! bars; past `F = P / (1 - Phi)` the interface opens and the bolt alone carries the pull. Every solve runs under a wall-clock guard.
use fea_problem::{solve, templates, ElementChoice, Fastener, FaceSig, Load, MeshSpec, Preload, Problem, Support, Tie};
use fea_problem::solid::SolidCache;
use solid_kernel::{SurfaceMesh, V3};
use std::sync::mpsc;
use std::time::Duration;

const STEP: &str = "ISO-10303-21;\nHEADER;\nENDSEC;\nDATA;\nENDSEC;\nEND-ISO-10303-21;\n";

fn box_at(m: &mut SurfaceMesh, lo: V3, size: V3, first: usize) {
    let p = |x: f64, y: f64, z: f64| [lo[0] + x * size[0], lo[1] + y * size[1], lo[2] + z * size[2]];
    let faces: [([V3; 4], usize); 6] = [
        ([p(0.0, 0.0, 0.0), p(0.0, 0.0, 1.0), p(0.0, 1.0, 1.0), p(0.0, 1.0, 0.0)], 0),
        ([p(1.0, 0.0, 0.0), p(1.0, 1.0, 0.0), p(1.0, 1.0, 1.0), p(1.0, 0.0, 1.0)], 1),
        ([p(0.0, 0.0, 0.0), p(1.0, 0.0, 0.0), p(1.0, 0.0, 1.0), p(0.0, 0.0, 1.0)], 2),
        ([p(0.0, 1.0, 0.0), p(0.0, 1.0, 1.0), p(1.0, 1.0, 1.0), p(1.0, 1.0, 0.0)], 3),
        ([p(0.0, 0.0, 0.0), p(0.0, 1.0, 0.0), p(1.0, 1.0, 0.0), p(1.0, 0.0, 0.0)], 4),
        ([p(0.0, 0.0, 1.0), p(1.0, 0.0, 1.0), p(1.0, 1.0, 1.0), p(0.0, 1.0, 1.0)], 5),
    ];
    for (q, f) in faces {
        m.push_triangle(q[0], q[1], q[2], first + f);
        m.push_triangle(q[0], q[2], q[3], first + f);
    }
}

/// Unit bars of footprint 1 x 1 and height 1 at `(x0, z0)` each, as one assembly (body `k` has faces `face{6k+1}..face{6k+6}`).
fn bars(list: &[(f64, f64)]) -> Problem {
    let mut m = SurfaceMesh::default();
    for (k, &(x0, z0)) in list.iter().enumerate() {
        box_at(&mut m, [x0, 0.0, z0], [1.0, 1.0, 1.0], 6 * k);
    }
    let mut cache = SolidCache::new(STEP, m, 0.01);
    cache.bodies = vec![6; list.len()];
    let mut p = templates::blank();
    p.name = "Bars".into();
    p.set_solid(STEP, cache);
    p.material.nu = 0.0;
    p.mesh = MeshSpec { size: 0.5, ..MeshSpec::default() };
    p
}

fn bottom(k: usize) -> String {
    format!("face{}", 6 * k + 5)
}

fn top(k: usize) -> String {
    format!("face{}", 6 * k + 6)
}

/// The tie of bar `a`'s top face and bar `b`'s bottom face (`friction` `None` bonded).
fn joint(list: &[(f64, f64)], a: usize, b: usize, friction: Option<f64>) -> Tie {
    let ((xa, za), (xb, zb)) = (list[a], list[b]);
    Tie {
        a: FaceSig { body: a, centroid: [xa + 0.5, 0.5, za + 1.0], normal: [0.0, 0.0, 1.0], area: 1.0 },
        b: FaceSig { body: b, centroid: [xb + 0.5, 0.5, zb], normal: [0.0, 0.0, -1.0], area: 1.0 },
        gap: 0.0,
        friction,
    }
}

/// The z displacement of the nodes at height `z` with `x` in `[lo, hi]`, as (smallest, largest).
fn uz_in(s: &fea_problem::solve::Solved, z: f64, lo: f64, hi: f64) -> (f64, f64) {
    let uz = s.model.mesh.nodes.iter().enumerate().filter(|(_, x)| (x[2] - z).abs() < 1e-9 && x[0] >= lo - 1e-9 && x[0] <= hi + 1e-9).map(|(i, _)| s.u[3 * i + 2]);
    uz.fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), v| (a.min(v), b.max(v)))
}

fn scaled(p: &mut Problem, ratios: &[f64]) {
    p.region_materials = ratios
        .iter()
        .map(|r| {
            let mut m = p.material.clone();
            m.e *= r;
            Some(m)
        })
        .collect();
}

/// The relative distance of `got` from `want`, printed so the penalty's error is on record in the test log.
fn rel(what: &str, got: f64, want: f64) -> f64 {
    let r = (got / want - 1.0).abs();
    eprintln!("{what}: {got:e} vs {want:e}, relative error {r:.2e}");
    r
}

/// Two unit bars (`E` and `ratio E`) stacked, bar 0 fixed at its base: that base is the nut's bearing plane (z = 0), the top of bar 1 the
/// head's (z = 2). Every bearing face is inside the bearing diameter, so the bolt acts on the whole face and the state stays uniform.
fn bolted(interface: Option<f64>, ratio: f64, size: &str, preload: Preload) -> (Problem, [f64; 2]) {
    let list = [(0.0, 0.0), (0.0, 1.0)];
    let mut p = bars(&list);
    scaled(&mut p, &[ratio]);
    p.ties = vec![joint(&list, 0, 1, interface)];
    p.supports = vec![Support::fixed(&bottom(0))];
    let mut f = Fastener::new([0.5, 0.5, 0.0], [0.0, 0.0, 1.0], 0.0, 2.0, size, preload);
    f.head_diameter = Some(100.0);
    f.nut_diameter = Some(100.0);
    p.fasteners = vec![f];
    let e = p.material.e;
    (p, [e, ratio * e])
}

/// The bolt's axial stiffness from first principles for `size` and a grip of `grip`: `1 / sum(L / (E A))` over the head's additional
/// length (0.4 d at d), the shank and the thread's (0.4 d3 at the root diameter d3), in the problem's units.
fn bolt_stiffness(p: &Problem, size: &str, grip: f64, e_bolt: f64) -> f64 {
    let (d, pitch) = match size {
        "M6" => (6.0, 1.0),
        "M10" => (10.0, 1.5),
        _ => unreachable!(),
    };
    let mm = 1e-3 / p.units.length_m();
    let (d, d3) = (d * mm, (d - 1.082_532 * pitch) * mm);
    let area = |x: f64| std::f64::consts::FRAC_PI_4 * x * x;
    1.0 / ((0.4 * d + grip) / (e_bolt * area(d)) + 0.4 * d3 / (e_bolt * area(d3)))
}

struct Bolted {
    k_b: f64,
    k_m: f64,
}

impl Bolted {
    fn phi(&self) -> f64 {
        self.k_b / (self.k_b + self.k_m)
    }
}

fn bolted_case(interface: Option<f64>, ratio: f64, size: &str, preload: f64, bolt_scale: f64, element: ElementChoice) -> (Problem, Bolted) {
    let (mut p, e) = bolted(interface, ratio, size, Preload::Force(preload));
    p.mesh.element = element;
    let e_bolt = bolt_scale * 200e9 / p.units.stress_pa();
    p.fasteners[0].bolt_modulus = Some(e_bolt);
    let k_m = 1.0 / (1.0 / e[0] + 1.0 / e[1]);
    let k_b = bolt_stiffness(&p, size, 2.0, e_bolt);
    (p, Bolted { k_b, k_m })
}


/// The z displacement of the nodes at height `z`, as (smallest, largest).
fn uz_at(s: &fea_problem::solve::Solved, z: f64) -> (f64, f64) {
    let uz = s.model.mesh.nodes.iter().enumerate().filter(|(_, x)| (x[2] - z).abs() < 1e-9).map(|(i, _)| s.u[3 * i + 2]);
    uz.fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| (lo.min(v), hi.max(v)))
}

/// Solve under a wall-clock guard: the release case must fail fast, not hang.
fn guarded(p: Problem, secs: u64) -> Result<fea_problem::solve::Solved, String> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(solve(&p, None));
    });
    rx.recv_timeout(Duration::from_secs(secs)).unwrap_or_else(|_| Err(format!("TIMEOUT after {secs} s")))
}

/// Past `F = P / (1 - Phi)` the interface opens and the bolt alone carries the pull: `T = F`, the upper bar unstressed (it moves as a
/// rigid body by `F / k_b - P / k_m - P / k_b`), the lower bar unstressed, no contact force.
#[test]
fn a_pull_past_the_separation_load_is_carried_by_the_bolt_alone() {
    let preload = 1.0e4;
    for mu in [0.0, 0.3] {
        for element in [ElementChoice::Tri3, ElementChoice::Tri6] {
            for factor in [1.02, 1.2, 3.0] {
                let (mut p, c) = bolted_case(Some(mu), 1.0, "M6", preload, 10.0, element);
                let sep = preload / (1.0 - c.phi());
                let pull = factor * sep;
                p.loads = vec![Load::Traction { edge: top(1), tx: 0.0, ty: 0.0, tz: pull }];
                let s = guarded(p, 120).unwrap_or_else(|e| panic!("mu {mu} {element:?} {factor} x separation: {e}"));
                let what = format!("mu {mu} {element:?} {factor} x separation");
                let f = &s.summary.fasteners[0];
                // the contact solve stops at 1e-6 of its force scale, which is the size of the pull here: the measured error is 1.3e-6 at worst
                assert!(rel(&format!("{what}: tension"), f.tension, pull) < 1e-5, "{what}: {f:?}");
                // the upper bar is a rigid body moved by the bolt's elongation; the bolt is the small difference of terms of the same size
                // A frictional pair carries an engaging overlap (`PairSetup::touch`, 5e-6 in these unit stacks, documented in `fea-problem/AGENTS.md`):
                // its faces stay in contact until the gap reaches it, so the released bar sits that much farther out.
                let overlap = if mu > 0.0 { 5.0e-6 } else { 0.0 };
                let head = pull / c.k_b - preload / c.k_m - preload / c.k_b + overlap;
                let (lo, hi) = uz_at(&s, 2.0);
                let scale = pull / c.k_b;
                eprintln!("{what}: head error {:.2e} / {:.2e} of F/k_b", (lo - head).abs() / scale, (hi - head).abs() / scale);
                assert!((lo - head).abs() < 1e-5 * scale && (hi - head).abs() < 1e-5 * scale, "{what}: {lo:e} {hi:e} vs {head:e}");
                let (lo, hi) = uz_at(&s, 1.0);
                // z = 1 holds the lower bar's top (unstressed, at rest) and the upper bar's bottom (moved with it: a rigid body)
                assert!(lo.abs() < 1e-5 * scale && (hi - head).abs() < 1e-5 * scale, "{what}: the lower bar's top is unstressed, the upper bar rigid: {lo:e} {hi:e} vs {head:e}");
                let n = s.summary.contacts[0].normal_force;
                assert!(n < 1e-6 * pull, "{what}: contact force {n:e}");
                // the solve's own 1e-6 residual against a force scale (the pull) it is not exactly measured on: 1.3e-6 at worst
                assert!(s.summary.equilibrium_error < 3e-6, "{what}: {}", s.summary.equilibrium_error);
            }
        }
    }
}

/// A bonded interface never separates: the bolt carries `P + Phi F` at any pull (the bonded penalty carries the rest).
#[test]
fn a_bonded_interface_carries_a_pull_past_the_separation_load_with_the_bolt_sharing_it() {
    let preload = 1.0e4;
    let (mut p, c) = bolted_case(None, 1.0, "M6", preload, 10.0, ElementChoice::Tri3);
    let pull = 3.0 * preload / (1.0 - c.phi());
    p.loads = vec![Load::Traction { edge: top(1), tx: 0.0, ty: 0.0, tz: pull }];
    let s = guarded(p, 120).unwrap();
    let f = &s.summary.fasteners[0];
    assert!(rel("tension", f.tension, preload + c.phi() * pull) < 1e-5, "{f:?}");
}
