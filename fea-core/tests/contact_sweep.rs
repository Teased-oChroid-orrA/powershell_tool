//! Seeded random sweeps of the contact kernel on problems other than the ones its fixes were found on: stacked blocks
//! (2D and 3D, matching and non-matching meshes, so slave points sit on master element edges and off them) pressed together and
//! sheared below the friction limit. Each must converge, transmit the pressure exactly and carry the shear by sticking.

use fea_core::contact::ContactSpec;
use fea_core::fit::Tuning;
use fea_core::generate::grid;
use fea_core::*;

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
    fn int(&mut self, lo: usize, hi: usize) -> usize {
        lo + (self.next() * (hi - lo + 1) as f64) as usize % (hi - lo + 1)
    }
}

#[derive(Debug)]
struct Case {
    dim: usize,
    div: [usize; 2],
    slave_is_bottom: bool,
    mu: f64,
    q: f64,
    shear: f64,
    e: [f64; 2],
    nu: f64,
}

fn run(c: &Case) -> Result<(), String> {
    let side = 1.5;
    let (kind, physics, e3) = if c.dim == 2 { (ElementKind::Quad9, Physics::PlaneStrain { thickness: 1.0 }, 1usize) } else { (ElementKind::Hex27, Physics::Solid, 2usize) };
    let block = |n: usize, z0: f64, e: f64| grid(physics, kind, Elastic::new(e, c.nu), [n, if c.dim == 2 { 2 } else { n.min(2) }, e3], &move |p| if c.dim == 2 { [side * p[0], z0 + p[1], 0.0] } else { [side * p[0], side * p[1], z0 + 0.6 * p[2]] });
    let mut mesh = block(c.div[0], 0.0, c.e[0])?;
    let n_bottom = mesh.nodes.len();
    mesh.append(&block(c.div[1], if c.dim == 2 { 1.0 } else { 0.6 }, c.e[1])?, "top_")?;
    let model = Model::new(mesh)?;
    let (bottom, top, face_lo, face_hi) = if c.dim == 2 { ("v0", "top_v1", "v1", "top_v0") } else { ("w0", "top_w1", "w1", "top_w0") };
    let mut bc = model.dirichlet();
    let nodes = &model.mesh.nodes;
    let find = |x: f64, y: f64| (0..nodes.len()).find(|&n| (nodes[n][0] - x).abs() < 1e-12 && (nodes[n][1] - y).abs() < 1e-12 && nodes[n][2] < 1e-12 && model.mesh.node_set(bottom).is_ok_and(|s| s.contains(&n)));
    for &n in model.mesh.node_set(bottom)? {
        bc.fix(n, c.dim - 1, 0.0);
    }
    if c.dim == 2 {
        // (the bottom edge of the 2D grid is `v0`: y = 0)
        bc.fix(model.mesh.node_set(bottom)?[0], 0, 0.0);
    } else {
        let a = find(0.0, 0.0).ok_or("corner")?;
        bc.fix(a, 0, 0.0);
        bc.fix(a, 1, 0.0);
        bc.fix(find(side, 0.0).ok_or("corner")?, 1, 0.0);
    }
    let area = if c.dim == 2 { side } else { side * side };
    let mut traction = [c.shear * c.mu * c.q, 0.0, 0.0];
    let loads = if c.dim == 2 {
        traction[1] = -c.q;
        Loads { faces: model.mesh.surfaces[top].iter().map(|f| (f.clone(), SurfaceLoad::Traction(traction))).collect(), ..Loads::default() }
    } else {
        traction[2] = -c.q;
        Loads { faces: model.mesh.surfaces[top].iter().map(|f| (f.clone(), SurfaceLoad::Traction(traction))).collect(), ..Loads::default() }
    };
    // The top block is held only by the contact: weak springs on the sliding directions keep the first tangent (before friction engages) regular; the pressure direction is held by the contact's own stabiliser, as in the frictionless patch test.
    let k = 1e-9 * c.e[1];
    let ground: Vec<(usize, f64)> = (n_bottom..model.mesh.nodes.len()).flat_map(|n| (0..c.dim - 1).map(move |i| (n * c.dim + i, k))).collect();
    let loads = Loads { ground, ..loads };
    let (lo, hi) = (model.mesh.surfaces[face_lo].clone(), model.mesh.surfaces[face_hi].clone());
    let tune = Tuning::friction();
    let eps_n = tune.eps_n(c.e[0].min(c.e[1]), 0.5);
    let spec = if c.slave_is_bottom { ContactSpec::deformable("interface", lo, hi, eps_n) } else { ContactSpec::deformable("interface", hi, lo, eps_n) };
    // Start engaged, as an interference fit does (a zero-gap start leaves the shear direction held by weak springs alone).
    let spec = spec.with_friction(c.mu, tune.eps_t(eps_n)).with_overlap(2e-4);
    let sol = model.solve_nonlinear_contact(&loads, &bc, vec![spec], &NlOptions { steps: 2, ..tune.options(2) }).map_err(|e| e.to_string())?;
    if !sol.complete() {
        return Err(format!("stopped: {:?}", sol.stop));
    }
    let f = sol.contact.as_ref().ok_or("no contact stats")?.master_force[0];
    let (normal, tang) = (f[c.dim - 1].abs(), f[0].abs());
    let (want_n, want_t) = (c.q * area, c.shear * c.mu * c.q * area);
    if (normal / want_n - 1.0).abs() > 2e-3 {
        return Err(format!("normal force {normal} vs {want_n}"));
    }
    if (tang - want_t).abs() > 2e-2 * want_n * c.mu {
        return Err(format!("shear carried {tang} vs {want_t}"));
    }
    Ok(())
}

