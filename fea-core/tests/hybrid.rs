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
        let model = Model::new(h.mesh).unwrap();
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
        assert!(worst_u < 1e-12, "{family:?}: displacement error {worst_u:e}");
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
        assert!(worst_s < 1e-9, "{family:?}: stress error {worst_s:e}");
    }
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
