//! `Geometry::Solid`: a B-rep solid stored as its STEP text plus the cached face-tagged surface mesh the solve needs.
//! The solve path (`solid_mesh`) uses only `solid-kernel`'s surface mesh and volume mesher; reading and editing STEP uses
//! the native kernel (OpenCascade was retired, ADR-023).
use crate::problem::{Geometry, Load, Problem, Support};
use fea_core::tetmesh::MeshReport;
use fea_core::Mesh;
use serde::{Deserialize, Serialize};
use solid_kernel::{mesh_volume, SurfaceMesh, SurfaceSolid, VolumeParams};

/// Elements the volume mesh may have (a tetrahedral mesh of a part is far bigger than a sketch mesh, so it has its own cap).
pub const MAX_TETS: f64 = 400_000.0;

/// The tessellation of a STEP solid at a chord deflection, tied to the text it came from by `hash`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SolidCache {
    /// Model length units per STEP kernel millimetre (kept through unit conversions).
    #[serde(default = "unit_scale")]
    pub coordinate_scale: f64,
    /// [`text_hash`] of the STEP text the mesh was made from.
    pub hash: String,
    pub deflection: f64,
    pub vertices: Vec<[f64; 3]>,
    pub triangles: Vec<[usize; 3]>,
    /// B-rep face index of each triangle.
    pub face: Vec<usize>,
    /// Faces of each body of an assembly, in body order: body `k` owns the face indices that follow body `k - 1`'s, so face names
    /// stay `faceN` over the whole assembly. Empty is one body (every face).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bodies: Vec<usize>,
}

/// FNV-1a 64 of the text as 16 hex digits: stable across Rust versions (unlike `DefaultHasher`) and platforms, which a
/// stored cache key needs. It detects an edited file, it is not a security measure.
pub fn text_hash(text: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in text.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}")
}

fn unit_scale() -> f64 {
    1.0
}

impl SolidCache {
    pub fn new(step: &str, mesh: SurfaceMesh, deflection: f64) -> Self {
        Self {
            coordinate_scale: 1.0,
            hash: text_hash(step), deflection, vertices: mesh.vertices, triangles: mesh.triangles, face: mesh.face, bodies: Vec::new(),
        }
    }
    /// Whether the cache was made from exactly this STEP text.
    pub fn is_current(&self, step: &str) -> bool {
        self.hash == text_hash(step)
    }
    pub fn validate(&self) -> Result<(), String> {
        if !self.coordinate_scale.is_finite() || self.coordinate_scale <= 0.0 || !self.deflection.is_finite() || self.deflection <= 0.0
            || self.vertices.len() < 4 || self.triangles.len() < 4 || self.face.len() != self.triangles.len()
            || self.vertices.iter().flatten().any(|v| !v.is_finite())
            || self.triangles.iter().flatten().any(|i| *i >= self.vertices.len())
            || self.face.iter().any(|f| *f >= self.triangles.len()) {
            return Err("invalid STEP surface cache: check finite coordinates, triangles, face tags and scale".into());
        }
        Ok(())
    }
    pub fn surface(&self) -> SurfaceMesh {
        SurfaceMesh { vertices: self.vertices.clone(), triangles: self.triangles.clone(), face: self.face.clone() }
    }
    /// `face1`.. for every B-rep face index that has a triangle, up to the largest (an assembly: up to the end of its last body).
    pub fn face_names(&self) -> Vec<String> {
        let top = self.face.iter().copied().max().map_or(0, |m| m + 1).max(self.bodies.iter().sum());
        (1..=top).map(|i| format!("face{i}")).collect()
    }
    /// The face-index range of each body (one range over every face for a single solid).
    pub fn body_faces(&self) -> Vec<std::ops::Range<usize>> {
        if self.bodies.is_empty() {
            return vec![0..self.face.iter().copied().max().map_or(0, |m| m + 1)];
        }
        let mut start = 0;
        self.bodies.iter().map(|n| { let r = start..start + n; start += n; r }).collect()
    }
    /// The body that owns face index `face`.
    pub fn body_of(&self, face: usize) -> usize {
        self.body_faces().iter().position(|r| r.contains(&face)).unwrap_or(0)
    }
    /// The closed surface of body `k` alone, its face tags renumbered from 0 and its vertices compacted.
    fn body_surface(&self, range: &std::ops::Range<usize>) -> SurfaceMesh {
        let mut index = std::collections::HashMap::new();
        let mut out = SurfaceMesh { vertices: Vec::new(), triangles: Vec::new(), face: Vec::new() };
        for (t, f) in self.triangles.iter().zip(&self.face).filter(|(_, f)| range.contains(f)) {
            out.triangles.push(t.map(|v| *index.entry(v).or_insert_with(|| { out.vertices.push(self.vertices[v]); out.vertices.len() - 1 })));
            out.face.push(f - range.start);
        }
        out
    }
}

