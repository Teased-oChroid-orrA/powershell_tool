//! Speed baseline of the lug solver (ignored by default: timings, not assertions).
//!
//! `cargo test -p lug-solver --release --test bench -- --ignored --nocapture`
//!
//! Every optimisation of the solver (and of the general `fea-core` kernel that builds on it) is
//! measured against these numbers; the recorded reference values live in `docs/fea-core.md`.

use lug_solver::*;
use std::time::{Duration, Instant};

const AL: Material = Material { e_psi: 10.3e6, nu: 0.33 };
const AXIAL: LoadCase = LoadCase { load_lbf: 5000.0, angle_deg: 0.0 };

fn lug() -> LugGeometry {
    LugGeometry::round_head(0.5, 1.5, 0.25, 3.75)
}

/// Best of `n` runs: the minimum is the least noisy estimate of the cost of deterministic work.
fn best<T>(n: usize, mut f: impl FnMut() -> T) -> (Duration, T) {
    let mut out = None;
    let mut min = Duration::MAX;
    for _ in 0..n {
        let t = Instant::now();
        let v = f();
        min = min.min(t.elapsed());
        out = Some(v);
    }
    (min, out.unwrap())
}

fn report(name: &str, d: Duration, extra: &str) {
    eprintln!("BENCH {name:<34} {:>10.2} ms  {extra}", d.as_secs_f64() * 1e3);
}

#[test]
#[ignore]
fn lug_solver_baseline() {
    let pin = PinSpec::new(0.4995, 0.0);

    let (d, half) = best(3, || LugModel::build(&lug(), AL, MeshSpec::default(), true).unwrap());
    report("build half (condense)", d, "");
    let (d, sol) = best(5, || half.solve(pin, AXIAL).unwrap());
    report("solve axial case", d, &format!("newton iters {}", sol.contact_iterations));

    let (d, full) = best(3, || LugModel::build(&lug(), AL, MeshSpec::default(), false).unwrap());
    report("build full (condense)", d, "");
    let oblique = LoadCase { load_lbf: 5000.0, angle_deg: 45.0 };
    let (d, sol) = best(3, || full.solve(pin, oblique).unwrap());
    report("solve oblique 45 deg", d, &format!("newton iters {}", sol.contact_iterations));

    let plane = LugModel::build_with(&lug(), AL, MeshSpec::default(), true, PlaneMode::Strain).unwrap();
    let (d, ll) = best(3, || plane.limit_load(pin, AXIAL, 60_000.0).unwrap());
    report("limit load (small strain EPP)", d, &format!("{:.0} lbf", ll.limit_load_lbf));

    let epp = FsMaterial { e_psi: AL.e_psi, nu: AL.nu, law: Hardening::linear(60_000.0, 1.0, 5.0).unwrap() };
    let mesh32 = MeshSpec { elements_around: 32, ..MeshSpec::default() };
    let (d, fl) = best(3, || FiniteLug::build(&lug(), mesh32, epp, true).unwrap());
    report("finite-strain build (32 around)", d, &format!("factor size {:?}", fl.factor_size()));
    let (d, r) = best(3, || fl.collapse(AXIAL, FsOptions::new(0.4995, 0.0)).unwrap());
    report("finite-strain collapse (EPP)", d, &format!("{:.0} lbf, {} factorisations", r.collapse_lbf, r.factorisations));

    // Raw Q9 element stiffness throughput on a distorted element.
    let xy: Vec<[f64; 2]> = (0..9).map(|n| [(n % 3) as f64 * 0.5 + 0.02 * (n / 3) as f64, (n / 3) as f64 * 0.5]).collect();
    let (nu, e) = (0.33, 10.3e6);
    let c = e / (1.0 - nu * nu);
    let dm = [[c, c * nu, 0.0], [c * nu, c, 0.0], [0.0, 0.0, c * (1.0 - nu) / 2.0]];
    let reps = 20_000;
    let (d, _) = best(5, || {
        let mut acc = 0.0;
        for _ in 0..reps {
            acc += edge_check::fem::element_stiffness(std::hint::black_box(&xy), &dm).unwrap()[0][0];
        }
        acc
    });
    report("Q9 element_stiffness x20000", d, &format!("{:.2} us per element", d.as_secs_f64() * 1e6 / reps as f64));
}
