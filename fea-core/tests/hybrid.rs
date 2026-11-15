//! Hybrid structured/unstructured meshes solve like any other mesh: patch test on a mixed Quad/Tri mesh, a plate with a hole
//! against an unstructured reference and Peterson's stress concentration factor, mesh-quality and conformity gates.

#![allow(clippy::needless_range_loop)] // index-parallel numeric kernels read clearer as loops

use fea_core::delaunay::MeshOptions;
use fea_core::geometry::{Loop, Region};
use fea_core::meshing::hybrid::{mesh_hybrid_2d, mesh_unstructured_2d, ElementFamily, HybridOptions2d, Strategy};
use fea_core::meshing::partition::PartitionOptions2d;
use fea_core::mesh2d::mesh_region;
use fea_core::*;

const E: f64 = 10.0e6;
const NU: f64 = 0.3;
const SIGMA: f64 = 1000.0;
const PHYS: Physics = Physics::PlaneStress { thickness: 1.0 };

fn plate(hole: bool) -> Region {
    let holes = if hole { vec![Loop::circle([6.0, 2.0], 0.6, "hole").unwrap()] } else { vec![] };
    Region::new(Loop::rectangle(0.0, 0.0, 12.0, 4.0).unwrap(), holes, Elastic::new(E, NU)).unwrap()
}

/// Uniaxial tension: traction on the right edge, `u_x = 0` on the left edge, `u_y = 0` at the left-edge node nearest the
/// centre line. Returns (strain energy, hole-edge stress `sigma_xx` at the top of the hole, stress variation of the Gauss
/// points relative to sigma for a hole-free plate is checked separately).
fn solve_tension(mesh: Mesh) -> (f64, f64, Model, Solution) {
    let model = Model::new(mesh).unwrap();
    let mut bc = model.dirichlet();
    for &n in model.mesh.node_set("left").unwrap() {
        bc.fix(n, 0, 0.0);
    }
    let anchor = *model.mesh.node_set("left").unwrap().iter().min_by(|&&a, &&b| (model.mesh.nodes[a][1] - 2.0).abs().total_cmp(&(model.mesh.nodes[b][1] - 2.0).abs())).unwrap();
    bc.fix(anchor, 1, 0.0);
    let loads = Loads { faces: model.mesh.surfaces["right"].iter().map(|f| (f.clone(), SurfaceLoad::Traction([SIGMA, 0.0, 0.0]))).collect(), ..Loads::default() };
    let sol = model.solve_static(&loads, &bc).unwrap();
    let energy = model.strain_energy(&sol.u).unwrap();
    let nodal = model.nodal_stresses(&sol.u, 0.0).unwrap();
    // the node of the hole boundary nearest the top of the hole
    let top = *model.mesh.node_set("hole").unwrap().iter().min_by(|&&a, &&b| {
        let d = |n: usize| (model.mesh.nodes[n][0] - 6.0).hypot(model.mesh.nodes[n][1] - 2.6);
        d(a).total_cmp(&d(b))
    }).unwrap();
    (energy, nodal[top][0], model, sol)
}

fn options(family: ElementFamily) -> HybridOptions2d {
    HybridOptions2d::new(PHYS, family)
}

/// An affine displacement field on every boundary node: the interior must reproduce it exactly and the stress must be uniform.
fn assert_patch_test(mesh: Mesh, label: &str) {
    let model = Model::new(mesh).unwrap();
    // affine displacement field on every boundary node; the interior must reproduce it and the stress must be uniform
    let (a, b, c, d, e, f) = (1.0e-4, 2.0e-5, -3.0e-5, 4.0e-5, 5.0e-6, -7.0e-6);
    let field = |x: &[f64; 3]| [a * x[0] + b * x[1] + e, c * x[0] + d * x[1] + f];
    let mut bc = model.dirichlet();
    let mut on_boundary = vec![false; model.mesh.nodes.len()];
    for face in model.mesh.boundary_faces() {
        for &n in &face.nodes {
            on_boundary[n] = true;
        }
    }
    for n in 0..model.mesh.nodes.len() {
        if on_boundary[n] {
            let u = field(&model.mesh.nodes[n]);
            bc.fix(n, 0, u[0]);
            bc.fix(n, 1, u[1]);
        }
    }
    let sol = model.solve_static(&Loads::default(), &bc).unwrap();
    let mut worst_u = 0.0f64;
    for (n, x) in model.mesh.nodes.iter().enumerate() {
        let u = field(x);
        worst_u = worst_u.max((sol.u[2 * n] - u[0]).abs()).max((sol.u[2 * n + 1] - u[1]).abs());
    }
    assert!(worst_u < 1e-12, "{label}: displacement error {worst_u:e}");
    let lam = E * NU / (1.0 - NU * NU);
    let mu = E / (2.0 * (1.0 + NU));
    let (ex, ey, gxy) = (a, d, b + c);
    let want = [lam * (ex + ey) + 2.0 * mu * ex, lam * (ex + ey) + 2.0 * mu * ey, 0.0, mu * gxy];
    let gauss = model.gauss_stresses(&sol.u, 0.0).unwrap();
    let mut worst_s = 0.0f64;
    for blk in &gauss {
        for (_, s) in blk {
            for k in [0usize, 1, 3] {
                worst_s = worst_s.max((s[k] - want[k]).abs() / want[0].abs());
            }
        }
    }
    assert!(worst_s < 1e-9, "{label}: stress error {worst_s:e}");
}

