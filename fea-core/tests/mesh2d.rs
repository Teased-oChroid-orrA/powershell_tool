//! Unstructured meshes of curved regions against closed-form solutions.

#![allow(clippy::needless_range_loop)] // index-parallel numeric kernels read clearer as loops

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

/// Quarter annulus `a <= r <= b`, `x, y >= 0`, segments named `xaxis`, `outer`, `yaxis`, `inner`.
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

fn element_area(mesh: &Mesh) -> f64 {
    let mut work = Work::new();
    let mut area = 0.0;
    for blk in &mesh.blocks {
        let t = blk.kind.table();
        for e in 0..blk.n_elems() {
            let xyz: Vec<[f64; 3]> = blk.elem(e).iter().map(|&n| mesh.nodes[n]).collect();
            geometry(blk.kind, mesh.physics, &xyz, &mut work).expect("positive Jacobian");
            area += (0..t.ngp).map(|g| work.wdet[g]).sum::<f64>();
        }
    }
    area
}

/// Max radial displacement error / `u_r(b)` of the plane-strain Lame problem on an unstructured mesh.
fn lame_error(kind: ElementKind, h: f64) -> (f64, usize) {
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
    let mut worst = 0.0f64;
    for (n, x) in model.mesh.nodes.iter().enumerate() {
        let r = x[0].hypot(x[1]);
        let ur = (sol.u[n * 2] * x[0] + sol.u[n * 2 + 1] * x[1]) / r;
        worst = worst.max((ur - lame_ur(r)).abs() / lame_ur(B));
    }
    (worst, model.mesh.nodes.len())
}

#[test]
fn unstructured_meshes_of_every_2d_element_type_converge_to_lame() {
    for (kind, tol) in [(ElementKind::Tri6, 4e-4), (ElementKind::Quad8, 4e-4), (ElementKind::Quad9, 4e-4), (ElementKind::Quad4, 3e-2), (ElementKind::Tri3, 1e-1)] {
        let runs: Vec<(f64, usize)> = [0.5, 0.25, 0.125].iter().map(|&h| lame_error(kind, h)).collect();
        eprintln!("{kind:?}: u_r error {:.2e} ({} nodes) -> {:.2e} ({}) -> {:.2e} ({})", runs[0].0, runs[0].1, runs[1].0, runs[1].1, runs[2].0, runs[2].1);
        assert!(runs[2].0 < tol, "{kind:?}: finest error {:e}", runs[2].0);
        assert!(runs[2].0 < runs[0].0 / 4.0, "{kind:?}: no convergence {runs:?}");
        if matches!(kind, ElementKind::Tri6 | ElementKind::Quad8 | ElementKind::Quad9) {
            // Quadratic elements: roughly h^3 in the max norm on an unstructured mesh.
            assert!(runs[1].0 < runs[0].0 / 3.0 && runs[2].0 < runs[1].0 / 3.0, "{kind:?}: rate {runs:?}");
        }
    }
}

#[test]
fn curved_boundaries_are_represented_by_quadratic_arcs_not_chords() {
    let exact = 0.25 * PI * (B * B - A * A);
    let area = |kind| element_area(&mesh_region(&quarter_annulus(), Physics::PlaneStrain { thickness: 1.0 }, kind, &|_| 0.5, MeshOptions::default()).unwrap());
    let (a3, a6, a8) = (area(ElementKind::Tri3), area(ElementKind::Tri6), area(ElementKind::Quad8));
    eprintln!("quarter-annulus area error at h = 0.5: Tri3 {:.2e}, Tri6 {:.2e}, Quad8 {:.2e}", (a3 / exact - 1.0).abs(), (a6 / exact - 1.0).abs(), (a8 / exact - 1.0).abs());
    assert!((a6 / exact - 1.0).abs() < (a3 / exact - 1.0).abs() / 20.0, "Tri6 must follow the arcs");
    assert!((a8 / exact - 1.0).abs() < (a3 / exact - 1.0).abs() / 20.0, "Quad8 must follow the arcs");
}

