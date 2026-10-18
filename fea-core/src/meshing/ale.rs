//! Arbitrary Lagrangian-Eulerian transport on a moving mesh (epic #59, "ALE: mesh velocity, transport").
//!
//! The motion subsystem ([`super::motion`]) moves nodes; this module says what a scalar does while they move. A cell-average
//! field `c` (one value per element, global element order) is advanced from the mesh `old` to the mesh `new` (same connectivity,
//! nodes moved linearly in time over the step) in a finite-volume update with the material velocity `u`:
//!
//! `c_new V_new = c V_old - sum over faces of c_face Q_face`, `Q_face = dt u . nA - S_face`,
//!
//! where `S_face` is the volume swept by the face during the step (the mesh velocity's flux, `w . nA` integrated in time:
//! exact, by Simpson's rule in time for the quadratic area vector) and `c_face` is the upwind value. Measures are those of the
//! polyhedron/polygon the corner nodes span, quadrilateral faces split by the diagonal through the smallest global node number
//! so that both neighbours use the same triangles. With that choice the swept volumes of an element's faces add up to its change
//! of measure exactly (the geometric conservation law): a uniform field stays uniform on any motion, whatever the distortion,
//! and `sum c V` changes only by what crosses the boundary. First-order upwind: robust and conservative, not accurate; it is the
//! verified frame (GCL, conservation) on which a higher-order scheme would sit. Corner nodes only (linear geometry).

use super::MeshError;
use crate::mesh::Mesh;
use crate::topology::corner_faces;
use std::collections::BTreeMap;

type V3 = [f64; 3];

#[derive(Debug, Clone, PartialEq)]
pub struct TransportReport {
    /// Largest `|V_new - V_old - sum of swept volumes|` over the elements, relative to the mean element measure (round-off for
    /// a sound motion: the geometric conservation law).
    pub gcl_defect: f64,
    /// Net content that left through boundary faces (outflow minus inflow).
    pub boundary_outflow: f64,
    /// Largest local Courant-like number: `|Q_face| / min(V_old, V_new)` over interior faces; above 1 the upwind update can
    /// overshoot (the field leaves the range of its neighbours).
    pub max_flux_ratio: f64,
}

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn lerp(a: V3, b: V3, s: f64) -> V3 {
    std::array::from_fn(|k| a[k] + s * (b[k] - a[k]))
}

/// Triangles (as corner-node ids, outward orientation kept) of a face with 2 (edge), 3 or 4 nodes; a quadrilateral is split by
/// the diagonal through its smallest node number. A 2D edge is returned as itself.
fn pieces(face: &[usize]) -> Vec<Vec<usize>> {
    match face.len() {
        4 => {
            let m = *face.iter().min().expect("four nodes");
            let at = face.iter().position(|&n| n == m).expect("minimum present");
            let q: Vec<usize> = (0..4).map(|k| face[(at + k) % 4]).collect();
            vec![vec![q[0], q[1], q[2]], vec![q[0], q[2], q[3]]]
        }
        _ => vec![face.to_vec()],
    }
}

/// Outward area vector of a piece at the node positions `x`: a triangle's `1/2 (b - a) x (c - a)`; an edge's rotation of
/// `(b - a)` by minus ninety degrees (body on the left, outward to the right).
fn area_vector(piece: &[usize], x: &dyn Fn(usize) -> V3) -> V3 {
    if piece.len() == 2 {
        let d = sub(x(piece[1]), x(piece[0]));
        [d[1], -d[0], 0.0]
    } else {
        let (a, b, c) = (x(piece[0]), x(piece[1]), x(piece[2]));
        let n = cross(sub(b, a), sub(c, a));
        [0.5 * n[0], 0.5 * n[1], 0.5 * n[2]]
    }
}

/// Area vector integrated over the unit-time step (Simpson: exact for the quadratic dependence on time) and the volume swept
/// by the piece, both outward.
fn integrated(piece: &[usize], old: &Mesh, new: &Mesh) -> (V3, f64) {
    let at = |s: f64| move |n: usize| lerp(old.nodes[n], new.nodes[n], s);
    let (a0, ah, a1) = (area_vector(piece, &at(0.0)), area_vector(piece, &at(0.5)), area_vector(piece, &at(1.0)));
    let mean: V3 = std::array::from_fn(|k| (a0[k] + 4.0 * ah[k] + a1[k]) / 6.0);
    let w: V3 = std::array::from_fn(|k| piece.iter().map(|&n| new.nodes[n][k] - old.nodes[n][k]).sum::<f64>() / piece.len() as f64);
    (mean, dot(w, mean))
}

