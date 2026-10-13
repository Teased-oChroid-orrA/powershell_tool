//! The equilibrated (H(div)) stress reconstruction: exact equilibrium, continuous tractions, boundary data, and the quality of
//! the estimator it feeds, on straight-sided triangle meshes.

#![allow(clippy::needless_range_loop)] // index-parallel numeric kernels read clearer as loops

use fea_core::adaptivity::equilibrated::{reconstruct, EquilibratedEstimator};
use fea_core::adaptivity::{ErrorEstimator, EstimatorContext, Reliability};
use fea_core::*;

const E: f64 = 10.0e6;
const NU: f64 = 0.3;

struct Case {
    model: Model,
    loads: Loads,
    bc: Dirichlet,
    u: Vec<f64>,
}

/// A 4 x 2 plate on structured triangles: clamped on the left (or symmetric about the bottom edge), loaded on the right edge,
/// with a body force.
fn plate(kind: ElementKind, n: usize, symmetric: bool, physics: Physics) -> Case {
    let mesh = fea_core::generate::grid(physics, kind, Elastic::new(E, NU), [4 * n, 2 * n, 1], &|p| [4.0 * p[0], 2.0 * p[1], 0.0]).unwrap();
    let model = Model::new(mesh).unwrap();
    let mut bc = model.dirichlet();
    if symmetric {
        for (nd, x) in model.mesh.nodes.iter().enumerate() {
            if x[0] < 1e-9 {
                bc.fix(nd, 0, 0.0);
            }
            if x[1] < 1e-9 {
                bc.fix(nd, 1, 0.0);
            }
        }
    } else {
        for &nd in model.mesh.node_set("u0").unwrap() {
            bc.fix(nd, 0, 0.0);
            bc.fix(nd, 1, 0.0);
        }
    }
    let mut loads = Loads { body: Some([0.0, -30.0, 0.0]), ..Loads::default() };
    loads.faces = model.mesh.surfaces["u1"].iter().map(|f| (f.clone(), SurfaceLoad::Traction([200.0, -50.0, 0.0]))).collect();
    loads.faces.extend(model.mesh.surfaces["v1"].iter().map(|f| (f.clone(), SurfaceLoad::Pressure(80.0))));
    let u = model.solve_static(&loads, &bc).unwrap().u;
    Case { model, loads, bc, u }
}

#[test]
fn the_reconstruction_is_exactly_equilibrated_with_continuous_tractions_and_the_applied_boundary_data() {
    for (kind, physics) in [
        (ElementKind::Tri3, Physics::PlaneStress { thickness: 1.0 }),
        (ElementKind::Tri6, Physics::PlaneStress { thickness: 1.0 }),
        (ElementKind::Tri6, Physics::PlaneStrain { thickness: 1.0 }),
    ] {
        for symmetric in [false, true] {
            let c = plate(kind, 3, symmetric, physics);
            let adj = c.model.mesh.adjacency();
            let ctx = EstimatorContext::new(&c.model, &c.loads, &c.bc, &c.u, &adj).unwrap();
            let rec = reconstruct(&ctx).unwrap();
            let r = &rec.report;
            eprintln!("{kind:?} {physics:?} symmetric={symmetric}: div {:.2e} jump {:.2e} traction {:.2e} defect {:.2e} skew {:.3e} (degree {}, {} patches)", r.max_div_residual, r.max_normal_jump, r.max_neumann_residual, r.max_compat_defect, r.skew_fraction, r.degree, r.patches);
            assert!(r.affine);
            assert!(r.max_div_residual < 1e-8, "{kind:?}: div residual {}", r.max_div_residual);
            assert!(r.max_normal_jump < 1e-8, "{kind:?}: normal jump {}", r.max_normal_jump);
            assert!(r.max_neumann_residual < 1e-8, "{kind:?}: traction residual {}", r.max_neumann_residual);
            assert!(r.max_compat_defect < 1e-8, "{kind:?}: compatibility defect {}", r.max_compat_defect);
            assert!(r.equilibrated());
        }
    }
}