pub(crate) fn cache_of(p: &Problem) -> Result<&SolidCache, String> {
    p.validate_geometry()?;
    match &p.geometry {
        Geometry::Solid { cache: Some(c), .. } => Ok(c),
        _ => Err("not a B-rep solid problem".into()),
    }
}

/// The closed, face-tagged surface of each body of a solid problem (one for a single solid; refused with the reason when a cached
/// mesh is open or broken). Face tags are local to the body: body `k`'s face `j` is the problem's face `body_faces()[k].start + j`.
pub fn solid_surfaces(p: &Problem) -> Result<Vec<SurfaceSolid>, String> {
    let c = cache_of(p)?;
    c.body_faces().iter().enumerate().map(|(k, range)| {
        let body = if c.bodies.is_empty() { c.surface() } else { c.body_surface(range) };
        SurfaceSolid::new(body, c.deflection * 1e-3).map_err(|e| if c.bodies.is_empty() { e } else { format!("body {}: {e}", k + 1) })
    }).collect()
}

/// The closed, face-tagged surface of a single-solid problem (an assembly has one per body: [`solid_surfaces`]).
pub fn solid_surface(p: &Problem) -> Result<SurfaceSolid, String> {
    let mut all = solid_surfaces(p)?;
    if all.len() != 1 {
        return Err(format!("the assembly has {} bodies, not one surface", all.len()));
    }
    Ok(all.remove(0))
}

/// The bodies' meshes as one: nodes and elements side by side, faces named by the problem's `faceN` (each body's own `face{j}`
/// moved on by the faces of the bodies before it). The bodies share no node: a tie joins them.
fn merge_bodies(parts: Vec<(Mesh, usize)>) -> Mesh {
    let mut out = Mesh::new(fea_core::Physics::Solid);
    for (m, face_offset) in parts {
        let node_offset = out.nodes.len();
        out.nodes.extend(m.nodes);
        for mut b in m.blocks {
            b.conn.iter_mut().for_each(|n| *n += node_offset);
            out.blocks.push(b);
        }
        for (name, list) in m.surfaces {
            let local: usize = name.strip_prefix("face").and_then(|d| d.parse().ok()).expect("the volume mesher names surfaces faceN");
            out.surfaces.insert(format!("face{}", local + face_offset), list.into_iter().map(|f| f.into_iter().map(|n| n + node_offset).collect()).collect());
        }
    }
    out
}

/// Tetrahedral volume mesh of a solid problem at the problem's mesh size (Tet10 for a quadratic element choice). Refinement
/// zones are sketch-plane features and do not apply. Faces are named `face1`.. as `Problem::edge_names` lists them. The bodies of an
/// assembly are meshed one by one (the size and volume checks run per body) and joined by `Problem::ties` at the solve.
pub fn solid_mesh(p: &Problem) -> Result<(Mesh, MeshReport), String> {
    let (mesh, report) = solid_mesh_of_geometry(p)?;
    check_faces_resolved(p, &mesh)?;
    Ok((mesh, report))
}