#[test]
fn surfaces_integrate_a_pressure_to_the_exact_resultant_for_every_type() {
    // Inner pressure on the quarter arc: F_x = p a (and F_y = p a); symmetric-edge pressure resultants too.
    for kind in [ElementKind::Tri3, ElementKind::Tri6, ElementKind::Quad4, ElementKind::Quad8, ElementKind::Quad9] {
        let mesh = mesh_region(&quarter_annulus(), Physics::PlaneStrain { thickness: 1.0 }, kind, &|_| 0.2, MeshOptions::default()).unwrap();
        let loads = Loads { faces: mesh.surfaces["inner"].iter().map(|f| (f.clone(), SurfaceLoad::Pressure(P))).collect(), ..Loads::default() };
        let f = loads::assemble(&mesh, &loads).unwrap();
        let (fx, fy): (f64, f64) = (0..mesh.nodes.len()).fold((0.0, 0.0), |s, n| (s.0 + f[n * 2], s.1 + f[n * 2 + 1]));
        // Pressure pushes into the body (toward +r outward from the hole): force on the body is outward.
        let want = P * A;
        let tol = if matches!(kind, ElementKind::Tri3 | ElementKind::Quad4) { 5e-3 } else { 1e-6 };
        assert!((fx - want).abs() < tol * want && (fy - want).abs() < tol * want, "{kind:?}: resultant ({fx}, {fy}) vs {want}");
        // Every named segment produced a node set and a surface.
        for name in ["xaxis", "outer", "yaxis", "inner"] {
            assert!(!mesh.node_sets[name].is_empty() && !mesh.surfaces[name].is_empty(), "{kind:?}: {name}");
        }
    }
}

#[test]
fn a_plate_with_a_hole_meshes_graded_and_reaches_the_kirsch_stress_concentration() {
    // Quarter of a 8a x 8a plate with a hole of radius a = 1 under uniaxial tension, exact Kirsch tractions outside.
    let (a, half, sig) = (1.0f64, 8.0f64, 1000.0f64);
    let seg = |curve, name: &str| Segment { curve, name: name.to_string() };
    let outer = Loop::new(vec![
        seg(Curve::line([a, 0.0], [half, 0.0]), "xaxis"),
        seg(Curve::line([half, 0.0], [half, half]), "right"),
        seg(Curve::line([half, half], [0.0, half]), "top"),
        seg(Curve::line([0.0, half], [0.0, a]), "yaxis"),
        seg(Curve::arc([0.0, 0.0], a, FRAC_PI_2, 0.0), "hole"),
    ])
    .unwrap();
    let region = Region::new(outer, vec![], Elastic::new(E, NU)).unwrap();
    let kirsch = move |x: f64, y: f64| -> [f64; 3] {
        let (r, th) = (x.hypot(y), y.atan2(x));
        let (q, q2) = ((a / r).powi(2), (a / r).powi(4));
        let (c2, s2) = ((2.0 * th).cos(), (2.0 * th).sin());
        let sr = 0.5 * sig * (1.0 - q) + 0.5 * sig * (1.0 - 4.0 * q + 3.0 * q2) * c2;
        let st = 0.5 * sig * (1.0 + q) - 0.5 * sig * (1.0 + 3.0 * q2) * c2;
        let tr = -0.5 * sig * (1.0 + 2.0 * q - 3.0 * q2) * s2;
        let (c, s) = (th.cos(), th.sin());
        [sr * c * c + st * s * s - 2.0 * tr * s * c, sr * s * s + st * c * c + 2.0 * tr * s * c, (sr - st) * s * c + tr * (c * c - s * s)]
    };
    let mut errors = Vec::new();
    for h0 in [0.2, 0.1] {
        // Fine at the hole, coarsening away from it.
        let size = move |x: [f64; 2]| (h0 + 0.25 * ((x[0].hypot(x[1]) - a).max(0.0))).min(1.0);
        let mesh = mesh_region(&region, Physics::PlaneStress { thickness: 1.0 }, ElementKind::Tri6, &size, MeshOptions::default()).unwrap();
        let model = Model::new(mesh).unwrap();
        let mut bc = model.dirichlet();
        for &n in model.mesh.node_set("xaxis").unwrap() {
            bc.fix(n, 1, 0.0);
        }
        for &n in model.mesh.node_set("yaxis").unwrap() {
            bc.fix(n, 0, 0.0);
        }
        let field = FaceField::new(move |x, n| {
            let s = kirsch(x[0], x[1]);
            [s[0] * n[0] + s[2] * n[1], s[2] * n[0] + s[1] * n[1], 0.0]
        });
        let faces: Vec<_> = ["right", "top"].iter().flat_map(|nm| model.mesh.surfaces[*nm].iter().map(|f| (f.clone(), field.clone()))).collect();
        let sol = model.solve_static(&Loads { field_faces: faces, ..Loads::default() }, &bc).unwrap();
        // Hole-edge hoop stress: SPR at the hole nodes against sigma (1 - 2 cos 2 theta).
        let spr = model.recover_spr(&model.gauss_stresses(&sol.u, 0.0).unwrap()).unwrap();
        let mut worst = 0.0f64;
        let mut pole = 0.0;
        for &n in model.mesh.node_set("hole").unwrap() {
            let x = model.mesh.nodes[n];
            let (c, s) = (x[0] / a, x[1] / a);
            let hoop = spr[n][0] * s * s + spr[n][1] * c * c - 2.0 * spr[n][3] * s * c;
            worst = worst.max((hoop - sig * (1.0 - 2.0 * (2.0 * x[1].atan2(x[0])).cos())).abs() / (3.0 * sig));
            if x[0].abs() < 1e-9 {
                pole = hoop / sig;
            }
        }
        eprintln!("Kirsch on an unstructured Tri6 mesh, h0 = {h0}: {} nodes, hole hoop error {worst:.2e} of 3 sigma, pole stress {pole:.4} sigma", model.mesh.nodes.len());
        errors.push((worst, pole));
    }
    assert!(errors[1].0 < errors[0].0, "refinement must reduce the hole stress error: {errors:?}");
    assert!(errors[1].0 < 1.5e-2 && (errors[1].1 - 3.0).abs() < 0.05, "{errors:?}");
}

