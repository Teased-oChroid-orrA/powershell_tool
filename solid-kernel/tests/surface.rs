//! Distance queries, per-face queries and volume meshing on analytic face-tagged tessellations (no OpenCascade).
use fea_core::Elastic;
use solid_kernel::{mesh_volume, SurfaceMesh, SurfaceSolid, VolumeParams, V3};

/// An axis-aligned box [0,w] x [0,h] x [0,d] as 12 triangles, faces 0..5 = x-, x+, y-, y+, z-, z+.
fn box_mesh(w: f64, h: f64, d: f64) -> SurfaceMesh {
    let p = |x: f64, y: f64, z: f64| [x * w, y * h, z * d];
    let mut m = SurfaceMesh::default();
    // Each face as two triangles, counter-clockwise seen from outside.
    let faces: [([V3; 4], usize); 6] = [
        ([p(0.0, 0.0, 0.0), p(0.0, 0.0, 1.0), p(0.0, 1.0, 1.0), p(0.0, 1.0, 0.0)], 0),
        ([p(1.0, 0.0, 0.0), p(1.0, 1.0, 0.0), p(1.0, 1.0, 1.0), p(1.0, 0.0, 1.0)], 1),
        ([p(0.0, 0.0, 0.0), p(1.0, 0.0, 0.0), p(1.0, 0.0, 1.0), p(0.0, 0.0, 1.0)], 2),
        ([p(0.0, 1.0, 0.0), p(0.0, 1.0, 1.0), p(1.0, 1.0, 1.0), p(1.0, 1.0, 0.0)], 3),
        ([p(0.0, 0.0, 0.0), p(0.0, 1.0, 0.0), p(1.0, 1.0, 0.0), p(1.0, 0.0, 0.0)], 4),
        ([p(0.0, 0.0, 1.0), p(1.0, 0.0, 1.0), p(1.0, 1.0, 1.0), p(0.0, 1.0, 1.0)], 5),
    ];
    for (q, f) in faces {
        m.push_triangle(q[0], q[1], q[2], f);
        m.push_triangle(q[0], q[2], q[3], f);
    }
    m
}

/// A tube (r0 <= r <= r1, 0 <= z <= len) with `n` segments: faces 0 outer wall, 1 inner wall, 2 bottom, 3 top.
fn tube_mesh(r0: f64, r1: f64, len: f64, n: usize) -> SurfaceMesh {
    let mut m = SurfaceMesh::default();
    let pt = |r: f64, i: usize, z: f64| {
        let t = std::f64::consts::TAU * (i % n) as f64 / n as f64;
        [r * t.cos(), r * t.sin(), z]
    };
    for i in 0..n {
        // Outer wall (outward normal +r), inner wall (outward normal -r), caps.
        m.push_triangle(pt(r1, i, 0.0), pt(r1, i + 1, 0.0), pt(r1, i + 1, len), 0);
        m.push_triangle(pt(r1, i, 0.0), pt(r1, i + 1, len), pt(r1, i, len), 0);
        m.push_triangle(pt(r0, i, 0.0), pt(r0, i + 1, len), pt(r0, i + 1, 0.0), 1);
        m.push_triangle(pt(r0, i, 0.0), pt(r0, i, len), pt(r0, i + 1, len), 1);
        m.push_triangle(pt(r0, i, 0.0), pt(r0, i + 1, 0.0), pt(r1, i + 1, 0.0), 2);
        m.push_triangle(pt(r0, i, 0.0), pt(r1, i + 1, 0.0), pt(r1, i, 0.0), 2);
        m.push_triangle(pt(r0, i, len), pt(r1, i + 1, len), pt(r0, i + 1, len), 3);
        m.push_triangle(pt(r0, i, len), pt(r1, i, len), pt(r1, i + 1, len), 3);
    }
    m
}