/// The volume mesh itself, which depends on the geometry, the mesh settings and the material but not on the supports and loads.
/// A mesh made for a problem without them can be reused by [`check_faces_resolved`] against the real ones.
fn solid_mesh_of_geometry(p: &Problem) -> Result<(Mesh, MeshReport), String> {
    let solids = solid_surfaces(p)?;
    let ranges = cache_of(p)?.body_faces();
    let h = p.mesh.size;
    let est: f64 = solids.iter().map(|s| s.volume()).sum::<f64>() / (0.1179 * h * h * h);
    if est > MAX_TETS {
        return Err(format!("about {est:.0} elements: that is too fine (limit {MAX_TETS:.0}); increase the mesh size"));
    }
    let params = VolumeParams { base: 2.0 * h, boundary_size: h, quadratic: p.mesh.element.is_quadratic(),
    };
    let (mut parts, mut report) = (Vec::new(), None::<MeshReport>);
    for (k, (solid, range)) in solids.iter().zip(&ranges).enumerate() {
        let who = if solids.len() > 1 { format!("body {}: ", k + 1) } else { String::new() };
        let (mesh, r) = mesh_volume(solid, p.material.elastic(), params, &|_| h).map_err(|e| format!("{who}{e}"))?;
        let volume_error = (r.volume / solid.volume() - 1.0).abs();
        if volume_error > 0.05 {
            return Err(format!("{who}the mesh does not resolve the STEP body: volume differs by {:.1}% (limit 5%); reduce element size from {h:.4e}", volume_error * 100.0));
        }
        report = Some(match report {
            None => r,
            Some(t) => MeshReport { tets: t.tets + r.tets, nodes: t.nodes + r.nodes, volume: t.volume + r.volume, worst_quality: t.worst_quality.min(r.worst_quality), snapped: t.snapped + r.snapped, refused: t.refused + r.refused },
        });
        parts.push((mesh, range.start));
    }
    let report = report.ok_or("the STEP solid has no body")?;
    let mesh = if parts.len() == 1 { parts.remove(0).0 } else { merge_bodies(parts) };
    Ok((mesh, report))
}

/// Refuse a support or load (mechanical or thermal) on a STEP surface that the mesh does not resolve: a face smaller than the
/// elements owns no boundary element, and a condition on it would be silently dropped.
pub(crate) fn check_faces_resolved(p: &Problem, mesh: &Mesh) -> Result<(), String> {
    let h = p.mesh.size;
    let unresolved = unresolved_faces(p, mesh);
    let missing: Vec<_> = p.supports.iter().filter_map(Support::edge).chain(p.loads.iter().filter_map(Load::edge))
        .chain(p.thermal.supports.iter().filter_map(crate::thermal::HeatSupport::edge))
        .chain(p.thermal.loads.iter().filter_map(crate::thermal::HeatLoad::edge))
        .filter(|name| unresolved.iter().any(|f| f == name)).collect();
    if !missing.is_empty() {
        return Err(format!("the mesh does not resolve STEP surfaces {}: reduce element size from {h:.4e}", missing.join(", ")));
    }
    Ok(())
}

/// CAD faces smaller than the volume mesh can remain visible in the body preview
/// without owning a boundary element. Never silently apply a condition to such a face.
pub fn unresolved_faces(p: &Problem, mesh: &Mesh) -> Vec<String> {
    match &p.geometry {
        Geometry::Solid { cache: Some(c), .. } => c.face_names().into_iter()
            .filter(|name| mesh.surfaces.get(name).is_none_or(Vec::is_empty)).collect(),
        _ => Vec::new(),
    }
}

pub fn resolution_note(p: &Problem, mesh: &Mesh) -> Option<String> {
    let names = unresolved_faces(p, mesh);
    (!names.is_empty()).then(|| format!("CAD surfaces {} have no boundary elements at this size; refine before applying conditions there and check mesh convergence.", names.join(", ")))
}

/// Tessellate STEP text into a cache (chord deflection `deflection`) with the native reader and sampler.

pub fn tessellate_step(step: &str, deflection: f64) -> Result<SolidCache, String> {
    crate::native_solid::tessellate_step_native(step, deflection)
}

impl Geometry {
    /// A solid geometry from STEP text, tessellated now by the selected CAD backend.
    pub fn solid_from_step(step: &str, deflection: f64) -> Result<Geometry, String> {
        let cache = tessellate_step(step, deflection)?;
        Ok(Geometry::Solid { step: step.to_string(), cache: Some(cache) })
    }
}

impl Problem {
    /// Make the problem a B-rep solid: the geometry, a 3D solid analysis and no sketch history (a solid is not built from
    /// one). Supports and loads are kept: they name faces (`faceN`), so the caller decides whether they still apply.
    pub fn set_solid(&mut self, step: &str, cache: SolidCache) {
        self.geometry = Geometry::Solid { step: step.to_string(), cache: Some(cache) };
        self.analysis = crate::problem::Analysis::Solid;
        self.history = None;
    }
}