#[test]
fn several_holes_and_a_thin_web_mesh_without_inverted_elements() {
    let c = Elastic::new(E, NU);
    let outer = Loop::rectangle(0.0, 0.0, 6.0, 3.0).unwrap();
    // Two bores and a slot (rounded ends), the web between the bores only 0.15 wide.
    let h1 = Loop::circle([1.5, 1.5], 0.7, "bore1").unwrap();
    let h2 = Loop::circle([3.05, 1.5], 0.7, "bore2").unwrap();
    let slot = Loop::new(vec![
        Segment { curve: Curve::line([4.5, 0.8], [5.3, 0.8]), name: "slot".into() },
        Segment { curve: Curve::arc([5.3, 1.0], 0.2, -FRAC_PI_2, FRAC_PI_2), name: "slot".into() },
        Segment { curve: Curve::line([5.3, 1.2], [4.5, 1.2]), name: "slot".into() },
        Segment { curve: Curve::arc([4.5, 1.0], 0.2, FRAC_PI_2, 3.0 * FRAC_PI_2), name: "slot".into() },
    ])
    .unwrap();
    let region = Region::new(outer, vec![h1, h2, slot], c).unwrap();
    let exact_area = 18.0 - 2.0 * PI * 0.49 - (0.8 * 0.4 + PI * 0.04);
    for kind in [ElementKind::Tri6, ElementKind::Quad8, ElementKind::Quad9] {
        let mesh = mesh_region(&region, Physics::PlaneStress { thickness: 0.2 }, kind, &|_| 0.35, MeshOptions { min_edge: 1e-3, ..MeshOptions::default() }).unwrap();
        let area = element_area(&mesh) / 0.2;
        assert!((area / exact_area - 1.0).abs() < 2e-3, "{kind:?}: area {area} vs {exact_area}");
        Model::new(mesh.clone()).unwrap().assemble().expect("positive Jacobians everywhere");
        for name in ["bore1", "bore2", "slot"] {
            assert!(!mesh.node_sets[name].is_empty());
        }
    }
}
