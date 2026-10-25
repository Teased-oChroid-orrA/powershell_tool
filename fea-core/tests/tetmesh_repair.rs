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

fn boss_patches(p: V3) -> [(V3, V3); 8] {
    let r = (p[0] - 6.0).hypot(p[1] - 2.0);
    let radial = if r > 0.0 { [(p[0] - 6.0) / r, (p[1] - 2.0) / r] } else { [1.0, 0.0] };
    let rim = [6.0 + 1.2 * radial[0], 2.0 + 1.2 * radial[1]];
    let xy = [p[0].clamp(0.0, 12.0), p[1].clamp(0.0, 4.0)];
    let top = if (xy[0] - 6.0).hypot(xy[1] - 2.0) < 1.2 { rim } else { xy };
    let cap = if r > 1.2 { rim } else { [p[0], p[1]] };
    [
        ([xy[0], xy[1], 0.0], [0.0, 0.0, -1.0]),
        ([top[0], top[1], 1.0], [0.0, 0.0, 1.0]),
        ([rim[0], rim[1], p[2].clamp(1.0, 4.0)], [radial[0], radial[1], 0.0]),
        ([cap[0], cap[1], 4.0], [0.0, 0.0, 1.0]),
        ([0.0, xy[1], p[2].clamp(0.0, 1.0)], [-1.0, 0.0, 0.0]),
        ([12.0, xy[1], p[2].clamp(0.0, 1.0)], [1.0, 0.0, 0.0]),
        ([xy[0], 0.0, p[2].clamp(0.0, 1.0)], [0.0, -1.0, 0.0]),
        ([xy[0], 4.0, p[2].clamp(0.0, 1.0)], [0.0, 1.0, 0.0]),
    ]
}

fn boss_sdf(p: V3) -> f64 {
    let d = boss_patches(p).iter().map(|(q, _)| (0..3).map(|k| (p[k] - q[k]).powi(2)).sum::<f64>().sqrt()).fold(f64::INFINITY, f64::min);
    let plate = (0.0..=12.0).contains(&p[0]) && (0.0..=4.0).contains(&p[1]) && (0.0..=1.0).contains(&p[2]);
    let boss = (p[0] - 6.0).hypot(p[1] - 2.0) <= 1.2 && (1.0..=4.0).contains(&p[2]);
    if plate || boss { -d } else { d }
}

const BOSS_H: f64 = 0.25;

fn boss_face_hits(p: V3) -> Vec<FaceHit> {
    boss_patches(p).into_iter().enumerate().map(|(face, (q, outward))| {
        let off: V3 = std::array::from_fn(|k| p[k] - q[k]);
        let d = off.iter().map(|x| x * x).sum::<f64>().sqrt();
        let plane: f64 = (0..3).map(|k| off[k] * outward[k]).sum();
        let side = if plane < 0.0 { -1.0 } else { 1.0 };
        if d > 1e-12 {
            FaceHit { face, dist: side * d, normal: off.map(|x| side * x / d) }
        } else {
            FaceHit { face, dist: plane, normal: outward }
        }
    }).collect()
}

fn boss_faces(p: V3) -> Vec<FaceHit> {
    boss_face_hits(p).into_iter().filter(|hit| hit.dist.abs() <= 1.2 * BOSS_H).collect()
}

#[test]
fn connected_boss_fixture_uses_only_external_trimmed_patches() {
    assert_eq!(boss_sdf([6.0, 2.0, 1.0]), -1.0);
    assert!((boss_sdf([7.2, 2.0, 1.0])).abs() < 1e-12);
    assert!(boss_sdf([7.3, 2.0, 1.1]) > 0.0);
    assert!(boss_sdf([7.1, 2.0, 0.9]) < 0.0);
    for p in [[7.1, 2.0, 0.9], [7.3, 2.0, 1.1], [6.0, 2.0, 1.0], [-0.1, 2.0, 0.5]] {
        let hits = boss_face_hits(p);
        assert_eq!(hits.len(), 8);
        for (hit, (q, _)) in hits.iter().zip(boss_patches(p)) {
            let projected: V3 = std::array::from_fn(|k| p[k] - hit.dist * hit.normal[k]);
            for k in 0..3 { assert!((projected[k] - q[k]).abs() < 1e-12); }
            assert!(boss_sdf(projected).abs() < 1e-12);
        }
    }
    let hits = boss_faces([7.1, 2.0, 0.9]);
    assert_eq!(hits.iter().map(|hit| hit.face).collect::<Vec<_>>(), [1, 2]);
    assert!(hits.iter().all(|hit| hit.dist.abs() < 0.15));
    assert!((boss_faces([-0.1, 2.0, 0.5])[0].dist - 0.1).abs() < 1e-12);
    assert!(boss_faces([6.0, 2.0, 1.0]).is_empty());
    for z in [2.0, 2.01] {
        assert_eq!(boss_faces([7.2, 2.0, z]).iter().map(|hit| hit.face).collect::<Vec<_>>(), [2]);
    }
}

#[test]
#[ignore = "diagnostic: the connected boss rim must pass the default solver gate"]
fn connected_plate_boss_rim_passes_solver_gate() {
    let size = |_: V3| 10.0;
    let face = |p: V3| boss_face_hits(p).into_iter().min_by(|a, b| a.dist.abs().total_cmp(&b.dist.abs())).unwrap().face;
    let solid = Solid { lo: [0.0, 0.0, 0.0], hi: [12.0, 4.0, 4.0], sdf: &boss_sdf, size: &size, face_of: &face, face_dists: Some(&boss_faces) };
    let (mesh, report, _) = mesh_solid_reported(&solid, Elastic::new(1.0, 0.3), TetOptions { base: 0.6, boundary_size: BOSS_H, repair: true, ..Default::default() }).unwrap();
    let quality = mesh.quality().with_progression(&fea_core::meshing::adjacency::Adjacency::new(&mesh));
    quality.require(&QualityThresholds::default()).unwrap();
    assert_eq!(quality.inverted, 0);
    assert_eq!(below_gate(&mesh), 0, "{} of {} below gate, min scaled Jacobian {}", below_gate(&mesh), report.tets, quality.min_scaled_jacobian);
    let cells = mesh.blocks.iter().flat_map(|b| b.conn.chunks_exact(4).map(|t| [t[0],t[1],t[2],t[3]])).collect();
    let topology = fea_core::meshing::tet_topology::TetTopology::new(mesh.nodes.clone(), cells, report.tets).unwrap();
    let mut edges = BTreeMap::<_, usize>::new();
    let mut links = BTreeMap::<_, BTreeMap<usize, Vec<usize>>>::new();
    for f in topology.boundary() {
        for k in 0..3 {
            let (v,a,b) = (f[k], f[(k+1)%3], f[(k+2)%3]);
            *edges.entry((v.min(a),v.max(a))).or_default() += 1;
            let link = links.entry(v).or_default();
            link.entry(a).or_default().push(b);
            link.entry(b).or_default().push(a);
        }
    }
    assert!(edges.values().all(|&count| count == 2), "nonmanifold boundary edge");
    for (v,link) in links {
        assert!(link.values().all(|ns| ns.len() == 2), "noncyclic boundary link at {v}");
        let mut seen = std::collections::BTreeSet::new();
        let mut queue = vec![*link.keys().next().unwrap()];
        while let Some(n) = queue.pop() { if seen.insert(n) { queue.extend(&link[&n]); } }
        assert_eq!(seen.len(), link.len(), "disconnected boundary link at {v}");
    }
}
