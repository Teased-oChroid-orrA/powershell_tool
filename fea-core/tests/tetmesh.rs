//! The signed-distance tetrahedral mesher on analytic solids: exact volumes where the boundary is planar, converging
//! volumes on curved ones, conformity, size-field grading, curved quadratic boundaries, and a finite-element patch test
//! (the mesh is only worth anything if a solve on it is right).
use fea_core::tetmesh::{mesh_solid, FaceHit, Solid, TetOptions};
use fea_core::{Elastic, ElementKind, Loads, Mesh, Model, SurfaceLoad};
use std::collections::HashMap;

fn cube_sdf(lo: f64, hi: f64) -> impl Fn([f64; 3]) -> f64 {
    let (c, h) = ((lo + hi) / 2.0, (hi - lo) / 2.0);
    move |x| (x[0] - c).abs().max((x[1] - c).abs()).max((x[2] - c).abs()) - h
}
/// Face of an axis-aligned box a surface point belongs to: x-, x+, y-, y+, z-, z+ = 0..5.
fn box_face(lo: [f64; 3], hi: [f64; 3]) -> impl Fn([f64; 3]) -> usize {
    move |x| {
        let mut best = (f64::INFINITY, 0);
        for k in 0..3 {
            for (s, v) in [(0, lo[k]), (1, hi[k])] {
                let d = (x[k] - v).abs();
                if d < best.0 {
                    best = (d, 2 * k + s);
                }
            }
        }
        best.1
    }
}
fn solid_box<'a>(lo: [f64; 3], hi: [f64; 3], sdf: &'a dyn Fn([f64; 3]) -> f64, size: &'a dyn Fn([f64; 3]) -> f64, face: &'a dyn Fn([f64; 3]) -> usize) -> Solid<'a> {
    Solid { lo, hi, sdf, size, face_of: face, face_dists: None }
}
fn tet_volume(mesh: &Mesh, t: &[usize]) -> f64 {
    let p = |i: usize| mesh.nodes[t[i]];
    let (a, b, c, d) = (p(0), p(1), p(2), p(3));
    let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let w = [d[0] - a[0], d[1] - a[1], d[2] - a[2]];
    (u[0] * (v[1] * w[2] - v[2] * w[1]) - u[1] * (v[0] * w[2] - v[2] * w[0]) + u[2] * (v[0] * w[1] - v[1] * w[0])) / 6.0
}

#[test]
fn a_box_is_filled_exactly_and_its_six_faces_are_named_and_outward() {
    let (lo, hi) = ([0.0; 3], [2.0, 2.0, 2.0]);
    let sdf = cube_sdf(0.0, 2.0);
    let (size, face) = (|_: [f64; 3]| 1.0, box_face(lo, hi));
    let (mesh, rep) = mesh_solid(&solid_box(lo, hi, &sdf, &size, &face), Elastic::new(1.0, 0.3), TetOptions { base: 0.5, boundary_size: 0.5, ..Default::default() }).unwrap();
    assert!((rep.volume - 8.0).abs() < 1e-9, "{rep:?}");
    assert!(rep.worst_quality > 0.2, "{rep:?}");
    let names: Vec<&String> = mesh.surfaces.keys().collect();
    assert_eq!(names.len(), 6, "{names:?}");
    // Each face is a 2 x 2 square, and the faces together enclose the volume with outward normals.
    let mut flux = 0.0;
    for (name, faces) in &mesh.surfaces {
        let mut area = 0.0;
        for f in faces {
            let (a, b, c) = (mesh.nodes[f[0]], mesh.nodes[f[1]], mesh.nodes[f[2]]);
            let n = [(b[1] - a[1]) * (c[2] - a[2]) - (b[2] - a[2]) * (c[1] - a[1]), (b[2] - a[2]) * (c[0] - a[0]) - (b[0] - a[0]) * (c[2] - a[2]), (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])];
            area += 0.5 * (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            // Divergence theorem with F = x: flux = integral of x . n dA = 3 V.
            flux += (a[0] + b[0] + c[0]) / 3.0 * n[0] / 2.0 + (a[1] + b[1] + c[1]) / 3.0 * n[1] / 2.0 + (a[2] + b[2] + c[2]) / 3.0 * n[2] / 2.0;
        }
        assert!((area - 4.0).abs() < 1e-9, "{name}: {area}");
    }
    assert!((flux - 3.0 * 8.0).abs() < 1e-9, "outward oriented faces enclose the volume: {flux}");
}

