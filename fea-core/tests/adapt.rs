//! Error-driven adaptive remeshing: the size field, and adaptive against uniform refinement on the
//! Kirsch plate (exact stress known) and on an L-shaped plate (re-entrant corner singularity).

#![allow(clippy::needless_range_loop)] // index-parallel numeric kernels read clearer as loops

use fea_core::adapt::{adapted_size_field, AdaptOptions, SizeField};
use fea_core::delaunay::MeshOptions;
use fea_core::geometry::{Curve, Loop, Region, Segment};
use fea_core::mesh2d::mesh_region;
use fea_core::*;
use std::f64::consts::FRAC_PI_2;

const E: f64 = 10.0e6;
const NU: f64 = 0.3;
const SIG: f64 = 1000.0;
const A: f64 = 1.0;
const HALF: f64 = 6.0;

fn kirsch(x: f64, y: f64) -> [f64; 6] {
    let (r, th) = (x.hypot(y), y.atan2(x));
    let (q, q2) = ((A / r).powi(2), (A / r).powi(4));
    let (c2, s2) = ((2.0 * th).cos(), (2.0 * th).sin());
    let sr = 0.5 * SIG * (1.0 - q) + 0.5 * SIG * (1.0 - 4.0 * q + 3.0 * q2) * c2;
    let st = 0.5 * SIG * (1.0 + q) - 0.5 * SIG * (1.0 + 3.0 * q2) * c2;
    let tr = -0.5 * SIG * (1.0 + 2.0 * q - 3.0 * q2) * s2;
    let (c, s) = (th.cos(), th.sin());
    [sr * c * c + st * s * s - 2.0 * tr * s * c, sr * s * s + st * c * c + 2.0 * tr * s * c, 0.0, (sr - st) * s * c + tr * (c * c - s * s), 0.0, 0.0]
}

fn plate_region() -> Region {
    let seg = |curve, name: &str| Segment { curve, name: name.to_string() };
    let outer = Loop::new(vec![
        seg(Curve::line([A, 0.0], [HALF, 0.0]), "xaxis"),
        seg(Curve::line([HALF, 0.0], [HALF, HALF]), "right"),
        seg(Curve::line([HALF, HALF], [0.0, HALF]), "top"),
        seg(Curve::line([0.0, HALF], [0.0, A]), "yaxis"),
        seg(Curve::arc([0.0, 0.0], A, FRAC_PI_2, 0.0), "hole"),
    ])
    .unwrap();
    Region::new(outer, vec![], Elastic::new(E, NU)).unwrap()
}

/// Solve the Kirsch plate on a mesh; returns the model, the solution and the true stress error energy.
fn solve_kirsch(mesh: Mesh) -> (Model, Solution, f64) {
    let model = Model::new(mesh).unwrap();
    let mut bc = model.dirichlet();
    for &n in model.mesh.node_set("xaxis").unwrap() {
        bc.fix(n, 1, 0.0);
    }
    for &n in model.mesh.node_set("yaxis").unwrap() {
        bc.fix(n, 0, 0.0);
    }
    let field = FaceField::new(|x, n| {
        let s = kirsch(x[0], x[1]);
        [s[0] * n[0] + s[3] * n[1], s[3] * n[0] + s[1] * n[1], 0.0]
    });
    let faces: Vec<_> = ["right", "top"].iter().flat_map(|nm| model.mesh.surfaces[*nm].iter().map(|f| (f.clone(), field.clone()))).collect();
    let sol = model.solve_static(&Loads { field_faces: faces, ..Loads::default() }, &bc).unwrap();
    let err = model.stress_error_energy(&sol.u, 0.0, |x| kirsch(x[0], x[1])).unwrap();
    (model, sol, err)
}

