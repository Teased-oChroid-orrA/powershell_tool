//! Goal-oriented (dual-weighted) error estimates against closed-form quantities of the Lame problem: a thick cylinder under
//! internal pressure (plane strain), quarter model with symmetry supports.

#![allow(clippy::needless_range_loop)] // index-parallel numeric kernels read clearer as loops

use fea_core::adaptivity::dwr::DwrEstimator;
use fea_core::adaptivity::{EstimatorContext, LinearFunctional, Reliability};
use fea_core::delaunay::MeshOptions;
use fea_core::geometry::{Curve, Loop, Region, Segment};
use fea_core::kernel::{geometry, Work};
use fea_core::mesh2d::mesh_region;
use fea_core::*;
use std::f64::consts::{FRAC_PI_2, PI};

const E: f64 = 10.0e6;
const NU: f64 = 0.3;
const A: f64 = 1.0;
const B: f64 = 2.0;
const P: f64 = 1000.0;

fn quarter_annulus() -> Region {
    let seg = |curve, name: &str| Segment { curve, name: name.to_string() };
    let outer = Loop::new(vec![
        seg(Curve::line([A, 0.0], [B, 0.0]), "xaxis"),
        seg(Curve::arc([0.0, 0.0], B, 0.0, FRAC_PI_2), "outer"),
        seg(Curve::line([0.0, B], [0.0, A]), "yaxis"),
        seg(Curve::arc([0.0, 0.0], A, FRAC_PI_2, 0.0), "inner"),
    ])
    .unwrap();
    Region::new(outer, vec![], Elastic::new(E, NU)).unwrap()
}

fn lame_ur(r: f64) -> f64 {
    let k = P * A * A / (B * B - A * A);
    (1.0 + NU) / E * k * ((1.0 - 2.0 * NU) * r + B * B / r)
}

/// Exact Cartesian stress `[xx, yy, zz, xy, yz, zx]` of the plane-strain Lame problem at `x`.
fn lame_stress(x: &[f64; 3]) -> [f64; 6] {
    let r = x[0].hypot(x[1]);
    let c = P * A * A / (B * B - A * A);
    let (sr, st) = (c * (1.0 - B * B / (r * r)), c * (1.0 + B * B / (r * r)));
    let (cs, sn) = (x[0] / r, x[1] / r);
    let szz = NU * (sr + st);
    [sr * cs * cs + st * sn * sn, sr * sn * sn + st * cs * cs, szz, (sr - st) * cs * sn, 0.0, 0.0]
}

struct Case {
    model: Model,
    loads: Loads,
    bc: Dirichlet,
    sol: Solution,
}

fn solve(kind: ElementKind, h: f64) -> Case {
    let mesh = mesh_region(&quarter_annulus(), Physics::PlaneStrain { thickness: 1.0 }, kind, &|_| h, MeshOptions::default()).unwrap();
    let model = Model::new(mesh).unwrap();
    let mut bc = model.dirichlet();
    for &n in model.mesh.node_set("xaxis").unwrap() {
        bc.fix(n, 1, 0.0);
    }
    for &n in model.mesh.node_set("yaxis").unwrap() {
        bc.fix(n, 0, 0.0);
    }
    let loads = Loads { faces: model.mesh.surfaces["inner"].iter().map(|f| (f.clone(), SurfaceLoad::Pressure(P))).collect(), ..Loads::default() };
    let sol = model.solve_static(&loads, &bc).unwrap();
    Case { model, loads, bc, sol }
}

/// `sum wdet sigma_exact[comp] / sum wdet` over the elements whose centroid satisfies `inside`.
fn exact_average(model: &Model, comp: usize, inside: &dyn Fn(&[f64; 3]) -> bool) -> f64 {
    let mut work = Work::new();
    let (mut num, mut den) = (0.0, 0.0);
    for blk in &model.mesh.blocks {
        let t = blk.kind.table();
        for e in 0..blk.n_elems() {
            let xyz: Vec<[f64; 3]> = blk.elem(e).iter().map(|&n| model.mesh.nodes[n]).collect();
            let c: [f64; 3] = std::array::from_fn(|k| xyz.iter().map(|x| x[k]).sum::<f64>() / xyz.len() as f64);
            if !inside(&c) {
                continue;
            }
            geometry(blk.kind, model.mesh.physics, &xyz, &mut work).unwrap();
            for g in 0..t.ngp {
                let n = t.n_at(g);
                let pos: [f64; 3] = std::array::from_fn(|k| (0..xyz.len()).map(|a| n[a] * xyz[a][k]).sum());
                num += work.wdet[g] * lame_stress(&pos)[comp];
                den += work.wdet[g];
            }
        }
    }
    num / den
}