#[test]
fn a_mixed_quad_tri_mesh_passes_the_patch_test_exactly() {
    // an unstructured block is forced into the middle of a hole-free plate: Quad8 | Tri6 | Quad8
    for family in [ElementFamily::Linear, ElementFamily::Quadratic] {
        let r = plate(false);
        let mut opt = options(family);
        opt.partition = PartitionOptions2d { extra_boxes: vec![([5.0, 1.0], [7.0, 3.0])], ..Default::default() };
        let h = mesh_hybrid_2d(&r, &|_| 0.5, &opt).unwrap();
        assert_eq!(h.report.strategy, Strategy::Hybrid);
        assert!(h.report.structured_elements > 0 && h.report.unstructured_elements > 0);
        assert_patch_test(h.mesh, &format!("{family:?}"));
    }
}

#[test]
fn a_hybrid_mesh_with_angled_edges_passes_the_patch_test_exactly() {
    // The legs of the trapezoid are meshed by the unstructured layer, the rest by Quad blocks: the interface between them is
    // conforming, so an affine field is still reproduced exactly (straight edges, so the elements are affine too).
    let outer = Loop::polygon(&[[0.0, 0.0], [12.0, 0.0], [10.0, 4.0], [2.0, 4.0]], "e").unwrap();
    let r = Region::new(outer, vec![], Elastic::new(E, NU)).unwrap();
    for family in [ElementFamily::Linear, ElementFamily::Quadratic] {
        let h = mesh_hybrid_2d(&r, &|_| 0.4, &options(family)).unwrap();
        assert_eq!(h.report.strategy, Strategy::Hybrid, "{family:?}");
        assert!(h.report.structured_elements > 0 && h.report.unstructured_elements > 0);
        assert_patch_test(h.mesh, &format!("trapezoid {family:?}"));
    }
}

/// Quarter annulus `2 <= r <= 5` under internal pressure, plane strain: the largest radial displacement error against Lame's
/// solution relative to the displacement at the bore.
fn lame_error(mesh: Mesh) -> f64 {
    const A: f64 = 2.0;
    const B: f64 = 5.0;
    let p = 1000.0;
    let model = Model::new(mesh).unwrap();
    let mut bc = model.dirichlet();
    for &n in model.mesh.node_set("xaxis").unwrap() {
        bc.fix(n, 1, 0.0);
    }
    for &n in model.mesh.node_set("yaxis").unwrap() {
        bc.fix(n, 0, 0.0);
    }
    let loads = Loads { faces: model.mesh.surfaces["inner"].iter().map(|f| (f.clone(), SurfaceLoad::Pressure(p))).collect(), ..Loads::default() };
    let sol = model.solve_static(&loads, &bc).unwrap();
    let ur_exact = |r: f64| (1.0 + NU) / E * (p * A * A / (B * B - A * A)) * ((1.0 - 2.0 * NU) * r + B * B / r);
    let mut worst = 0.0f64;
    for (n, x) in model.mesh.nodes.iter().enumerate() {
        let r = x[0].hypot(x[1]);
        let ur = (sol.u[n * 2] * x[0] + sol.u[n * 2 + 1] * x[1]) / r;
        worst = worst.max((ur - ur_exact(r)).abs() / ur_exact(A));
    }
    worst
}