mod lshape {
    use super::*;
    use fea_core::delaunay::MeshOptions;
    use fea_core::geometry::{Loop, Region};
    use fea_core::kernel::{geometry, Work};
    use fea_core::mesh2d::mesh_region;
    use fea_core::recover::complementary_energy;

    pub fn region() -> Region {
        let outer = Loop::polygon_named(&[[0.0, 0.0], [2.0, 0.0], [2.0, 1.0], [1.0, 1.0], [1.0, 2.0], [0.0, 2.0]], &["bottom", "right", "ledge", "inner", "right_top", "top_left"]).unwrap();
        // the segment between (0,2) and (0,0) closes the polygon: name it
        Region::new(outer, vec![], Elastic::new(E, NU)).unwrap()
    }

    pub fn solve(h: f64, kind: ElementKind) -> (Model, Loads, Dirichlet, Vec<f64>) {
        let mesh = mesh_region(&region(), Physics::PlaneStress { thickness: 1.0 }, kind, &|_| h, MeshOptions::default()).unwrap();
        let model = Model::new(mesh).unwrap();
        let mut bc = model.dirichlet();
        // clamp the whole left edge x = 0 (the closing segment of the polygon is named by the last name)
        for (nd, x) in model.mesh.nodes.iter().enumerate() {
            if x[0] < 1e-9 {
                bc.fix(nd, 0, 0.0);
                bc.fix(nd, 1, 0.0);
            }
        }
        // downward traction on the free end x = 2 (the right edge of the lower arm)
        let loads = Loads { faces: model.mesh.surfaces["right"].iter().map(|f| (f.clone(), SurfaceLoad::Traction([0.0, -100.0, 0.0]))).collect(), ..Loads::default() };
        let u = model.solve_static(&loads, &bc).unwrap().u;
        (model, loads, bc, u)
    }

    /// Energy-norm error of every coarse element against a fine reference solution.
    pub fn true_error(coarse: &Model, u: &[f64], fine: &Model, u_fine: &[f64]) -> Vec<f64> {
        let gauss = coarse.gauss_stresses(u, 0.0).unwrap();
        let loc = fine.locator();
        let mut work = Work::new();
        let mut out = Vec::new();
        for (bi, blk) in coarse.mesh.blocks.iter().enumerate() {
            let t = blk.kind.table();
            for e in 0..blk.n_elems() {
                let xyz: Vec<[f64; 3]> = blk.elem(e).iter().map(|&n| coarse.mesh.nodes[n]).collect();
                geometry(blk.kind, coarse.mesh.physics, &xyz, &mut work).unwrap();
                let mut sum = 0.0;
                for g in 0..t.ngp {
                    let (x, sh) = gauss[bi][e * t.ngp + g];
                    let sr = loc.stress_at(fine, u_fine, x, 0.0).expect("inside the fine mesh");
                    let d: [f64; 6] = std::array::from_fn(|c| sr[c] - sh[c]);
                    sum += work.wdet[g] * 2.0 * complementary_energy(&blk.material, &d);
                }
                out.push(sum.sqrt());
            }
        }
        out
    }
}

#[test]
fn the_equilibrated_estimator_tracks_the_true_energy_error_on_an_l_shape() {
    let (fine, _, _, u_fine) = lshape::solve(0.025, ElementKind::Tri6);
    let mut rows = Vec::new();
    for h in [0.4, 0.2] {
        let (model, loads, bc, u) = lshape::solve(h, ElementKind::Tri6);
        let adj = model.mesh.adjacency();
        let ctx = EstimatorContext::new(&model, &loads, &bc, &u, &adj).unwrap();
        let eq = EquilibratedEstimator.estimate(&ctx).unwrap();
        let zz = fea_core::adaptivity::ZzEstimator.estimate(&ctx).unwrap();
        let truth = lshape::true_error(&model, &u, &fine, &u_fine);
        let tot = truth.iter().map(|v| v * v).sum::<f64>().sqrt();
        eprintln!("h={h}: {} elements; true error {tot:.4e}; equilibrated {:.4e} (effectivity {:.2}, {:?}); ZZ {:.4e} (effectivity {:.2})", model.mesh.n_elems(), eq.total, eq.total / tot, eq.reliability, zz.total, zz.total / tot);
        // where is the true error: element carrying the largest share, and the rank the estimators give it
        let worst = truth.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).unwrap().0;
        let rank = |f: &[f64]| f.iter().filter(|&&v| v > f[worst]).count();
        eprintln!("   worst element {worst}: equilibrated rank {}, ZZ rank {}", rank(&eq.values), rank(&zz.values));
        assert!(eq.reliability.usable(), "{:?}", eq.reliability);
        rows.push((tot, eq.total, rank(&eq.values), model.mesh.n_elems()));
    }
    for &(tot, est, rank, n) in &rows {
        assert!(est / tot > 0.7 && est / tot < 3.0, "effectivity {}", est / tot);
        assert!(rank < n / 8, "the worst element ranks {rank} of {n}");
    }
    // the estimate decreases with the error under refinement
    assert!(rows[1].1 < rows[0].1);
}