#[test]
fn compliance_error_estimate_has_effectivity_near_one_and_the_correction_improves_the_value() {
    let exact = P * lame_ur(A) * (PI * A / 2.0); // f . u = p u_r(a) times the arc length of the quarter bore
    for kind in [ElementKind::Tri6, ElementKind::Quad8] {
        for h in [0.5, 0.25, 0.125] {
            let c = solve(kind, h);
            let adj = c.model.mesh.adjacency();
            let ctx = EstimatorContext::new(&c.model, &c.loads, &c.bc, &c.sol.u, &adj).unwrap();
            let goal = LinearFunctional::compliance(&c.model, &c.loads).unwrap();
            let r = &DwrEstimator.estimate_goals(&ctx, &[&goal]).unwrap()[0];
            let (err, est) = (exact - r.value, r.estimated_error);
            let eff = est / err;
            eprintln!("{kind:?} h={h}: J_h {:.6e} exact {exact:.6e} true err {err:.3e} estimate {est:.3e} effectivity {eff:.3}", r.value);
            assert_eq!(r.field.reliability, Reliability::Reliable);
            assert!((r.field.sum() - est).abs() < 1e-12 * est.abs().max(1e-300));
            // below ~1e-5 relative the closed form itself is limited by the curved boundary (quadratic edges on a circle), so
            // the finest level is informational; the coarse levels are verified
            if err.abs() > 2e-5 * exact {
                // the first mesh has a handful of elements per side: the asymptotic effectivity of the recovery does not apply yet
                let (lo, hi) = if h >= 0.5 { (0.3, 4.0) } else { (0.5, 2.0) };
                assert!(eff > lo && eff < hi, "{kind:?} h={h}: effectivity {eff}");
                if h <= 0.25 {
                    assert!((exact - r.corrected).abs() < err.abs(), "{kind:?} h={h}: the corrected value is not closer");
                }
            }
            // the estimate is positive: compliance is bounded below by the discrete one (J(u) >= J(u_h))
            assert!(est > 0.0);
        }
    }
}

#[test]
fn the_regional_stress_average_estimate_tracks_the_true_error() {
    for kind in [ElementKind::Tri6, ElementKind::Quad8] {
        let mut rows = Vec::new();
        for h in [0.5, 0.25, 0.125] {
            let c = solve(kind, h);
            let adj = c.model.mesh.adjacency();
            let ctx = EstimatorContext::new(&c.model, &c.loads, &c.bc, &c.sol.u, &adj).unwrap();
            let inside = |x: &[f64; 3]| x[0].hypot(x[1]) < 1.6;
            let goal = LinearFunctional::average_stress(&c.model, 0, &inside).unwrap();
            let r = &DwrEstimator.estimate_goals(&ctx, &[&goal]).unwrap()[0];
            let exact = exact_average(&c.model, 0, &inside);
            let (err, est) = (exact - r.value, r.estimated_error);
            eprintln!("{kind:?} h={h}: J_h {:.4} exact {exact:.4} true err {err:.3e} estimate {est:.3e} effectivity {:.3}", r.value, est / err);
            rows.push((err, est, r.corrected, exact));
            assert!(matches!(r.field.reliability, Reliability::Reliable));
        }
        // coarse and medium meshes: the estimate follows the true error to within a factor of two and the correction helps
        // (at the finest level the true error is 1e-6 of the value, below what the curved-boundary closed form resolves)
        for (k, &(err, est, corrected, exact)) in rows[..2].iter().enumerate() {
            assert!(est / err > 0.4 && est / err < 2.5, "{kind:?}: effectivity {}", est / err);
            if k == 1 {
                assert!((exact - corrected).abs() < err.abs(), "{kind:?}: correction did not improve the value");
            }
        }
        // the estimate decreases under refinement, like the error
        assert!(rows[2].1.abs() < rows[0].1.abs() / 5.0, "{kind:?}: estimate did not fall: {rows:?}");
    }
}

#[test]
fn the_reaction_estimate_of_an_equilibrium_determined_quantity_stays_small() {
    // The sum of all vertical support reactions along the x-axis is fixed by equilibrium (-int sigma_theta dr, here exactly
    // the net pressure load): the discrete value has no error, so the estimate must be small against the value itself.
    let c0 = P * A * A / (B * B - A * A);
    let exact = -c0 * ((B - A) + B * B * (1.0 / A - 1.0 / B));
    for kind in [ElementKind::Tri6, ElementKind::Quad8] {
        let c = solve(kind, 0.125);
        let adj = c.model.mesh.adjacency();
        let ctx = EstimatorContext::new(&c.model, &c.loads, &c.bc, &c.sol.u, &adj).unwrap();
        let nodes = c.model.mesh.node_set("xaxis").unwrap().to_vec();
        let goal = LinearFunctional::reaction(&c.model, &c.loads, &nodes, 1).unwrap();
        let direct: f64 = nodes.iter().map(|&n| c.sol.reactions[2 * n + 1]).sum();
        let r = &DwrEstimator.estimate_goals(&ctx, &[&goal]).unwrap()[0];
        assert!((r.value - direct).abs() < 1e-7 * direct.abs(), "functional {} vs solver {direct}", r.value);
        eprintln!("{kind:?} reaction: J_h {:.6} exact {exact:.6} estimate {:.3e}", r.value, r.estimated_error);
        assert!((r.value - exact).abs() < 1e-6 * exact.abs());
        assert!(r.estimated_error.abs() < 1e-3 * exact.abs(), "estimate {} against a value of {exact}", r.estimated_error);
    }
}

