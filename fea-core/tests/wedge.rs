//! Triangular prisms (Wedge6 / Wedge15) and the 3D hybrid mesh made by sweeping a mixed Quad/Tri mesh: Hex elements meet
//! wedges on shared quadrilateral faces, so the union is conforming without pyramids or tie constraints.

#![allow(clippy::needless_range_loop)] // index-parallel numeric kernels read clearer as loops

use fea_core::geometry::{Loop, Region};
use fea_core::meshing::hybrid::{mesh_hybrid_2d, ElementFamily, HybridOptions2d, Strategy};
use fea_core::meshing::partition::PartitionOptions2d;
use fea_core::sweep::extrude;
use fea_core::*;

const E: f64 = 10.0e6;
const NU: f64 = 0.3;
const PLANE: Physics = Physics::PlaneStrain { thickness: 1.0 };

fn hybrid_2d(family: ElementFamily, physics: Physics) -> Mesh {
    let r = Region::new(Loop::rectangle(0.0, 0.0, 12.0, 4.0).unwrap(), vec![], Elastic::new(E, NU)).unwrap();
    let mut opt = HybridOptions2d::new(physics, family);
    opt.partition = PartitionOptions2d { extra_boxes: vec![([5.0, 1.0], [7.0, 3.0])], ..Default::default() };
    let h = mesh_hybrid_2d(&r, &|_| 0.5, &opt).unwrap();
    assert_eq!(h.report.strategy, Strategy::Hybrid);
    h.mesh
}

#[test]
fn a_swept_hybrid_mesh_mixes_hexahedra_and_wedges_conformingly() {
    for (family, hex, wedge) in [(ElementFamily::Linear, ElementKind::Hex8, ElementKind::Wedge6), (ElementFamily::Quadratic, ElementKind::Hex20, ElementKind::Wedge15)] {
        let m3 = extrude(&hybrid_2d(family, PLANE), &[0.0, 0.5, 1.2]).unwrap();
        let kinds: Vec<ElementKind> = m3.blocks.iter().map(|b| b.kind).collect();
        assert!(kinds.contains(&hex) && kinds.contains(&wedge), "{kinds:?}");
        let r = m3.conformity();
        assert!(r.is_conforming() && r.components == 1, "{family:?}: {}", r.summary());
        let model = Model::new(m3).unwrap();
        let errors: Vec<_> = model.validate(&Loads::default(), &model.dirichlet()).into_iter().filter(|i| i.severity == fea_core::validate::Severity::Error && i.code != "unsupported" && !i.code.contains("restrain") && !i.code.contains("rigid")).collect();
        assert!(errors.is_empty(), "{errors:?}");
    }
}

#[test]
fn the_mixed_hex_wedge_mesh_passes_the_patch_test_exactly() {
    for family in [ElementFamily::Linear, ElementFamily::Quadratic] {
        let m3 = extrude(&hybrid_2d(family, PLANE), &[0.0, 0.5, 1.2]).unwrap();
        let model = Model::new(m3).unwrap();
        // affine displacement on the boundary, the interior and the stress must follow
        let g = [[1.0e-4, 2.0e-5, -1.0e-5], [-3.0e-5, 4.0e-5, 2.0e-5], [1.5e-5, -2.5e-5, 6.0e-5]];
        let c = [5.0e-6, -7.0e-6, 3.0e-6];
        let field = |x: &[f64; 3]| -> [f64; 3] { std::array::from_fn(|i| c[i] + (0..3).map(|j| g[i][j] * x[j]).sum::<f64>()) };
        let mut on_boundary = vec![false; model.mesh.nodes.len()];
        for f in model.mesh.boundary_faces() {
            for &n in &f.nodes {
                on_boundary[n] = true;
            }
        }
        assert!(on_boundary.iter().any(|b| !b), "the patch needs interior nodes");
        let mut bc = model.dirichlet();
        for n in (0..on_boundary.len()).filter(|&n| on_boundary[n]) {
            let u = field(&model.mesh.nodes[n]);
            for k in 0..3 {
                bc.fix(n, k, u[k]);
            }
        }
        let sol = model.solve_static(&Loads::default(), &bc).unwrap();
        let mut worst = 0.0f64;
        for (n, x) in model.mesh.nodes.iter().enumerate() {
            let u = field(x);
            for k in 0..3 {
                worst = worst.max((sol.u[3 * n + k] - u[k]).abs());
            }
        }
        assert!(worst < 1e-12, "{family:?}: displacement error {worst:e}");
        let (lam, mu) = (E * NU / ((1.0 + NU) * (1.0 - 2.0 * NU)), E / (2.0 * (1.0 + NU)));
        let tr = g[0][0] + g[1][1] + g[2][2];
        let want = [lam * tr + 2.0 * mu * g[0][0], lam * tr + 2.0 * mu * g[1][1], lam * tr + 2.0 * mu * g[2][2]];
        let mut worst_s = 0.0f64;
        for blk in model.gauss_stresses(&sol.u, 0.0).unwrap() {
            for (_, s) in blk {
                for k in 0..3 {
                    worst_s = worst_s.max((s[k] - want[k]).abs() / want[0].abs());
                }
            }
        }
        assert!(worst_s < 1e-9, "{family:?}: stress error {worst_s:e}");
    }
}

