//! Held-out validation of `meshing::cost` (#60): the factor size is predicted exactly from the mesh (deterministic, always
//! checked); the predicted solve time uses rates calibrated on one mesh and is compared with the measured time on other
//! sizes of the same family and on other families (timing, `--ignored`, release: `docs/issue-46-phase-82.md`).

use fea_core::delaunay::MeshOptions;
use fea_core::generate::grid;
use fea_core::geometry::{Loop, Region};
use fea_core::meshing::cost::{CostModel, MeshCost, Workload};
use fea_core::mesh2d::mesh_region;
use fea_core::*;

fn quad(n: usize) -> Mesh {
    grid(Physics::PlaneStress { thickness: 1.0 }, ElementKind::Quad8, Elastic::new(1.0e7, 0.3), [n, n, 1], &|p| p).unwrap()
}

fn hex(n: usize) -> Mesh {
    grid(Physics::Solid, ElementKind::Hex8, Elastic::new(1.0e7, 0.3), [n, n, n], &|p| p).unwrap()
}

fn hole(h: f64) -> Mesh {
    let region = Region::new(Loop::rectangle(0.0, 0.0, 12.0, 4.0).unwrap(), vec![Loop::circle([6.0, 2.0], 0.6, "hole").unwrap()], Elastic::new(1.0e7, 0.3)).unwrap();
    mesh_region(&region, Physics::PlaneStress { thickness: 1.0 }, ElementKind::Tri6, &|_| h, MeshOptions::default()).unwrap()
}

/// Measured seconds of one assemble + factor + back-solve (best of three), and the factor size the solver reports.
fn measure(mesh: &Mesh) -> (f64, usize) {
    let model = Model::new(mesh.clone()).unwrap();
    let mut bc = model.dirichlet();
    let xmin = mesh.nodes.iter().map(|x| x[0]).fold(f64::INFINITY, f64::min);
    let xmax = mesh.nodes.iter().map(|x| x[0]).fold(f64::NEG_INFINITY, f64::max);
    for (n, x) in mesh.nodes.iter().enumerate() {
        if x[0] <= xmin + 1e-9 {
            for c in 0..mesh.dim() {
                bc.fix(n, c, 0.0);
            }
        }
    }
    let loads = Loads { nodal: mesh.nodes.iter().enumerate().filter(|(_, x)| x[0] >= xmax - 1e-9).map(|(n, _)| (n, [1.0, 0.0, 0.0])).collect(), ..Loads::default() };
    let (mut best, mut nnz) = (f64::INFINITY, 0);
    for _ in 0..3 {
        let sol = model.solve_static(&loads, &bc).unwrap();
        let t = sol.timings;
        best = best.min(1e-3 * (t.assemble_ms + t.factor_ms + t.solve_ms));
        nnz = sol.factor_nnz;
    }
    (best, nnz)
}

#[test]
fn the_predicted_factor_size_matches_the_solver_on_every_family() {
    for (name, mesh) in [("quad8 20", quad(20)), ("quad8 45", quad(45)), ("hex8 8", hex(8)), ("hex8 12", hex(12)), ("hole 0.5", hole(0.5)), ("hole 0.25", hole(0.25))] {
        let predicted = MeshCost::of(&mesh).unwrap().factor_nnz;
        // `MeshCost::of` fixes one node, the measurement fixes a face: the factor sizes agree to a small fraction.
        let (_, actual) = measure(&mesh);
        let ratio = predicted as f64 / actual as f64;
        eprintln!("{name}: predicted factor nnz {predicted}, solver {actual} (ratio {ratio:.3})");
        assert!(ratio > 0.9 && ratio < 1.25, "{name}: {ratio}");
    }
}

#[test]
#[ignore = "timing: run with --release --ignored --nocapture"]
fn predicted_solve_times_are_compared_with_measured_on_held_out_meshes() {
    let families: [(&str, Vec<Mesh>); 3] = [
        ("quad8", vec![quad(30), quad(15), quad(45), quad(70)]),
        ("hex8", vec![hex(10), hex(6), hex(14), hex(18)]),
        ("hole tri6", vec![hole(0.3), hole(0.6), hole(0.2), hole(0.12)]),
    ];
    let mut models = Vec::new();
    for (name, meshes) in &families {
        let model = CostModel::calibrate(&meshes[0], 3).unwrap();
        models.push((*name, model));
        for m in &meshes[1..] {
            let (meas, _) = measure(m);
            let pred = model.predict(&MeshCost::of(m).unwrap(), &Workload::linear_static(), 0.0);
            eprintln!("{name:<10} same family, {:>6} dofs: predicted {pred:.4} s, measured {meas:.4} s, ratio {:.2}", m.n_dofs(), pred / meas);
        }
    }
    // Cross-family: rates calibrated on one family applied to another.
    for (from, model) in &models {
        for (to, meshes) in &families {
            if from == to {
                continue;
            }
            let m = &meshes[2];
            let (meas, _) = measure(m);
            let pred = model.predict(&MeshCost::of(m).unwrap(), &Workload::linear_static(), 0.0);
            eprintln!("calibrated on {from:<9} applied to {to:<9} ({:>6} dofs): predicted {pred:.4} s, measured {meas:.4} s, ratio {:.2}", m.n_dofs(), pred / meas);
        }
    }
}

