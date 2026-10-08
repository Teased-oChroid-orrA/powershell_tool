//! Random sweep over valid bushed-plate problems (the Workbench's general contact path): every one must solve, balance and
//! return an interface. The failing case is printed with its inputs (the reproduction). Seeded: the same cases everywhere.

use fea_problem::solve::solve;
use fea_problem::templates::templates;
use fea_problem::*;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        ((z ^ (z >> 31)) >> 11) as f64 / (1u64 << 53) as f64
    }
    fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.next()
    }
    fn pick<T: Copy>(&mut self, v: &[T]) -> T {
        v[(self.next() * v.len() as f64) as usize % v.len()]
    }
}

const MODULI: [f64; 5] = [6.5e6, 10.4e6, 15.0e6, 16.5e6, 29.0e6];

fn random_case(rng: &mut Rng) -> Problem {
    let mut p = templates().into_iter().find(|(n, _)| *n == "Lug with an eccentric bushing").unwrap().1;
    let r = rng.range(0.15, 0.6);
    let wall = rng.range(0.1, 0.35) * r;
    let width = r * rng.range(3.0, 6.0);
    let len = width * rng.range(1.5, 3.0);
    p.geometry = Geometry::Sketch { outer: Shape::Rect { x0: 0.0, y0: 0.0, x1: len, y1: width }, holes: vec![Shape::Circle { cx: width / 2.0, cy: width / 2.0, r }], depth: 1.0, layers: 1 };
    p.thickness = rng.range(0.2, 1.0) * r * 2.0;
    p.material.e = rng.pick(&MODULI);
    p.material.nu = rng.range(0.28, 0.35);
    p.mesh = MeshSpec { size: r / rng.range(1.2, 2.0), hole_factor: rng.range(0.12, 0.3), ..MeshSpec::default() };
    let id = 2.0 * (r - wall);
    let ecc = rng.range(0.0, 0.5 * (wall - 0.03 * 2.0 * r).max(0.0)) * rng.pick(&[0.0, 1.0, 1.0]);
    let ang = rng.range(0.0, std::f64::consts::TAU);
    let load = rng.range(0.03, 0.4) * 20_000.0 * id * p.thickness;
    let dir = rng.pick(&[0.0f64, 0.4, 1.2, 2.0]);
    p.bushings[0].inner_diameter = id;
    p.bushings[0].offset = [ecc * ang.cos(), ecc * ang.sin()];
    p.bushings[0].interference = rng.range(0.0008, 0.004) * 2.0 * r / 0.5;
    p.bushings[0].friction = rng.range(0.05, 0.3);
    p.bushings[0].material.e = rng.pick(&MODULI);
    p.bushings[0].material.nu = rng.range(0.28, 0.35);
    p.loads = vec![Load::Bearing { hole: 1, fx: -load * dir.cos(), fy: -load * dir.sin() }];
    p
}

