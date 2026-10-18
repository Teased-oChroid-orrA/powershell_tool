//! NAFEMS linear-static benchmarks with published targets, the independent accuracy references of the benchmark plan
//! (`docs/issue-46-benchmarks.md`, section B): LE1 (elliptic membrane, plane stress) and LE10 (thick elliptic plate,
//! three-dimensional). Both quarter models share one geometry: an elliptic annulus between the inner ellipse
//! `x^2/2^2 + y^2/1^2 = 1` and the outer ellipse `x^2/3.25^2 + y^2/2.75^2 = 1`, point D = (2, 0).
//! Units: m and MPa. The targets are the values the benchmark publishes (`sigma_yy` at D: LE1 92.7 MPa, LE10 -5.38 MPa).

#![allow(clippy::needless_range_loop)] // index-parallel numeric kernels read clearer as loops

use fea_core::generate::grid;
use fea_core::*;
use std::f64::consts::PI;

const E: f64 = 210.0e3;
const NU: f64 = 0.3;

/// `u` from the inner to the outer ellipse, `v` the angle parameter `t` in `[0, pi/2]` (the ellipses are
/// `(a cos t, b sin t)`); `w` (3D) the height `0 .. thickness`.
fn elliptic_annulus(thickness: f64) -> impl Fn([f64; 3]) -> [f64; 3] {
    move |p| {
        let t = 0.5 * PI * p[1];
        let (c, s) = (t.cos(), t.sin());
        let (inner, outer) = ([2.0 * c, 1.0 * s], [3.25 * c, 2.75 * s]);
        [
            (1.0 - p[0]) * inner[0] + p[0] * outer[0],
            (1.0 - p[0]) * inner[1] + p[0] * outer[1],
            p[2] * thickness,
        ]
    }
}

fn nodes_of(model: &Model, name: &str) -> Vec<usize> {
    model.mesh.node_set(name).unwrap().to_vec()
}

/// The node of the mesh at the point `(x, y, z)` (within 1e-9).
fn node_at(model: &Model, x: f64, y: f64, z: f64) -> usize {
    model
        .mesh
        .nodes
        .iter()
        .position(|n| (n[0] - x).abs() < 1e-9 && (n[1] - y).abs() < 1e-9 && (n[2] - z).abs() < 1e-9)
        .unwrap_or_else(|| panic!("no node at ({x}, {y}, {z})"))
}

/// LE1: plane stress, thickness 0.1, outward pressure 10 on the outer ellipse, symmetry on both axes.
/// Returns `sigma_yy` at D.
fn le1(kind: ElementKind, radial: usize, angular: usize) -> f64 {
    let mesh = grid(
        Physics::PlaneStress { thickness: 0.1 },
        kind,
        Elastic::new(E, NU),
        [radial, angular, 1],
        &elliptic_annulus(0.0),
    )
    .unwrap();
    let model = Model::new(mesh).unwrap();
    let mut bc = model.dirichlet();
    for n in nodes_of(&model, "v0") {
        bc.fix(n, 1, 0.0); // y = 0: no motion in y
    }
    for n in nodes_of(&model, "v1") {
        bc.fix(n, 0, 0.0); // x = 0: no motion in x
    }
    let p = 10.0;
    let outward = FaceField::new(move |_x, n| [p * n[0], p * n[1], 0.0]);
    let loads = Loads {
        field_faces: model.mesh.surfaces["u1"]
            .iter()
            .map(|f| (f.clone(), outward.clone()))
            .collect(),
        ..Loads::default()
    };
    let sol = model.solve_static(&loads, &bc).unwrap();
    let stress = model.nodal_stresses(&sol.u, 0.0).unwrap();
    let d = node_at(&model, 2.0, 0.0, 0.0);
    stress[d][1]
}

