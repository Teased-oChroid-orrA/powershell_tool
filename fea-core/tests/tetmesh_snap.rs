//! Boundary snapping of the signed-distance tetrahedral mesher: the boundary layer is reconnected between snap rounds, so
//! (almost) every boundary vertex ends on the surface. Before the reconnection, 96 / 99 / 192 boundary vertices of the unit
//! sphere (h = 0.5 / 0.3 / 0.2) stayed off it because boundary-layer slivers made their snap steps unacceptable.

use fea_core::tetmesh::{mesh_solid, Solid, TetOptions};
use fea_core::Elastic;

fn sphere(h: f64) -> (fea_core::Mesh, fea_core::tetmesh::MeshReport) {
    let sdf = |x: [f64; 3]| (x[0] * x[0] + x[1] * x[1] + x[2] * x[2]).sqrt() - 1.0;
    let (size, face) = (|_: [f64; 3]| 10.0, |_: [f64; 3]| 0usize);
    let solid = Solid {
        lo: [-1.2; 3],
        hi: [1.2; 3],
        sdf: &sdf,
        size: &size,
        face_of: &face,
        face_dists: None,
    };
    mesh_solid(
        &solid,
        Elastic::new(1.0, 0.3),
        TetOptions {
            base: 0.6,
            boundary_size: h,
            quadratic: false,
            ..Default::default()
        },
    )
    .unwrap()
}

#[test]
fn nearly_every_boundary_vertex_of_a_sphere_ends_on_the_surface() {
    for h in [0.5, 0.3, 0.2] {
        let (mesh, rep) = sphere(h);
        let total = rep.snapped + rep.refused;
        eprintln!(
            "h {h}: snapped {} refused {} worst quality {:.3}, {} tets",
            rep.snapped, rep.refused, rep.worst_quality, rep.tets
        );
        assert!(total > 100, "h {h}: {rep:?}");
        // At most 5 % may stay off (it was 33 %, 19 % and 10 % before the reconnection).
        assert!(
            rep.refused * 20 <= total,
            "h {h}: {} of {} boundary vertices off the surface",
            rep.refused,
            total
        );
        // The quality floor of the snapping is still honoured and nothing is inverted.
        assert!(
            rep.worst_quality >= 0.05 - 1e-12,
            "h {h}: {}",
            rep.worst_quality
        );
        let q = mesh.quality();
        assert_eq!(q.inverted, 0, "h {h}");
        // The mesh encloses (nearly) the ball.
        let exact = 4.0 / 3.0 * std::f64::consts::PI;
        assert!(
            (rep.volume - exact).abs() / exact < 0.1,
            "h {h}: volume {}",
            rep.volume
        );
    }
}

#[test]
fn the_meshing_is_deterministic() {
    let (a, ra) = sphere(0.4);
    let (b, rb) = sphere(0.4);
    assert_eq!(a.nodes, b.nodes);
    assert_eq!(a.blocks[0].conn, b.blocks[0].conn);
    assert_eq!((ra.snapped, ra.refused), (rb.snapped, rb.refused));
}

#[test]
fn named_surface_faces_point_out_of_the_solid() {
    // The outward orientation of a named face comes from the positively oriented tetrahedron behind it, so reconnecting the
    // boundary layer must not change it (a stale interior-side vertex once flipped about half of the thick-cylinder bore
    // faces and the pressure load changed sign there). A face that lies on the sphere itself can still face the axis when the
    // thin tetrahedron behind it is a cap sliver lying outside its chord plane; those are counted, not hidden (36 of 1044 at h = 0.3, identical with the reconnection switched off: a pre-existing property of the
    // quality floor, bounded here at 5 %).
    for h in [0.5, 0.3, 0.2] {
        let (mesh, _) = sphere(h);
        let faces = mesh.surfaces.get("face1").expect("the sphere surface");
        assert!(faces.len() > 50, "h {h}: {} faces", faces.len());
        let mut inward = 0;
        for f in faces {
            let (a, b, c) = (mesh.nodes[f[0]], mesh.nodes[f[1]], mesh.nodes[f[2]]);
            let (u, v) = (
                [b[0] - a[0], b[1] - a[1], b[2] - a[2]],
                [c[0] - a[0], c[1] - a[1], c[2] - a[2]],
            );
            let n = [
                u[1] * v[2] - u[2] * v[1],
                u[2] * v[0] - u[0] * v[2],
                u[0] * v[1] - u[1] * v[0],
            ];
            let centre = [
                (a[0] + b[0] + c[0]) / 3.,
                (a[1] + b[1] + c[1]) / 3.,
                (a[2] + b[2] + c[2]) / 3.,
            ];
            if n[0] * centre[0] + n[1] * centre[1] + n[2] * centre[2] <= 0. {
                inward += 1;
            }
        }
        eprintln!(
            "h {h}: {inward} of {} sphere faces face the axis",
            faces.len()
        );
        assert!(
            inward * 20 <= faces.len(),
            "h {h}: {inward} of {} faces point into the solid",
            faces.len()
        );
    }
}