#[test]
fn size_field_interpolates_a_linear_field_exactly_and_degrades_gracefully_outside() {
    // Unit square split into triangles with nodal size h(x, y) = 0.1 + 0.3 x + 0.2 y.
    let f = |p: [f64; 2]| 0.1 + 0.3 * p[0] + 0.2 * p[1];
    let n = 6;
    let pts: Vec<[f64; 2]> = (0..=n).flat_map(|j| (0..=n).map(move |i| [i as f64 / n as f64, j as f64 / n as f64])).collect();
    let mut tris = Vec::new();
    for j in 0..n {
        for i in 0..n {
            let a = j * (n + 1) + i;
            tris.push([a, a + 1, a + n + 2]);
            tris.push([a, a + n + 2, a + n + 1]);
        }
    }
    let sf = SizeField::new(pts.clone(), tris, pts.iter().map(|p| f(*p)).collect());
    for k in 0..200 {
        let p = [((k * 37) % 101) as f64 / 101.0, ((k * 53) % 97) as f64 / 97.0];
        assert!((sf.at(p) - f(p)).abs() < 1e-12, "{p:?}: {} vs {}", sf.at(p), f(p));
    }
    // A hair outside: clamped to the edge value; far outside: the nearest vertex.
    assert!((sf.at([1.0 + 1e-6, 0.5]) - f([1.0, 0.5])).abs() < 1e-3);
    assert!((sf.at([5.0, 5.0]) - f([1.0, 1.0])).abs() < 1e-12);
}

#[test]
fn adaptive_refinement_beats_uniform_refinement_on_the_kirsch_plate() {
    let region = plate_region();
    let opt = MeshOptions::default();
    // Uniform reference series (error against degrees of freedom).
    let mut uniform: Vec<(usize, f64)> = Vec::new();
    for h in [1.2, 0.8, 0.55, 0.38, 0.26, 0.18] {
        let mesh = mesh_region(&region, Physics::PlaneStress { thickness: 1.0 }, ElementKind::Tri6, &|_| h, opt).unwrap();
        let (m, _, err) = solve_kirsch(mesh);
        uniform.push((m.mesh.n_dofs(), err));
    }
    // Adaptive passes starting from a coarse uniform mesh.
    let mut mesh = mesh_region(&region, Physics::PlaneStress { thickness: 1.0 }, ElementKind::Tri6, &|_| 1.2, opt).unwrap();
    let mut adaptive: Vec<(usize, f64, f64)> = Vec::new();
    for pass in 0..6 {
        let (model, sol, err) = solve_kirsch(mesh);
        let zz = model.zz_error(&sol.u, 0.0).unwrap();
        adaptive.push((model.mesh.n_dofs(), err, zz.total));
        eprintln!("pass {pass}: {} dofs, true error {err:.4e}, ZZ eta {:.4e} (effectivity {:.2})", model.mesh.n_dofs(), zz.total, zz.total / err);
        let sf = adapted_size_field(&model.mesh, &zz, false, &AdaptOptions { target_rel_error: 0.005, order: 2.0, h_min: 0.02, h_max: 2.0, ..AdaptOptions::default() }).unwrap();
        mesh = mesh_region(&region, Physics::PlaneStress { thickness: 1.0 }, ElementKind::Tri6, &|x| sf.at(x), opt).unwrap();
    }
    for (n, e) in &uniform {
        eprintln!("uniform: {n} dofs, true error {e:.4e}");
    }
    // Interpolate the uniform curve (log-log) at the adaptive error levels.
    let dofs_for = |target: f64| -> Option<f64> {
        for w in uniform.windows(2) {
            let ((n0, e0), (n1, e1)) = (w[0], w[1]);
            if target <= e0 && target >= e1 {
                let t = (target.ln() - e0.ln()) / (e1.ln() - e0.ln());
                return Some((n0 as f64).ln().mul_add(1.0 - t, (n1 as f64).ln() * t).exp());
            }
        }
        None
    };
    let mut compared = 0;
    for &(n, e, _) in adaptive.iter().skip(2) {
        if let Some(nu) = dofs_for(e) {
            eprintln!("error {e:.3e}: adaptive {n} dofs vs uniform {nu:.0} dofs ({:.2}x fewer)", nu / n as f64);
            assert!(nu > 1.25 * n as f64, "adaptive ({n} dofs) should need clearly fewer dofs than uniform ({nu:.0}) for error {e:e}");
            compared += 1;
        }
    }
    assert!(compared >= 1, "no adaptive pass fell inside the uniform error range");
    // The estimator tracks the true error (effectivity near one) once the mesh resolves the hole.
    let last = adaptive.last().unwrap();
    assert!((0.7..1.5).contains(&(last.2 / last.1)), "effectivity {}", last.2 / last.1);
    assert!(adaptive.last().unwrap().1 < adaptive[0].1 / 3.0, "adaptive error must fall");
}