#[test]
fn the_signed_distance_of_a_box_is_exact_inside_on_faces_edges_and_corners() {
    let s = SurfaceSolid::new(box_mesh(2.0, 2.0, 2.0), 1e-9).unwrap();
    assert!((s.volume() - 8.0).abs() < 1e-12 && s.face_count() == 6);
    let d = |p: V3| s.signed_distance(p);
    assert!((d([1.0, 1.0, 1.0]) + 1.0).abs() < 1e-12, "the centre is 1 inside");
    assert!((d([3.0, 1.0, 1.0]) - 1.0).abs() < 1e-12, "1 outside a face");
    assert!((d([3.0, 3.0, 1.0]) - 2f64.sqrt()).abs() < 1e-12, "sqrt 2 outside an edge");
    assert!((d([3.0, 3.0, 3.0]) - 3f64.sqrt()).abs() < 1e-12, "sqrt 3 outside a corner");
    assert!((d([0.1, 1.0, 1.0]) + 0.1).abs() < 1e-12, "0.1 inside a face");
    assert!((d([0.1, 0.1, 1.0]) + 0.1).abs() < 1e-12, "inside near an edge: the nearer face");
    assert!(d([0.0, 1.0, 1.0]).abs() < 1e-12, "on the surface");
    // Many points: the sign agrees with the analytic inside test, and the magnitude with the analytic distance.
    let mut worst: f64 = 0.0;
    for i in 0..40 {
        for j in 0..40 {
            for k in 0..8 {
                let p = [-1.0 + 4.0 * i as f64 / 39.0, -1.0 + 4.0 * j as f64 / 39.0, -1.0 + 4.0 * k as f64 / 7.0];
                let q = [p[0] - 1.0, p[1] - 1.0, p[2] - 1.0];
                let out = [q[0].abs() - 1.0, q[1].abs() - 1.0, q[2].abs() - 1.0];
                let outside = [out[0].max(0.0), out[1].max(0.0), out[2].max(0.0)];
                let exact = (outside[0].powi(2) + outside[1].powi(2) + outside[2].powi(2)).sqrt() + out[0].max(out[1]).max(out[2]).min(0.0);
                worst = worst.max((d(p) - exact).abs());
            }
        }
    }
    assert!(worst < 1e-12, "{worst}");
}

#[test]
fn per_face_distances_name_the_faces_a_point_is_close_to() {
    let s = SurfaceSolid::new(box_mesh(2.0, 2.0, 2.0), 1e-9).unwrap();
    let near = s.faces_near([0.05, 1.0, 1.0], 0.3);
    assert_eq!(near.len(), 1);
    assert_eq!(near[0].face, 0);
    assert!((near[0].dist + 0.05).abs() < 1e-12, "negative inside: {:?}", near[0]);
    assert_eq!(near[0].normal, [-1.0, 0.0, 0.0]);
    let corner: Vec<usize> = s.faces_near([0.05, 0.05, 1.0], 0.3).iter().map(|h| h.face).collect();
    assert_eq!(corner, [0, 2], "an edge point is close to two faces");
    assert_eq!(s.nearest_face([1.9, 1.0, 1.0]), 1);
}

#[test]
fn a_surface_that_is_open_is_refused_and_an_inside_out_one_is_turned_around() {
    let mut open = box_mesh(1.0, 1.0, 1.0);
    open.triangles.pop();
    open.face.pop();
    let err = SurfaceSolid::new(open, 1e-9).err().expect("open surface refused");
    assert!(err.contains("not a closed manifold"), "{err}");
    let mut inside_out = box_mesh(1.0, 1.0, 1.0);
    for t in &mut inside_out.triangles {
        t.swap(1, 2);
    }
    let s = SurfaceSolid::new(inside_out, 1e-9).unwrap();
    assert!((s.volume() - 1.0).abs() < 1e-12 && s.signed_distance([0.5, 0.5, 0.5]) < 0.0);
}

#[test]
fn a_box_is_meshed_from_its_surface_with_named_faces_and_the_exact_volume() {
    let s = SurfaceSolid::new(box_mesh(2.0, 2.0, 2.0), 1e-9).unwrap();
    let (mesh, rep) = mesh_volume(&s, Elastic::new(1.0, 0.3), VolumeParams { base: 0.5, boundary_size: 0.5, quadratic: false }, &|_| 1.0).unwrap();
    assert!((rep.volume - 8.0).abs() < 1e-9, "{rep:?}");
    let names: Vec<&String> = mesh.surfaces.keys().collect();
    assert_eq!(names.len(), 6, "{names:?}");
    assert!(rep.worst_quality > 0.15, "{rep:?}");
}