fn sweep(seed: u64, n: usize) {
    let mut rng = Rng(seed);
    let cases: Vec<Problem> = (0..n).map(|_| random_case(&mut rng)).collect();
    let results: Vec<String> = std::thread::scope(|s| {
        let hs: Vec<_> = cases
            .chunks(n.div_ceil(4))
            .map(|chunk| {
                s.spawn(move || {
                    chunk
                        .iter()
                        .map(|p| match p.validate().and_then(|()| solve(p, None)) {
                            Ok(s) if s.summary.equilibrium_error < 1e-2 && !s.summary.interfaces.is_empty() => format!("ok {:.2}s eq {:.1e}", s.summary.solve_ms / 1e3, s.summary.equilibrium_error),
                            Ok(s) => format!("FAIL eq {:e} ifaces {} for {}", s.summary.equilibrium_error, s.summary.interfaces.len(), p.to_json().unwrap().replace('\n', " ")),
                            Err(e) => format!("FAIL {e} for {}", p.to_json().unwrap().replace('\n', " ")),
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        hs.into_iter().flat_map(|h| h.join().unwrap()).collect()
    });
    for r in &results {
        eprintln!("{}", &r[..r.len().min(2000)]);
    }
    let failed: Vec<&String> = results.iter().filter(|m| m.starts_with("FAIL")).collect();
    assert!(failed.is_empty(), "{} of {n} random bushed plates failed", failed.len());
}

#[test]
#[ignore = "slow soak (~2 min): run with --ignored"]
fn random_valid_bushed_plates_always_solve() {
    let env = |k: &str, d: u64| std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(d);
    sweep(env("SWEEP_SEED", 11), env("SWEEP_N", 16) as usize);
}

/// Found by the sweep above (seed 11): a concentric fit whose slave and master meshes match, so slave points sit exactly on
/// master element edges. The minimum-gap face choice flipped between two faces with different normals on round-off, the
/// residual jumped by ~5 lbf at 5 points, and Newton stagnated at every step size. The slave point now stays on its
/// committed face while it is over it (`fea-core` contact, `STICKY_TOL`).
#[test]
fn a_slave_point_on_a_master_element_edge_does_not_flip_faces() {
    let p: Problem = serde_json::from_str(include_str!("data/edge_ambiguity_concentric.json")).unwrap();
    let s = solve(&p, None).unwrap();
    assert!(s.summary.equilibrium_error < 1e-3, "equilibrium {:e}", s.summary.equilibrium_error);
    assert!((s.summary.reaction[0].abs() - 4100.13).abs() < 0.01 * 4100.13, "reaction {:?}", s.summary.reaction);
}

/// Replays one problem JSON (`SWEEP_CASE=path`), e.g. a failure printed above; combine with `NL_TRACE` / `NL_DEBUG`.
#[test]
#[ignore = "manual: set SWEEP_CASE to a problem JSON"]
fn replay_one_case() {
    let path = std::env::var("SWEEP_CASE").expect("SWEEP_CASE");
    let p: Problem = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let t = std::time::Instant::now();
    let r = solve(&p, None);
    eprintln!("{:?} in {:.1}s", r.as_ref().map(|s| s.summary.equilibrium_error), t.elapsed().as_secs_f64());
    r.unwrap();
}

/// Measurement: adaptive passes on the bushed template against a uniformly fine reference.
#[test]
#[ignore = "manual measurement"]
fn adaptive_bushed_measure() {
    let base = templates().into_iter().find(|(n, _)| *n == "Lug with an eccentric bushing").unwrap().1;
    let run = |label: &str, p: &Problem| {
        let t = std::time::Instant::now();
        let s = solve(p, None).unwrap();
        let f = &s.summary.interfaces[0];
        eprintln!("{label}: {} nodes, zz {:.4}, vm {:.0}, press mean {:.0} peak {:.0} open {:.0} torque {:.2}, eq {:.1e}, {:.1}s, history {:?}", s.summary.nodes, s.summary.zz_error, s.summary.max_von_mises.value, f.mean_pressure, f.peak_pressure, f.open_arc_deg, f.torque_capacity, s.summary.equilibrium_error, t.elapsed().as_secs_f64(), s.history.iter().map(|h| (h.nodes, (h.zz_error * 1e4).round() / 1e4)).collect::<Vec<_>>());
    };
    let mut p = base.clone();
    p.mesh.size = 0.08;
    run("fine ref", &p);
    run("default", &base);
    let mut a = base.clone();
    a.mesh.adapt_passes = 3;
    a.mesh.target_error = 0.02;
    run("adaptive", &a);
}

/// The regression corpus: every problem in `tests/data` once failed the kernel (each file's name says how). Slow (a
/// minute for the hardest); the cheapest has its own test above.
#[test]
#[ignore = "slow: run with --ignored"]
fn every_corpus_case_solves() {
    let mut failed = Vec::new();
    for entry in std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data")).unwrap() {
        let path = entry.unwrap().path();
        let p: Problem = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        match solve(&p, None) {
            Ok(s) if s.summary.equilibrium_error < 1e-2 => {}
            Ok(s) => failed.push(format!("{}: equilibrium {:e}", path.display(), s.summary.equilibrium_error)),
            Err(e) => failed.push(format!("{}: {e}", path.display())),
        }
    }
    assert!(failed.is_empty(), "{failed:#?}");
}

fn template(name: &str) -> Problem {
    templates().into_iter().find(|(n, _)| *n == name).unwrap().1
}

/// The linear (no contact) templates with random geometry, element type, mesh size and adaptive passes: every one must solve
/// and balance its reactions against its loads to round-off.
#[test]
fn random_linear_problems_solve_and_balance() {
    let mut rng = Rng(21);
    let mut failures = Vec::new();
    for k in 0..14 {
        let name = rng.pick(&["Plate with a hole", "Pin-loaded lug", "Cantilever beam", "L-bracket (adaptive mesh)", "Extruded plate with a hole (3D)"]);
        let mut p = template(name);
        p.material.e = rng.pick(&MODULI);
        p.material.nu = rng.range(0.2, 0.4);
        p.thickness *= rng.range(0.5, 2.0);
        p.mesh.size *= rng.range(0.7, 1.6);
        p.mesh.element = rng.pick(&[ElementChoice::Tri6, ElementChoice::Quad8, ElementChoice::Quad9]);
        if p.analysis != Analysis::Solid {
            p.mesh.adapt_passes = rng.pick(&[0, 1, 2]);
        }
        for l in &mut p.loads {
            match l {
                Load::Pressure { p: v, .. } => *v *= rng.range(0.3, 3.0),
                Load::Traction { tx, ty, tz, .. } => {
                    let f = rng.range(0.3, 3.0);
                    (*tx, *ty, *tz) = (*tx * f, *ty * f, *tz * f);
                }
                Load::Force { fx, fy, fz, .. } => {
                    let f = rng.range(0.3, 3.0);
                    (*fx, *fy, *fz) = (*fx * f, *fy * f, *fz * f);
                }
                Load::Bearing { fx, fy, .. } => {
                    let f = rng.range(0.3, 3.0);
                    (*fx, *fy) = (*fx * f, *fy * f);
                }
                _ => {}
            }
        }
        match p.validate().and_then(|()| solve(&p, None)) {
            Ok(s) if s.summary.equilibrium_error < 1e-6 && s.summary.zz_error.is_finite() && s.summary.max_von_mises.value > 0.0 => {}
            Ok(s) => failures.push(format!("#{k} {name}: equilibrium {:e}, zz {}, vm {}", s.summary.equilibrium_error, s.summary.zz_error, s.summary.max_von_mises.value)),
            Err(e) => failures.push(format!("#{k} {name}: {e}")),
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

/// Random bolted-joint stacks (the Preload Analysis FE member compliance): positive, finite, and converged in the mesh.
#[test]
fn random_joint_stacks_have_a_converged_member_compliance() {
    use fea_problem::joint::{member_compliance, member_compliance_sized, Layer};
    let mut rng = Rng(5);
    for k in 0..12 {
        let hole = rng.range(0.15, 0.5);
        let head = hole * rng.range(1.5, 2.2);
        let n = rng.pick(&[1usize, 2, 3]);
        let stack: Vec<Layer> = (0..n).map(|_| Layer { thickness: rng.range(0.1, 0.6), e: rng.pick(&MODULI), nu: rng.range(0.25, 0.35), hole_diameter: hole * rng.range(1.0, 1.1), outer_diameter: { let od = head * rng.range(1.5, 4.0); rng.pick(&[None, Some(od)]) } }).collect();
        let coarse = member_compliance(&stack, head, head).unwrap_or_else(|e| panic!("#{k} {stack:?}: {e}"));
        let fine = member_compliance_sized(&stack, head, head, 0.06).unwrap_or_else(|e| panic!("#{k} fine {stack:?}: {e}"));
        assert!(coarse.compliance.is_finite() && coarse.compliance > 0.0, "#{k} {stack:?}");
        assert!((coarse.compliance / fine.compliance - 1.0).abs() < 0.03, "#{k} {:e} vs {:e} for {stack:?}", coarse.compliance, fine.compliance);
    }
}