#[test]
fn a_hybrid_mesh_of_a_curved_region_converges_to_lame_like_the_unstructured_one() {
    use fea_core::geometry::{Curve, Segment};
    use std::f64::consts::FRAC_PI_2;
    let seg = |curve, name: &str| Segment { curve, name: name.to_string() };
    let outer = Loop::new(vec![
        seg(Curve::line([2.0, 0.0], [5.0, 0.0]), "xaxis"),
        seg(Curve::arc([0.0, 0.0], 5.0, 0.0, FRAC_PI_2), "outer"),
        seg(Curve::line([0.0, 5.0], [0.0, 2.0]), "yaxis"),
        seg(Curve::arc([0.0, 0.0], 2.0, FRAC_PI_2, 0.0), "inner"),
    ])
    .unwrap();
    let r = Region::new(outer, vec![], Elastic::new(E, NU)).unwrap();
    let phys = Physics::PlaneStrain { thickness: 1.0 };
    let mut previous = f64::INFINITY;
    for h in [0.5, 0.25] {
        let mut opt = HybridOptions2d::new(phys, ElementFamily::Quadratic);
        opt.smooth = None;
        let hybrid = mesh_hybrid_2d(&r, &|_| h, &opt).unwrap();
        assert_eq!(hybrid.report.strategy, Strategy::Hybrid, "h = {h}");
        assert!(hybrid.report.conformity.is_conforming() && hybrid.report.structured_elements > 0);
        let plain = mesh_unstructured_2d(&r, &|_| h, &opt, "baseline").unwrap();
        let (e_h, e_u) = (lame_error(hybrid.mesh), lame_error(plain.mesh));
        eprintln!("h = {h}: Lame radial displacement error hybrid {e_h:.2e}, unstructured {e_u:.2e}");
        // the same order of accuracy as the plain quadratic mesh of the same size, and converging
        assert!(e_h < 4.0 * e_u + 1e-4, "h = {h}: hybrid {e_h:e} vs unstructured {e_u:e}");
        assert!(e_h < previous / 2.5, "h = {h}: no convergence ({previous:e} -> {e_h:e})");
        previous = e_h;
    }
    assert!(previous < 5e-4, "finest hybrid error {previous:e}");
}

#[test]
fn a_hybrid_plate_with_a_hole_agrees_with_the_unstructured_mesh() {
    let r = plate(true);
    let size = |_: [f64; 2]| 0.4;
    let reference = mesh_region(&r, PHYS, ElementKind::Tri6, &|_| 0.06, MeshOptions::default()).unwrap();
    let (e_ref, s_ref, ..) = solve_tension(reference);
    let hybrid = mesh_hybrid_2d(&r, &size, &options(ElementFamily::Quadratic)).unwrap();
    assert_eq!(hybrid.report.strategy, Strategy::Hybrid);
    let plain = mesh_unstructured_2d(&r, &size, &options(ElementFamily::Quadratic), "baseline").unwrap();
    let (e_h, s_h, ..) = solve_tension(hybrid.mesh.clone());
    let (e_u, s_u, ..) = solve_tension(plain.mesh.clone());
    eprintln!("energy: reference {e_ref:.6e}  hybrid {e_h:.6e} ({} nodes)  unstructured {e_u:.6e} ({} nodes)", hybrid.mesh.nodes.len(), plain.mesh.nodes.len());
    eprintln!("hole-top stress: reference {s_ref:.1}  hybrid {s_h:.1}  unstructured {s_u:.1}");
    let err = |x: f64, y: f64| ((x - y) / y).abs();
    assert!(err(e_h, e_ref) < 2.5e-3, "hybrid energy {e_h} vs {e_ref}");
    assert!(err(e_h, e_ref) < 2.0 * err(e_u, e_ref) + 5e-4, "hybrid energy error {} vs unstructured {}", err(e_h, e_ref), err(e_u, e_ref));
    assert!(err(s_h, s_ref) < 0.06, "hybrid peak stress {s_h} vs {s_ref}");
    // Peterson (Ktg = 3 - 3.14 d/W + 3.667 (d/W)^2 - 1.527 (d/W)^3 on the net section) for the finite-width plate
    let dw = 1.2 / 4.0;
    let peterson = (3.0 - (314.0 / 100.0) * dw + 3.667 * dw * dw - 1.527 * dw * dw * dw) * SIGMA / (1.0 - dw);
    eprintln!("Peterson: {peterson:.1}");
    assert!(err(s_ref, peterson) < 0.06, "reference {s_ref} vs Peterson {peterson}");
    assert!(err(s_h, peterson) < 0.10, "hybrid {s_h} vs Peterson {peterson}");
}