// ------------------------------------------------------------------------------------------ editing a stored solid

/// Area, area-weighted centroid sum and net area vector of a face, built up triangle by triangle.
pub(crate) type FaceStat = (f64, [f64; 3], [f64; 3]);

/// Add triangle `[a, b, d]` to `e`; the centroid is a sum until [`finish_stat`].
pub(crate) fn add_triangle(e: &mut FaceStat, [a, b, d]: [[f64; 3]; 3]) {
    let (u, w) = ([b[0] - a[0], b[1] - a[1], b[2] - a[2]], [d[0] - a[0], d[1] - a[1], d[2] - a[2]]);
    let nv = [u[1] * w[2] - u[2] * w[1], u[2] * w[0] - u[0] * w[2], u[0] * w[1] - u[1] * w[0]];
    let ar = 0.5 * (nv[0] * nv[0] + nv[1] * nv[1] + nv[2] * nv[2]).sqrt();
    e.0 += ar;
    for k in 0..3 {
        e.1[k] += ar * (a[k] + b[k] + d[k]) / 3.0;
        e.2[k] += 0.5 * nv[k];
    }
}

/// The centroid of a finished face (its sum divided by the area).
pub(crate) fn finish_stat(e: &mut FaceStat) {
    if e.0 > 0.0 {
        e.1 = e.1.map(|v| v / e.0);
    }
}

/// Area, area-weighted centroid and net area vector of every face of a cache (one entry per `face` index).
pub(crate) fn face_stats(c: &SolidCache) -> Vec<FaceStat> {
    let n = c.face.iter().copied().max().map_or(0, |m| m + 1).max(c.bodies.iter().sum());
    let mut out = vec![(0.0, [0.0; 3], [0.0; 3]); n];
    for (t, f) in c.triangles.iter().zip(&c.face) {
        add_triangle(&mut out[*f], t.map(|i| c.vertices[i]));
    }
    out.iter_mut().for_each(finish_stat);
    out
}

/// Which face of `new` each face of `old` became across a solid edit (`None`: the edit removed it). Face numbers are the
/// kernel's order and change with every boolean or fillet, so the faces are matched by what they are: the same kind of
/// surface (flat with the same normal and plane, or curved) whose centroid stays within a third of the face's size and whose
/// area stays within a factor of 20, best match first, each new face used once. This holds for what these edits do to a
/// surviving face: a hole or boss through it keeps its centroid and loses area, a fillet trims its rim. Done on the
/// tessellations, so it needs no kernel and does not rely on the kernel's face ids (not unique, lost by STEP).
pub fn remap_faces(old: &SolidCache, new: &SolidCache) -> Vec<Option<usize>> {
    let (a, b) = (face_stats(old), face_stats(new));
    let diag = {
        let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
        for v in &old.vertices {
            for k in 0..3 {
                lo[k] = lo[k].min(v[k]);
                hi[k] = hi[k].max(v[k]);
            }
        }
        (0..3).map(|k| (hi[k] - lo[k]).powi(2)).sum::<f64>().sqrt()
    };
    let len = |v: [f64; 3]| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    let mut pairs: Vec<(f64, usize, usize)> = Vec::new();
    for (i, (ai, ci, ni)) in a.iter().enumerate() {
        if *ai <= 0.0 {
            continue;
        }
        let flat_i = len(*ni) / ai;
        for (j, (aj, cj, nj)) in b.iter().enumerate() {
            if *aj <= 0.0 {
                continue;
            }
            let flat_j = len(*nj) / aj;
            let ratio = aj / ai;
            if !(0.05..=20.0).contains(&ratio) || (flat_i - flat_j).abs() > 0.15 {
                continue;
            }
            let delta = [cj[0] - ci[0], cj[1] - ci[1], cj[2] - ci[2]];
            if flat_i > 0.99 && flat_j > 0.99 {
                let u = ni.map(|v| v / len(*ni));
                let dot = u[0] * nj[0] / len(*nj) + u[1] * nj[1] / len(*nj) + u[2] * nj[2] / len(*nj);
                if dot < 0.999 || (delta[0] * u[0] + delta[1] * u[1] + delta[2] * u[2]).abs() > 1e-3 * diag {
                    continue;
                }
            } else if flat_i > 0.5 && flat_j > 0.5 {
                // Gently curved: the normals must still agree.
                let dot = ni[0] * nj[0] + ni[1] * nj[1] + ni[2] * nj[2];
                if dot < 0.9 * len(*ni) * len(*nj) {
                    continue;
                }
            }
            let d = len(delta) / ai.sqrt();
            if d > 1.0 / 3.0 {
                continue;
            }
            pairs.push((d + 0.1 * ratio.ln().abs(), i, j));
        }
    }
    pairs.sort_by(|x, y| x.0.total_cmp(&y.0));
    let mut out = vec![None; a.len()];
    let mut used = vec![false; b.len()];
    for (_, i, j) in pairs {
        if out[i].is_none() && !used[j] {
            out[i] = Some(j);
            used[j] = true;
        }
    }
    out
}