#[test]
fn a_sphere_converges_in_volume_and_stays_conforming() {
    let exact = 4.0 / 3.0 * std::f64::consts::PI;
    let sdf = |x: [f64; 3]| (x[0] * x[0] + x[1] * x[1] + x[2] * x[2]).sqrt() - 1.0;
    let (lo, hi) = ([-1.2; 3], [1.2; 3]);
    let (size, face) = (|_: [f64; 3]| 10.0, |_: [f64; 3]| 0usize);
    let mut previous = f64::INFINITY;
    for h in [0.3, 0.15] {
        let (mesh, rep) = mesh_solid(&solid_box(lo, hi, &sdf, &size, &face), Elastic::new(1.0, 0.3), TetOptions { base: 0.6, boundary_size: h, ..Default::default() }).unwrap();
        let err = (rep.volume - exact).abs() / exact;
        eprintln!("h {h}: {} tets, volume error {err:.3e}, worst quality {:.3}, snapped {} refused {}", rep.tets, rep.worst_quality, rep.snapped, rep.refused);
        assert!(err < previous, "the volume converges: {err} after {previous}");
        previous = err;
        assert!(rep.worst_quality >= 0.049 && rep.refused * 3 < rep.snapped + rep.refused, "{rep:?}");
        // Conforming: every face belongs to one tetrahedron (boundary) or two (interior), never three, and the boundary
        // faces are exactly the named surfaces.
        let conn = &mesh.blocks[0].conn;
        let mut faces: HashMap<[usize; 3], usize> = HashMap::new();
        for t in conn.chunks(4) {
            assert!(tet_volume(&mesh, t) > 0.0, "every tetrahedron is positively oriented");
            for f in [[t[1], t[2], t[3]], [t[0], t[2], t[3]], [t[0], t[1], t[3]], [t[0], t[1], t[2]]] {
                let mut k = f;
                k.sort_unstable();
                *faces.entry(k).or_default() += 1;
            }
        }
        assert!(faces.values().all(|&c| c == 1 || c == 2), "a hanging or doubled face");
        let outer = faces.values().filter(|&&c| c == 1).count();
        let named: usize = mesh.surfaces.values().map(Vec::len).sum();
        assert_eq!(outer, named);
        // Boundary vertices lie on the sphere.
        let on: Vec<f64> = mesh.surfaces.values().flatten().flatten().map(|&n| (mesh.nodes[n][0].powi(2) + mesh.nodes[n][1].powi(2) + mesh.nodes[n][2].powi(2)).sqrt()).collect();
        let worst = on.iter().map(|r| (r - 1.0).abs()).fold(0.0, f64::max);
        assert!(worst < 0.5 * h, "snapped to the surface within a fraction of an element: {worst}");
    }
}

#[test]
fn the_size_field_grades_the_mesh_and_tet10_boundary_nodes_lie_on_the_surface() {
    let sdf = cube_sdf(0.0, 4.0);
    let (lo, hi) = ([0.0; 3], [4.0; 3]);
    let face = box_face(lo, hi);
    // Fine near the origin corner, coarse elsewhere.
    let size = |x: [f64; 3]| 0.15 + 0.5 * (x[0] * x[0] + x[1] * x[1] + x[2] * x[2]).sqrt();
    let (mesh, rep) = mesh_solid(&solid_box(lo, hi, &sdf, &size, &face), Elastic::new(1.0, 0.3), TetOptions { base: 1.0, boundary_size: 1.5, ..Default::default() }).unwrap();
    assert!((rep.volume - 64.0).abs() < 1e-9);
    let count = |c: [f64; 3], r: f64| mesh.nodes.iter().filter(|x| ((x[0] - c[0]).powi(2) + (x[1] - c[1]).powi(2) + (x[2] - c[2]).powi(2)).sqrt() < r).count();
    let near = count([0.0; 3], 1.0);
    let far = count([4.0; 3], 1.0);
    assert!(near > 8 * far.max(1), "finer near the origin: {near} nodes vs {far} at the far corner");
    // Quadratic sphere: the midside nodes of boundary edges are on the surface.
    let ssdf = |x: [f64; 3]| (x[0] * x[0] + x[1] * x[1] + x[2] * x[2]).sqrt() - 1.0;
    let (slo, shi) = ([-1.2; 3], [1.2; 3]);
    let (ssize, sface) = (|_: [f64; 3]| 10.0, |_: [f64; 3]| 0usize);
    let (m, _) = mesh_solid(&solid_box(slo, shi, &ssdf, &ssize, &sface), Elastic::new(1.0, 0.3), TetOptions { base: 0.6, boundary_size: 0.3, quadratic: true, ..Default::default() }).unwrap();
    assert_eq!(m.blocks[0].kind, ElementKind::Tet10);
    for f in m.surfaces.values().flatten() {
        assert_eq!(f.len(), 6, "Tri6 faces for Tet10");
        for &n in f {
            let r = (m.nodes[n][0].powi(2) + m.nodes[n][1].powi(2) + m.nodes[n][2].powi(2)).sqrt();
            assert!((r - 1.0).abs() < 0.12, "a boundary node near the sphere: {r}");
        }
    }
}