#[test]
fn the_hybrid_mesh_has_fewer_unknowns_than_the_unstructured_mesh_at_the_same_feature_resolution() {
    // fine near the hole, coarse far away: the structured blocks follow the coarse size, the unstructured cells the fine one
    let r = plate(true);
    let size = |p: [f64; 2]| {
        let d = ((p[0] - 6.0).hypot(p[1] - 2.0) - 0.6).max(0.0);
        (0.08 + 0.15 * d).min(0.5)
    };
    let h = mesh_hybrid_2d(&r, &size, &options(ElementFamily::Quadratic)).unwrap();
    assert_eq!(h.report.strategy, Strategy::Hybrid);
    let u = mesh_unstructured_2d(&r, &size, &options(ElementFamily::Quadratic), "baseline").unwrap();
    eprintln!("dofs: hybrid {} (max size jump {:.2}, min Jacobian {:.2})  unstructured {}", h.mesh.n_dofs(), h.report.quality.max_size_jump, h.report.quality.min_scaled_jacobian, u.mesh.n_dofs());
    let (e_h, s_h, ..) = solve_tension(h.mesh.clone());
    let (e_u, s_u, ..) = solve_tension(u.mesh.clone());
    eprintln!("energy hybrid {e_h:.6e} unstructured {e_u:.6e}; stress {s_h:.1} {s_u:.1}");
    assert!(((e_h - e_u) / e_u).abs() < 3e-3);
    assert!(h.report.conformity.is_conforming());
}

#[test]
fn every_hybrid_mesh_passes_the_conformity_and_quality_gates() {
    let l_shape = Region::new(Loop::polygon(&[[0.0, 0.0], [10.0, 0.0], [10.0, 3.0], [3.0, 3.0], [3.0, 10.0], [0.0, 10.0]], "edge").unwrap(), vec![Loop::circle([1.5, 1.5], 0.5, "hole").unwrap()], Elastic::new(E, NU)).unwrap();
    let two_holes = Region::new(Loop::rectangle(0.0, 0.0, 14.0, 4.0).unwrap(), vec![Loop::circle([4.0, 2.0], 0.5, "a").unwrap(), Loop::circle([10.0, 2.0], 0.7, "b").unwrap()], Elastic::new(E, NU)).unwrap();
    for (name, region) in [("plate", plate(true)), ("l-shape", l_shape), ("two holes", two_holes)] {
        for family in [ElementFamily::Linear, ElementFamily::Quadratic] {
            let h = mesh_hybrid_2d(&region, &|_| 0.4, &options(family)).unwrap();
            let c = &h.report.conformity;
            assert!(c.is_conforming() && c.components == 1 && c.unused_nodes == 0, "{name} {family:?}: {}", c.summary());
            assert_eq!(h.report.quality.inverted, 0, "{name} {family:?}");
            assert!(h.report.quality.min_scaled_jacobian > 0.15, "{name} {family:?}: {}", h.report.quality.min_scaled_jacobian);
            assert!(matches!(h.report.strategy, Strategy::Hybrid), "{name} {family:?}: {:?}", h.report.strategy);
            // the mesh solves (constrained enough): no rigid-body mode, finite displacements
            let model = Model::new(h.mesh.clone()).unwrap();
            let mut bc = model.dirichlet();
            for n in 0..model.mesh.nodes.len() {
                if model.mesh.nodes[n][0] < 1e-9 {
                    bc.fix(n, 0, 0.0);
                    bc.fix(n, 1, 0.0);
                }
            }
            let right: Vec<Vec<usize>> = model.mesh.boundary_faces().iter().filter(|f| f.nodes.iter().take(2).all(|&n| (model.mesh.nodes[n][0] - model.mesh.nodes.iter().map(|x| x[0]).fold(0.0, f64::max)).abs() < 1e-9)).map(|f| f.nodes.clone()).collect();
            let loads = Loads { faces: right.into_iter().map(|f| (f, SurfaceLoad::Traction([1.0, 0.0, 0.0]))).collect(), ..Loads::default() };
            let sol = model.solve_static(&loads, &bc).unwrap();
            assert!(sol.u.iter().all(|v| v.is_finite()), "{name} {family:?}");
        }
    }
}

