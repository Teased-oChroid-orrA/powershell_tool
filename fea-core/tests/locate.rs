//! Point location: stress at an arbitrary point of every element type.

use fea_core::element::ALL_KINDS;
use fea_core::generate::grid;
use fea_core::kernel::stress_from_strain;
use fea_core::*;

fn skew(p: [f64; 3]) -> [f64; 3] {
    [2.0 * p[0] + 0.3 * p[1] + 0.1 * p[2], 0.2 * p[0] + 1.5 * p[1] + 0.2 * p[2], 0.1 * p[0] - 0.1 * p[1] + 1.2 * p[2]]
}

#[test]
fn a_linear_field_has_the_same_stress_at_every_located_point_for_every_element_type() {
    let h = [[1.0e-3, 4.0e-4, -2.0e-4], [-3.0e-4, 2.0e-3, 5.0e-4], [2.0e-4, -1.0e-4, 8.0e-4]];
    for kind in ALL_KINDS {
        let d = kind.dim();
        let physics = if d == 3 { Physics::Solid } else { Physics::PlaneStress { thickness: 0.5 } };
        let div = if d == 2 { [3, 3, 1] } else { [2, 2, 2] };
        let mesh = grid(physics, kind, Elastic::new(10.0e6, 0.3), div, &skew).unwrap();
        let model = Model::new(mesh).unwrap();
        let dd = model.mesh.dim();
        let mut u = vec![0.0; model.mesh.n_dofs()];
        for (n, x) in model.mesh.nodes.iter().enumerate() {
            for i in 0..dd {
                u[n * dd + i] = (0..dd).map(|k| h[i][k] * x[k]).sum::<f64>() + 1e-4 * (i as f64 + 1.0);
            }
        }
        let strain = if d == 3 {
            [h[0][0], h[1][1], h[2][2], h[0][1] + h[1][0], h[1][2] + h[2][1], h[2][0] + h[0][2]]
        } else {
            [h[0][0], h[1][1], 0.0, h[0][1] + h[1][0], 0.0, 0.0]
        };
        let want = stress_from_strain(physics, &Elastic::new(10.0e6, 0.3), strain, 0.0);
        let loc = model.locator();
        // Interior points: images of reference points under the skew map (the grid spans unit coordinates).
        let mut found = 0;
        for &(a, b, c) in &[(0.31, 0.22, 0.4), (0.5, 0.5, 0.5), (0.9, 0.1, 0.7), (0.05, 0.95, 0.2), (1.0 / 3.0, 2.0 / 3.0, 0.5)] {
            let x = skew([a, b, if d == 3 { c } else { 0.0 }]);
            let s = loc.stress_at(&model, &u, x, 0.0).unwrap_or_else(|| panic!("{kind:?}: {x:?} not located"));
            for k in 0..6 {
                assert!((s[k] - want[k]).abs() < 1e-6 * want.iter().fold(1.0f64, |m, v| m.max(v.abs())), "{kind:?} comp {k}: {} vs {}", s[k], want[k]);
            }
            found += 1;
        }
        assert_eq!(found, 5);
        // Outside the mesh.
        assert!(loc.stress_at(&model, &u, skew([1.5, 0.5, 0.5]), 0.0).is_none(), "{kind:?}");
        assert!(loc.stress_at(&model, &u, skew([-0.2, 0.5, 0.5]), 0.0).is_none(), "{kind:?}");
    }
}

#[test]
fn a_point_on_a_shared_edge_is_found_in_every_element_that_has_it() {
    let mesh = grid(Physics::PlaneStress { thickness: 1.0 }, ElementKind::Quad8, Elastic::new(1.0e7, 0.3), [2, 2, 1], &|p| p).unwrap();
    let model = Model::new(mesh).unwrap();
    let loc = model.locator();
    // The centre of a 2 x 2 grid (unit square) belongs to all four elements.
    assert_eq!(loc.locate(&model, [0.5, 0.5, 0.0]).len(), 4);
    // Mid-edge: two elements; interior: one.
    assert_eq!(loc.locate(&model, [0.5, 0.25, 0.0]).len(), 2);
    assert_eq!(loc.locate(&model, [0.2, 0.2, 0.0]).len(), 1);
}

#[test]
fn a_point_just_off_a_curved_boundary_takes_the_nearest_element_but_a_far_one_is_outside() {
    use fea_core::delaunay::MeshOptions;
    use fea_core::geometry::{Loop, Region};
    use fea_core::mesh2d::mesh_region;
    // Disc of radius 1: the quadratic boundary edge cuts slightly inside the true circle.
    let region = Region::new(Loop::circle([0.0, 0.0], 1.0, "rim").unwrap(), vec![], Elastic::new(1.0e7, 0.3)).unwrap();
    let mesh = mesh_region(&region, Physics::PlaneStress { thickness: 1.0 }, ElementKind::Quad8, &|_| 0.4, MeshOptions::default()).unwrap();
    let model = Model::new(mesh).unwrap();
    let loc = model.locator();
    let u = vec![0.0; model.mesh.n_dofs()];
    for k in 0..24 {
        let a = k as f64 * std::f64::consts::TAU / 24.0;
        assert!(loc.stress_at(&model, &u, [a.cos(), a.sin(), 0.0], 0.0).is_some(), "rim point at {a}");
    }
    assert!(loc.stress_at(&model, &u, [1.2, 0.0, 0.0], 0.0).is_none());
}