#[test]
fn a_bar_meshed_from_a_distance_function_passes_the_uniaxial_patch_test() {
    // 1 x 1 x 3 bar, held at z = 0 (symmetry conditions on x = 0 and y = 0), pulled by a traction on z = 3: the strain is
    // uniform, so the elongation is exactly sigma L / E whatever the mesh (a patch test for the whole pipeline).
    let (e, nu, sigma, len) = (200e3, 0.3, 50.0, 3.0);
    let (lo, hi) = ([0.0, 0.0, 0.0], [1.0, 1.0, len]);
    let sdf = move |x: [f64; 3]| {
        let d = [(x[0] - 0.5).abs() - 0.5, (x[1] - 0.5).abs() - 0.5, (x[2] - len / 2.0).abs() - len / 2.0];
        d[0].max(d[1]).max(d[2])
    };
    let (size, face) = (|_: [f64; 3]| 0.6, box_face(lo, hi));
    for quadratic in [false, true] {
        let (mesh, _) = mesh_solid(&solid_box(lo, hi, &sdf, &size, &face), Elastic::new(e, nu), TetOptions { base: 0.5, boundary_size: 0.5, quadratic, ..Default::default() }).unwrap();
        let top = mesh.surfaces.get("face6").cloned().expect("the z = 3 face");
        let model = Model::new(mesh).unwrap();
        let mut bc = model.dirichlet();
        for (n, x) in model.mesh.nodes.iter().enumerate() {
            if x[0].abs() < 1e-9 {
                bc.fix(n, 0, 0.0);
            }
            if x[1].abs() < 1e-9 {
                bc.fix(n, 1, 0.0);
            }
            if x[2].abs() < 1e-9 {
                bc.fix(n, 2, 0.0);
            }
        }
        let mut loads = Loads::default();
        for f in top {
            loads.faces.push((f, SurfaceLoad::Traction([0.0, 0.0, sigma])));
        }
        let sol = model.solve_static(&loads, &bc).unwrap();
        let expected = sigma * len / e;
        for (n, x) in model.mesh.nodes.iter().enumerate() {
            if (x[2] - len).abs() < 1e-9 {
                let uz = sol.u[n * 3 + 2];
                assert!((uz - expected).abs() < 1e-8 * expected.max(1.0) + 1e-12, "quadratic {quadratic}: uz {uz} vs {expected}");
            }
        }
    }
}

#[test]
fn impossible_input_is_refused_with_a_reason() {
    let sdf = |_: [f64; 3]| 1.0; // nothing inside
    let (size, face) = (|_: [f64; 3]| 1.0, |_: [f64; 3]| 0usize);
    let err = mesh_solid(&solid_box([0.0; 3], [1.0; 3], &sdf, &size, &face), Elastic::new(1.0, 0.3), TetOptions::default()).unwrap_err();
    assert!(err.contains("no tetrahedron"), "{err}");
    let err = mesh_solid(&solid_box([0.0; 3], [1.0; 3], &cube_sdf(0.0, 1.0), &size, &face), Elastic::new(1.0, 0.3), TetOptions { base: 0.0, ..Default::default() }).unwrap_err();
    assert!(err.contains("positive"), "{err}");
    let tiny = |_: [f64; 3]| 0.001;
    let err = mesh_solid(&solid_box([0.0; 3], [1.0; 3], &cube_sdf(0.0, 1.0), &tiny, &face), Elastic::new(1.0, 0.3), TetOptions { max_tets: 5_000, ..Default::default() }).unwrap_err();
    assert!(err.contains("too fine"), "{err}");
}