#[test]
#[ignore = "timing: run with --release --ignored --nocapture"]
fn stage_times_against_their_work_units() {
    let sets: [(&str, Vec<Mesh>); 3] = [
        ("quad8", vec![quad(20), quad(40), quad(80), quad(120)]),
        ("hex8", vec![hex(8), hex(12), hex(16), hex(20), hex(24)]),
        ("hole tri6", vec![hole(0.4), hole(0.2), hole(0.12), hole(0.08)]),
    ];
    for (name, meshes) in &sets {
        for m in meshes {
            let c = MeshCost::of(m).unwrap();
            let model = Model::new(m.clone()).unwrap();
            let mut bc = model.dirichlet();
            let xmin = m.nodes.iter().map(|x| x[0]).fold(f64::INFINITY, f64::min);
            for (n, x) in m.nodes.iter().enumerate() {
                if x[0] <= xmin + 1e-9 {
                    for k in 0..m.dim() {
                        bc.fix(n, k, 0.0);
                    }
                }
            }
            let loads = Loads { nodal: vec![(m.nodes.len() - 1, [1.0, 0.0, 0.0])], ..Loads::default() };
            let (mut a, mut f, mut s) = (f64::INFINITY, f64::INFINITY, f64::INFINITY);
            for _ in 0..3 {
                let t = model.solve_static(&loads, &bc).unwrap().timings;
                a = a.min(t.assemble_ms);
                f = f.min(t.factor_ms);
                s = s.min(t.solve_ms);
            }
            eprintln!("{name:<9} dofs {:>7} factor_nnz {:>9} | assemble {a:>8.2} ms ({:.2e} ms/unit) factor {f:>8.2} ms ({:.2e} ms/nnz^1.5) solve {s:>7.2} ms ({:.2e} ms/nnz)", c.dofs, c.factor_nnz, a / c.assembly_units, f / c.factor_units, s / c.factor_nnz as f64);
        }
    }
}

#[test]
#[ignore = "timing: run with --release --ignored --nocapture"]
fn large_hex_blocks_show_a_solver_strategy_change() {
    for n in [22usize, 26, 29, 32] {
        let m = hex(n);
        let model = Model::new(m.clone()).unwrap();
        let mut bc = model.dirichlet();
        for (i, x) in m.nodes.iter().enumerate() {
            if x[0] <= 1e-9 {
                for k in 0..3 {
                    bc.fix(i, k, 0.0);
                }
            }
        }
        let loads = Loads { nodal: vec![(m.nodes.len() - 1, [1.0, 0.0, 0.0])], ..Loads::default() };
        let sol = model.solve_static(&loads, &bc).unwrap();
        let t = sol.timings;
        // The cost model flags exactly the sizes the solver switched to the iterative path for.
        assert_eq!(MeshCost::of(&m).unwrap().iterative, sol.factor_nnz == 0, "hex {n}");
        if n == 32 {
            use fea_core::meshing::cost::{choose_within, Candidate, Equivalence, Limits};
            let model = CostModel { assembly_s_per_unit: 1e-9, factor_s_per_unit: 1e-9, solve_s_per_factor_nnz: 1e-9 };
            let cand = |name: &str, mesh: Mesh| Candidate { name: name.into(), mesh, generation_seconds: 0.0, equivalence: Equivalence::Nominal };
            let cands = [cand("small", hex(6)), cand("iterative", m.clone())];
            let lim = Limits { max_memory_bytes: Some(u64::MAX), max_seconds: None };
            let choice = choose_within(&cands, &model, &Workload::linear_static(), false, lim).unwrap();
            assert_eq!(choice.index, 0);
            assert!(choice.rejected.iter().any(|(i, why)| *i == 1 && why.contains("iteratively")), "{:?}", choice.rejected);
        }
        eprintln!("hex {n}: dofs {} factor_nnz {} predicted {} | assemble {:.0} symbolic {:.0} factor {:.0} solve {:.0} ms", m.n_dofs(), sol.factor_nnz, MeshCost::of(&m).unwrap().factor_nnz, t.assemble_ms, t.symbolic_ms, t.factor_ms, t.solve_ms);
    }
}

