//! B-rep solids for the FEA preprocessor, kernel-agnostic part (ADR-019).
//!
//! A solid arrives as a **face-tagged surface mesh**: triangles, each carrying the index of the B-rep face it tessellates.
//! [`SurfaceSolid`] welds it, checks it is a closed manifold, orients it outward, and answers the two questions the volume
//! mesher asks: the signed distance to the surface (negative inside) and which faces are near a point
//! ([`fea_core::tetmesh::FaceHit`]). [`mesh_volume`] then fills it with tetrahedra.
//!
//! Distances come from a bounding-volume tree over the triangles with angle-weighted pseudo-normals for the sign
//! (Baerentzen and Aanaes), which is robust at edges and vertices where a plain face normal is not. The tessellation
//! deflection bounds the geometric error, so it should be a small fraction of the boundary element size.
//!
//! The OpenCascade side (`occt` feature) only has to produce the tessellation, the faces and the STEP files; nothing
//! here depends on it, which is also what makes this part testable on analytic shapes.

pub mod analytic_step;
pub mod chart_mesh;
pub mod native_surface;
pub mod nurbs;
pub mod native_volume;
pub mod orientation_repair;
pub mod triangle_intersection;
pub mod planar_step;
pub mod plc;
pub mod plc_star;
pub mod cylindrical_step;
mod bvh;
pub mod cad;
pub mod step;
pub mod step_nurbs;
mod step_units;
mod step_topology;
#[cfg(feature = "occt")]
pub mod occt;

use bvh::{Bvh, Region};
use fea_core::tetmesh::{FaceHit, MeshReport, Solid, TetOptions};
use fea_core::{Elastic, Mesh};
use std::collections::HashMap;

pub type V3 = [f64; 3];

/// A triangle soup tagged with the model face of each triangle.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SurfaceMesh {
    pub vertices: Vec<V3>,
    pub triangles: Vec<[usize; 3]>,
    /// Model face (B-rep face index) of each triangle.
    pub face: Vec<usize>,
}