/// An edit of a stored solid. The available kernel applies it or reports why the shape is unsupported.
#[derive(Clone, Debug, PartialEq)]
pub enum SolidOp {
    /// Round every edge.
    Fillet { radius: f64 },
    /// A round hole through the part at the middle of flat face `face` (1-based, `faceN`), perpendicular to it.
    Hole { face: usize, diameter: f64 },
    /// A round boss standing on flat face `face`.
    Boss { face: usize, diameter: f64, height: f64 },
}

/// A solid read from STEP: its cache and a sensible element size (an eighteenth of the bounding-box diagonal, capped by wall thickness).
#[derive(Clone, Debug)]
pub struct StepImport {
    pub cache: SolidCache,
    pub size: f64,
}

/// Middle and outward normal of a flat face of a cache, from its triangles (`Err`: no such face, or not flat).
pub fn flat_face(c: &SolidCache, face: usize) -> Result<([f64; 3], [f64; 3]), String> {
    let f = face.checked_sub(1).ok_or("faces are numbered from face1")?;
    let (mut sum, mut area, mut normal) = ([0.0; 3], 0.0, [0.0; 3]);
    let mut tri_normals = Vec::new();
    for (t, tf) in c.triangles.iter().zip(&c.face) {
        if *tf != f {
            continue;
        }
        let [a, b, d] = t.map(|i| c.vertices[i]);
        let (u, w) = ([b[0] - a[0], b[1] - a[1], b[2] - a[2]], [d[0] - a[0], d[1] - a[1], d[2] - a[2]],
        );
        let n = [u[1] * w[2] - u[2] * w[1], u[2] * w[0] - u[0] * w[2], u[0] * w[1] - u[1] * w[0],
        ];
        let ar = 0.5 * (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        if ar <= 0.0 {
            continue;
        }
        for k in 0..3 {
            sum[k] += ar * (a[k] + b[k] + d[k]) / 3.0;
            normal[k] += 0.5 * n[k];
        }
        tri_normals.push([n[0] / (2.0 * ar), n[1] / (2.0 * ar), n[2] / (2.0 * ar)]);
        area += ar;
    }
    if tri_normals.is_empty() || area <= 0.0 {
        return Err(format!("the solid has no face{face}"));
    }
    let len = (normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2]).sqrt();
    let n = [normal[0] / len, normal[1] / len, normal[2] / len];
    if tri_normals.iter().any(|t| t[0] * n[0] + t[1] * n[1] + t[2] * n[2] < 0.999) {
        return Err(format!("face{face} is curved: pick a flat face"));
    }
    // The tessellation's winding is the kernel's: orient the normal outward by the sign of the enclosed volume.
    let mut vol = 0.0;
    for t in &c.triangles {
        let [a, b, d] = t.map(|i| c.vertices[i]);
        vol += a[0] * (b[1] * d[2] - b[2] * d[1]) + a[1] * (b[2] * d[0] - b[0] * d[2]) + a[2] * (b[0] * d[1] - b[1] * d[0]);
    }
    let s = if vol < 0.0 { -1.0 } else { 1.0 };
    Ok(([sum[0] / area, sum[1] / area, sum[2] / area], [s * n[0], s * n[1], s * n[2]],
    ))
}