// ---- #61: held-out evaluation of the learned model --------------------------------------------------------------------

use fea_core::meshing::learned_cost::{LearnedCost, Sample, Source};

/// Family name, dimension, mesh, its cost features and the measured (assemble, factor, solve) seconds.
type Measured<'a> = (&'a str, usize, Mesh, MeshCost, (f64, f64, f64));

fn stage_times(mesh: &Mesh) -> Option<(f64, f64, f64)> {
    let cost = MeshCost::of(mesh).unwrap();
    if cost.iterative {
        return None;
    }
    let model = Model::new(mesh.clone()).unwrap();
    let mut bc = model.dirichlet();
    let xmin = mesh.nodes.iter().map(|x| x[0]).fold(f64::INFINITY, f64::min);
    for (n, x) in mesh.nodes.iter().enumerate() {
        if x[0] <= xmin + 1e-9 {
            for c in 0..mesh.dim() {
                bc.fix(n, c, 0.0);
            }
        }
    }
    let loads = Loads { nodal: vec![(mesh.nodes.len() - 1, [1.0, 0.0, 0.0])], ..Loads::default() };
    let (mut a, mut f, mut s) = (f64::INFINITY, f64::INFINITY, f64::INFINITY);
    for _ in 0..3 {
        let t = model.solve_static(&loads, &bc).unwrap().timings;
        a = a.min(t.assemble_ms);
        f = f.min(t.factor_ms);
        s = s.min(t.solve_ms);
    }
    Some((a * 1e-3, f * 1e-3, s * 1e-3))
}

fn quad4(n: usize) -> Mesh {
    grid(Physics::PlaneStress { thickness: 1.0 }, ElementKind::Quad4, Elastic::new(1.0e7, 0.3), [n, n, 1], &|p| p).unwrap()
}

fn hex20(n: usize) -> Mesh {
    grid(Physics::Solid, ElementKind::Hex20, Elastic::new(1.0e7, 0.3), [n, n, n], &|p| p).unwrap()
}

fn grid2(kind: ElementKind, n: usize) -> Mesh {
    grid(Physics::PlaneStress { thickness: 1.0 }, kind, Elastic::new(1.0e7, 0.3), [n, n, 1], &|p| p).unwrap()
}

fn grid3(kind: ElementKind, n: usize) -> Mesh {
    grid(Physics::Solid, kind, Elastic::new(1.0e7, 0.3), [n, n, n], &|p| p).unwrap()
}

fn tri3_strip(n: usize) -> Mesh {
    grid(Physics::PlaneStress { thickness: 1.0 }, ElementKind::Tri3, Elastic::new(1.0e7, 0.3), [2 * n, n, 1], &|p| p).unwrap()
}