impl SurfaceMesh {
    pub fn push_triangle(&mut self, a: V3, b: V3, c: V3, face: usize) {
        let base = self.vertices.len();
        self.vertices.extend([a, b, c]);
        self.triangles.push([base, base + 1, base + 2]);
        self.face.push(face);
    }
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
fn norm(a: V3) -> f64 {
    dot(a, a).sqrt()
}
fn unit(a: V3) -> V3 {
    let n = norm(a);
    if n > 0.0 { [a[0] / n, a[1] / n, a[2] / n] } else { [0.0; 3] }
}

/// Distance of `p` from the segment `ab` and the parameter of the closest point along it.
fn segment_distance(p: V3, a: V3, b: V3) -> (f64, f64) {
    let ab = sub(b, a);
    let len2 = dot(ab, ab);
    let t = if len2 > 0.0 { (dot(sub(p, a), ab) / len2).clamp(0.0, 1.0) } else { 0.0 };
    let q = [a[0] + t * ab[0], a[1] + t * ab[1], a[2] + t * ab[2]];
    (norm(sub(p, q)), t)
}

/// Split every triangle whose edge has no opposite edge at the vertices that lie on that edge (T-junctions), until the
/// open edges are gone or no more can be closed. Orientation and face tags are kept.
fn repair_t_junctions(vertices: &[V3], triangles: &mut Vec<[usize; 3]>, face: &mut Vec<usize>) {
    for _round in 0..8 {
        let mut count: HashMap<(usize, usize), usize> = HashMap::new();
        for t in triangles.iter() {
            for k in 0..3 {
                *count.entry((t[k], t[(k + 1) % 3])).or_default() += 1;
            }
        }
        let open = |a: usize, b: usize, count: &HashMap<(usize, usize), usize>| count.get(&(b, a)).is_none();
        let mut on_open: Vec<usize> = Vec::new();
        for t in triangles.iter() {
            for k in 0..3 {
                let (a, b) = (t[k], t[(k + 1) % 3]);
                if open(a, b, &count) {
                    on_open.extend([a, b]);
                }
            }
        }
        on_open.sort_unstable();
        on_open.dedup();
        if on_open.is_empty() {
            return;
        }
        let mut changed = false;
        let mut next: Vec<[usize; 3]> = Vec::with_capacity(triangles.len());
        let mut next_face: Vec<usize> = Vec::with_capacity(face.len());
        for (t, f) in triangles.iter().zip(face.iter()) {
            // Find one open edge of this triangle with vertices on it.
            let mut split = None;
            for k in 0..3 {
                let (a, b) = (t[k], t[(k + 1) % 3]);
                if !open(a, b, &count) {
                    continue;
                }
                let len = norm(sub(vertices[b], vertices[a]));
                let tol = 1e-6 * len.max(1e-12);
                let mut between: Vec<(f64, usize)> = on_open
                    .iter()
                    .filter(|&&v| v != a && v != b)
                    .filter_map(|&v| {
                        let (d, u) = segment_distance(vertices[v], vertices[a], vertices[b]);
                        (d <= tol && u > 1e-9 && u < 1.0 - 1e-9).then_some((u, v))
                    })
                    .collect();
                if !between.is_empty() {
                    between.sort_by(|x, y| x.0.total_cmp(&y.0));
                    split = Some((k, between.into_iter().map(|(_, v)| v).collect::<Vec<_>>()));
                    break;
                }
            }
            match split {
                Some((k, vs)) => {
                    // Fan from the opposite vertex over the edge a -> v1 -> ... -> b.
                    let (a, b, c) = (t[k], t[(k + 1) % 3], t[(k + 2) % 3]);
                    let mut chain = vec![a];
                    chain.extend(vs);
                    chain.push(b);
                    for w in chain.windows(2) {
                        next.push([w[0], w[1], c]);
                        next_face.push(*f);
                    }
                    changed = true;
                }
                None => {
                    next.push(*t);
                    next_face.push(*f);
                }
            }
        }
        *triangles = next;
        *face = next_face;
        if !changed {
            return;
        }
    }
}

/// A closed, outward-oriented, face-tagged surface with distance queries.
pub struct SurfaceSolid {
    vertices: Vec<V3>,
    triangles: Vec<[usize; 3]>,
    face: Vec<usize>,
    normal: Vec<V3>,
    /// Pseudo-normals: per vertex, and per edge `(min, max)`.
    vertex_normal: Vec<V3>,
    edge_normal: HashMap<(usize, usize), V3>,
    bvh: Bvh,
    faces: usize,
    pub lo: V3,
    pub hi: V3,
}

impl SurfaceSolid {
    /// Weld the vertices (positions equal within `weld`), require every edge to belong to exactly two triangles that
    /// traverse it in opposite directions (a closed manifold), and orient the surface outward.
    pub fn new(mesh: SurfaceMesh, weld: f64) -> Result<Self, String> {
        if mesh.triangles.is_empty() || mesh.face.len() != mesh.triangles.len() {
            return Err("the surface mesh has no triangles (or a face tag is missing)".into());
        }
        if !weld.is_finite() || weld <= 0.0 || mesh.vertices.is_empty()
            || mesh.vertices.iter().flatten().any(|x| !x.is_finite())
            || mesh.triangles.iter().flatten().any(|i| *i >= mesh.vertices.len()) {
            return Err("invalid surface coordinates, indices or weld distance".into());
        }
        let origin = mesh.vertices[0];
        let mut index: HashMap<[i64; 3], Vec<usize>> = HashMap::new();
        let mut vertices: Vec<V3> = Vec::new();
        let mut remap = vec![0usize; mesh.vertices.len()];
        for (i, v) in mesh.vertices.iter().enumerate() {
            let relative = sub(*v, origin).map(|x| (x / weld).floor());
            if relative.iter().any(|x| x.abs() >= (i64::MAX / 2) as f64) {
                return Err("surface extent exceeds the weld grid range".into());
            }
            let q = relative.map(|x| x as i64);
            let mut nearest: Option<(f64, usize)> = None;
            for x in -1..=1 { for y in -1..=1 { for z in -1..=1 {
                if let Some(ids) = index.get(&[q[0]+x,q[1]+y,q[2]+z]) {
                    for &j in ids {
                        let d = norm(sub(*v, vertices[j]));
                        if d <= weld && nearest.is_none_or(|best| (d,j) < best) { nearest=Some((d,j)); }
                    }
                }
            } } }
            remap[i] = match nearest {
                Some((_, j)) => j,
                None => { let j=vertices.len(); vertices.push(*v); index.entry(q).or_default().push(j); j }
            };
        }
        let mut triangles = Vec::with_capacity(mesh.triangles.len());
        let mut face = Vec::with_capacity(mesh.triangles.len());
        for (t, f) in mesh.triangles.iter().zip(&mesh.face) {
            let t = [remap[t[0]], remap[t[1]], remap[t[2]]];
            if t[0] == t[1] || t[1] == t[2] || t[2] == t[0] {
                continue; // collapsed by the weld
            }
            let n = cross(sub(vertices[t[1]], vertices[t[0]]), sub(vertices[t[2]], vertices[t[0]]));
            if norm(n) < 1e-18 {
                continue;
            }
            triangles.push(t);
            face.push(*f);
        }
        // A kernel's tessellation can have T-junctions (a planar face with a hole discretises a straight edge more finely
        // than the neighbouring face): split the triangle on the coarse side at the vertices that lie on its edge.
        repair_t_junctions(&vertices, &mut triangles, &mut face);
        // Closed manifold: every directed edge appears once and its reverse appears once.
        let mut directed: HashMap<(usize, usize), usize> = HashMap::new();
        for t in &triangles {
            for k in 0..3 {
                *directed.entry((t[k], t[(k + 1) % 3])).or_default() += 1;
            }
        }
        let mut bad: Vec<(usize, usize)> = directed.iter().filter(|((a, b), c)| **c != 1 || directed.get(&(*b, *a)) != Some(&1)).map(|(e, _)| *e).collect();
        bad.sort_unstable();
        if let Some(&(a, b)) = bad.first() {
            let (pa, pb) = (vertices[a], vertices[b]);
            return Err(format!(
                "the surface is not a closed manifold ({} open or doubled edges; first from ({:.4}, {:.4}, {:.4}) to ({:.4}, {:.4}, {:.4})): tessellate finer or heal the model",
                bad.len(),
                pa[0], pa[1], pa[2], pb[0], pb[1], pb[2]
            ));
        }
        // Orient outward: the signed volume must be positive.
        let volume: f64 = triangles.iter().map(|t| dot(sub(vertices[t[0]], origin), cross(sub(vertices[t[1]], origin), sub(vertices[t[2]], origin))) / 6.0).sum();
        if volume < 0.0 {
            for t in &mut triangles {
                t.swap(1, 2);
            }
        }
        if volume.abs() < 1e-300 {
            return Err("the surface encloses no volume".into());
        }
        Ok(Self::from_oriented_mesh(SurfaceMesh { vertices, triangles, face }))
    }