#[test]
fn a_thick_cylinder_meshed_from_a_distance_function_matches_the_lame_solution() {
    // Quarter of a thick cylinder (a = 1, b = 2, length 0.5) under internal pressure with the ends held (plane strain):
    // the radial displacement is the Lame solution, so the curved boundary and the whole pipeline are tested together.
    let (e, nu, a, b, p, len) = (200e3, 0.3, 1.0, 2.0, 100.0, 0.5);
    let sdf = move |x: [f64; 3]| {
        let r = x[0].hypot(x[1]);
        (r - b).max(a - r).max(-x[0]).max(-x[1]).max((x[2] - len / 2.0).abs() - len / 2.0)
    };
    // Which face a surface point belongs to: inner wall, outer wall, x = 0, y = 0, z = 0, z = len.
    let face = move |x: [f64; 3]| {
        let r = x[0].hypot(x[1]);
        let d = [(r - a).abs(), (r - b).abs(), x[0].abs(), x[1].abs(), x[2].abs(), (x[2] - len).abs()];
        (0..6).min_by(|&i, &j| d[i].total_cmp(&d[j])).unwrap()
    };
    let faces = move |x: [f64; 3]| {
        let r = x[0].hypot(x[1]).max(1e-12);
        let radial = [x[0] / r, x[1] / r, 0.0];
        vec![
            FaceHit { face: 0, dist: a - r, normal: [-radial[0], -radial[1], 0.0] },
            FaceHit { face: 1, dist: r - b, normal: radial },
            FaceHit { face: 2, dist: -x[0], normal: [-1.0, 0.0, 0.0] },
            FaceHit { face: 3, dist: -x[1], normal: [0.0, -1.0, 0.0] },
            FaceHit { face: 4, dist: -x[2], normal: [0.0, 0.0, -1.0] },
            FaceHit { face: 5, dist: x[2] - len, normal: [0.0, 0.0, 1.0] },
        ]
    };
    let size = |_: [f64; 3]| 10.0;
    let mut previous = f64::INFINITY;
    for h in [0.2, 0.1] {
        let solid = Solid { lo: [0.0, 0.0, 0.0], hi: [2.05, 2.05, len], sdf: &sdf, size: &size, face_of: &face, face_dists: Some(&faces) };
        let (mesh, rep) = mesh_solid(&solid, Elastic::new(e, nu), TetOptions { base: 0.5, boundary_size: h, quadratic: true, ..Default::default() }).unwrap();
        let inner = mesh.surfaces.get("face1").cloned().expect("the inner wall");
        let model = Model::new(mesh).unwrap();
        let mut bc = model.dirichlet();
        for (n, x) in model.mesh.nodes.iter().enumerate() {
            if x[0].abs() < 1e-6 {
                bc.fix(n, 0, 0.0);
            }
            if x[1].abs() < 1e-6 {
                bc.fix(n, 1, 0.0);
            }
            if x[2].abs() < 1e-6 || (x[2] - len).abs() < 1e-6 {
                bc.fix(n, 2, 0.0);
            }
        }
        let mut loads = Loads::default();
        for f in inner {
            loads.faces.push((f, SurfaceLoad::Pressure(p)));
        }
        let sol = model.solve_static(&loads, &bc).unwrap();
        let (aa, bb) = (p * a * a / (b * b - a * a), p * a * a * b * b / (b * b - a * a));
        let ur = |r: f64| (1.0 + nu) / e * ((1.0 - 2.0 * nu) * aa * r + bb / r);
        let mut worst = 0.0f64;
        for (n, x) in model.mesh.nodes.iter().enumerate() {
            let r = x[0].hypot(x[1]);
            if r < 1e-9 {
                continue;
            }
            let u = (sol.u[n * 3] * x[0] + sol.u[n * 3 + 1] * x[1]) / r;
            worst = worst.max((u - ur(r)).abs() / ur(a));
        }
        eprintln!("h {h}: {} tets, worst radial displacement error {worst:.2e}, quality {:.3}, snapped {} refused {}", rep.tets, rep.worst_quality, rep.snapped, rep.refused);
        assert!(worst < previous, "refining improves the answer: {worst} after {previous}");
        previous = worst;
    }
    assert!(previous < 0.02, "the finest mesh is within 2% of the Lame solution: {previous}");
}

