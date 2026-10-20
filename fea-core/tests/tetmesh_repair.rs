//! Boundary-layer repair of the signed-distance tetrahedral mesher (`meshing::boundary_repair`, Phase 125).
//!
//! The snapped lattice mesh of a curved solid has 0.2 to 5 % of its tetrahedra under the solver gate's scaled Jacobian of 0.1,
//! all of them touching the boundary. These tests mesh a sphere, a capped cylinder (with model faces, so its rims are features)
//! and a torus at sizes where that happens, and require: every element passes the gate, nothing is inverted, the element count
//! grows by at most 10 %, the enclosed volume and the surface fidelity do not degrade, and the face names and their areas are
//! unchanged. The same meshes without the repair (`repair: false`) must still show the original failure, so the tests fail on
//! the old code.
use fea_core::meshing::quality::QualityThresholds;
use fea_core::tetmesh::{mesh_solid_reported, FaceHit, MeshReport, Solid, TetOptions};
use fea_core::{Elastic, Mesh};
use std::collections::BTreeMap;

type V3 = [f64; 3];
type Fd<'a> = Option<&'a dyn Fn(V3) -> Vec<FaceHit>>;

const GATE: f64 = 0.1;

fn mesh(lo: V3, hi: V3, sdf: &dyn Fn(V3) -> f64, fd: Fd, h: f64, quadratic: bool, repair: bool) -> (Mesh, MeshReport) {
    let (size, face) = (|_: V3| 10.0, |_: V3| 0usize);
    let solid = Solid { lo, hi, sdf, size: &size, face_of: &face, face_dists: fd };
    let (m, r, _) = mesh_solid_reported(&solid, Elastic::new(1.0, 0.3), TetOptions { base: 0.6, boundary_size: h, quadratic, repair, ..Default::default() }).unwrap();
    (m, r)
}

fn below_gate(m: &Mesh) -> usize {
    m.quality().elements.iter().filter(|e| e.scaled_jacobian < GATE).count()
}

/// Area of each named face.
fn areas(m: &Mesh) -> BTreeMap<String, f64> {
    m.surfaces
        .iter()
        .map(|(name, faces)| {
            let a: f64 = faces
                .iter()
                .map(|f| {
                    let (p, q, r) = (m.nodes[f[0]], m.nodes[f[1]], m.nodes[f[2]]);
                    let (u, v) = ([q[0] - p[0], q[1] - p[1], q[2] - p[2]], [r[0] - p[0], r[1] - p[1], r[2] - p[2]]);
                    let c = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
                    0.5 * (c[0] * c[0] + c[1] * c[1] + c[2] * c[2]).sqrt()
                })
                .sum();
            (name.clone(), a)
        })
        .collect()
}

fn sphere(x: V3) -> f64 {
    (x[0] * x[0] + x[1] * x[1] + x[2] * x[2]).sqrt() - 1.0
}
fn cylinder(x: V3) -> f64 {
    ((x[0] * x[0] + x[1] * x[1]).sqrt() - 1.0).max(x[2].abs() - 1.0)
}
fn cylinder_faces(x: V3) -> Vec<FaceHit> {
    let r = (x[0] * x[0] + x[1] * x[1]).sqrt().max(1e-12);
    let mut v = vec![
        FaceHit { face: 0, dist: r - 1.0, normal: [x[0] / r, x[1] / r, 0.0] },
        FaceHit { face: 1, dist: x[2] - 1.0, normal: [0.0, 0.0, 1.0] },
        FaceHit { face: 2, dist: -x[2] - 1.0, normal: [0.0, 0.0, -1.0] },
    ];
    v.retain(|h| h.dist.abs() < 0.5);
    v
}
fn torus(x: V3) -> f64 {
    ((x[0] * x[0] + x[1] * x[1]).sqrt() - 1.0).hypot(x[2]) - 0.4
}

struct Case {
    name: &'static str,
    lo: V3,
    hi: V3,
    sdf: fn(V3) -> f64,
    faces: Option<fn(V3) -> Vec<FaceHit>>,
    h: f64,
    /// The unrepaired mesh is known to have elements under the gate here.
    broken_before: bool,
}

fn cases() -> Vec<Case> {
    let s = [-1.2; 3];
    let t = [1.2; 3];
    let c = ([-1.0; 3], [1.0; 3]);
    let to = ([-1.4, -1.4, -0.4], [1.4, 1.4, 0.4]);
    vec![
        Case { name: "sphere h0.3", lo: s, hi: t, sdf: sphere, faces: None, h: 0.3, broken_before: true },
        Case { name: "sphere h0.2", lo: s, hi: t, sdf: sphere, faces: None, h: 0.2, broken_before: true },
        Case { name: "cylinder h0.3", lo: c.0, hi: c.1, sdf: cylinder, faces: Some(cylinder_faces), h: 0.3, broken_before: true },
        Case { name: "cylinder h0.2", lo: c.0, hi: c.1, sdf: cylinder, faces: Some(cylinder_faces), h: 0.2, broken_before: true },
        Case { name: "torus h0.5", lo: to.0, hi: to.1, sdf: torus, faces: None, h: 0.5, broken_before: true },
        Case { name: "torus h0.3", lo: to.0, hi: to.1, sdf: torus, faces: None, h: 0.3, broken_before: true },
        Case { name: "torus h0.2", lo: to.0, hi: to.1, sdf: torus, faces: None, h: 0.2, broken_before: true },
        Case { name: "sphere h0.5 (already fine)", lo: s, hi: t, sdf: sphere, faces: None, h: 0.5, broken_before: false },
    ]
}