    // Called after either the legacy repair/validation path or the sealed
    // sampled-facet intersection gate; this stage never welds or changes indices.
    fn from_oriented_mesh(mesh: SurfaceMesh) -> Self {
        let SurfaceMesh { vertices, triangles, face } = mesh;
        let normal: Vec<V3> = triangles.iter().map(|t| unit(cross(sub(vertices[t[1]], vertices[t[0]]), sub(vertices[t[2]], vertices[t[0]])))).collect();
        // Angle-weighted vertex normals and summed edge normals.
        let mut vertex_normal = vec![[0.0; 3]; vertices.len()];
        let mut edge_normal: HashMap<(usize, usize), V3> = HashMap::new();
        for (t, n) in triangles.iter().zip(&normal) {
            for k in 0..3 {
                let (a, b, c) = (vertices[t[k]], vertices[t[(k + 1) % 3]], vertices[t[(k + 2) % 3]]);
                let (u, v) = (unit(sub(b, a)), unit(sub(c, a)));
                let angle = dot(u, v).clamp(-1.0, 1.0).acos();
                for j in 0..3 {
                    vertex_normal[t[k]][j] += angle * n[j];
                }
                let e = (t[k].min(t[(k + 1) % 3]), t[k].max(t[(k + 1) % 3]));
                let slot = edge_normal.entry(e).or_insert([0.0; 3]);
                for j in 0..3 {
                    slot[j] += n[j];
                }
            }
        }
        let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
        for v in &vertices {
            for k in 0..3 {
                lo[k] = lo[k].min(v[k]);
                hi[k] = hi[k].max(v[k]);
            }
        }
        let bvh = Bvh::build(&vertices, &triangles);
        let faces = face.iter().max().map_or(0, |m| m + 1);
        Self { vertices, triangles, face, normal, vertex_normal, edge_normal, bvh, faces, lo, hi }
    }

    /// Number of model faces (one more than the largest face tag).
    pub fn face_count(&self) -> usize {
        self.faces
    }
    pub fn triangle_count(&self) -> usize {
        self.triangles.len()
    }
    /// Enclosed volume of the tessellation.
    pub fn volume(&self) -> f64 {
        self.triangles.iter().map(|t| dot(sub(self.vertices[t[0]], self.vertices[0]), cross(sub(self.vertices[t[1]], self.vertices[0]), sub(self.vertices[t[2]], self.vertices[0]))) / 6.0).sum()
    }
    /// Area of each model face.
    pub fn face_areas(&self) -> Vec<f64> {
        let mut a = vec![0.0; self.faces];
        for (t, f) in self.triangles.iter().zip(&self.face) {
            a[*f] += 0.5 * norm(cross(sub(self.vertices[t[1]], self.vertices[t[0]]), sub(self.vertices[t[2]], self.vertices[t[0]])));
        }
        a
    }