/// Measure (area in 2D, volume in 3D) of the element with corner list `corners` and outward faces `faces`.
fn measure(mesh: &Mesh, faces: &[Vec<usize>], corners: &[usize]) -> f64 {
    if mesh.dim() == 2 {
        let n = corners.len();
        0.5 * (0..n)
            .map(|i| {
                let (p, q) = (mesh.nodes[corners[i]], mesh.nodes[corners[(i + 1) % n]]);
                p[0] * q[1] - p[1] * q[0]
            })
            .sum::<f64>()
    } else {
        let mut v = 0.0;
        for f in faces {
            for tri in pieces(f) {
                let (a, b, c) = (mesh.nodes[tri[0]], mesh.nodes[tri[1]], mesh.nodes[tri[2]]);
                v += dot(a, cross(b, c)) / 6.0;
            }
        }
        v
    }
}

/// Nodal mesh velocity `(new - old) / dt`.
pub fn mesh_velocity(old: &Mesh, new: &Mesh, dt: f64) -> Result<Vec<V3>, MeshError> {
    if old.nodes.len() != new.nodes.len() || dt <= 0.0 || !dt.is_finite() {
        return Err(MeshError::Failed("mesh velocity needs two meshes of the same node count and a positive time step".into()));
    }
    Ok(old.nodes.iter().zip(&new.nodes).map(|(a, b)| std::array::from_fn(|k| (b[k] - a[k]) / dt)).collect())
}