/// Where the hybrid mesh is meant to pay: a long bar with one local feature. Same feature resolution, same global target
/// size; compares unknowns, factor size, solve time and accuracy against a fine reference. (`--ignored --nocapture`.)
#[test]
#[ignore]
fn hybrid_benefit_on_a_long_bar_with_a_hole() {
    use std::time::Instant;
    let bar = Region::new(Loop::rectangle(0.0, 0.0, 40.0, 4.0).unwrap(), vec![Loop::circle([20.0, 2.0], 0.6, "hole").unwrap()], Elastic::new(E, NU)).unwrap();
    let size = |p: [f64; 2]| {
        let d = ((p[0] - 20.0).hypot(p[1] - 2.0) - 0.6).max(0.0);
        (0.1 + 0.2 * d).min(0.6)
    };
    let tension = |mesh: Mesh| {
        let t = Instant::now();
        let model = Model::new(mesh).unwrap();
        let mut bc = model.dirichlet();
        for &n in model.mesh.node_set("left").unwrap() {
            bc.fix(n, 0, 0.0);
        }
        let anchor = *model.mesh.node_set("left").unwrap().iter().min_by(|&&a, &&b| (model.mesh.nodes[a][1] - 2.0).abs().total_cmp(&(model.mesh.nodes[b][1] - 2.0).abs())).unwrap();
        bc.fix(anchor, 1, 0.0);
        let loads = Loads { faces: model.mesh.surfaces["right"].iter().map(|f| (f.clone(), SurfaceLoad::Traction([SIGMA, 0.0, 0.0]))).collect(), ..Loads::default() };
        let sol = model.solve_static(&loads, &bc).unwrap();
        let wall = t.elapsed().as_secs_f64();
        let nodal = model.nodal_stresses(&sol.u, 0.0).unwrap();
        let top = *model.mesh.node_set("hole").unwrap().iter().min_by(|&&a, &&b| {
            let d = |n: usize| (model.mesh.nodes[n][0] - 20.0).hypot(model.mesh.nodes[n][1] - 2.6);
            d(a).total_cmp(&d(b))
        }).unwrap();
        (model.strain_energy(&sol.u).unwrap(), nodal[top][0], model.mesh.n_dofs(), wall)
    };
    let reference = tension(mesh_region(&bar, PHYS, ElementKind::Tri6, &|p| size(p) * 0.5, MeshOptions::default()).unwrap());
    eprintln!("reference: energy {:.6e} stress {:.1} dofs {}", reference.0, reference.1, reference.2);
    for (family, scale, coarsen, aspect) in [(ElementFamily::Quadratic, 1.2, 1.0, 1.0), (ElementFamily::Quadratic, 1.2, 1.0, 4.0), (ElementFamily::Quadratic, 1.2, 2.0, 4.0), (ElementFamily::Quadratic, 1.2, 2.0, 8.0)] {
        let mut o = options(family);
        o.structured_scale = scale;
        o.structured_aspect = [aspect, 1.0];
        eprintln!("aspect {aspect}");
        let size = |p: [f64; 2]| size(p) * coarsen;
        let t = Instant::now();
        let h = mesh_hybrid_2d(&bar, &size, &o).unwrap();
        let t_h = t.elapsed().as_secs_f64();
        let t = Instant::now();
        let u = mesh_unstructured_2d(&bar, &size, &options(family), "baseline").unwrap();
        let t_u = t.elapsed().as_secs_f64();
        eprintln!("size x{coarsen}: mesh generation hybrid {t_h:.3}s unstructured {t_u:.3}s");
        let rh = tension(h.mesh.clone());
        let ru = tension(u.mesh.clone());
        eprintln!("scale {scale}:");
        let e = |x: (f64, f64, usize, f64)| ((x.0 - reference.0) / reference.0).abs();
        let s = |x: (f64, f64, usize, f64)| ((x.1 - reference.1) / reference.1).abs();
        eprintln!("{family:?} hybrid:       dofs {:>6} energy err {:.2e} stress err {:.2e} wall {:.3}s  structured elements {:.0} %", rh.2, e(rh), s(rh), rh.3, 100.0 * h.report.structured_elements as f64 / h.mesh.n_elems() as f64);
        eprintln!("{family:?} unstructured: dofs {:>6} energy err {:.2e} stress err {:.2e} wall {:.3}s", ru.2, e(ru), s(ru), ru.3);
    }
}

