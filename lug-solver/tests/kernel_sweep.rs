//! Random sweep over valid pin-loaded lugs on the kernel (`FeaLug`): bushed and plain, rigid pin with clearance or
//! interference, friction, axial and oblique loads. Every case must solve and pass its own verification (equilibrium,
//! contact force); failing cases are printed with their inputs. Seeded SplitMix64: the same cases on every platform.
//! Slow soak: `cargo test -p lug-solver --test kernel_sweep -- --ignored` (`SWEEP_SEED`, `SWEEP_N`).

use lug_solver::fea::FeaLug;
use lug_solver::*;

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

#[derive(Debug, Clone)]
struct Case {
    geom: LugGeometry,
    material: Material,
    around: usize,
    half: bool,
    bushing: Option<BushingSpec>,
    pin: (f64, f64),
    load: f64,
    angle: f64,
}

fn random_case(rng: &mut Rng) -> Case {
    let hole = rng.range(0.25, 0.8);
    let width = hole * rng.range(2.0, 3.5);
    let geom = LugGeometry::round_head(hole, width, rng.range(0.1, 0.4) * hole * 2.0, width * rng.range(2.0, 3.5));
    let material = Material { e_psi: rng.pick(&MODULI), nu: rng.range(0.28, 0.35) };
    let angle = rng.pick(&[0.0, 0.0, 30.0, 45.0, 60.0]);
    let bushing = rng.pick(&[false, true, true]).then(|| BushingSpec { inner_dia: hole * rng.range(0.7, 0.88), material: Material { e_psi: rng.pick(&MODULI), nu: 0.3 }, interference_dia: rng.range(0.0008, 0.004) * hole / 0.5, friction: rng.range(0.05, 0.3) });
    let bore = bushing.map_or(hole, |b| b.inner_dia);
    let clearance = rng.pick(&[0.0, 0.0005, 0.001, -0.001]);
    let bearing = rng.range(0.05, 0.3) * 20_000.0;
    Case { geom, material, around: rng.pick(&[24, 32, 40]), half: angle == 0.0, bushing, pin: (bore - clearance * bore / 0.5, rng.pick(&[0.0, 0.1, 0.2])), load: bearing * bore * geom.thickness, angle }
}

fn run(c: &Case) -> Result<(), String> {
    let spec = MeshSpec { elements_around: c.around, ..Default::default() };
    let fe = FeaLug::build_bushed(&c.geom, c.material, spec, c.half, PlaneMode::Stress, c.bushing)?;
    let sol = fe.analyze(PinSpec::new(c.pin.0, c.pin.1), LoadCase { load_lbf: c.load, angle_deg: c.angle })?;
    if !(sol.peak_pressure > 0.0 && sol.peak_von_mises.is_finite()) {
        return Err(format!("no contact pressure / non-finite stress ({} / {})", sol.peak_pressure, sol.peak_von_mises));
    }
    if !sol.verification.ok() {
        return Err(format!("verification failed: {:?}", sol.verification));
    }
    Ok(())
}

#[test]
#[ignore = "slow soak: run with --ignored"]
fn random_valid_lugs_always_solve_on_the_kernel() {
    let env = |k: &str, d: u64| std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(d);
    let (seed, n) = (env("SWEEP_SEED", 5), env("SWEEP_N", 24) as usize);
    let mut rng = Rng(seed);
    let mut cases: Vec<Case> = (0..n).map(|_| random_case(&mut rng)).collect();
    if let Ok(i) = std::env::var("SWEEP_ONLY") {
        cases = vec![cases[i.parse::<usize>().unwrap()].clone()];
    }
    let n = cases.len();
    let failures: Vec<String> = std::thread::scope(|s| {
        let hs: Vec<_> = cases.chunks(n.div_ceil(4)).enumerate().map(|(k, ch)| s.spawn(move || ch.iter().enumerate().filter_map(|(i, c)| run(c).err().map(|e| format!("#{} {e} for {c:?}", k * n.div_ceil(4) + i))).collect::<Vec<_>>())).collect();
        hs.into_iter().flat_map(|h| h.join().unwrap()).collect()
    });
    for f in &failures {
        eprintln!("FAIL {f}");
    }
    assert!(failures.is_empty(), "{} of {n} random lugs failed", failures.len());
}