#[test]
fn several_goals_share_one_factorisation_and_a_point_goal_is_flagged() {
    let c = solve(ElementKind::Tri6, 0.25);
    let adj = c.model.mesh.adjacency();
    let ctx = EstimatorContext::new(&c.model, &c.loads, &c.bc, &c.sol.u, &adj).unwrap();
    let comp = LinearFunctional::compliance(&c.model, &c.loads).unwrap();
    let node = *c.model.mesh.node_set("inner").unwrap().first().unwrap();
    let point = LinearFunctional::nodal_displacement(&c.model, node, 0).unwrap();
    let rs = DwrEstimator.estimate_goals(&ctx, &[&comp, &point]).unwrap();
    assert_eq!(rs.len(), 2);
    assert_eq!(rs[0].field.reliability, Reliability::Reliable);
    assert!(matches!(rs[1].field.reliability, Reliability::Degraded(_)), "{:?}", rs[1].field.reliability);
    assert!(rs[1].field.usable_for_ranking());
}

#[test]
fn the_estimate_is_linear_in_the_goal() {
    let c = solve(ElementKind::Tri6, 0.25);
    let adj = c.model.mesh.adjacency();
    let ctx = EstimatorContext::new(&c.model, &c.loads, &c.bc, &c.sol.u, &adj).unwrap();
    let g1 = LinearFunctional::average_stress(&c.model, 0, &|x| x[0].hypot(x[1]) < 1.6).unwrap();
    let g2 = LinearFunctional::average_stress(&c.model, 1, &|x| x[0].hypot(x[1]) > 1.4).unwrap();
    let sum = LinearFunctional { name: "sum".into(), gradient: g1.gradient.iter().zip(&g2.gradient).map(|(a, b)| 2.0 * a - 3.0 * b).collect(), offset: 0.0, regularised: true };
    let r = DwrEstimator.estimate_goals(&ctx, &[&g1, &g2, &sum]).unwrap();
    let want = 2.0 * r[0].estimated_error - 3.0 * r[1].estimated_error;
    assert!((r[2].estimated_error - want).abs() < 1e-7 * want.abs().max(1e-12), "{} vs {want}", r[2].estimated_error);
    for (a, b) in r[2].field.values.iter().zip(r[0].field.values.iter().zip(&r[1].field.values)) {
        assert!((a - (2.0 * b.0 - 3.0 * b.1)).abs() < 1e-6 * a.abs().max(1e-6));
    }
}

#[test]
fn the_dual_weighted_field_is_bit_identical_on_one_thread_and_many() {
    // 60 x 60 Quad8 elements: above the parallel threshold of the element loop
    let mesh = fea_core::generate::grid(Physics::PlaneStress { thickness: 1.0 }, ElementKind::Quad8, Elastic::new(E, NU), [60, 60, 1], &|p| [p[0], p[1] * 1.0, 0.0]).unwrap();
    let model = Model::new(mesh).unwrap();
    let mut bc = model.dirichlet();
    for &n in model.mesh.node_set("u0").unwrap() {
        bc.fix(n, 0, 0.0);
        bc.fix(n, 1, 0.0);
    }
    let loads = Loads { faces: model.mesh.surfaces["u1"].iter().map(|f| (f.clone(), SurfaceLoad::Traction([0.0, -10.0, 0.0]))).collect(), ..Loads::default() };
    let u = model.solve_static(&loads, &bc).unwrap().u;
    let adj = model.mesh.adjacency();
    let ctx = EstimatorContext::new(&model, &loads, &bc, &u, &adj).unwrap();
    let goal = LinearFunctional::average_stress(&model, 0, &|x| x[0] < 0.5).unwrap();
    let many = DwrEstimator.estimate_goals(&ctx, &[&goal]).unwrap().remove(0);
    let one = fea_core::sequentially(|| {
        let ctx = EstimatorContext::new(&model, &loads, &bc, &u, &adj).unwrap();
        DwrEstimator.estimate_goals(&ctx, &[&goal]).unwrap().remove(0)
    });
    assert_eq!(many.field.values, one.field.values);
    assert_eq!(many.estimated_error.to_bits(), one.estimated_error.to_bits());
}