#[test]
#[ignore = "timing: run with --release --ignored --nocapture"]
fn learned_cost_against_the_deterministic_model_on_held_out_families() {
    // (family, dimension, meshes from small to large)
    let families: Vec<(&str, usize, Vec<Mesh>)> = vec![
        ("quad8", 2, [12, 20, 30, 45, 60, 80, 100].iter().map(|&n| quad(n)).collect()),
        ("quad4", 2, [20, 35, 55, 80, 110, 150, 200].iter().map(|&n| quad4(n)).collect()),
        ("tri3", 2, [15, 25, 40, 60, 90, 130, 180].iter().map(|&n| tri3_strip(n)).collect()),
        ("hole tri6", 2, [0.6, 0.45, 0.32, 0.22, 0.15, 0.1, 0.075].iter().map(|&h| hole(h)).collect()),
        ("hex8", 3, [5, 7, 9, 11, 14, 17, 21].iter().map(|&n| hex(n)).collect()),
        ("hex20", 3, [3, 4, 5, 6, 8, 10, 12].iter().map(|&n| hex20(n)).collect()),
        ("quad9", 2, [10, 18, 28, 40, 55, 70, 90].iter().map(|&n| grid2(ElementKind::Quad9, n)).collect()),
        ("tri6 grid", 2, [8, 14, 22, 34, 50, 70, 95].iter().map(|&n| grid2(ElementKind::Tri6, n)).collect()),
        ("hex27", 3, [2, 3, 4, 5, 6, 8, 10].iter().map(|&n| grid3(ElementKind::Hex27, n)).collect()),
        ("wedge6", 3, [5, 7, 9, 12, 15, 19, 24].iter().map(|&n| grid3(ElementKind::Wedge6, n)).collect()),
        ("wedge15", 3, [3, 4, 5, 7, 9, 11, 13].iter().map(|&n| grid3(ElementKind::Wedge15, n)).collect()),
    ];
    let mut data: Vec<Measured> = Vec::new();
    for (name, dim, meshes) in &families {
        for m in meshes {
            if let Some(t) = stage_times(m) {
                data.push((name, *dim, m.clone(), MeshCost::of(m).unwrap(), t));
            }
        }
    }
    let sample = |d: &Measured| Sample { cost: d.3, dim: d.1, assemble_s: d.4 .0, factor_s: d.4 .1, solve_s: d.4 .2 };
    let w = Workload::linear_static();
    let (mut learned_err, mut det_cross_err, mut det_own_err) = (Vec::new(), Vec::new(), Vec::new());
    let (mut covered, mut total, mut fallbacks) = (0usize, 0usize, 0usize);
    for (name, dim, _) in &families {
        let train: Vec<Sample> = data.iter().filter(|d| d.0 != *name).map(sample).collect();
        let Some(model) = LearnedCost::fit(&train, 1e-3) else {
            eprintln!("{name}: no fit");
            continue;
        };
        // Deterministic baselines: rates calibrated on the mid-size mesh of the held-out family itself, and of another family.
        let held: Vec<_> = data.iter().filter(|d| d.0 == *name).collect();
        let own = CostModel::calibrate(&held[held.len() / 2].2, 3).unwrap();
        let other = data.iter().filter(|d| d.0 != *name && d.1 == *dim).nth(3).unwrap();
        let cross = CostModel::calibrate(&other.2, 3).unwrap();
        for d in &held {
            let truth = d.4 .0 + d.4 .1 + d.4 .2;
            let p = model.predict(&d.3, *dim, &w, 0.0, &cross);
            let (le, ce, oe) = ((p.seconds / truth).ln().abs(), (cross.predict(&d.3, &w, 0.0) / truth).ln().abs(), (own.predict(&d.3, &w, 0.0) / truth).ln().abs());
            eprintln!("{name:<9} {:>7} dofs: truth {truth:.4} s | learned x{:.2} [{:.2}, {:.2}] {:?} | det cross-family x{:.2} | det own-family x{:.2}", d.3.dofs, p.seconds / truth, p.low / truth, p.high / truth, p.source, cross.predict(&d.3, &w, 0.0) / truth, own.predict(&d.3, &w, 0.0) / truth);
            if p.source == Source::Learned {
                learned_err.push(le);
                total += 1;
                if p.low <= truth && truth <= p.high {
                    covered += 1;
                }
            } else {
                fallbacks += 1;
            }
            det_cross_err.push(ce);
            det_own_err.push(oe);
        }
    }
    let quantile = |v: &mut Vec<f64>, q: f64| {
        v.sort_by(f64::total_cmp);
        v[((v.len() - 1) as f64 * q) as usize]
    };
    let (ml, mc, mo) = (quantile(&mut learned_err, 0.5), quantile(&mut det_cross_err, 0.5), quantile(&mut det_own_err, 0.5));
    let (pl, pc) = (quantile(&mut learned_err, 0.9), quantile(&mut det_cross_err, 0.9));
    let (xl, xc) = (quantile(&mut learned_err, 1.0), quantile(&mut det_cross_err, 1.0));
    eprintln!("median |ln(pred/truth)|: learned {ml:.3} (x{:.2}), deterministic cross-family {mc:.3} (x{:.2}), deterministic own-family {mo:.3} (x{:.2})", ml.exp(), mc.exp(), mo.exp());
    eprintln!("90th percentile: learned x{:.2}, deterministic x{:.2}; worst: learned x{:.2}, deterministic x{:.2}; interval coverage {covered}/{total}; fallbacks {fallbacks} of {}", pl.exp(), pc.exp(), xl.exp(), xc.exp(), total + fallbacks);
    // The qualification gate: held out by family, the learned model that answers is clearly better than the deterministic one,
    // its interval covers about as often as nominal, and the fallback is not the common case.
    assert!(ml < 0.5 * mc, "median error {ml} vs {mc}");
    assert!(covered as f64 >= 0.8 * total as f64, "coverage {covered}/{total}");
    assert!((fallbacks as f64) < 0.25 * (total + fallbacks) as f64, "fallbacks {fallbacks} of {}", total + fallbacks);
}