    fn pseudo_normal(&self, t: usize, region: Region) -> V3 {
        let tri = self.triangles[t];
        match region {
            Region::Face => self.normal[t],
            Region::Edge(i, j) => {
                let (a, b) = (tri[i], tri[j]);
                self.edge_normal[&(a.min(b), a.max(b))]
            }
            Region::Vertex(i) => self.vertex_normal[tri[i]],
        }
    }

    /// Signed distance to the surface: negative inside.
    pub fn signed_distance(&self, p: V3) -> f64 {
        let (d2, t, region, closest) = self.bvh.nearest(&self.vertices, &self.triangles, p);
        let n = self.pseudo_normal(t, region);
        let side = dot(sub(p, closest), n);
        let d = d2.sqrt();
        if side < 0.0 { -d } else { d }
    }

    /// The model faces within `radius` of `p`: the signed distance to the plane of the nearest triangle of each, and that
    /// triangle's outward normal (what `fea_core::tetmesh` snaps with).
    pub fn faces_near(&self, p: V3, radius: f64) -> Vec<FaceHit> {
        let mut best: HashMap<usize, (f64, usize)> = HashMap::new();
        self.bvh.within(&self.vertices, &self.triangles, p, radius, &mut |t, d2| {
            let f = self.face[t];
            let e = best.entry(f).or_insert((f64::INFINITY, t));
            if d2 < e.0 {
                *e = (d2, t);
            }
        });
        let mut hits: Vec<FaceHit> = best
            .into_iter()
            .map(|(face, (_, t))| {
                let n = self.normal[t];
                let a = self.vertices[self.triangles[t][0]];
                FaceHit { face, dist: dot(sub(p, a), n), normal: n }
            })
            .collect();
        hits.sort_by_key(|h| h.face);
        hits
    }

    /// Area-weighted mean outward normal of each model face (exact for a planar face).
    pub fn face_normals(&self) -> Vec<V3> {
        let mut sum = vec![[0.0; 3]; self.faces];
        for (t, f) in self.triangles.iter().zip(&self.face) {
            let n = cross(sub(self.vertices[t[1]], self.vertices[t[0]]), sub(self.vertices[t[2]], self.vertices[t[0]]));
            for k in 0..3 {
                sum[*f][k] += n[k] / 2.0;
            }
        }
        sum.into_iter().map(unit).collect()
    }

    /// Nearest model face to `p` (for naming a surface point).
    pub fn nearest_face(&self, p: V3) -> usize {
        let (_, t, _, _) = self.bvh.nearest(&self.vertices, &self.triangles, p);
        self.face[t]
    }

    /// Closest point on the surface and the face it lies on.
    pub fn closest_point(&self, p: V3) -> (V3, usize) {
        let (_, t, _, c) = self.bvh.nearest(&self.vertices, &self.triangles, p);
        (c, self.face[t])
    }
}

/// How finely to mesh a solid.
#[derive(Clone, Copy, Debug)]
pub struct VolumeParams {
    /// Largest element (the starting lattice).
    pub base: f64,
    /// Element size at the boundary.
    pub boundary_size: f64,
    pub quadratic: bool,
}

/// Fill a solid with tetrahedra. `size` is the size field (target edge length at a point); the boundary is meshed at
/// `boundary_size`. Faces are named `face1`.. in the order of the face tags.
pub fn mesh_volume(solid: &SurfaceSolid, mat: Elastic, params: VolumeParams, size: &dyn Fn(V3) -> f64) -> Result<(Mesh, MeshReport), String> {
    // The lattice covers exactly the bounding box, so planar faces on its sides coincide with lattice planes and the
    // snapping has nothing to do there (a padded box would leave every such face cutting through the cells).
    let (lo, hi) = (solid.lo, solid.hi);
    let sdf = |x: V3| solid.signed_distance(x);
    let face_of = |x: V3| solid.nearest_face(x);
    let radius = 1.2 * params.boundary_size;
    let face_dists = |x: V3| solid.faces_near(x, radius);
    let s = Solid { lo, hi, sdf: &sdf, size, face_of: &face_of, face_dists: Some(&face_dists) };
    fea_core::tetmesh::mesh_solid(&s, mat, TetOptions { base: params.base, boundary_size: params.boundary_size, quadratic: params.quadratic, ..Default::default() })
}