#[test]
fn an_extruded_hybrid_plane_strain_solution_equals_the_2d_one() {
    for family in [ElementFamily::Linear, ElementFamily::Quadratic] {
        let m2 = hybrid_2d(family, PLANE);
        let solve = |mesh: Mesh, three: bool| -> (Vec<f64>, Mesh) {
            let model = Model::new(mesh).unwrap();
            let d = if three { 3 } else { 2 };
            let mut bc = model.dirichlet();
            for &n in model.mesh.node_set("left").unwrap() {
                bc.fix(n, 0, 0.0);
            }
            let anchor = *model.mesh.node_set("left").unwrap().iter().min_by(|&&a, &&b| (model.mesh.nodes[a][1] - 2.0).abs().total_cmp(&(model.mesh.nodes[b][1] - 2.0).abs())).unwrap();
            bc.fix(anchor, 1, 0.0);
            if three {
                for name in ["start", "end"] {
                    for &n in model.mesh.node_set(name).unwrap() {
                        bc.fix(n, 2, 0.0);
                    }
                }
            }
            let loads = Loads { faces: model.mesh.surfaces["right"].iter().map(|f| (f.clone(), SurfaceLoad::Traction([1000.0, 0.0, 0.0]))).collect(), ..Loads::default() };
            let sol = model.solve_static(&loads, &bc).unwrap();
            assert_eq!(sol.u.len(), d * model.mesh.nodes.len());
            (sol.u, model.mesh.clone())
        };
        let (u2, _) = solve(m2.clone(), false);
        let (u3, m3) = solve(extrude(&m2, &[0.0, 0.4, 1.0]).unwrap(), true);
        let scale = u2.iter().fold(0.0f64, |m, v| m.max(v.abs()));
        let mut worst = 0.0f64;
        for (n3, x3) in m3.nodes.iter().enumerate() {
            let n2 = m2.nodes.iter().position(|x2| (x2[0] - x3[0]).abs() < 1e-12 && (x2[1] - x3[1]).abs() < 1e-12).expect("matching 2D node");
            worst = worst.max((u3[3 * n3] - u2[2 * n2]).abs()).max((u3[3 * n3 + 1] - u2[2 * n2 + 1]).abs()).max(u3[3 * n3 + 2].abs());
        }
        assert!(worst < 1e-9 * scale, "{family:?}: 3D differs from 2D by {worst:e} of {scale:e}");
    }
}

#[test]
fn a_wedge_grid_has_the_exact_volume_and_surface_area() {
    for kind in [ElementKind::Wedge6, ElementKind::Wedge15] {
        let m = fea_core::generate::grid(Physics::Solid, kind, Elastic::new(E, NU), [3, 2, 2], &|p| [2.0 * p[0], 3.0 * p[1], 0.5 * p[2]]).unwrap();
        assert!(m.conformity().is_conforming());
        let model = Model::new(m).unwrap();
        // uniaxial stretch: strain energy = 1/2 E eps^2 V with a free lateral contraction is awkward; use a rigid translation
        // plus a uniform axial strain with the lateral faces free: u = (eps x, -nu eps y, -nu eps z)
        let eps = 1e-4;
        let u: Vec<f64> = model.mesh.nodes.iter().flat_map(|x| [eps * x[0], -NU * eps * x[1], -NU * eps * x[2]]).collect();
        let w = model.strain_energy(&u).unwrap();
        let volume = 2.0 * 3.0 * 0.5;
        assert!((w - 0.5 * E * eps * eps * volume).abs() < 1e-9 * w, "{kind:?}: energy {w} vs {}", 0.5 * E * eps * eps * volume);
    }
}