#[test]
fn a_tube_is_meshed_to_its_volume_and_its_four_faces_keep_their_area() {
    let (r0, r1, len) = (1.0, 2.0, 0.5);
    let s = SurfaceSolid::new(tube_mesh(r0, r1, len, 96), 1e-9).unwrap();
    let exact = std::f64::consts::PI * (r1 * r1 - r0 * r0) * len;
    assert!((s.volume() - exact).abs() / exact < 2e-3, "tessellated volume {} vs {exact}", s.volume());
    assert_eq!(s.face_count(), 4);
    let areas = s.face_areas();
    assert!((areas[0] - std::f64::consts::TAU * r1 * len).abs() / areas[0] < 2e-3);
    assert!((areas[2] - std::f64::consts::PI * (r1 * r1 - r0 * r0)).abs() / areas[2] < 2e-3);
    for h in [0.3, 0.2] {
        let (mesh, rep) = mesh_volume(&s, Elastic::new(1.0, 0.3), VolumeParams { base: 0.6, boundary_size: h, quadratic: false }, &|_| 10.0).unwrap();
        let err = (rep.volume - exact).abs() / exact;
        eprintln!("h {h}: {} tets, volume error {err:.3e}, quality {:.3}", rep.tets, rep.worst_quality);
        assert!(err < 0.015, "the meshed volume is within 1.5%: {err}");
        assert_eq!(mesh.surfaces.len(), 4, "outer wall, inner wall and two caps: {:?}", mesh.surfaces.keys().collect::<Vec<_>>());
    }
}

#[test]
fn a_t_junction_between_faces_is_repaired_so_the_surface_closes() {
    // The bottom face is triangulated with an extra vertex in the middle of its y = 0 edge; the side face on that edge
    // keeps the single long edge. Without the repair the surface is open there.
    let mut m = box_mesh(2.0, 2.0, 2.0);
    // Faces 4 is z = 0: replace its two triangles by a fan around the edge midpoint (1, 0, 0).
    let keep: Vec<usize> = (0..m.triangles.len()).filter(|&t| m.face[t] != 4).collect();
    let (tris, faces) = (keep.iter().map(|&t| m.triangles[t]).collect::<Vec<_>>(), keep.iter().map(|&t| m.face[t]).collect::<Vec<_>>());
    m.triangles = tris;
    m.face = faces;
    // Counter-clockwise seen from below (outside): (0,0,0) (0,2,0) (2,2,0) (2,0,0), with (1,0,0) on the last-to-first edge.
    let (a, b, c, d, mid) = ([0.0, 0.0, 0.0], [0.0, 2.0, 0.0], [2.0, 2.0, 0.0], [2.0, 0.0, 0.0], [1.0, 0.0, 0.0]);
    m.push_triangle(a, b, c, 4);
    m.push_triangle(a, c, mid, 4);
    m.push_triangle(mid, c, d, 4);
    let s = SurfaceSolid::new(m, 1e-9).expect("the T-junction is repaired");
    assert!((s.volume() - 8.0).abs() < 1e-12);
    assert!(s.signed_distance([1.0, 1.0, 1.0]) < 0.0);
}

#[test]
fn welding_uses_distance_across_grid_boundaries_and_volume_is_translation_invariant() {
    let weld=1e-5;
    let mut mesh=box_mesh(1.0,1.0,1.0);
    for (i,p) in mesh.vertices.iter_mut().enumerate() {
        for x in p { *x += if i%2==0 { 0.49*weld } else { 0.51*weld }; }
    }
    let solid=SurfaceSolid::new(mesh,weld).unwrap();
    assert!((solid.volume()-1.0).abs()<1e-6);
    let mut shifted=box_mesh(1.0,1.0,1.0);
    for p in &mut shifted.vertices { for x in p { *x+=1.0e9; } }
    let solid=SurfaceSolid::new(shifted,1e-6).unwrap();
    assert!((solid.volume()-1.0).abs()<1e-12);
    assert!(solid.signed_distance([1e9+0.5;3])<0.0);
}
