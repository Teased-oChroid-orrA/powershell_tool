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