#[test]
fn the_estimate_of_an_exact_multi_material_solution_vanishes_without_any_smoothing() {
    // two layers (E = 1e7 and 3e7) stretched along x: the finite-element stress is the exact one and already equilibrated, so
    // the reconstruction must reproduce it (an interface-blind recovery would blend the jump)
    let eps = 1e-3;
    let mut mesh = fea_core::generate::grid(Physics::PlaneStress { thickness: 1.0 }, ElementKind::Tri6, Elastic::new(E, NU), [8, 8, 1], &|p| [4.0 * p[0], 2.0 * p[1], 0.0]).unwrap();
    let blk = mesh.blocks.remove(0);
    let (mut soft, mut stiff) = (Vec::new(), Vec::new());
    for e in 0..blk.n_elems() {
        let c = blk.elem(e);
        let y = c.iter().map(|&n| mesh.nodes[n][1]).sum::<f64>() / c.len() as f64;
        if y < 1.0 { soft.extend_from_slice(c) } else { stiff.extend_from_slice(c) }
    }
    mesh.add_block(ElementKind::Tri6, soft, Elastic::new(E, NU), "soft").unwrap();
    mesh.add_block(ElementKind::Tri6, stiff, Elastic::new(3.0 * E, NU), "stiff").unwrap();
    let model = Model::new(mesh).unwrap();
    let mut bc = model.dirichlet();
    let anchor = model.mesh.nodes.iter().position(|x| x[0] < 1e-9 && x[1] < 1e-9).unwrap();
    for (n, x) in model.mesh.nodes.iter().enumerate() {
        if x[0] < 1e-9 {
            bc.fix(n, 0, 0.0);
        }
        if x[0] > 4.0 - 1e-9 {
            bc.fix(n, 0, eps * 4.0);
        }
    }
    bc.fix(anchor, 1, 0.0);
    let loads = Loads::default();
    let u = model.solve_static(&loads, &bc).unwrap().u;
    let adj = model.mesh.adjacency();
    let ctx = EstimatorContext::new(&model, &loads, &bc, &u, &adj).unwrap();
    let f = EquilibratedEstimator.estimate(&ctx).unwrap();
    assert!(f.reliability.usable(), "{:?}", f.reliability);
    let norm = (E * eps).powi(2).sqrt();
    assert!(f.total < 1e-8 * norm, "the estimate of an exact solution is {} (stress scale {norm})", f.total);
}