/// One conservative upwind step. `c` holds one value per element (blocks in order); `u` is the material velocity at a point;
/// `inflow` is the value carried in through a boundary face where the relative flux enters.
pub fn transport_step(old: &Mesh, new: &Mesh, c: &[f64], u: &dyn Fn(V3) -> V3, dt: f64, inflow: f64) -> Result<(Vec<f64>, TransportReport), MeshError> {
    let n_elems = old.n_elems();
    if new.nodes.len() != old.nodes.len() || new.n_elems() != n_elems || new.blocks.len() != old.blocks.len() || new.blocks.iter().zip(&old.blocks).any(|(a, b)| a.conn != b.conn || a.kind != b.kind) {
        return Err(MeshError::Failed("transport needs two meshes with identical connectivity".into()));
    }
    if c.len() != n_elems || dt <= 0.0 || !dt.is_finite() || c.iter().any(|v| !v.is_finite()) {
        return Err(MeshError::Failed("transport needs one finite value per element and a positive time step".into()));
    }
    let d = old.dim();
    // Per element: corners and outward faces (corner nodes), flattened in global element order.
    let mut elements: Vec<(Vec<usize>, Vec<Vec<usize>>)> = Vec::with_capacity(n_elems);
    for b in &old.blocks {
        let local = corner_faces(b.kind);
        let nc = b.kind.n_corners();
        for e in 0..b.n_elems() {
            let nodes = b.elem(e);
            elements.push((nodes[..nc].to_vec(), local.iter().map(|f| f.iter().map(|&i| nodes[i]).collect()).collect()));
        }
    }
    // Faces by sorted corner nodes: the (element, face) copies that share them.
    let mut owners: BTreeMap<Vec<usize>, Vec<(usize, usize)>> = BTreeMap::new();
    for (g, (_, faces)) in elements.iter().enumerate() {
        for (k, f) in faces.iter().enumerate() {
            let mut key = f.clone();
            key.sort_unstable();
            owners.entry(key).or_default().push((g, k));
        }
    }
    let v_old: Vec<f64> = elements.iter().map(|(corners, faces)| measure(old, faces, corners)).collect();
    let v_new: Vec<f64> = elements.iter().map(|(corners, faces)| measure(new, faces, corners)).collect();
    if v_old.iter().chain(&v_new).any(|v| !(*v > 0.0 && v.is_finite())) {
        return Err(MeshError::Failed("an element has a non-positive measure on one of the meshes".into()));
    }
    let mean_measure = v_old.iter().sum::<f64>() / n_elems as f64;
    let mut content: Vec<f64> = (0..n_elems).map(|g| c[g] * v_old[g]).collect();
    let mut swept_sum = vec![0.0; n_elems];
    let (mut boundary_outflow, mut max_ratio) = (0.0f64, 0.0f64);
    for copies in owners.values() {
        if copies.len() > 2 {
            return Err(MeshError::NonConforming("a face is shared by more than two elements".into()));
        }
        // Flux across the face as seen from its first element; the second sees the exact opposite.
        let (g, k) = copies[0];
        let face = &elements[g].1[k];
        let mut q = 0.0;
        let mut swept = 0.0;
        for piece in pieces(face) {
            let (mean, s) = integrated(&piece, old, new);
            let mid: V3 = std::array::from_fn(|i| piece.iter().map(|&n| 0.5 * (old.nodes[n][i] + new.nodes[n][i])).sum::<f64>() / piece.len() as f64);
            let uv = u(mid);
            q += dt * dot(if d == 2 { [uv[0], uv[1], 0.0] } else { uv }, mean) - s;
            swept += s;
        }
        swept_sum[g] += swept;
        match copies.get(1) {
            Some(&(h, _)) => {
                swept_sum[h] -= swept;
                let upwind = if q >= 0.0 { c[g] } else { c[h] };
                content[g] -= upwind * q;
                content[h] += upwind * q;
                max_ratio = max_ratio.max(q.abs() / v_old[g].min(v_new[g]).min(v_old[h]).min(v_new[h]));
            }
            None => {
                let carried = if q >= 0.0 { c[g] } else { inflow };
                content[g] -= carried * q;
                boundary_outflow += carried * q;
            }
        }
    }
    let gcl = (0..n_elems).map(|g| (v_new[g] - v_old[g] - swept_sum[g]).abs()).fold(0.0, f64::max) / mean_measure;
    let out: Vec<f64> = (0..n_elems).map(|g| content[g] / v_new[g]).collect();
    Ok((out, TransportReport { gcl_defect: gcl, boundary_outflow, max_flux_ratio: max_ratio }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element::ElementKind;
    use crate::generate::grid;
    use crate::mesh::{Elastic, Physics};

    fn cells2(n: usize) -> Mesh {
        grid(Physics::PlaneStress { thickness: 1.0 }, ElementKind::Quad4, Elastic::new(1., 0.3), [n, n, 1], &|p| p).unwrap()
    }

    fn cells3(kind: ElementKind, n: usize) -> Mesh {
        grid(Physics::Solid, kind, Elastic::new(1., 0.3), [n, n, n], &|p| p).unwrap()
    }

    /// Interior nodes moved by a deterministic pseudo-random amount of up to `amp` times the cell size; the boundary stays.
    fn jiggle(m: &Mesh, n: usize, amp: f64, seed: u64) -> Mesh {
        let mut out = m.clone();
        let mut s = seed;
        let mut next = || {
            s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((s >> 33) as f64) / (1u64 << 31) as f64 - 0.5
        };
        for x in &mut out.nodes {
            let interior = (0..m.dim()).all(|k| x[k] > 1e-9 && x[k] < 1.0 - 1e-9);
            if interior {
                for v in x.iter_mut().take(m.dim()) {
                    *v += 2.0 * amp * next() / n as f64;
                }
            }
        }
        out
    }

    #[test]
    fn a_uniform_field_stays_uniform_on_any_motion_in_two_and_three_dimensions() {
        // The geometric conservation law: on arbitrary distorted meshes a uniform field stays uniform to round-off, with a
        // constant material velocity that carries content through the boundary (the inflow value is the uniform value too) and
        // with no velocity at all. A closed polyhedron has a vanishing sum of area vectors, so a constant velocity has no net
        // flux through any element.
        let wind = |_: V3| [0.7, -0.3, 0.2];
        for (name, base) in [("quad4", cells2(6)), ("hex8", cells3(ElementKind::Hex8, 4)), ("wedge6", cells3(ElementKind::Wedge6, 4)), ("tet4", crate::meshing::transition::tetrahedralize(&cells3(ElementKind::Hex8, 3), &|_, _, _| true).unwrap().0)] {
            let n = if base.dim() == 2 { 6 } else { 4 };
            let mut mesh = base.clone();
            let mut c = vec![1.0; mesh.n_elems()];
            for step in 0..5u64 {
                let next = jiggle(&base, n, 0.3, 11 + step);
                let (c2, r) = transport_step(&mesh, &next, &c, &wind, 0.01, 1.0).unwrap();
                assert!(r.gcl_defect < 1e-13, "{name}: GCL defect {}", r.gcl_defect);
                let worst = c2.iter().map(|v| (v - 1.0).abs()).fold(0.0, f64::max);
                assert!(worst < 1e-12, "{name} step {step}: uniform field changed by {worst}");
                c = c2;
                mesh = next;
            }
            // Zero material velocity: the mesh motion alone must leave c = 1 exactly.
            let mut mesh = base.clone();
            let mut c = vec![1.0; mesh.n_elems()];
            for step in 0..5u64 {
                let next = jiggle(&base, n, 0.3, 100 + step);
                let (c2, r) = transport_step(&mesh, &next, &c, &|_| [0.0; 3], 0.01, 1.0).unwrap();
                assert!(r.gcl_defect < 1e-13, "{name}: GCL defect {}", r.gcl_defect);
                assert!(c2.iter().all(|v| (v - 1.0).abs() < 1e-13), "{name}: c = 1 not preserved: {:?}", c2.iter().map(|v| (v - 1.0).abs()).fold(0.0, f64::max));
                c = c2;
                mesh = next;
            }
        }
    }

    #[test]
    fn content_is_conserved_exactly_and_the_boundary_accounts_for_any_change() {
        let base = cells2(10);
        let centre = |x: V3| (-((x[0] - 0.5).powi(2) + (x[1] - 0.5).powi(2)) / 0.04).exp();
        let measure_of = |m: &Mesh| -> Vec<f64> {
            let mut v = Vec::new();
            for b in &m.blocks {
                let local = corner_faces(b.kind);
                for e in 0..b.n_elems() {
                    let nodes = b.elem(e);
                    let faces: Vec<Vec<usize>> = local.iter().map(|f| f.iter().map(|&i| nodes[i]).collect()).collect();
                    v.push(measure(m, &faces, &nodes[..b.kind.n_corners()]));
                }
            }
            v
        };
        let v0 = measure_of(&base);
        let mut c: Vec<f64> = (0..base.n_elems()).map(|e| {
            let nodes = base.blocks[0].elem(e);
            centre(std::array::from_fn(|k| nodes.iter().map(|&n| base.nodes[n][k]).sum::<f64>() / 4.0))
        }).collect();
        let total0: f64 = c.iter().zip(&v0).map(|(a, b)| a * b).sum();
        let mut mesh = base.clone();
        let mut exchanged = 0.0;
        for step in 0..8u64 {
            let next = jiggle(&base, 10, 0.35, 7 + step);
            // A uniform translation of the material through the box: the boundary now carries content.
            let (c2, r) = transport_step(&mesh, &next, &c, &|_| [0.5, 0.0, 0.0], 0.02, 0.0).unwrap();
            exchanged += r.boundary_outflow;
            c = c2;
            mesh = next;
        }
        let total1: f64 = c.iter().zip(&measure_of(&mesh)).map(|(a, b)| a * b).sum();
        assert!((total0 - total1 - exchanged).abs() < 1e-13 * total0, "content {total0} -> {total1}, boundary {exchanged}");
        // With closed boundaries (zero material velocity, fixed boundary) the content is conserved to round-off.
        let mut mesh = base.clone();
        let mut c2: Vec<f64> = c.clone();
        let t0: f64 = c2.iter().zip(&measure_of(&mesh)).map(|(a, b)| a * b).sum();
        for step in 0..8u64 {
            let next = jiggle(&base, 10, 0.35, 70 + step);
            c2 = transport_step(&mesh, &next, &c2, &|_| [0.0; 3], 0.02, 0.0).unwrap().0;
            mesh = next;
        }
        let t1: f64 = c2.iter().zip(&measure_of(&mesh)).map(|(a, b)| a * b).sum();
        assert!((t0 - t1).abs() < 1e-13 * t0, "{t0} vs {t1}");
    }

    #[test]
    fn a_static_field_seen_on_a_mesh_that_oscillates_returns_to_itself_with_an_error_that_falls_with_the_mesh_size() {
        let c0 = |x: V3| (-((x[0] - 0.5).powi(2) + (x[1] - 0.5).powi(2)) / 0.1).exp();
        let mut errors = Vec::new();
        for n in [12usize, 24, 48] {
            let base = cells2(n);
            let displaced = |t: f64| -> Mesh {
                let mut m = base.clone();
                for x in &mut m.nodes {
                    let g = (std::f64::consts::PI * x[0]).sin() * (std::f64::consts::PI * x[1]).sin();
                    let phase = (2.0 * std::f64::consts::PI * t).sin();
                    x[0] += 0.02 * phase * g;
                    x[1] += 0.012 * phase * g;
                }
                m
            };
            let mean_of = |m: &Mesh, e: usize| -> f64 {
                // Cell average of c0 by a 3x3 Gauss rule on the (bilinear) cell.
                let nodes = m.blocks[0].elem(e);
                let p: Vec<V3> = nodes.iter().map(|&k| m.nodes[k]).collect();
                let g = [-(0.6f64).sqrt(), 0.0, (0.6f64).sqrt()];
                let w = [5.0 / 9.0, 8.0 / 9.0, 5.0 / 9.0];
                let (mut num, mut den) = (0.0, 0.0);
                for (i, &xi) in g.iter().enumerate() {
                    for (j, &eta) in g.iter().enumerate() {
                        let sh = [0.25 * (1.0 - xi) * (1.0 - eta), 0.25 * (1.0 + xi) * (1.0 - eta), 0.25 * (1.0 + xi) * (1.0 + eta), 0.25 * (1.0 - xi) * (1.0 + eta)];
                        let dxi = [-0.25 * (1.0 - eta), 0.25 * (1.0 - eta), 0.25 * (1.0 + eta), -0.25 * (1.0 + eta)];
                        let deta = [-0.25 * (1.0 - xi), -0.25 * (1.0 + xi), 0.25 * (1.0 + xi), 0.25 * (1.0 - xi)];
                        let x: V3 = std::array::from_fn(|k| (0..4).map(|a| sh[a] * p[a][k]).sum());
                        let jac = (0..4).map(|a| dxi[a] * p[a][0]).sum::<f64>() * (0..4).map(|a| deta[a] * p[a][1]).sum::<f64>() - (0..4).map(|a| dxi[a] * p[a][1]).sum::<f64>() * (0..4).map(|a| deta[a] * p[a][0]).sum::<f64>();
                        num += w[i] * w[j] * jac * c0(x);
                        den += w[i] * w[j] * jac;
                    }
                }
                num / den
            };
            let steps = 4 * n;
            let mut mesh = displaced(0.0);
            let mut c: Vec<f64> = (0..mesh.n_elems()).map(|e| mean_of(&mesh, e)).collect();
            for s in 0..steps {
                let next = displaced((s + 1) as f64 / steps as f64);
                c = transport_step(&mesh, &next, &c, &|_| [0.0; 3], 1.0 / steps as f64, 0.0).unwrap().0;
                mesh = next;
            }
            let err = (0..mesh.n_elems()).map(|e| (c[e] - mean_of(&mesh, e)).abs()).fold(0.0, f64::max);
            eprintln!("n {n}: max cell-average error after one period {err:.3e}");
            errors.push(err);
        }
        assert!(errors[1] < 0.75 * errors[0] && errors[2] < 0.75 * errors[1], "{errors:?}");
    }

    #[test]
    fn mismatched_meshes_bad_values_and_inverted_cells_are_refused() {
        let m = cells2(3);
        let c = vec![1.0; m.n_elems()];
        let zero = |_: V3| [0.0; 3];
        assert!(transport_step(&m, &cells2(4), &c, &zero, 0.1, 0.0).is_err());
        assert!(transport_step(&m, &m, &c[..3], &zero, 0.1, 0.0).is_err());
        assert!(transport_step(&m, &m, &c, &zero, 0.0, 0.0).is_err());
        let mut bad = c.clone();
        bad[0] = f64::NAN;
        assert!(transport_step(&m, &m, &bad, &zero, 0.1, 0.0).is_err());
        let mut folded = m.clone();
        folded.nodes[5] = [-1.0, -1.0, 0.0];
        assert!(transport_step(&m, &folded, &c, &zero, 0.1, 0.0).is_err());
        assert!(mesh_velocity(&m, &cells2(4), 0.1).is_err());
        let w = mesh_velocity(&m, &folded, 0.5).unwrap();
        assert_eq!(w[5], [(-1.0 - m.nodes[5][0]) / 0.5, (-1.0 - m.nodes[5][1]) / 0.5, 0.0]);
    }
}