/// The element size a freshly imported solid gets: an eighteenth of the bounding-box diagonal, capped by a third of the thinnest
/// wall between opposite flat faces and of the smallest box side (shared by the OpenCascade and the native import).
pub(crate) fn finish_import(cache: SolidCache, lo: [f64; 3], hi: [f64; 3], diag: f64) -> StepImport {
    // A wall is the thickness of one body: faces of different bodies of an assembly (an air gap between plates) are not one.
    let planes: Vec<_> = (0..cache.face_names().len()).filter_map(|f| flat_face(&cache, f + 1).ok().map(|p| (cache.body_of(f), p))).collect();
    let mut wall = f64::INFINITY;
    for (i, (body_a, (a, n))) in planes.iter().enumerate() {
        for (body_b, (b, m)) in planes.iter().skip(i + 1) {
            if body_a != body_b {
                continue;
            }
            let dot = n.iter().zip(m).map(|(a,b)| a*b).sum::<f64>();
            let distance = (0..3).map(|d| (b[d]-a[d])*n[d]).sum::<f64>().abs();
            if dot < -0.999 && distance > diag * 1e-4 { wall = wall.min(distance); }
        }
    }
    let base = (diag / 18.0).min(wall / 3.0).min((0..3).map(|a| hi[a]-lo[a]).fold(f64::INFINITY, f64::min) / 3.0);
    let size = feature_size(&cache, base);
    StepImport { cache, size }
}

/// Elements per radius of the smallest curved feature the default size resolves (two across a radius: about four around a
/// quarter-turn fillet).
const ELEMENTS_PER_RADIUS: f64 = 2.0;
/// The most elements a default size may ask for (estimated by the same V / (0.1179 h^3) as `MAX_TETS`; the real count runs about 2.5 times higher, ~65k Tet4 elements, minutes to solve): a part with features too small to resolve within
/// it keeps the coarser size, and `solid_mesh` then says which feature the mesh does not resolve.
const DEFAULT_TETS: f64 = 25_000.0;

/// The element size a fresh import gets: `base` (bounding box and wall rules), refined to resolve the smallest curved feature
/// (`smallest_curvature_radius`) but never so fine that the estimated element count exceeds [`DEFAULT_TETS`].
pub(crate) fn feature_size(c: &SolidCache, base: f64) -> f64 {
    let Some(r) = smallest_curvature_radius(c) else { return base };
    let volume: f64 = c.triangles.iter().map(|t| {
        let [a, b, d] = t.map(|i| c.vertices[i]);
        a[0] * (b[1] * d[2] - b[2] * d[1]) + a[1] * (b[2] * d[0] - b[0] * d[2]) + a[2] * (b[0] * d[1] - b[1] * d[0])
    }).sum::<f64>().abs() / 6.0;
    let floor = (volume / (0.1179 * DEFAULT_TETS)).cbrt();
    base.min((r / ELEMENTS_PER_RADIUS).max(floor))
}