#[test]
fn an_l_shaped_plate_refines_toward_the_reentrant_corner() {
    // Re-entrant corner at (1, 1): the stress singularity concentrates the ZZ error there.
    let l = Loop::polygon_named(&[[0.0, 0.0], [3.0, 0.0], [3.0, 1.0], [1.0, 1.0], [1.0, 3.0], [0.0, 3.0]], &["bottom", "right", "inner_h", "inner_v", "top", "left"]).unwrap();
    let region = Region::new(l, vec![], Elastic::new(E, NU)).unwrap();
    let mut mesh = mesh_region(&region, Physics::PlaneStress { thickness: 1.0 }, ElementKind::Tri6, &|_| 0.5, MeshOptions::default()).unwrap();
    let mut sizes: Vec<(f64, f64)> = Vec::new();
    for _ in 0..5 {
        let model = Model::new(mesh).unwrap();
        let mut bc = model.dirichlet();
        for &n in model.mesh.node_set("left").unwrap() {
            bc.fix_node(n);
        }
        let loads = Loads { faces: model.mesh.surfaces["right"].iter().map(|f| (f.clone(), SurfaceLoad::Traction([0.0, 500.0, 0.0]))).collect(), ..Loads::default() };
        let sol = model.solve_static(&loads, &bc).unwrap();
        let zz = model.zz_error(&sol.u, 0.0).unwrap();
        // Mean element size within 0.3 of the corner vs. beyond 1.5 of it.
        let (mut near, mut far) = ((0.0, 0), (0.0, 0));
        let blk = &model.mesh.blocks[0];
        for e in 0..blk.n_elems() {
            let c = blk.elem(e);
            let cen = [(0..3).map(|k| model.mesh.nodes[c[k]][0]).sum::<f64>() / 3.0, (0..3).map(|k| model.mesh.nodes[c[k]][1]).sum::<f64>() / 3.0];
            let size = (0..3).map(|k| (model.mesh.nodes[c[k]][0] - model.mesh.nodes[c[(k + 1) % 3]][0]).hypot(model.mesh.nodes[c[k]][1] - model.mesh.nodes[c[(k + 1) % 3]][1])).sum::<f64>() / 3.0;
            let d = (cen[0] - 1.0).hypot(cen[1] - 1.0);
            if d < 0.3 {
                near = (near.0 + size, near.1 + 1);
            } else if d > 1.5 {
                far = (far.0 + size, far.1 + 1);
            }
        }
        if near.1 > 0 && far.1 > 0 {
            sizes.push((near.0 / near.1 as f64, far.0 / far.1 as f64));
        }
        let sf = adapted_size_field(&model.mesh, &zz, false, &AdaptOptions { target_rel_error: 0.01, h_min: 0.01, h_max: 1.0, ..AdaptOptions::default() }).unwrap();
        mesh = mesh_region(&region, Physics::PlaneStress { thickness: 1.0 }, ElementKind::Tri6, &|x| sf.at(x), MeshOptions::default()).unwrap();
    }
    let (near, far) = *sizes.last().unwrap();
    eprintln!("L plate: element size near the corner {near:.3}, far {far:.3} (ratio {:.1}); history {sizes:?}", far / near);
    // The corner ends up clearly finer than the far field, and far finer than where it started
    // (equidistribution against a 1 % target also shrinks the far field, so a fixed ratio is not the law).
    assert!(far / near > 2.5, "the singular corner must be finer than the far field: {sizes:?}");
    assert!(sizes[0].0 / near > 5.0, "the corner size must drop strongly over the passes: {sizes:?}");
}