#[test]
fn smoothing_improves_the_unstructured_part_and_leaves_the_structured_part_and_boundary_alone() {
    use fea_core::meshing::smooth::{smooth, SmoothOptions};
    let r = plate(true);
    let h = mesh_hybrid_2d(&r, &|_| 0.35, &options(ElementFamily::Quadratic)).unwrap();
    let mut mesh = h.mesh.clone();
    let rep = smooth(&mut mesh, &SmoothOptions::default());
    eprintln!("smoothing: {} movable, {} moves; min J {:.3} -> {:.3}; mean J {:.4} -> {:.4}", rep.movable_nodes, rep.moves, rep.min_scaled_jacobian_before, rep.min_scaled_jacobian_after, rep.mean_scaled_jacobian_before, rep.mean_scaled_jacobian_after);
    assert!(rep.moves > 0);
    assert!(rep.min_scaled_jacobian_after >= rep.min_scaled_jacobian_before - 1e-12);
    assert!(rep.mean_scaled_jacobian_after > rep.mean_scaled_jacobian_before);
    assert!(mesh.conformity().is_conforming());
    // structured elements did not move
    let mut g = 0;
    for blk in &h.mesh.blocks {
        for e in 0..blk.n_elems() {
            if h.labels.0[g] == fea_core::meshing::RegionKind::Structured {
                for &n in blk.elem(e) {
                    assert_eq!(h.mesh.nodes[n], mesh.nodes[n]);
                }
            }
            g += 1;
        }
    }
    // smoothing does not change the answer beyond the discretisation: same energy within 0.1 %
    let (e0, ..) = solve_tension(h.mesh.clone());
    let (e1, ..) = solve_tension(mesh);
    assert!(((e1 - e0) / e0).abs() < 1e-3, "{e0} {e1}");
}

#[test]
fn the_size_predictor_refines_the_hole_and_reduces_the_energy_error_at_the_same_base_size() {
    use fea_core::meshing::predictor::{GeometryRules, SizePredictor2d};
    let r = plate(true);
    // judge: a fine reference mesh. The strain energy is the robust global measure; one nodal stress is noisy at this size.
    let (e_ref, s_ref, ..) = solve_tension(mesh_region(&r, PHYS, ElementKind::Tri6, &|_| 0.04, MeshOptions::default()).unwrap());
    let mut p = SizePredictor2d::new(1.2, 0.05, 0.35).unwrap();
    p.add_geometry(&r, GeometryRules::default());
    let predicted = mesh_unstructured_2d(&r, &p.size_fn(), &options(ElementFamily::Quadratic), "predictor").unwrap();
    let uniform = mesh_unstructured_2d(&r, &|_| 1.2, &options(ElementFamily::Quadratic), "uniform").unwrap();
    let (e_p, s_p, ..) = solve_tension(predicted.mesh.clone());
    let (e_u, s_u, ..) = solve_tension(uniform.mesh.clone());
    let err = |x: f64, y: f64| ((x - y) / y).abs();
    eprintln!("energy error: predictor {:.2e} ({} nodes)  uniform {:.2e} ({} nodes); hole-top stress {s_p:.1} / {s_u:.1} vs {s_ref:.1}", err(e_p, e_ref), predicted.mesh.nodes.len(), err(e_u, e_ref), uniform.mesh.nodes.len());
    assert!(err(e_p, e_ref) < err(e_u, e_ref), "predictor energy error {} not below uniform {}", err(e_p, e_ref), err(e_u, e_ref));
    assert!(err(s_p, s_ref) < 0.03);
    assert!(predicted.mesh.quality().min_scaled_jacobian > 0.2);
}