/// Smallest radius of curvature of any curved face of the tessellation, from the turn between neighbouring triangles of one face
/// (`R = (distances of the two far vertices from the shared edge) / (2 x angle between the normals)`). Per face the smallest over its edges is taken (an edge oblique to the curvature direction reads R / sin^2 of its angle to it, too flat, so every error is
/// one-sided and the axial edges give the true radius); flat faces (turn below 2 mrad everywhere) have no radius. `None` for an all-flat solid.
pub fn smallest_curvature_radius(c: &SolidCache) -> Option<f64> {
    use std::collections::HashMap;
    let sub = |a: [f64; 3], b: [f64; 3]| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    let cross = |a: [f64; 3], b: [f64; 3]| [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
    let norm = |a: [f64; 3]| (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt();
    let mut info: Vec<Option<([f64; 3], [f64; 3])>> = Vec::with_capacity(c.triangles.len());
    for t in &c.triangles {
        let [a, b, d] = t.map(|i| c.vertices[i]);
        let n = cross(sub(b, a), sub(d, a));
        let len = norm(n);
        info.push((len > 0.0).then(|| ([n[0] / len, n[1] / len, n[2] / len], [(a[0] + b[0] + d[0]) / 3.0, (a[1] + b[1] + d[1]) / 3.0, (a[2] + b[2] + d[2]) / 3.0])));
    }
    let mut edges: HashMap<(usize, usize), usize> = HashMap::new();
    let mut per_face: HashMap<usize, Vec<f64>> = HashMap::new();
    for (ti, t) in c.triangles.iter().enumerate() {
        for k in 0..3 {
            let (a, b) = (t[k], t[(k + 1) % 3]);
            let key = (a.min(b), a.max(b));
            match edges.remove(&key) {
                None => {
                    edges.insert(key, ti);
                }
                Some(tj) => {
                    if c.face[tj] != c.face[ti] {
                        continue;
                    }
                    let (Some((n1, _)), Some((n2, _))) = (info[ti], info[tj]) else { continue };
                    let angle = norm(cross(n1, n2)).atan2(n1[0] * n2[0] + n1[1] * n2[1] + n1[2] * n2[2]);
                    if angle > 2e-3 {
                        // Each triangle tilts by half the arc angle to its far vertex, so the dihedral angle is half the sum of the two arc
                        // angles, and the arcs are the far vertices' distances from the shared edge: R = (p1 + p2) / (2 angle), exact on
                        // a cylinder or sphere whatever the triangles' shapes.
                        let (pa, pb) = (c.vertices[a], c.vertices[b]);
                        let e = sub(pb, pa);
                        let el = norm(e);
                        let far = |tri: usize| {
                            let w = c.triangles[tri].into_iter().find(|&w| w != a && w != b).map_or(pa, |w| c.vertices[w]);
                            let v = sub(w, pa);
                            let t = (v[0] * e[0] + v[1] * e[1] + v[2] * e[2]) / (el * el);
                            norm([v[0] - t * e[0], v[1] - t * e[1], v[2] - t * e[2]])
                        };
                        per_face.entry(c.face[ti]).or_default().push((far(ti) + far(tj)) / (2.0 * angle));
                    }
                }
            }
        }
    }
    per_face.into_values().filter(|v| v.len() >= 3).map(|mut v| {
        v.sort_by(f64::total_cmp);
        v[0]
    }).min_by(f64::total_cmp)
}

/// The native pipeline reads the solid (`native_solid`).
pub fn import_step(step: &str) -> Result<StepImport, String> {
    crate::native_solid::import_step_native(step)
}

/// Apply `op` to the solid (`step`, `cache`); returns the new STEP text and its tessellation. Face numbers change with the
/// shape: supports and loads that name faces must be checked afterwards (`diagnose` finds the ones left dangling).

pub fn apply_op(step: &str, cache: &SolidCache, op: &SolidOp) -> Result<(String, SolidCache), String> {
    crate::native_solid::apply_op_native(step, cache, op)
}

impl Problem {
    /// A new problem around a STEP solid (default material, no supports or loads; the caller adds boundary conditions).
    pub fn from_step(name: &str, step: &str) -> Result<Problem, String> {
        let import = import_step(step)?;
        let mut p = crate::templates::blank();
        p.convert_units(crate::UnitSystem::MmMpa)?;
        p.name = name.to_string();
        p.mesh.size = import.size;
        p.set_solid(step, import.cache);
        p.supports.clear();
        p.loads.clear();
        Ok(p)
    }

    /// Apply a solid edit to a solid problem (a new STEP text and cache). Supports and loads that name faces follow the
    /// faces across the edit (`remap_faces`); one whose face the edit removed is renamed `removed:faceN`, a name the
    /// geometry does not have, so `diagnose` reports it instead of it silently landing on another face.
    pub fn apply_solid_op(&mut self, op: &SolidOp) -> Result<(), String> {
        let Geometry::Solid { step, cache: Some(cache) } = &self.geometry else { return Err("this model is not a STEP solid".into()) };
        if !cache.is_current(step) {
            return Err("the solid's cached surface mesh is out of date".into());
        }
        let (new_step, new_cache) = apply_op(step, cache, op)?;
        let map = remap_faces(cache, &new_cache);
        let rename = |name: &mut String| {
            let Some(n) = name.strip_prefix("face").and_then(|d| d.parse::<usize>().ok()).filter(|n| *n >= 1) else { return };
            *name = match map.get(n - 1) {
                Some(Some(j)) => format!("face{}", j + 1),
                _ => format!("removed:face{n}"),
            };
        };
        for s in &mut self.supports {
            if let Support::Edge { edge, .. } = s {
                rename(edge);
            }
        }
        for l in &mut self.loads {
            if let Load::Pressure { edge, .. } | Load::Traction { edge, .. } | Load::Force { edge, .. } = l {
                rename(edge);
            }
        }
        self.geometry = Geometry::Solid { step: new_step, cache: Some(new_cache) };
        Ok(())
    }
}