#[test]
fn error_driven_remeshing_puts_the_elements_where_the_stress_gradient_is() {
    // The thick cylinder again, started coarse. The ZZ error of the first solve sizes the second mesh: it must have more
    // elements near the bore than near the outer wall (stress is highest at the bore), and the radial displacement error
    // must not get worse for it.
    use fea_core::adapt::{adapted_size_field_3d, AdaptOptions};
    let (e, nu, a, b, p, len) = (200e3, 0.3, 1.0, 2.0, 100.0, 0.5);
    let sdf = move |x: [f64; 3]| (x[0].hypot(x[1]) - b).max(a - x[0].hypot(x[1])).max(-x[0]).max(-x[1]).max((x[2] - len / 2.0).abs() - len / 2.0);
    let face = move |x: [f64; 3]| {
        let r = x[0].hypot(x[1]);
        let d = [(r - a).abs(), (r - b).abs(), x[0].abs(), x[1].abs(), x[2].abs(), (x[2] - len).abs()];
        (0..6).min_by(|&i, &j| d[i].total_cmp(&d[j])).unwrap()
    };
    let faces = move |x: [f64; 3]| {
        let r = x[0].hypot(x[1]).max(1e-12);
        let radial = [x[0] / r, x[1] / r, 0.0];
        vec![
            FaceHit { face: 0, dist: a - r, normal: [-radial[0], -radial[1], 0.0] },
            FaceHit { face: 1, dist: r - b, normal: radial },
            FaceHit { face: 2, dist: -x[0], normal: [-1.0, 0.0, 0.0] },
            FaceHit { face: 3, dist: -x[1], normal: [0.0, -1.0, 0.0] },
            FaceHit { face: 4, dist: -x[2], normal: [0.0, 0.0, -1.0] },
            FaceHit { face: 5, dist: x[2] - len, normal: [0.0, 0.0, 1.0] },
        ]
    };
    let (aa, bb) = (p * a * a / (b * b - a * a), p * a * a * b * b / (b * b - a * a));
    let ur = |r: f64| (1.0 + nu) / e * ((1.0 - 2.0 * nu) * aa * r + bb / r);
    let run = |size: &dyn Fn([f64; 3]) -> f64| {
        let solid = Solid { lo: [0.0, 0.0, 0.0], hi: [2.05, 2.05, len], sdf: &sdf, size, face_of: &face, face_dists: Some(&faces) };
        let (mesh, _) = mesh_solid(&solid, Elastic::new(e, nu), TetOptions { base: 0.5, boundary_size: 0.5, quadratic: true, ..Default::default() }).unwrap();
        let inner = mesh.surfaces.get("face1").cloned().unwrap();
        let model = Model::new(mesh).unwrap();
        let mut bc = model.dirichlet();
        // Symmetry planes are bound by face name, like a user would: x = 0 (face3), y = 0 (face4), z = 0 / len (face5, face6).
        for (name, comp) in [("face3", 0), ("face4", 1), ("face5", 2), ("face6", 2)] {
            for f in model.mesh.surfaces.get(name).into_iter().flatten() {
                for &n in f.iter() {
                    bc.fix(n, comp, 0.0);
                }
            }
        }
        let mut loads = Loads::default();
        for f in inner {
            loads.faces.push((f, SurfaceLoad::Pressure(p)));
        }
        let sol = model.solve_static(&loads, &bc).unwrap();
        let mut worst = 0.0f64;
        for (n, x) in model.mesh.nodes.iter().enumerate() {
            let r = x[0].hypot(x[1]);
            if r > 1e-9 {
                let d = ((sol.u[n * 3] * x[0] + sol.u[n * 3 + 1] * x[1]) / r - ur(r)).abs() / ur(a);
                worst = worst.max(d);
            }
        }
        let zz = model.zz_error(&sol.u, 0.0).unwrap();
        (model, zz, worst)
    };
    let (m0, zz0, err0) = run(&|_| 0.5);
    let field = adapted_size_field_3d(&m0.mesh, &zz0, &AdaptOptions { target_rel_error: 0.01, order: 2.0, grading: 0.5, h_min: 0.05, h_max: 0.5, ..Default::default() }).unwrap();
    let size = |x: [f64; 3]| field.at(x);
    let (m1, zz1, err1) = run(&size);
    // Elements by radius of their centroid.
    let count = |m: &Model| {
        let (mut inner, mut outer) = (0usize, 0usize);
        for blk in &m.mesh.blocks {
            for c in blk.conn.chunks_exact(blk.kind.n_nodes()) {
                let (mut x, mut y) = (0.0, 0.0);
                for &v in &c[..4] {
                    x += m.mesh.nodes[v][0] / 4.0;
                    y += m.mesh.nodes[v][1] / 4.0;
                }
                if x.hypot(y) < 1.5 {
                    inner += 1;
                } else {
                    outer += 1;
                }
            }
        }
        (inner, outer)
    };
    let (i0, o0) = count(&m0);
    let (i1, o1) = count(&m1);
    let (rel0, rel1) = (zz0.total / zz0.norm, zz1.total / zz1.norm);
    eprintln!("pass 0: {i0}/{o0} err {err0:.2e} zz {rel0:.2e}; pass 1: {i1}/{o1} err {err1:.2e} zz {rel1:.2e}");
    assert!(rel1 < 0.5 * rel0, "the error estimate falls by at least half: {rel0} -> {rel1}");
    assert!(i1 as f64 / o1 as f64 > 1.5 * (i0 as f64 / o0 as f64), "refinement follows the error: {i0}/{o0} -> {i1}/{o1}");
    assert!(err1 < err0, "the adapted mesh is more accurate: {err0} -> {err1}");
}

