//! Random sweep over valid inputs: every one must analyse without a solver failure, the interface must return the pin load,
//! and the margin must be finite. The case that fails is printed with its inputs (the reproduction).

use eccentric_bushing::{analyze, Elasticity, Inputs};

/// A small deterministic generator (SplitMix64): no dependency, the same cases on every platform.
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

/// Elastic moduli (psi) of the materials these parts are made of: magnesium, aluminium, bronze, titanium, steel.
const MODULI: [f64; 5] = [6.5e6, 10.4e6, 15.0e6, 16.5e6, 29.0e6];

fn random_case(rng: &mut Rng, coarse: bool) -> Inputs {
    let bore_dia = rng.range(0.25, 1.0);
    let wall = rng.range(0.1, 0.35) * bore_dia / 2.0 * 2.0 / 2.0;
    let bushing_id = bore_dia - 2.0 * wall;
    let min_thin = 0.03 * bore_dia;
    let offset = rng.range(0.0, (wall - min_thin - 1e-4).max(0.0)) * rng.pick(&[0.0, 0.5, 1.0, 1.0]);
    let thickness = rng.range(0.1, 0.6) * bore_dia * 2.0;
    let housing = Elasticity::iso(rng.pick(&MODULI), rng.range(0.28, 0.35));
    let bushing = Elasticity::iso(rng.pick(&MODULI), rng.range(0.28, 0.35));
    let pin = Elasticity::iso(rng.pick(&[10.4e6, 16.5e6, 29.0e6]), 0.30);
    let bearing = rng.range(0.03, 0.6) * 20_000.0; // psi on the pin's projected area
    Inputs {
        bore_dia,
        housing_od: bore_dia * rng.range(1.8, 3.0),
        edge_distance: None,
        bushing_id,
        offset,
        interference_dia: rng.range(0.0008, 0.004) * bore_dia / 0.5,
        thickness,
        housing,
        bushing,
        friction: rng.range(0.08, 0.3),
        pin,
        pin_friction: rng.range(0.05, 0.25),
        pin_clearance_dia: rng.range(0.0005, 0.002),
        credit_pin_load: true,
        load_lbf: bearing * bushing_id * thickness,
        load_angle_deg: rng.pick(&[90.0, 90.0, 45.0, 135.0, 0.0]),
        direct_onset: false,
        min_wall: 0.0,
        plane_strain: false,
        mesh_size: coarse.then_some(bore_dia / 2.0 / 4.0),
    }
}