#[test]
fn le1_elliptic_membrane_converges_to_the_published_92_7_mpa() {
    let target = 92.7;
    let runs: Vec<(usize, f64)> = [2usize, 4, 8, 16]
        .iter()
        .map(|&m| (m, le1(ElementKind::Quad9, m, 3 * m)))
        .collect();
    for (m, s) in &runs {
        eprintln!(
            "LE1 Quad9 {m}x{}: sigma_yy(D) = {s:.4} MPa ({:+.2} %)",
            3 * m,
            100.0 * (s / target - 1.0)
        );
    }
    let err: Vec<f64> = runs.iter().map(|r| (r.1 / target - 1.0).abs()).collect();
    assert!(
        err[3] < 0.01,
        "finest mesh {} MPa is not within 1 % of {target}",
        runs[3].1
    );
    assert!(
        err[3] < err[1],
        "refinement does not improve the answer: {err:?}"
    );
    // a second element family reaches the same value
    let tri = le1(ElementKind::Tri6, 12, 36);
    eprintln!("LE1 Tri6 12x36: sigma_yy(D) = {tri:.4} MPa");
    assert!((tri / target - 1.0).abs() < 0.03, "Tri6 gives {tri} MPa");
}

/// LE10 (NAFEMS): thickness 0.6, uniform normal pressure 1 on the upper surface (z = 0.6), symmetry on both axes, the outer
/// face held in x and y, and `u_z = 0` along the mid-height line of the outer face. Returns `sigma_yy` at D = (2, 0, 0.6), on
/// the upper surface at the inner edge.
fn le10(kind: ElementKind, radial: usize, angular: usize, layers: usize) -> f64 {
    let mesh = grid(
        Physics::Solid,
        kind,
        Elastic::new(E, NU),
        [radial, angular, layers],
        &elliptic_annulus(0.6),
    )
    .unwrap();
    let model = Model::new(mesh).unwrap();
    let mut bc = model.dirichlet();
    for n in nodes_of(&model, "v0") {
        bc.fix(n, 1, 0.0);
    }
    for n in nodes_of(&model, "v1") {
        bc.fix(n, 0, 0.0);
    }
    for n in nodes_of(&model, "u1") {
        bc.fix(n, 0, 0.0);
        bc.fix(n, 1, 0.0);
        if (model.mesh.nodes[n][2] - 0.3).abs() < 1e-9 {
            bc.fix(n, 2, 0.0);
        }
    }
    let loads = Loads {
        faces: model.mesh.surfaces["w1"]
            .iter()
            .map(|f| (f.clone(), SurfaceLoad::Pressure(1.0)))
            .collect(),
        ..Loads::default()
    };
    let sol = model.solve_static(&loads, &bc).unwrap();
    let stress = model.nodal_stresses(&sol.u, 0.0).unwrap();
    stress[node_at(&model, 2.0, 0.0, 0.6)][1]
}

#[test]
fn le10_thick_elliptic_plate_converges_to_the_published_minus_5_38_mpa() {
    let target = -5.38;
    let runs: Vec<(usize, f64)> = [(2usize, 2usize), (4, 2), (6, 4), (10, 4), (14, 6)]
        .iter()
        .map(|&(m, l)| (m, le10(ElementKind::Hex27, m, 3 * m, l)))
        .collect();
    for (m, s) in &runs {
        eprintln!(
            "LE10 Hex27 {m}x{}: sigma_yy(D) = {s:.4} MPa ({:+.2} %)",
            3 * m,
            100.0 * (s / target - 1.0)
        );
    }
    let last = runs.last().unwrap().1;
    assert!(
        (last / target - 1.0).abs() < 0.02,
        "upper-surface stress {last} MPa is not within 2 % of {target}"
    );
    // the nodal stress at the surface node converges slowly and not monotonically, but the finest mesh beats the coarse ones
    assert!(
        (last / target - 1.0).abs() < (runs[1].1 / target - 1.0).abs(),
        "refinement does not improve the answer: {runs:?}"
    );
}