#[test]
fn unsupported_problems_are_reported_not_approximated() {
    // quadrilaterals
    let mesh = fea_core::generate::grid(Physics::PlaneStress { thickness: 1.0 }, ElementKind::Quad8, Elastic::new(E, NU), [3, 3, 1], &|p| p).unwrap();
    let model = Model::new(mesh).unwrap();
    let mut bc = model.dirichlet();
    for &nd in model.mesh.node_set("u0").unwrap() {
        bc.fix(nd, 0, 0.0);
        bc.fix(nd, 1, 0.0);
    }
    let loads = Loads { faces: model.mesh.surfaces["u1"].iter().map(|f| (f.clone(), SurfaceLoad::Traction([1.0, 0.0, 0.0]))).collect(), ..Loads::default() };
    let u = model.solve_static(&loads, &bc).unwrap().u;
    let adj = model.mesh.adjacency();
    let ctx = EstimatorContext::new(&model, &loads, &bc, &u, &adj).unwrap();
    assert!(matches!(EquilibratedEstimator.estimate(&ctx).unwrap().reliability, Reliability::Unsupported(_)));
    // a nodal point load on triangles
    let c = plate(ElementKind::Tri3, 2, false, Physics::PlaneStress { thickness: 1.0 });
    let mut loads = c.loads.clone();
    loads.nodal.push((5, [0.0, -10.0, 0.0]));
    let u = c.model.solve_static(&loads, &c.bc).unwrap().u;
    let adj = c.model.mesh.adjacency();
    let ctx = EstimatorContext::new(&c.model, &loads, &c.bc, &u, &adj).unwrap();
    let f = EquilibratedEstimator.estimate(&ctx).unwrap();
    assert!(matches!(f.reliability, Reliability::Unsupported(ref w) if w.contains("point")), "{:?}", f.reliability);
}

#[test]
fn curved_boundaries_are_reconstructed_but_flagged() {
    // a quarter annulus under internal pressure: the bore is a circle, the Tri6 edges on it are curved
    use fea_core::delaunay::MeshOptions;
    use fea_core::geometry::{Curve, Loop, Region, Segment};
    let seg = |curve, name: &str| Segment { curve, name: name.to_string() };
    let outer = Loop::new(vec![
        seg(Curve::line([1.0, 0.0], [2.0, 0.0]), "xaxis"),
        seg(Curve::arc([0.0, 0.0], 2.0, 0.0, std::f64::consts::FRAC_PI_2), "outer"),
        seg(Curve::line([0.0, 2.0], [0.0, 1.0]), "yaxis"),
        seg(Curve::arc([0.0, 0.0], 1.0, std::f64::consts::FRAC_PI_2, 0.0), "inner"),
    ])
    .unwrap();
    let region = Region::new(outer, vec![], Elastic::new(E, NU)).unwrap();
    let mesh = fea_core::mesh2d::mesh_region(&region, Physics::PlaneStrain { thickness: 1.0 }, ElementKind::Tri6, &|_| 0.25, MeshOptions::default()).unwrap();
    let model = Model::new(mesh).unwrap();
    let mut bc = model.dirichlet();
    for &n in model.mesh.node_set("xaxis").unwrap() {
        bc.fix(n, 1, 0.0);
    }
    for &n in model.mesh.node_set("yaxis").unwrap() {
        bc.fix(n, 0, 0.0);
    }
    let loads = Loads { faces: model.mesh.surfaces["inner"].iter().map(|f| (f.clone(), SurfaceLoad::Pressure(1000.0))).collect(), ..Loads::default() };
    let u = model.solve_static(&loads, &bc).unwrap().u;
    let adj = model.mesh.adjacency();
    let ctx = EstimatorContext::new(&model, &loads, &bc, &u, &adj).unwrap();
    let f = EquilibratedEstimator.estimate(&ctx).unwrap();
    assert!(matches!(f.reliability, Reliability::Degraded(_)), "{:?}", f.reliability);
    assert!(f.total.is_finite() && f.total > 0.0);
}

#[test]
fn the_reconstruction_is_bit_identical_on_one_thread_and_many() {
    let c = plate(ElementKind::Tri3, 8, false, Physics::PlaneStress { thickness: 1.0 });
    let adj = c.model.mesh.adjacency();
    let ctx = EstimatorContext::new(&c.model, &c.loads, &c.bc, &c.u, &adj).unwrap();
    let many = EquilibratedEstimator.estimate(&ctx).unwrap();
    let one = fea_core::sequentially(|| {
        let ctx = EstimatorContext::new(&c.model, &c.loads, &c.bc, &c.u, &adj).unwrap();
        EquilibratedEstimator.estimate(&ctx).unwrap()
    });
    assert_eq!(many.values, one.values);
}