#[test]
fn every_element_of_a_curved_solid_passes_the_gate_after_the_repair() {
    for c in cases() {
        let fd = c.faces.as_ref().map(|f| f as &dyn Fn(V3) -> Vec<FaceHit>);
        let (off, off_rep) = mesh(c.lo, c.hi, &c.sdf, fd, c.h, false, false);
        let (on, on_rep) = mesh(c.lo, c.hi, &c.sdf, fd, c.h, false, true);
        let bad_off = below_gate(&off);
        if c.broken_before {
            assert!(bad_off > 0, "{}: the unrepaired mesh was expected to have elements under the gate", c.name);
        }
        let q = on.quality();
        assert_eq!(q.inverted, 0, "{}: inverted elements", c.name);
        assert_eq!(below_gate(&on), 0, "{}: {} of {} elements still under the gate (minimum {:.4}; unrepaired {} of {}, minimum {:.4})", c.name, below_gate(&on), on_rep.tets, q.min_scaled_jacobian, bad_off, off_rep.tets, off.quality().min_scaled_jacobian);
        assert!(on_rep.tets as f64 <= 1.10 * off_rep.tets as f64, "{}: {} against {} elements", c.name, on_rep.tets, off_rep.tets);
        assert!((on_rep.volume - off_rep.volume).abs() <= 0.005 * off_rep.volume, "{}: volume {} against {}", c.name, on_rep.volume, off_rep.volume);
        // the surface stays on the solid: no more boundary vertices left off it than before, up to a few
        assert!(on_rep.refused <= off_rep.refused + 3 + off_rep.refused / 10, "{}: {} boundary vertices off the surface against {}", c.name, on_rep.refused, off_rep.refused);
        // every node is used
        let used: std::collections::BTreeSet<usize> = on.blocks[0].conn.iter().copied().collect();
        assert_eq!(used.len(), on.nodes.len(), "{}: unused nodes", c.name);
        // face names and their areas
        let (a0, a1) = (areas(&off), areas(&on));
        assert_eq!(a0.keys().collect::<Vec<_>>(), a1.keys().collect::<Vec<_>>(), "{}: face names", c.name);
        for (k, v0) in &a0 {
            assert!((a1[k] - v0).abs() <= 0.03 * v0, "{}: area of {k} {} against {}", c.name, a1[k], v0);
        }
    }
}

#[test]
fn the_repair_is_deterministic_and_can_be_switched_off() {
    let (m1, r1) = mesh([-1.2; 3], [1.2; 3], &sphere, None, 0.3, false, true);
    let (m2, r2) = mesh([-1.2; 3], [1.2; 3], &sphere, None, 0.3, false, true);
    assert_eq!(r1, r2);
    assert_eq!(m1.nodes, m2.nodes);
    assert_eq!(m1.blocks[0].conn, m2.blocks[0].conn);
    let (off, _) = mesh([-1.2; 3], [1.2; 3], &sphere, None, 0.3, false, false);
    assert!(below_gate(&off) > 0 && below_gate(&m1) == 0);
}

/// Diagnostic: `CASE=<substring> REPAIR_TRACE=1 cargo test --release --test tetmesh_repair -- --ignored --nocapture diag`.
#[test]
#[ignore = "diagnostic"]
fn diag() {
    let only = std::env::var("CASE").unwrap_or_default();
    for c in cases().into_iter().filter(|c| c.name.contains(&only)) {
        let fd = c.faces.as_ref().map(|f| f as &dyn Fn(V3) -> Vec<FaceHit>);
        let t = std::time::Instant::now();
        let (on, rep) = mesh(c.lo, c.hi, &c.sdf, fd, c.h, false, true);
        eprintln!("{}: {} tets, {} under the gate, min sJ {:.4}, {:.2}s", c.name, rep.tets, below_gate(&on), on.quality().min_scaled_jacobian, t.elapsed().as_secs_f64());
    }
}

#[test]
fn a_quadratic_mesh_of_the_repaired_boundary_layer_is_valid() {
    let fd = &cylinder_faces as &dyn Fn(V3) -> Vec<FaceHit>;
    let (m, _) = mesh([-1.0; 3], [1.0; 3], &cylinder, Some(fd), 0.3, true, true);
    let q = m.quality();
    assert_eq!(q.inverted, 0);
    assert!(q.min_scaled_jacobian >= QualityThresholds::default().min_scaled_jacobian, "{}", q.min_scaled_jacobian);
}