#[test]
fn a_bimaterial_bar_welds_conformingly_and_reproduces_the_series_spring_elongation() {
    use fea_core::meshing::multimaterial::{mesh_multi_material_2d, MaterialRegion2d};
    // nu = 0 in both materials: uniform axial stress is then the exact solution and the interface carries no mismatch
    let (e1, e2) = (E, 4.0 * E);
    let rect = |x0: f64, x1: f64, e: f64| Region::new(Loop::rectangle(x0, 0.0, x1, 4.0).unwrap(), vec![], Elastic::new(e, 0.0)).unwrap();
    let parts = vec![MaterialRegion2d { name: "soft".into(), region: rect(0.0, 9.0, e1) }, MaterialRegion2d { name: "stiff".into(), region: rect(9.0, 20.0, e2) }];
    for family in [ElementFamily::Linear, ElementFamily::Quadratic] {
        let mut opt = options(family);
        opt.partition = PartitionOptions2d { extra_boxes: vec![([2.0, 1.0], [4.0, 3.0])], ..Default::default() };
        let m = mesh_multi_material_2d(&parts, &|p| 0.5 + 0.05 * p[0], &opt).unwrap();
        assert!(m.conformity.is_conforming() && m.conformity.components == 1, "{family:?}: {}", m.conformity.summary());
        assert!(m.interfaces.len() == 1 && m.interfaces[0].2.len() >= 3, "{:?}", m.interfaces.iter().map(|i| (&i.0, &i.1, i.2.len())).collect::<Vec<_>>());
        let blocks: Vec<&str> = m.mesh.blocks.iter().map(|b| b.name.as_str()).collect();
        assert!(blocks.iter().any(|b| b.starts_with("soft/")) && blocks.iter().any(|b| b.starts_with("stiff/")), "{blocks:?}");
        eprintln!("{family:?}: strategy {:?}, {} nodes, {} elements", m.strategy, m.mesh.nodes.len(), m.mesh.n_elems());
        assert!(m.labels.count(fea_core::meshing::RegionKind::Structured) > 0, "expected structured blocks to survive next to the interface");
        let model = Model::new(m.mesh).unwrap();
        let mut bc = model.dirichlet();
        for &n in model.mesh.node_set("left").unwrap() {
            bc.fix(n, 0, 0.0);
        }
        bc.fix(model.mesh.node_set("left").unwrap()[0], 1, 0.0);
        let loads = Loads { faces: model.mesh.surfaces["right"].iter().map(|f| (f.clone(), SurfaceLoad::Traction([SIGMA, 0.0, 0.0]))).collect(), ..Loads::default() };
        let sol = model.solve_static(&loads, &bc).unwrap();
        let want = SIGMA * (9.0 / e1 + 11.0 / e2);
        let tip = model.mesh.node_set("right").unwrap().iter().map(|&n| sol.u[2 * n]).fold(f64::INFINITY, f64::min);
        assert!((tip - want).abs() < 1e-9 * want, "{family:?}: tip {tip:e} vs {want:e}");
    }
}

#[test]
fn bonded_regions_weld_even_when_one_side_split_the_shared_boundary_further() {
    use fea_core::meshing::multimaterial::{mesh_multi_material_2d, mesh_multi_material_unstructured_2d, MaterialRegion2d};
    // Two 6 x 2 bars at size 0.4: the unstructured mesher splits the shared edge where it is encroached, which depends on the
    // interior, so the two sides first disagree (3 hanging nodes); cutting the edge at the nodes of both sides makes them match.
    let part = |name: &str, x0: f64, e: f64| MaterialRegion2d { name: name.into(), region: Region::new(Loop::rectangle(x0, 0.0, x0 + 6.0, 2.0).unwrap(), vec![], Elastic::new(e, NU)).unwrap() };
    for family in [ElementFamily::Linear, ElementFamily::Quadratic] {
        let parts = [part("a", 0.0, 30e6), part("b", 6.0, 10e6)];
        let opt = options(family);
        for (label, m) in [("unstructured", mesh_multi_material_unstructured_2d(&parts, &|_| 0.4, &opt)), ("hybrid", mesh_multi_material_2d(&parts, &|_| 0.4, &opt))] {
            let m = m.unwrap_or_else(|e| panic!("{family:?} {label}: {e}"));
            assert!(m.conformity.is_conforming(), "{family:?} {label}: {}", m.conformity.summary());
            assert_eq!(m.conformity.components, 1);
            assert!(!m.interfaces.is_empty() && m.interfaces[0].2.len() >= 2, "{family:?} {label}: {:?}", m.interfaces.iter().map(|i| i.2.len()).collect::<Vec<_>>());
            assert_eq!(m.quality.inverted, 0);
        }
    }
}