/// The cases whose pin load could not be solved are returned as fit-alone analyses (`Analysis::loaded_failure`): they are
/// listed, and only a few are tolerated, so a regression of the loaded stage cannot hide behind the fallback.
fn sweep(seed: u64, n: usize, coarse: bool, tolerated_degraded: usize) {
    let mut rng = Rng(seed);
    let cases: Vec<Inputs> = (0..n).map(|_| random_case(&mut rng, coarse)).collect();
    let results: Vec<(String, bool)> = std::thread::scope(|s| {
        let handles: Vec<_> = cases
            .chunks(n.div_ceil(4))
            .map(|chunk| {
                s.spawn(move || {
                    chunk
                        .iter()
                        .map(|c| {
                            let no_torque = c.offset == 0.0 || c.load_angle_deg == 0.0;
                            match c.validate().and_then(|()| analyze(c)) {
                                Ok(a) if a.margin.is_finite() == !no_torque && a.margin > -1.0 => (format!("ok margin {:.3}", a.margin), a.loaded_failure.is_some()),
                                Ok(a) => (format!("FAIL margin {} (no spin torque: {no_torque}) for {c:?}", a.margin), true),
                                Err(e) => (format!("FAIL {e} for {c:?}"), true),
                            }
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles.into_iter().flat_map(|h| h.join().unwrap()).collect()
    });
    let failed: Vec<&String> = results.iter().filter(|(m, _)| m.starts_with("FAIL")).map(|(m, _)| m).collect();
    assert!(failed.is_empty(), "{} of {n} random cases failed:\n{}", failed.len(), failed.iter().map(|m| m.as_str()).collect::<Vec<_>>().join("\n"));
    let degraded = results.iter().filter(|(_, d)| *d).count();
    assert!(degraded <= tolerated_degraded, "{degraded} of {n} cases could not solve the pin load (at most {tolerated_degraded} tolerated)");
}

#[test]
fn random_valid_inputs_are_always_analysable_on_a_coarse_mesh() {
    // Seed 2024 holds the cases that used to fail the loaded stage (concentric, load along the offset line, stiff walls).
    sweep(2024, 12, true, 0);
}

#[test]
#[ignore = "slow soak: default meshes, run with --ignored"]
fn random_valid_inputs_are_always_analysable_on_the_default_mesh() {
    sweep(77, 24, false, 0);
}

/// Regression: at a stagnating step of this case the Newton iterate had lost the pin's contact altogether, and holding that active
/// set "converged" to a pin carrying none of its load (silently, until the consumer's equilibrium guard fell back to the fit alone).
/// A hold is only taken when the iterate still has at least half the contact the step started with (`nonlinear.rs`).
#[test]
fn a_stagnating_step_that_lost_the_pin_is_not_frozen_into_a_wrong_answer() {
    let mut rng = Rng(77);
    let case = (0..24).map(|_| random_case(&mut rng, false)).find(|c| (c.bore_dia - 0.4766061927066888).abs() < 1e-12).expect("the case of seed 77");
    let a = analyze(&case).unwrap();
    assert!(a.loaded_failure.is_none(), "{:?}", a.loaded_failure);
}

/// Measurement, not a gate: how often the random cases need the kernel's active-set hold, and whether any falls back to the fit alone
/// (the nets that once sat in between, other loaded attempts, mesh routes and stalled-residual acceptance, were reached 0, 0 and 3 times in 192 cases and were deleted). `SWEEP_SEED`, `SWEEP_N`,
/// `SWEEP_COARSE=1`. A net that nothing reaches over a large corpus is dead weight.
#[test]
#[ignore = "measurement: run with --ignored --nocapture"]
fn safety_net_usage_measurement() {
    let env = |k: &str, d: u64| std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(d);
    let (seed, n, coarse) = (env("SWEEP_SEED", 31), env("SWEEP_N", 48) as usize, env("SWEEP_COARSE", 0) == 1);
    let mut rng = Rng(seed);
    let only = std::env::var("SWEEP_MATCH").ok(); // replay the cases whose `{:?}` contains this text (a failure printed above)
    let cases: Vec<Inputs> = (0..n).map(|_| random_case(&mut rng, coarse)).filter(|c| only.as_ref().is_none_or(|m| format!("{c:?}").contains(m.as_str()))).collect();
    let n = cases.len().max(1);
    let rows: Vec<String> = std::thread::scope(|s| {
        let hs: Vec<_> = cases
            .chunks(n.div_ceil(4))
            .map(|chunk| {
                s.spawn(move || {
                    chunk
                        .iter()
                        .map(|c| match c.validate().and_then(|()| analyze(c)) {
                            Ok(a) => {
                                if let Some(why) = &a.loaded_failure {
                                    eprintln!("FIT ONLY ({why}): {c:?}");
                                }
                                format!("held={} fitonly={}", a.held_solves > 0, a.loaded_failure.is_some())
                            }
                            Err(e) => format!("ERROR {e}"),
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        hs.into_iter().flat_map(|h| h.join().unwrap()).collect()
    });
    let mut tally = std::collections::BTreeMap::new();
    for r in &rows {
        *tally.entry(r.clone()).or_insert(0usize) += 1;
    }
    for (k, v) in &tally {
        eprintln!("{v:4} x {k}");
    }
    eprintln!("net usage over {n} random cases (seed {seed}, coarse {coarse}): {tally:?}");
}
