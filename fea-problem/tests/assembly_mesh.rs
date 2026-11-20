//! Multi-body viewer mesh (epic Phase B): every placed body sampled in the root frame, triangles tagged `(body, STEP face id)`.
use fea_problem::assembly_advice::recommend_step;
use fea_problem::assembly_mesh::{assembly_mesh, AssemblyMesh};
use solid_kernel::native_brep::NativeBrep;

fn fixture() -> String {
    std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../solid-kernel/tests/data/assembly-two-instances.step")).unwrap()
}

/// Signed volume and bounding box of one body's triangles.
fn body(m: &AssemblyMesh, b: usize) -> (f64, [f64; 3], [f64; 3]) {
    let (mut vol, mut lo, mut hi) = (0.0, [f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
    for (t, _) in m.triangles.iter().zip(&m.face).filter(|(_, k)| k.body == b) {
        let [a, p, q] = [m.vertices[t[0]], m.vertices[t[1]], m.vertices[t[2]]];
        vol += (a[0] * (p[1] * q[2] - p[2] * q[1]) - a[1] * (p[0] * q[2] - p[2] * q[0]) + a[2] * (p[0] * q[1] - p[1] * q[0])) / 6.0;
        for v in [a, p, q] {
            for i in 0..3 {
                lo[i] = lo[i].min(v[i]);
                hi[i] = hi[i].max(v[i]);
            }
        }
    }
    (vol, lo, hi)
}

#[test]
fn an_assembly_meshes_every_body_in_the_root_frame_with_face_keys() {
    let m = assembly_mesh(&fixture(), 0.0).unwrap();
    assert_eq!(m.bodies(), 2);
    assert_eq!(m.chord_mm, 0.05);
    assert_eq!(m.triangles.len(), m.face.len());
    assert_eq!(m.body_triangles.iter().sum::<usize>(), m.triangles.len());
    // stored body by body, every index inside the vertex list
    assert!(m.face.windows(2).all(|w| w[0].body <= w[1].body));
    assert!(m.triangles.iter().all(|t| t.iter().all(|&i| i < m.vertices.len())));
    // the same solid placed twice: equal closed volumes, but not in the same place
    let (v0, lo0, hi0) = body(&m, 0);
    let (v1, lo1, hi1) = body(&m, 1);
    assert!(v0 > 0.0 && (v0 - v1).abs() <= 1e-9 * v0, "{v0} {v1}");
    assert!(lo0 != lo1 || hi0 != hi1, "both bodies sit at {lo0:?}");
    // equal construction: the same face ids repeat in both bodies, kept apart by the body index
    let faces = |b: usize| m.face.iter().filter(|k| k.body == b).map(|k| k.face).collect::<std::collections::BTreeSet<_>>();
    assert_eq!(faces(0), faces(1));
    assert_eq!(faces(0).len(), 6);
}

#[test]
fn every_advice_face_has_triangles_in_the_mesh() {
    let step = fixture();
    let m = assembly_mesh(&step, 0.05).unwrap();
    for item in recommend_step(&step).unwrap() {
        let faces = item.faces();
        assert!(!m.triangles_of(&faces).is_empty() || faces.is_empty(), "{item:?}");
    }
    assert!(m.triangles_of(&[solid_kernel::features::FaceKey { body: 9, face: 0 }]).is_empty());
}

#[test]
fn one_body_and_bad_text_behave() {
    let single = NativeBrep::box_from_corners([0., 0., 0.], [40., 30., 10.]).unwrap().to_step_text();
    let m = assembly_mesh(&single, 0.05).unwrap();
    assert_eq!((m.bodies(), m.body_triangles[0]), (1, m.triangles.len()));
    assert!((body(&m, 0).0 - 12000.0).abs() < 1e-6);
    assert!(assembly_mesh("not a step file", 0.05).is_err());
}