/// `max_shear` bounds the share of the friction limit the applied shear takes in 3D (2D takes up to 0.8 everywhere): above
/// ~0.4 the 3D interface is mostly at its friction limit, which Newton does not converge in ~8 % of random cases (see
/// `partial_slip_3d_measurement`, `fea-core/AGENTS.md`).
fn sweep(seed: u64, n: usize, max_shear_3d: f64) {
    let mut rng = Rng(seed);
    let mut failures = Vec::new();
    for k in 0..n {
        let dim = if rng.next() < 0.6 { 2 } else { 3 };
        let m = if dim == 2 { 6 } else { 3 };
        let div = [rng.int(1, m), rng.int(1, m)];
        let c = Case { dim, div, slave_is_bottom: rng.next() < 0.5, mu: rng.range(0.1, 0.5), q: rng.range(1e3, 5e4), shear: rng.range(0.0, if dim == 3 { max_shear_3d } else { 0.8 }), e: [rng.range(5e6, 3e7), rng.range(5e6, 3e7)], nu: rng.range(0.2, 0.4) };
        if std::env::var("SWEEP_ONLY").is_ok_and(|v| v != k.to_string()) {
            continue;
        }
        if let Err(e) = run(&c) {
            failures.push(format!("#{k} {c:?}: {e}"));
        }
    }
    assert!(failures.is_empty(), "{} of {n} failed:\n{}", failures.len(), failures.join("\n"));
}

#[test]
fn random_stacked_blocks_stick_under_shear() {
    sweep(3, 14, 0.4);
}

#[test]
#[ignore = "slow soak: run with --ignored"]
fn random_stacked_blocks_stick_under_shear_soak() {
    sweep(99, 60, 0.4);
}





/// Measurement, not a gate: the share of random 3D cases sheared to 0.4-0.8 of the friction limit that fail to converge.
#[test]
#[ignore = "measurement: run with --ignored --nocapture"]
fn partial_slip_3d_measurement() {
    let mut rng = Rng(7);
    let (mut failed, mut total) = (0, 0);
    while total < 24 {
        let c = Case { dim: 3, div: [rng.int(1, 3), rng.int(1, 3)], slave_is_bottom: rng.next() < 0.5, mu: rng.range(0.1, 0.5), q: rng.range(1e3, 5e4), shear: rng.range(0.4, 0.8), e: [rng.range(5e6, 3e7), rng.range(5e6, 3e7)], nu: rng.range(0.2, 0.4) };
        total += 1;
        if let Err(e) = run(&c) {
            failed += 1;
            eprintln!("3D partial slip failure {c:?}: {e}");
        }
    }
    eprintln!("3D partial slip: {failed} of {total} failed");
}