#[test]
fn a_fine_size_field_under_a_coarse_boundary_size_does_not_pile_vertices_onto_one_corner() {
    // Regression: the snapping reach used to come from `boundary_size` alone, so where the size field asked for elements far
    // smaller than that, several vertices snapped onto the same box corner and the mesh had coincident, unconnected nodes.
    let (lo, hi) = ([0.0; 3], [1.0; 3]);
    let sdf = cube_sdf(0.0, 1.0);
    let (size, face) = (|_: [f64; 3]| 0.1, box_face(lo, hi));
    let (mesh, rep) = mesh_solid(&solid_box(lo, hi, &sdf, &size, &face), Elastic::new(1.0, 0.3), TetOptions { base: 0.5, boundary_size: 1.0, ..Default::default() }).unwrap();
    let mut seen: HashMap<[i64; 3], usize> = HashMap::new();
    for x in &mesh.nodes {
        *seen.entry([(x[0] * 1e7).round() as i64, (x[1] * 1e7).round() as i64, (x[2] * 1e7).round() as i64]).or_default() += 1;
    }
    assert!(seen.values().all(|&c| c == 1), "coincident nodes: {}", seen.values().filter(|&&c| c > 1).count());
    assert!((rep.volume - 1.0).abs() < 1e-6, "{rep:?}");
}

#[test]
fn the_3d_size_field_reproduces_a_linear_field_and_clamps_outside() {
    use fea_core::adapt::SizeField3;
    // One unit tetrahedron with h = 1 + x + 2y + 3z at the corners.
    let pts = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    let f = |x: [f64; 3]| 1.0 + x[0] + 2.0 * x[1] + 3.0 * x[2];
    let field = SizeField3::new(pts.clone(), vec![[0, 1, 2, 3]], pts.iter().map(|&p| f(p)).collect());
    for x in [[0.2, 0.2, 0.2], [0.1, 0.5, 0.3], [0.0, 0.0, 0.0]] {
        assert!((field.at(x) - f(x)).abs() < 1e-12, "{x:?}");
    }
    // Slightly outside it uses clamped weights (a finite value between the corner sizes); far outside, the nearest corner.
    let near = field.at([0.4, 0.4, 0.4]);
    assert!((1.0..=7.0).contains(&near), "{near}");
    assert_eq!(field.at([100.0, 0.0, 0.0]), f([1.0, 0.0, 0.0]));
}
